//! Scene primitive별 SVG 요소 표기 (XX.3.1).
//!
//! [`crate::svg::mapping`] 표의 vector row를 실제 요소 문자열로 씁니다. 각 함수는 요소 하나(또는
//! `<clipPath>`·image 묶음 하나)를 줄 단위(`\n` 끝)로 씁니다. 속성 순서는 고정입니다: 기하 →
//! 칠하기 → `transform` → `clip-path`.
//!
//! 이 모듈은 id를 만들지 않습니다. 호출자가 넘긴 id는 [`is_safe_id`]를 통과해야 하며, 아니면
//! [`SvgValueError::UnsafeId`]입니다.

use stellar_core_types::color::Color;
use stellar_core_types::value::{Rect, Transform2D};
use stellar_scene::bounds::Aabb;
use stellar_scene::geometry::{FillRule, PathGeometry};
use stellar_scene::node::{
    CornerRadii, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, SceneContent,
};
use stellar_scene::stack::Paint;
use stellar_scene::stack::{ClipShape, Opacity, StrokeAlign, StrokeCap, StrokeLayer};

use crate::capture::EncodedImage;
use crate::svg::format::{
    color_value, is_safe_id, path_data, push_base64, rounded_rect_path_data, transform_value,
    write_number, write_numbers, SvgValueError,
};

/// SVG로 그릴 기하.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgGeometry<'a> {
    /// 직각 사각형(`<rect>`).
    Rect(Rect),
    /// 둥근 모서리 사각형(`<path>` 원호).
    RoundedRect(Rect, CornerRadii),
    /// path(`<path>`).
    Path(&'a PathGeometry, FillRule),
}

impl<'a> SvgGeometry<'a> {
    /// content의 칠하기 기하. Text는 glyph outline(XX.3.2) 전이라 `None`, Image는 배치 상자입니다.
    pub fn of_content(content: &'a SceneContent) -> Option<SvgGeometry<'a>> {
        match content {
            SceneContent::Rect(rect) => Some(if rect.radii.is_zero() {
                SvgGeometry::Rect(rect.rect)
            } else {
                SvgGeometry::RoundedRect(rect.rect, rect.radii)
            }),
            SceneContent::Path(path) => Some(SvgGeometry::Path(&path.geometry, path.fill_rule)),
            SceneContent::Text(_) => None,
            SceneContent::Image(image) => Some(SvgGeometry::Rect(image.bounds)),
        }
    }

    /// clip 기하와 clip world transform.
    pub fn of_clip(clip: &'a ClipShape) -> (SvgGeometry<'a>, Transform2D) {
        match clip {
            ClipShape::Rect {
                rect,
                radii,
                transform,
            } => (
                if radii.is_zero() {
                    SvgGeometry::Rect(*rect)
                } else {
                    SvgGeometry::RoundedRect(*rect, *radii)
                },
                *transform,
            ),
            ClipShape::Path {
                geometry,
                fill_rule,
                transform,
            } => (SvgGeometry::Path(geometry, *fill_rule), *transform),
        }
    }

    /// 아무것도 덮지 않는 빈 path면 참입니다.
    pub fn is_empty_path(&self) -> bool {
        matches!(self, SvgGeometry::Path(geometry, _) if geometry.is_empty())
    }

    fn is_even_odd(&self) -> bool {
        matches!(self, SvgGeometry::Path(_, FillRule::EvenOdd))
    }
}

fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(value);
    out.push('"');
}

fn attr_number(out: &mut String, name: &str, value: f32) -> Result<(), SvgValueError> {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    write_number(out, value)?;
    out.push('"');
    Ok(())
}

fn attr_url(out: &mut String, name: &str, id: &str) -> Result<(), SvgValueError> {
    if !is_safe_id(id) {
        return Err(SvgValueError::UnsafeId);
    }
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"url(#");
    out.push_str(id);
    out.push_str(")\"");
    Ok(())
}

fn attr_id(out: &mut String, id: &str) -> Result<(), SvgValueError> {
    if !is_safe_id(id) {
        return Err(SvgValueError::UnsafeId);
    }
    attr(out, "id", id);
    Ok(())
}

fn attr_transform(out: &mut String, transform: &Transform2D) -> Result<(), SvgValueError> {
    if let Some(value) = transform_value(transform)? {
        attr(out, "transform", &value);
    }
    Ok(())
}

fn attr_rect(out: &mut String, rect: Rect) -> Result<(), SvgValueError> {
    attr_number(out, "x", rect.origin().x())?;
    attr_number(out, "y", rect.origin().y())?;
    attr_number(out, "width", rect.size().width())?;
    attr_number(out, "height", rect.size().height())
}

/// 요소 이름과 기하 속성까지 씁니다(`<rect x y width height` 또는 `<path d`).
fn open_shape(out: &mut String, geometry: &SvgGeometry<'_>) -> Result<(), SvgValueError> {
    match geometry {
        SvgGeometry::Rect(rect) => {
            out.push_str("<rect");
            attr_rect(out, *rect)
        }
        SvgGeometry::RoundedRect(rect, radii) => {
            out.push_str("<path");
            attr(out, "d", &rounded_rect_path_data(*rect, *radii)?);
            Ok(())
        }
        SvgGeometry::Path(path, _) => {
            out.push_str("<path");
            attr(out, "d", &path_data(path)?);
            Ok(())
        }
    }
}

fn attr_color(
    out: &mut String,
    paint_attr: &str,
    opacity_attr: &str,
    color: Color,
) -> Result<(), SvgValueError> {
    let value = color_value(color)?;
    attr(out, paint_attr, &value.paint);
    if let Some(opacity) = value.opacity {
        attr_number(out, opacity_attr, opacity)?;
    }
    Ok(())
}

/// 채우기 layer 하나. 빈 path는 아무것도 쓰지 않습니다.
pub fn write_fill(
    out: &mut String,
    geometry: &SvgGeometry<'_>,
    transform: &Transform2D,
    paint: Paint,
) -> Result<(), SvgValueError> {
    if geometry.is_empty_path() {
        return Ok(());
    }
    let mut element = String::new();
    open_shape(&mut element, geometry)?;
    match paint {
        Paint::Solid(color) => attr_color(&mut element, "fill", "fill-opacity", color)?,
    }
    if geometry.is_even_odd() {
        attr(&mut element, "fill-rule", "evenodd");
    }
    attr_transform(&mut element, transform)?;
    element.push_str("/>\n");
    out.push_str(&element);
    Ok(())
}

/// stroke layer 하나. 빈 path는 아무것도 쓰지 않습니다.
///
/// Center는 `inside_clip`이 `None`이어야 합니다. Inside는 폭을 두 배로 그리고 자기 기하
/// clip(`inside_clip`, [`write_geometry_clip_def`])으로 바깥 절반을 자릅니다. Outside는 mask가
/// 필요해(XX.3.4) [`SvgValueError::Unrepresentable`]입니다.
pub fn write_stroke(
    out: &mut String,
    geometry: &SvgGeometry<'_>,
    transform: &Transform2D,
    stroke: &StrokeLayer,
    inside_clip: Option<&str>,
) -> Result<(), SvgValueError> {
    let width = match (stroke.align(), inside_clip) {
        (StrokeAlign::Center, None) => stroke.width(),
        (StrokeAlign::Inside, Some(_)) => stroke.width() * 2.0,
        _ => return Err(SvgValueError::Unrepresentable),
    };
    if geometry.is_empty_path() {
        return Ok(());
    }
    let mut element = String::new();
    open_shape(&mut element, geometry)?;
    attr(&mut element, "fill", "none");
    match stroke.paint() {
        Paint::Solid(color) => attr_color(&mut element, "stroke", "stroke-opacity", color)?,
    }
    attr_number(&mut element, "stroke-width", width)?;
    match stroke.cap() {
        StrokeCap::Butt => {}
        StrokeCap::Round => attr(&mut element, "stroke-linecap", "round"),
        StrokeCap::Square => attr(&mut element, "stroke-linecap", "square"),
    }
    attr_transform(&mut element, transform)?;
    if let Some(clip) = inside_clip {
        attr_url(&mut element, "clip-path", clip)?;
    }
    element.push_str("/>\n");
    out.push_str(&element);
    Ok(())
}

fn write_clip_body(
    out: &mut String,
    geometry: &SvgGeometry<'_>,
    transform: &Transform2D,
) -> Result<(), SvgValueError> {
    // 빈 path clip은 자식 없는 clipPath(모두 가림)입니다.
    if geometry.is_empty_path() {
        return Ok(());
    }
    open_shape(out, geometry)?;
    if geometry.is_even_odd() {
        attr(out, "clip-rule", "evenodd");
    }
    attr_transform(out, transform)?;
    out.push_str("/>\n");
    Ok(())
}

/// Inside stroke용 자기 기하 `<clipPath>`(node local 좌표, 참조 요소의 transform을 따름).
pub fn write_geometry_clip_def(
    out: &mut String,
    id: &str,
    geometry: &SvgGeometry<'_>,
) -> Result<(), SvgValueError> {
    let mut element = String::from("<clipPath");
    attr_id(&mut element, id)?;
    element.push_str(">\n");
    write_clip_body(&mut element, geometry, &Transform2D::identity())?;
    element.push_str("</clipPath>\n");
    out.push_str(&element);
    Ok(())
}

/// scene clip 하나의 `<clipPath>`(world 좌표). `parent`가 있으면 그 clip과 교집합입니다.
pub fn write_clip_def(
    out: &mut String,
    id: &str,
    clip: &ClipShape,
    parent: Option<&str>,
) -> Result<(), SvgValueError> {
    let (geometry, transform) = SvgGeometry::of_clip(clip);
    let mut element = String::from("<clipPath");
    attr_id(&mut element, id)?;
    if let Some(parent) = parent {
        attr_url(&mut element, "clip-path", parent)?;
    }
    element.push_str(">\n");
    write_clip_body(&mut element, &geometry, &transform)?;
    element.push_str("</clipPath>\n");
    out.push_str(&element);
    Ok(())
}

/// node 그룹을 엽니다. opacity 1은 생략합니다.
pub fn write_group_open(
    out: &mut String,
    id: &str,
    clip: Option<&str>,
    opacity: Opacity,
) -> Result<(), SvgValueError> {
    let mut element = String::from("<g");
    attr_id(&mut element, id)?;
    if let Some(clip) = clip {
        attr_url(&mut element, "clip-path", clip)?;
    }
    if opacity != Opacity::OPAQUE {
        attr_number(&mut element, "opacity", opacity.get())?;
    }
    element.push_str(">\n");
    out.push_str(&element);
    Ok(())
}

/// node 그룹을 닫습니다.
pub fn write_group_close(out: &mut String) {
    out.push_str("</g>\n");
}

/// 내보내기 영역 전체를 덮는 배경 사각형(world 좌표).
pub fn write_background(out: &mut String, bounds: Aabb, color: Color) -> Result<(), SvgValueError> {
    let mut element = String::from("<rect");
    attr_number(&mut element, "x", bounds.min_x())?;
    attr_number(&mut element, "y", bounds.min_y())?;
    attr_number(&mut element, "width", bounds.width())?;
    attr_number(&mut element, "height", bounds.height())?;
    attr_color(&mut element, "fill", "fill-opacity", color)?;
    element.push_str("/>\n");
    out.push_str(&element);
    Ok(())
}

/// image content(픽셀). 칠하기 layer는 호출자가 이 뒤(위)에 씁니다.
///
/// 바깥 `<svg>`가 배치 상자이고 viewBox는 image pixel 좌표입니다. crop이 전체가 아니면 안쪽
/// `<svg>`가 crop 영역 밖 pixel을 자릅니다(Contain 여백·None 여백에 crop 밖이 보이지 않게).
/// 색공간 override는 표현할 수 없어(래스터화 대상) 이 함수가 반영하지 않습니다.
pub fn write_image(
    out: &mut String,
    image: &ImagePrimitive,
    transform: &Transform2D,
    encoded: &EncodedImage,
) -> Result<(), SvgValueError> {
    let width = encoded.pixel_width() as f32;
    let height = encoded.pixel_height() as f32;
    let (cx, cy, cw, ch) = image.crop.components();
    let (px, py, pw, ph) = (cx * width, cy * height, cw * width, ch * height);
    let bounds = image.bounds;
    let (bw, bh) = (bounds.size().width(), bounds.size().height());
    let (view_box, aspect) = match image.fit {
        ImageFit::Fill => ([px, py, pw, ph], "none"),
        ImageFit::Contain => ([px, py, pw, ph], "xMidYMid meet"),
        ImageFit::Cover => ([px, py, pw, ph], "xMidYMid slice"),
        ImageFit::None => (
            [px + pw / 2.0 - bw / 2.0, py + ph / 2.0 - bh / 2.0, bw, bh],
            "none",
        ),
    };
    let mut element = String::new();
    let transform_attr = transform_value(transform)?;
    if let Some(value) = &transform_attr {
        element.push_str("<g");
        attr(&mut element, "transform", value);
        element.push_str(">\n");
    }
    element.push_str("<svg");
    attr_rect(&mut element, bounds)?;
    element.push_str(" viewBox=\"");
    write_numbers(&mut element, &view_box)?;
    element.push('"');
    attr(&mut element, "preserveAspectRatio", aspect);
    attr(&mut element, "overflow", "hidden");
    element.push_str(">\n");
    let cropped = image.crop != ImageCrop::FULL;
    if cropped {
        element.push_str("<svg");
        attr_number(&mut element, "x", px)?;
        attr_number(&mut element, "y", py)?;
        attr_number(&mut element, "width", pw)?;
        attr_number(&mut element, "height", ph)?;
        element.push_str(" viewBox=\"");
        write_numbers(&mut element, &[px, py, pw, ph])?;
        element.push('"');
        attr(&mut element, "overflow", "hidden");
        element.push_str(">\n");
    }
    element.push_str("<image");
    attr_number(&mut element, "width", width)?;
    attr_number(&mut element, "height", height)?;
    attr(&mut element, "preserveAspectRatio", "none");
    match image.sampling {
        ImageSampling::Linear => {}
        ImageSampling::Nearest => attr(&mut element, "style", "image-rendering:pixelated"),
    }
    element.push_str(" href=\"data:");
    element.push_str(encoded.media().mime());
    element.push_str(";base64,");
    push_base64(&mut element, encoded.bytes());
    element.push_str("\"/>\n");
    if cropped {
        element.push_str("</svg>\n");
    }
    element.push_str("</svg>\n");
    if transform_attr.is_some() {
        element.push_str("</g>\n");
    }
    out.push_str(&element);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::reference::AssetRef;

    use crate::capture::ImageMedia;
    use crate::test_fixtures as fx;

    fn red() -> Paint {
        Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 1.0))
    }

    fn written(f: impl FnOnce(&mut String) -> Result<(), SvgValueError>) -> String {
        let mut out = String::new();
        f(&mut out).unwrap();
        out
    }

    #[test]
    fn rect_and_path_fills() {
        let rect = SvgGeometry::Rect(fx::rect(0.0, 0.0, 100.0, 80.0));
        assert_eq!(
            written(|o| write_fill(o, &rect, &fx::translate(20.0, 10.0), red())),
            "<rect x=\"0\" y=\"0\" width=\"100\" height=\"80\" fill=\"rgb(100%,0%,0%)\" \
             transform=\"matrix(1 0 0 1 20 10)\"/>\n"
        );
        let triangle = fx::triangle();
        let even_odd = SvgGeometry::Path(&triangle, FillRule::EvenOdd);
        assert_eq!(
            written(|o| write_fill(
                o,
                &even_odd,
                &Transform2D::identity(),
                Paint::Solid(Color::new(ColorSpace::DisplayP3, 0.0, 1.0, 0.0, 0.5).unwrap())
            )),
            "<path d=\"M0 0 L40 0 L20 30 Z\" fill=\"color(display-p3 0 1 0)\" \
             fill-opacity=\"0.5\" fill-rule=\"evenodd\"/>\n"
        );
        let empty = PathGeometry::empty();
        assert_eq!(
            written(|o| write_fill(
                o,
                &SvgGeometry::Path(&empty, FillRule::NonZero),
                &Transform2D::identity(),
                red()
            )),
            ""
        );
        let rounded = SvgGeometry::RoundedRect(
            fx::rect(0.0, 0.0, 10.0, 10.0),
            CornerRadii::new(2.0, 2.0, 2.0, 2.0).unwrap(),
        );
        assert!(
            written(|o| write_fill(o, &rounded, &Transform2D::identity(), red()))
                .starts_with("<path d=\"M2 0 L8 0 A2 2 0 0 1 10 2")
        );
    }

    #[test]
    fn strokes_center_inside_and_outside() {
        let triangle = fx::triangle();
        let path = SvgGeometry::Path(&triangle, FillRule::NonZero);
        let blue = Paint::Solid(fx::srgb(0.0, 0.0, 1.0, 0.5));
        let center = StrokeLayer::new(blue, 3.0, StrokeAlign::Center, StrokeCap::Round).unwrap();
        assert_eq!(
            written(|o| write_stroke(o, &path, &Transform2D::identity(), &center, None)),
            "<path d=\"M0 0 L40 0 L20 30 Z\" fill=\"none\" stroke=\"rgb(0%,0%,100%)\" \
             stroke-opacity=\"0.5\" stroke-width=\"3\" stroke-linecap=\"round\"/>\n"
        );
        let rect = SvgGeometry::Rect(fx::rect(0.0, 0.0, 10.0, 10.0));
        let inside = StrokeLayer::new(blue, 2.0, StrokeAlign::Inside, StrokeCap::Square).unwrap();
        assert_eq!(
            written(|o| write_stroke(o, &rect, &fx::translate(1.0, 2.0), &inside, Some("n-a-g"))),
            "<rect x=\"0\" y=\"0\" width=\"10\" height=\"10\" fill=\"none\" \
             stroke=\"rgb(0%,0%,100%)\" stroke-opacity=\"0.5\" stroke-width=\"4\" \
             stroke-linecap=\"square\" transform=\"matrix(1 0 0 1 1 2)\" \
             clip-path=\"url(#n-a-g)\"/>\n"
        );
        let mut out = String::new();
        assert_eq!(
            write_stroke(&mut out, &rect, &Transform2D::identity(), &inside, None),
            Err(SvgValueError::Unrepresentable)
        );
        assert_eq!(
            write_stroke(
                &mut out,
                &rect,
                &Transform2D::identity(),
                &center,
                Some("c")
            ),
            Err(SvgValueError::Unrepresentable)
        );
        let outside = StrokeLayer::new(blue, 2.0, StrokeAlign::Outside, StrokeCap::Butt).unwrap();
        assert_eq!(
            write_stroke(&mut out, &rect, &Transform2D::identity(), &outside, None),
            Err(SvgValueError::Unrepresentable)
        );
        assert_eq!(
            write_stroke(
                &mut out,
                &rect,
                &Transform2D::identity(),
                &inside,
                Some("bad id")
            ),
            Err(SvgValueError::UnsafeId)
        );
        assert!(out.is_empty(), "실패한 요소는 쓰지 않습니다");
    }

    #[test]
    fn clip_definitions_and_groups() {
        let rounded = ClipShape::Rect {
            rect: fx::rect(0.0, 0.0, 100.0, 80.0),
            radii: CornerRadii::new(8.0, 8.0, 8.0, 8.0).unwrap(),
            transform: fx::translate(20.0, 10.0),
        };
        assert_eq!(
            written(|o| write_clip_def(o, "n-f-c1", &rounded, None)),
            "<clipPath id=\"n-f-c1\">\n<path d=\"M8 0 L92 0 A8 8 0 0 1 100 8 L100 72 A8 8 0 0 1 \
             92 80 L8 80 A8 8 0 0 1 0 72 L0 8 A8 8 0 0 1 8 0 Z\" \
             transform=\"matrix(1 0 0 1 20 10)\"/>\n</clipPath>\n"
        );
        let path_clip = ClipShape::Path {
            geometry: fx::triangle(),
            fill_rule: FillRule::EvenOdd,
            transform: Transform2D::identity(),
        };
        assert_eq!(
            written(|o| write_clip_def(o, "n-x-c2", &path_clip, Some("n-f-c1"))),
            "<clipPath id=\"n-x-c2\" clip-path=\"url(#n-f-c1)\">\n<path d=\"M0 0 L40 0 L20 30 Z\" \
             clip-rule=\"evenodd\"/>\n</clipPath>\n"
        );
        let empty = ClipShape::Path {
            geometry: PathGeometry::empty(),
            fill_rule: FillRule::NonZero,
            transform: Transform2D::identity(),
        };
        assert_eq!(
            written(|o| write_clip_def(o, "e", &empty, None)),
            "<clipPath id=\"e\">\n</clipPath>\n"
        );
        assert_eq!(
            written(|o| write_geometry_clip_def(
                o,
                "n-a-g",
                &SvgGeometry::Rect(fx::rect(0.0, 0.0, 4.0, 5.0))
            )),
            "<clipPath id=\"n-a-g\">\n<rect x=\"0\" y=\"0\" width=\"4\" height=\"5\"/>\n</clipPath>\n"
        );
        assert_eq!(
            written(|o| write_group_open(o, "n-a", Some("n-f-c1"), Opacity::new(0.5).unwrap())),
            "<g id=\"n-a\" clip-path=\"url(#n-f-c1)\" opacity=\"0.5\">\n"
        );
        assert_eq!(
            written(|o| write_group_open(o, "n-a", None, Opacity::OPAQUE)),
            "<g id=\"n-a\">\n"
        );
        let mut out = String::new();
        assert_eq!(
            write_group_open(&mut out, "1bad", None, Opacity::OPAQUE),
            Err(SvgValueError::UnsafeId)
        );
        write_group_close(&mut out);
        assert_eq!(out, "</g>\n");
        let bounds = Aabb::new(20.0, 10.0, 120.0, 90.0).unwrap();
        assert_eq!(
            written(|o| write_background(o, bounds, fx::srgb(1.0, 1.0, 1.0, 1.0))),
            "<rect x=\"20\" y=\"10\" width=\"100\" height=\"80\" fill=\"rgb(100%,100%,100%)\"/>\n"
        );
    }

    fn image(fit: ImageFit, crop: ImageCrop, sampling: ImageSampling) -> ImagePrimitive {
        ImagePrimitive {
            asset: AssetRef::new(fx::node_id(9)),
            bounds: fx::rect(0.0, 0.0, 30.0, 20.0),
            fit,
            crop,
            sampling,
            color_space: None,
        }
    }

    fn png() -> EncodedImage {
        EncodedImage::new(ImageMedia::Png, Arc::from(b"foo".to_vec()), 60, 20).unwrap()
    }

    #[test]
    fn image_fit_modes() {
        let cover = written(|o| {
            write_image(
                o,
                &image(ImageFit::Cover, ImageCrop::FULL, ImageSampling::Linear),
                &fx::translate(10.0, 45.0),
                &png(),
            )
        });
        assert_eq!(
            cover,
            "<g transform=\"matrix(1 0 0 1 10 45)\">\n\
             <svg x=\"0\" y=\"0\" width=\"30\" height=\"20\" viewBox=\"0 0 60 20\" \
             preserveAspectRatio=\"xMidYMid slice\" overflow=\"hidden\">\n\
             <image width=\"60\" height=\"20\" preserveAspectRatio=\"none\" \
             href=\"data:image/png;base64,Zm9v\"/>\n</svg>\n</g>\n"
        );
        let aspects = [
            (
                ImageFit::Fill,
                "viewBox=\"0 0 60 20\" preserveAspectRatio=\"none\"",
            ),
            (
                ImageFit::Contain,
                "viewBox=\"0 0 60 20\" preserveAspectRatio=\"xMidYMid meet\"",
            ),
            // None: pixel 1 = 단위 1, 상자(30×20)를 image(60×20) 중앙에 둡니다.
            (
                ImageFit::None,
                "viewBox=\"15 0 30 20\" preserveAspectRatio=\"none\"",
            ),
        ];
        for (fit, expected) in aspects {
            let text = written(|o| {
                write_image(
                    o,
                    &image(fit, ImageCrop::FULL, ImageSampling::Linear),
                    &Transform2D::identity(),
                    &png(),
                )
            });
            assert!(text.contains(expected), "{fit:?}: {text}");
            assert!(!text.starts_with("<g"), "항등 변환은 그룹 없음");
        }
    }

    #[test]
    fn image_crop_and_sampling() {
        let text = written(|o| {
            write_image(
                o,
                &image(
                    ImageFit::Contain,
                    ImageCrop::new(0.5, 0.0, 0.5, 1.0).unwrap(),
                    ImageSampling::Nearest,
                ),
                &Transform2D::identity(),
                &png(),
            )
        });
        assert_eq!(
            text,
            "<svg x=\"0\" y=\"0\" width=\"30\" height=\"20\" viewBox=\"30 0 30 20\" \
             preserveAspectRatio=\"xMidYMid meet\" overflow=\"hidden\">\n\
             <svg x=\"30\" y=\"0\" width=\"30\" height=\"20\" viewBox=\"30 0 30 20\" \
             overflow=\"hidden\">\n\
             <image width=\"60\" height=\"20\" preserveAspectRatio=\"none\" \
             style=\"image-rendering:pixelated\" href=\"data:image/png;base64,Zm9v\"/>\n\
             </svg>\n</svg>\n"
        );
    }

    #[test]
    fn content_geometry_follows_mapping() {
        use stellar_scene::node::{PathPrimitive, RectPrimitive, TextLayoutKey, TextPrimitive};
        let rect = SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 1.0, 1.0),
            radii: CornerRadii::ZERO,
        });
        assert!(matches!(
            SvgGeometry::of_content(&rect),
            Some(SvgGeometry::Rect(_))
        ));
        let rounded = SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 1.0, 1.0),
            radii: CornerRadii::new(0.5, 0.0, 0.0, 0.0).unwrap(),
        });
        assert!(matches!(
            SvgGeometry::of_content(&rounded),
            Some(SvgGeometry::RoundedRect(..))
        ));
        let path = SceneContent::Path(PathPrimitive {
            geometry: fx::triangle(),
            fill_rule: FillRule::EvenOdd,
        });
        assert!(matches!(
            SvgGeometry::of_content(&path),
            Some(SvgGeometry::Path(_, FillRule::EvenOdd))
        ));
        let text = SceneContent::Text(TextPrimitive {
            layout: TextLayoutKey::from_bytes([0; 16]),
            bounds: fx::rect(0.0, 0.0, 1.0, 1.0),
        });
        assert_eq!(SvgGeometry::of_content(&text), None);
        let image = SceneContent::Image(image(
            ImageFit::Fill,
            ImageCrop::FULL,
            ImageSampling::Linear,
        ));
        assert_eq!(
            SvgGeometry::of_content(&image),
            Some(SvgGeometry::Rect(fx::rect(0.0, 0.0, 30.0, 20.0)))
        );
    }
}
