//! CSS layout longhand → Apple layout 규칙 (XXII.3.2, ADR-0107 결정 3·4).
//!
//! layout longhand 40개와 local transform의 SwiftUI·AppKit 규칙은 XXII.1.4 capability matrix에 등록되어 있습니다
//! (항목 규칙 [`crate::matrix::css::CSS_LOWERINGS`]의 Apple·AppKit 열, 규칙 문장과 값별 진단
//! [`crate::matrix::dialect`], ADR-0121 결정 1·2). 생성기는 그 규칙대로만 낮추고, 1:1이 아닌 값은
//! [`crate::matrix::dialect::finding`]으로 [`LayoutFinding`]을 만듭니다. 상태는 matrix가 정합니다. 이 module은 정렬
//! 상수 이름 같은 Swift 표기 도우미만 둡니다.
//!
//! 단위: CSS `px` 1은 Apple point 1입니다.

use crate::apple::native::{Axis, Edge};

pub use crate::matrix::dialect::LayoutFinding;

/// SwiftUI `Alignment` 상수 이름.
pub(crate) fn swiftui_alignment(horizontal: Edge, vertical: Edge) -> &'static str {
    match (horizontal, vertical) {
        (Edge::Start, Edge::Start) => ".topLeading",
        (Edge::Center, Edge::Start) => ".top",
        (Edge::End, Edge::Start) => ".topTrailing",
        (Edge::Start, Edge::Center) => ".leading",
        (Edge::Center, Edge::Center) => ".center",
        (Edge::End, Edge::Center) => ".trailing",
        (Edge::Start, Edge::End) => ".bottomLeading",
        (Edge::Center, Edge::End) => ".bottom",
        (Edge::End, Edge::End) => ".bottomTrailing",
    }
}

/// SwiftUI stack의 cross 축 정렬 이름(HStack은 세로 정렬, VStack은 가로 정렬).
pub(crate) fn swiftui_stack_alignment(main: Axis, edge: Edge, baseline: bool) -> &'static str {
    match (main, edge) {
        (Axis::Horizontal, _) if baseline => ".firstTextBaseline",
        (Axis::Horizontal, Edge::Start) => ".top",
        (Axis::Horizontal, Edge::Center) => ".center",
        (Axis::Horizontal, Edge::End) => ".bottom",
        (Axis::Vertical, Edge::Start) => ".leading",
        (Axis::Vertical, Edge::Center) => ".center",
        (Axis::Vertical, Edge::End) => ".trailing",
    }
}

/// SwiftUI `UnitPoint` 이름(grid 칸 anchor).
pub(crate) fn swiftui_unit_point(horizontal: Edge, vertical: Edge) -> &'static str {
    swiftui_alignment(horizontal, vertical)
}

/// AppKit `NSStackView` 정렬 attribute 이름.
pub(crate) fn appkit_stack_alignment(main: Axis, edge: Edge, baseline: bool) -> &'static str {
    match (main, edge) {
        (Axis::Horizontal, _) if baseline => ".firstBaseline",
        (Axis::Horizontal, Edge::Start) => ".top",
        (Axis::Horizontal, Edge::Center) => ".centerY",
        (Axis::Horizontal, Edge::End) => ".bottom",
        (Axis::Vertical, Edge::Start) => ".leading",
        (Axis::Vertical, Edge::Center) => ".centerX",
        (Axis::Vertical, Edge::End) => ".trailing",
    }
}
