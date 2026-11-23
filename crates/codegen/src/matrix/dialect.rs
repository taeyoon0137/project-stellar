//! Apple·Android adapter dialect 열: SwiftUI·AppKit·Compose 규칙 문장과 값별 진단 등록 (XXII.1.4, ADR-0121 결정 1·2).
//!
//! XXII.3.2·XXII.4.2 adapter가 따로 두던 layout 규칙 표(`SWIFTUI_RULES`·`APPKIT_RULES`·`COMPOSE_RULES`)를 이 matrix에
//! 등록했습니다. adapter는 규칙을 다시 정의하지 않고 여기서 읽습니다.
//!
//! - 항목 상태·규칙·template은 [`super::css::CSS_LOWERINGS`]의 한 행입니다. SwiftUI는 Apple 열, Compose는 Android 열,
//!   AppKit은 그 행의 `appkit` 열입니다([`lowering`]).
//! - 규칙 문장([`DIALECT_TEXTS`])은 값에 따라 상태가 달라지는 조건을 적습니다. layout longhand 40개와 local transform에
//!   dialect마다 하나씩 있습니다.
//! - 값별 진단([`FINDINGS`])은 adapter가 1:1이 아닌 값을 만났을 때 내는 (항목, code)와 그 상태입니다. adapter는
//!   [`finding`]으로 진단을 만들고 상태를 이 표에서 받습니다. 표에 없는 진단은 debug build에서 panic합니다.

use std::fmt::Write as _;

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::profile::TargetProfileKind;

use super::css::{find, PlatformLowering};

/// adapter dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dialect {
    /// SwiftUI(Apple profile kind의 기준 dialect).
    SwiftUi,
    /// AppKit(macOS).
    AppKit,
    /// Jetpack Compose(Android).
    Compose,
}

impl Dialect {
    /// 모든 dialect.
    pub const ALL: [Dialect; 3] = [Dialect::SwiftUi, Dialect::AppKit, Dialect::Compose];

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Dialect::SwiftUi => "swiftui",
            Dialect::AppKit => "appkit",
            Dialect::Compose => "compose",
        }
    }

    /// dialect가 속한 profile kind.
    pub const fn kind(self) -> TargetProfileKind {
        match self {
            Dialect::SwiftUi | Dialect::AppKit => TargetProfileKind::Apple,
            Dialect::Compose => TargetProfileKind::Android,
        }
    }
}

/// adapter의 값별 진단 하나(1:1이 아닌 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutFinding {
    /// CSS longhand(또는 registry 이름).
    pub longhand: &'static str,
    /// 상태(Approximated 또는 Unsupported). [`FINDINGS`]가 정합니다.
    pub status: CapabilityStatus,
    /// 안정 진단 code.
    pub code: &'static str,
    /// 설명.
    pub reason: &'static str,
}

/// 항목 하나의 dialect별 규칙 문장.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialectText {
    /// registry 범위.
    pub scope: &'static str,
    /// registry 이름.
    pub name: &'static str,
    /// SwiftUI.
    pub swiftui: &'static str,
    /// AppKit.
    pub appkit: &'static str,
    /// Compose.
    pub compose: &'static str,
}

impl DialectText {
    /// dialect의 문장.
    pub const fn of(&self, dialect: Dialect) -> &'static str {
        match dialect {
            Dialect::SwiftUi => self.swiftui,
            Dialect::AppKit => self.appkit,
            Dialect::Compose => self.compose,
        }
    }
}

/// 등록된 값별 진단(dialect, 범위, 이름, code, 상태).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindingRule {
    /// dialect.
    pub dialect: Dialect,
    /// registry 범위.
    pub scope: &'static str,
    /// registry 이름.
    pub name: &'static str,
    /// 진단 code.
    pub code: &'static str,
    /// 상태.
    pub status: CapabilityStatus,
}

const fn text(
    scope: &'static str,
    name: &'static str,
    swiftui: &'static str,
    appkit: &'static str,
    compose: &'static str,
) -> DialectText {
    DialectText {
        scope,
        name,
        swiftui,
        appkit,
        compose,
    }
}

/// layout longhand 40개(`stellar_layout::declared::LONGHANDS` 순서)와 local transform의 dialect별 규칙 문장.
pub const DIALECT_TEXTS: &[DialectText] = &[
    text(
        "node",
        "position",
        "static·relative는 흐름 item, absolute는 부모 padding box 위 `.overlay(alignment:)` 층. 양쪽 inset이 모두 있거나 모두 auto면 Approximated",
        "absolute는 부모에 `addSubview` + inset 제약, relative inset은 Unsupported(시각 이동 API 없음)",
        "static·relative는 흐름 item, absolute는 부모 `Box` 위 `Modifier.align(Alignment.TopStart).offset`. 양쪽 inset이 모두 있거나 모두 auto면 Approximated",
    ),
    text(
        "node",
        "box-sizing",
        "border-box는 `.padding` 뒤 `.frame`, content-box는 `.frame` 뒤 `.padding`(Native)",
        "border-box는 크기 제약에 padding 포함, content-box는 padding을 더한 상수(Native)",
        "border-box는 크기 modifier 뒤 `padding`, content-box는 `padding` 뒤 크기 modifier(Native)",
    ),
    text(
        "node",
        "top",
        "relative는 `.offset(y:)`, absolute는 overlay 위쪽 정렬 + `.offset(y:)`. `%`는 Unsupported",
        "absolute는 `topAnchor` 제약, `%`는 Unsupported",
        "relative는 `Modifier.offset(y =)`, absolute는 위쪽 정렬 + offset. `%`는 Unsupported",
    ),
    text(
        "node",
        "right",
        "relative는 `.offset(x: -값)`, absolute는 overlay trailing 정렬 + `.offset(x: -값)`. `%`는 Unsupported",
        "absolute는 `trailingAnchor` 제약, `%`는 Unsupported",
        "relative는 `offset(x = -값)`, absolute는 `Alignment.*End` 정렬 + offset. `%`는 Unsupported",
    ),
    text(
        "node",
        "bottom",
        "relative는 `.offset(y: -값)`, absolute는 overlay bottom 정렬 + `.offset(y: -값)`. `%`는 Unsupported",
        "absolute는 `bottomAnchor` 제약, `%`는 Unsupported",
        "relative는 `offset(y = -값)`, absolute는 `Alignment.Bottom*` 정렬 + offset. `%`는 Unsupported",
    ),
    text(
        "node",
        "left",
        "relative는 `.offset(x:)`, absolute는 overlay leading 정렬 + `.offset(x:)`. `%`는 Unsupported",
        "absolute는 `leadingAnchor` 제약, `%`는 Unsupported",
        "relative는 `offset(x =)`, absolute는 시작 정렬 + offset. `%`는 Unsupported",
    ),
    text(
        "node",
        "width",
        "`<length>`는 `.frame(width:)`, 채움(block·stretch·grow·root)은 `.frame(maxWidth: .infinity)`, max-content는 `.fixedSize(horizontal: true)`, fit-content는 자연 크기(Approximated), `%`·min-content는 Unsupported",
        "`<length>`는 `widthAnchor` 상수 제약, 채움은 부모 폭 제약, 그 밖은 Unsupported(자연 크기)",
        "`<length>`는 `width(dp)`, 채움은 `fillMaxWidth()`, min-content·max-content는 `width(IntrinsicSize.Min|Max)`, `%`는 `fillMaxWidth(비율)`(Approximated), fit-content는 자연 크기(Approximated)",
    ),
    text(
        "node",
        "height",
        "width와 같은 규칙(세로)",
        "width와 같은 규칙(세로)",
        "width와 같은 규칙(세로)",
    ),
    text(
        "node",
        "min-width",
        "`<length>`는 `.frame(minWidth:)`, auto는 플랫폼 최소 크기(root에 한 번 Approximated), 그 밖은 Unsupported",
        "`<length>`는 `greaterThanOrEqualToConstant`, auto는 출력 없음, 그 밖은 Unsupported",
        "`<length>`는 `widthIn(min =)`, auto는 플랫폼 최소 크기(root에 한 번 Approximated), 그 밖은 Unsupported",
    ),
    text(
        "node",
        "min-height",
        "min-width와 같은 규칙(세로)",
        "min-width와 같은 규칙(세로)",
        "min-width와 같은 규칙(세로)",
    ),
    text(
        "node",
        "max-width",
        "`<length>`는 `.frame(maxWidth:)`, none은 출력 없음, 그 밖은 Unsupported",
        "`<length>`는 `lessThanOrEqualToConstant`, none은 출력 없음, 그 밖은 Unsupported",
        "`<length>`는 `widthIn(max =)`, none은 출력 없음, 그 밖은 Unsupported",
    ),
    text(
        "node",
        "max-height",
        "max-width와 같은 규칙(세로)",
        "max-width와 같은 규칙(세로)",
        "max-width와 같은 규칙(세로)",
    ),
    text(
        "node",
        "aspect-ratio",
        "`.aspectRatio(w / h, contentMode: .fit)`(Approximated)",
        "`widthAnchor == heightAnchor * 비율` 제약(Native)",
        "`Modifier.aspectRatio(w / h)`(Approximated: 제약 안 맞춤)",
    ),
    text(
        "node",
        "flex-grow",
        "부모 main 축이 확정이면 main 축 채움. 성장하는 형제가 모두 `flex-basis: 0`·main 크기 auto이고 grow 값이 같으면 Native, 아니면 Approximated(균등 분배). 확정이 아니면 CSS에서도 효과가 없어 출력 없음",
        "성장 item의 hugging priority를 낮춤. 모든 자식이 같은 grow·basis 0이면 `.fillEqually`(Native), 아니면 Approximated",
        "`Modifier.weight(grow)`. 성장 형제가 둘 이상이면 모두 `flex-basis: 0`·main 크기 auto일 때 Native, 아니면 Approximated(콘텐츠 크기를 빼지 않고 비율 분배). 부모 main 축이 확정이 아니면 효과 없음",
    ),
    text(
        "node",
        "flex-shrink",
        "0은 main 축 `.fixedSize`, 1은 플랫폼 압축(root에 한 번 Approximated), 그 밖은 Approximated",
        "0은 compression resistance `.required`, 1은 출력 없음, 그 밖은 Approximated",
        "1은 순차 측정 압축(root에 한 번 Approximated), 그 밖은 Approximated",
    ),
    text(
        "node",
        "flex-basis",
        "grow 0의 `<length>`는 main 크기 `.frame`, auto·content는 출력 없음, `%`는 Unsupported",
        "grow 0의 `<length>`는 main 크기 제약, auto·content는 출력 없음, `%`는 Unsupported",
        "grow 0의 `<length>`는 main 크기 modifier, auto·content는 출력 없음, `%`는 Unsupported",
    ),
    text(
        "node",
        "align-self",
        "부모 stack 정렬과 다르면 cross 축 `.frame(max…: .infinity, alignment:)` slot. column의 baseline은 Approximated",
        "`NSStackView`는 item별 정렬이 없어 부모 정렬과 다르면 Approximated",
        "`Modifier.align(Alignment.*)`, row의 baseline은 `alignByBaseline()`, column의 baseline은 Approximated",
    ),
    text(
        "node",
        "justify-self",
        "grid item은 `.gridCellAnchor`·채움, flex·block item은 CSS에서도 효과 없음(출력 없음)",
        "grid item은 `NSGridCell.xPlacement`, flex·block은 효과 없음",
        "grid 칸 `Box(contentAlignment =)`, flex·block은 효과 없음",
    ),
    text(
        "node",
        "grid-column-start",
        "모든 item이 명시 line이거나 모두 자동 배치면 `GridRow` 칸 좌표, 섞이면 Unsupported(문서 순서 자동 배치)",
        "모든 item이 명시 line이거나 모두 자동 배치면 `NSGridView` 칸, 섞이면 Unsupported",
        "모든 item이 명시 line이거나 모두 자동 배치면 행 `Row`의 칸, 섞이면 Unsupported(문서 순서 자동 배치)",
    ),
    text(
        "node",
        "grid-column-end",
        "열 span은 `.gridCellColumns(n)`",
        "열 span은 `mergeCells(inHorizontalRange:verticalRange:)`",
        "열 span은 span한 track 합(고정 폭 + gap 또는 weight 합). gap이 있는 fr span은 Approximated",
    ),
    text(
        "node",
        "grid-row-start",
        "grid-column-start와 같은 규칙",
        "grid-column-start와 같은 규칙",
        "grid-column-start와 같은 규칙",
    ),
    text(
        "node",
        "grid-row-end",
        "행 span은 Unsupported(span 1로 냄)",
        "행 span은 Unsupported",
        "행 span은 Unsupported(span 1로 냄)",
    ),
    text(
        "frame",
        "display",
        "block은 `VStack(alignment: .leading, spacing: 0)` + 폭 채움, flex는 `HStack`·`VStack`, grid는 `Grid`(iOS·tvOS 16, macOS 13, watchOS 9 미만은 행·열 stack으로 Approximated), none은 출력 없음",
        "block은 세로 `NSStackView`(.leading, spacing 0) + 폭 제약, flex는 `NSStackView`, grid는 `NSGridView`, none은 출력 없음",
        "block은 `Column` + 폭 채움, flex는 `Row`·`Column`(wrap은 `FlowRow`·`FlowColumn`), grid는 행 `Column` 안 칸 `Row`, none은 출력 없음",
    ),
    text(
        "frame",
        "padding-top",
        "`.padding(EdgeInsets(top:))`",
        "`NSStackView.edgeInsets.top`, grid·leaf는 감싼 view의 제약",
        "`Modifier.padding(top =)`",
    ),
    text(
        "frame",
        "padding-right",
        "`.padding(EdgeInsets(trailing:))`",
        "`edgeInsets.right`",
        "`Modifier.padding(end =)`",
    ),
    text(
        "frame",
        "padding-bottom",
        "`.padding(EdgeInsets(bottom:))`",
        "`edgeInsets.bottom`",
        "`Modifier.padding(bottom =)`",
    ),
    text(
        "frame",
        "padding-left",
        "`.padding(EdgeInsets(leading:))`",
        "`edgeInsets.left`",
        "`Modifier.padding(start =)`",
    ),
    text(
        "frame",
        "flex-direction",
        "row는 `HStack`, column은 `VStack`. reverse는 자식 출력 순서를 뒤집어 읽기 순서도 바뀌므로 Approximated",
        "row는 `.horizontal`, column은 `.vertical`. reverse는 자식 순서를 뒤집어 Approximated",
        "row는 `Row`, column은 `Column`. reverse는 자식 출력 순서를 뒤집어 접근성 순회 순서도 바뀌므로 Approximated",
    ),
    text(
        "frame",
        "flex-wrap",
        "nowrap만 Native, wrap·wrap-reverse는 Unsupported(한 줄로 냄)",
        "wrap·wrap-reverse는 Unsupported",
        "wrap은 `FlowRow`·`FlowColumn`(Native), wrap-reverse는 Unsupported(wrap으로 냄)",
    ),
    text(
        "frame",
        "justify-content",
        "확정 main 크기에서 start·center·end는 `.frame(alignment:)`, space-between은 사이 `Spacer(minLength: gap)` + stack spacing 0(Native), space-evenly는 양끝·사이 `Spacer`(gap이 있으면 Approximated), space-around는 Approximated",
        "start·center·end는 `.gravityAreas` gravity, space-between은 `.equalSpacing`, space-around·space-evenly는 `.equalCentering`(Approximated)",
        "`Arrangement.Start|Center|End`(gap이 있으면 `spacedBy(gap, alignment)`), space-between·around·evenly는 `Arrangement.Space*`(gap이 있으면 Approximated)",
    ),
    text(
        "frame",
        "align-content",
        "nowrap flex에서는 CSS에서도 효과 없음. grid는 start·center·end를 `.frame(alignment:)`, 그 밖은 Approximated",
        "nowrap flex는 효과 없음, grid는 Approximated(시작으로 냄)",
        "nowrap flex는 효과 없음, wrap은 Flow의 cross arrangement, grid는 start·center·end를 행 묶음 arrangement로",
    ),
    text(
        "frame",
        "align-items",
        "stack `alignment:`(row: top·center·bottom·firstTextBaseline, column: leading·center·trailing), stretch·normal은 cross 축 채움 + 확정이 아닌 container `.fixedSize`. column의 baseline은 Approximated",
        "`NSStackView.alignment`(row: top·centerY·bottom·firstBaseline, column: leading·centerX·trailing), stretch는 자식 cross 크기 제약",
        "`verticalAlignment`·`horizontalAlignment`, stretch·normal은 자식 `fillMaxHeight()`·`fillMaxWidth()` + 확정이 아닌 container `height|width(IntrinsicSize.Max)`, row baseline은 자식 `alignByBaseline()`",
    ),
    text(
        "frame",
        "justify-items",
        "grid는 `Grid(alignment:)` 가로 성분·채움, flex·block은 효과 없음",
        "grid는 `NSGridView.xPlacement`(stretch는 `.fill`), flex·block은 효과 없음",
        "grid 칸 `Box(contentAlignment =)` 가로 성분·채움, flex·block은 효과 없음",
    ),
    text(
        "frame",
        "row-gap",
        "column flex는 `VStack(spacing:)`, grid는 `verticalSpacing:`, row nowrap flex·block은 효과 없음",
        "column `NSStackView.spacing`, grid `rowSpacing`",
        "column `Arrangement.spacedBy`, Flow·grid의 행 간격",
    ),
    text(
        "frame",
        "column-gap",
        "row flex는 `HStack(spacing:)`, grid는 `horizontalSpacing:`, column nowrap flex·block은 효과 없음",
        "row `NSStackView.spacing`, grid `columnSpacing`",
        "row `Arrangement.spacedBy`, Flow·grid의 열 간격",
    ),
    text(
        "frame",
        "grid-template-columns",
        "`<length>`는 칸 `.frame(width:)`, `minmax(0, fr)` 같은 비율은 칸 채움, 비율이 다르거나 최소 auto인 fr은 Approximated, `%`·섞인 minmax는 Unsupported",
        "`<length>`는 `NSGridColumn.width`, 그 밖은 Approximated(콘텐츠 크기)",
        "`<length>`는 칸 `width(dp)`, `minmax(0, fr)`는 `weight(fr)`(비율 그대로), 최소 auto fr은 Approximated, 콘텐츠 track은 행마다 폭이 달라 Approximated, `%`·섞인 minmax는 Unsupported",
    ),
    text(
        "frame",
        "grid-template-rows",
        "`<length>`는 칸 `.frame(height:)`, fr은 Approximated, 콘텐츠 track은 자연 크기",
        "`<length>`는 `NSGridRow.height`, 그 밖은 Approximated",
        "`<length>`는 행 `height(dp)`, fr은 Approximated, 콘텐츠 track은 자연 크기",
    ),
    text(
        "frame",
        "grid-auto-columns",
        "암시 열에 grid-template-columns 규칙",
        "암시 열에 grid-template-columns 규칙",
        "암시 열에 grid-template-columns 규칙",
    ),
    text(
        "frame",
        "grid-auto-rows",
        "암시 행에 grid-template-rows 규칙",
        "암시 행에 grid-template-rows 규칙",
        "암시 행에 grid-template-rows 규칙",
    ),
    text(
        "frame",
        "grid-auto-flow",
        "row·column 자동 배치 순서를 칸 좌표로(Native)",
        "row·column 자동 배치 순서를 칸 좌표로(Native)",
        "row·column 자동 배치 순서를 칸 좌표로(Native)",
    ),
    text(
        "frame",
        "transform",
        "layout 뒤 적용, 원점 좌상단. 이동은 `.offset`, 선형 부분은 `.transformEffect`(Native)",
        "시각 이동 API가 없어 Unsupported(layout 위치로만 냄)",
        "이동은 `Modifier.offset`(Native), 선형 부분은 `drawWithContent` 그리기 변환. hit test가 바뀌지 않아 Approximated",
    ),
];

const A: CapabilityStatus = CapabilityStatus::Approximated;
const U: CapabilityStatus = CapabilityStatus::Unsupported;

const fn f(
    dialect: Dialect,
    scope: &'static str,
    name: &'static str,
    code: &'static str,
    status: CapabilityStatus,
) -> FindingRule {
    FindingRule {
        dialect,
        scope,
        name,
        code,
        status,
    }
}

use Dialect::{AppKit as K, Compose as C, SwiftUi as S};

/// adapter가 낼 수 있는 값별 진단 전체(dialect, 항목, code 순).
pub const FINDINGS: &[FindingRule] = &[
    // SwiftUI
    f(S, "node", "position", "absolute-stretch", A),
    f(S, "node", "position", "static-position", A),
    f(S, "node", "top", "percent-inset", U),
    f(S, "node", "right", "percent-inset", U),
    f(S, "node", "bottom", "percent-inset", U),
    f(S, "node", "left", "percent-inset", U),
    f(S, "node", "width", "fit-content", A),
    f(S, "node", "width", "min-content", U),
    f(S, "node", "width", "percent-size", U),
    f(S, "node", "height", "fit-content", A),
    f(S, "node", "height", "min-content", U),
    f(S, "node", "height", "percent-size", U),
    f(S, "node", "min-width", "intrinsic-limit", U),
    f(S, "node", "min-height", "intrinsic-limit", U),
    f(S, "node", "max-width", "intrinsic-limit", U),
    f(S, "node", "max-height", "intrinsic-limit", U),
    f(S, "node", "aspect-ratio", "aspect-ratio", A),
    f(S, "node", "flex-grow", "grow-distribution", A),
    f(S, "node", "flex-shrink", "platform-compression", A),
    f(S, "node", "flex-shrink", "shrink-ratio", A),
    f(S, "node", "flex-basis", "percent-size", U),
    f(S, "node", "align-self", "column-baseline", A),
    f(S, "node", "grid-column-start", "grid-placement", U),
    f(S, "node", "grid-column-end", "grid-span-unavailable", A),
    f(S, "frame", "display", "grid-unavailable", A),
    f(S, "frame", "flex-direction", "reverse-reading-order", A),
    f(S, "frame", "flex-wrap", "wrap", U),
    f(S, "frame", "justify-content", "space-around", A),
    f(S, "frame", "justify-content", "spread-gap", A),
    f(S, "frame", "align-items", "column-baseline", A),
    f(S, "frame", "justify-items", "stretch-content-track", A),
    f(S, "frame", "grid-template-columns", "fr-auto-minimum", A),
    f(S, "frame", "grid-template-columns", "track-size", U),
    f(S, "frame", "grid-template-columns", "unequal-fr", A),
    f(S, "frame", "grid-template-rows", "row-track", A),
    // AppKit
    f(K, "node", "position", "relative-offset", U),
    f(K, "node", "position", "static-position", A),
    f(K, "node", "top", "percent-inset", U),
    f(K, "node", "right", "percent-inset", U),
    f(K, "node", "bottom", "percent-inset", U),
    f(K, "node", "left", "percent-inset", U),
    f(K, "node", "width", "fit-content", A),
    f(K, "node", "width", "intrinsic-size", U),
    f(K, "node", "height", "fit-content", A),
    f(K, "node", "height", "intrinsic-size", U),
    f(K, "node", "min-width", "intrinsic-limit", U),
    f(K, "node", "min-height", "intrinsic-limit", U),
    f(K, "node", "max-width", "intrinsic-limit", U),
    f(K, "node", "max-height", "intrinsic-limit", U),
    f(K, "node", "flex-grow", "grow-distribution", A),
    f(K, "node", "flex-shrink", "shrink-ratio", A),
    f(K, "node", "flex-basis", "percent-size", U),
    f(K, "node", "align-self", "align-self", A),
    f(K, "node", "grid-column-start", "grid-placement", U),
    f(K, "frame", "flex-direction", "reverse-reading-order", A),
    f(K, "frame", "flex-wrap", "wrap", U),
    f(K, "frame", "justify-content", "spread", A),
    f(K, "frame", "align-content", "grid-content-alignment", A),
    f(K, "frame", "grid-template-columns", "flexible-track", A),
    f(K, "frame", "transform", "transform", U),
    // Compose
    f(C, "node", "position", "absolute-in-natural-box", A),
    f(C, "node", "position", "absolute-stretch", A),
    f(C, "node", "position", "static-position", A),
    f(C, "node", "top", "percent-inset", U),
    f(C, "node", "right", "percent-inset", U),
    f(C, "node", "bottom", "percent-inset", U),
    f(C, "node", "left", "percent-inset", U),
    f(C, "node", "width", "fit-content", A),
    f(C, "node", "width", "percent-size", A),
    f(C, "node", "height", "fit-content", A),
    f(C, "node", "height", "percent-size", A),
    f(C, "node", "min-width", "intrinsic-limit", U),
    f(C, "node", "min-height", "intrinsic-limit", U),
    f(C, "node", "max-width", "intrinsic-limit", U),
    f(C, "node", "max-height", "intrinsic-limit", U),
    f(C, "node", "aspect-ratio", "aspect-ratio", A),
    f(C, "node", "flex-grow", "grow-distribution", A),
    f(C, "node", "flex-shrink", "platform-compression", A),
    f(C, "node", "flex-shrink", "shrink-ratio", A),
    f(C, "node", "flex-basis", "percent-size", U),
    f(C, "node", "align-self", "column-baseline", A),
    f(C, "node", "grid-column-start", "grid-placement", U),
    f(C, "node", "grid-column-end", "grid-span-gap", A),
    f(C, "node", "grid-column-end", "grid-span-mixed", A),
    f(C, "frame", "flex-direction", "reverse-reading-order", A),
    f(C, "frame", "flex-wrap", "wrap-reverse", U),
    f(C, "frame", "flex-wrap", "wrap-unavailable", U),
    f(C, "frame", "justify-content", "spread-gap", A),
    f(C, "frame", "align-items", "column-baseline", A),
    f(C, "frame", "grid-template-columns", "content-track", A),
    f(C, "frame", "grid-template-columns", "fr-auto-minimum", A),
    f(C, "frame", "grid-template-columns", "track-size", U),
    f(C, "frame", "grid-template-rows", "row-track", A),
    f(C, "frame", "transform", "draw-transform", A),
];

/// (범위, 이름)의 dialect 규칙. SwiftUI는 Apple 열, Compose는 Android 열, AppKit은 `appkit` 열입니다. AppKit 열이 없는
/// 항목은 AppKit dialect에 등록되지 않았습니다(`None`).
pub fn lowering(scope: &str, name: &str, dialect: Dialect) -> Option<PlatformLowering> {
    let row = find(scope, name)?;
    match dialect {
        Dialect::SwiftUi => Some(row.apple),
        Dialect::AppKit => row.appkit,
        Dialect::Compose => Some(row.android),
    }
}

/// (범위, 이름)의 dialect 규칙 문장.
pub fn rule_text(scope: &str, name: &str, dialect: Dialect) -> Option<&'static str> {
    DIALECT_TEXTS
        .iter()
        .find(|t| t.scope == scope && t.name == name)
        .map(|t| t.of(dialect))
}

/// 등록된 값별 진단. adapter는 longhand 이름만 알므로 이름과 code로 찾습니다(한 dialect 안에서 이름·code 쌍은
/// 범위 하나에만 있습니다, test로 확인).
pub fn registered(dialect: Dialect, name: &str, code: &str) -> Option<&'static FindingRule> {
    FINDINGS
        .iter()
        .find(|r| r.dialect == dialect && r.name == name && r.code == code)
}

/// adapter 값별 진단을 만듭니다. 상태는 [`FINDINGS`]가 정합니다. 등록되지 않은 진단은 debug build에서 panic하고
/// release build에서는 Unsupported로 냅니다(표현 가능성을 과장하지 않음).
pub fn finding(
    dialect: Dialect,
    longhand: &'static str,
    code: &'static str,
    reason: &'static str,
) -> LayoutFinding {
    let status = match registered(dialect, longhand, code) {
        Some(rule) => rule.status,
        None => {
            debug_assert!(
                false,
                "{} dialect에 등록되지 않은 진단: {longhand} {code}",
                dialect.name()
            );
            CapabilityStatus::Unsupported
        }
    };
    LayoutFinding {
        longhand,
        status,
        code,
        reason,
    }
}

/// dialect 열의 결정론 텍스트(항목 순서, dialect 순서): 상태·규칙·template·문장·값별 진단.
pub fn dialect_matrix_text() -> String {
    let mut out = String::new();
    for t in DIALECT_TEXTS {
        for dialect in Dialect::ALL {
            let lowering = lowering(t.scope, t.name, dialect).expect("dialect 열");
            let findings: Vec<String> = FINDINGS
                .iter()
                .filter(|r| r.dialect == dialect && r.scope == t.scope && r.name == t.name)
                .map(|r| format!("{}:{}", r.code, r.status.name()))
                .collect();
            let _ = writeln!(
                out,
                "{}/{} {} {} rule={} template={:?} findings=[{}]\n  {}",
                t.scope,
                t.name,
                dialect.name(),
                lowering.status.name(),
                lowering.rule.id(),
                lowering.template,
                findings.join(", "),
                t.of(dialect)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fixture::check_golden;
    use crate::matrix::css::CSS_LOWERINGS;

    fn registered_items() -> BTreeSet<(&'static str, &'static str)> {
        let mut items: BTreeSet<(&str, &str)> = stellar_layout::declared::LONGHANDS
            .iter()
            .map(|name| {
                // item longhand는 `node/<이름>` 항목, container longhand는 `frame/<이름>` 항목입니다.
                let scope = if find("node", name).is_some() {
                    "node"
                } else {
                    "frame"
                };
                assert!(find(scope, name).is_some(), "{scope}/{name} 항목 없음");
                (scope, *name)
            })
            .collect();
        items.insert(("frame", "transform"));
        items
    }

    #[test]
    fn every_layout_longhand_has_a_rule_and_text_for_every_dialect() {
        // 검증 기준(XXII.1.4): Apple(SwiftUI·AppKit)·Android(Compose) 열이 layout longhand 40개와 local transform 전부에
        // 상태·규칙·문장을 가집니다. AppKit 열은 이 항목에만 있습니다.
        let items = registered_items();
        assert_eq!(items.len(), 41);
        let texts: BTreeSet<(&str, &str)> =
            DIALECT_TEXTS.iter().map(|t| (t.scope, t.name)).collect();
        assert_eq!(texts, items);
        assert_eq!(
            DIALECT_TEXTS.len(),
            items.len(),
            "같은 항목의 문장이 두 번 있습니다"
        );
        for (scope, name) in &items {
            for dialect in Dialect::ALL {
                let lowering = lowering(scope, name, dialect)
                    .unwrap_or_else(|| panic!("{scope}/{name} {} 규칙 없음", dialect.name()));
                assert!(!rule_text(scope, name, dialect).unwrap().is_empty());
                if lowering.status != CapabilityStatus::Native {
                    assert!(
                        lowering.rule.diagnostic().is_some(),
                        "{scope}/{name} {}: 근사·미지원 규칙에 진단이 없습니다",
                        dialect.name()
                    );
                }
            }
        }
        let appkit: BTreeSet<(&str, &str)> = CSS_LOWERINGS
            .iter()
            .filter(|row| row.appkit.is_some())
            .map(|row| (row.scope, row.name))
            .collect();
        assert_eq!(
            appkit, items,
            "AppKit 열은 AppKit adapter가 낮추는 항목에만 있습니다"
        );
        // SwiftUI 열은 Apple profile kind 열 그대로입니다(다시 정의하지 않음).
        for row in CSS_LOWERINGS {
            assert_eq!(
                lowering(row.scope, row.name, Dialect::SwiftUi),
                Some(row.apple)
            );
            assert_eq!(
                lowering(row.scope, row.name, Dialect::Compose),
                Some(row.android)
            );
        }
    }

    #[test]
    fn findings_name_registered_items_and_are_unique() {
        let items = registered_items();
        let mut keys = BTreeSet::new();
        let mut names = BTreeSet::new();
        for rule in FINDINGS {
            assert!(
                items.contains(&(rule.scope, rule.name)),
                "{}/{}는 dialect 열 항목이 아닙니다",
                rule.scope,
                rule.name
            );
            assert_ne!(
                rule.status,
                CapabilityStatus::Native,
                "Native는 진단하지 않습니다"
            );
            assert!(
                keys.insert((rule.dialect, rule.name, rule.code)),
                "{} {} {} 중복",
                rule.dialect.name(),
                rule.name,
                rule.code
            );
            names.insert((rule.dialect, rule.name, rule.scope));
        }
        // 한 dialect 안에서 이름은 범위 하나에만 있습니다(adapter는 이름으로 찾음).
        let mut by_name = BTreeSet::new();
        for (dialect, name, _) in &names {
            assert!(by_name.insert((*dialect, *name)), "{name} 범위가 둘입니다");
        }
    }

    #[test]
    fn findings_take_their_status_from_the_table() {
        let out = finding(Dialect::Compose, "width", "percent-size", "r");
        assert_eq!(out.status, CapabilityStatus::Approximated);
        let out = finding(Dialect::SwiftUi, "width", "percent-size", "r");
        assert_eq!(out.status, CapabilityStatus::Unsupported);
        assert!(registered(Dialect::AppKit, "width", "percent-size").is_none());
    }

    #[test]
    fn adapter_sources_only_emit_registered_findings() {
        // adapter 소스의 `matrix_finding("이름", "code", …)` 호출이 모두 이 dialect 표에 있습니다. 이름이 변수인 호출은
        // code가 그 dialect에 등록되어 있는지만 봅니다(실행 경로는 fixture test의 debug_assert가 확인).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for (file, dialect) in [
            ("apple/swiftui.rs", Dialect::SwiftUi),
            ("apple/appkit.rs", Dialect::AppKit),
            ("android/compose.rs", Dialect::Compose),
        ] {
            let source = std::fs::read_to_string(root.join(file)).expect("adapter 소스");
            let mut calls = 0;
            for chunk in source.split("matrix_finding(").skip(1) {
                let args: Vec<&str> = chunk.splitn(3, ',').collect();
                let name = args[0].trim();
                let code = args
                    .get(1)
                    .map(|c| c.trim().trim_matches('"'))
                    .unwrap_or("");
                if code.is_empty() || name.starts_with("longhand:") {
                    continue; // 함수 정의
                }
                calls += 1;
                if let Some(literal) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
                    assert!(
                        registered(dialect, literal, code).is_some(),
                        "{file}: {literal} {code}가 등록되지 않았습니다"
                    );
                } else {
                    assert!(
                        FINDINGS
                            .iter()
                            .any(|r| r.dialect == dialect && r.code == code),
                        "{file}: {code}가 등록되지 않았습니다"
                    );
                }
            }
            assert!(calls >= 15, "{file}: 진단 호출 {calls}개");
            for legacy in ["approximated(", "unsupported("] {
                assert!(
                    !source.contains(legacy),
                    "{file}: 상태를 직접 정하는 {legacy}가 남았습니다"
                );
            }
        }
    }

    #[test]
    fn dialect_matrix_text_is_stable() {
        check_golden("matrix_dialect.txt", &dialect_matrix_text());
    }
}
