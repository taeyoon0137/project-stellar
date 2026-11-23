//! `SetProperty` command와 속성 값 (VII.2.4).
//!
//! 속성은 문자열이 아니라 numeric [`PropertyKey`]로 지정하며, **metadata registry**가 node
//! 종류별로 허용 key, 값 타입, 무효화 class를 정합니다:
//!
//! - 공통 node 필드(모든 종류): 표시 여부·잠금(`Bool`), 표시 이름(`Text`) — [`node_keys`].
//!   부모·순서는 [`MoveNode`](crate::node_ops::MoveNode)로만 바꿉니다.
//! - Frame payload: Frame registry(VI.3.5)의 7개 속성.
//! - Shape·Image payload: `payload_metadata` registry(UX.2.1)의 longhand. Shape key는 도형 종류마다 정해집니다
//!   (Rectangle만 `border-*-radius`, Line만 `x1`·`y1`·`x2`·`y2`). Text의 style longhand는 IX.11.11 뒤에 더합니다.
//!
//! 등록되지 않은 key는 [`CommandError::UnknownProperty`], 값 타입이 다르면
//! [`CommandError::PropertyTypeMismatch`]로 거부합니다. 결과 change set의 무효화 class도 같은
//! registry에서 가져옵니다.
//!
//! **specialization (VII.2.5)**: 표시 여부·잠금·이름 변경은 별도 command 종류가 아니라
//! [`SetProperty::visibility`]·[`SetProperty::locked`]·[`SetProperty::rename`](그리고 같은 이름의
//! [`Command`](crate::command::Command) 생성자)로 만든 `SetProperty`입니다. 따라서 직렬화·검증·
//! 적용·undo가 모두 같은 core patch 경로를 지납니다.
//!
//! 값 직렬화: type tag `u8` + 값(f32는 LE bit 그대로, 색은 [`Color::to_bytes`], id는 16바이트).
//! tag는 저장·journal 계약입니다.

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::Rect;
use stellar_core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
use stellar_document::frame::{keys as frame_keys, Border, Clipping, CornerRadius, FramePayload};
use stellar_document::frame_metadata::{frame_property, FramePropertyType};
use stellar_document::image::{FitMode, ImagePayload, SamplingMode};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{BlendMode, BooleanOp};
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_document::node_kind::NodeKind;
use stellar_document::payload_metadata::{
    image_property, shape_kind_keys, shape_property, PayloadPropertyMetadata, PayloadPropertyType,
    IMAGE_PROPERTIES,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{EllipseParams, LineCap, PolygonParams, ShapeGeometry, ShapeKind};

use crate::change::{node_field_invalidation, node_keys};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::{id_use, IdUse};

/// 속성 값 타입(registry가 key마다 하나를 정합니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyValueType {
    /// bool.
    Bool,
    /// UTF-8 문자열.
    Text,
    /// 비음수 크기.
    Size,
    /// 2D 아핀 변환.
    Transform,
    /// Frame clipping.
    Clipping,
    /// 선택적 색.
    OptionalColor,
    /// 선택적 테두리.
    OptionalBorder,
    /// 4귀퉁이 radius.
    CornerRadius,
    /// 선택적 layout 참조.
    OptionalLayoutRef,
    /// boolean 연산 종류(modifier, VII.2.6).
    BooleanOp,
    /// node 참조 목록(modifier).
    NodeRefList,
    /// node 참조 하나(modifier, NIL = 미지정).
    NodeRef,
    /// 비음수 finite 수.
    NonNegative,
    /// 색(modifier).
    Color,
    /// 2D 오프셋(modifier).
    Offset,
    /// 0..=1 값(modifier).
    UnitInterval,
    /// blend mode(modifier).
    BlendMode,
    /// 부호 있는 finite 수(좌표·각도 deg·개수, UX.2.1).
    Number,
    /// keyword의 저장 tag(UX.2.1). 허용 tag는 key가 정합니다.
    Keyword,
}

impl PropertyValueType {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            PropertyValueType::Bool => "bool",
            PropertyValueType::Text => "text",
            PropertyValueType::Size => "size",
            PropertyValueType::Transform => "transform",
            PropertyValueType::Clipping => "clipping",
            PropertyValueType::OptionalColor => "optional-color",
            PropertyValueType::OptionalBorder => "optional-border",
            PropertyValueType::CornerRadius => "corner-radius",
            PropertyValueType::OptionalLayoutRef => "optional-layout-ref",
            PropertyValueType::BooleanOp => "boolean-op",
            PropertyValueType::NodeRefList => "node-ref-list",
            PropertyValueType::NodeRef => "node-ref",
            PropertyValueType::NonNegative => "non-negative",
            PropertyValueType::Color => "color",
            PropertyValueType::Offset => "offset",
            PropertyValueType::UnitInterval => "unit-interval",
            PropertyValueType::BlendMode => "blend-mode",
            PropertyValueType::Number => "number",
            PropertyValueType::Keyword => "keyword",
        }
    }
}

impl fmt::Display for PropertyValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 속성 값.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// bool.
    Bool(bool),
    /// 문자열.
    Text(String),
    /// 크기.
    Size(Size2),
    /// 변환.
    Transform(Transform2D),
    /// clipping.
    Clipping(Clipping),
    /// 선택적 색.
    OptionalColor(Option<Color>),
    /// 선택적 테두리.
    OptionalBorder(Option<Border>),
    /// corner radius.
    CornerRadius(CornerRadius),
    /// 선택적 layout 참조.
    OptionalLayoutRef(Option<StableId>),
    /// boolean 연산.
    BooleanOp(BooleanOp),
    /// node 참조 목록.
    NodeRefList(Vec<NodeRef>),
    /// node 참조.
    NodeRef(NodeRef),
    /// 비음수 수(범위는 payload 검증이 확인).
    NonNegative(FiniteF32),
    /// 색.
    Color(Color),
    /// 오프셋.
    Offset(Vec2),
    /// 0..=1 값(범위는 payload 검증이 확인).
    UnitInterval(FiniteF32),
    /// blend mode.
    BlendMode(BlendMode),
    /// 부호 있는 수.
    Number(FiniteF32),
    /// keyword 저장 tag.
    Keyword(u8),
}

const TAG_BOOL: u8 = 1;
const TAG_TEXT: u8 = 2;
const TAG_SIZE: u8 = 3;
const TAG_TRANSFORM: u8 = 4;
const TAG_CLIPPING: u8 = 5;
const TAG_OPTIONAL_COLOR: u8 = 6;
const TAG_OPTIONAL_BORDER: u8 = 7;
const TAG_CORNER_RADIUS: u8 = 8;
const TAG_OPTIONAL_LAYOUT_REF: u8 = 9;
const TAG_BOOLEAN_OP: u8 = 10;
const TAG_NODE_REF_LIST: u8 = 11;
const TAG_NODE_REF: u8 = 12;
const TAG_NON_NEGATIVE: u8 = 13;
const TAG_COLOR: u8 = 14;
const TAG_OFFSET: u8 = 15;
const TAG_UNIT_INTERVAL: u8 = 16;
const TAG_BLEND_MODE: u8 = 17;
const TAG_NUMBER: u8 = 18;
const TAG_KEYWORD: u8 = 19;

/// boolean 연산 저장 tag(document `BooleanOp` repr과 같은 값).
fn boolean_op_from_tag(tag: u8) -> Option<BooleanOp> {
    [
        BooleanOp::Union,
        BooleanOp::Subtract,
        BooleanOp::Intersect,
        BooleanOp::Exclude,
    ]
    .into_iter()
    .find(|op| *op as u8 == tag)
}

/// blend mode 저장 tag(document `BlendMode` repr과 같은 값).
pub(crate) fn blend_mode_from_tag(tag: u8) -> Option<BlendMode> {
    [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
    ]
    .into_iter()
    .find(|mode| *mode as u8 == tag)
}

impl PropertyValue {
    /// 값의 타입.
    pub fn value_type(&self) -> PropertyValueType {
        match self {
            PropertyValue::Bool(_) => PropertyValueType::Bool,
            PropertyValue::Text(_) => PropertyValueType::Text,
            PropertyValue::Size(_) => PropertyValueType::Size,
            PropertyValue::Transform(_) => PropertyValueType::Transform,
            PropertyValue::Clipping(_) => PropertyValueType::Clipping,
            PropertyValue::OptionalColor(_) => PropertyValueType::OptionalColor,
            PropertyValue::OptionalBorder(_) => PropertyValueType::OptionalBorder,
            PropertyValue::CornerRadius(_) => PropertyValueType::CornerRadius,
            PropertyValue::OptionalLayoutRef(_) => PropertyValueType::OptionalLayoutRef,
            PropertyValue::BooleanOp(_) => PropertyValueType::BooleanOp,
            PropertyValue::NodeRefList(_) => PropertyValueType::NodeRefList,
            PropertyValue::NodeRef(_) => PropertyValueType::NodeRef,
            PropertyValue::NonNegative(_) => PropertyValueType::NonNegative,
            PropertyValue::Color(_) => PropertyValueType::Color,
            PropertyValue::Offset(_) => PropertyValueType::Offset,
            PropertyValue::UnitInterval(_) => PropertyValueType::UnitInterval,
            PropertyValue::BlendMode(_) => PropertyValueType::BlendMode,
            PropertyValue::Number(_) => PropertyValueType::Number,
            PropertyValue::Keyword(_) => PropertyValueType::Keyword,
        }
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        match self {
            PropertyValue::Bool(value) => {
                out.u8(TAG_BOOL);
                out.bool(*value);
            }
            PropertyValue::Text(text) => {
                out.u8(TAG_TEXT);
                out.str(text);
            }
            PropertyValue::Size(size) => {
                out.u8(TAG_SIZE);
                out.f32(size.width());
                out.f32(size.height());
            }
            PropertyValue::Transform(transform) => {
                out.u8(TAG_TRANSFORM);
                let (a, b, c, d, tx, ty) = transform.components();
                for component in [a, b, c, d, tx, ty] {
                    out.f32(component);
                }
            }
            PropertyValue::Clipping(clipping) => {
                out.u8(TAG_CLIPPING);
                match clipping {
                    Clipping::Disabled => out.u8(0),
                    Clipping::FrameBounds => out.u8(1),
                    Clipping::Path(target) => {
                        out.u8(2);
                        out.id(target.id());
                    }
                }
            }
            PropertyValue::OptionalColor(color) => {
                out.u8(TAG_OPTIONAL_COLOR);
                write_optional_color(out, color.as_ref());
            }
            PropertyValue::OptionalBorder(border) => {
                out.u8(TAG_OPTIONAL_BORDER);
                match border {
                    None => out.u8(0),
                    Some(border) => {
                        out.u8(1);
                        out.raw(&border.color.to_bytes());
                        out.f32(border.thickness());
                    }
                }
            }
            PropertyValue::CornerRadius(radius) => {
                out.u8(TAG_CORNER_RADIUS);
                let (tl, tr, br, bl) = radius.components();
                for value in [tl, tr, br, bl] {
                    out.f32(value);
                }
            }
            PropertyValue::OptionalLayoutRef(layout) => {
                out.u8(TAG_OPTIONAL_LAYOUT_REF);
                out.opt_id(*layout);
            }
            PropertyValue::BooleanOp(op) => {
                out.u8(TAG_BOOLEAN_OP);
                out.u8(*op as u8);
            }
            PropertyValue::NodeRefList(refs) => {
                out.u8(TAG_NODE_REF_LIST);
                out.len(refs.len());
                for target in refs {
                    out.id(target.id());
                }
            }
            PropertyValue::NodeRef(target) => {
                out.u8(TAG_NODE_REF);
                out.id(target.id());
            }
            PropertyValue::NonNegative(value) => {
                out.u8(TAG_NON_NEGATIVE);
                out.f32(value.get());
            }
            PropertyValue::Color(color) => {
                out.u8(TAG_COLOR);
                out.raw(&color.to_bytes());
            }
            PropertyValue::Offset(offset) => {
                out.u8(TAG_OFFSET);
                out.f32(offset.x());
                out.f32(offset.y());
            }
            PropertyValue::UnitInterval(value) => {
                out.u8(TAG_UNIT_INTERVAL);
                out.f32(value.get());
            }
            PropertyValue::BlendMode(mode) => {
                out.u8(TAG_BLEND_MODE);
                out.u8(*mode as u8);
            }
            PropertyValue::Number(value) => {
                out.u8(TAG_NUMBER);
                out.f32(value.get());
            }
            PropertyValue::Keyword(tag) => {
                out.u8(TAG_KEYWORD);
                out.u8(*tag);
            }
        }
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<PropertyValue, CommandCodecError> {
        let malformed = |reason| CommandCodecError::Malformed { reason };
        match input.u8()? {
            TAG_BOOL => Ok(PropertyValue::Bool(input.bool()?)),
            TAG_TEXT => Ok(PropertyValue::Text(input.string()?)),
            TAG_SIZE => {
                let (w, h) = (input.f32()?, input.f32()?);
                Size2::new(w, h)
                    .map(PropertyValue::Size)
                    .map_err(|_| malformed("잘못된 size"))
            }
            TAG_TRANSFORM => {
                let mut c = [0f32; 6];
                for slot in &mut c {
                    *slot = input.f32()?;
                }
                Transform2D::new(c[0], c[1], c[2], c[3], c[4], c[5])
                    .map(PropertyValue::Transform)
                    .map_err(|_| malformed("잘못된 transform"))
            }
            TAG_CLIPPING => Ok(PropertyValue::Clipping(match input.u8()? {
                0 => Clipping::Disabled,
                1 => Clipping::FrameBounds,
                2 => Clipping::Path(NodeRef::new(input.id()?)),
                _ => return Err(malformed("알 수 없는 clipping tag")),
            })),
            TAG_OPTIONAL_COLOR => Ok(PropertyValue::OptionalColor(read_optional_color(input)?)),
            TAG_OPTIONAL_BORDER => Ok(PropertyValue::OptionalBorder(match input.u8()? {
                0 => None,
                1 => {
                    let color = read_color(input)?;
                    let thickness = input.f32()?;
                    Some(Border::new(color, thickness).map_err(|_| malformed("잘못된 border"))?)
                }
                _ => return Err(malformed("border flag")),
            })),
            TAG_CORNER_RADIUS => {
                let mut r = [0f32; 4];
                for slot in &mut r {
                    *slot = input.f32()?;
                }
                CornerRadius::new(r[0], r[1], r[2], r[3])
                    .map(PropertyValue::CornerRadius)
                    .map_err(|_| malformed("잘못된 corner radius"))
            }
            TAG_OPTIONAL_LAYOUT_REF => Ok(PropertyValue::OptionalLayoutRef(input.opt_id()?)),
            TAG_BOOLEAN_OP => boolean_op_from_tag(input.u8()?)
                .map(PropertyValue::BooleanOp)
                .ok_or(malformed("알 수 없는 boolean 연산")),
            TAG_NODE_REF_LIST => {
                let count = input.len()?;
                let mut refs = Vec::with_capacity(count);
                for _ in 0..count {
                    refs.push(NodeRef::new(input.id()?));
                }
                Ok(PropertyValue::NodeRefList(refs))
            }
            TAG_NODE_REF => Ok(PropertyValue::NodeRef(NodeRef::new(input.id()?))),
            TAG_NON_NEGATIVE => finite(input.f32()?).map(PropertyValue::NonNegative),
            TAG_COLOR => Ok(PropertyValue::Color(read_color(input)?)),
            TAG_OFFSET => {
                let (x, y) = (input.f32()?, input.f32()?);
                Vec2::new(x, y)
                    .map(PropertyValue::Offset)
                    .map_err(|_| malformed("잘못된 offset"))
            }
            TAG_UNIT_INTERVAL => finite(input.f32()?).map(PropertyValue::UnitInterval),
            TAG_BLEND_MODE => blend_mode_from_tag(input.u8()?)
                .map(PropertyValue::BlendMode)
                .ok_or(malformed("알 수 없는 blend mode")),
            TAG_NUMBER => finite(input.f32()?).map(PropertyValue::Number),
            TAG_KEYWORD => Ok(PropertyValue::Keyword(input.u8()?)),
            _ => Err(malformed("알 수 없는 속성 값 tag")),
        }
    }
}

fn finite(value: f32) -> Result<FiniteF32, CommandCodecError> {
    FiniteF32::new(value).map_err(|_| CommandCodecError::Malformed {
        reason: "유한하지 않은 f32",
    })
}

pub(crate) fn read_color(input: &mut Reader<'_>) -> Result<Color, CommandCodecError> {
    let mut raw = [0u8; Color::BYTE_LEN];
    raw.copy_from_slice(input.take(Color::BYTE_LEN)?);
    Color::from_bytes(raw).map_err(|_| CommandCodecError::Malformed {
        reason: "잘못된 색",
    })
}

fn write_optional_color(out: &mut Writer, color: Option<&Color>) {
    match color {
        None => out.u8(0),
        Some(color) => {
            out.u8(1);
            out.raw(&color.to_bytes());
        }
    }
}

fn read_optional_color(input: &mut Reader<'_>) -> Result<Option<Color>, CommandCodecError> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(read_color(input)?)),
        _ => Err(CommandCodecError::Malformed {
            reason: "color flag",
        }),
    }
}

/// registry가 정한 한 속성의 metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyInfo {
    /// 값 타입.
    pub value_type: PropertyValueType,
    /// 변경 시 무효화 class.
    pub invalidation: InvalidationSet,
}

fn frame_value_type(value_type: FramePropertyType) -> PropertyValueType {
    match value_type {
        FramePropertyType::Size => PropertyValueType::Size,
        FramePropertyType::Dimension => PropertyValueType::NonNegative,
        FramePropertyType::Transform => PropertyValueType::Transform,
        FramePropertyType::Clipping => PropertyValueType::Clipping,
        FramePropertyType::OptionalColor => PropertyValueType::OptionalColor,
        FramePropertyType::OptionalBorder => PropertyValueType::OptionalBorder,
        FramePropertyType::CornerRadius => PropertyValueType::CornerRadius,
        FramePropertyType::OptionalLayoutRef => PropertyValueType::OptionalLayoutRef,
    }
}

/// `kind` node에서 `key`가 SetProperty로 설정 가능한 속성인지와 그 metadata.
///
/// Shape key는 도형 종류마다 다르므로 Shape에는 어느 한 도형 종류라도 가진 key를 돌려줍니다. 실제 편집
/// 가능 여부는 payload를 받는 [`payload_property_info`]가 정합니다.
pub fn node_property_info(kind: NodeKind, key: PropertyKey) -> Option<PropertyInfo> {
    match key {
        node_keys::VISIBLE | node_keys::LOCKED => Some(PropertyInfo {
            value_type: PropertyValueType::Bool,
            invalidation: node_field_invalidation(key)?,
        }),
        node_keys::DISPLAY_NAME => Some(PropertyInfo {
            value_type: PropertyValueType::Text,
            invalidation: node_field_invalidation(key)?,
        }),
        _ if kind == NodeKind::Frame => frame_property(key).map(|metadata| PropertyInfo {
            value_type: frame_value_type(metadata.value_type),
            invalidation: InvalidationSet::from(metadata.invalidation),
        }),
        _ if kind == NodeKind::Shape => ShapeKind::ALL
            .into_iter()
            .find_map(|shape| shape_property(shape, key))
            .and_then(payload_info),
        _ if kind == NodeKind::Image => image_property(key).and_then(payload_info),
        _ => None,
    }
}

/// payload registry 항목의 command 값 타입. 구조 값(asset·crop·색공간·path·text 단위)은 `SetProperty`
/// 대상이 아니라 `None`입니다.
fn payload_info(metadata: &PayloadPropertyMetadata) -> Option<PropertyInfo> {
    Some(PropertyInfo {
        value_type: match metadata.value_type {
            PayloadPropertyType::Dimension => PropertyValueType::NonNegative,
            PayloadPropertyType::Coordinate
            | PayloadPropertyType::Angle
            | PayloadPropertyType::Count => PropertyValueType::Number,
            PayloadPropertyType::Keyword => PropertyValueType::Keyword,
            PayloadPropertyType::Bool => PropertyValueType::Bool,
            PayloadPropertyType::Text => PropertyValueType::Text,
            PayloadPropertyType::Structured => return None,
        },
        invalidation: InvalidationSet::from(metadata.invalidation),
    })
}

/// `payload`를 가진 node에서 `key`의 metadata(도형 종류까지 확인).
pub fn payload_property_info(payload: &NodePayload, key: PropertyKey) -> Option<PropertyInfo> {
    match payload {
        NodePayload::Shape(shape) => match key {
            node_keys::VISIBLE | node_keys::LOCKED | node_keys::DISPLAY_NAME => {
                node_property_info(NodeKind::Shape, key)
            }
            _ => shape_property(shape.kind(), key).and_then(payload_info),
        },
        _ => node_property_info(payload.kind(), key),
    }
}

/// `payload` node가 SetProperty로 받는 key 목록(공통 필드 + payload longhand, registry 순서).
///
/// 묶음 key(Frame `size`·`corner-radius`)는 longhand가 따로 있어 목록에 넣지 않습니다.
pub fn node_property_keys(payload: &NodePayload) -> Vec<PropertyKey> {
    let mut keys = vec![
        node_keys::VISIBLE,
        node_keys::LOCKED,
        node_keys::DISPLAY_NAME,
    ];
    match payload {
        NodePayload::Frame(_) => keys.extend(
            stellar_document::frame_metadata::FRAME_PROPERTIES
                .iter()
                .filter(|m| !m.is_bundle())
                .map(|m| m.key),
        ),
        NodePayload::Shape(shape) => keys.extend_from_slice(shape_kind_keys(shape.kind())),
        NodePayload::Image(_) => keys.extend(
            IMAGE_PROPERTIES
                .iter()
                .filter(|m| m.is_settable())
                .map(|m| m.key),
        ),
        _ => {}
    }
    keys
}

fn number(value: f32) -> Option<PropertyValue> {
    FiniteF32::new(value).ok().map(PropertyValue::Number)
}

fn length(value: f32) -> Option<PropertyValue> {
    FiniteF32::new(value).ok().map(PropertyValue::NonNegative)
}

fn read_shape(shape: &ShapeGeometry, key: PropertyKey) -> Option<PropertyValue> {
    use stellar_document::payload_metadata::shape_keys as k;
    shape_property(shape.kind(), key)?;
    match shape {
        ShapeGeometry::Rectangle(rect) => match key {
            k::WIDTH => length(rect.size.width()),
            k::HEIGHT => length(rect.size.height()),
            _ => {
                let index = k::BORDER_RADIUS_LONGHANDS.iter().position(|c| *c == key)?;
                let (tl, tr, br, bl) = rect.corner_radius.components();
                length([tl, tr, br, bl][index])
            }
        },
        ShapeGeometry::Ellipse(ellipse) => match key {
            k::WIDTH => length(ellipse.bounds.size().width()),
            _ => length(ellipse.bounds.size().height()),
        },
        ShapeGeometry::Line(line) => match key {
            k::X1 => number(line.start.x()),
            k::Y1 => number(line.start.y()),
            k::X2 => number(line.end.x()),
            k::Y2 => number(line.end.y()),
            _ => Some(PropertyValue::Keyword(line.cap as u8)),
        },
        ShapeGeometry::Polygon(polygon) => match key {
            k::WIDTH => length(polygon.bounds().size().width()),
            k::HEIGHT => length(polygon.bounds().size().height()),
            k::POLYGON_SIDES => number(polygon.sides() as f32),
            k::POLYGON_ROTATION => number(polygon.rotation_degrees()),
            _ => length(polygon.corner_radius()),
        },
    }
}

fn read_image(image: &ImagePayload, key: PropertyKey) -> Option<PropertyValue> {
    use stellar_document::payload_metadata::image_keys as k;
    match key {
        k::WIDTH => length(image.size.width()),
        k::HEIGHT => length(image.size.height()),
        k::OBJECT_FIT => Some(PropertyValue::Keyword(image.fit as u8)),
        k::IMAGE_RENDERING => Some(PropertyValue::Keyword(image.sampling as u8)),
        _ => None,
    }
}

/// 현재 값을 읽습니다(registry에 있는 key만).
pub fn read_property(record: &NodeRecord, key: PropertyKey) -> Option<PropertyValue> {
    match key {
        node_keys::VISIBLE => return Some(PropertyValue::Bool(record.visible)),
        node_keys::LOCKED => return Some(PropertyValue::Bool(record.locked)),
        node_keys::DISPLAY_NAME => return Some(PropertyValue::Text(record.display_name.clone())),
        _ => {}
    }
    let frame = match &record.payload {
        NodePayload::Frame(frame) => frame,
        NodePayload::Shape(shape) => return read_shape(shape, key),
        NodePayload::Image(image) => return read_image(image, key),
        _ => return None,
    };
    Some(match key {
        frame_keys::SIZE => PropertyValue::Size(frame.size),
        frame_keys::WIDTH => PropertyValue::NonNegative(FiniteF32::new(frame.size.width()).ok()?),
        frame_keys::HEIGHT => PropertyValue::NonNegative(FiniteF32::new(frame.size.height()).ok()?),
        frame_keys::TRANSFORM => PropertyValue::Transform(frame.transform),
        frame_keys::CLIPPING => PropertyValue::Clipping(frame.clipping),
        frame_keys::BACKGROUND => PropertyValue::OptionalColor(frame.background),
        frame_keys::BORDER => PropertyValue::OptionalBorder(frame.border),
        frame_keys::CORNER_RADIUS => PropertyValue::CornerRadius(frame.corner_radius),
        frame_keys::LAYOUT_MODE => PropertyValue::OptionalLayoutRef(frame.layout_mode),
        _ => {
            let index = frame_keys::BORDER_RADIUS_LONGHANDS
                .iter()
                .position(|&corner| corner == key)?;
            let (tl, tr, br, bl) = frame.corner_radius.components();
            PropertyValue::NonNegative(FiniteF32::new([tl, tr, br, bl][index]).ok()?)
        }
    })
}

/// Shape payload에 값을 씁니다. 값이 도형 제약(변 수 3..=1000 등)을 어기면 이유를 돌려줍니다.
fn write_shape(
    shape: &mut ShapeGeometry,
    key: PropertyKey,
    value: &PropertyValue,
) -> Result<(), &'static str> {
    use stellar_document::payload_metadata::shape_keys as k;
    let invalid = "도형 값이 범위를 벗어났습니다";
    let scalar = match value {
        PropertyValue::NonNegative(v) | PropertyValue::Number(v) => v.get(),
        PropertyValue::Keyword(tag) => f32::from(*tag),
        _ => return Err(invalid),
    };
    let resized = |size: Size2| match key {
        k::WIDTH => Size2::new(scalar, size.height()).map_err(|_| invalid),
        _ => Size2::new(size.width(), scalar).map_err(|_| invalid),
    };
    match shape {
        ShapeGeometry::Rectangle(rect) => match key {
            k::WIDTH | k::HEIGHT => rect.size = resized(rect.size)?,
            _ => {
                let index = k::BORDER_RADIUS_LONGHANDS
                    .iter()
                    .position(|c| *c == key)
                    .ok_or(invalid)?;
                let (tl, tr, br, bl) = rect.corner_radius.components();
                let mut radii = [tl, tr, br, bl];
                radii[index] = scalar;
                let [tl, tr, br, bl] = radii;
                rect.corner_radius = CornerRadius::new(tl, tr, br, bl).map_err(|_| invalid)?;
            }
        },
        ShapeGeometry::Ellipse(ellipse) => {
            let bounds = Rect::new(ellipse.bounds.origin(), resized(ellipse.bounds.size())?);
            *ellipse = EllipseParams::new(bounds).map_err(|_| invalid)?;
        }
        ShapeGeometry::Line(line) => {
            let point = |p: Vec2, x: bool| {
                if x {
                    Vec2::new(scalar, p.y())
                } else {
                    Vec2::new(p.x(), scalar)
                }
                .map_err(|_| invalid)
            };
            match key {
                k::X1 => line.start = point(line.start, true)?,
                k::Y1 => line.start = point(line.start, false)?,
                k::X2 => line.end = point(line.end, true)?,
                k::Y2 => line.end = point(line.end, false)?,
                _ => {
                    line.cap = [LineCap::Butt, LineCap::Round, LineCap::Square]
                        .into_iter()
                        .find(|cap| f32::from(*cap as u8) == scalar)
                        .ok_or(invalid)?
                }
            }
        }
        ShapeGeometry::Polygon(polygon) => {
            let (mut bounds, mut sides, mut rotation, mut radius) = (
                polygon.bounds(),
                polygon.sides(),
                polygon.rotation_degrees(),
                polygon.corner_radius(),
            );
            match key {
                k::WIDTH | k::HEIGHT => {
                    bounds = Rect::new(bounds.origin(), resized(bounds.size())?)
                }
                k::POLYGON_SIDES => {
                    if scalar.fract() != 0.0 || scalar < 3.0 {
                        return Err("변 수는 3 이상의 정수입니다");
                    }
                    sides = scalar as u32;
                }
                k::POLYGON_ROTATION => rotation = scalar,
                _ => radius = scalar,
            }
            let angle = stellar_core_types::value::Angle::new(rotation).map_err(|_| invalid)?;
            *polygon = PolygonParams::new(bounds, sides, angle, radius).map_err(|_| invalid)?;
        }
    }
    Ok(())
}

/// Image payload에 값을 씁니다.
fn write_image(
    image: &mut ImagePayload,
    key: PropertyKey,
    value: &PropertyValue,
) -> Result<(), &'static str> {
    use stellar_document::payload_metadata::image_keys as k;
    let invalid = "이미지 값이 범위를 벗어났습니다";
    match (key, value) {
        (k::WIDTH, PropertyValue::NonNegative(v)) => {
            image.size = Size2::new(v.get(), image.size.height()).map_err(|_| invalid)?
        }
        (k::HEIGHT, PropertyValue::NonNegative(v)) => {
            image.size = Size2::new(image.size.width(), v.get()).map_err(|_| invalid)?
        }
        (k::OBJECT_FIT, PropertyValue::Keyword(tag)) => {
            image.fit = [
                FitMode::Fill,
                FitMode::Contain,
                FitMode::Cover,
                FitMode::None,
            ]
            .into_iter()
            .find(|fit| *fit as u8 == *tag)
            .ok_or(invalid)?
        }
        (k::IMAGE_RENDERING, PropertyValue::Keyword(tag)) => {
            image.sampling = [SamplingMode::Linear, SamplingMode::Nearest]
                .into_iter()
                .find(|mode| *mode as u8 == *tag)
                .ok_or(invalid)?
        }
        _ => return Err(invalid),
    }
    Ok(())
}

/// Frame payload에 값을 씁니다. 타입이 맞지 않으면 `false`.
fn write_frame(frame: &mut FramePayload, key: PropertyKey, value: &PropertyValue) -> bool {
    match (key, value) {
        (frame_keys::SIZE, PropertyValue::Size(v)) => frame.size = *v,
        (frame_keys::WIDTH, PropertyValue::NonNegative(v)) => {
            return frame.set_size(v.get(), frame.size.height()).is_ok();
        }
        (frame_keys::HEIGHT, PropertyValue::NonNegative(v)) => {
            return frame.set_size(frame.size.width(), v.get()).is_ok();
        }
        (frame_keys::TRANSFORM, PropertyValue::Transform(v)) => frame.transform = *v,
        (frame_keys::CLIPPING, PropertyValue::Clipping(v)) => frame.clipping = *v,
        (frame_keys::BACKGROUND, PropertyValue::OptionalColor(v)) => frame.background = *v,
        (frame_keys::BORDER, PropertyValue::OptionalBorder(v)) => frame.border = *v,
        (frame_keys::CORNER_RADIUS, PropertyValue::CornerRadius(v)) => frame.corner_radius = *v,
        (frame_keys::LAYOUT_MODE, PropertyValue::OptionalLayoutRef(v)) => frame.layout_mode = *v,
        (key, PropertyValue::NonNegative(v)) => {
            let Some(index) = frame_keys::BORDER_RADIUS_LONGHANDS
                .iter()
                .position(|&corner| corner == key)
            else {
                return false;
            };
            let (tl, tr, br, bl) = frame.corner_radius.components();
            let mut radii = [tl, tr, br, bl];
            radii[index] = v.get();
            let [tl, tr, br, bl] = radii;
            let Ok(radius) = CornerRadius::new(tl, tr, br, bl) else {
                return false;
            };
            frame.corner_radius = radius;
        }
        _ => return false,
    }
    true
}

/// node 속성 하나를 바꿉니다.
#[derive(Debug, Clone, PartialEq)]
pub struct SetProperty {
    /// 대상 node.
    pub node: StableId,
    /// 속성 key.
    pub key: PropertyKey,
    /// 새 값.
    pub value: PropertyValue,
}

#[cfg(test)]
thread_local! {
    /// 테스트 전용: core patch 경로(`SetProperty::apply`) 호출 수.
    static CORE_PATCH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 테스트 전용: 현재 thread의 core patch 호출 수.
#[cfg(test)]
pub(crate) fn core_patch_calls() -> usize {
    CORE_PATCH_CALLS.with(std::cell::Cell::get)
}

impl SetProperty {
    /// 고정 크기를 한 transaction에 적용할 width·height 명령으로 펼칩니다.
    pub fn frame_dimensions(node: StableId, size: Size2) -> [Self; 2] {
        [
            (frame_keys::WIDTH, size.width()),
            (frame_keys::HEIGHT, size.height()),
        ]
        .map(|(key, value)| Self {
            node,
            key,
            value: PropertyValue::NonNegative(FiniteF32::new(value).expect("Size2의 유한 좌표")),
        })
    }

    /// 표시 여부 변경(`SetVisibility` specialization, VII.2.5).
    pub fn visibility(node: StableId, visible: bool) -> SetProperty {
        SetProperty {
            node,
            key: node_keys::VISIBLE,
            value: PropertyValue::Bool(visible),
        }
    }

    /// 잠금 변경(`SetLocked` specialization, VII.2.5).
    pub fn locked(node: StableId, locked: bool) -> SetProperty {
        SetProperty {
            node,
            key: node_keys::LOCKED,
            value: PropertyValue::Bool(locked),
        }
    }

    /// 표시 이름 변경(`RenameNode` specialization, VII.2.5).
    pub fn rename(node: StableId, name: impl Into<String>) -> SetProperty {
        SetProperty {
            node,
            key: node_keys::DISPLAY_NAME,
            value: PropertyValue::Text(name.into()),
        }
    }

    fn info(&self, model: &DocumentModel) -> Result<PropertyInfo, CommandError> {
        let payload = model
            .payload(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        if self.key == node_keys::PARENT || self.key == node_keys::ORDER_KEY {
            return Err(CommandError::PropertyNotSettable { key: self.key });
        }
        payload_property_info(payload, self.key).ok_or(CommandError::UnknownProperty {
            node: self.node,
            key: self.key,
        })
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let info = self.info(model)?;
        let found = self.value.value_type();
        if found != info.value_type {
            return Err(CommandError::PropertyTypeMismatch {
                key: self.key,
                expected: info.value_type,
                found,
            });
        }
        if let PropertyValue::NonNegative(value) = self.value {
            if value.get() < 0.0 {
                return Err(CommandError::InvalidRequest {
                    reason: "Frame 길이는 음수일 수 없습니다",
                });
            }
        }
        // clip 대상이 문서에 있으면 Path여야 합니다(없으면 미해결 진단).
        if let PropertyValue::Clipping(Clipping::Path(target)) = &self.value {
            match id_use(model, target.id()) {
                None | Some(IdUse::Node(NodeKind::Path)) => {}
                Some(_) => {
                    return Err(CommandError::ReferenceTypeMismatch {
                        owner: self.node,
                        target: target.id(),
                    })
                }
            }
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<SetProperty, CommandError> {
        self.info(model)?;
        let record = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let value = read_property(&record, self.key).ok_or(CommandError::UnknownProperty {
            node: self.node,
            key: self.key,
        })?;
        Ok(SetProperty {
            node: self.node,
            key: self.key,
            value,
        })
    }

    /// 모든 속성 변경이 지나는 core patch 경로(VII.2.5 specialization 포함).
    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        #[cfg(test)]
        CORE_PATCH_CALLS.with(|calls| calls.set(calls.get() + 1));
        let info = self.info(ctx.model())?;
        let mut record = ctx
            .model()
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let mismatch = || CommandError::PropertyTypeMismatch {
            key: self.key,
            expected: info.value_type,
            found: self.value.value_type(),
        };
        match (self.key, &self.value) {
            (node_keys::VISIBLE, PropertyValue::Bool(v)) => record.visible = *v,
            (node_keys::LOCKED, PropertyValue::Bool(v)) => record.locked = *v,
            (node_keys::DISPLAY_NAME, PropertyValue::Text(v)) => record.display_name = v.clone(),
            (key, value) => {
                let invalid = |reason| CommandError::InvalidRequest { reason };
                let written = match &mut record.payload {
                    NodePayload::Frame(frame) => write_frame(frame, key, value),
                    NodePayload::Shape(shape) => {
                        write_shape(shape, key, value).map_err(invalid)?;
                        true
                    }
                    NodePayload::Image(image) => {
                        write_image(image, key, value).map_err(invalid)?;
                        true
                    }
                    _ => false,
                };
                if !written {
                    return Err(mismatch());
                }
            }
        }
        ctx.replace_record(record)?;
        ctx.changes()
            .node_property(self.node, self.key, info.invalidation);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.u32(self.key.0);
        self.value.encode_into(out);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<SetProperty, CommandCodecError> {
        Ok(SetProperty {
            node: input.id()?,
            key: PropertyKey(input.u32()?),
            value: PropertyValue::decode_from(input)?,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{sid, small_model};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::property::PropertyPath;
    use stellar_document::frame_metadata::FRAME_PROPERTIES;
    use stellar_document::modifier_metadata::InvalidationClass;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_document::text::TextPayload;

    fn set(node: u64, key: PropertyKey, value: PropertyValue) -> Command {
        Command::SetProperty(SetProperty {
            node: sid(node),
            key,
            value,
        })
    }

    #[test]
    fn legacy_size_bytes_apply_like_both_dimensions() {
        // SetProperty(0x0010), payload 길이 29, node, size key(0x0101), Size2 tag(3).
        let mut bytes = vec![0x10, 0, 29, 0, 0, 0];
        bytes.extend_from_slice(&sid(11).to_bytes());
        bytes.extend_from_slice(&[1, 1, 0, 0, 3]);
        bytes.extend_from_slice(&144.0f32.to_le_bytes());
        bytes.extend_from_slice(&96.0f32.to_le_bytes());
        let old = decode_command(&bytes).unwrap();
        assert_eq!(encode_command(&old), bytes);
        let mut legacy = DocumentState::new(small_model());
        legacy.execute(&CommandRequest::at(&legacy, old)).unwrap();
        let mut split = DocumentState::new(small_model());
        for field in SetProperty::frame_dimensions(sid(11), Size2::new(144., 96.).unwrap()) {
            split
                .execute(&CommandRequest::at(&split, Command::SetProperty(field)))
                .unwrap();
        }
        assert_eq!(
            legacy.model().payload(sid(11)),
            split.model().payload(sid(11))
        );
        let negative = set(
            11,
            frame_keys::HEIGHT,
            PropertyValue::NonNegative(FiniteF32::new(-1.).unwrap()),
        );
        assert!(split
            .execute(&CommandRequest::at(&split, negative))
            .is_err());
        assert_eq!(
            legacy.model().payload(sid(11)),
            split.model().payload(sid(11))
        );
    }

    #[test]
    fn grouped_radius_journal_bytes_match_four_independent_edits() {
        let mut bytes = vec![0x10, 0, 37, 0, 0, 0];
        bytes.extend_from_slice(&sid(11).to_bytes());
        bytes.extend_from_slice(&[6, 1, 0, 0, 8]); // 0x0106, CornerRadius tag.
        let values = [1.25_f32, 2.5, 3.75, 4.0];
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let old = decode_command(&bytes).unwrap();
        assert_eq!(encode_command(&old), bytes);
        let mut legacy = DocumentState::new(small_model());
        legacy.execute(&CommandRequest::at(&legacy, old)).unwrap();
        let mut split = DocumentState::new(small_model());
        for (key, value) in frame_keys::BORDER_RADIUS_LONGHANDS.into_iter().zip(values) {
            let command = set(
                11,
                key,
                PropertyValue::NonNegative(FiniteF32::new(value).unwrap()),
            );
            split.execute(&CommandRequest::at(&split, command)).unwrap();
        }
        assert_eq!(
            split.model().payload(sid(11)),
            legacy.model().payload(sid(11))
        );
    }

    fn sample_value(value_type: PropertyValueType) -> PropertyValue {
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
        match value_type {
            PropertyValueType::Bool => PropertyValue::Bool(false),
            PropertyValueType::Text => PropertyValue::Text("이름 🎨".into()),
            PropertyValueType::Size => PropertyValue::Size(Size2::new(120.0, 48.5).unwrap()),
            PropertyValueType::Transform => PropertyValue::Transform(
                Transform2D::new(0.5, 0.25, -0.25, 0.5, 10.0, -3.0).unwrap(),
            ),
            PropertyValueType::Clipping => PropertyValue::Clipping(Clipping::Disabled),
            PropertyValueType::OptionalColor => PropertyValue::OptionalColor(Some(red)),
            PropertyValueType::OptionalBorder => {
                PropertyValue::OptionalBorder(Some(Border::new(red, 2.0).unwrap()))
            }
            PropertyValueType::CornerRadius => {
                PropertyValue::CornerRadius(CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap())
            }
            PropertyValueType::OptionalLayoutRef => PropertyValue::OptionalLayoutRef(Some(sid(77))),
            // modifier 값 타입(VII.2.6).
            PropertyValueType::BooleanOp => PropertyValue::BooleanOp(BooleanOp::Intersect),
            PropertyValueType::NodeRefList => {
                PropertyValue::NodeRefList(vec![NodeRef::new(sid(5)), NodeRef::new(sid(6))])
            }
            PropertyValueType::NodeRef => PropertyValue::NodeRef(NodeRef::new(sid(5))),
            PropertyValueType::NonNegative => {
                PropertyValue::NonNegative(FiniteF32::new(2.0).unwrap())
            }
            PropertyValueType::Color => PropertyValue::Color(red),
            PropertyValueType::Offset => PropertyValue::Offset(Vec2::new(1.5, -2.0).unwrap()),
            PropertyValueType::UnitInterval => {
                PropertyValue::UnitInterval(FiniteF32::new(0.5).unwrap())
            }
            PropertyValueType::BlendMode => PropertyValue::BlendMode(BlendMode::Screen),
            PropertyValueType::Number => PropertyValue::Number(FiniteF32::new(-3.).unwrap()),
            PropertyValueType::Keyword => PropertyValue::Keyword(1),
        }
    }

    #[test]
    fn every_registered_frame_property_round_trips_with_registry_invalidation() {
        let mut state = DocumentState::new(small_model());
        let before = model_snapshot_json(state.model()).unwrap();
        let mut keys: Vec<PropertyKey> = FRAME_PROPERTIES.iter().map(|m| m.key).collect();
        keys.extend([
            node_keys::VISIBLE,
            node_keys::LOCKED,
            node_keys::DISPLAY_NAME,
        ]);
        for key in keys {
            let info = node_property_info(NodeKind::Frame, key).unwrap();
            let request = CommandRequest::at(&state, set(11, key, sample_value(info.value_type)));
            let outcome = state.execute(&request).unwrap();
            assert_eq!(
                outcome.changes.changed_properties(),
                vec![PropertyPath::with_keys(sid(11), vec![key])]
            );
            assert_eq!(outcome.changes.invalidation(), info.invalidation, "{key:?}");
            let record = state.model().record(sid(11)).unwrap();
            assert_eq!(
                read_property(&record, key),
                Some(sample_value(info.value_type))
            );
            // 자손은 유지됩니다(표시 여부 변경은 subtree 재삽입 경로).
            assert_eq!(state.model().children_in_order(sid(11)), vec![sid(13)]);
            let request = CommandRequest::at(&state, outcome.inverse);
            state.execute(&request).unwrap();
            assert_eq!(
                model_snapshot_json(state.model()).unwrap(),
                before,
                "{key:?}"
            );
        }
        let layout = node_property_info(NodeKind::Frame, frame_keys::SIZE).unwrap();
        assert!(layout.invalidation.contains(InvalidationClass::Layout));
        let paint = node_property_info(NodeKind::Frame, frame_keys::BACKGROUND).unwrap();
        assert_eq!(paint.invalidation.classes(), vec![InvalidationClass::Paint]);
    }

    #[test]
    fn wrong_property_key_or_type_is_rejected() {
        // 검증 기준(VII.2.4): 잘못된 PropertyKey/type 거부.
        let mut state = DocumentState::new(small_model());
        let mut text = crate::command::test_fixtures::frame(20, Some(10), 0x90);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        let request = CommandRequest::at(
            &state,
            Command::InsertNode(crate::node_ops::InsertNode::single(text)),
        );
        state.execute(&request).unwrap();
        let before = model_snapshot_json(state.model()).unwrap();
        let run = |state: &mut DocumentState, command| {
            let request = CommandRequest::at(state, command);
            state.execute(&request).map(|_| ())
        };
        // 등록되지 않은 key.
        assert_eq!(
            run(
                &mut state,
                set(11, PropertyKey(0x01FF), PropertyValue::Bool(true))
            ),
            Err(CommandError::UnknownProperty {
                node: sid(11),
                key: PropertyKey(0x01FF)
            })
        );
        // Frame key를 Text node에.
        assert_eq!(
            run(
                &mut state,
                set(20, frame_keys::SIZE, sample_value(PropertyValueType::Size))
            ),
            Err(CommandError::UnknownProperty {
                node: sid(20),
                key: frame_keys::SIZE
            })
        );
        // 타입 불일치.
        assert_eq!(
            run(
                &mut state,
                set(11, frame_keys::SIZE, PropertyValue::Bool(true))
            ),
            Err(CommandError::PropertyTypeMismatch {
                key: frame_keys::SIZE,
                expected: PropertyValueType::Size,
                found: PropertyValueType::Bool
            })
        );
        assert_eq!(
            run(
                &mut state,
                set(11, node_keys::VISIBLE, PropertyValue::Text("x".into()))
            ),
            Err(CommandError::PropertyTypeMismatch {
                key: node_keys::VISIBLE,
                expected: PropertyValueType::Bool,
                found: PropertyValueType::Text
            })
        );
        // 부모·순서는 MoveNode 전용.
        assert_eq!(
            run(
                &mut state,
                set(11, node_keys::PARENT, PropertyValue::Bool(true))
            ),
            Err(CommandError::PropertyNotSettable {
                key: node_keys::PARENT
            })
        );
        // clip 대상이 Path가 아닌 node.
        assert_eq!(
            run(
                &mut state,
                set(
                    11,
                    frame_keys::CLIPPING,
                    PropertyValue::Clipping(Clipping::Path(NodeRef::new(sid(12))))
                )
            ),
            Err(CommandError::ReferenceTypeMismatch {
                owner: sid(11),
                target: sid(12)
            })
        );
        assert_eq!(
            run(
                &mut state,
                set(99, node_keys::LOCKED, PropertyValue::Bool(true))
            ),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        // 공통 필드는 Text에도 설정 가능.
        run(
            &mut state,
            set(20, node_keys::LOCKED, PropertyValue::Bool(true)),
        )
        .unwrap();
        assert!(state.model().record(sid(20)).unwrap().locked);
    }

    /// 7종 노드(Frame·Rectangle·Ellipse·Line·Polygon·Text·Image)를 root 아래에 둔 모델과 id.
    pub(crate) fn seven_kind_model() -> (DocumentModel, [StableId; 7]) {
        use stellar_core_types::reference::AssetRef;
        use stellar_core_types::value::Angle;
        use stellar_document::shape::{LineParams, RectangleParams};
        let mut model = small_model();
        let rect = |w, h| Rect::new(Vec2::ZERO, Size2::new(w, h).unwrap());
        let payloads = [
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(80., 40.).unwrap(),
                corner_radius: CornerRadius::new(1., 2., 3., 4.).unwrap(),
            })),
            NodePayload::Shape(ShapeGeometry::Ellipse(
                EllipseParams::new(rect(30., 20.)).unwrap(),
            )),
            NodePayload::Shape(ShapeGeometry::Line(LineParams {
                start: Vec2::ZERO,
                end: Vec2::new(40., -10.).unwrap(),
                cap: LineCap::Butt,
            })),
            NodePayload::Shape(ShapeGeometry::Polygon(
                PolygonParams::new(rect(50., 50.), 5, Angle::new(0.).unwrap(), 0.).unwrap(),
            )),
            NodePayload::Text(stellar_document::text::TextPayload::empty(0)),
            NodePayload::Image(ImagePayload::new(
                AssetRef::new(sid(90)),
                Size2::new(64., 48.).unwrap(),
            )),
        ];
        let mut ids = [sid(11); 7];
        for (index, payload) in payloads.into_iter().enumerate() {
            let mut record =
                crate::command::test_fixtures::frame(30 + index as u64, Some(10), 0x30);
            record.order_key =
                stellar_core_types::order_key::OrderKey::from_bytes(vec![0x30 + index as u8])
                    .unwrap();
            record.payload = payload;
            ids[index + 1] = record.id;
            model.insert_node(record).unwrap();
        }
        (model, ids)
    }

    #[test]
    fn node_property_keys_cover_seven_node_kinds_and_round_trip() {
        // 검증 기준(UX.2.1): node_property_info가 7종 노드의 키를 반환, key 상위 byte 규칙.
        use stellar_document::payload_metadata::{image_keys, shape_keys};
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let expected: [&[PropertyKey]; 7] = [
            &[
                frame_keys::WIDTH,
                frame_keys::BACKGROUND,
                frame_keys::BORDER_TOP_LEFT_RADIUS,
            ],
            &[shape_keys::WIDTH, shape_keys::BORDER_BOTTOM_LEFT_RADIUS],
            &[shape_keys::HEIGHT],
            &[shape_keys::X1, shape_keys::Y2, shape_keys::STROKE_LINECAP],
            &[shape_keys::POLYGON_SIDES, shape_keys::POLYGON_ROTATION],
            &[node_keys::DISPLAY_NAME],
            &[image_keys::WIDTH, image_keys::OBJECT_FIT],
        ];
        let upper = [0x01, 0x03, 0x03, 0x03, 0x03, 0x05, 0x06];
        for ((id, wanted), upper) in ids.into_iter().zip(expected).zip(upper) {
            let record = state.model().record(id).unwrap();
            let keys = node_property_keys(&record.payload);
            for key in wanted {
                assert!(keys.contains(key), "{:?} {key:?}", record.kind());
            }
            for key in &keys {
                let info = payload_property_info(&record.payload, *key).unwrap();
                assert_eq!(
                    node_property_info(record.kind(), *key).map(|i| i.value_type),
                    Some(info.value_type)
                );
                if key.0 > 0xFF {
                    assert_eq!(key.0 >> 8, upper, "{key:?}");
                }
                // 현재 값을 다시 쓰면 같은 값, 역 command로 원래 상태.
                let current = read_property(&record, *key).unwrap();
                assert_eq!(current.value_type(), info.value_type, "{key:?}");
            }
        }
        // Shape 편집과 Undo.
        let before = model_snapshot_json(state.model()).unwrap();
        let edits = [
            (
                ids[1],
                shape_keys::WIDTH,
                PropertyValue::NonNegative(FiniteF32::new(120.).unwrap()),
            ),
            (
                ids[1],
                shape_keys::BORDER_TOP_RIGHT_RADIUS,
                PropertyValue::NonNegative(FiniteF32::new(9.).unwrap()),
            ),
            (
                ids[2],
                shape_keys::HEIGHT,
                PropertyValue::NonNegative(FiniteF32::new(7.).unwrap()),
            ),
            (
                ids[3],
                shape_keys::X2,
                PropertyValue::Number(FiniteF32::new(-5.).unwrap()),
            ),
            (
                ids[3],
                shape_keys::STROKE_LINECAP,
                PropertyValue::Keyword(1),
            ),
            (
                ids[4],
                shape_keys::POLYGON_SIDES,
                PropertyValue::Number(FiniteF32::new(8.).unwrap()),
            ),
            (
                ids[4],
                shape_keys::POLYGON_ROTATION,
                PropertyValue::Number(FiniteF32::new(540.).unwrap()),
            ),
            (
                ids[6],
                image_keys::HEIGHT,
                PropertyValue::NonNegative(FiniteF32::new(10.).unwrap()),
            ),
            (ids[6], image_keys::OBJECT_FIT, PropertyValue::Keyword(2)),
        ];
        let mut inverses = Vec::new();
        for (node, key, value) in edits {
            let request = CommandRequest::at(
                &state,
                Command::SetProperty(SetProperty {
                    node,
                    key,
                    value: value.clone(),
                }),
            );
            let outcome = state.execute(&request).unwrap();
            assert_eq!(
                read_property(&state.model().record(node).unwrap(), key),
                Some(value)
            );
            inverses.push(outcome.inverse);
        }
        for inverse in inverses.into_iter().rev() {
            let request = CommandRequest::at(&state, inverse);
            state.execute(&request).unwrap();
        }
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        // 도형 종류에 없는 key·범위 밖 값은 거부합니다.
        let run = |state: &mut DocumentState, node, key, value| {
            let request = CommandRequest::at(
                state,
                Command::SetProperty(SetProperty { node, key, value }),
            );
            state.execute(&request).map(|_| ())
        };
        assert!(matches!(
            run(
                &mut state,
                ids[2],
                shape_keys::X1,
                PropertyValue::Number(FiniteF32::new(1.).unwrap())
            ),
            Err(CommandError::UnknownProperty { .. })
        ));
        assert!(run(
            &mut state,
            ids[4],
            shape_keys::POLYGON_SIDES,
            PropertyValue::Number(FiniteF32::new(2.5).unwrap())
        )
        .is_err());
        assert!(run(
            &mut state,
            ids[6],
            image_keys::OBJECT_FIT,
            PropertyValue::Keyword(9)
        )
        .is_err());
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }

    #[test]
    fn set_property_codec_round_trips_every_value_type() {
        let types = [
            PropertyValueType::Bool,
            PropertyValueType::Text,
            PropertyValueType::Size,
            PropertyValueType::Transform,
            PropertyValueType::Clipping,
            PropertyValueType::OptionalColor,
            PropertyValueType::OptionalBorder,
            PropertyValueType::CornerRadius,
            PropertyValueType::OptionalLayoutRef,
        ];
        let mut values: Vec<PropertyValue> = types.iter().map(|t| sample_value(*t)).collect();
        values.extend([
            PropertyValue::Clipping(Clipping::FrameBounds),
            PropertyValue::Clipping(Clipping::Path(NodeRef::new(sid(5)))),
            PropertyValue::OptionalColor(None),
            PropertyValue::OptionalBorder(None),
            PropertyValue::OptionalLayoutRef(None),
            PropertyValue::Number(FiniteF32::new(-2.5).unwrap()),
            PropertyValue::Keyword(3),
        ]);
        for value in values {
            let command = set(11, frame_keys::SIZE, value);
            let bytes = encode_command(&command);
            assert_eq!(&bytes[..2], &0x0010u16.to_le_bytes());
            assert_eq!(decode_command(&bytes).unwrap(), command);
        }
        let mut bad = encode_command(&set(11, frame_keys::SIZE, PropertyValue::Bool(true)));
        let tag_at = 2 + 4 + 16 + 4;
        bad[tag_at] = 0x7F;
        assert!(decode_command(&bad).is_err());
    }

    #[test]
    fn specializations_use_the_same_core_patch_path() {
        // 검증 기준(VII.2.5): 동일 core patch 경로 사용 확인.
        use crate::kind::CommandKind;
        let specializations = [
            (Command::set_visibility(sid(11), false), node_keys::VISIBLE),
            (Command::set_locked(sid(11), true), node_keys::LOCKED),
            (
                Command::rename_node(sid(11), "새 이름"),
                node_keys::DISPLAY_NAME,
            ),
        ];
        // 별도 command 종류가 없습니다.
        for name in ["set-visibility", "set-locked", "rename-node"] {
            assert!(CommandKind::ALL.iter().all(|k| k.name() != name), "{name}");
        }
        for (command, key) in specializations {
            assert_eq!(command.kind(), CommandKind::SetProperty);
            let Command::SetProperty(patch) = &command else {
                panic!("SetProperty여야 함");
            };
            assert_eq!(patch.key, key);
            // 같은 직렬화(kind 0x0010).
            let bytes = encode_command(&command);
            assert_eq!(&bytes[..2], &0x0010u16.to_le_bytes());
            assert_eq!(decode_command(&bytes).unwrap(), command);

            // 같은 apply 경로: core patch 호출 1회, 일반 SetProperty와 같은 결과.
            let mut via_specialization = DocumentState::new(small_model());
            let calls = core_patch_calls();
            let request = CommandRequest::at(&via_specialization, command.clone());
            let outcome = via_specialization.execute(&request).unwrap();
            assert_eq!(core_patch_calls(), calls + 1);
            let mut via_generic = DocumentState::new(small_model());
            let generic = set(11, key, patch.value.clone());
            let request = CommandRequest::at(&via_generic, generic);
            let generic_outcome = via_generic.execute(&request).unwrap();
            assert_eq!(
                model_snapshot_json(via_specialization.model()).unwrap(),
                model_snapshot_json(via_generic.model()).unwrap()
            );
            assert_eq!(outcome.changes, generic_outcome.changes);
            // undo도 SetProperty.
            assert_eq!(outcome.inverse.kind(), CommandKind::SetProperty);
        }
        // 의미 확인.
        let mut state = DocumentState::new(small_model());
        for command in [
            Command::set_visibility(sid(12), false),
            Command::set_locked(sid(12), true),
            Command::rename_node(sid(12), "카드"),
        ] {
            let request = CommandRequest::at(&state, command);
            state.execute(&request).unwrap();
        }
        let record = state.model().record(sid(12)).unwrap();
        assert!(!record.visible && record.locked);
        assert_eq!(record.display_name, "카드");
    }
}
