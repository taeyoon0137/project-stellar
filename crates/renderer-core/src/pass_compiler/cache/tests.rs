//! 증분 frame compile 테스트 (ADR-0090).
//!
//! 기준은 한 가지입니다: [`FrameCompiler`]가 어떤 frame 순서로 compile하든 결과는 같은 입력의
//! [`compile_frame`] 결과와 같아야 합니다. 그 위에서 재사용 집계가 바뀐 부분만 다시 처리했는지 확인합니다.

use std::fmt::Write as _;
use std::sync::Arc;

use stellar_scene::SceneSnapshot;

use super::super::tests::{input, random_scene, viewport_with, Rng};
use super::super::*;
use crate::test_scene::{group, opaque, rect, rect_clip, snapshot, NodeFx, IDENTITY};

/// plan의 비교용 표현(draw·pass·layer·graph 순서). 같은 snapshot의 node는 z로 식별합니다.
fn fingerprint(plan: &FramePlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "order {:?}", plan.graph.order());
    for pass in &plan.passes {
        let _ = writeln!(
            out,
            "pass {:?} {:?} {:?}",
            pass.pass, pass.class, pass.layer
        );
        for draw in &pass.draws {
            let source = match &draw.source {
                DrawSource::Content(node) => format!("content {}", node.z().0),
                DrawSource::Layer { layer, node } => format!("layer {} {}", layer.0, node.z().0),
            };
            let _ = writeln!(
                out,
                "  z={} depth={} {:?} {source} opacity={:08x} {:?} {:?}",
                draw.z.0,
                draw.depth,
                draw.class,
                draw.opacity.get().to_bits(),
                draw.blend,
                draw.clip
            );
        }
    }
    for layer in &plan.layers {
        let _ = writeln!(
            out,
            "layer {} node={} parent={:?} output={:?}",
            layer.id.0,
            layer.node.z().0,
            layer.parent,
            layer.output
        );
    }
    out
}

fn full(scene: &SceneSnapshot, viewport: &Viewport, boundaries: EffectBoundaries) -> String {
    fingerprint(&compile_frame(&input(scene, viewport, boundaries)).unwrap())
}

/// node 위치(최상위 index, 자식 index, …) 목록(pre-order).
fn paths(roots: &[NodeFx]) -> Vec<Vec<usize>> {
    fn walk(node: &NodeFx, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        out.push(path.clone());
        for (index, child) in node.children.iter().enumerate() {
            path.push(index);
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let mut path = vec![index];
        walk(root, &mut path, &mut out);
    }
    out
}

fn node_at<'a>(roots: &'a mut [NodeFx], path: &[usize]) -> &'a mut NodeFx {
    let (first, rest) = path.split_first().unwrap();
    let mut node = &mut roots[*first];
    for index in rest {
        node = &mut node.children[*index];
    }
    node
}

/// `scene`에서 `path` 위치 subtree만 `edited`의 같은 위치 subtree로 바꾼 snapshot. 나머지 node는 `scene`과 같은
/// `Arc`입니다(scene 증분 build가 만드는 공유와 같음).
fn replaced(scene: &SceneSnapshot, edited: &SceneSnapshot, path: &[usize]) -> SceneSnapshot {
    let (first, rest) = path.split_first().unwrap();
    let mut node = edited.root(*first).unwrap();
    for index in rest {
        node = node.child(*index).unwrap();
    }
    scene
        .replace_node(
            scene.document_revision(),
            edited.scene_revision(),
            path,
            Arc::clone(node),
        )
        .unwrap()
}

/// node 하나의 내용을 바꿉니다(색·불투명도·위치·숨김 중 하나).
fn edit(node: &mut NodeFx, rng: &mut Rng) {
    match rng.below(5) {
        0 => node.paint.fills = vec![[0.9, 0.1, 0.1, 1.0]],
        1 => node.paint.opacity = if node.paint.opacity < 1.0 { 1.0 } else { 0.5 },
        2 => node.transform = [1.0, 0.0, 0.0, 1.0, 0.25, 1.0],
        3 => node.hidden = !node.hidden,
        _ => node.paint.blend = if node.paint.blend == 0 { 2 } else { 0 },
    }
}

#[test]
fn incremental_plans_match_full_compile_on_random_edits() {
    let viewport = viewport_with(16.0, 16.0, IDENTITY);
    let mut rng = Rng(0x1ce5_7a11_0000_0090);
    let mut reused = 0u64;
    for round in 0..300 {
        let boundaries = if round % 2 == 0 {
            EffectBoundaries::Minimal
        } else {
            EffectBoundaries::Conservative
        };
        let roots = random_scene(&mut rng);
        let mut compiler = FrameCompiler::new();
        let mut scene = snapshot(&roots);
        // 문맥이 정해진 뒤 두 번째 frame부터 다음 frame용 상태를 남깁니다.
        for _ in 0..2 {
            let plan = compiler
                .compile(&input(&scene, &viewport, boundaries))
                .unwrap();
            assert_eq!(fingerprint(&plan), full(&scene, &viewport, boundaries));
        }
        let mut current = roots.clone();
        for _ in 0..4 {
            let all = paths(&current);
            let path = all[rng.below(all.len() as u64) as usize].clone();
            let mut edited_roots = current.clone();
            edit(node_at(&mut edited_roots, &path), &mut rng);
            let edited = snapshot(&edited_roots);
            scene = replaced(&scene, &edited, &path);
            current = edited_roots;
            let plan = compiler
                .compile(&input(&scene, &viewport, boundaries))
                .unwrap();
            assert_eq!(
                fingerprint(&plan),
                full(&scene, &viewport, boundaries),
                "round {round} path {path:?}"
            );
            assert!(!compiler.last_reuse().context_changed);
            reused += compiler.last_reuse().reused_subtrees;
        }
    }
    // 무작위 편집이 재사용 경로를 충분히 지납니다.
    assert!(reused > 200, "{reused}");
}

/// 화면을 채우는 card 50개(card마다 사각형 clip 안 사각형 4개).
fn cards() -> Vec<NodeFx> {
    let mut roots = Vec::new();
    let mut id = 0u8;
    for card in 0..50u8 {
        let x = f32::from(card % 10) * 10.0;
        let y = f32::from(card / 10) * 10.0;
        let clip = rect_clip([x, y, 9.0, 9.0]);
        let mut children = Vec::new();
        for leaf in 0..4u8 {
            id = id.wrapping_add(1);
            children.push(
                rect(
                    id,
                    [x + f32::from(leaf) * 2.0, y, 2.0, 2.0],
                    opaque(0.1, 0.2, 0.3),
                )
                .with_clip(clip.clone()),
            );
        }
        id = id.wrapping_add(1);
        roots.push(group(id, [x, y, 9.0, 9.0], children));
    }
    roots
}

#[test]
fn single_leaf_edit_revisits_only_its_card() {
    // 검증 기준(XI.2.5와 같은 방식): 단일 property 편집의 재처리 수가 예상과 같습니다.
    let viewport = viewport_with(100.0, 50.0, IDENTITY);
    let roots = cards();
    let before = snapshot(&roots);
    let mut compiler = FrameCompiler::new();
    compiler
        .compile(&input(&before, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert!(compiler.last_reuse().context_changed);
    compiler
        .compile(&input(&before, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert_eq!(compiler.last_reuse().reused_subtrees, 0);

    // card 7의 둘째 사각형 색만 바꿉니다.
    let mut edited_roots = roots.clone();
    edited_roots[7].children[1].paint.fills = vec![opaque(0.9, 0.0, 0.0)];
    let after = replaced(&before, &snapshot(&edited_roots), &[7, 1]);
    let plan = compiler
        .compile(&input(&after, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert_eq!(
        fingerprint(&plan),
        full(&after, &viewport, EffectBoundaries::Minimal)
    );
    let reuse = compiler.last_reuse();
    // 나머지 card 49개는 재사용하고, card 7은 자신과 자식 4개만 다시 처리합니다.
    assert_eq!(reuse.reused_subtrees, 49);
    assert_eq!(reuse.reused_draws, 49 * 4);
    assert_eq!(reuse.visited_nodes, 49 + 1 + 4);
    assert!(!reuse.context_changed);

    // 바뀌지 않은 frame은 최상위 card 50개를 모두 재사용합니다.
    compiler
        .compile(&input(&after, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert_eq!(compiler.last_reuse().reused_subtrees, 50);
    assert_eq!(compiler.last_reuse().visited_nodes, 50);
}

#[test]
fn viewport_change_compiles_fully_and_resumes_reuse_when_stable() {
    let roots = cards();
    let scene = snapshot(&roots);
    let mut compiler = FrameCompiler::new();
    let views: Vec<Viewport> = (0..4)
        .map(|step| viewport_with(100.0, 50.0, [1.0, 0.0, 0.0, 1.0, step as f32 * 0.5, 0.0]))
        .collect();
    // Pan: frame마다 문맥이 바뀌어 재사용도, 다음 frame용 기록도 없습니다.
    for viewport in &views {
        let plan = compiler
            .compile(&input(&scene, viewport, EffectBoundaries::Minimal))
            .unwrap();
        assert_eq!(
            fingerprint(&plan),
            full(&scene, viewport, EffectBoundaries::Minimal)
        );
        let reuse = compiler.last_reuse();
        assert!(reuse.context_changed);
        assert_eq!(reuse.reused_subtrees, 0);
    }
    // 멈춘 뒤 첫 frame은 기록만 하고, 그다음 frame부터 재사용합니다.
    let last = &views[3];
    compiler
        .compile(&input(&scene, last, EffectBoundaries::Minimal))
        .unwrap();
    assert!(!compiler.last_reuse().context_changed);
    assert_eq!(compiler.last_reuse().reused_subtrees, 0);
    compiler
        .compile(&input(&scene, last, EffectBoundaries::Minimal))
        .unwrap();
    assert_eq!(compiler.last_reuse().reused_subtrees, 50);
    // 경계 방식이 바뀌어도 문맥 변경입니다.
    compiler
        .compile(&input(&scene, last, EffectBoundaries::Conservative))
        .unwrap();
    assert!(compiler.last_reuse().context_changed);
}

#[test]
fn effect_subtree_boundary_plans_are_reused_by_identity() {
    let viewport = viewport_with(100.0, 50.0, IDENTITY);
    let mut roots = cards();
    // card 0·1을 겹치는 자식을 가진 반투명 group으로 만듭니다(layer 경계), card 2는 접힘.
    roots[0].paint.opacity = 0.5;
    roots[0].children[1] = rect(240, [0.0, 0.0, 4.0, 4.0], opaque(0.0, 1.0, 0.0))
        .with_clip(rect_clip([0.0, 0.0, 9.0, 9.0]));
    roots[1].paint.opacity = 0.5;
    roots[2].paint.opacity = 0.25;
    let scene = snapshot(&roots);
    let mut compiler = FrameCompiler::new();
    for _ in 0..2 {
        compiler
            .compile(&input(&scene, &viewport, EffectBoundaries::Minimal))
            .unwrap();
    }
    let plan = compiler
        .compile(&input(&scene, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert_eq!(
        fingerprint(&plan),
        full(&scene, &viewport, EffectBoundaries::Minimal)
    );
    assert!(!plan.layers.is_empty());
    let reuse = compiler.last_reuse();
    // 효과 card 3개의 결정은 재사용하고, 효과 없는 card 47개는 draw 구간을 재사용합니다.
    assert_eq!(reuse.reused_boundary_plans, 3);
    assert_eq!(reuse.reused_subtrees, 47);
}

#[test]
fn reset_drops_the_reuse_state() {
    let viewport = viewport_with(100.0, 50.0, IDENTITY);
    let scene = snapshot(&cards());
    let mut compiler = FrameCompiler::new();
    for _ in 0..2 {
        compiler
            .compile(&input(&scene, &viewport, EffectBoundaries::Minimal))
            .unwrap();
    }
    compiler.reset();
    compiler
        .compile(&input(&scene, &viewport, EffectBoundaries::Minimal))
        .unwrap();
    assert!(compiler.last_reuse().context_changed);
    assert_eq!(compiler.last_reuse().reused_subtrees, 0);
}
