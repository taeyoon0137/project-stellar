//! 쓰기 v4 node 레코드: 모든 node 종류와 모든 modifier(ADR-0077 결정 2·3).
//!
//! node 하나는 mapping 하나입니다. 공통 필드(`id`, `parent`, `order_key`, `visible`, `locked`,
//! `display_name`, `kind`) 뒤에 종류별 `payload`를 두고, 문서 node는 마지막에 `modifiers` 목록을 둡니다
//! (컴포넌트 정의 tree node는 modifier stack이 없어 `modifiers`를 쓰지 않습니다).
//!
//! payload 키는 registry 이름·순서([`super::keys`])를 따르고 값은 CSS 문법([`super::css`])입니다. 모델이
//! 표현하는 값만 쓰고, 표현할 수 없는 입력(변마다 다른 테두리, 모르는 키·값)은 거부합니다.
//!
//! 명시 layout longhand(IX.11.6)가 있는 문서 node는 `payload` 뒤에 `layout` mapping을 둡니다. 한 줄이 longhand
//! 하나이고 명시한 longhand만 씁니다(ADR-0086, ADR-0077 개정). 명시 값이 없으면 `layout` 키를 쓰지 않습니다.
//! 읽을 때 없는 longhand는 미설정입니다.
//!
//! Frame payload에서 CSS initial value가 곧 모델 기본값인 키(`transform`, `background-color`, 테두리 열두 줄,
//! `border-*-radius`, `layout-mode`, `clipping`)는 읽을 때 생략을 허용하고 모델 기본값으로 읽습니다. 쓰기는 모든
//! 키를 그대로 씁니다(payload 값은 모델에서 언제나 명시 값입니다). `width`·`height`는 생략할 수 없습니다.

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::{AssetRef, ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{
    Angle, Duration, FiniteF32, Length, Percentage, Rect, Size2, Vec2,
};
use stellar_document::component::definition::{ComponentPropertyId, ComponentSlotId};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use stellar_document::css::border::{BorderLonghand, BorderSides};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::keys as frame_keys;
use stellar_document::frame::{Clipping, CornerRadius, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode, IMAGE_PROP_ASSET};
use stellar_document::layout_record::{format_value, LayoutLonghand, LayoutRecord};
use stellar_document::modifier::{
    BlendMode, BooleanOp, ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::modifier_metadata::keys as modifier_keys;
use stellar_document::path::{
    CommandIds, IdentifiedPath, PathCommand, PathData, PathHandleId, PathPointId, WindingRule,
};
use stellar_document::payload_metadata::{image_keys, shape_keys};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry, ShapeKind,
};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};

use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::css::{self, invalid, unsupported};
use super::keys::{frame_lines, image_lines, modifier_lines, shape_lines, FrameLine};
use crate::yaml::YamlValue;

const COMMON: [&str; 7] = [
    "id",
    "parent",
    "order_key",
    "visible",
    "locked",
    "display_name",
    "kind",
];

fn line(value: impl Into<String>) -> YamlValue {
    YamlValue::scalar(value.into())
}

fn get<'a>(value: &'a YamlValue, key: &str) -> Result<&'a str, TextSourceError> {
    text(field(value, key)?)
}

/// node 종류의 파일 이름.
pub(crate) fn kind_name(payload: &NodePayload) -> &'static str {
    match payload {
        NodePayload::Frame(_) => "frame",
        NodePayload::Fragment(_) => "fragment",
        NodePayload::Shape(shape) => shape.kind().name(),
        NodePayload::Path(_) => "path",
        NodePayload::Text(_) => "text",
        NodePayload::Image(_) => "image",
        NodePayload::ComponentInstance(_) => "component-instance",
    }
}

// --- Frame ---

fn clipping(value: &Clipping) -> String {
    match value {
        Clipping::Disabled => "disabled".into(),
        Clipping::FrameBounds => "frame-bounds".into(),
        Clipping::Path(target) => format!("path {}", target.id()),
    }
}

fn parse_clipping(value: &str) -> Result<Clipping, TextSourceError> {
    match value {
        "disabled" => Ok(Clipping::Disabled),
        "frame-bounds" => Ok(Clipping::FrameBounds),
        other => match other.strip_prefix("path ") {
            Some(target) => Ok(Clipping::Path(NodeRef::new(css::parse_id(target)?))),
            None => Err(unsupported(format!("알 수 없는 clipping 방식: {other}"))),
        },
    }
}

fn background(value: Option<stellar_core_types::color::Color>) -> String {
    value.map_or_else(|| "transparent".into(), css::color)
}

fn parse_background(
    value: &str,
) -> Result<Option<stellar_core_types::color::Color>, TextSourceError> {
    if value == "transparent" {
        Ok(None)
    } else {
        css::parse_color(value).map(Some)
    }
}

fn frame_key_value(payload: &FramePayload, key: PropertyKey) -> Result<String, TextSourceError> {
    let (tl, tr, br, bl) = payload.corner_radius.components();
    Ok(match key {
        frame_keys::TRANSFORM => css::matrix(&payload.transform),
        frame_keys::CLIPPING => clipping(&payload.clipping),
        frame_keys::BACKGROUND => background(payload.background),
        frame_keys::LAYOUT_MODE => css::optional_id(payload.layout_mode),
        frame_keys::WIDTH => css::px(payload.size.width()),
        frame_keys::HEIGHT => css::px(payload.size.height()),
        frame_keys::BORDER_TOP_LEFT_RADIUS => css::px(tl),
        frame_keys::BORDER_TOP_RIGHT_RADIUS => css::px(tr),
        frame_keys::BORDER_BOTTOM_RIGHT_RADIUS => css::px(br),
        frame_keys::BORDER_BOTTOM_LEFT_RADIUS => css::px(bl),
        other => {
            return Err(unsupported(format!(
                "codec이 모르는 Frame registry key {other:?}"
            )))
        }
    })
}

/// Frame payload를 registry 이름·순서의 한 속성 한 줄로 씁니다.
pub(crate) fn frame(payload: &FramePayload) -> Result<YamlValue, TextSourceError> {
    let mut out = Vec::new();
    let border = BorderSides::from_payload(payload.border);
    for (name, which) in frame_lines() {
        let value = match which {
            FrameLine::Key(key) => frame_key_value(payload, key)?,
            FrameLine::Border(index) => border.format(BorderLonghand::ALL[index]),
        };
        out.push((scalar(name), line(value)));
    }
    Ok(YamlValue::Mapping(out))
}

/// 테두리 longhand 한 줄을 읽습니다. 값은 registry 테두리 표의 parser·formatter(`css::border`)를 쓰고, 정규 텍스트와
/// 같아야 합니다(결정론적 bytes).
fn parse_border_line(
    sides: &mut BorderSides,
    longhand: BorderLonghand,
    text: &str,
) -> Result<(), TextSourceError> {
    sides.set(longhand, text).map_err(|error| {
        if error.code.starts_with("unsupported") {
            unsupported(format!("{longhand}: {error}"))
        } else {
            invalid(format!("{longhand}: {error}"))
        }
    })?;
    let expected = sides.format(longhand);
    if expected != text {
        return Err(invalid(format!(
            "{longhand}: 정규 표기 {expected}이어야 합니다: {text}"
        )));
    }
    Ok(())
}

/// [`frame`]을 읽습니다.
pub(crate) fn parse_frame(value: &YamlValue) -> Result<FramePayload, TextSourceError> {
    let lines = frame_lines();
    let names: Vec<&str> = lines.iter().map(|(name, _)| *name).collect();
    known_fields(value, &names)?;
    let mut payload = FramePayload::default();
    let mut width = 0.0;
    let mut height = 0.0;
    let mut radius = [0.0_f32; 4];
    let mut border = BorderSides::INITIAL;
    for (name, which) in &lines {
        let required = matches!(
            which,
            FrameLine::Key(frame_keys::WIDTH) | FrameLine::Key(frame_keys::HEIGHT)
        );
        if !required && value.get(name).is_none() {
            continue;
        }
        let text = get(value, name)?;
        match which {
            FrameLine::Border(index) => {
                parse_border_line(&mut border, BorderLonghand::ALL[*index], text)?
            }
            FrameLine::Key(key) => match *key {
                frame_keys::TRANSFORM => payload.transform = css::parse_matrix(text)?,
                frame_keys::CLIPPING => payload.clipping = parse_clipping(text)?,
                frame_keys::BACKGROUND => payload.background = parse_background(text)?,
                frame_keys::LAYOUT_MODE => payload.layout_mode = css::parse_optional_id(text)?,
                frame_keys::WIDTH => width = css::parse_px(text)?,
                frame_keys::HEIGHT => height = css::parse_px(text)?,
                frame_keys::BORDER_TOP_LEFT_RADIUS => radius[0] = css::parse_px(text)?,
                frame_keys::BORDER_TOP_RIGHT_RADIUS => radius[1] = css::parse_px(text)?,
                frame_keys::BORDER_BOTTOM_RIGHT_RADIUS => radius[2] = css::parse_px(text)?,
                frame_keys::BORDER_BOTTOM_LEFT_RADIUS => radius[3] = css::parse_px(text)?,
                other => {
                    return Err(unsupported(format!(
                        "codec이 모르는 Frame registry key {other:?}"
                    )))
                }
            },
        }
    }
    payload.size = Size2::new(width, height).map_err(invalid)?;
    payload.corner_radius =
        CornerRadius::new(radius[0], radius[1], radius[2], radius[3]).map_err(invalid)?;
    // 묶음 값으로 표현할 수 없는 조합(변마다 다른 값 등)은 거부합니다(Frame payload schema 2 전).
    payload.border = border.to_payload().map_err(unsupported)?;
    Ok(payload)
}

// --- Shape ---

fn cap_name(cap: LineCap) -> &'static str {
    match cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

fn parse_cap(text: &str) -> Result<LineCap, TextSourceError> {
    match text {
        "butt" => Ok(LineCap::Butt),
        "round" => Ok(LineCap::Round),
        "square" => Ok(LineCap::Square),
        other => Err(unsupported(format!("stroke-linecap {other}"))),
    }
}

/// registry 키 뒤에 오는 codec 고유 키(경계 원점). registry에 원점 key가 없습니다.
const ORIGIN_KEYS: [&str; 2] = ["origin-x", "origin-y"];

fn shape(geometry: &ShapeGeometry) -> Result<YamlValue, TextSourceError> {
    let kind = geometry.kind();
    let mut out = Vec::new();
    for (name, key) in shape_lines(kind) {
        let value = match (geometry, key) {
            (ShapeGeometry::Rectangle(r), shape_keys::WIDTH) => css::px(r.size.width()),
            (ShapeGeometry::Rectangle(r), shape_keys::HEIGHT) => css::px(r.size.height()),
            (ShapeGeometry::Rectangle(r), shape_keys::BORDER_TOP_LEFT_RADIUS) => {
                css::px(r.corner_radius.components().0)
            }
            (ShapeGeometry::Rectangle(r), shape_keys::BORDER_TOP_RIGHT_RADIUS) => {
                css::px(r.corner_radius.components().1)
            }
            (ShapeGeometry::Rectangle(r), shape_keys::BORDER_BOTTOM_RIGHT_RADIUS) => {
                css::px(r.corner_radius.components().2)
            }
            (ShapeGeometry::Rectangle(r), shape_keys::BORDER_BOTTOM_LEFT_RADIUS) => {
                css::px(r.corner_radius.components().3)
            }
            (ShapeGeometry::Ellipse(e), shape_keys::WIDTH) => css::px(e.bounds.size().width()),
            (ShapeGeometry::Ellipse(e), shape_keys::HEIGHT) => css::px(e.bounds.size().height()),
            (ShapeGeometry::Line(l), shape_keys::X1) => css::px(l.start.x()),
            (ShapeGeometry::Line(l), shape_keys::Y1) => css::px(l.start.y()),
            (ShapeGeometry::Line(l), shape_keys::X2) => css::px(l.end.x()),
            (ShapeGeometry::Line(l), shape_keys::Y2) => css::px(l.end.y()),
            (ShapeGeometry::Line(l), shape_keys::STROKE_LINECAP) => cap_name(l.cap).into(),
            (ShapeGeometry::Polygon(p), shape_keys::WIDTH) => css::px(p.bounds().size().width()),
            (ShapeGeometry::Polygon(p), shape_keys::HEIGHT) => css::px(p.bounds().size().height()),
            (ShapeGeometry::Polygon(p), shape_keys::POLYGON_SIDES) => p.sides().to_string(),
            (ShapeGeometry::Polygon(p), shape_keys::POLYGON_ROTATION) => {
                css::deg(p.rotation_degrees())
            }
            (ShapeGeometry::Polygon(p), shape_keys::POLYGON_CORNER_RADIUS) => {
                css::px(p.corner_radius())
            }
            (_, other) => {
                return Err(unsupported(format!(
                    "codec이 모르는 {} registry key {other:?}",
                    kind.name()
                )))
            }
        };
        out.push((scalar(name), line(value)));
    }
    let origin = match geometry {
        ShapeGeometry::Ellipse(e) => Some(e.bounds.origin()),
        ShapeGeometry::Polygon(p) => Some(p.bounds().origin()),
        _ => None,
    };
    if let Some(origin) = origin {
        out.push((scalar(ORIGIN_KEYS[0]), line(css::px(origin.x()))));
        out.push((scalar(ORIGIN_KEYS[1]), line(css::px(origin.y()))));
    }
    Ok(YamlValue::Mapping(out))
}

fn parse_shape(value: &YamlValue, kind: ShapeKind) -> Result<ShapeGeometry, TextSourceError> {
    let lines = shape_lines(kind);
    let mut names: Vec<&str> = lines.iter().map(|(name, _)| *name).collect();
    let has_origin = matches!(kind, ShapeKind::Ellipse | ShapeKind::Polygon);
    if has_origin {
        names.extend(ORIGIN_KEYS);
    }
    fields(value, &names)?;
    let by_key = |key: PropertyKey| -> Result<&str, TextSourceError> {
        let (name, _) = lines
            .iter()
            .find(|(_, k)| *k == key)
            .ok_or_else(|| unsupported(format!("registry에 {key:?}가 없습니다")))?;
        get(value, name)
    };
    let px = |key| css::parse_px(by_key(key)?);
    let size = || Size2::new(px(shape_keys::WIDTH)?, px(shape_keys::HEIGHT)?).map_err(invalid);
    let bounds = || -> Result<Rect, TextSourceError> {
        let origin = Vec2::new(
            css::parse_px(get(value, ORIGIN_KEYS[0])?)?,
            css::parse_px(get(value, ORIGIN_KEYS[1])?)?,
        )
        .map_err(invalid)?;
        Ok(Rect::new(origin, size()?))
    };
    Ok(match kind {
        ShapeKind::Rectangle => ShapeGeometry::Rectangle(RectangleParams {
            size: size()?,
            corner_radius: CornerRadius::new(
                px(shape_keys::BORDER_TOP_LEFT_RADIUS)?,
                px(shape_keys::BORDER_TOP_RIGHT_RADIUS)?,
                px(shape_keys::BORDER_BOTTOM_RIGHT_RADIUS)?,
                px(shape_keys::BORDER_BOTTOM_LEFT_RADIUS)?,
            )
            .map_err(invalid)?,
        }),
        ShapeKind::Ellipse => {
            ShapeGeometry::Ellipse(EllipseParams::new(bounds()?).map_err(invalid)?)
        }
        ShapeKind::Line => ShapeGeometry::Line(LineParams {
            start: Vec2::new(px(shape_keys::X1)?, px(shape_keys::Y1)?).map_err(invalid)?,
            end: Vec2::new(px(shape_keys::X2)?, px(shape_keys::Y2)?).map_err(invalid)?,
            cap: parse_cap(by_key(shape_keys::STROKE_LINECAP)?)?,
        }),
        ShapeKind::Polygon => ShapeGeometry::Polygon(
            PolygonParams::new(
                bounds()?,
                css::parse_u32(by_key(shape_keys::POLYGON_SIDES)?)?,
                Angle::new(css::parse_deg(by_key(shape_keys::POLYGON_ROTATION)?)?)
                    .map_err(invalid)?,
                px(shape_keys::POLYGON_CORNER_RADIUS)?,
            )
            .map_err(invalid)?,
        ),
    })
}

// --- Path ---

fn path(value: &IdentifiedPath) -> YamlValue {
    let data = value.data();
    let commands = data
        .commands()
        .iter()
        .zip(value.ids())
        .map(|(command, ids)| {
            let point = ids.point.map(|p| css::id(p.0)).unwrap_or_default();
            line(match command {
                PathCommand::MoveTo(to) => {
                    format!("M {point} {} {}", css::number(to.x()), css::number(to.y()))
                }
                PathCommand::LineTo(to) => {
                    format!("L {point} {} {}", css::number(to.x()), css::number(to.y()))
                }
                PathCommand::CubicTo { c1, c2, to } => {
                    let (h1, h2) = ids.handles.expect("검증된 cubic handle id");
                    format!(
                        "C {} {} {point} {} {} {} {} {} {}",
                        css::id(h1.0),
                        css::id(h2.0),
                        css::number(c1.x()),
                        css::number(c1.y()),
                        css::number(c2.x()),
                        css::number(c2.y()),
                        css::number(to.x()),
                        css::number(to.y())
                    )
                }
                PathCommand::Close => "Z".into(),
            })
        })
        .collect();
    mapping([
        (
            "fill-rule",
            line(match data.winding() {
                WindingRule::NonZero => "nonzero",
                WindingRule::EvenOdd => "evenodd",
            }),
        ),
        ("commands", YamlValue::Sequence(commands)),
    ])
}

fn parse_path(value: &YamlValue) -> Result<IdentifiedPath, TextSourceError> {
    fields(value, &["fill-rule", "commands"])?;
    let winding = match get(value, "fill-rule")? {
        "nonzero" => WindingRule::NonZero,
        "evenodd" => WindingRule::EvenOdd,
        other => return Err(unsupported(format!("fill-rule {other}"))),
    };
    let mut commands = Vec::new();
    let mut ids = Vec::new();
    let items = field(value, "commands")?
        .as_sequence()
        .ok_or_else(|| invalid("commands는 sequence여야 합니다"))?;
    for item in items {
        let parts: Vec<&str> = text(item)?.split(' ').collect();
        let point = |at: usize| css::parse_id(parts[at]).map(PathPointId);
        let vec = |at: usize| -> Result<Vec2, TextSourceError> {
            Vec2::new(
                css::parse_number(parts[at])?,
                css::parse_number(parts[at + 1])?,
            )
            .map_err(invalid)
        };
        match parts.as_slice() {
            ["M", _, _, _] | ["L", _, _, _] => {
                let to = vec(2)?;
                commands.push(if parts[0] == "M" {
                    PathCommand::MoveTo(to)
                } else {
                    PathCommand::LineTo(to)
                });
                ids.push(CommandIds {
                    point: Some(point(1)?),
                    handles: None,
                });
            }
            ["C", _, _, _, _, _, _, _, _, _] => {
                commands.push(PathCommand::CubicTo {
                    c1: vec(4)?,
                    c2: vec(6)?,
                    to: vec(8)?,
                });
                ids.push(CommandIds {
                    point: Some(point(3)?),
                    handles: Some((
                        PathHandleId(css::parse_id(parts[1])?),
                        PathHandleId(css::parse_id(parts[2])?),
                    )),
                });
            }
            ["Z"] => {
                commands.push(PathCommand::Close);
                ids.push(CommandIds {
                    point: None,
                    handles: None,
                });
            }
            _ => {
                return Err(invalid(format!(
                    "path 명령 형식이 잘못되었습니다: {}",
                    text(item)?
                )))
            }
        }
    }
    IdentifiedPath::new(PathData::new(winding, commands).map_err(invalid)?, ids).map_err(invalid)
}

// --- Text ---

fn text_payload(payload: &TextPayload) -> Result<YamlValue, TextSourceError> {
    if payload
        .style_runs()
        .iter()
        .any(|run| run.range.start > run.range.end)
    {
        return Err(invalid("Text run의 start는 end 이하여야 합니다"));
    }
    Ok(mapping([
        ("content", line(payload.content())),
        (
            "paragraphs",
            YamlValue::Sequence(
                payload
                    .paragraphs()
                    .iter()
                    .map(|paragraph| line(paragraph.style.to_string()))
                    .collect(),
            ),
        ),
        (
            "style-runs",
            YamlValue::Sequence(
                payload
                    .style_runs()
                    .iter()
                    .map(|run| {
                        line(format!(
                            "{} {} {}",
                            run.range.start.0, run.range.end.0, run.style
                        ))
                    })
                    .collect(),
            ),
        ),
    ]))
}

fn parse_text(value: &YamlValue) -> Result<TextPayload, TextSourceError> {
    fields(value, &["content", "paragraphs", "style-runs"])?;
    let sequence = |key| {
        field(value, key)?
            .as_sequence()
            .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
    };
    let paragraphs = sequence("paragraphs")?
        .iter()
        .map(|item| {
            Ok(Paragraph {
                style: css::parse_u32(text(item)?)?,
            })
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let runs = sequence("style-runs")?
        .iter()
        .map(|item| {
            let parts: Vec<&str> = text(item)?.split(' ').collect();
            let [start, end, style] = parts.as_slice() else {
                return Err(invalid("style run은 start end style입니다"));
            };
            Ok(StyleRun {
                range: TextRange::new(css::parse_u32(start)?, css::parse_u32(end)?)
                    .ok_or_else(|| invalid("Text run의 start는 end 이하여야 합니다"))?,
                style: css::parse_u32(style)?,
            })
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    TextPayload::new(UserText::new(get(value, "content")?), paragraphs, runs).map_err(invalid)
}

// --- Image ---

fn fit_name(fit: FitMode) -> &'static str {
    match fit {
        FitMode::Fill => "fill",
        FitMode::Contain => "contain",
        FitMode::Cover => "cover",
        FitMode::None => "none",
    }
}

fn sampling_name(sampling: SamplingMode) -> &'static str {
    match sampling {
        SamplingMode::Linear => "auto",
        SamplingMode::Nearest => "pixelated",
    }
}

fn image(payload: &ImagePayload) -> Result<YamlValue, TextSourceError> {
    let mut out = Vec::new();
    for (name, key) in image_lines() {
        let value = match key {
            IMAGE_PROP_ASSET => css::id(payload.asset.id()),
            image_keys::WIDTH => css::px(payload.size.width()),
            image_keys::HEIGHT => css::px(payload.size.height()),
            image_keys::OBJECT_FIT => fit_name(payload.fit).into(),
            image_keys::IMAGE_RENDERING => sampling_name(payload.sampling).into(),
            image_keys::IMAGE_CROP => {
                let (x, y, w, h) = payload.crop.components();
                format!(
                    "{} {} {} {}",
                    css::number(x),
                    css::number(y),
                    css::number(w),
                    css::number(h)
                )
            }
            image_keys::IMAGE_COLOR_SPACE => match payload.color_space_override {
                None => "none",
                Some(ColorSpace::Srgb) => "srgb",
                Some(ColorSpace::DisplayP3) => "display-p3",
            }
            .into(),
            other => {
                return Err(unsupported(format!(
                    "codec이 모르는 Image registry key {other:?}"
                )))
            }
        };
        out.push((scalar(name), line(value)));
    }
    Ok(YamlValue::Mapping(out))
}

fn parse_image(value: &YamlValue) -> Result<ImagePayload, TextSourceError> {
    let lines = image_lines();
    let names: Vec<&str> = lines.iter().map(|(name, _)| *name).collect();
    fields(value, &names)?;
    let by_key = |key: PropertyKey| -> Result<&str, TextSourceError> {
        let (name, _) = lines
            .iter()
            .find(|(_, k)| *k == key)
            .ok_or_else(|| unsupported(format!("registry에 {key:?}가 없습니다")))?;
        get(value, name)
    };
    let size = Size2::new(
        css::parse_px(by_key(image_keys::WIDTH)?)?,
        css::parse_px(by_key(image_keys::HEIGHT)?)?,
    )
    .map_err(invalid)?;
    let mut payload = ImagePayload::new(
        AssetRef::new(css::parse_id(by_key(IMAGE_PROP_ASSET)?)?),
        size,
    );
    payload.fit = match by_key(image_keys::OBJECT_FIT)? {
        "fill" => FitMode::Fill,
        "contain" => FitMode::Contain,
        "cover" => FitMode::Cover,
        "none" => FitMode::None,
        other => return Err(unsupported(format!("object-fit {other}"))),
    };
    payload.sampling = match by_key(image_keys::IMAGE_RENDERING)? {
        "auto" => SamplingMode::Linear,
        "pixelated" => SamplingMode::Nearest,
        other => return Err(unsupported(format!("image-rendering {other}"))),
    };
    let crop: Vec<f32> = by_key(image_keys::IMAGE_CROP)?
        .split(' ')
        .map(css::parse_number)
        .collect::<Result<_, _>>()?;
    let [x, y, w, h] = crop.as_slice() else {
        return Err(invalid("crop은 x y width height입니다"));
    };
    payload.crop = CropRect::from_components(*x, *y, *w, *h).map_err(invalid)?;
    payload.color_space_override = match by_key(image_keys::IMAGE_COLOR_SPACE)? {
        "none" => None,
        "srgb" => Some(ColorSpace::Srgb),
        "display-p3" => Some(ColorSpace::DisplayP3),
        other => return Err(unsupported(format!("color-space {other}"))),
    };
    Ok(payload)
}

// --- ComponentInstance ---

/// TypedValue 리터럴(타입 이름, CSS 값). 표현하지 않는 타입은 거부합니다.
pub(crate) fn typed_value(value: &TypedValue) -> Result<(&'static str, String), TextSourceError> {
    Ok(match value {
        TypedValue::Boolean(v) => ("boolean", v.to_string()),
        TypedValue::Number(v) => ("number", css::number(v.get())),
        TypedValue::String(v) => ("string", v.clone()),
        TypedValue::Length(v) => ("length", css::px(v.get())),
        TypedValue::Percentage(v) => ("percentage", css::percent(v.get())),
        TypedValue::Angle(v) => ("angle", css::deg(v.get())),
        TypedValue::Duration(v) => ("duration", css::ms(v.get())),
        TypedValue::Color(v) => ("color", css::color(*v)),
        TypedValue::Enum(_) => return Err(unsupported("텍스트 리터럴로 쓰지 않는 값 타입: enum")),
        TypedValue::Gradient(_) => {
            return Err(unsupported("텍스트 리터럴로 쓰지 않는 값 타입: gradient"))
        }
        TypedValue::Shadow(_) => {
            return Err(unsupported("텍스트 리터럴로 쓰지 않는 값 타입: shadow"))
        }
        TypedValue::Typography(_) => {
            return Err(unsupported("텍스트 리터럴로 쓰지 않는 값 타입: typography"))
        }
        TypedValue::ResourceRef(_) => {
            return Err(unsupported(
                "텍스트 리터럴로 쓰지 않는 값 타입: resource-ref",
            ))
        }
    })
}

/// [`typed_value`]를 읽습니다.
pub(crate) fn parse_typed_value(kind: &str, value: &str) -> Result<TypedValue, TextSourceError> {
    let finite = |v: f32| FiniteF32::new(v).map_err(invalid);
    Ok(match kind {
        "boolean" => TypedValue::Boolean(css::parse_bool(value)?),
        "number" => TypedValue::Number(finite(css::parse_number(value)?)?),
        "string" => TypedValue::String(value.to_owned()),
        "length" => TypedValue::Length(Length::new(css::parse_px(value)?).map_err(invalid)?),
        "percentage" => {
            TypedValue::Percentage(Percentage::new(css::parse_percent(value)?).map_err(invalid)?)
        }
        "angle" => TypedValue::Angle(Angle::new(css::parse_deg(value)?).map_err(invalid)?),
        "duration" => TypedValue::Duration(Duration::new(css::parse_ms(value)?).map_err(invalid)?),
        "color" => TypedValue::Color(css::parse_color(value)?),
        other => return Err(unsupported(format!("값 타입 {other}"))),
    })
}

fn instance(payload: &ComponentInstancePayload) -> Result<YamlValue, TextSourceError> {
    let component = payload.component().id();
    if component == StableId::NIL {
        return Err(invalid("빈 component 참조"));
    }
    let properties = payload
        .property_overrides()
        .map(|(key, value)| {
            let (kind, value) = typed_value(value)?;
            Ok(mapping([
                ("path", line(css::id_list(key.path().iter().copied()))),
                ("property", line(css::id(key.property().stable_id()))),
                ("type", line(kind)),
                ("value", line(value)),
            ]))
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let slots = payload
        .slot_overrides()
        .map(|(slot, content)| {
            mapping([
                ("slot", line(css::id(slot.stable_id()))),
                (
                    "content",
                    line(css::id_list(content.content().iter().copied())),
                ),
            ])
        })
        .collect();
    Ok(mapping([
        ("component", line(css::id(component))),
        ("property-overrides", YamlValue::Sequence(properties)),
        ("slot-overrides", YamlValue::Sequence(slots)),
    ]))
}

fn parse_instance(value: &YamlValue) -> Result<ComponentInstancePayload, TextSourceError> {
    fields(
        value,
        &["component", "property-overrides", "slot-overrides"],
    )?;
    let component = css::parse_id(get(value, "component")?)?;
    if component == StableId::NIL {
        return Err(invalid("빈 component 참조"));
    }
    let mut payload = ComponentInstancePayload::new(ComponentRef::new(component));
    let sequence = |key| {
        field(value, key)?
            .as_sequence()
            .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
    };
    for item in sequence("property-overrides")? {
        fields(item, &["path", "property", "type", "value"])?;
        let property = ComponentPropertyId::new(css::parse_id(get(item, "property")?)?);
        let path = css::parse_id_list(get(item, "path")?)?;
        let key = if path.is_empty() {
            OverrideKey::direct(property)
        } else {
            OverrideKey::nested(path, property)
        };
        if payload.property_override(&key).is_some() {
            return Err(invalid("중복 property override"));
        }
        payload.set_property_override(
            key,
            parse_typed_value(get(item, "type")?, get(item, "value")?)?,
        );
    }
    for item in sequence("slot-overrides")? {
        fields(item, &["slot", "content"])?;
        let slot = ComponentSlotId::new(css::parse_id(get(item, "slot")?)?);
        if payload.slot_override(slot).is_some() {
            return Err(invalid("중복 slot override"));
        }
        let content =
            SlotOverride::new(css::parse_id_list(get(item, "content")?)?).map_err(invalid)?;
        payload.set_slot_override(slot, content);
    }
    Ok(payload)
}

// --- Modifier ---

fn modifier_value(entry: &ModifierEntry, key: PropertyKey) -> Result<String, TextSourceError> {
    use ModifierPayload as P;
    Ok(match (key, &entry.payload) {
        (modifier_keys::ENABLED, _) => entry.enabled.to_string(),
        (modifier_keys::ORDER, _) => css::hex(entry.order.as_bytes()),
        (modifier_keys::TRANSFORM_MATRIX, P::Transform(matrix)) => css::matrix(matrix),
        (modifier_keys::BOOLEAN_OP, P::Boolean { op, .. }) => match op {
            BooleanOp::Union => "union",
            BooleanOp::Subtract => "subtract",
            BooleanOp::Intersect => "intersect",
            BooleanOp::Exclude => "exclude",
        }
        .into(),
        (modifier_keys::BOOLEAN_OPERANDS, P::Boolean { operands, .. }) => {
            css::id_list(operands.iter().map(NodeRef::id))
        }
        (modifier_keys::STROKE_EXPAND_WIDTH, P::StrokeExpand { width }) => css::px(width.get()),
        (modifier_keys::CORNER_RADIUS, P::Corner { radius }) => css::px(radius.get()),
        (modifier_keys::MASK_SOURCE, P::Mask { source })
        | (modifier_keys::CLIP_SOURCE, P::Clip { source }) => css::id(source.id()),
        (modifier_keys::FILL_COLOR, P::Fill { color })
        | (modifier_keys::STROKE_COLOR, P::Stroke { color, .. })
        | (modifier_keys::SHADOW_COLOR, P::Shadow { color, .. }) => css::color(*color),
        (modifier_keys::STROKE_WIDTH, P::Stroke { width, .. }) => css::px(width.get()),
        (modifier_keys::SHADOW_OFFSET_X, P::Shadow { offset, .. }) => css::px(offset.x()),
        (modifier_keys::SHADOW_OFFSET_Y, P::Shadow { offset, .. }) => css::px(offset.y()),
        (modifier_keys::SHADOW_BLUR, P::Shadow { blur, .. }) => css::px(blur.get()),
        (modifier_keys::BLUR_RADIUS, P::Blur { radius }) => css::px(radius.get()),
        (modifier_keys::BLEND_MODE, P::Blend { mode }) => blend_name(*mode).into(),
        (modifier_keys::OPACITY_VALUE, P::Opacity { value }) => css::number(value.get()),
        (other, _) => {
            return Err(unsupported(format!(
                "codec이 모르는 modifier registry key {other:?}"
            )))
        }
    })
}

fn blend_name(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "normal",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
    }
}

const MODIFIER_KINDS: [ModifierKind; 12] = [
    ModifierKind::Transform,
    ModifierKind::Boolean,
    ModifierKind::StrokeExpand,
    ModifierKind::Corner,
    ModifierKind::Mask,
    ModifierKind::Clip,
    ModifierKind::Fill,
    ModifierKind::Stroke,
    ModifierKind::Shadow,
    ModifierKind::Blur,
    ModifierKind::Blend,
    ModifierKind::Opacity,
];

fn modifier(entry: &ModifierEntry) -> Result<YamlValue, TextSourceError> {
    let mut out = vec![
        (scalar("id"), line(css::id(entry.id.stable_id()))),
        (scalar("kind"), line(entry.kind.name())),
    ];
    for metadata in modifier_lines(entry.kind) {
        out.push((
            scalar(metadata.name),
            line(modifier_value(entry, metadata.key)?),
        ));
    }
    Ok(YamlValue::Mapping(out))
}

fn parse_modifier(value: &YamlValue) -> Result<ModifierEntry, TextSourceError> {
    let kind_text = get(value, "kind")?;
    let kind = MODIFIER_KINDS
        .into_iter()
        .find(|kind| kind.name() == kind_text)
        .ok_or_else(|| unsupported(format!("modifier 종류 {kind_text}")))?;
    let lines = modifier_lines(kind);
    let mut names = vec!["id", "kind"];
    names.extend(lines.iter().map(|m| m.name));
    fields(value, &names)?;
    let by_key = |key: PropertyKey| -> Result<&str, TextSourceError> {
        let metadata = lines
            .iter()
            .find(|m| m.key == key)
            .ok_or_else(|| unsupported(format!("modifier registry에 {key:?}가 없습니다")))?;
        get(value, metadata.name)
    };
    let px = |key| -> Result<FiniteF32, TextSourceError> {
        FiniteF32::new(css::parse_px(by_key(key)?)?).map_err(invalid)
    };
    let color = |key| css::parse_color(by_key(key)?);
    let reference = |key| -> Result<NodeRef, TextSourceError> {
        Ok(NodeRef::new(css::parse_id(by_key(key)?)?))
    };
    use ModifierPayload as P;
    let payload = match kind {
        ModifierKind::Transform => {
            P::Transform(css::parse_matrix(by_key(modifier_keys::TRANSFORM_MATRIX)?)?)
        }
        ModifierKind::Boolean => P::Boolean {
            op: match by_key(modifier_keys::BOOLEAN_OP)? {
                "union" => BooleanOp::Union,
                "subtract" => BooleanOp::Subtract,
                "intersect" => BooleanOp::Intersect,
                "exclude" => BooleanOp::Exclude,
                other => return Err(unsupported(format!("boolean 연산 {other}"))),
            },
            operands: css::parse_id_list(by_key(modifier_keys::BOOLEAN_OPERANDS)?)?
                .into_iter()
                .map(NodeRef::new)
                .collect(),
        },
        ModifierKind::StrokeExpand => P::StrokeExpand {
            width: px(modifier_keys::STROKE_EXPAND_WIDTH)?,
        },
        ModifierKind::Corner => P::Corner {
            radius: px(modifier_keys::CORNER_RADIUS)?,
        },
        ModifierKind::Mask => P::Mask {
            source: reference(modifier_keys::MASK_SOURCE)?,
        },
        ModifierKind::Clip => P::Clip {
            source: reference(modifier_keys::CLIP_SOURCE)?,
        },
        ModifierKind::Fill => P::Fill {
            color: color(modifier_keys::FILL_COLOR)?,
        },
        ModifierKind::Stroke => P::Stroke {
            color: color(modifier_keys::STROKE_COLOR)?,
            width: px(modifier_keys::STROKE_WIDTH)?,
        },
        ModifierKind::Shadow => P::Shadow {
            color: color(modifier_keys::SHADOW_COLOR)?,
            offset: Vec2::new(
                css::parse_px(by_key(modifier_keys::SHADOW_OFFSET_X)?)?,
                css::parse_px(by_key(modifier_keys::SHADOW_OFFSET_Y)?)?,
            )
            .map_err(invalid)?,
            blur: px(modifier_keys::SHADOW_BLUR)?,
        },
        ModifierKind::Blur => P::Blur {
            radius: px(modifier_keys::BLUR_RADIUS)?,
        },
        ModifierKind::Blend => P::Blend {
            mode: match by_key(modifier_keys::BLEND_MODE)? {
                "normal" => BlendMode::Normal,
                "multiply" => BlendMode::Multiply,
                "screen" => BlendMode::Screen,
                "overlay" => BlendMode::Overlay,
                "darken" => BlendMode::Darken,
                "lighten" => BlendMode::Lighten,
                other => return Err(unsupported(format!("blend mode {other}"))),
            },
        },
        ModifierKind::Opacity => P::Opacity {
            value: FiniteF32::new(css::parse_number(by_key(modifier_keys::OPACITY_VALUE)?)?)
                .map_err(invalid)?,
        },
    };
    Ok(ModifierEntry {
        id: ModifierId::new(css::parse_id(get(value, "id")?)?),
        order: parse_order_key(by_key(modifier_keys::ORDER)?)?,
        enabled: css::parse_bool(by_key(modifier_keys::ENABLED)?)?,
        kind,
        payload,
    })
}

// --- 공통 ---

/// OrderKey hex를 읽습니다.
pub(crate) fn parse_order_key(text: &str) -> Result<OrderKey, TextSourceError> {
    OrderKey::from_bytes(css::parse_hex(text)?).map_err(invalid)
}

/// node payload를 씁니다. Fragment는 `none`입니다.
pub(crate) fn payload(value: &NodePayload) -> Result<YamlValue, TextSourceError> {
    match value {
        NodePayload::Frame(frame_payload) => frame(frame_payload),
        NodePayload::Fragment(_) => Ok(line("none")),
        NodePayload::Shape(geometry) => shape(geometry),
        NodePayload::Path(value) => Ok(path(value)),
        NodePayload::Text(value) => text_payload(value),
        NodePayload::Image(value) => image(value),
        NodePayload::ComponentInstance(value) => instance(value),
    }
}

fn parse_payload(kind: &str, value: &YamlValue) -> Result<NodePayload, TextSourceError> {
    if let Some(shape_kind) = ShapeKind::ALL.into_iter().find(|s| s.name() == kind) {
        return Ok(NodePayload::Shape(parse_shape(value, shape_kind)?));
    }
    Ok(match kind {
        "frame" => NodePayload::Frame(parse_frame(value)?),
        "fragment" if value.as_scalar() == Some("none") => NodePayload::Fragment(FragmentPayload),
        "fragment" => return Err(invalid("Fragment payload는 none이어야 합니다")),
        "path" => NodePayload::Path(parse_path(value)?),
        "text" => NodePayload::Text(parse_text(value)?),
        "image" => NodePayload::Image(parse_image(value)?),
        "component-instance" => NodePayload::ComponentInstance(parse_instance(value)?),
        other => return Err(unsupported(format!("node 종류 {other}"))),
    })
}

/// 모르는 키는 거부하고 빠진 키는 허용합니다.
fn known_fields(value: &YamlValue, allowed: &[&str]) -> Result<(), TextSourceError> {
    let pairs = value
        .as_mapping()
        .ok_or_else(|| invalid("mapping이 필요합니다"))?;
    for (key, _) in pairs {
        let name = text(key)?;
        if !allowed.contains(&name) {
            return Err(unsupported(format!("알 수 없는 키: {name}")));
        }
    }
    Ok(())
}

/// 명시 layout longhand를 longhand 번호 순서의 한 줄씩으로 씁니다.
fn layout(record: &LayoutRecord) -> YamlValue {
    YamlValue::Mapping(
        record
            .iter()
            .map(|(longhand, value)| (scalar(longhand.name()), line(format_value(value))))
            .collect(),
    )
}

/// [`layout`]을 읽습니다. 줄 순서는 longhand 번호 순이어야 합니다(결정론적 bytes).
fn parse_layout(value: &YamlValue, node: StableId) -> Result<LayoutRecord, TextSourceError> {
    let pairs = value
        .as_mapping()
        .ok_or_else(|| invalid("layout은 mapping이어야 합니다"))?;
    let mut record = LayoutRecord::new(node);
    let mut last: Option<LayoutLonghand> = None;
    for (key, item) in pairs {
        let name = text(key)?;
        let longhand = LayoutLonghand::from_name(name)
            .ok_or_else(|| unsupported(format!("layout longhand가 아닙니다: {name}")))?;
        if last.is_some_and(|l| l >= longhand) {
            return Err(invalid("layout longhand는 번호 순서로 한 번씩 씁니다"));
        }
        last = Some(longhand);
        let parsed = longhand
            .parse(text(item)?)
            .map_err(|e| invalid(format!("{name}: {e}")))?;
        record.set(longhand, parsed).map_err(invalid)?;
    }
    if record.is_empty() {
        return Err(invalid("빈 layout은 쓰지 않습니다"));
    }
    Ok(record)
}

/// node 레코드 하나를 씁니다. `stack`이 `Some`이면 문서 node로 보고 `modifiers` 목록을 붙입니다. `layout`이
/// 있으면 `payload` 뒤에 명시 layout longhand를 씁니다.
pub(crate) fn node(
    record: &NodeRecord,
    stack: Option<Option<&ModifierStack>>,
    layout_record: Option<&LayoutRecord>,
) -> Result<YamlValue, TextSourceError> {
    let mut out = vec![
        (scalar("id"), line(css::id(record.id))),
        (scalar("parent"), line(css::optional_id(record.parent))),
        (
            scalar("order_key"),
            line(css::hex(record.order_key.as_bytes())),
        ),
        (scalar("visible"), line(record.visible.to_string())),
        (scalar("locked"), line(record.locked.to_string())),
        (scalar("display_name"), line(record.display_name.clone())),
        (scalar("kind"), line(kind_name(&record.payload))),
        (scalar("payload"), payload(&record.payload)?),
    ];
    if let Some(record) = layout_record.filter(|r| !r.is_empty()) {
        out.push((scalar("layout"), layout(record)));
    }
    if let Some(stack) = stack {
        out.push((
            scalar("modifiers"),
            YamlValue::Sequence(
                stack
                    .into_iter()
                    .flat_map(ModifierStack::entries)
                    .map(modifier)
                    .collect::<Result<_, _>>()?,
            ),
        ));
    }
    Ok(YamlValue::Mapping(out))
}

/// [`node`]를 읽은 결과: node 레코드, 비어 있지 않은 modifier stack, 명시 layout record.
pub(crate) type ParsedNode = (NodeRecord, Option<ModifierStack>, Option<LayoutRecord>);

/// [`node`]를 읽습니다. `with_modifiers`이면 문서 node로 보고 `modifiers` 목록을 요구하며 선택 `layout`을 읽습니다.
pub(crate) fn parse_node(
    value: &YamlValue,
    with_modifiers: bool,
) -> Result<ParsedNode, TextSourceError> {
    let mut keys: Vec<&str> = COMMON.to_vec();
    keys.push("payload");
    if with_modifiers {
        keys.push("modifiers");
        if value.get("layout").is_some() {
            keys.push("layout");
        }
    }
    fields(value, &keys)?;
    let record = NodeRecord {
        id: css::parse_id(get(value, "id")?)?,
        parent: css::parse_optional_id(get(value, "parent")?)?,
        order_key: parse_order_key(get(value, "order_key")?)?,
        visible: css::parse_bool(get(value, "visible")?)?,
        locked: css::parse_bool(get(value, "locked")?)?,
        display_name: get(value, "display_name")?.to_owned(),
        payload: parse_payload(get(value, "kind")?, field(value, "payload")?)?,
    };
    let mut stack = None;
    if with_modifiers {
        let entries = field(value, "modifiers")?
            .as_sequence()
            .ok_or_else(|| invalid("modifiers는 sequence여야 합니다"))?;
        if !entries.is_empty() {
            let mut modifiers = ModifierStack::new(record.id);
            for entry in entries {
                modifiers.insert(parse_modifier(entry)?).map_err(invalid)?;
            }
            // 파일 순서는 OrderKey 순서여야 합니다(결정론적 bytes).
            let ids: Vec<_> = modifiers.entries().iter().map(|e| e.id).collect();
            let written: Vec<_> = entries
                .iter()
                .map(|e| get(e, "id").and_then(css::parse_id).map(ModifierId::new))
                .collect::<Result<_, _>>()?;
            if ids != written {
                return Err(invalid("modifier는 order 순서로 써야 합니다"));
            }
            stack = Some(modifiers);
        }
    }
    let layout_record = match value.get("layout") {
        Some(item) if with_modifiers => Some(parse_layout(item, record.id)?),
        _ => None,
    };
    Ok((record, stack, layout_record))
}
