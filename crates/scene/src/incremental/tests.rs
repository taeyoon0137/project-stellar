//! 증분 scene build 테스트 (XI.2.4, XI.2.5, XI.2.6, XII.3.7, XII.10.30).

use super::*;
use crate::boolean::{signed_area, BooleanOp};
use crate::builder::{FragmentInput, FrameClipInput, FrameInput, NodeInput, PathInput, ShapeInput};
use crate::dirty::SceneDirtyKind;
use crate::fixtures::{id, p, rect, solid, translate};
use crate::geometry::{PathCommand, PathGeometry};
use crate::node::{CornerRadii, SceneContent};
use crate::source::{BooleanSource, SceneKey, SceneSource, SourceEdit, SourceKind, SourceNode};
use crate::stack::StrokeCap;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::Size2;

const SECTIONS: u64 = 4;
const LEAVES: u64 = 5;

fn key(n: u64) -> SceneKey {
    SceneKey::node(id(n))
}

fn frame_input(n: u64, x: f32, y: f32, w: f32, h: f32) -> FrameInput {
    FrameInput {
        node: NodeInput::new(id(n), translate(x, y)),
        size: Size2::new(w, h).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: Some(Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).unwrap()),
        border: None,
        clip: FrameClipInput::Bounds,
    }
}

fn square(size: f32) -> PathGeometry {
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, 0.0)),
        PathCommand::LineTo(p(size, 0.0)),
        PathCommand::LineTo(p(size, size)),
        PathCommand::LineTo(p(0.0, size)),
        PathCommand::Close,
    ])
    .unwrap()
}

fn shape_input(n: u64, x: f32, red: f32) -> ShapeInput {
    let mut node = NodeInput::new(id(n), translate(x, 2.0));
    node.paint.fills.push(solid(red, 0.0, 0.0, 1.0));
    ShapeInput {
        node,
        layout_rect: rect(0.0, 0.0, 8.0, 8.0),
        geometry: square(8.0),
        cap: StrokeCap::Butt,
    }
}

fn section(s: u64) -> u64 {
    10 + s * 100
}

fn leaf(s: u64, l: u64) -> u64 {
    section(s) + 1 + l
}

/// root Frame(1) 아래 section Frame 4개, section마다 Shape leaf 5개: scene node 25개.
fn source() -> SceneSource {
    let mut nodes = Vec::new();
    let sections: Vec<SceneKey> = (0..SECTIONS).map(|s| key(section(s))).collect();
    nodes.push((
        key(1),
        SourceNode::with_children(
            SourceKind::Frame(frame_input(1, 0.0, 0.0, 1000.0, 1000.0)),
            sections.clone(),
        ),
    ));
    for s in 0..SECTIONS {
        let leaves: Vec<SceneKey> = (0..LEAVES).map(|l| key(leaf(s, l))).collect();
        nodes.push((
            key(section(s)),
            SourceNode::with_children(
                SourceKind::Frame(frame_input(section(s), 0.0, s as f32 * 50.0, 200.0, 40.0)),
                leaves,
            ),
        ));
        for l in 0..LEAVES {
            nodes.push((
                key(leaf(s, l)),
                SourceNode::leaf(SourceKind::Shape(shape_input(
                    leaf(s, l),
                    l as f32 * 10.0,
                    0.5,
                ))),
            ));
        }
    }
    SceneSource::from_nodes(vec![key(1)], nodes).unwrap()
}

fn clock() -> ManualClock {
    ManualClock::new(1_000)
}

fn built() -> IncrementalScene {
    IncrementalScene::build(source(), DocumentRevision(1), &clock())
        .unwrap()
        .0
}

/// 같은 source를 처음부터 build한 snapshot의 semantic hash.
fn fresh_hash(scene: &IncrementalScene) -> crate::snapshot::SceneSemanticHash {
    IncrementalScene::build(
        scene.source().clone(),
        scene.snapshot().document_revision(),
        &clock(),
    )
    .unwrap()
    .0
    .snapshot()
    .semantic_hash()
}

fn find(snapshot: &SceneSnapshot, source: u64) -> Arc<SceneNode> {
    fn walk(node: &Arc<SceneNode>, target: StableId) -> Option<Arc<SceneNode>> {
        if node.source() == target {
            return Some(node.clone());
        }
        node.children().find_map(|child| walk(child, target))
    }
    snapshot
        .roots()
        .find_map(|root| walk(root, id(source)))
        .expect("node")
}

fn upsert_shape(n: u64, input: ShapeInput) -> SourceEdit {
    SourceEdit::upsert(key(n), SourceNode::leaf(SourceKind::Shape(input)))
}

#[test]
fn first_build_evaluates_every_node() {
    let (scene, trace) = IncrementalScene::build(source(), DocumentRevision(1), &clock()).unwrap();
    let total = 1 + SECTIONS * (1 + LEAVES);
    assert_eq!(scene.snapshot().node_count(), total);
    assert_eq!(trace.stats.rebuilt, total);
    assert_eq!(trace.stats.shared, 0);
    assert!(trace.stats.full);
    assert_eq!(scene.snapshot().scene_revision().0, 1);
}

#[test]
fn single_paint_edit_rebuilds_one_node_and_shares_the_rest() {
    // 검증 기준(XI.2.5): 단일 property edit rebuild count 예상 일치.
    let mut scene = built();
    let before = scene.snapshot().clone();
    let target = leaf(2, 3);
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(target), SceneDirtyKind::Paint);
    let trace = scene
        .update(
            DocumentRevision(2),
            &[upsert_shape(target, shape_input(target, 30.0, 0.9))],
            &dirty,
            &clock(),
        )
        .unwrap();
    // leaf 1개 평가, 조상 2개(section, root) path copying, 나머지 22개 공유.
    assert_eq!(trace.stats.rebuilt, 1);
    assert_eq!(trace.stats.copied, 2);
    assert_eq!(trace.stats.renumbered, 0);
    assert_eq!(trace.stats.shared, 22);
    // 다른 section 3개 + 같은 section의 다른 leaf 4개.
    assert_eq!(trace.stats.shared_subtrees, 3 + 4);
    assert_eq!(trace.stats.dirty, [1, 0, 0]);
    let after = scene.snapshot();
    let sharing = after.sharing_with(&before);
    assert_eq!(sharing.new_nodes, 3);
    // 바뀌지 않은 node는 같은 Arc와 같은 paint·clip 참조를 씁니다.
    let (old, new) = (find(&before, leaf(0, 0)), find(after, leaf(0, 0)));
    assert!(Arc::ptr_eq(&old, &new));
    let (old, new) = (find(&before, leaf(2, 2)), find(after, leaf(2, 2)));
    assert!(Arc::ptr_eq(&old, &new));
    assert!(old.paint().ptr_eq(new.paint()));
    assert_eq!(after.resource_refs(), before.resource_refs());
    assert_eq!(after.semantic_hash(), fresh_hash(&scene));
}

#[test]
fn frame_move_rebuilds_its_subtree_because_scope_changed() {
    let mut scene = built();
    let target = section(1);
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(target), SceneDirtyKind::Geometry);
    let children = (0..LEAVES).map(|l| key(leaf(1, l))).collect();
    let trace = scene
        .update(
            DocumentRevision(2),
            &[SourceEdit::upsert(
                key(target),
                SourceNode::with_children(
                    SourceKind::Frame(frame_input(target, 7.0, 50.0, 200.0, 40.0)),
                    children,
                ),
            )],
            &dirty,
            &clock(),
        )
        .unwrap();
    // section + leaf 5개 재평가(world·clip 변경), root path copying.
    assert_eq!(trace.stats.rebuilt, 1 + LEAVES);
    assert_eq!(trace.stats.copied, 1);
    assert_eq!(trace.stats.shared, 3 * (1 + LEAVES));
    assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
}

#[test]
fn paint_edit_on_frame_shares_its_children() {
    let mut scene = built();
    let target = section(1);
    let mut input = frame_input(target, 0.0, 50.0, 200.0, 40.0);
    input.background = Some(Color::new(ColorSpace::Srgb, 0.0, 0.0, 1.0, 1.0).unwrap());
    let children = (0..LEAVES).map(|l| key(leaf(1, l))).collect();
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(target), SceneDirtyKind::Paint);
    let trace = scene
        .update(
            DocumentRevision(2),
            &[SourceEdit::upsert(
                key(target),
                SourceNode::with_children(SourceKind::Frame(input), children),
            )],
            &dirty,
            &clock(),
        )
        .unwrap();
    assert_eq!(trace.stats.rebuilt, 1);
    assert_eq!(trace.stats.shared, LEAVES + 3 * (1 + LEAVES));
}

#[test]
fn subtree_dirty_rebuilds_every_descendant() {
    let mut scene = built();
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(section(3)), SceneDirtyKind::Subtree);
    let trace = scene
        .update(DocumentRevision(2), &[], &dirty, &clock())
        .unwrap();
    assert_eq!(trace.stats.rebuilt, 1 + LEAVES);
    assert_eq!(trace.stats.copied, 1);
    assert_eq!(trace.stats.dirty, [0, 0, 1]);
}

#[test]
fn insert_renumbers_following_nodes_and_keeps_their_paint_shared() {
    let mut scene = built();
    let before = scene.snapshot().clone();
    let new_leaf = 999;
    let mut children: Vec<SceneKey> = (0..LEAVES).map(|l| key(leaf(0, l))).collect();
    children.insert(0, key(new_leaf));
    let trace = scene
        .update(
            DocumentRevision(2),
            &[
                upsert_shape(new_leaf, shape_input(new_leaf, 90.0, 0.1)),
                SourceEdit::upsert(
                    key(section(0)),
                    SourceNode::with_children(
                        SourceKind::Frame(frame_input(section(0), 0.0, 0.0, 200.0, 40.0)),
                        children,
                    ),
                ),
            ],
            &SceneDirtySet::new(),
            &clock(),
        )
        .unwrap();
    // 새 leaf와 section 재평가, root path copying. section 0의 기존 leaf 5개와 뒤 section 3개(18 node)는 z만 바뀝니다.
    assert_eq!(trace.stats.rebuilt, 2);
    assert_eq!(trace.stats.copied, 1);
    assert_eq!(trace.stats.renumbered, LEAVES + 3 * (1 + LEAVES));
    assert_eq!(trace.stats.shared, 0);
    let (old, new) = (
        find(&before, leaf(3, 4)),
        find(scene.snapshot(), leaf(3, 4)),
    );
    assert!(!Arc::ptr_eq(&old, &new));
    assert!(old.paint().ptr_eq(new.paint()));
    assert_eq!(old.content(), new.content());
    assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
    // 지우면 처음과 같은 내용입니다.
    let children: Vec<SceneKey> = (0..LEAVES).map(|l| key(leaf(0, l))).collect();
    scene
        .update(
            DocumentRevision(3),
            &[
                SourceEdit::Remove(key(new_leaf)),
                SourceEdit::upsert(
                    key(section(0)),
                    SourceNode::with_children(
                        SourceKind::Frame(frame_input(section(0), 0.0, 0.0, 200.0, 40.0)),
                        children,
                    ),
                ),
            ],
            &SceneDirtySet::new(),
            &clock(),
        )
        .unwrap();
    assert_eq!(scene.snapshot().semantic_hash(), before.semantic_hash());
}

#[test]
fn invalid_edits_are_rejected_without_changing_the_scene() {
    let mut scene = built();
    let hash = scene.snapshot().semantic_hash();
    // 부모 목록에 없는 새 node(닿지 않음)와 부모 둘.
    let orphan = scene.update(
        DocumentRevision(2),
        &[upsert_shape(5000, shape_input(5000, 0.0, 0.0))],
        &SceneDirtySet::new(),
        &clock(),
    );
    assert!(matches!(
        orphan,
        Err(IncrementalError::Source(SourceError::Unreachable { .. }))
    ));
    let twice = scene.update(
        DocumentRevision(2),
        &[SourceEdit::upsert(
            key(section(1)),
            SourceNode::with_children(
                SourceKind::Frame(frame_input(section(1), 0.0, 50.0, 200.0, 40.0)),
                vec![key(leaf(0, 0))],
            ),
        )],
        &SceneDirtySet::new(),
        &clock(),
    );
    assert!(matches!(
        twice,
        Err(IncrementalError::Source(
            SourceError::MultipleParents { .. }
        ))
    ));
    assert_eq!(scene.snapshot().semantic_hash(), hash);
    assert_eq!(scene.source(), &source());
}

#[test]
fn removing_a_node_its_parent_or_roots_still_list_is_rejected() {
    // 부모 Upsert 없이 지우면 부모 자식 목록에 끊긴 참조가 남습니다. 편집 전으로 되돌려야 합니다.
    let mut scene = built();
    let hash = scene.snapshot().semantic_hash();
    let dangling = scene.update(
        DocumentRevision(2),
        &[SourceEdit::Remove(key(leaf(2, 3)))],
        &SceneDirtySet::new(),
        &clock(),
    );
    assert_eq!(
        dangling,
        Err(IncrementalError::Source(SourceError::MissingNode {
            key: key(leaf(2, 3))
        }))
    );
    // root를 SetRoots 없이 지우는 것도 같습니다.
    let mut lone = SceneSource::from_nodes(
        vec![key(7)],
        [(
            key(7),
            SourceNode::leaf(SourceKind::Shape(shape_input(7, 0.0, 0.5))),
        )],
    )
    .unwrap();
    assert_eq!(
        lone.apply(&[SourceEdit::Remove(key(7))]),
        Err(SourceError::MissingNode { key: key(7) })
    );
    assert_eq!(lone.len(), 1);
    assert_eq!(scene.snapshot().semantic_hash(), hash);
    assert_eq!(scene.source(), &source());
    // 부모를 함께 고치면 받아들입니다.
    let leaves: Vec<SceneKey> = (0..LEAVES)
        .filter(|l| *l != 3)
        .map(|l| key(leaf(2, l)))
        .collect();
    scene
        .update(
            DocumentRevision(2),
            &[
                SourceEdit::Remove(key(leaf(2, 3))),
                SourceEdit::upsert(
                    key(section(2)),
                    SourceNode::with_children(
                        SourceKind::Frame(frame_input(section(2), 0.0, 100.0, 200.0, 40.0)),
                        leaves,
                    ),
                ),
            ],
            &SceneDirtySet::new(),
            &clock(),
        )
        .unwrap();
    assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
}

#[test]
fn fragments_flatten_and_stay_shareable() {
    let mut nodes = vec![(
        key(1),
        SourceNode::with_children(
            SourceKind::Frame(frame_input(1, 0.0, 0.0, 100.0, 100.0)),
            vec![key(2), key(5)],
        ),
    )];
    nodes.push((
        key(2),
        SourceNode::with_children(
            SourceKind::Fragment(FragmentInput {
                source: id(2),
                visible: true,
                locked: false,
            }),
            vec![key(3), key(4)],
        ),
    ));
    for n in [3, 4, 5] {
        nodes.push((
            key(n),
            SourceNode::leaf(SourceKind::Shape(shape_input(n, n as f32, 0.2))),
        ));
    }
    let source = SceneSource::from_nodes(vec![key(1)], nodes).unwrap();
    let (mut scene, _) = IncrementalScene::build(source, DocumentRevision(1), &clock()).unwrap();
    assert_eq!(scene.snapshot().node_count(), 4);
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(5), SceneDirtyKind::Paint);
    let trace = scene
        .update(
            DocumentRevision(2),
            &[upsert_shape(5, shape_input(5, 5.0, 0.7))],
            &dirty,
            &clock(),
        )
        .unwrap();
    // Fragment subtree(scene node 2개)를 그대로 씁니다.
    assert_eq!(trace.stats.rebuilt, 1);
    assert_eq!(trace.stats.shared, 2);
    assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
}

#[test]
fn preview_changes_only_the_scene_revision_and_is_discarded_on_clear() {
    // 검증 기준(XII.3.7): drag 중 document revision 증가 없음, 정의된 preview revision만 사용.
    let mut scene = built();
    let original = scene.snapshot().semantic_hash();
    let document = scene.snapshot().document_revision();
    let target = leaf(1, 1);
    let mut last_scene_revision = scene.snapshot().scene_revision();
    for step in 0..500u32 {
        let mut preview = ScenePreview::new();
        preview.set(
            key(target),
            SourceKind::Shape(shape_input(target, 10.0 + step as f32 * 0.25, 0.5)),
        );
        let trace = scene.set_preview(preview, &clock()).unwrap();
        assert_eq!(scene.snapshot().document_revision(), document);
        assert!(scene.snapshot().scene_revision() > last_scene_revision);
        last_scene_revision = scene.snapshot().scene_revision();
        // preview node 하나만 다시 평가합니다.
        assert_eq!(trace.stats.rebuilt, 1, "step {step}");
    }
    // source는 바뀌지 않았습니다(history·문서와 무관).
    assert_eq!(scene.source(), &source());
    assert_ne!(scene.snapshot().semantic_hash(), original);
    scene.clear_preview(&clock()).unwrap();
    assert_eq!(scene.snapshot().semantic_hash(), original);
    assert_eq!(scene.snapshot().document_revision(), document);
    // 종류가 다른 preview와 없는 node는 거부합니다.
    let mut wrong = ScenePreview::new();
    wrong.set(
        key(target),
        SourceKind::Frame(frame_input(target, 0.0, 0.0, 1.0, 1.0)),
    );
    assert_eq!(
        scene.set_preview(wrong, &clock()),
        Err(IncrementalError::PreviewKindMismatch { key: key(target) })
    );
    let mut missing = ScenePreview::new();
    missing.set(key(4242), SourceKind::Shape(shape_input(4242, 0.0, 0.0)));
    assert_eq!(
        scene.set_preview(missing, &clock()),
        Err(IncrementalError::UnknownPreviewNode { key: key(4242) })
    );
}

#[test]
fn boolean_host_draws_the_result_path_and_absorbs_operands() {
    // 검증 기준(XII.10.30): 결과 Path를 scene builder에 연결.
    let mut host = shape_input(2, 0.0, 0.3);
    host.geometry = square(10.0);
    let boolean = BooleanSource {
        op: BooleanOp::Subtract,
        operands: vec![crate::boolean::BooleanOperand {
            geometry: square(10.0),
            fill_rule: FillRule::NonZero,
            transform: translate(5.0, 5.0),
        }],
    };
    let nodes = vec![
        (
            key(1),
            SourceNode::with_children(
                SourceKind::Frame(frame_input(1, 0.0, 0.0, 100.0, 100.0)),
                vec![key(2)],
            ),
        ),
        (
            key(2),
            SourceNode {
                kind: SourceKind::Shape(host),
                children: Vec::new(),
                boolean: Some(boolean),
            },
        ),
    ];
    let source = SceneSource::from_nodes(vec![key(1)], nodes).unwrap();
    let (scene, trace) = IncrementalScene::build(source, DocumentRevision(1), &clock()).unwrap();
    assert_eq!(trace.stats.boolean_failures, 0);
    // operand는 scene node가 아닙니다: root Frame + host 결과 path.
    assert_eq!(scene.snapshot().node_count(), 2);
    let node = find(scene.snapshot(), 2);
    let SceneContent::Path(path) = node.content() else {
        panic!("path 결과가 아닙니다");
    };
    assert_eq!(path.fill_rule, FillRule::NonZero);
    assert_eq!(signed_area(&path.geometry).abs(), 75.0);
    // host paint로 칠합니다.
    assert_eq!(node.paint().get().layers.len(), 1);
}

#[test]
fn boolean_on_frame_is_rejected() {
    let nodes = vec![(
        key(1),
        SourceNode {
            kind: SourceKind::Frame(frame_input(1, 0.0, 0.0, 1.0, 1.0)),
            children: Vec::new(),
            boolean: Some(BooleanSource {
                op: BooleanOp::Union,
                operands: Vec::new(),
            }),
        },
    )];
    assert_eq!(
        SceneSource::from_nodes(vec![key(1)], nodes),
        Err(SourceError::BooleanNotAllowed { key: key(1) })
    );
}

#[test]
fn path_hosts_keep_their_fill_rule_when_boolean_fails() {
    let mut node = NodeInput::new(id(2), translate(0.0, 0.0));
    node.paint.fills.push(solid(0.0, 0.0, 0.0, 1.0));
    let far = PathGeometry::new(vec![
        PathCommand::MoveTo(p(40_000.0, 0.0)),
        PathCommand::LineTo(p(40_001.0, 0.0)),
        PathCommand::LineTo(p(40_001.0, 1.0)),
        PathCommand::Close,
    ])
    .unwrap();
    let input = PathInput {
        node,
        layout_rect: rect(0.0, 0.0, 1.0, 1.0),
        geometry: far.clone(),
        fill_rule: FillRule::EvenOdd,
    };
    let nodes = vec![(
        key(2),
        SourceNode {
            kind: SourceKind::Path(input),
            children: Vec::new(),
            boolean: Some(BooleanSource {
                op: BooleanOp::Union,
                operands: Vec::new(),
            }),
        },
    )];
    let source = SceneSource::from_nodes(vec![key(2)], nodes).unwrap();
    let (scene, trace) = IncrementalScene::build(source, DocumentRevision(1), &clock()).unwrap();
    assert_eq!(trace.stats.boolean_failures, 1);
    let SceneContent::Path(path) = find(scene.snapshot(), 2).content().clone() else {
        panic!("path");
    };
    assert_eq!(path.geometry, far);
    assert_eq!(path.fill_rule, FillRule::EvenOdd);
}

#[test]
fn virtual_keys_are_deterministic_and_distinct_from_document_ids() {
    // XI.2.4: 가상 subtree node key.
    let chain = [id(100), id(200)];
    let a = SceneKey::virtual_node(&chain, id(7));
    assert_eq!(a, SceneKey::virtual_node(&chain, id(7)));
    assert_ne!(a, SceneKey::virtual_node(&chain, id(8)));
    assert_ne!(a, SceneKey::virtual_node(&[id(100)], id(7)));
    assert_ne!(a, SceneKey::virtual_node(&[id(200), id(100)], id(7)));
    assert_eq!(a.id().version(), 8);
    assert_ne!(a, SceneKey::node(id(7)));
}

#[test]
fn build_trace_has_fixed_numeric_fields() {
    // 검증 기준(XI.2.6): trace schema. 내용·이름 없이 숫자 field만.
    let (mut scene, first) =
        IncrementalScene::build(source(), DocumentRevision(1), &clock()).unwrap();
    assert_eq!(
        first.to_line(),
        "scene.build scene_revision=1 start_ns=1000 end_ns=1000 duration_ns=0 nodes=25 rebuilt=25 \
         copied=0 renumbered=0 shared=0 dirty_paint=0 dirty_geometry=0 dirty_subtree=0 full=1"
    );
    let names: Vec<&str> = first.fields().iter().map(|(name, _)| *name).collect();
    assert_eq!(names, SceneBuildTrace::FIELDS);
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(leaf(0, 0)), SceneDirtyKind::Paint);
    let trace = scene
        .update(DocumentRevision(2), &[], &dirty, &clock())
        .unwrap();
    let fields: BTreeMap<&str, u64> = trace.fields().into_iter().collect();
    assert_eq!(fields["scene_revision"], 2);
    assert_eq!(fields["rebuilt"], 1);
    assert_eq!(fields["dirty_paint"], 1);
    assert_eq!(fields["full"], 0);
}

#[test]
fn random_edit_sequence_matches_full_build() {
    let mut scene = built();
    let mut rng = crate::fixtures::SplitMix64::new(7);
    for revision in 2..40u64 {
        let s = rng.below(SECTIONS);
        let l = rng.below(LEAVES);
        let target = leaf(s, l);
        let mut dirty = SceneDirtySet::new();
        let kind = if rng.below(2) == 0 {
            SceneDirtyKind::Paint
        } else {
            SceneDirtyKind::Geometry
        };
        dirty.insert(id(target), kind);
        let x = rng.range(0.0, 50.0);
        let red = rng.range(0.0, 1.0);
        scene
            .update(
                DocumentRevision(revision),
                &[upsert_shape(target, shape_input(target, x, red))],
                &dirty,
                &clock(),
            )
            .unwrap();
        assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
    }
}

#[test]
fn commit_clears_preview_and_applies_the_confirmed_edit_in_one_build() {
    // XII.3.7: pointer up에서 확정한 command만 문서 revision을 올립니다.
    let mut scene = built();
    let target = leaf(0, 0);
    let mut preview = ScenePreview::new();
    preview.set(
        key(target),
        SourceKind::Shape(shape_input(target, 33.0, 0.5)),
    );
    scene.set_preview(preview, &clock()).unwrap();
    let revision = scene.snapshot().scene_revision();
    let trace = scene
        .commit(
            DocumentRevision(2),
            &[upsert_shape(target, shape_input(target, 33.0, 0.5))],
            &SceneDirtySet::new(),
            &clock(),
        )
        .unwrap();
    assert!(scene.preview().is_empty());
    assert_eq!(scene.snapshot().document_revision(), DocumentRevision(2));
    assert_eq!(scene.snapshot().scene_revision().0, revision.0 + 1);
    assert_eq!(trace.stats.rebuilt, 1);
    assert_eq!(scene.snapshot().semantic_hash(), fresh_hash(&scene));
}

#[test]
fn spatial_index_refresh_matches_a_fresh_index() {
    use crate::spatial::{SceneSpatialIndex, SpatialIndex};
    let mut scene = built();
    let mut index = SceneSpatialIndex::build(scene.snapshot());
    let same = |index: &SceneSpatialIndex, snapshot: &SceneSnapshot| {
        let fresh = SceneSpatialIndex::build(snapshot);
        assert_eq!(index.len(), fresh.len());
        for z in 1..=snapshot.node_count() {
            let z = crate::node::ZOrder(z);
            assert!(Arc::ptr_eq(index.node(z).unwrap(), fresh.node(z).unwrap()));
            assert_eq!(index.index().bounds_of(z), fresh.index().bounds_of(z));
        }
    };
    // paint edit: 바뀐 path(조상 2 + leaf 1)만 갱신합니다.
    let target = leaf(1, 2);
    let mut dirty = SceneDirtySet::new();
    dirty.insert(id(target), SceneDirtyKind::Paint);
    scene
        .update(
            DocumentRevision(2),
            &[upsert_shape(target, shape_input(target, 40.0, 0.1))],
            &dirty,
            &clock(),
        )
        .unwrap();
    assert_eq!(index.refresh(scene.snapshot()), 3);
    same(&index, scene.snapshot());
    // 삭제로 node 수가 줄어도 같습니다.
    let children: Vec<SceneKey> = (1..LEAVES).map(|l| key(leaf(0, l))).collect();
    scene
        .update(
            DocumentRevision(3),
            &[
                SourceEdit::Remove(key(leaf(0, 0))),
                SourceEdit::upsert(
                    key(section(0)),
                    SourceNode::with_children(
                        SourceKind::Frame(frame_input(section(0), 0.0, 0.0, 200.0, 40.0)),
                        children,
                    ),
                ),
            ],
            &SceneDirtySet::new(),
            &clock(),
        )
        .unwrap();
    index.refresh(scene.snapshot());
    same(&index, scene.snapshot());
}
