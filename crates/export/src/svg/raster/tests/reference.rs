//! blend 영역 검색을 단계별 순차 탐색 기준 구현과 비교합니다 (XX.4.1 후속).
//!
//! 기준 구현은 모듈 문서 규칙을 그대로 따릅니다. blend node에서 isolation group(없으면 canvas)까지 한
//! 단계씩 올라가며 단계마다 조상 자기 내용과 경로 자식보다 앞선 형제 subtree를 앞에서부터 하나씩 훑고,
//! 겹침이 있는 더 높은 단계가 나오면 영역을 덮어씁니다. 작업량은 형제 수에 비례하지만 구간 tree를 쓰지
//! 않습니다. 구현의 pre-order 구간 검색과 영역·backdrop 의존이 모든 blend node에서 같아야 합니다.

use super::painter::{random_scene, Rng};
use super::*;

/// `parent`의 자식(paint 순서).
fn children(arena: &Arena<'_>, parent: usize) -> Vec<usize> {
    let end = arena.entries[parent].end;
    let mut out = Vec::new();
    let mut next = parent + 1;
    while next < end {
        out.push(next);
        next = arena.entries[next].end;
    }
    out
}

/// `start` subtree에서 `target`과 겹치는 첫 내용(pre-order). blur node는 subtree가 겹치면 그 node
/// 자체가 겹침입니다.
fn first_hit(arena: &Arena<'_>, start: usize, target: &PixelRect) -> Option<usize> {
    let mut stack = vec![start];
    while let Some(index) = stack.pop() {
        let entry = &arena.entries[index];
        if !arena.grid.overlaps(entry.subtree, target) {
            continue;
        }
        if entry.blur > 0 || arena.grid.overlaps(entry.own, target) {
            return Some(index);
        }
        stack.extend(children(arena, index).into_iter().rev());
    }
    None
}

/// `siblings` 중 `before` 앞에서 `target`과 겹치는 첫 subtree와 그 안의 첫 겹침 내용.
fn first_overlapping(
    arena: &Arena<'_>,
    siblings: &[usize],
    before: usize,
    target: &PixelRect,
) -> Option<(usize, usize)> {
    siblings
        .iter()
        .copied()
        .take_while(|&sibling| sibling != before)
        .find_map(|sibling| first_hit(arena, sibling, target).map(|hit| (sibling, hit)))
}

/// blend node의 영역과 backdrop 의존(단계별 순차 탐색).
fn blend_region(arena: &Arena<'_>, index: usize, background: bool) -> (Kind, BlendDependency) {
    let mut kind = arena.subtree_kind(index);
    let mut widest = None;
    let target = arena.grid.in_canvas(arena.entries[index].subtree);
    let mut child = index;
    let isolation = loop {
        match arena.entries[child].parent {
            Some(parent) => {
                if let Some(target) = &target {
                    if arena.grid.overlaps(arena.entries[parent].own, target) {
                        kind = arena.subtree_kind(parent);
                        widest = Some(BackdropItem::Node(arena.raster_node(parent)));
                    } else if let Some((first, hit)) =
                        first_overlapping(arena, &children(arena, parent), child, target)
                    {
                        kind = Kind::Range {
                            parent: Some(parent),
                            first,
                            last: child,
                        };
                        widest = Some(BackdropItem::Node(arena.raster_node(hit)));
                    }
                }
                if arena.entries[parent].isolating {
                    break Some(arena.raster_node(parent));
                }
                child = parent;
            }
            None => {
                if let Some(target) = &target {
                    if background {
                        kind = Kind::Canvas;
                        widest = Some(BackdropItem::Background);
                    } else if let Some((first, hit)) =
                        first_overlapping(arena, &arena.roots, child, target)
                    {
                        kind = Kind::Range {
                            parent: None,
                            first,
                            last: child,
                        };
                        widest = Some(BackdropItem::Node(arena.raster_node(hit)));
                    }
                }
                break None;
            }
        }
    };
    (kind, BlendDependency { isolation, widest })
}

#[test]
fn indexed_blend_search_matches_level_by_level_scan() {
    let mut rng = Rng(0x0b1e_5ca1_ab1e_0042);
    let (mut blends, mut isolated, mut ranges, mut ancestors, mut far) = (0, 0, 0, 0, 0);
    for round in 0..600 {
        // 작은 밀집 scene부터 구간 tree가 여러 단계인 큰 성긴 scene까지.
        let (extra, spread) = match round % 3 {
            0 => (10, 26),
            1 => (80, 60),
            _ => (400, 160),
        };
        let roots = random_scene(&mut rng, extra, spread);
        let refs: Vec<&SceneNode> = roots.iter().map(|root| &**root).collect();
        let side = (spread - 6) as f32;
        let arena = Arena::build(&refs, grid(side, side, 1.0));
        for background in [false, true] {
            for index in 0..arena.entries.len() {
                if arena.entries[index].node.paint().get().blend == SceneBlendMode::Normal {
                    continue;
                }
                let expected = blend_region(&arena, index, background);
                assert_eq!(
                    arena.blend_region(index, background),
                    expected,
                    "round {round}, entry {index}, background {background}"
                );
                blends += 1;
                let (kind, dependency) = expected;
                if dependency.isolation.is_some() {
                    isolated += 1;
                }
                let (Kind::Range { first, last, .. }, Some(BackdropItem::Node(hit))) =
                    (kind, dependency.widest)
                else {
                    continue;
                };
                if first == last {
                    // 경로 위 조상(또는 isolation group)의 자기 내용.
                    ancestors += 1;
                    continue;
                }
                ranges += 1;
                let at = arena
                    .entries
                    .iter()
                    .position(|entry| RasterNode::of(entry.node) == hit);
                if at.is_some_and(|at| index - at > BLOCK) {
                    far += 1;
                }
            }
        }
    }
    // 검출력: 앞선 형제 범위, 조상 자기 내용, isolation group 안 blend, blend보다 구간 tree 잎 하나 넘게
    // 앞선 backdrop이 충분히 나옵니다.
    assert!(blends > 10_000, "{blends}");
    assert!(isolated > 4000, "{isolated}");
    assert!(ranges > 1000, "{ranges}");
    assert!(ancestors > 1500, "{ancestors}");
    assert!(far > 500, "{far}");
}
