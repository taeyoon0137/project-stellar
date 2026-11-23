//! Scene primitive → SVG element mapping table (XX.3.1).
//!
//! [`SVG_MAPPING`]은 scene이 표현하는 모든 기능([`SvgFeature`])을 SVG 요소·속성에 대응시키고 지원
//! 상태([`SvgSupport`])를 명시합니다. [`node_features`]는 scene node를 기능 집합으로 분류하며,
//! scene enum을 **wildcard 없이** match하므로 scene에 variant가 추가되면 이 표를 갱신하기 전까지
//! 컴파일되지 않습니다.
//!
//! ## 좌표계
//!
//! node 그룹(`<g>`)은 변환 없이 world 좌표에 놓입니다. 내용 요소는 node world transform을
//! `transform="matrix(a b c d e f)"`로 직접 가지며, 조상 clip은 clip 자체의 world transform으로
//! 그린 `<clipPath>`(userSpaceOnUse)를 node 그룹에 겁니다. 그래서 subtree 합성 효과(opacity)는 그룹에,
//! 기하는 요소에 대응해 world transform을 한 번만 적용합니다.
//!
//! ## 지원 상태
//!
//! - [`SvgSupport::Vector`]: SVG 요소·속성으로 기록합니다.
//! - [`SvgSupport::Deferred`]: 후속 WBS row가 구현할 때까지 writer가 typed error로 거부합니다.
//! - [`SvgSupport::Rasterize`]: SVG로 정확히 표현할 수 없어 부분 래스터화(XX.4) 대상입니다. writer는 raster
//!   경계 planner(XX.4.1)가 고른 최소 영역만 host rasterizer(XX.4.2)로 PNG를 받아 `<image>`로 넣습니다. host가
//!   rasterizer를 주지 않으면 사전 진단(XX.1.3)이 형식 차단으로 거부합니다(조용히 빼거나 근사하지 않습니다).
//! - [`SvgSupport::NotInScene`]: scene에 아직 해당 primitive가 없습니다. 대응 SVG 요소만 명시합니다.
//!
//! 알파 합성 색공간 차이(Stellar 렌더러는 선형 합성, 일반 SVG 렌더러는 sRGB 합성, 아키텍처 §10.3)는
//! 구조 대응과 별개입니다. 반투명·opacity의 외형 허용 오차는 XX.3.4의 browser render fixture가
//! 고정합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_scene::geometry::FillRule;
use stellar_scene::node::{
    ImageCrop, ImageFit, ImageSampling, SceneContent, SceneNode, Visibility,
};
use stellar_scene::stack::{
    ClipShape, Opacity, Paint, PaintLayer, SceneBlendMode, StrokeAlign, StrokeCap,
};

/// scene이 표현하는 기능 하나. 선언 순서가 [`SVG_MAPPING`] 순서이자 보고 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SvgFeature {
    /// 직각 사각형 content(Frame 배경 상자).
    RectContent,
    /// 둥근 모서리 사각형 content.
    RoundedRectContent,
    /// path content(Shape·Path).
    PathContent,
    /// even-odd 채우기 규칙.
    EvenOddFill,
    /// text layout content.
    TextContent,
    /// image content.
    ImageContent,
    /// image fit: 비율 무시 채우기.
    ImageFitFill,
    /// image fit: 비율 유지 전체 보이기.
    ImageFitContain,
    /// image fit: 비율 유지 덮기.
    ImageFitCover,
    /// image fit: 원본 크기.
    ImageFitNone,
    /// image crop(전체가 아닌 영역).
    ImageCrop,
    /// image 최근접 sampling.
    ImageNearestSampling,
    /// image 색공간 override.
    ImageColorSpaceOverride,
    /// 단색 채우기 layer.
    SolidFill,
    /// sRGB 범위 안 색.
    SrgbColor,
    /// Display P3 또는 `[0, 1]` 밖 sRGB 색.
    WideGamutColor,
    /// 알파 < 1 색.
    TranslucentColor,
    /// 중심선 stroke.
    CenterStroke,
    /// 안쪽 stroke.
    InsideStroke,
    /// 바깥쪽 stroke.
    OutsideStroke,
    /// 둥근 선 끝.
    RoundCap,
    /// 사각 선 끝.
    SquareCap,
    /// drop shadow.
    DropShadow,
    /// layer blur.
    LayerBlur,
    /// subtree 합성 opacity < 1.
    GroupOpacity,
    /// Normal이 아닌 blend mode.
    BlendMode,
    /// 직각 사각형 clip.
    RectClip,
    /// 둥근 모서리 사각형 clip.
    RoundedRectClip,
    /// path clip.
    PathClip,
    /// 2개 이상 중첩 clip.
    NestedClip,
    /// 항등이 아닌 world transform.
    WorldTransform,
    /// 숨긴 node.
    HiddenNode,
    /// 선형 gradient paint.
    LinearGradient,
    /// 원형 gradient paint.
    RadialGradient,
}

impl SvgFeature {
    /// 정의된 모든 기능(선언 순서).
    pub const ALL: [SvgFeature; 34] = [
        SvgFeature::RectContent,
        SvgFeature::RoundedRectContent,
        SvgFeature::PathContent,
        SvgFeature::EvenOddFill,
        SvgFeature::TextContent,
        SvgFeature::ImageContent,
        SvgFeature::ImageFitFill,
        SvgFeature::ImageFitContain,
        SvgFeature::ImageFitCover,
        SvgFeature::ImageFitNone,
        SvgFeature::ImageCrop,
        SvgFeature::ImageNearestSampling,
        SvgFeature::ImageColorSpaceOverride,
        SvgFeature::SolidFill,
        SvgFeature::SrgbColor,
        SvgFeature::WideGamutColor,
        SvgFeature::TranslucentColor,
        SvgFeature::CenterStroke,
        SvgFeature::InsideStroke,
        SvgFeature::OutsideStroke,
        SvgFeature::RoundCap,
        SvgFeature::SquareCap,
        SvgFeature::DropShadow,
        SvgFeature::LayerBlur,
        SvgFeature::GroupOpacity,
        SvgFeature::BlendMode,
        SvgFeature::RectClip,
        SvgFeature::RoundedRectClip,
        SvgFeature::PathClip,
        SvgFeature::NestedClip,
        SvgFeature::WorldTransform,
        SvgFeature::HiddenNode,
        SvgFeature::LinearGradient,
        SvgFeature::RadialGradient,
    ];

    /// 이 기능의 mapping row.
    pub fn mapping(self) -> &'static SvgMapping {
        // SVG_MAPPING은 ALL 순서와 같고(inventory test로 고정) enum 판별값이 곧 index입니다.
        &SVG_MAPPING[self as usize]
    }

    /// 지원 상태.
    pub fn support(self) -> SvgSupport {
        self.mapping().support
    }
}

impl fmt::Display for SvgFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl SvgFeature {
    /// 이 기능이 출력하는 값의 원천인 registry CSS longhand 이름(IX.11.20). 속성 registry
    /// (`stellar_document::css::registry`)의 CSS 매핑 longhand와 같은 이름이고, Stellar 전용 원천(Text 본문, image
    /// asset·crop, 색 표기 방식 등)은 빈 목록입니다. export crate는 document에 의존하지 않으므로 이름으로 잇고, 종단
    /// test가 registry와 대조합니다(`tests/integration/tests/svg_registry_mapping.rs`).
    pub fn css_longhands(self) -> &'static [&'static str] {
        match self {
            SvgFeature::RectContent => &["width", "height", "background-color"],
            SvgFeature::RoundedRectContent => &[
                "border-top-left-radius",
                "border-top-right-radius",
                "border-bottom-right-radius",
                "border-bottom-left-radius",
            ],
            SvgFeature::PathContent => &["d"],
            SvgFeature::EvenOddFill => &["fill-rule"],
            SvgFeature::TextContent => &[],
            SvgFeature::ImageContent => &["width", "height"],
            SvgFeature::ImageFitFill
            | SvgFeature::ImageFitContain
            | SvgFeature::ImageFitCover
            | SvgFeature::ImageFitNone => &["object-fit"],
            SvgFeature::ImageCrop => &[],
            SvgFeature::ImageNearestSampling => &["image-rendering"],
            SvgFeature::ImageColorSpaceOverride => &[],
            SvgFeature::SolidFill => &["fill", "background-color"],
            SvgFeature::SrgbColor | SvgFeature::WideGamutColor => &[],
            SvgFeature::TranslucentColor => &["fill", "stroke"],
            SvgFeature::CenterStroke | SvgFeature::InsideStroke | SvgFeature::OutsideStroke => {
                &["stroke", "stroke-width"]
            }
            SvgFeature::RoundCap | SvgFeature::SquareCap => &["stroke-linecap"],
            SvgFeature::DropShadow => &["box-shadow", "filter"],
            SvgFeature::LayerBlur => &["filter"],
            SvgFeature::GroupOpacity => &["opacity"],
            SvgFeature::BlendMode => &["mix-blend-mode"],
            SvgFeature::RectClip | SvgFeature::RoundedRectClip => &["overflow-x", "overflow-y"],
            SvgFeature::PathClip | SvgFeature::NestedClip => &["clip-path"],
            SvgFeature::WorldTransform => &["transform"],
            SvgFeature::HiddenNode => &["display"],
            SvgFeature::LinearGradient | SvgFeature::RadialGradient => &["background-image"],
        }
    }
}

/// SVG 지원 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SvgSupport {
    /// SVG 요소·속성으로 기록합니다.
    Vector,
    /// 후속 WBS row가 구현합니다. 그 전까지 writer는 거부합니다.
    Deferred {
        /// 구현 row.
        row: &'static str,
    },
    /// 부분 래스터화 대상입니다. 래스터화가 구현될 때까지 writer는 거부합니다.
    Rasterize {
        /// 구현 row.
        row: &'static str,
    },
    /// scene에 아직 없는 primitive입니다(대응 요소만 명시).
    NotInScene,
}

impl SvgSupport {
    /// writer가 지금 기록할 수 있으면 참입니다.
    pub const fn is_vector(self) -> bool {
        matches!(self, SvgSupport::Vector)
    }
}

impl fmt::Display for SvgSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvgSupport::Vector => f.write_str("vector"),
            SvgSupport::Deferred { row } => write!(f, "후속 구현({row})"),
            SvgSupport::Rasterize { row } => write!(f, "부분 래스터화({row})"),
            SvgSupport::NotInScene => f.write_str("scene 미지원"),
        }
    }
}

/// mapping table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgMapping {
    /// 기능.
    pub feature: SvgFeature,
    /// scene 원천(어느 document node·scene 값에서 오는지).
    pub scene: &'static str,
    /// 대응 SVG 요소·속성.
    pub svg: &'static str,
    /// 지원 상태.
    pub support: SvgSupport,
}

const fn row(
    feature: SvgFeature,
    scene: &'static str,
    svg: &'static str,
    support: SvgSupport,
) -> SvgMapping {
    SvgMapping {
        feature,
        scene,
        svg,
        support,
    }
}

const V: SvgSupport = SvgSupport::Vector;

/// Scene primitive → SVG mapping table(XX.3.1). [`SvgFeature::ALL`] 순서입니다.
pub const SVG_MAPPING: [SvgMapping; 34] = [
    row(
        SvgFeature::RectContent,
        "Frame 배경 상자: SceneContent::Rect, radius 0",
        "<rect x y width height>",
        V,
    ),
    row(
        SvgFeature::RoundedRectContent,
        "Frame 배경 상자: SceneContent::Rect, radius > 0",
        "<path d> — 모서리마다 원호 A rx=ry=radius(시계 방향)",
        V,
    ),
    row(
        SvgFeature::PathContent,
        "Shape(VI.5.6 평가 결과)·Path: SceneContent::Path",
        "<path d> — MoveTo=M, LineTo=L, CubicTo=C, Close=Z",
        V,
    ),
    row(
        SvgFeature::EvenOddFill,
        "Path winding rule EvenOdd(FillRule::EvenOdd)",
        "fill-rule=\"evenodd\" / clip-rule=\"evenodd\"(NonZero는 기본값이라 생략)",
        V,
    ),
    row(
        SvgFeature::TextContent,
        "Text: SceneContent::Text(TextLayoutKey) → 고정 font revision glyph run·outline(ExportTextSource)",
        "run마다 glyph outline <path d fill=run 색>(non-zero), paint layer는 모든 glyph를 이은 <path> — <text>·font 포함 없음",
        V,
    ),
    row(
        SvgFeature::ImageContent,
        "Image: SceneContent::Image(AssetRef, 고정 asset revision)",
        "<svg x y width height viewBox preserveAspectRatio overflow=hidden><image href=\"data:<mime>;base64,…\">",
        V,
    ),
    row(
        SvgFeature::ImageFitFill,
        "ImageFit::Fill",
        "preserveAspectRatio=\"none\"",
        V,
    ),
    row(
        SvgFeature::ImageFitContain,
        "ImageFit::Contain",
        "preserveAspectRatio=\"xMidYMid meet\"",
        V,
    ),
    row(
        SvgFeature::ImageFitCover,
        "ImageFit::Cover",
        "preserveAspectRatio=\"xMidYMid slice\"(상자 밖은 viewport가 자름)",
        V,
    ),
    row(
        SvgFeature::ImageFitNone,
        "ImageFit::None(CSS object-fit: none과 같은 의미: image pixel 1 = scene 단위 1, 중앙 정렬)",
        "viewBox를 상자 크기로 두고 중앙 정렬, preserveAspectRatio=\"none\"",
        V,
    ),
    row(
        SvgFeature::ImageCrop,
        "ImageCrop(정규화 0..1, 전체가 아님)",
        "viewBox = crop 영역(pixel), 안쪽 <svg overflow=hidden>로 crop 밖을 자름",
        V,
    ),
    row(
        SvgFeature::ImageNearestSampling,
        "ImageSampling::Nearest",
        "style=\"image-rendering:pixelated\"",
        V,
    ),
    row(
        SvgFeature::ImageColorSpaceOverride,
        "ImagePrimitive::color_space override",
        "SVG에 image 색공간 재해석 수단 없음",
        SvgSupport::Rasterize { row: "XX.4.2" },
    ),
    row(
        SvgFeature::SolidFill,
        "PaintLayer::Fill(Paint::Solid) — Frame 배경·Fill modifier",
        "fill=<color> 요소(layer마다 하나, 아래 → 위)",
        V,
    ),
    row(
        SvgFeature::SrgbColor,
        "Color sRGB, 채널 [0, 1]",
        "rgb(R%,G%,B%)",
        V,
    ),
    row(
        SvgFeature::WideGamutColor,
        "Color Display P3 또는 [0, 1] 밖 sRGB",
        "CSS Color 4 color(display-p3 r g b) / color(srgb r g b)",
        V,
    ),
    row(
        SvgFeature::TranslucentColor,
        "Color 알파 < 1(비연관 알파)",
        "fill-opacity / stroke-opacity",
        V,
    ),
    row(
        SvgFeature::CenterStroke,
        "StrokeAlign::Center — Shape·Path·Text stroke",
        "fill=\"none\" stroke stroke-width(join은 XI.6.2 미정의, SVG 기본 miter·limit 4)",
        V,
    ),
    row(
        SvgFeature::InsideStroke,
        "StrokeAlign::Inside — Frame 테두리·Image stroke",
        "stroke-width = 2×폭 + 자기 geometry <clipPath>",
        V,
    ),
    row(
        SvgFeature::OutsideStroke,
        "StrokeAlign::Outside",
        "stroke-width = 2×폭 + geometry 밖 <mask maskUnits=userSpaceOnUse>(흰 영역 위 geometry 검정, XX.3.4)",
        V,
    ),
    row(
        SvgFeature::RoundCap,
        "StrokeCap::Round(Line shape LineCap)",
        "stroke-linecap=\"round\"(Butt는 기본값이라 생략)",
        V,
    ),
    row(
        SvgFeature::SquareCap,
        "StrokeCap::Square",
        "stroke-linecap=\"square\"",
        V,
    ),
    row(
        SvgFeature::DropShadow,
        "PaintStack::shadows(Shadow modifier)",
        "feDropShadow blur 반경 의미가 렌더러 수식(XI.10.4)과 다름",
        SvgSupport::Rasterize { row: "XX.4.2" },
    ),
    row(
        SvgFeature::LayerBlur,
        "PaintStack::layer_blur > 0(Blur modifier)",
        "feGaussianBlur 반경 의미가 렌더러 수식(XI.10.4)과 다름",
        SvgSupport::Rasterize { row: "XX.4.2" },
    ),
    row(
        SvgFeature::GroupOpacity,
        "PaintStack::opacity < 1(Opacity modifier, subtree 합성 결과에 적용)",
        "node 그룹 <g opacity>",
        V,
    ),
    row(
        SvgFeature::BlendMode,
        "PaintStack::blend ≠ Normal(Blend modifier)",
        "mix-blend-mode — 외부 backdrop 의존 범위 분석(XX.4.1) 후 래스터 경계 포함",
        SvgSupport::Rasterize { row: "XX.4.2" },
    ),
    row(
        SvgFeature::RectClip,
        "Frame bounds clip: ClipShape::Rect, radius 0",
        "<clipPath><rect transform></clipPath> + clip-path=\"url(#id)\"",
        V,
    ),
    row(
        SvgFeature::RoundedRectClip,
        "Frame bounds clip: ClipShape::Rect, radius > 0",
        "<clipPath><path d(원호) transform></clipPath>",
        V,
    ),
    row(
        SvgFeature::PathClip,
        "Frame PathRef clip: ClipShape::Path",
        "<clipPath><path d clip-rule transform></clipPath>",
        V,
    ),
    row(
        SvgFeature::NestedClip,
        "clip stack 깊이 ≥ 2(조상 Frame clip 교집합)",
        "안쪽 <clipPath clip-path=\"url(#바깥)\"> chain",
        V,
    ),
    row(
        SvgFeature::WorldTransform,
        "SceneNode::world_transform(조상 변환 합성 결과)",
        "내용 요소 transform=\"matrix(a b c d e f)\"(항등은 생략)",
        V,
    ),
    row(
        SvgFeature::HiddenNode,
        "Visibility::Hidden(실효 값, 자손도 숨김)",
        "요소를 만들지 않음(subtree 생략)",
        V,
    ),
    row(
        SvgFeature::LinearGradient,
        "core-types Gradient(III.4.3) — scene Paint에 아직 없음",
        "<linearGradient gradientUnits=\"userSpaceOnUse\"> + stop 시각 순서",
        SvgSupport::NotInScene,
    ),
    row(
        SvgFeature::RadialGradient,
        "core-types Gradient(III.4.3) — scene Paint에 아직 없음",
        "<radialGradient gradientUnits=\"userSpaceOnUse\"> + stop 시각 순서",
        SvgSupport::NotInScene,
    ),
];

/// sRGB `[0, 1]` 안의 색이면 참입니다(그 외는 CSS Color 4 표현이 필요합니다).
pub fn is_srgb_in_gamut(color: Color) -> bool {
    color.space() == ColorSpace::Srgb
        && [color.r(), color.g(), color.b()]
            .iter()
            .all(|c| (0.0..=1.0).contains(c))
}

fn color_features(color: Color, out: &mut BTreeSet<SvgFeature>) {
    out.insert(if is_srgb_in_gamut(color) {
        SvgFeature::SrgbColor
    } else {
        SvgFeature::WideGamutColor
    });
    if color.alpha() < 1.0 {
        out.insert(SvgFeature::TranslucentColor);
    }
}

fn paint_features(paint: Paint, out: &mut BTreeSet<SvgFeature>) {
    match paint {
        Paint::Solid(color) => color_features(color, out),
    }
}

/// scene node(자식 제외)가 쓰는 기능 집합.
///
/// 숨긴 node도 내용을 분류하며 [`SvgFeature::HiddenNode`]를 함께 넣습니다. writer는 숨김을 먼저
/// 확인해 subtree를 생략하므로 숨긴 node의 다른 기능은 결과에 영향이 없습니다.
pub fn node_features(node: &SceneNode) -> BTreeSet<SvgFeature> {
    let mut out = BTreeSet::new();
    match node.content() {
        SceneContent::Rect(rect) => {
            out.insert(if rect.radii.is_zero() {
                SvgFeature::RectContent
            } else {
                SvgFeature::RoundedRectContent
            });
        }
        SceneContent::Path(path) => {
            out.insert(SvgFeature::PathContent);
            match path.fill_rule {
                FillRule::NonZero => {}
                FillRule::EvenOdd => {
                    out.insert(SvgFeature::EvenOddFill);
                }
            }
        }
        SceneContent::Text(_) => {
            out.insert(SvgFeature::TextContent);
        }
        SceneContent::Image(image) => {
            out.insert(SvgFeature::ImageContent);
            out.insert(match image.fit {
                ImageFit::Fill => SvgFeature::ImageFitFill,
                ImageFit::Contain => SvgFeature::ImageFitContain,
                ImageFit::Cover => SvgFeature::ImageFitCover,
                ImageFit::None => SvgFeature::ImageFitNone,
            });
            if image.crop != ImageCrop::FULL {
                out.insert(SvgFeature::ImageCrop);
            }
            match image.sampling {
                ImageSampling::Linear => {}
                ImageSampling::Nearest => {
                    out.insert(SvgFeature::ImageNearestSampling);
                }
            }
            if image.color_space.is_some() {
                out.insert(SvgFeature::ImageColorSpaceOverride);
            }
        }
    }
    let paint = node.paint().get();
    for layer in &paint.layers {
        match layer {
            PaintLayer::Fill(fill) => {
                out.insert(SvgFeature::SolidFill);
                paint_features(*fill, &mut out);
            }
            PaintLayer::Stroke(stroke) => {
                out.insert(match stroke.align() {
                    StrokeAlign::Center => SvgFeature::CenterStroke,
                    StrokeAlign::Inside => SvgFeature::InsideStroke,
                    StrokeAlign::Outside => SvgFeature::OutsideStroke,
                });
                match stroke.cap() {
                    StrokeCap::Butt => {}
                    StrokeCap::Round => {
                        out.insert(SvgFeature::RoundCap);
                    }
                    StrokeCap::Square => {
                        out.insert(SvgFeature::SquareCap);
                    }
                }
                paint_features(stroke.paint(), &mut out);
            }
        }
    }
    if !paint.shadows.is_empty() {
        out.insert(SvgFeature::DropShadow);
    }
    if paint.layer_blur.get() > 0.0 {
        out.insert(SvgFeature::LayerBlur);
    }
    if paint.opacity != Opacity::OPAQUE {
        out.insert(SvgFeature::GroupOpacity);
    }
    match paint.blend {
        SceneBlendMode::Normal => {}
        SceneBlendMode::Multiply
        | SceneBlendMode::Screen
        | SceneBlendMode::Overlay
        | SceneBlendMode::Darken
        | SceneBlendMode::Lighten => {
            out.insert(SvgFeature::BlendMode);
        }
    }
    for clip in node.clip().iter_inner_first() {
        out.insert(match clip {
            ClipShape::Rect { radii, .. } if radii.is_zero() => SvgFeature::RectClip,
            ClipShape::Rect { .. } => SvgFeature::RoundedRectClip,
            ClipShape::Path { .. } => SvgFeature::PathClip,
        });
    }
    if node.clip().depth() >= 2 {
        out.insert(SvgFeature::NestedClip);
    }
    if node.world_transform() != stellar_core_types::value::Transform2D::identity() {
        out.insert(SvgFeature::WorldTransform);
    }
    match node.hit().visibility {
        Visibility::Visible => {}
        Visibility::Hidden => {
            out.insert(SvgFeature::HiddenNode);
        }
    }
    out
}

/// 기능 집합에서 지금 기록할 수 없는 첫 기능(선언 순서).
pub fn first_unsupported(features: &BTreeSet<SvgFeature>) -> Option<SvgFeature> {
    features
        .iter()
        .copied()
        .find(|feature| !feature.support().is_vector())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use stellar_core_types::reference::AssetRef;
    use stellar_core_types::shadow::{BlurRadius, Shadow};
    use stellar_core_types::value::Transform2D;
    use stellar_scene::geometry::PathGeometry;
    use stellar_scene::node::{
        CornerRadii, HitTestMeta, ImagePrimitive, PathPrimitive, PointerEvents, RectPrimitive,
        SceneNodeSpec, TextLayoutKey, TextPrimitive, ZOrder,
    };
    use stellar_scene::stack::{ClipStackRef, PaintStack, PaintStackRef, StrokeLayer};

    use crate::test_fixtures as fx;

    fn spec(content: SceneContent) -> SceneNodeSpec {
        SceneNodeSpec {
            source: fx::node_id(1),
            content,
            world_transform: Transform2D::identity(),
            layout_rect: fx::rect(0.0, 0.0, 10.0, 10.0),
            clip: ClipStackRef::none(),
            paint: PaintStackRef::empty(),
            z: ZOrder(1),
            hit: HitTestMeta::INTERACTIVE,
        }
    }

    fn rect_content(radius: f32) -> SceneContent {
        SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 10.0, 10.0),
            radii: CornerRadii::new(radius, radius, radius, radius).unwrap(),
        })
    }

    fn image_content(fit: ImageFit, crop: ImageCrop, sampling: ImageSampling) -> SceneContent {
        SceneContent::Image(ImagePrimitive {
            asset: AssetRef::new(fx::node_id(9)),
            bounds: fx::rect(0.0, 0.0, 10.0, 10.0),
            fit,
            crop,
            sampling,
            color_space: None,
        })
    }

    fn features(spec: SceneNodeSpec) -> BTreeSet<SvgFeature> {
        node_features(&SceneNode::leaf(spec))
    }

    fn set(items: &[SvgFeature]) -> BTreeSet<SvgFeature> {
        items.iter().copied().collect()
    }

    fn painted(layers: Vec<PaintLayer>) -> PaintStackRef {
        PaintStackRef::new(PaintStack {
            layers,
            ..PaintStack::empty()
        })
    }

    /// scene enum의 모든 variant를 하나 이상 쓰는 node 목록.
    fn every_scene_variant() -> Vec<SceneNodeSpec> {
        let red = Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 1.0));
        let p3 = Paint::Solid(Color::new(ColorSpace::DisplayP3, 1.0, 0.5, 0.0, 0.5).unwrap());
        let mut specs = vec![
            spec(rect_content(0.0)),
            spec(rect_content(4.0)),
            spec(SceneContent::Path(PathPrimitive {
                geometry: fx::triangle(),
                fill_rule: FillRule::NonZero,
            })),
            spec(SceneContent::Path(PathPrimitive {
                geometry: PathGeometry::empty(),
                fill_rule: FillRule::EvenOdd,
            })),
            spec(SceneContent::Text(TextPrimitive {
                layout: TextLayoutKey::from_bytes([1; 16]),
                bounds: fx::rect(0.0, 0.0, 10.0, 10.0),
            })),
        ];
        for fit in [
            ImageFit::Fill,
            ImageFit::Contain,
            ImageFit::Cover,
            ImageFit::None,
        ] {
            specs.push(spec(image_content(
                fit,
                ImageCrop::FULL,
                ImageSampling::Linear,
            )));
        }
        specs.push(spec(image_content(
            ImageFit::Cover,
            ImageCrop::new(0.25, 0.25, 0.5, 0.5).unwrap(),
            ImageSampling::Nearest,
        )));
        let mut override_space = spec(image_content(
            ImageFit::Contain,
            ImageCrop::FULL,
            ImageSampling::Linear,
        ));
        if let SceneContent::Image(image) = &mut override_space.content {
            image.color_space = Some(ColorSpace::DisplayP3);
        }
        specs.push(override_space);
        for align in [
            StrokeAlign::Center,
            StrokeAlign::Inside,
            StrokeAlign::Outside,
        ] {
            for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
                let mut s = spec(rect_content(0.0));
                s.paint = painted(vec![
                    PaintLayer::Fill(p3),
                    PaintLayer::Stroke(StrokeLayer::new(red, 2.0, align, cap).unwrap()),
                ]);
                specs.push(s);
            }
        }
        for blend in SceneBlendMode::ALL {
            let mut s = spec(rect_content(0.0));
            s.paint = PaintStackRef::new(PaintStack {
                blend,
                opacity: Opacity::new(0.5).unwrap(),
                ..PaintStack::empty()
            });
            specs.push(s);
        }
        let mut effects = spec(rect_content(0.0));
        effects.paint = PaintStackRef::new(PaintStack {
            shadows: vec![Shadow::new(
                stellar_core_types::value::Vec2::new(1.0, 1.0).unwrap(),
                BlurRadius::new(2.0).unwrap(),
                fx::srgb(0.0, 0.0, 0.0, 0.5),
            )],
            layer_blur: BlurRadius::new(3.0).unwrap(),
            ..PaintStack::empty()
        });
        specs.push(effects);
        let clips = ClipStackRef::none()
            .push(ClipShape::Rect {
                rect: fx::rect(0.0, 0.0, 10.0, 10.0),
                radii: CornerRadii::ZERO,
                transform: Transform2D::identity(),
            })
            .push(ClipShape::Rect {
                rect: fx::rect(0.0, 0.0, 10.0, 10.0),
                radii: CornerRadii::new(1.0, 1.0, 1.0, 1.0).unwrap(),
                transform: Transform2D::identity(),
            })
            .push(ClipShape::Path {
                geometry: fx::triangle(),
                fill_rule: FillRule::EvenOdd,
                transform: fx::translate(1.0, 2.0),
            });
        let mut clipped = spec(rect_content(0.0));
        clipped.clip = clips;
        clipped.world_transform = fx::translate(3.0, 4.0);
        clipped.hit = HitTestMeta {
            visibility: Visibility::Hidden,
            pointer_events: PointerEvents::None,
        };
        specs.push(clipped);
        specs
    }

    #[test]
    fn mapping_table_covers_every_feature_once_in_order() {
        // 검증 기준(XX.3.1): mapping inventory 통과.
        assert_eq!(SVG_MAPPING.len(), SvgFeature::ALL.len());
        for (index, (row, feature)) in SVG_MAPPING.iter().zip(SvgFeature::ALL).enumerate() {
            assert_eq!(row.feature, feature, "row {index} 순서");
            assert_eq!(feature as usize, index, "판별값 = index");
            assert_eq!(feature.mapping().feature, feature);
            assert!(!row.scene.is_empty() && !row.svg.is_empty());
        }
        let unique: BTreeSet<SvgFeature> = SvgFeature::ALL.into_iter().collect();
        assert_eq!(unique.len(), SvgFeature::ALL.len());
    }

    #[test]
    fn required_basic_support_is_vector() {
        // Frame/Shape/Path/Image/clip 기본 지원은 vector, gradient는 대응 요소를 명시합니다.
        for feature in [
            SvgFeature::RectContent,
            SvgFeature::RoundedRectContent,
            SvgFeature::PathContent,
            SvgFeature::EvenOddFill,
            SvgFeature::TextContent,
            SvgFeature::ImageContent,
            SvgFeature::ImageFitFill,
            SvgFeature::ImageFitContain,
            SvgFeature::ImageFitCover,
            SvgFeature::ImageFitNone,
            SvgFeature::ImageCrop,
            SvgFeature::SolidFill,
            SvgFeature::CenterStroke,
            SvgFeature::InsideStroke,
            SvgFeature::OutsideStroke,
            SvgFeature::GroupOpacity,
            SvgFeature::TranslucentColor,
            SvgFeature::RectClip,
            SvgFeature::RoundedRectClip,
            SvgFeature::PathClip,
            SvgFeature::NestedClip,
            SvgFeature::WorldTransform,
        ] {
            assert_eq!(feature.support(), SvgSupport::Vector, "{feature}");
        }
        let non_vector: Vec<(SvgFeature, SvgSupport)> = SvgFeature::ALL
            .into_iter()
            .filter(|f| !f.support().is_vector())
            .map(|f| (f, f.support()))
            .collect();
        assert_eq!(
            non_vector,
            vec![
                (
                    SvgFeature::ImageColorSpaceOverride,
                    SvgSupport::Rasterize { row: "XX.4.2" }
                ),
                (
                    SvgFeature::DropShadow,
                    SvgSupport::Rasterize { row: "XX.4.2" }
                ),
                (
                    SvgFeature::LayerBlur,
                    SvgSupport::Rasterize { row: "XX.4.2" }
                ),
                (
                    SvgFeature::BlendMode,
                    SvgSupport::Rasterize { row: "XX.4.2" }
                ),
                (SvgFeature::LinearGradient, SvgSupport::NotInScene),
                (SvgFeature::RadialGradient, SvgSupport::NotInScene),
            ]
        );
        assert!(SvgFeature::LinearGradient
            .mapping()
            .svg
            .contains("<linearGradient"));
        assert!(SvgFeature::RadialGradient
            .mapping()
            .svg
            .contains("<radialGradient"));
    }

    #[test]
    fn every_scene_variant_maps_to_a_table_row() {
        // scene의 모든 variant 조합을 분류하면, scene에 없는 gradient를 뺀 모든 row가 쓰입니다.
        let mut reached = BTreeSet::new();
        for spec in every_scene_variant() {
            reached.extend(features(spec));
        }
        let expected: BTreeSet<SvgFeature> = SvgFeature::ALL
            .into_iter()
            .filter(|f| f.support() != SvgSupport::NotInScene)
            .collect();
        assert_eq!(reached, expected);
    }

    #[test]
    fn classification_is_exact_per_node() {
        assert_eq!(
            features(spec(rect_content(0.0))),
            set(&[SvgFeature::RectContent])
        );
        assert_eq!(
            features(spec(rect_content(2.0))),
            set(&[SvgFeature::RoundedRectContent])
        );
        let mut frame = spec(rect_content(0.0));
        frame.paint = painted(vec![
            PaintLayer::Fill(Paint::Solid(fx::srgb(1.0, 1.0, 1.0, 1.0))),
            PaintLayer::Stroke(
                StrokeLayer::new(
                    Paint::Solid(fx::srgb(0.0, 0.0, 0.0, 0.5)),
                    2.0,
                    StrokeAlign::Inside,
                    StrokeCap::Butt,
                )
                .unwrap(),
            ),
        ]);
        frame.world_transform = fx::translate(1.0, 0.0);
        assert_eq!(
            features(frame),
            set(&[
                SvgFeature::RectContent,
                SvgFeature::SolidFill,
                SvgFeature::SrgbColor,
                SvgFeature::TranslucentColor,
                SvgFeature::InsideStroke,
                SvgFeature::WorldTransform,
            ])
        );
        // [0, 1] 밖 sRGB는 wide gamut 표현이 필요합니다.
        let mut extended = spec(rect_content(0.0));
        extended.paint = painted(vec![PaintLayer::Fill(Paint::Solid(fx::srgb(
            1.2, 0.0, 0.0, 1.0,
        )))]);
        assert_eq!(
            features(extended),
            set(&[
                SvgFeature::RectContent,
                SvgFeature::SolidFill,
                SvgFeature::WideGamutColor
            ])
        );
        let text = spec(SceneContent::Text(TextPrimitive {
            layout: TextLayoutKey::from_bytes([2; 16]),
            bounds: fx::rect(0.0, 0.0, 1.0, 1.0),
        }));
        let text_features = features(text);
        assert_eq!(text_features, set(&[SvgFeature::TextContent]));
        assert_eq!(first_unsupported(&text_features), None);
        assert_eq!(first_unsupported(&set(&[SvgFeature::RectContent])), None);
    }

    #[test]
    fn basic_fixture_is_fully_vector() {
        let roots = fx::basic_roots(&fx::Basic::default());
        let mut stack: Vec<Arc<SceneNode>> = roots;
        let mut all = BTreeSet::new();
        while let Some(node) = stack.pop() {
            all.extend(node_features(&node));
            stack.extend(node.children().cloned());
        }
        assert_eq!(first_unsupported(&all), None);
        assert!(all.contains(&SvgFeature::RoundedRectClip));
        assert!(all.contains(&SvgFeature::ImageFitCover));
    }

    #[test]
    fn support_display_is_descriptive() {
        assert_eq!(SvgSupport::Vector.to_string(), "vector");
        assert!(SvgSupport::Deferred { row: "XX.3.2" }
            .to_string()
            .contains("XX.3.2"));
        assert!(SvgSupport::Rasterize { row: "XX.4.2" }
            .to_string()
            .contains("XX.4.2"));
        assert!(!SvgSupport::NotInScene.to_string().is_empty());
        assert_eq!(SvgFeature::PathClip.to_string(), "PathClip");
    }
}
