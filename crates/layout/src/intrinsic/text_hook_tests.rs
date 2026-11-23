//! Text intrinsic·baseline hook 연결 테스트 (X.10.4).

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::engine::{compute_layout_cached, compute_layout_with};
use crate::input::{LayoutContent, LayoutInput, LayoutNode};
use crate::measure::MeasureCache;
use crate::output::LayoutResult;
use crate::style::{Display, FlexDirection, ItemAlignment, LayoutStyle, Size, TrackSize};
use crate::test_support::{finite, fixed, id, key, len, px, style};
use stellar_core_types::value::Insets;

/// 실제 텍스트 엔진처럼 동작하는 hook: 단어 폭은 입력 값, 단어 사이 공백 4, 줄 높이 20, 첫 baseline 15.
struct FakeShaper {
    calls: AtomicU64,
    revision: u64,
}

impl FakeShaper {
    fn new(revision: u64) -> FakeShaper {
        FakeShaper {
            calls: AtomicU64::new(0),
            revision,
        }
    }
}

impl TextLayoutHook for FakeShaper {
    fn min_content_width(
        &self,
        _: StableId,
        text: &TextIntrinsic,
    ) -> Result<LayoutUnit, MeasureError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut widest = LayoutUnit::ZERO;
        for word in &text.words {
            widest = widest.max(LayoutUnit::from_length(*word)?);
        }
        Ok(widest)
    }

    fn layout(
        &self,
        _: StableId,
        text: &TextIntrinsic,
        width: Option<LayoutUnit>,
    ) -> Result<TextLineLayout, MeasureError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let space = px(4);
        let mut lines = 1;
        let mut widest = LayoutUnit::ZERO;
        let mut current: Option<LayoutUnit> = None;
        for word in &text.words {
            let word = LayoutUnit::from_length(*word)?;
            let next = match current {
                None => word,
                Some(w) => w.checked_add(space)?.checked_add(word)?,
            };
            current = match (current, width) {
                (Some(w), Some(limit)) if next > limit => {
                    widest = widest.max(w);
                    lines += 1;
                    Some(word)
                }
                _ => Some(next),
            };
        }
        widest = widest.max(current.unwrap_or_default());
        Ok(TextLineLayout {
            width: widest,
            height: px(20).checked_mul_int(lines)?,
            first_baseline: Some(px(15)),
        })
    }

    fn revision(&self) -> u64 {
        self.revision
    }
}

fn text(words: &[f32]) -> LayoutContent {
    LayoutContent::Measured(MeasuredContent::Text(TextIntrinsic {
        words: words.iter().map(|w| len(*w)).collect(),
        space: len(5.0),
        line_height: len(12.0),
        ascent: len(9.0),
    }))
}

fn layout(nodes: Vec<LayoutNode>, hook: &FakeShaper) -> LayoutResult {
    compute_layout_with(&LayoutInput::new(id(1), nodes), &TextHookMeasurer(hook)).unwrap()
}

fn container(s: LayoutStyle, children: &[u64]) -> LayoutNode {
    children
        .iter()
        .enumerate()
        .fold(LayoutNode::new(id(1)).with_style(s), |node, (k, c)| {
            node.with_child(id(*c), key(k as u32 + 1))
        })
}

fn baseline(result: &LayoutResult, n: u64) -> Option<f64> {
    result.get(id(n)).unwrap().baseline().map(|b| b.to_px_f64())
}

fn size(result: &LayoutResult, n: u64) -> (f64, f64) {
    let r = result.get(id(n)).unwrap().local_rect();
    (r.width().to_px_f64(), r.height().to_px_f64())
}

#[test]
fn text_leaf_uses_hook_widths_heights_and_baseline() {
    let hook = FakeShaper::new(0);
    // fit-content root 안 text: max-content 한 줄(10 + 4 + 20 + 4 + 15 = 53), 높이 20, baseline 15 + padding 3.
    let result = layout(
        vec![
            container(style(|s| s.width = Size::FitContent), &[2]),
            LayoutNode::new(id(2))
                .with_style(style(|s| {
                    s.width = Size::FitContent;
                    s.padding = Insets::new(3.0, 0.0, 0.0, 0.0).unwrap();
                }))
                .with_content(text(&[10.0, 20.0, 15.0])),
        ],
        &hook,
    );
    assert_eq!(size(&result, 2), (53.0, 23.0));
    assert_eq!(baseline(&result, 2), Some(18.0));
    // 기본 구현과 다른 값이라 hook이 실제로 쓰였습니다.
    let default = compute_layout_with(
        &LayoutInput::new(
            id(1),
            vec![
                container(style(|s| s.width = Size::FitContent), &[2]),
                LayoutNode::new(id(2)).with_content(text(&[10.0, 20.0, 15.0])),
            ],
        ),
        &DefaultMeasurer,
    )
    .unwrap();
    assert_eq!(size(&default, 2), (55.0, 12.0));
    // 너비 30에서는 hook 줄바꿈으로 세 줄(높이 60), min-content keyword는 가장 넓은 단어 20.
    let result = layout(
        vec![
            container(fixed(300.0, 200.0), &[2, 3]),
            LayoutNode::new(id(2))
                .with_style(style(|s| s.width = crate::test_support::size_px(30.0)))
                .with_content(text(&[10.0, 20.0, 15.0])),
            LayoutNode::new(id(3))
                .with_style(style(|s| s.width = Size::MinContent))
                .with_content(text(&[10.0, 20.0, 15.0])),
        ],
        &hook,
    );
    assert_eq!(size(&result, 2), (30.0, 60.0));
    assert_eq!(size(&result, 3), (20.0, 60.0));
    assert!(hook.calls.load(Ordering::Relaxed) > 0);
}

#[test]
fn text_baseline_reaches_container_first_baselines() {
    let hook = FakeShaper::new(0);
    let leaf = |n: u64| LayoutNode::new(id(n)).with_content(text(&[10.0]));
    let tall = |n: u64| LayoutNode::new(id(n)).with_style(fixed(10.0, 30.0));
    let flex_row = style(|s| {
        s.display = Display::Flex;
        s.align_items = ItemAlignment::FlexStart;
    });
    // flex row: 첫 item이 text면 그 baseline(15).
    let result = layout(
        vec![container(flex_row.clone(), &[2, 3]), leaf(2), tall(3)],
        &hook,
    );
    assert_eq!(baseline(&result, 1), Some(15.0));
    // flex row: 첫 item에 baseline이 없으면 그 border box 아래 변(30)으로 합성.
    let result = layout(vec![container(flex_row, &[3, 2]), leaf(2), tall(3)], &hook);
    assert_eq!(baseline(&result, 1), Some(30.0));
    // column flex: 첫 item의 baseline.
    let column = style(|s| {
        s.display = Display::Flex;
        s.flex_direction = FlexDirection::Column;
        s.padding = Insets::new(5.0, 0.0, 0.0, 0.0).unwrap();
    });
    let result = layout(vec![container(column, &[2, 3]), leaf(2), tall(3)], &hook);
    assert_eq!(baseline(&result, 1), Some(20.0));
    // block: baseline을 가진 첫 in-flow 자식(높이 30 box 다음 text → 30 + 15).
    let result = layout(
        vec![container(fixed(100.0, 100.0), &[3, 2]), leaf(2), tall(3)],
        &hook,
    );
    assert_eq!(baseline(&result, 1), Some(45.0));
    // grid: 첫 행 첫 item(text) baseline.
    let grid = style(|s| {
        s.display = Display::Grid;
        s.grid_template_columns = vec![TrackSize::fr(finite(1.0)), TrackSize::fr(finite(1.0))];
        s.align_items = ItemAlignment::Start;
    });
    let result = layout(vec![container(grid, &[2, 3]), leaf(2), tall(3)], &hook);
    assert_eq!(baseline(&result, 1), Some(15.0));
    // 중첩: block 안 flex row 안 text → 바깥 block까지 15.
    let result = layout(
        vec![
            container(fixed(100.0, 100.0), &[4]),
            LayoutNode::new(id(4))
                .with_style(style(|s| s.display = Display::Flex))
                .with_child(id(2), key(1)),
            leaf(2),
        ],
        &hook,
    );
    assert_eq!(baseline(&result, 1), Some(15.0));
    assert_eq!(baseline(&result, 4), Some(15.0));
}

#[test]
fn hook_revision_invalidates_cached_text_measurements() {
    let input = LayoutInput::new(
        id(1),
        vec![
            container(style(|s| s.width = Size::FitContent), &[2]),
            LayoutNode::new(id(2)).with_content(text(&[10.0, 20.0])),
        ],
    );
    let mut cache = MeasureCache::new();
    let first = FakeShaper::new(1);
    compute_layout_cached(&input, &TextHookMeasurer(&first), &mut cache).unwrap();
    assert!(first.calls.load(Ordering::Relaxed) > 0);
    // 같은 revision: hook을 다시 부르지 않습니다.
    let same = FakeShaper::new(1);
    let run = compute_layout_cached(&input, &TextHookMeasurer(&same), &mut cache).unwrap();
    assert_eq!(same.calls.load(Ordering::Relaxed), 0);
    assert_eq!(run.stats.measurer_calls, 0);
    // revision이 바뀌면 다시 측정합니다.
    let next = FakeShaper::new(2);
    compute_layout_cached(&input, &TextHookMeasurer(&next), &mut cache).unwrap();
    assert!(next.calls.load(Ordering::Relaxed) > 0);
}
