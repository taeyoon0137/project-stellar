//! CSS 기반 속성 registry 항목의 Apple·Android lowering 규칙 (XXII.1.4, ADR-0106 결정 3).
//!
//! 상태는 CSS 매핑 상태에서 유도하지 않고 이 표의 규칙이 정합니다(ADR-0025 결정 1.3). 표에 없는 항목은
//! 허용하지 않습니다(ops test가 문서 registry 전체와 layout longhand 전체에 대해 검사).
//!
//! 규칙([`Rule`])은 닫힌 집합이고, 1:1이 아닌 규칙은 짝 진단 code를 가집니다. [`apply`]는 CSS 값 텍스트를 받아 대상
//! API 텍스트를 만드는 순수 함수입니다. template의 `{v}`는 값 텍스트 그대로, `{n}`은 단위(`px`·`%`·`deg`·`ms`)를 뗀
//! 수입니다(px는 pt·dp 1:1).

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::profile::TargetProfileKind;

/// lowering 규칙(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// 같은 의미의 view modifier·`Modifier` 하나(1:1).
    Modifier,
    /// 크기 longhand → frame·size modifier.
    FrameSize,
    /// flex container → stack(`HStack`·`VStack`, `Row`·`Column`)과 명시 spacing·alignment.
    StackLayout,
    /// grid container·item 배치 → platform grid(근사).
    GridLayout,
    /// flex item 늘이기·줄이기 → layout priority·weight(근사).
    FlexItem,
    /// 자유 배치(`position`·inset) → offset·position(근사).
    AbsoluteOffset,
    /// CSS 초기값과 플랫폼 기본값이 다른 항목은 항상 명시 값을 냅니다.
    ExplicitDefault,
    /// modifier stack 순서 → modifier chain 선언 순서(그대로 보존).
    ModifierChain,
    /// 블렌드·겹친 opacity·blur·shadow의 플랫폼 sRGB 합성(근사).
    SrgbComposite,
    /// 변별 테두리 → overlay stroke(근사).
    BorderOverlay,
    /// 모서리별 반경 → uneven rounded shape.
    CornerShape,
    /// clip → clip shape.
    ClipShape,
    /// mask → mask modifier·layer 합성(근사).
    MaskLayer,
    /// 글꼴 longhand → font 구성.
    FontAttribute,
    /// 줄 높이 → line spacing(근사, 계산 방식이 다름).
    LineSpacing,
    /// 이미지 맞춤·보간.
    ImageScaling,
    /// 비율 크기 → aspect ratio modifier(근사, indefinite 축 해석이 다름).
    AspectRatio,
    /// affine transform → transform effect·graphics layer.
    TransformEffect,
    /// 표시 여부.
    Visibility,
    /// 묶음 key → 구성 longhand 규칙으로 나눠 냄.
    GroupKey,
    /// Stellar 구조 → platform 구조(텍스트 내용, 이미지 자원, crop wrapper, 꺼진 modifier 생략).
    Structural,
    /// stellar-only 기하 → path로 굽기(근사).
    PathBake,
    /// 색공간 재해석 → 래스터화(근사).
    Rasterize,
    /// 출력 없음(편집기 전용 정보).
    NoOutput,
}

impl Rule {
    /// 모든 규칙.
    pub const ALL: [Rule; 24] = [
        Rule::Modifier,
        Rule::FrameSize,
        Rule::StackLayout,
        Rule::GridLayout,
        Rule::FlexItem,
        Rule::AbsoluteOffset,
        Rule::ExplicitDefault,
        Rule::ModifierChain,
        Rule::SrgbComposite,
        Rule::BorderOverlay,
        Rule::CornerShape,
        Rule::ClipShape,
        Rule::MaskLayer,
        Rule::FontAttribute,
        Rule::LineSpacing,
        Rule::ImageScaling,
        Rule::AspectRatio,
        Rule::TransformEffect,
        Rule::Visibility,
        Rule::GroupKey,
        Rule::Structural,
        Rule::PathBake,
        Rule::Rasterize,
        Rule::NoOutput,
    ];

    /// 안정 id.
    pub const fn id(self) -> &'static str {
        match self {
            Rule::Modifier => "modifier",
            Rule::FrameSize => "frame-size",
            Rule::StackLayout => "stack-layout",
            Rule::GridLayout => "grid-layout",
            Rule::FlexItem => "flex-item",
            Rule::AbsoluteOffset => "absolute-offset",
            Rule::ExplicitDefault => "explicit-default",
            Rule::ModifierChain => "modifier-chain",
            Rule::SrgbComposite => "srgb-composite",
            Rule::BorderOverlay => "border-overlay",
            Rule::CornerShape => "corner-shape",
            Rule::ClipShape => "clip-shape",
            Rule::MaskLayer => "mask-layer",
            Rule::FontAttribute => "font-attribute",
            Rule::LineSpacing => "line-spacing",
            Rule::ImageScaling => "image-scaling",
            Rule::AspectRatio => "aspect-ratio",
            Rule::TransformEffect => "transform-effect",
            Rule::Visibility => "visibility",
            Rule::GroupKey => "group-key",
            Rule::Structural => "structural",
            Rule::PathBake => "path-bake",
            Rule::Rasterize => "rasterize",
            Rule::NoOutput => "no-output",
        }
    }

    /// 짝 진단 code(1:1 규칙은 `None`).
    pub const fn diagnostic(self) -> Option<&'static str> {
        match self {
            Rule::Modifier
            | Rule::FrameSize
            | Rule::CornerShape
            | Rule::ClipShape
            | Rule::FontAttribute
            | Rule::ImageScaling
            | Rule::Visibility
            | Rule::GroupKey
            | Rule::Structural => None,
            Rule::StackLayout => Some("stack-layout"),
            Rule::GridLayout => Some("grid-approximated"),
            Rule::FlexItem => Some("flex-item-approximated"),
            Rule::AbsoluteOffset => Some("absolute-offset"),
            Rule::ExplicitDefault => Some("explicit-default"),
            Rule::ModifierChain => Some("modifier-chain-order"),
            Rule::SrgbComposite => Some("srgb-composite"),
            Rule::BorderOverlay => Some("border-overlay"),
            Rule::MaskLayer => Some("mask-layer"),
            Rule::LineSpacing => Some("line-spacing-approximated"),
            Rule::AspectRatio => Some("aspect-ratio-approximated"),
            Rule::TransformEffect => Some("transform-effect"),
            Rule::PathBake => Some("path-bake"),
            Rule::Rasterize => Some("rasterize"),
            Rule::NoOutput => Some("no-output"),
        }
    }
}

/// profile 하나의 규칙 적용(상태, 규칙, API template).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformLowering {
    /// 상태.
    pub status: CapabilityStatus,
    /// 규칙.
    pub rule: Rule,
    /// 대상 API template(`{v}` 값, `{n}` 수).
    pub template: &'static str,
}

/// registry 항목 하나의 Apple·Android 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssLowering {
    /// registry 범위.
    pub scope: &'static str,
    /// registry 이름.
    pub name: &'static str,
    /// Apple(SwiftUI 기준 dialect).
    pub apple: PlatformLowering,
    /// Android(Compose).
    pub android: PlatformLowering,
}

const N: CapabilityStatus = CapabilityStatus::Native;
const A: CapabilityStatus = CapabilityStatus::Approximated;
const U: CapabilityStatus = CapabilityStatus::Unsupported;

const fn row(
    scope: &'static str,
    name: &'static str,
    apple: (CapabilityStatus, Rule, &'static str),
    android: (CapabilityStatus, Rule, &'static str),
) -> CssLowering {
    CssLowering {
        scope,
        name,
        apple: PlatformLowering {
            status: apple.0,
            rule: apple.1,
            template: apple.2,
        },
        android: PlatformLowering {
            status: android.0,
            rule: android.1,
            template: android.2,
        },
    }
}

use Rule as R;

/// Apple·Android 규칙 표(registry 범위·이름 순서).
pub const CSS_LOWERINGS: &[CssLowering] = &[
    // node 공통
    row(
        "node",
        "visible",
        (N, R::Visibility, ".hidden()"),
        (N, R::Visibility, "Modifier.alpha(0f)"),
    ),
    row("node", "locked", (U, R::NoOutput, ""), (U, R::NoOutput, "")),
    row("node", "name", (U, R::NoOutput, ""), (U, R::NoOutput, "")),
    // frame
    row(
        "frame",
        "size",
        (N, R::GroupKey, ".frame(width:height:)"),
        (N, R::GroupKey, "Modifier.size()"),
    ),
    row(
        "frame",
        "transform",
        (N, R::TransformEffect, ".transformEffect({v})"),
        (A, R::TransformEffect, "Modifier.graphicsLayer({v})"),
    ),
    row(
        "frame",
        "clipping",
        (N, R::ClipShape, ".clipped()"),
        (N, R::ClipShape, "Modifier.clipToBounds()"),
    ),
    row(
        "frame",
        "background-color",
        (N, R::Modifier, ".background({v})"),
        (N, R::Modifier, "Modifier.background({v})"),
    ),
    row(
        "frame",
        "border",
        (N, R::GroupKey, ".overlay(strokeBorder)"),
        (N, R::GroupKey, "Modifier.border()"),
    ),
    row(
        "frame",
        "border-radius",
        (N, R::GroupKey, ".clipShape(RoundedRectangle)"),
        (N, R::GroupKey, "RoundedCornerShape()"),
    ),
    row(
        "frame",
        "layout-mode",
        (U, R::NoOutput, ""),
        (U, R::NoOutput, ""),
    ),
    row(
        "frame",
        "width",
        (N, R::FrameSize, ".frame(width: {n})"),
        (N, R::FrameSize, "Modifier.width({n}.dp)"),
    ),
    row(
        "frame",
        "height",
        (N, R::FrameSize, ".frame(height: {n})"),
        (N, R::FrameSize, "Modifier.height({n}.dp)"),
    ),
    row(
        "frame",
        "border-top-left-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(topLeadingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(topStart = {n}.dp)"),
    ),
    row(
        "frame",
        "border-top-right-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(topTrailingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(topEnd = {n}.dp)"),
    ),
    row(
        "frame",
        "border-bottom-right-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(bottomTrailingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(bottomEnd = {n}.dp)"),
    ),
    row(
        "frame",
        "border-bottom-left-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(bottomLeadingRadius: {n})",
        ),
        (
            N,
            R::CornerShape,
            "RoundedCornerShape(bottomStart = {n}.dp)",
        ),
    ),
    row(
        "frame",
        "border-top-width",
        (A, R::BorderOverlay, ".overlay(edge: .top, width: {n})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(top = {n}.dp)"),
    ),
    row(
        "frame",
        "border-top-style",
        (A, R::BorderOverlay, ".overlay(edge: .top, style: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(top = {v})"),
    ),
    row(
        "frame",
        "border-top-color",
        (A, R::BorderOverlay, ".overlay(edge: .top, color: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(top = {v})"),
    ),
    row(
        "frame",
        "border-right-width",
        (A, R::BorderOverlay, ".overlay(edge: .trailing, width: {n})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(end = {n}.dp)"),
    ),
    row(
        "frame",
        "border-right-style",
        (A, R::BorderOverlay, ".overlay(edge: .trailing, style: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(end = {v})"),
    ),
    row(
        "frame",
        "border-right-color",
        (A, R::BorderOverlay, ".overlay(edge: .trailing, color: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(end = {v})"),
    ),
    row(
        "frame",
        "border-bottom-width",
        (A, R::BorderOverlay, ".overlay(edge: .bottom, width: {n})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(bottom = {n}.dp)"),
    ),
    row(
        "frame",
        "border-bottom-style",
        (A, R::BorderOverlay, ".overlay(edge: .bottom, style: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(bottom = {v})"),
    ),
    row(
        "frame",
        "border-bottom-color",
        (A, R::BorderOverlay, ".overlay(edge: .bottom, color: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(bottom = {v})"),
    ),
    row(
        "frame",
        "border-left-width",
        (A, R::BorderOverlay, ".overlay(edge: .leading, width: {n})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(start = {n}.dp)"),
    ),
    row(
        "frame",
        "border-left-style",
        (A, R::BorderOverlay, ".overlay(edge: .leading, style: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(start = {v})"),
    ),
    row(
        "frame",
        "border-left-color",
        (A, R::BorderOverlay, ".overlay(edge: .leading, color: {v})"),
        (A, R::BorderOverlay, "Modifier.drawBehind(start = {v})"),
    ),
    // shape
    row(
        "shape",
        "width",
        (N, R::FrameSize, ".frame(width: {n})"),
        (N, R::FrameSize, "Modifier.width({n}.dp)"),
    ),
    row(
        "shape",
        "height",
        (N, R::FrameSize, ".frame(height: {n})"),
        (N, R::FrameSize, "Modifier.height({n}.dp)"),
    ),
    row(
        "shape",
        "border-top-left-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(topLeadingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(topStart = {n}.dp)"),
    ),
    row(
        "shape",
        "border-top-right-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(topTrailingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(topEnd = {n}.dp)"),
    ),
    row(
        "shape",
        "border-bottom-right-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(bottomTrailingRadius: {n})",
        ),
        (N, R::CornerShape, "RoundedCornerShape(bottomEnd = {n}.dp)"),
    ),
    row(
        "shape",
        "border-bottom-left-radius",
        (
            N,
            R::CornerShape,
            "UnevenRoundedRectangle(bottomLeadingRadius: {n})",
        ),
        (
            N,
            R::CornerShape,
            "RoundedCornerShape(bottomStart = {n}.dp)",
        ),
    ),
    row(
        "shape",
        "x1",
        (A, R::PathBake, "Path.move(x: {n})"),
        (A, R::PathBake, "Path.moveTo(x = {n}f)"),
    ),
    row(
        "shape",
        "y1",
        (A, R::PathBake, "Path.move(y: {n})"),
        (A, R::PathBake, "Path.moveTo(y = {n}f)"),
    ),
    row(
        "shape",
        "x2",
        (A, R::PathBake, "Path.addLine(x: {n})"),
        (A, R::PathBake, "Path.lineTo(x = {n}f)"),
    ),
    row(
        "shape",
        "y2",
        (A, R::PathBake, "Path.addLine(y: {n})"),
        (A, R::PathBake, "Path.lineTo(y = {n}f)"),
    ),
    row(
        "shape",
        "stroke-linecap",
        (N, R::Modifier, "StrokeStyle(lineCap: {v})"),
        (N, R::Modifier, "Stroke(cap = {v})"),
    ),
    row(
        "shape",
        "polygon-sides",
        (A, R::PathBake, "Path(polygon sides: {n})"),
        (A, R::PathBake, "Path(polygon sides = {n})"),
    ),
    row(
        "shape",
        "polygon-rotation",
        (A, R::PathBake, "Path(polygon rotation: {n})"),
        (A, R::PathBake, "Path(polygon rotation = {n})"),
    ),
    row(
        "shape",
        "polygon-corner-radius",
        (A, R::PathBake, "Path(polygon cornerRadius: {n})"),
        (A, R::PathBake, "Path(polygon cornerRadius = {n})"),
    ),
    // path
    row(
        "path",
        "path-data",
        (N, R::Structural, "Path({v})"),
        (N, R::Structural, "PathParser().parsePathString({v})"),
    ),
    row(
        "path",
        "fill-rule",
        (N, R::Modifier, "FillStyle(eoFill: {v})"),
        (N, R::Modifier, "PathFillType.{v}"),
    ),
    // text
    row(
        "text",
        "text-content",
        (N, R::Structural, "Text({v})"),
        (N, R::Structural, "Text({v})"),
    ),
    row(
        "text",
        "font-family",
        (N, R::FontAttribute, ".font(.custom({v}))"),
        (N, R::FontAttribute, "FontFamily({v})"),
    ),
    row(
        "text",
        "font-size",
        (N, R::FontAttribute, ".font(.system(size: {n}))"),
        (N, R::FontAttribute, "fontSize = {n}.sp"),
    ),
    row(
        "text",
        "font-weight",
        (N, R::FontAttribute, ".fontWeight({v})"),
        (N, R::FontAttribute, "FontWeight({v})"),
    ),
    row(
        "text",
        "font-style",
        (N, R::FontAttribute, ".italic()"),
        (N, R::FontAttribute, "FontStyle.{v}"),
    ),
    row(
        "text",
        "font-stretch",
        (N, R::FontAttribute, ".fontWidth({v})"),
        (U, R::NoOutput, ""),
    ),
    row(
        "text",
        "line-height",
        (A, R::LineSpacing, ".lineSpacing({n})"),
        (A, R::LineSpacing, "lineHeight = {n}.sp"),
    ),
    row(
        "text",
        "letter-spacing",
        (N, R::Modifier, ".tracking({n})"),
        (N, R::Modifier, "letterSpacing = {n}.sp"),
    ),
    // image
    row(
        "image",
        "image-asset",
        (N, R::Structural, "Image({v})"),
        (N, R::Structural, "painterResource({v})"),
    ),
    row(
        "image",
        "width",
        (N, R::FrameSize, ".frame(width: {n})"),
        (N, R::FrameSize, "Modifier.width({n}.dp)"),
    ),
    row(
        "image",
        "height",
        (N, R::FrameSize, ".frame(height: {n})"),
        (N, R::FrameSize, "Modifier.height({n}.dp)"),
    ),
    row(
        "image",
        "object-fit",
        (N, R::ImageScaling, ".aspectRatio(contentMode: {v})"),
        (N, R::ImageScaling, "ContentScale.{v}"),
    ),
    row(
        "image",
        "image-rendering",
        (N, R::ImageScaling, ".interpolation({v})"),
        (N, R::ImageScaling, "FilterQuality.{v}"),
    ),
    row(
        "image",
        "image-crop",
        (N, R::Structural, ".clipped() crop wrapper"),
        (N, R::Structural, "Modifier.clipToBounds() crop wrapper"),
    ),
    row(
        "image",
        "image-color-space",
        (A, R::Rasterize, "rasterized image"),
        (A, R::Rasterize, "rasterized image"),
    ),
    // modifier
    row(
        "modifier",
        "enabled",
        (N, R::Structural, ""),
        (N, R::Structural, ""),
    ),
    row(
        "modifier",
        "modifier-order",
        (N, R::ModifierChain, "modifier chain"),
        (N, R::ModifierChain, "Modifier chain"),
    ),
    row(
        "modifier",
        "transform",
        (N, R::TransformEffect, ".transformEffect({v})"),
        (A, R::TransformEffect, "Modifier.graphicsLayer({v})"),
    ),
    row(
        "modifier",
        "boolean-operation",
        (A, R::PathBake, "baked Path"),
        (A, R::PathBake, "baked Path"),
    ),
    row(
        "modifier",
        "boolean-operands",
        (A, R::PathBake, "baked Path"),
        (A, R::PathBake, "baked Path"),
    ),
    row(
        "modifier",
        "stroke-expand-width",
        (A, R::PathBake, "baked Path"),
        (A, R::PathBake, "baked Path"),
    ),
    row(
        "modifier",
        "corner-radius",
        (A, R::PathBake, "baked Path"),
        (A, R::PathBake, "baked Path"),
    ),
    row(
        "modifier",
        "mask-image",
        (N, R::MaskLayer, ".mask({v})"),
        (A, R::MaskLayer, "Modifier.graphicsLayer(BlendMode.DstIn)"),
    ),
    row(
        "modifier",
        "clip-path",
        (N, R::ClipShape, ".clipShape({v})"),
        (N, R::ClipShape, "Modifier.clip({v})"),
    ),
    row(
        "modifier",
        "fill",
        (N, R::Modifier, ".fill({v})"),
        (N, R::Modifier, "drawRect(color = {v})"),
    ),
    row(
        "modifier",
        "stroke",
        (N, R::Modifier, ".stroke({v})"),
        (N, R::Modifier, "Stroke(color = {v})"),
    ),
    row(
        "modifier",
        "stroke-width",
        (N, R::Modifier, "StrokeStyle(lineWidth: {n})"),
        (N, R::Modifier, "Stroke(width = {n}f)"),
    ),
    row(
        "modifier",
        "shadow-color",
        (A, R::SrgbComposite, ".shadow(color: {v})"),
        (A, R::SrgbComposite, "Modifier.shadow(color = {v})"),
    ),
    row(
        "modifier",
        "shadow-offset",
        (A, R::SrgbComposite, ".shadow(x:y:)"),
        (A, R::SrgbComposite, "Modifier.shadow(offset)"),
    ),
    row(
        "modifier",
        "shadow-blur",
        (A, R::SrgbComposite, ".shadow(radius: {n})"),
        (A, R::SrgbComposite, "Modifier.shadow(elevation = {n}.dp)"),
    ),
    row(
        "modifier",
        "shadow-offset-x",
        (A, R::SrgbComposite, ".shadow(x: {n})"),
        (U, R::NoOutput, ""),
    ),
    row(
        "modifier",
        "shadow-offset-y",
        (A, R::SrgbComposite, ".shadow(y: {n})"),
        (U, R::NoOutput, ""),
    ),
    row(
        "modifier",
        "blur-radius",
        (A, R::SrgbComposite, ".blur(radius: {n})"),
        (A, R::SrgbComposite, "Modifier.blur({n}.dp)"),
    ),
    row(
        "modifier",
        "mix-blend-mode",
        (A, R::SrgbComposite, ".blendMode({v})"),
        (
            A,
            R::SrgbComposite,
            "Modifier.graphicsLayer(blendMode = {v})",
        ),
    ),
    row(
        "modifier",
        "opacity",
        (A, R::SrgbComposite, ".opacity({n})"),
        (A, R::SrgbComposite, "Modifier.alpha({n}f)"),
    ),
    // layout item longhand(범위 node)
    row(
        "node",
        "position",
        (A, R::AbsoluteOffset, "ZStack + .position({v})"),
        (A, R::AbsoluteOffset, "Box + Modifier.offset({v})"),
    ),
    row(
        "node",
        "box-sizing",
        (N, R::ExplicitDefault, "border-box frame"),
        (N, R::ExplicitDefault, "border-box size"),
    ),
    row(
        "node",
        "top",
        (A, R::AbsoluteOffset, ".offset(y: {n})"),
        (A, R::AbsoluteOffset, "Modifier.offset(y = {n}.dp)"),
    ),
    row(
        "node",
        "right",
        (A, R::AbsoluteOffset, ".offset(x: -{n})"),
        (A, R::AbsoluteOffset, "Modifier.offset(x = -{n}.dp)"),
    ),
    row(
        "node",
        "bottom",
        (A, R::AbsoluteOffset, ".offset(y: -{n})"),
        (A, R::AbsoluteOffset, "Modifier.offset(y = -{n}.dp)"),
    ),
    row(
        "node",
        "left",
        (A, R::AbsoluteOffset, ".offset(x: {n})"),
        (A, R::AbsoluteOffset, "Modifier.offset(x = {n}.dp)"),
    ),
    row(
        "node",
        "width",
        (N, R::FrameSize, ".frame(width: {n})"),
        (N, R::FrameSize, "Modifier.width({n}.dp)"),
    ),
    row(
        "node",
        "height",
        (N, R::FrameSize, ".frame(height: {n})"),
        (N, R::FrameSize, "Modifier.height({n}.dp)"),
    ),
    row(
        "node",
        "min-width",
        (N, R::FrameSize, ".frame(minWidth: {n})"),
        (N, R::FrameSize, "Modifier.widthIn(min = {n}.dp)"),
    ),
    row(
        "node",
        "min-height",
        (N, R::FrameSize, ".frame(minHeight: {n})"),
        (N, R::FrameSize, "Modifier.heightIn(min = {n}.dp)"),
    ),
    row(
        "node",
        "max-width",
        (N, R::FrameSize, ".frame(maxWidth: {n})"),
        (N, R::FrameSize, "Modifier.widthIn(max = {n}.dp)"),
    ),
    row(
        "node",
        "max-height",
        (N, R::FrameSize, ".frame(maxHeight: {n})"),
        (N, R::FrameSize, "Modifier.heightIn(max = {n}.dp)"),
    ),
    row(
        "node",
        "aspect-ratio",
        (A, R::AspectRatio, ".aspectRatio({v})"),
        (A, R::AspectRatio, "Modifier.aspectRatio({v}f)"),
    ),
    row(
        "node",
        "flex-grow",
        (A, R::FlexItem, ".frame(maxWidth: .infinity) priority {n}"),
        (A, R::FlexItem, "Modifier.weight({n}f)"),
    ),
    row(
        "node",
        "flex-shrink",
        (A, R::FlexItem, ".layoutPriority(-{n})"),
        (A, R::FlexItem, "Modifier.weight({n}f, fill = false)"),
    ),
    row(
        "node",
        "flex-basis",
        (A, R::FlexItem, ".frame(idealWidth: {n})"),
        (A, R::FlexItem, "Modifier.width({n}.dp)"),
    ),
    row(
        "node",
        "align-self",
        (A, R::StackLayout, ".frame(alignment: {v})"),
        (N, R::StackLayout, "Modifier.align({v})"),
    ),
    row(
        "node",
        "justify-self",
        (A, R::GridLayout, ".gridColumnAlignment({v})"),
        (A, R::GridLayout, "Modifier.align({v})"),
    ),
    row(
        "node",
        "grid-column-start",
        (A, R::GridLayout, ".gridCellColumns({v})"),
        (A, R::GridLayout, "GridItemSpan({v})"),
    ),
    row(
        "node",
        "grid-column-end",
        (A, R::GridLayout, ".gridCellColumns({v})"),
        (A, R::GridLayout, "GridItemSpan({v})"),
    ),
    row(
        "node",
        "grid-row-start",
        (U, R::NoOutput, ""),
        (U, R::NoOutput, ""),
    ),
    row(
        "node",
        "grid-row-end",
        (U, R::NoOutput, ""),
        (U, R::NoOutput, ""),
    ),
    // layout container longhand(범위 frame)
    row(
        "frame",
        "display",
        (N, R::StackLayout, "VStack/HStack/Grid({v})"),
        (N, R::StackLayout, "Column/Row/LazyVerticalGrid({v})"),
    ),
    row(
        "frame",
        "padding-top",
        (N, R::Modifier, ".padding(.top, {n})"),
        (N, R::Modifier, "Modifier.padding(top = {n}.dp)"),
    ),
    row(
        "frame",
        "padding-right",
        (N, R::Modifier, ".padding(.trailing, {n})"),
        (N, R::Modifier, "Modifier.padding(end = {n}.dp)"),
    ),
    row(
        "frame",
        "padding-bottom",
        (N, R::Modifier, ".padding(.bottom, {n})"),
        (N, R::Modifier, "Modifier.padding(bottom = {n}.dp)"),
    ),
    row(
        "frame",
        "padding-left",
        (N, R::Modifier, ".padding(.leading, {n})"),
        (N, R::Modifier, "Modifier.padding(start = {n}.dp)"),
    ),
    row(
        "frame",
        "flex-direction",
        (N, R::StackLayout, "HStack/VStack({v})"),
        (N, R::StackLayout, "Row/Column({v})"),
    ),
    row(
        "frame",
        "flex-wrap",
        (A, R::StackLayout, "custom Layout wrap({v})"),
        (A, R::StackLayout, "FlowRow({v})"),
    ),
    row(
        "frame",
        "justify-content",
        (A, R::StackLayout, "Spacer distribution({v})"),
        (N, R::StackLayout, "Arrangement.{v}"),
    ),
    row(
        "frame",
        "align-content",
        (U, R::NoOutput, ""),
        (A, R::StackLayout, "FlowRow verticalArrangement({v})"),
    ),
    row(
        "frame",
        "align-items",
        (N, R::ExplicitDefault, "VStack(alignment: {v})"),
        (N, R::ExplicitDefault, "Alignment.{v}"),
    ),
    row(
        "frame",
        "justify-items",
        (A, R::GridLayout, "GridRow(alignment: {v})"),
        (A, R::GridLayout, "Alignment.{v}"),
    ),
    row(
        "frame",
        "row-gap",
        (N, R::ExplicitDefault, "VStack(spacing: {n})"),
        (N, R::ExplicitDefault, "Arrangement.spacedBy({n}.dp)"),
    ),
    row(
        "frame",
        "column-gap",
        (N, R::ExplicitDefault, "HStack(spacing: {n})"),
        (N, R::ExplicitDefault, "Arrangement.spacedBy({n}.dp)"),
    ),
    row(
        "frame",
        "grid-template-columns",
        (A, R::GridLayout, "Grid columns({v})"),
        (A, R::GridLayout, "GridCells.Fixed({v})"),
    ),
    row(
        "frame",
        "grid-template-rows",
        (A, R::GridLayout, "Grid rows({v})"),
        (U, R::NoOutput, ""),
    ),
    row(
        "frame",
        "grid-auto-columns",
        (A, R::GridLayout, "GridItem(.flexible)({v})"),
        (A, R::GridLayout, "GridCells.Adaptive({v})"),
    ),
    row(
        "frame",
        "grid-auto-rows",
        (A, R::GridLayout, "GridItem(.flexible)({v})"),
        (U, R::NoOutput, ""),
    ),
    row(
        "frame",
        "grid-auto-flow",
        (A, R::GridLayout, "LazyHGrid/LazyVGrid({v})"),
        (A, R::GridLayout, "LazyHorizontalGrid/LazyVerticalGrid({v})"),
    ),
];

/// (범위, 이름)의 규칙.
pub fn find(scope: &str, name: &str) -> Option<&'static CssLowering> {
    CSS_LOWERINGS
        .iter()
        .find(|row| row.scope == scope && row.name == name)
}

/// profile kind의 규칙(Web은 `None`: Web은 IX.11.19 matrix를 그대로 씀).
pub fn platform(row: &CssLowering, kind: TargetProfileKind) -> Option<&PlatformLowering> {
    match kind {
        TargetProfileKind::Web => None,
        TargetProfileKind::Apple => Some(&row.apple),
        TargetProfileKind::Android => Some(&row.android),
    }
}

/// 규칙 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutput {
    /// 대상 API 텍스트(출력 없음이면 빈 문자열).
    pub code: String,
    /// 짝 진단 code.
    pub diagnostic: Option<&'static str>,
}

/// 단위를 뗀 수 텍스트(`16px` → `16`).
fn number_text(value: &str) -> &str {
    for unit in ["px", "%", "deg", "ms"] {
        if let Some(number) = value.strip_suffix(unit) {
            return number;
        }
    }
    value
}

/// 규칙을 CSS 값 텍스트 하나에 적용합니다.
///
/// - `Visibility`: 값이 `false`·`hidden`일 때만 template을 냅니다.
/// - `NoOutput`: 항상 빈 출력입니다.
/// - `Structural`의 꺼진 modifier(`enabled` = `false`)는 출력에서 뺍니다(빈 출력).
pub fn apply(lowering: &PlatformLowering, value: &str) -> RuleOutput {
    let code = match lowering.rule {
        Rule::NoOutput => String::new(),
        Rule::Visibility if !matches!(value, "false" | "hidden") => String::new(),
        _ => lowering
            .template
            .replace("{v}", value)
            .replace("{n}", number_text(value)),
    };
    RuleOutput {
        code,
        diagnostic: lowering.rule.diagnostic(),
    }
}
