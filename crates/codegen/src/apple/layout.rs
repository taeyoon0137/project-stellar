//! CSS layout longhand → Apple layout 규칙 (XXII.3.2, ADR-0107 결정 3·4).
//!
//! [`SWIFTUI_RULES`]·[`APPKIT_RULES`]는 layout longhand 40개(`stellar_layout::declared::LONGHANDS`) 각각의 Apple
//! profile 규칙입니다. 이 표가 XXII.1.4 capability matrix의 Apple 열(CSS layout longhand 부분)이며, 생성기는 이
//! 규칙대로만 낮추고 1:1이 아닌 경우를 [`LayoutFinding`]으로 냅니다(ADR-0025 결정 4.2). 상태 어휘는 IX.10.5의
//! Native·Approximated·Unsupported입니다. Apple 상태는 CSS 매핑 상태에서 유도하지 않습니다(결정 1.3).
//!
//! 단위: CSS `px` 1은 Apple point 1입니다.

use stellar_target_profile::capability::CapabilityStatus;

use crate::apple::native::{Axis, Edge};

/// layout 규칙 진단 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutFinding {
    /// CSS longhand.
    pub longhand: &'static str,
    /// 상태(Approximated 또는 Unsupported).
    pub status: CapabilityStatus,
    /// 안정 진단 code.
    pub code: &'static str,
    /// 설명.
    pub reason: &'static str,
}

/// SwiftUI 규칙(longhand, 규칙). 상태가 값에 따라 다르면 규칙 문장이 조건을 적습니다.
pub const SWIFTUI_RULES: [(&str, &str); 40] = [
    ("position", "static·relative는 흐름 item, absolute는 부모 padding box 위 `.overlay(alignment:)` 층. 양쪽 inset이 모두 있거나 모두 auto면 Approximated"),
    ("box-sizing", "border-box는 `.padding` 뒤 `.frame`, content-box는 `.frame` 뒤 `.padding`(Native)"),
    ("top", "relative는 `.offset(y:)`, absolute는 overlay 위쪽 정렬 + `.offset(y:)`. `%`는 Unsupported"),
    ("right", "relative는 `.offset(x: -값)`, absolute는 overlay trailing 정렬 + `.offset(x: -값)`. `%`는 Unsupported"),
    ("bottom", "relative는 `.offset(y: -값)`, absolute는 overlay bottom 정렬 + `.offset(y: -값)`. `%`는 Unsupported"),
    ("left", "relative는 `.offset(x:)`, absolute는 overlay leading 정렬 + `.offset(x:)`. `%`는 Unsupported"),
    ("width", "`<length>`는 `.frame(width:)`, 채움(block·stretch·grow·root)은 `.frame(maxWidth: .infinity)`, max-content는 `.fixedSize(horizontal: true)`, fit-content는 자연 크기(Approximated), `%`·min-content는 Unsupported"),
    ("height", "width와 같은 규칙(세로)"),
    ("min-width", "`<length>`는 `.frame(minWidth:)`, auto는 플랫폼 최소 크기(root에 한 번 Approximated), 그 밖은 Unsupported"),
    ("min-height", "min-width와 같은 규칙(세로)"),
    ("max-width", "`<length>`는 `.frame(maxWidth:)`, none은 출력 없음, 그 밖은 Unsupported"),
    ("max-height", "max-width와 같은 규칙(세로)"),
    ("aspect-ratio", "`.aspectRatio(w / h, contentMode: .fit)`(Approximated)"),
    ("flex-grow", "부모 main 축이 확정이면 main 축 채움. 성장하는 형제가 모두 `flex-basis: 0`·main 크기 auto이고 grow 값이 같으면 Native, 아니면 Approximated(균등 분배). 확정이 아니면 CSS에서도 효과가 없어 출력 없음"),
    ("flex-shrink", "0은 main 축 `.fixedSize`, 1은 플랫폼 압축(root에 한 번 Approximated), 그 밖은 Approximated"),
    ("flex-basis", "grow 0의 `<length>`는 main 크기 `.frame`, auto·content는 출력 없음, `%`는 Unsupported"),
    ("align-self", "부모 stack 정렬과 다르면 cross 축 `.frame(max…: .infinity, alignment:)` slot. column의 baseline은 Approximated"),
    ("justify-self", "grid item은 `.gridCellAnchor`·채움, flex·block item은 CSS에서도 효과 없음(출력 없음)"),
    ("grid-column-start", "모든 item이 명시 line이거나 모두 자동 배치면 `GridRow` 칸 좌표, 섞이면 Unsupported(문서 순서 자동 배치)"),
    ("grid-column-end", "열 span은 `.gridCellColumns(n)`"),
    ("grid-row-start", "grid-column-start와 같은 규칙"),
    ("grid-row-end", "행 span은 Unsupported(span 1로 냄)"),
    ("display", "block은 `VStack(alignment: .leading, spacing: 0)` + 폭 채움, flex는 `HStack`·`VStack`, grid는 `Grid`(iOS·tvOS 16, macOS 13, watchOS 9 미만은 행·열 stack으로 Approximated), none은 출력 없음"),
    ("padding-top", "`.padding(EdgeInsets(top:))`"),
    ("padding-right", "`.padding(EdgeInsets(trailing:))`"),
    ("padding-bottom", "`.padding(EdgeInsets(bottom:))`"),
    ("padding-left", "`.padding(EdgeInsets(leading:))`"),
    ("flex-direction", "row는 `HStack`, column은 `VStack`. reverse는 자식 출력 순서를 뒤집어 읽기 순서도 바뀌므로 Approximated"),
    ("flex-wrap", "nowrap만 Native, wrap·wrap-reverse는 Unsupported(한 줄로 냄)"),
    ("justify-content", "확정 main 크기에서 start·center·end는 `.frame(alignment:)`, space-between은 사이 `Spacer`, space-evenly는 양끝·사이 `Spacer`(gap이 있으면 Approximated), space-around는 Approximated"),
    ("align-content", "nowrap flex에서는 CSS에서도 효과 없음. grid는 start·center·end를 `.frame(alignment:)`, 그 밖은 Approximated"),
    ("align-items", "stack `alignment:`(row: top·center·bottom·firstTextBaseline, column: leading·center·trailing), stretch·normal은 cross 축 채움 + 확정이 아닌 container `.fixedSize`. column의 baseline은 Approximated"),
    ("justify-items", "grid는 `Grid(alignment:)` 가로 성분·채움, flex·block은 효과 없음"),
    ("row-gap", "column flex는 `VStack(spacing:)`, grid는 `verticalSpacing:`, row nowrap flex·block은 효과 없음"),
    ("column-gap", "row flex는 `HStack(spacing:)`, grid는 `horizontalSpacing:`, column nowrap flex·block은 효과 없음"),
    ("grid-template-columns", "`<length>`는 칸 `.frame(width:)`, `minmax(0, fr)` 같은 비율은 칸 채움, 비율이 다르거나 최소 auto인 fr은 Approximated, `%`·섞인 minmax는 Unsupported"),
    ("grid-template-rows", "`<length>`는 칸 `.frame(height:)`, fr은 Approximated, 콘텐츠 track은 자연 크기"),
    ("grid-auto-columns", "암시 열에 grid-template-columns 규칙"),
    ("grid-auto-rows", "암시 행에 grid-template-rows 규칙"),
    ("grid-auto-flow", "row·column 자동 배치 순서를 칸 좌표로(Native)"),
];

/// AppKit 규칙(longhand, 규칙).
pub const APPKIT_RULES: [(&str, &str); 40] = [
    ("position", "absolute는 부모에 `addSubview` + inset 제약, relative inset은 Unsupported(시각 이동 API 없음)"),
    ("box-sizing", "border-box는 크기 제약에 padding 포함, content-box는 padding을 더한 상수(Native)"),
    ("top", "absolute는 `topAnchor` 제약, `%`는 Unsupported"),
    ("right", "absolute는 `trailingAnchor` 제약, `%`는 Unsupported"),
    ("bottom", "absolute는 `bottomAnchor` 제약, `%`는 Unsupported"),
    ("left", "absolute는 `leadingAnchor` 제약, `%`는 Unsupported"),
    ("width", "`<length>`는 `widthAnchor` 상수 제약, 채움은 부모 폭 제약, 그 밖은 Unsupported(자연 크기)"),
    ("height", "width와 같은 규칙(세로)"),
    ("min-width", "`<length>`는 `greaterThanOrEqualToConstant`, auto는 출력 없음, 그 밖은 Unsupported"),
    ("min-height", "min-width와 같은 규칙(세로)"),
    ("max-width", "`<length>`는 `lessThanOrEqualToConstant`, none은 출력 없음, 그 밖은 Unsupported"),
    ("max-height", "max-width와 같은 규칙(세로)"),
    ("aspect-ratio", "`widthAnchor == heightAnchor * 비율` 제약(Native)"),
    ("flex-grow", "성장 item의 hugging priority를 낮춤. 모든 자식이 같은 grow·basis 0이면 `.fillEqually`(Native), 아니면 Approximated"),
    ("flex-shrink", "0은 compression resistance `.required`, 1은 출력 없음, 그 밖은 Approximated"),
    ("flex-basis", "grow 0의 `<length>`는 main 크기 제약, auto·content는 출력 없음, `%`는 Unsupported"),
    ("align-self", "`NSStackView`는 item별 정렬이 없어 부모 정렬과 다르면 Approximated"),
    ("justify-self", "grid item은 `NSGridCell.xPlacement`, flex·block은 효과 없음"),
    ("grid-column-start", "모든 item이 명시 line이거나 모두 자동 배치면 `NSGridView` 칸, 섞이면 Unsupported"),
    ("grid-column-end", "열 span은 `mergeCells(inHorizontalRange:verticalRange:)`"),
    ("grid-row-start", "grid-column-start와 같은 규칙"),
    ("grid-row-end", "행 span은 Unsupported"),
    ("display", "block은 세로 `NSStackView`(.leading, spacing 0) + 폭 제약, flex는 `NSStackView`, grid는 `NSGridView`, none은 출력 없음"),
    ("padding-top", "`NSStackView.edgeInsets.top`, grid·leaf는 감싼 view의 제약"),
    ("padding-right", "`edgeInsets.right`"),
    ("padding-bottom", "`edgeInsets.bottom`"),
    ("padding-left", "`edgeInsets.left`"),
    ("flex-direction", "row는 `.horizontal`, column은 `.vertical`. reverse는 자식 순서를 뒤집어 Approximated"),
    ("flex-wrap", "wrap·wrap-reverse는 Unsupported"),
    ("justify-content", "start·center·end는 `.gravityAreas` gravity, space-between은 `.equalSpacing`, space-around·space-evenly는 `.equalCentering`(Approximated)"),
    ("align-content", "nowrap flex는 효과 없음, grid는 Approximated(시작으로 냄)"),
    ("align-items", "`NSStackView.alignment`(row: top·centerY·bottom·firstBaseline, column: leading·centerX·trailing), stretch는 자식 cross 크기 제약"),
    ("justify-items", "grid는 `NSGridView.xPlacement`(stretch는 `.fill`), flex·block은 효과 없음"),
    ("row-gap", "column `NSStackView.spacing`, grid `rowSpacing`"),
    ("column-gap", "row `NSStackView.spacing`, grid `columnSpacing`"),
    ("grid-template-columns", "`<length>`는 `NSGridColumn.width`, 그 밖은 Approximated(콘텐츠 크기)"),
    ("grid-template-rows", "`<length>`는 `NSGridRow.height`, 그 밖은 Approximated"),
    ("grid-auto-columns", "암시 열에 grid-template-columns 규칙"),
    ("grid-auto-rows", "암시 행에 grid-template-rows 규칙"),
    ("grid-auto-flow", "row·column 자동 배치 순서를 칸 좌표로(Native)"),
];

/// 근사 진단.
pub(crate) const fn approximated(
    longhand: &'static str,
    code: &'static str,
    reason: &'static str,
) -> LayoutFinding {
    LayoutFinding {
        longhand,
        status: CapabilityStatus::Approximated,
        code,
        reason,
    }
}

/// 미지원 진단.
pub(crate) const fn unsupported(
    longhand: &'static str,
    code: &'static str,
    reason: &'static str,
) -> LayoutFinding {
    LayoutFinding {
        longhand,
        status: CapabilityStatus::Unsupported,
        code,
        reason,
    }
}

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
