//! 문서 node payload → scene builder 입력 (XI.2.2, ADR-0084, ADR-0116 결정 1).
//!
//! 앱 캔버스 행(`stellar-runtime` canvas)과 증분 scene source([`super::document_scene_source`])가 같은 함수를 씁니다.
//! 규칙은 문서 계층의 순수 함수([`evaluate_shape_modifiers`], [`RectangleParams::effective_corner_radius`] 등)에 있고,
//! 여기서는 그 결과를 scene 값으로 옮기기만 합니다. 평가하지 못하는 입력은 `None`입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{Clipping, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::modifier::{ModifierPayload, ModifierStack};
use stellar_document::path::PathCommand as DocumentPath;
use stellar_document::shape::{
    evaluate_shape_modifiers, evaluate_shape_path, RectangleParams, ShapeGeometry,
    ShapeModifierStyle,
};

use crate::builder::{
    BorderInput, FrameClipInput, FrameInput, ImageInput, NodeInput, ShapeInput, StrokeInput,
};
use crate::geometry::{PathCommand, PathGeometry};
use crate::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling};
use crate::stack::{Paint, StrokeCap};

/// 고정 Free Frame payload를 scene 입력으로 바꿉니다. Path clip은 아직 그리지 않습니다(호출자가 거릅니다).
pub fn frame_input(id: StableId, frame: &FramePayload, locked: bool) -> Option<FrameInput> {
    let radius = RectangleParams {
        size: frame.size,
        corner_radius: frame.corner_radius,
    }
    .effective_corner_radius();
    let radii = CornerRadii::new(radius.0, radius.1, radius.2, radius.3).ok()?;
    let mut node = NodeInput::new(id, frame.transform);
    node.locked = locked;
    Some(FrameInput {
        node,
        size: frame.size,
        corner_radii: radii,
        background: frame.background,
        border: frame.border.map(|b| BorderInput {
            color: b.color,
            thickness: b.thickness(),
        }),
        clip: if frame.clipping == Clipping::FrameBounds {
            FrameClipInput::Bounds
        } else {
            FrameClipInput::None
        },
    })
}

/// 도형이 평가하는 transform·fill·stroke. 규칙은 웹 출력과 같은 [`evaluate_shape_modifiers`] 한 곳에 있고, 평가하지
/// 못하는 stack은 `None`입니다.
pub fn shape_style(stack: Option<&ModifierStack>) -> Option<ShapeModifierStyle> {
    evaluate_shape_modifiers(stack).ok()
}

/// 도형 하나의 scene 입력과 캔버스 행 보조 값.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSource {
    /// scene builder 입력.
    pub input: ShapeInput,
    /// 평가한 modifier 값.
    pub style: ShapeModifierStyle,
    /// Rectangle 실효 모서리 반경.
    pub radii: CornerRadii,
    /// 길이 0인 선의 cap bounds.
    pub point_cap: Option<Rect>,
}

/// 도형 payload와 modifier stack(`stack`)을 scene 입력으로 바꿉니다. 평가하지 못하는 stack이면 `None`입니다.
pub fn shape_source(
    id: StableId,
    shape: &ShapeGeometry,
    stack: Option<&ModifierStack>,
    locked: bool,
) -> Option<ShapeSource> {
    let style = shape_style(stack)?;
    let mut node = NodeInput::new(id, style.transform);
    node.locked = locked;
    if let Some(color) = style.fill {
        node.paint.fills.push(Paint::Solid(color));
    }
    if let Some(stroke) = style.stroke {
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(stroke.color),
            width: stroke.thickness(),
        });
    }
    let mut cap = StrokeCap::Butt;
    let mut point_cap = None;
    let mut radii = CornerRadii::ZERO;
    let bounds = match shape {
        ShapeGeometry::Rectangle(rect) => {
            let (tl, tr, br, bl) = rect.effective_corner_radius();
            radii = CornerRadii::new(tl, tr, br, bl).ok()?;
            Rect::new(Vec2::ZERO, rect.size)
        }
        ShapeGeometry::Ellipse(ellipse) => ellipse.bounds,
        ShapeGeometry::Polygon(polygon) => polygon.bounds(),
        ShapeGeometry::Line(line) => {
            cap = StrokeCap::from_tag(line.cap as u8)?;
            if line.is_zero_length() {
                point_cap = style
                    .stroke
                    .and_then(|stroke| line.stroked_bounds(stroke.thickness()));
            }
            let min = Vec2::new(
                line.start.x().min(line.end.x()),
                line.start.y().min(line.end.y()),
            )
            .ok()?;
            let size = Size2::new(
                (line.end.x() - line.start.x()).abs(),
                (line.end.y() - line.start.y()).abs(),
            )
            .ok()?;
            Rect::new(min, size)
        }
    };
    let path = evaluate_shape_path(shape)
        .ok()?
        .into_iter()
        .map(scene_command)
        .collect();
    Some(ShapeSource {
        input: ShapeInput {
            node,
            layout_rect: bounds,
            geometry: PathGeometry::new(path).ok()?,
            cap,
        },
        style,
        radii,
        point_cap,
    })
}

/// 문서 path command를 scene path command로 옮깁니다(두 모델은 같은 의미, scene `geometry` 모듈 표).
pub fn scene_command(command: DocumentPath) -> PathCommand {
    match command {
        DocumentPath::MoveTo(p) => PathCommand::MoveTo(p),
        DocumentPath::LineTo(p) => PathCommand::LineTo(p),
        DocumentPath::CubicTo { c1, c2, to } => PathCommand::CubicTo { c1, c2, to },
        DocumentPath::Close => PathCommand::Close,
    }
}

/// 문서 `FitMode`를 scene `ImageFit`으로 옮깁니다(두 tag는 같습니다).
pub fn scene_fit(fit: FitMode) -> ImageFit {
    match fit {
        FitMode::Fill => ImageFit::Fill,
        FitMode::Contain => ImageFit::Contain,
        FitMode::Cover => ImageFit::Cover,
        FitMode::None => ImageFit::None,
    }
}

/// 이미지 payload와 modifier stack을 scene 입력으로 바꿉니다. 아직 그리지 않는 crop·sampling·색공간 override와
/// Transform 밖 modifier는 `None`입니다.
pub fn image_input(
    id: StableId,
    image: &ImagePayload,
    stack: Option<&ModifierStack>,
    locked: bool,
) -> Option<ImageInput> {
    if image.crop != CropRect::FULL
        || image.sampling != SamplingMode::Linear
        || image.color_space_override.is_some()
    {
        return None;
    }
    let mut transform = Transform2D::identity();
    for modifier in stack
        .into_iter()
        .flat_map(|s| s.entries())
        .filter(|m| m.enabled)
    {
        let ModifierPayload::Transform(next) = modifier.payload else {
            return None;
        };
        transform = transform.then(&next).ok()?;
    }
    let mut node = NodeInput::new(id, transform);
    node.locked = locked;
    Some(ImageInput {
        node,
        asset: image.asset,
        size: image.size,
        fit: scene_fit(image.fit),
        crop: ImageCrop::new(0., 0., 1., 1.).ok()?,
        sampling: ImageSampling::Linear,
        color_space: None,
    })
}
