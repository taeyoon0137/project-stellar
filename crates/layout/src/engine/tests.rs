use super::*;
use crate::input::LayoutNode;
use crate::measure::MeasureCache;
use crate::style::{Display, FlexWrap, LayoutStyle, Position, Size};
use crate::test_support::{finite, fixed, id, key, px, size_px, style};

/// 깊이 `depth` 사슬(root = 0). container는 `auto` 크기이고 마지막 leaf는 10×10입니다.
/// `kind`: 0 = wrap flex(자식 flex-grow·shrink 1), 1 = `width: fit-content` block, 2 = absolute 사슬.
fn chain(depth: usize, kind: u8) -> LayoutInput {
    let mut nodes = Vec::new();
    for n in 0..=depth as u64 {
        let mut s = match kind {
            0 => style(|s| {
                s.display = Display::Flex;
                s.flex_wrap = FlexWrap::Wrap;
            }),
            1 => style(|s| s.width = Size::FitContent),
            _ => style(|s| {
                if n > 0 {
                    s.position = Position::Absolute;
                }
            }),
        };
        if n > 0 && kind == 0 {
            s.flex_grow = finite(1.0);
            s.flex_shrink = finite(1.0);
        }
        if n == depth as u64 {
            s.width = size_px(10.0);
            s.height = size_px(10.0);
        }
        let mut node = LayoutNode::new(id(n + 1)).with_style(s);
        if n < depth as u64 {
            node = node.with_child(id(n + 2), key(1));
        }
        nodes.push(node);
    }
    LayoutInput::new(id(1), nodes)
}

fn on_thread<T: Send>(stack: usize, f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn_scoped(scope, f)
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn max_depth_chain_does_not_overflow_default_thread_stack() {
    const DEFAULT_THREAD_STACK: usize = 2 << 20;
    for kind in [0u8, 1, 2] {
        let input = chain(MAX_LAYOUT_DEPTH, kind);
        assert_eq!(input.tree().unwrap().depth, MAX_LAYOUT_DEPTH);
        let result = on_thread(DEFAULT_THREAD_STACK, || compute_layout(&input)).unwrap();
        assert_eq!(result.boxes().len(), MAX_LAYOUT_DEPTH + 1);
        // 콘텐츠 크기 사슬은 leaf 크기 10×10(absolute 사슬은 root만 콘텐츠와 무관).
        let leaf = result
            .get(id(MAX_LAYOUT_DEPTH as u64 + 1))
            .unwrap()
            .local_rect();
        assert_eq!((leaf.width(), leaf.height()), (px(10), px(10)), "{kind}");
        if kind != 2 {
            let root = result.get(id(1)).unwrap().local_rect();
            assert_eq!((root.width(), root.height()), (px(10), px(10)), "{kind}");
        }
        let direct = on_thread(DEEP_LAYOUT_STACK_BYTES, || {
            compute_validated(&input, input.tree().unwrap(), &DefaultMeasurer, None)
        })
        .unwrap();
        assert_eq!(direct.result.encode(), result.encode());
        let mut cache = MeasureCache::new();
        let cached = on_thread(DEFAULT_THREAD_STACK, || {
            compute_layout_cached(&input, &DefaultMeasurer, &mut cache)
        })
        .unwrap();
        assert_eq!(cached.result, result);
        let again = compute_layout_cached(&input, &DefaultMeasurer, &mut cache).unwrap();
        assert_eq!((again.stats.measured, again.result), (0, result));
    }
}

#[test]
fn inline_depth_chain_runs_on_caller_thread_stack() {
    for kind in [0u8, 1, 2] {
        let input = chain(INLINE_LAYOUT_DEPTH, kind);
        let result = on_thread(1 << 20, || compute_layout(&input)).unwrap();
        assert_eq!(result.boxes().len(), INLINE_LAYOUT_DEPTH + 1);
    }
    assert!(matches!(
        compute_layout(&chain(MAX_LAYOUT_DEPTH + 1, 2)),
        Err(LayoutError::DepthLimitExceeded { .. })
    ));
    assert_eq!(DEEP_LAYOUT_STACK_BYTES, 16 << 20);
}

fn rect(result: &LayoutResult, n: u64) -> (f64, f64, f64, f64) {
    let r = result.get(id(n)).unwrap().local_rect();
    (
        r.x().to_px_f64(),
        r.y().to_px_f64(),
        r.width().to_px_f64(),
        r.height().to_px_f64(),
    )
}

#[test]
fn root_auto_width_is_max_content_and_root_percentages_are_auto() {
    let input = LayoutInput::new(
        id(1),
        vec![
            LayoutNode::new(id(1))
                .with_style(style(|s| {
                    s.width =
                        Size::Percentage(stellar_core_types::value::Percentage::new(50.0).unwrap());
                    s.display = Display::Flex;
                }))
                .with_child(id(2), key(1))
                .with_child(id(3), key(2)),
            LayoutNode::new(id(2)).with_style(fixed(30.0, 10.0)),
            LayoutNode::new(id(3)).with_style(fixed(40.0, 20.0)),
        ],
    );
    let result = compute_layout(&input).unwrap();
    assert_eq!(rect(&result, 1), (0.0, 0.0, 70.0, 20.0));
    assert!(result.diagnostics().is_empty());
}

#[test]
fn cyclic_percentage_is_resolved_like_css_and_reported() {
    // fit-content 부모 안 50% 너비 자식: 부모 크기 계산에서는 자식 백분율을 auto(콘텐츠 60)로 보고, 정해진 부모
    // 너비 60의 50%로 다시 해석합니다(CSS Sizing 3 §5.2.1).
    let input = LayoutInput::new(
        id(1),
        vec![
            LayoutNode::new(id(1))
                .with_style(fixed(400.0, 100.0))
                .with_child(id(2), key(1)),
            LayoutNode::new(id(2))
                .with_style(style(|s| s.width = Size::FitContent))
                .with_child(id(3), key(1)),
            LayoutNode::new(id(3))
                .with_style(style(|s| {
                    s.width =
                        Size::Percentage(stellar_core_types::value::Percentage::new(50.0).unwrap())
                }))
                .with_child(id(4), key(1)),
            LayoutNode::new(id(4)).with_style(fixed(60.0, 10.0)),
        ],
    );
    let result = compute_layout(&input).unwrap();
    assert_eq!(rect(&result, 2).2, 60.0);
    assert_eq!(rect(&result, 3).2, 30.0);
    assert_eq!(
        result.diagnostics(),
        &[LayoutDiagnostic::CyclicPercentage {
            ancestor: id(2),
            node: id(3),
            axis: Axis::Horizontal
        }]
    );
    // 부모 너비가 정해져 있으면 순환이 아니라 진단이 없습니다.
    let mut definite = input.clone();
    definite.nodes[1].style.width = size_px(200.0);
    let result = compute_layout(&definite).unwrap();
    assert_eq!(rect(&result, 3).2, 100.0);
    assert!(result.diagnostics().is_empty());
}

#[test]
fn overflow_bounds_include_relative_and_absolute_descendants() {
    let input = LayoutInput::new(
        id(1),
        vec![
            LayoutNode::new(id(1))
                .with_style(fixed(100.0, 50.0))
                .with_child(id(2), key(1))
                .with_child(id(3), key(2)),
            LayoutNode::new(id(2)).with_style(style(|s| {
                *s = fixed(20.0, 20.0);
                s.position = Position::Relative;
                s.left =
                    crate::style::LengthPercentageAuto::Length(crate::test_support::len(-10.0));
            })),
            LayoutNode::new(id(3)).with_style(style(|s| {
                *s = fixed(30.0, 30.0);
                s.position = Position::Absolute;
                s.left = crate::style::LengthPercentageAuto::Length(crate::test_support::len(90.0));
                s.top = crate::style::LengthPercentageAuto::Length(crate::test_support::len(40.0));
            })),
        ],
    );
    let result = compute_layout(&input).unwrap();
    let overflow = result.get(id(1)).unwrap().overflow_bounds();
    assert_eq!(
        (
            overflow.x(),
            overflow.y(),
            overflow.width(),
            overflow.height()
        ),
        (px(-10), px(0), px(130), px(70))
    );
    // absolute 자식은 부모 auto 크기에 기여하지 않습니다.
    let mut auto = input;
    auto.nodes[0].style = LayoutStyle::default();
    let result = compute_layout(&auto).unwrap();
    assert_eq!(rect(&result, 1), (0.0, 0.0, 20.0, 20.0));
}

// --- grid (IX.11.5) ---

fn grid_input(container: LayoutStyle, items: Vec<LayoutStyle>) -> LayoutInput {
    let mut root = LayoutNode::new(id(1)).with_style(container);
    let mut nodes = Vec::new();
    for (k, item) in items.into_iter().enumerate() {
        let n = k as u64 + 2;
        root = root.with_child(id(n), key(k as u32 + 1));
        nodes.push(LayoutNode::new(id(n)).with_style(item));
    }
    nodes.insert(0, root);
    LayoutInput::new(id(1), nodes)
}

#[test]
fn grid_subset_rejections() {
    use crate::style::{GridLine, ItemAlignment, SelfAlignment};
    let grid = style(|s| s.display = Display::Grid);
    // grid 안 absolute 자식의 명시 line 배치.
    let input = grid_input(
        grid.clone(),
        vec![style(|s| {
            s.position = Position::Absolute;
            s.grid_column_start = GridLine::Line(1);
        })],
    );
    assert!(matches!(
        input.validate(),
        Err(LayoutError::Unsupported {
            property: crate::property::LayoutProperty::GridColumnStart,
            ..
        })
    ));
    // grid item baseline 정렬.
    let input = grid_input(
        grid,
        vec![style(|s| {
            s.align_self = SelfAlignment::Item(ItemAlignment::Baseline)
        })],
    );
    assert!(matches!(
        input.validate(),
        Err(LayoutError::Unsupported {
            property: crate::property::LayoutProperty::AlignSelf,
            ..
        })
    ));
    // 0 line·0 span은 InvalidValue.
    let input = grid_input(
        style(|s| s.display = Display::Grid),
        vec![style(|s| s.grid_row_end = GridLine::Span(0))],
    );
    assert!(matches!(
        input.validate(),
        Err(LayoutError::InvalidValue { .. })
    ));
}

#[test]
fn grid_cell_limit_is_enforced() {
    use crate::style::{GridLine, TrackSize};
    let input = grid_input(
        style(|s| {
            s.display = Display::Grid;
            s.grid_template_columns = vec![TrackSize::length(crate::test_support::len(1.0)); 2048];
        }),
        vec![style(|s| s.grid_row_start = GridLine::Line(1024))],
    );
    assert!(matches!(
        compute_layout(&input),
        Err(LayoutError::LimitExceeded {
            limit: crate::error::LayoutLimit::GridCells,
            ..
        })
    ));
}

#[test]
fn grid_cyclic_percentage_is_reported() {
    use crate::style::TrackSize;
    // fit-content grid 안 50% 너비 item: 열 크기 계산에서는 auto(콘텐츠), 배치에서는 영역의 50%입니다.
    let mut input = grid_input(
        style(|s| {
            s.display = Display::Grid;
            s.width = Size::FitContent;
            s.grid_template_columns = vec![TrackSize::AUTO];
        }),
        vec![style(|s| {
            s.width = Size::Percentage(stellar_core_types::value::Percentage::new(50.0).unwrap())
        })],
    );
    input.nodes[1] = input.nodes[1].clone().with_child(id(9), key(1));
    input
        .nodes
        .push(LayoutNode::new(id(9)).with_style(fixed(60.0, 10.0)));
    let result = compute_layout(&input).unwrap();
    assert_eq!(rect(&result, 1).2, 60.0);
    assert_eq!(rect(&result, 2).2, 30.0);
    assert_eq!(
        result.diagnostics(),
        &[LayoutDiagnostic::CyclicPercentage {
            ancestor: id(1),
            node: id(2),
            axis: Axis::Horizontal
        }]
    );
}
