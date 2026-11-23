use super::*;
use crate::corpus::generate_corpus;
use crate::engine::{compute_layout, compute_layout_with};
use crate::input::{IntrinsicContent, LayoutChild, LayoutContent};
use crate::intrinsic::{DefaultMeasurer, IntrinsicMeasure, IntrinsicSize, MeasureError};
use crate::intrinsic::{MeasureRequest, MeasuredContent};
use crate::style::{
    ContentAlignment, FlexDirection, ItemAlignment, LayoutStyle, LengthPercentageAuto, MaxSize,
    Size,
};
use crate::test_support::{finite, fixed, id, key, len, size_px, style};
use stellar_core_types::value::{Insets, Length, Percentage, Transform2D};

/// SplitMix64(테스트 결정적 난수).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn length(&mut self, max: u32) -> Length {
        len(self.below(max as usize * 4 + 1) as f32 / 4.0)
    }
}

fn sorted_nodes(input: &LayoutInput) -> Vec<LayoutNode> {
    let mut nodes = input.nodes.clone();
    nodes.sort_by_key(|node| node.id);
    nodes
}

fn assert_matches_full(session: &IncrementalLayout, context: &str) {
    let expected = compute_layout(session.input()).unwrap_or_else(|e| panic!("{context}: {e}"));
    let actual = session
        .result()
        .unwrap_or_else(|e| panic!("{context}: {e}"));
    assert_eq!(actual.encode(), expected.encode(), "{context}");
}

fn apply_model(model: &mut LayoutInput, edits: &[LayoutEdit]) {
    for edit in edits {
        match edit {
            LayoutEdit::Upsert(node) => match model.nodes.iter_mut().find(|n| n.id == node.id) {
                Some(slot) => *slot = (**node).clone(),
                None => model.nodes.push((**node).clone()),
            },
            LayoutEdit::Remove(target) => model.nodes.retain(|n| n.id != *target),
        }
    }
}

fn random_size(rng: &mut Rng) -> Size {
    match rng.below(6) {
        0 | 1 => Size::Length(rng.length(120)),
        2 => Size::FitContent,
        3 => Size::Auto,
        4 => Size::MinContent,
        _ => Size::Percentage(Percentage::new(rng.below(100) as f32).unwrap()),
    }
}

fn random_edits(model: &LayoutInput, rng: &mut Rng, next_id: &mut u64) -> Vec<LayoutEdit> {
    let parents: HashMap<StableId, StableId> = model
        .nodes
        .iter()
        .flat_map(|n| n.children.iter().map(move |c| (c.id, n.id)))
        .collect();
    let by_id: HashMap<StableId, &LayoutNode> = model.nodes.iter().map(|n| (n.id, n)).collect();
    let node = &model.nodes[rng.below(model.nodes.len())];
    let mut new = node.clone();
    let mut edits = Vec::new();
    match rng.below(16) {
        0 => new.style.width = random_size(rng),
        1 => new.style.height = random_size(rng),
        2 => {
            let p = rng.length(8).get();
            new.style.padding = Insets::new(p, p / 2.0, p, 0.0).unwrap();
        }
        3 => {
            new.style.display = if new.style.display == Display::None {
                Display::Block
            } else {
                Display::None
            }
        }
        4 => {
            new.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, rng.length(9).get(), 0.0).unwrap();
        }
        5 => {
            new.style.flex_grow = finite(rng.below(3) as f32);
            new.style.flex_shrink = finite(rng.below(3) as f32);
        }
        6 => {
            new.style.position = if rng.below(2) == 0 {
                Position::Absolute
            } else {
                Position::Relative
            };
            new.style.left = LengthPercentageAuto::Length(rng.length(50));
            new.style.top = LengthPercentageAuto::Length(rng.length(50));
        }
        7 => {
            if let LayoutContent::Intrinsic(content) = &mut new.content {
                content.width = rng.length(90);
            } else if new.children.is_empty() {
                new.content = LayoutContent::Intrinsic(IntrinsicContent {
                    width: rng.length(40),
                    height: rng.length(20),
                    baseline: Some(len(8.0)),
                });
            }
        }
        8 => {
            *next_id += 1;
            let child = LayoutNode::new(id(*next_id))
                .with_style(fixed(rng.length(30).get(), rng.length(30).get()));
            if !new.content.is_leaf() {
                new.children.push(LayoutChild {
                    id: child.id,
                    order_key: key(rng.below(5000) as u32),
                });
                edits.push(LayoutEdit::upsert(child));
            }
        }
        9 => {
            let leaves: Vec<StableId> = new
                .children
                .iter()
                .map(|c| c.id)
                .filter(|c| by_id.get(c).is_some_and(|n| n.children.is_empty()))
                .collect();
            if let Some(&leaf) = leaves.get(rng.below(leaves.len().max(1))) {
                new.children.retain(|c| c.id != leaf);
                edits.push(LayoutEdit::Remove(leaf));
            }
        }
        10 => {
            if !new.children.is_empty() {
                let at = rng.below(new.children.len());
                new.children[at].order_key = key(rng.below(5000) as u32);
            }
        }
        11 => {
            let leaf = &model.nodes[rng.below(model.nodes.len())];
            if leaf.id != node.id && leaf.id != model.root && !new.content.is_leaf() {
                if let Some(&old_parent) = parents.get(&leaf.id) {
                    if old_parent != node.id {
                        let mut old = by_id[&old_parent].clone();
                        old.children.retain(|c| c.id != leaf.id);
                        edits.push(LayoutEdit::upsert(old));
                        new.children.push(LayoutChild {
                            id: leaf.id,
                            order_key: key(rng.below(5000) as u32),
                        });
                    }
                }
            }
        }
        12 => {
            new.style.display = match rng.below(3) {
                0 => Display::Block,
                _ => Display::Flex,
            };
            new.style.flex_direction = if rng.below(2) == 0 {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };
            new.style.justify_content = ContentAlignment::Center;
            new.style.align_items = ItemAlignment::FlexEnd;
        }
        13 => {
            let child = if rng.below(2) == 0 {
                id(999_999)
            } else {
                model.root
            };
            new.children.push(LayoutChild {
                id: child,
                order_key: key(1),
            });
        }
        14 => new.style.max_width = MaxSize::Length(rng.length(60)),
        _ => new.style.min_width = Size::Length(rng.length(60)),
    }
    edits.push(LayoutEdit::upsert(new));
    edits
}

#[test]
fn initial_layout_matches_full_compute_for_corpus() {
    for fixture in generate_corpus(20) {
        let (session, stats) = IncrementalLayout::new(fixture.input.clone(), &DefaultMeasurer)
            .unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
        assert_eq!(stats.full_layouts, 1);
        assert_eq!(
            session.result().unwrap(),
            compute_layout(&fixture.input).unwrap(),
            "{}",
            fixture.name
        );
    }
}

#[test]
fn random_edits_match_full_compute_and_reject_invalid_batches() {
    let mut applied = 0;
    let mut rejected = 0;
    let mut reused = 0;
    for (n, fixture) in generate_corpus(16).into_iter().enumerate() {
        let mut rng = Rng(0x5eed + n as u64);
        let mut next_id = 1_000_000;
        let mut model = fixture.input.clone();
        let (mut session, _) =
            IncrementalLayout::new(fixture.input.clone(), &DefaultMeasurer).unwrap();
        for step in 0..24 {
            let context = format!("{} step {step}", fixture.name);
            let edits = random_edits(&model, &mut rng, &mut next_id);
            let mut expected = model.clone();
            apply_model(&mut expected, &edits);
            let valid = expected.validate().is_ok();
            let full = if valid {
                compute_layout(&expected).ok()
            } else {
                None
            };
            let before = session.result().ok();
            let recovering = before.is_none();
            let outcome = session.apply(edits, &DefaultMeasurer);
            if !valid {
                assert!(outcome.is_err(), "{context}: 잘못된 batch를 받아들임");
                assert_eq!(
                    sorted_nodes(session.input()),
                    sorted_nodes(&model),
                    "{context}"
                );
                assert_eq!(session.result().ok(), before, "{context}");
                rejected += 1;
                continue;
            }
            model = expected;
            assert_eq!(
                sorted_nodes(session.input()),
                sorted_nodes(&model),
                "{context}"
            );
            assert_eq!(
                session.tree.depth,
                model.tree().unwrap().depth,
                "{context}: tree 깊이"
            );
            match full {
                Some(full) => {
                    let stats = outcome.unwrap_or_else(|e| panic!("{context}: {e}"));
                    assert_eq!(
                        stats.full_layouts,
                        u64::from(recovering),
                        "{context}: 전체 레이아웃으로 대신함"
                    );
                    assert_eq!(
                        session.result().unwrap().encode(),
                        full.encode(),
                        "{context}"
                    );
                    for node in &model.nodes {
                        let expected = full.get(node.id).copied();
                        assert_eq!(session.layout_box(node.id), expected, "{context}");
                    }
                    reused += stats.reused;
                    applied += 1;
                }
                None => {
                    assert!(outcome.is_err(), "{context}");
                    assert_eq!(session.result(), Err(LayoutError::LayoutUnavailable));
                }
            }
        }
    }
    assert!(applied > 500, "{applied}");
    assert!(rejected > 50, "{rejected}");
    assert!(reused > 0);
}

/// 배치 경계 확인용 문서.
///
/// ```text
/// 1 root 400×300 block
/// ├─ 2 A absolute 200×200 flex column ── 3 B fit-content flex row ── 4 L leaf, 5 S flex(6, 7)
/// │                                    └─ 8 C leaf
/// ├─ 9 F 150×100 block ── 10 M 30×10 leaf, 11 N fit-content flex(12)
/// └─ 13 O auto block ── 14 P leaf
/// ```
fn modes_document() -> LayoutInput {
    let leaf = |n: u64, w: f32| LayoutNode::new(id(n)).with_style(fixed(w, 10.0));
    let flex = |f: &dyn Fn(&mut LayoutStyle)| {
        style(|s| {
            s.display = Display::Flex;
            f(s);
        })
    };
    LayoutInput::new(
        id(1),
        vec![
            LayoutNode::new(id(1))
                .with_style(fixed(400.0, 300.0))
                .with_child(id(2), key(1))
                .with_child(id(9), key(2))
                .with_child(id(13), key(3)),
            LayoutNode::new(id(2))
                .with_style(flex(&|s| {
                    *s = fixed(200.0, 200.0);
                    s.display = Display::Flex;
                    s.flex_direction = FlexDirection::Column;
                    s.position = Position::Absolute;
                    s.left = LengthPercentageAuto::Length(len(150.0));
                }))
                .with_child(id(3), key(1))
                .with_child(id(8), key(2)),
            LayoutNode::new(id(3))
                .with_style(flex(&|s| s.width = Size::FitContent))
                .with_child(id(4), key(1))
                .with_child(id(5), key(2)),
            leaf(4, 10.0),
            LayoutNode::new(id(5))
                .with_style(flex(&|_| {}))
                .with_child(id(6), key(1))
                .with_child(id(7), key(2)),
            leaf(6, 5.0),
            leaf(7, 6.0),
            leaf(8, 20.0),
            LayoutNode::new(id(9))
                .with_style(fixed(150.0, 100.0))
                .with_child(id(10), key(1))
                .with_child(id(11), key(2)),
            leaf(10, 30.0),
            LayoutNode::new(id(11))
                .with_style(flex(&|s| s.width = Size::FitContent))
                .with_child(id(12), key(1)),
            leaf(12, 8.0),
            LayoutNode::new(id(13)).with_child(id(14), key(1)),
            leaf(14, 12.0),
        ],
    )
}

fn edited(session: &IncrementalLayout, n: u64, f: impl FnOnce(&mut LayoutNode)) -> LayoutEdit {
    let mut node = session.node(id(n)).unwrap().clone();
    f(&mut node);
    LayoutEdit::upsert(node)
}

fn ids(list: &[u64]) -> Vec<StableId> {
    list.iter().map(|n| id(*n)).collect()
}

#[test]
fn single_edits_stop_at_layout_boundaries() {
    // (edit node, 기대 방문 node(배치 순서)).
    let cases: Vec<(u64, Vec<u64>)> = vec![
        // flex item leaf 크기: 콘텐츠 크기 row(3)·nested flex는 경계가 아니라 px absolute A(2)에서 멈춥니다.
        (4, vec![2, 3, 4]),
        (6, vec![2, 3, 5, 6]),
        // px block F(9) 안 px leaf: F의 in-flow 자식이라 F가 다시 배치합니다(F는 px block 자식이라 경계).
        (10, vec![9, 10]),
        (12, vec![9, 11, 12]),
        // auto block O(13)는 경계가 아니라 root까지 올라갑니다.
        (14, vec![1, 13, 14]),
    ];
    for (node, expected) in cases {
        let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
        let change = edited(&session, node, |n| n.style.width = size_px(17.0));
        let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
        assert_eq!(
            session.last_arranged(),
            ids(&expected).as_slice(),
            "node {node}"
        );
        assert_eq!(stats.arranged as usize, expected.len(), "node {node}");
        assert_eq!(stats.full_layouts, 0);
        assert_matches_full(&session, &format!("node {node}"));
    }
}

#[test]
fn leaf_transform_change_does_zero_layout() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    let change = edited(&session, 4, |n| {
        n.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 9.0, 0.0).unwrap()
    });
    let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
    assert_eq!(stats.arranged, 0);
    assert_eq!(stats.paint_only, 1);
    assert_eq!(stats.layout_dirty, 0);
    assert_matches_full(&session, "transform");
}

#[test]
fn hidden_subtree_edits_are_skipped_until_shown() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    let hide = edited(&session, 3, |n| n.style.display = Display::None);
    session.apply(vec![hide], &DefaultMeasurer).unwrap();
    assert!(session.layout_box(id(4)).is_none());
    assert_matches_full(&session, "hide");
    let inside = edited(&session, 4, |n| n.style.width = size_px(33.0));
    let stats = session.apply(vec![inside], &DefaultMeasurer).unwrap();
    assert_eq!(stats.arranged, 0);
    assert_matches_full(&session, "hidden edit");
    let show = edited(&session, 3, |n| n.style.display = Display::Flex);
    session.apply(vec![show], &DefaultMeasurer).unwrap();
    assert_eq!(
        session.layout_box(id(4)).unwrap().local_rect().width(),
        crate::test_support::px(33)
    );
    assert_matches_full(&session, "show");
}

#[test]
fn structure_edits_insert_move_and_remove() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    let insert = vec![
        edited(&session, 3, |n| {
            n.children.push(LayoutChild {
                id: id(50),
                order_key: key(3),
            })
        }),
        LayoutEdit::upsert(LayoutNode::new(id(50)).with_style(fixed(7.0, 7.0))),
    ];
    session.apply(insert, &DefaultMeasurer).unwrap();
    assert_matches_full(&session, "insert");
    let moved = vec![
        edited(&session, 3, |n| n.children.retain(|c| c.id != id(50))),
        edited(&session, 9, |n| {
            n.children.push(LayoutChild {
                id: id(50),
                order_key: key(9),
            })
        }),
    ];
    session.apply(moved, &DefaultMeasurer).unwrap();
    assert_matches_full(&session, "move");
    let removed = vec![
        edited(&session, 9, |n| n.children.retain(|c| c.id != id(50))),
        LayoutEdit::Remove(id(50)),
        edited(&session, 5, |n| n.children.retain(|c| c.id != id(6))),
        LayoutEdit::Remove(id(6)),
    ];
    session.apply(removed, &DefaultMeasurer).unwrap();
    assert!(session.node(id(50)).is_none());
    assert_matches_full(&session, "remove");
}

#[test]
fn invalid_batches_leave_the_session_unchanged() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    let before = session.result().unwrap();
    let input = session.input().clone();
    let cases: Vec<Vec<LayoutEdit>> = vec![
        vec![LayoutEdit::Remove(id(1))],
        vec![LayoutEdit::Remove(id(77))],
        vec![edited(&session, 4, |n| n.style.width = size_px(-1.0))],
        vec![edited(&session, 9, |n| {
            n.children.push(LayoutChild {
                id: id(4),
                order_key: key(7),
            })
        })],
        vec![edited(&session, 4, |n| {
            n.children.push(LayoutChild {
                id: id(1),
                order_key: key(7),
            })
        })],
        vec![LayoutEdit::Remove(id(14))],
    ];
    for edits in cases {
        assert!(session.apply(edits, &DefaultMeasurer).is_err());
        assert_eq!(session.result().unwrap(), before);
        assert_eq!(sorted_nodes(session.input()), sorted_nodes(&input));
    }
}

struct Revisioned(u64);

impl crate::intrinsic::ContentMeasurer for Revisioned {
    fn measure(
        &self,
        request: &MeasureRequest,
        content: &MeasuredContent,
    ) -> Result<IntrinsicSize, MeasureError> {
        content.measure(request)
    }

    fn revision(&self) -> u64 {
        self.0
    }
}

#[test]
fn measurer_revision_change_relayouts_everything() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &Revisioned(0)).unwrap();
    let change = edited(&session, 4, |n| n.style.width = size_px(12.0));
    let stats = session.apply(vec![change], &Revisioned(1)).unwrap();
    assert_eq!(stats.full_layouts, 1);
    assert_eq!(stats.arranged as usize, session.input().nodes.len());
    assert_eq!(
        session.result().unwrap(),
        compute_layout_with(session.input(), &Revisioned(1)).unwrap()
    );
}

#[test]
fn deep_chain_updates_on_default_thread_stack() {
    let mut nodes = Vec::new();
    for n in 0..=MAX_LAYOUT_DEPTH as u64 {
        let mut node = LayoutNode::new(id(n + 1)).with_style(style(|s| {
            s.display = Display::Flex;
            s.flex_direction = FlexDirection::Column;
            s.width = Size::FitContent;
        }));
        if n < MAX_LAYOUT_DEPTH as u64 {
            node = node.with_child(id(n + 2), key(1));
        } else {
            node = node.with_style(fixed(10.0, 10.0));
        }
        nodes.push(node);
    }
    let input = LayoutInput::new(id(1), nodes);
    let leaf = MAX_LAYOUT_DEPTH as u64 + 1;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn_scoped(scope, || {
                let (mut session, _) = IncrementalLayout::new(input, &DefaultMeasurer).unwrap();
                let change = edited(&session, leaf, |n| n.style.width = size_px(20.0));
                let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
                assert_eq!(stats.arranged, leaf);
                assert_matches_full(&session, "deep chain");
            })
            .unwrap()
            .join()
            .unwrap();
    });
}

#[test]
fn tree_depth_shrinks_after_deep_subtree_is_removed_or_moved_up() {
    use crate::engine::INLINE_LAYOUT_DEPTH;
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    let shallow = session.tree.depth;
    let expect_depth = |session: &IncrementalLayout, context: &str| {
        assert_eq!(
            session.tree.depth,
            session.input().tree().unwrap().depth,
            "{context}"
        );
    };
    let length = INLINE_LAYOUT_DEPTH as u64 + 8;
    let chain_ids: Vec<u64> = (7000..7000 + length).collect();
    let mut edits = vec![edited(&session, 3, |n| {
        n.children.push(LayoutChild {
            id: id(7000),
            order_key: key(9),
        })
    })];
    for (at, &n) in chain_ids.iter().enumerate() {
        let mut node = LayoutNode::new(id(n));
        match chain_ids.get(at + 1) {
            Some(&next) => node = node.with_child(id(next), key(1)),
            None => node = node.with_style(fixed(4.0, 4.0)),
        }
        edits.push(LayoutEdit::upsert(node));
    }
    session.apply(edits, &DefaultMeasurer).unwrap();
    assert!(session.tree.depth > INLINE_LAYOUT_DEPTH);
    expect_depth(&session, "attach deep chain");
    assert_matches_full(&session, "attach deep chain");
    let half = chain_ids[chain_ids.len() / 2];
    let detach = edited(&session, half - 1, |n| n.children.clear());
    let attach = edited(&session, 1, |n| {
        n.children.push(LayoutChild {
            id: id(half),
            order_key: key(99),
        })
    });
    session
        .apply(vec![detach, attach], &DefaultMeasurer)
        .unwrap();
    expect_depth(&session, "move lower half up");
    assert_matches_full(&session, "move lower half up");
    let mut edits = vec![
        edited(&session, 3, |n| n.children.retain(|c| c.id != id(7000))),
        edited(&session, 1, |n| n.children.retain(|c| c.id != id(half))),
    ];
    edits.extend(chain_ids.iter().map(|&n| LayoutEdit::Remove(id(n))));
    session.apply(edits, &DefaultMeasurer).unwrap();
    assert_eq!(session.tree.depth, shallow);
    expect_depth(&session, "remove chain");
    assert_matches_full(&session, "remove chain");
}

fn single_edit(kind: usize, node: &mut LayoutNode) {
    match kind {
        0 => {
            if let Size::Length(width) = node.style.width {
                node.style.width = size_px(width.get() + 1.0);
            }
        }
        1 => node.style.padding = Insets::new(2.0, 1.0, 0.0, 3.0).unwrap(),
        2 => {
            node.style.display = if node.style.display == Display::None {
                Display::Block
            } else {
                Display::None
            }
        }
        3 => {
            node.style.left = LengthPercentageAuto::Length(len(3.0));
            node.style.top = LengthPercentageAuto::Length(len(1.0));
        }
        4 => node.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 2.0, 2.0).unwrap(),
        5 => node.style.max_height = MaxSize::Length(len(500.0)),
        _ => node.style.flex_grow = finite(1.0),
    }
}

#[test]
fn single_edit_fixtures_never_use_full_layout() {
    let (mut session, initial) =
        IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    assert_eq!(initial.full_layout_reason, Some(FullLayoutReason::Initial));
    let all: Vec<StableId> = session.input().nodes.iter().map(|n| n.id).collect();
    let mut updates = 0;
    for kind in 0..7 {
        for node in &all {
            let change = edited(&session, node.timestamp_ms(), |n| single_edit(kind, n));
            let Ok(stats) = session.apply(vec![change], &DefaultMeasurer) else {
                continue;
            };
            updates += 1;
            assert_eq!(stats.full_layouts, 0, "kind {kind} node {node}");
            assert_eq!(stats.full_layout_reason, None);
            assert_matches_full(&session, &format!("kind {kind} node {node}"));
        }
    }
    let metrics = session.metrics();
    assert_eq!(metrics.full_layouts, 1);
    assert_eq!(metrics.unexpected_fallback, 0);
    assert_eq!(metrics.updates, updates);
    assert!(updates > 80, "{updates}");
    for (n, fixture) in generate_corpus(4).into_iter().enumerate() {
        let mut rng = Rng(0x76 + n as u64);
        let mut next_id = 2_000_000;
        let (mut session, _) =
            IncrementalLayout::new(fixture.input.clone(), &DefaultMeasurer).unwrap();
        for _ in 0..16 {
            let edits = random_edits(session.input(), &mut rng, &mut next_id);
            let _ = session.apply(edits, &DefaultMeasurer);
        }
        let metrics = session.metrics();
        assert_eq!(metrics.unexpected_fallback, 0, "{}", fixture.name);
        assert_eq!(
            metrics.full_layouts,
            1 + metrics.recovery,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn diagnostic_and_recovery_full_layouts_are_counted_by_reason() {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &Revisioned(0)).unwrap();
    let change = edited(&session, 4, |n| n.style.width = size_px(14.0));
    session.apply(vec![change], &Revisioned(0)).unwrap();
    let incremental = session.result().unwrap();
    let stats = session.full_layout_for_diagnostics(&Revisioned(0)).unwrap();
    assert_eq!(stats.full_layout_reason, Some(FullLayoutReason::Diagnostic));
    assert_eq!(session.result().unwrap(), incremental);
    let change = edited(&session, 4, |n| n.style.width = size_px(15.0));
    let stats = session.apply(vec![change], &Revisioned(1)).unwrap();
    assert_eq!(
        stats.full_layout_reason,
        Some(FullLayoutReason::MeasurerChanged)
    );
    // 계산 실패(콘텐츠 크기 row의 너비 합이 레이아웃 수치 범위를 넘음) 뒤 다음 edit는 복구 전체 레이아웃입니다.
    let huge = |w: f32| move |n: &mut LayoutNode| n.style.width = size_px(w);
    let overflow = vec![
        edited(&session, 4, huge(30_000_000.0)),
        edited(&session, 6, huge(30_000_000.0)),
    ];
    assert!(session.apply(overflow, &Revisioned(1)).is_err());
    assert_eq!(session.result(), Err(LayoutError::LayoutUnavailable));
    assert!(session.layout_box(id(1)).is_none());
    let fix = vec![
        edited(&session, 4, huge(10.0)),
        edited(&session, 6, huge(5.0)),
    ];
    let stats = session.apply(fix, &Revisioned(1)).unwrap();
    assert_eq!(stats.full_layout_reason, Some(FullLayoutReason::Recovery));
    assert_eq!(
        session.result().unwrap(),
        compute_layout_with(session.input(), &Revisioned(1)).unwrap()
    );
    let metrics = session.metrics();
    assert_eq!(
        (
            metrics.initial,
            metrics.diagnostic,
            metrics.measurer_changed,
            metrics.recovery,
            metrics.unexpected_fallback
        ),
        (1, 1, 1, 1, 0)
    );
    assert_eq!(metrics.full_layouts, 4);
    assert_eq!(metrics.updates, 4);
}

fn broken_session() -> (IncrementalLayout, LayoutEdit) {
    let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
    session.forget_layout(id(2));
    let change = edited(&session, 4, |n| n.style.width = size_px(21.0));
    (session, change)
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "전체 레이아웃으로 대체")]
fn unexpected_fallback_asserts_in_debug_builds() {
    let (mut session, change) = broken_session();
    let _ = session.apply(vec![change], &DefaultMeasurer);
}

#[test]
#[cfg(not(debug_assertions))]
fn unexpected_fallback_is_counted_in_release_builds() {
    let (mut session, change) = broken_session();
    let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
    assert_eq!(
        stats.full_layout_reason,
        Some(FullLayoutReason::UnexpectedFallback)
    );
    assert_eq!(session.metrics().unexpected_fallback, 1);
    assert_matches_full(&session, "fallback");
}

#[test]
fn unexpected_fallback_metric_is_recorded_before_the_assertion() {
    let (mut session, change) = broken_session();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = session.apply(vec![change], &DefaultMeasurer);
    }));
    assert_eq!(outcome.is_err(), cfg!(debug_assertions));
    assert_eq!(session.metrics().unexpected_fallback, 1);
}
