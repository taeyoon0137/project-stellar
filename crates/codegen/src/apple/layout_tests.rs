//! XXII.3.2 검증: layout 규칙 완전성, 규칙별 진단 fixture, layout fixture golden과 `swiftc -typecheck` compile fixture.

use std::collections::{BTreeMap, BTreeSet};

use stellar_accessibility::node::{AccessibilityNode, AccessibilityRole};
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::tree::AccessibilityTree;
use stellar_core_types::value::{FiniteF32, Transform2D};
use stellar_layout::input::{IntrinsicContent, LayoutContent, LayoutInput, LayoutNode};
use stellar_layout::style::{
    AspectRatio, BoxSizing, ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap,
    GridLine, ItemAlignment, LayoutStyle, LengthPercentageAuto, Position, Size, TrackSize,
};
use stellar_target_profile::builtin;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::overrides::VariantLayer;
use stellar_target_profile::validate::{ViewModel, ViewNode};

use super::ui_tests::{check_golden, ui, with_minimum_os, GOLDEN};
use super::*;
use crate::fixture::{self as fx, key, len, sid, Fixture};

fn css(f: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
    let mut style = LayoutStyle::default();
    f(&mut style);
    style
}

fn text_content(width: f32, height: f32) -> LayoutContent {
    LayoutContent::Intrinsic(IntrinsicContent {
        width: len(width),
        height: len(height),
        baseline: Some(len(height * 0.75)),
    })
}

fn node(id: u8, parent: Option<u8>, element: ElementKind) -> ViewNode {
    ViewNode {
        id: sid(id),
        parent: parent.map(sid),
        element,
        contracts: Vec::new(),
    }
}

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("유한")
}

/// layout 규칙 진단 fixture(XXII.3.2·XXII.4.2 공용).
///
/// ```text
/// 1 Frame(flex column, content-box, 360×640, padding 16, space-between)
/// ├ 2 Frame(flex row wrap, baseline) ─ 3 Text(width 50%), 4 Text(min-width min-content, shrink 0)
/// ├ 5 Frame(flex row-reverse, space-around) ─ 6 Frame(grow 2, basis 0), 7 Frame(grow 1, basis 0)
/// ├ 8 Frame(grid 100px 1fr 2fr / 40px) ─ 9 Text(1 / 1, span 2), 10 Text(자동 배치)
/// ├ 11 Frame(160px, aspect-ratio 16 / 9, relative top 4 left 8, 90° 회전)
/// ├ 12 Frame(display none) ─ 13 Text
/// └ 14 Frame(200×100) ─ 15 Frame(absolute right 8 bottom 8, 20×20), 16 Frame(absolute, inset 모두 auto)
/// ```
pub(crate) fn layout_fixture() -> Fixture {
    let view = ViewModel {
        nodes: vec![
            node(1, None, ElementKind::FRAME),
            node(2, Some(1), ElementKind::FRAME),
            node(3, Some(2), ElementKind::TEXT),
            node(4, Some(2), ElementKind::TEXT),
            node(5, Some(1), ElementKind::FRAME),
            node(6, Some(5), ElementKind::FRAME),
            node(7, Some(5), ElementKind::FRAME),
            node(8, Some(1), ElementKind::FRAME),
            node(9, Some(8), ElementKind::TEXT),
            node(10, Some(8), ElementKind::TEXT),
            node(11, Some(1), ElementKind::FRAME),
            node(12, Some(1), ElementKind::FRAME),
            node(13, Some(12), ElementKind::TEXT),
            node(14, Some(1), ElementKind::FRAME),
            node(15, Some(14), ElementKind::FRAME),
            node(16, Some(14), ElementKind::FRAME),
        ],
        ..ViewModel::default()
    };
    let px = |v: f32| Size::Length(len(v));
    let children = |node: LayoutNode, ids: &[u8]| {
        ids.iter().enumerate().fold(node, |node, (order, child)| {
            node.with_child(sid(*child), key(order as u32 + 1))
        })
    };
    let half = stellar_core_types::value::Percentage::new(50.0).expect("%");
    let nodes = vec![
        children(
            LayoutNode::new(sid(1)).with_style(css(|s| {
                s.display = Display::Flex;
                s.flex_direction = FlexDirection::Column;
                s.box_sizing = BoxSizing::ContentBox;
                s.width = px(360.0);
                s.height = px(640.0);
                s.padding = fx::inset(16.0, 16.0, 16.0, 16.0);
                s.justify_content = ContentAlignment::SpaceBetween;
                s.row_gap = len(8.0);
            })),
            &[2, 5, 8, 11, 12, 14],
        ),
        children(
            LayoutNode::new(sid(2)).with_style(css(|s| {
                s.display = Display::Flex;
                s.flex_wrap = FlexWrap::Wrap;
                s.align_items = ItemAlignment::Baseline;
                s.column_gap = len(4.0);
            })),
            &[3, 4],
        ),
        LayoutNode::new(sid(3))
            .with_style(css(|s| s.width = Size::Percentage(half)))
            .with_content(text_content(20.0, 16.0)),
        LayoutNode::new(sid(4))
            .with_style(css(|s| {
                s.min_width = Size::MinContent;
                s.flex_shrink = FiniteF32::ZERO;
            }))
            .with_content(text_content(20.0, 16.0)),
        children(
            LayoutNode::new(sid(5)).with_style(css(|s| {
                s.display = Display::Flex;
                s.flex_direction = FlexDirection::RowReverse;
                s.justify_content = ContentAlignment::SpaceAround;
                s.height = px(40.0);
            })),
            &[6, 7],
        ),
        LayoutNode::new(sid(6)).with_style(css(|s| {
            s.flex_grow = finite(2.0);
            s.flex_basis = FlexBasis::Length(len(0.0));
        })),
        LayoutNode::new(sid(7)).with_style(css(|s| {
            s.flex_grow = finite(1.0);
            s.flex_basis = FlexBasis::Length(len(0.0));
        })),
        children(
            LayoutNode::new(sid(8)).with_style(css(|s| {
                s.display = Display::Grid;
                s.grid_template_columns = vec![
                    TrackSize::length(len(100.0)),
                    TrackSize::fr(finite(1.0)),
                    TrackSize::fr(finite(2.0)),
                ];
                s.grid_template_rows = vec![TrackSize::length(len(40.0))];
                s.column_gap = len(8.0);
            })),
            &[9, 10],
        ),
        LayoutNode::new(sid(9))
            .with_style(css(|s| {
                s.grid_column_start = GridLine::Line(1);
                s.grid_row_start = GridLine::Line(1);
                s.grid_column_end = GridLine::Span(2);
            }))
            .with_content(text_content(30.0, 16.0)),
        LayoutNode::new(sid(10)).with_content(text_content(30.0, 16.0)),
        LayoutNode::new(sid(11))
            .with_style(css(|s| {
                s.width = px(160.0);
                s.aspect_ratio = AspectRatio::new(16.0, 9.0);
                s.position = Position::Relative;
                s.top = LengthPercentageAuto::Length(len(4.0));
                s.left = LengthPercentageAuto::Length(len(8.0));
            }))
            .with_transform(Transform2D::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0).expect("회전")),
        children(
            LayoutNode::new(sid(12)).with_style(css(|s| s.display = Display::None)),
            &[13],
        ),
        LayoutNode::new(sid(13)).with_content(text_content(10.0, 10.0)),
        children(
            LayoutNode::new(sid(14)).with_style(css(|s| {
                s.width = px(200.0);
                s.height = px(100.0);
            })),
            &[15, 16],
        ),
        LayoutNode::new(sid(15)).with_style(css(|s| {
            s.position = Position::Absolute;
            s.right = LengthPercentageAuto::Length(len(8.0));
            s.bottom = LengthPercentageAuto::Length(len(8.0));
            s.width = px(20.0);
            s.height = px(20.0);
        })),
        LayoutNode::new(sid(16)).with_style(css(|s| {
            s.position = Position::Absolute;
            s.width = px(10.0);
            s.height = px(10.0);
        })),
    ];
    let mut tree = AccessibilityTree::new();
    tree.insert(AccessibilityNode::new(sid(1), AccessibilityRole::Generic))
        .expect("root");
    tree.push_root(sid(1));
    let texts = [
        (3, "가"),
        (4, "나"),
        (9, "넓은 칸"),
        (10, "자동"),
        (13, "숨김"),
    ]
    .into_iter()
    .map(|(id, value)| (sid(id), value.to_string()))
    .collect();
    Fixture {
        view,
        variants: VariantLayer::none(),
        layout: LayoutInput::new(sid(1), nodes),
        accessibility: AccessibilitySnapshot::from_tree(&tree).expect("snapshot"),
        texts,
        states: Vec::new(),
        tokens: Vec::new(),
        token_bindings: BTreeMap::new(),
        css_properties: BTreeMap::new(),
        machines: Vec::new(),
        motions: Vec::new(),
        contexts: Default::default(),
        catalog: builtin::catalog().expect("catalog"),
    }
}

fn layout_codes(export: &AppleExport) -> BTreeSet<(u8, &'static str, &'static str)> {
    export
        .notes
        .iter()
        .filter_map(|note| match (&note.subject, note.kind) {
            (AppleNoteSubject::Node(id), AppleNoteKind::Layout(finding)) => {
                Some((id.to_bytes()[15], finding.longhand, finding.code))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn layout_sources_match_golden_static_fixtures() {
    let layout = layout_fixture();
    for (name, profile, dialect) in [
        (
            "layout_ios_swiftui.swift",
            fx::ios_profile(),
            AppleDialect::SwiftUi,
        ),
        (
            "layout_mac_appkit.swift",
            fx::mac_profile(),
            AppleDialect::AppKit,
        ),
    ] {
        let first = ui(&layout, &profile, dialect);
        assert_eq!(first, ui(&layout, &profile, dialect), "{name} 결정론");
        check_golden(GOLDEN, name, &first.source);
    }
}

#[test]
fn layout_rules_cover_every_longhand() {
    // XXII.1.4 Apple 열 중 CSS layout longhand 부분: 모든 longhand에 SwiftUI·AppKit 규칙이 matrix에 있고, adapter가
    // 따로 정의한 표는 없습니다(ADR-0121 결정 1).
    use crate::matrix::dialect::{lowering, rule_text, Dialect};
    let expected: BTreeSet<&str> = stellar_layout::declared::LONGHANDS
        .iter()
        .copied()
        .collect();
    for dialect in [Dialect::SwiftUi, Dialect::AppKit] {
        for name in &expected {
            let scope = ["node", "frame"]
                .into_iter()
                .find(|scope| rule_text(scope, name, dialect).is_some())
                .unwrap_or_else(|| panic!("{name} 규칙 문장 없음"));
            assert!(lowering(scope, name, dialect).is_some(), "{name}");
        }
    }
}

#[test]
fn layout_findings_follow_swiftui_rules() {
    // 검증 기준(XXII.3.2): 1:1이 아닌 layout 값은 규칙의 상태로 진단하고, 표현 가능한 값은 진단 없이 냅니다.
    let export = ui(&layout_fixture(), &fx::ios_profile(), AppleDialect::SwiftUi);
    let codes = layout_codes(&export);
    let expected: BTreeSet<(u8, &str, &str)> = [
        (1, "flex-shrink", "platform-compression"),
        (2, "flex-wrap", "wrap"),
        (3, "width", "percent-size"),
        (4, "min-width", "intrinsic-limit"),
        (5, "flex-direction", "reverse-reading-order"),
        (6, "flex-grow", "grow-distribution"),
        (7, "flex-grow", "grow-distribution"),
        (8, "grid-column-start", "grid-placement"),
        (8, "grid-template-columns", "unequal-fr"),
        (8, "grid-template-columns", "fr-auto-minimum"),
        (11, "aspect-ratio", "aspect-ratio"),
        (16, "position", "static-position"),
    ]
    .into_iter()
    .collect();
    assert_eq!(codes, expected);
    let source = &export.source;
    // content-box: 크기 frame 뒤 padding. space-between: stack spacing은 Spacer 양옆에도 붙으므로 0이고, row-gap 8은
    // 사이 Spacer의 최소 길이입니다(사이 간격 = gap + 남은 공간 몫, CSS와 같음).
    assert!(source.contains(
        "        .frame(width: 360, height: 640, alignment: .topLeading)\n        .padding(EdgeInsets(top: 16, leading: 16, bottom: 16, trailing: 16))"
    ));
    assert!(source.contains(
        "        VStack(alignment: .leading, spacing: 0) {\n            node00000000000000000000000000000002\n            Spacer(minLength: 8)\n            node00000000000000000000000000000005"
    ));
    // row-reverse는 출력 순서를 뒤집습니다. 성장 item이 남은 공간을 받으므로 space-around Spacer는 내지 않습니다
    // (CSS에서도 분배할 공간이 없음).
    assert!(source.contains(
        "        HStack(alignment: .top, spacing: 0) {\n            node00000000000000000000000000000007\n            node00000000000000000000000000000006\n        }"
    ));
    // relative inset은 offset, 회전은 transformEffect, absolute 오른쪽·아래 inset은 trailing 정렬 overlay.
    assert!(source
        .contains(".transformEffect(CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: 0, ty: 0))"));
    assert!(source.contains("        .offset(x: 8, y: 4)"));
    assert!(source.contains(".overlay(alignment: .bottomTrailing) {"));
    assert!(source.contains("        .offset(x: -8, y: -8)"));
    // display none subtree는 view가 없습니다.
    assert!(!source.contains("node0000000000000000000000000000000c"));
    assert!(!source.contains("node0000000000000000000000000000000d"));
    // grid: 고정 열 폭과 행 높이.
    assert!(
        source.contains("Grid(alignment: .topLeading, horizontalSpacing: 8, verticalSpacing: 0)")
    );
    assert!(source.contains(".frame(width: 100, height: 40, alignment: .topLeading)"));
    // baseline row는 firstTextBaseline, shrink 0은 main 축 fixedSize.
    assert!(source.contains("HStack(alignment: .firstTextBaseline, spacing: 4)"));
    assert!(source.contains(
        "        Text(verbatim: \"나\")\n        .fixedSize(horizontal: true, vertical: false)\n    }"
    ));
}

#[test]
fn layout_findings_follow_appkit_rules() {
    let export = ui(&layout_fixture(), &fx::mac_profile(), AppleDialect::AppKit);
    let codes = layout_codes(&export);
    let expected: BTreeSet<(u8, &str, &str)> = [
        (2, "flex-wrap", "wrap"),
        (3, "width", "intrinsic-size"),
        (4, "min-width", "intrinsic-limit"),
        (5, "flex-direction", "reverse-reading-order"),
        (5, "flex-grow", "grow-distribution"),
        (8, "grid-column-start", "grid-placement"),
        (8, "grid-template-columns", "flexible-track"),
        (11, "position", "relative-offset"),
        (11, "transform", "transform"),
        (16, "position", "static-position"),
    ]
    .into_iter()
    .collect();
    assert_eq!(codes, expected);
    let source = &export.source;
    // content-box 크기 상수에는 padding을 더합니다. aspect-ratio는 제약으로 냅니다.
    assert!(source.contains("view.widthAnchor.constraint(equalToConstant: 392).isActive = true"));
    assert!(source.contains(
        "view.widthAnchor.constraint(equalTo: view.heightAnchor, multiplier: 16 / 9).isActive = true"
    ));
    assert!(source.contains("view.distribution = .equalSpacing"));
    assert!(source.contains(
        "overlay1.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8).isActive = true"
    ));
    assert!(source.contains("grid.column(at: 0).width = 100"));
    assert!(source.contains("grid.row(at: 0).height = 40"));
}

#[test]
fn grid_falls_back_to_stacks_below_minimum_os() {
    let export = ui(
        &layout_fixture(),
        &with_minimum_os(fx::ios_profile(), 15, 0),
        AppleDialect::SwiftUi,
    );
    assert!(!export.source.contains("Grid("));
    assert!(layout_codes(&export).contains(&(8, "display", "grid-unavailable")));
}

#[test]
#[cfg(target_os = "macos")]
fn layout_sources_typecheck_against_apple_sdks() {
    // 검증 기준(XXII.3.2) compile fixture: layout fixture 출력을 SwiftUI(iOS·macOS, Grid가 없는 iOS 15 fallback)와
    // AppKit으로 typecheck합니다.
    use super::ui_tests::{sdk_target, typecheck};
    let layout = layout_fixture();
    let cases = [
        ("layout-ios", fx::ios_profile(), AppleDialect::SwiftUi),
        (
            "layout-ios15",
            with_minimum_os(fx::ios_profile(), 15, 0),
            AppleDialect::SwiftUi,
        ),
        ("layout-mac", fx::mac_profile(), AppleDialect::SwiftUi),
        ("layout-appkit", fx::mac_profile(), AppleDialect::AppKit),
    ];
    for (name, profile, dialect) in cases {
        let (sdk, target) = sdk_target(&profile);
        typecheck(&ui(&layout, &profile, dialect).source, name, sdk, &target);
    }
}
