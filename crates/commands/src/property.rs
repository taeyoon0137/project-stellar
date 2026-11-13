//! `SetProperty` command와 속성 값 (VII.2.4).
//!
//! 속성은 문자열이 아니라 numeric [`PropertyKey`]로 지정하며, **metadata registry**가 node
//! 종류별로 허용 key, 값 타입, 무효화 class를 정합니다:
//!
//! - 공통 node 필드(모든 종류): 표시 여부·잠금(`Bool`), 표시 이름(`Text`) — [`node_keys`].
//!   부모·순서는 [`MoveNode`](crate::node_ops::MoveNode)로만 바꿉니다.
//! - Frame payload: Frame registry(VI.3.5)의 7개 속성.
//!
//! 등록되지 않은 key는 [`CommandError::UnknownProperty`], 값 타입이 다르면
//! [`CommandError::PropertyTypeMismatch`]로 거부합니다. 결과 change set의 무효화 class도 같은
//! registry에서 가져옵니다.
//!
//! 값 직렬화: type tag `u8` + 값(f32는 LE bit 그대로, 색은 [`Color::to_bytes`], id는 16바이트).
//! tag는 저장·journal 계약입니다.

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::frame::{keys as frame_keys, Border, Clipping, CornerRadius, FramePayload};
use stellar_document::frame_metadata::{frame_property, FramePropertyType};
use stellar_document::model::DocumentModel;
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_document::node_kind::NodeKind;
use stellar_document::records::{NodePayload, NodeRecord};

use crate::change::{node_field_invalidation, node_keys};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::{id_owners, IdOwner};

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
            _ => Err(malformed("알 수 없는 속성 값 tag")),
        }
    }
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
        FramePropertyType::Transform => PropertyValueType::Transform,
        FramePropertyType::Clipping => PropertyValueType::Clipping,
        FramePropertyType::OptionalColor => PropertyValueType::OptionalColor,
        FramePropertyType::OptionalBorder => PropertyValueType::OptionalBorder,
        FramePropertyType::CornerRadius => PropertyValueType::CornerRadius,
        FramePropertyType::OptionalLayoutRef => PropertyValueType::OptionalLayoutRef,
    }
}

/// `kind` node에서 `key`가 SetProperty로 설정 가능한 속성인지와 그 metadata.
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
        _ => None,
    }
}

/// 현재 값을 읽습니다(registry에 있는 key만).
fn read_property(record: &NodeRecord, key: PropertyKey) -> Option<PropertyValue> {
    match key {
        node_keys::VISIBLE => return Some(PropertyValue::Bool(record.visible)),
        node_keys::LOCKED => return Some(PropertyValue::Bool(record.locked)),
        node_keys::DISPLAY_NAME => return Some(PropertyValue::Text(record.display_name.clone())),
        _ => {}
    }
    let NodePayload::Frame(frame) = &record.payload else {
        return None;
    };
    Some(match key {
        frame_keys::SIZE => PropertyValue::Size(frame.size),
        frame_keys::TRANSFORM => PropertyValue::Transform(frame.transform),
        frame_keys::CLIPPING => PropertyValue::Clipping(frame.clipping),
        frame_keys::BACKGROUND => PropertyValue::OptionalColor(frame.background),
        frame_keys::BORDER => PropertyValue::OptionalBorder(frame.border),
        frame_keys::CORNER_RADIUS => PropertyValue::CornerRadius(frame.corner_radius),
        frame_keys::LAYOUT_MODE => PropertyValue::OptionalLayoutRef(frame.layout_mode),
        _ => return None,
    })
}

/// Frame payload에 값을 씁니다. 타입이 맞지 않으면 `false`.
fn write_frame(frame: &mut FramePayload, key: PropertyKey, value: &PropertyValue) -> bool {
    match (key, value) {
        (frame_keys::SIZE, PropertyValue::Size(v)) => frame.size = *v,
        (frame_keys::TRANSFORM, PropertyValue::Transform(v)) => frame.transform = *v,
        (frame_keys::CLIPPING, PropertyValue::Clipping(v)) => frame.clipping = *v,
        (frame_keys::BACKGROUND, PropertyValue::OptionalColor(v)) => frame.background = *v,
        (frame_keys::BORDER, PropertyValue::OptionalBorder(v)) => frame.border = *v,
        (frame_keys::CORNER_RADIUS, PropertyValue::CornerRadius(v)) => frame.corner_radius = *v,
        (frame_keys::LAYOUT_MODE, PropertyValue::OptionalLayoutRef(v)) => frame.layout_mode = *v,
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

impl SetProperty {
    fn info(&self, model: &DocumentModel) -> Result<PropertyInfo, CommandError> {
        let kind = model
            .payload(self.node)
            .map(NodePayload::kind)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        if self.key == node_keys::PARENT || self.key == node_keys::ORDER_KEY {
            return Err(CommandError::PropertyNotSettable { key: self.key });
        }
        node_property_info(kind, self.key).ok_or(CommandError::UnknownProperty {
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
        // clip 대상이 문서에 있으면 Path여야 합니다(없으면 미해결 진단).
        if let PropertyValue::Clipping(Clipping::Path(target)) = &self.value {
            match id_owners(model).get(&target.id()) {
                None | Some(IdOwner::Node(NodeKind::Path)) => {}
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
                let written = match &mut record.payload {
                    NodePayload::Frame(frame) => write_frame(frame, key, value),
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
mod tests {
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
}
