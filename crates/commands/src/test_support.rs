//! **테스트 전용** command 생성기 (VII.6).
//!
//! property test(VII.6.1·VII.6.2)와 fuzz target(VII.6.3)이 쓰는 결정론적 command 생성기입니다.
//! 프로덕션 경로는 이 모듈을 호출하지 않습니다. 다른 crate(history)의 테스트도 쓸 수 있도록 공개
//! 모듈로 두되 문서에서는 숨깁니다(`#[doc(hidden)]`). PRNG는 document test_support의 SplitMix64를
//! 재사용합니다(외부 crate 없음).
//!
//! 생성기는 현재 문서를 보고 **대체로 유효한** command를 만듭니다. 유효성은 command의 `validate`가
//! 최종 판단하므로, 호출자는 실패한 후보를 건너뛰거나 [`random_valid_request`]를 씁니다.
#![cfg(any(test, feature = "test-support"))]

use stellar_core_types::clock::ManualClock;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::{StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::frame_metadata::FRAME_PROPERTIES;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    BlendMode, BooleanOp, ModifierEntry, ModifierId, ModifierKind, ModifierPayload,
};
use stellar_document::modifier_metadata::{keys as modifier_keys, properties_of};
use stellar_document::node_kind::NodeKind;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_document::test_support::random_document::SplitMix64;
use stellar_document::text::TextPayload;
use stellar_document::validate::{can_have_children, MAX_NODE_DEPTH};

use crate::change::node_keys;
use crate::command::Command;
use crate::convert::{ConvertFragmentToFrame, ConvertShapeToPath, RestoreNodeRecords};
use crate::duplicate::DuplicateSubtree;
use crate::kind::CommandKind;
use crate::modifier_ops::{InsertModifier, MoveModifier, RemoveModifier, SetModifierProperty};
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use crate::property::{node_property_info, PropertyValue, PropertyValueType, SetProperty};
use crate::request::CommandRequest;
use crate::state::DocumentState;
use crate::tree::{depth_of, node_ids, order_key_of, parent_of, ChildIndex};

/// 문서에서 쓰이지 않는 새 StableId 공급기(문서 생성기 id 대역 밖의 timestamp 사용).
#[derive(Debug, Clone)]
pub struct FreshIds {
    next: u64,
    rng: SplitMix64,
}

impl FreshIds {
    /// seed별 공급기.
    pub fn new(seed: u64) -> FreshIds {
        FreshIds {
            next: 0,
            rng: SplitMix64::new(seed ^ 0xF4E5_11D5),
        }
    }

    /// 다음 새 id.
    pub fn fresh(&mut self) -> StableId {
        self.next += 1;
        StableId::new_v7(
            4_000_000_000_000 + self.next,
            (self.rng.next_u64() & 0x0FFF) as u16,
            self.rng.next_u64(),
        )
    }
}

fn pick<T: Copy>(rng: &mut SplitMix64, items: &[T]) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(*rng.pick(items))
    }
}

fn grid(rng: &mut SplitMix64, range: u64) -> f32 {
    (rng.below(range * 8 + 1) as f32 - (range * 4) as f32) / 4.0
}

fn length(rng: &mut SplitMix64, range: u64) -> f32 {
    rng.below(range * 4 + 1) as f32 / 4.0
}

fn color(rng: &mut SplitMix64) -> Color {
    let channel = |rng: &mut SplitMix64| rng.below(17) as f32 / 16.0;
    let (r, g, b, a) = (channel(rng), channel(rng), channel(rng), channel(rng));
    Color::new(ColorSpace::Srgb, r, g, b, a)
        .unwrap_or_else(|_| Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("상수 색"))
}

fn transform(rng: &mut SplitMix64) -> Transform2D {
    let tx = grid(rng, 200);
    let ty = grid(rng, 200);
    Transform2D::new(1.0, 0.0, 0.0, 1.0, tx, ty).unwrap_or_else(|_| Transform2D::identity())
}

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).unwrap_or(FiniteF32::ZERO)
}

/// 부모의 자식 사이 무작위 위치의 새 OrderKey.
fn key_among(
    model: &DocumentModel,
    index: &ChildIndex,
    parent: StableId,
    rng: &mut SplitMix64,
    skip: Option<StableId>,
) -> OrderKey {
    let siblings: Vec<StableId> = index
        .children(parent)
        .iter()
        .copied()
        .filter(|id| Some(*id) != skip)
        .collect();
    let at = rng.below(siblings.len() as u64 + 1) as usize;
    let left = at
        .checked_sub(1)
        .and_then(|i| siblings.get(i))
        .and_then(|id| order_key_of(model, *id));
    let right = siblings.get(at).and_then(|id| order_key_of(model, *id));
    OrderKey::between(left.as_ref(), right.as_ref())
}

fn nodes_where(model: &DocumentModel, keep: impl Fn(StableId) -> bool) -> Vec<StableId> {
    node_ids(model).into_iter().filter(|id| keep(*id)).collect()
}

fn kind_of(model: &DocumentModel, id: StableId) -> Option<NodeKind> {
    model.payload(id).map(NodePayload::kind)
}

fn non_root(model: &DocumentModel, id: StableId) -> bool {
    matches!(parent_of(model, id), Some(Some(_)))
}

fn property_value(
    rng: &mut SplitMix64,
    value_type: PropertyValueType,
    model: &DocumentModel,
) -> PropertyValue {
    match value_type {
        PropertyValueType::Bool => PropertyValue::Bool(rng.one_in(2)),
        PropertyValueType::Text => PropertyValue::Text(format!("이름 {}", rng.below(100))),
        PropertyValueType::Size => PropertyValue::Size(
            Size2::new(length(rng, 400), length(rng, 400)).unwrap_or(Size2::ZERO),
        ),
        PropertyValueType::Transform => PropertyValue::Transform(transform(rng)),
        PropertyValueType::Clipping => PropertyValue::Clipping(match rng.below(3) {
            0 => Clipping::Disabled,
            1 => Clipping::FrameBounds,
            _ => {
                let paths = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Path));
                match pick(rng, &paths) {
                    Some(path) => Clipping::Path(NodeRef::new(path)),
                    None => Clipping::FrameBounds,
                }
            }
        }),
        PropertyValueType::OptionalColor => {
            PropertyValue::OptionalColor(rng.one_in(2).then(|| color(rng)))
        }
        PropertyValueType::OptionalBorder => PropertyValue::OptionalBorder(if rng.one_in(2) {
            Border::new(color(rng), length(rng, 8)).ok()
        } else {
            None
        }),
        PropertyValueType::CornerRadius => PropertyValue::CornerRadius(
            CornerRadius::new(
                length(rng, 20),
                length(rng, 20),
                length(rng, 20),
                length(rng, 20),
            )
            .unwrap_or(CornerRadius::ZERO),
        ),
        PropertyValueType::OptionalLayoutRef => PropertyValue::OptionalLayoutRef(
            rng.one_in(2)
                .then(|| StableId::new_v7(7, 7, rng.next_u64())),
        ),
        PropertyValueType::BooleanOp => PropertyValue::BooleanOp(*rng.pick(&[
            BooleanOp::Union,
            BooleanOp::Subtract,
            BooleanOp::Intersect,
            BooleanOp::Exclude,
        ])),
        PropertyValueType::NodeRefList => {
            let ids = node_ids(model);
            let count = rng.below(3);
            let refs = (0..count)
                .filter_map(|_| pick(rng, &ids))
                .map(NodeRef::new)
                .collect();
            PropertyValue::NodeRefList(refs)
        }
        PropertyValueType::NodeRef => {
            let ids = node_ids(model);
            PropertyValue::NodeRef(NodeRef::new(if rng.one_in(3) {
                StableId::NIL
            } else {
                pick(rng, &ids).unwrap_or(StableId::NIL)
            }))
        }
        PropertyValueType::NonNegative => PropertyValue::NonNegative(finite(length(rng, 16))),
        PropertyValueType::Color => PropertyValue::Color(color(rng)),
        PropertyValueType::Offset => {
            PropertyValue::Offset(Vec2::new(grid(rng, 20), grid(rng, 20)).unwrap_or(Vec2::ZERO))
        }
        PropertyValueType::UnitInterval => {
            PropertyValue::UnitInterval(finite(rng.below(9) as f32 / 8.0))
        }
        PropertyValueType::BlendMode => PropertyValue::BlendMode(*rng.pick(&[
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
        ])),
    }
}

fn random_payload(rng: &mut SplitMix64) -> NodePayload {
    match rng.below(4) {
        0 => NodePayload::Frame(FramePayload {
            size: Size2::new(length(rng, 200), length(rng, 200)).unwrap_or(Size2::ZERO),
            transform: transform(rng),
            ..FramePayload::default()
        }),
        1 => NodePayload::Fragment(FragmentPayload),
        2 => NodePayload::Text(TextPayload::empty(0)),
        _ => NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(length(rng, 100), length(rng, 100)).unwrap_or(Size2::ZERO),
            corner_radius: CornerRadius::ZERO,
        })),
    }
}

/// `kind` command 후보를 만듭니다. 대상이 없으면 `None`입니다.
///
/// `match`에 wildcard가 없어 새 command 종류를 등록하면 생성기가 없어 컴파일이 실패합니다.
pub fn generate(
    kind: CommandKind,
    model: &DocumentModel,
    rng: &mut SplitMix64,
    ids: &mut FreshIds,
) -> Option<Command> {
    let index = ChildIndex::build(model);
    match kind {
        CommandKind::InsertNode => {
            let containers = nodes_where(model, |id| {
                kind_of(model, id).is_some_and(can_have_children)
                    && depth_of(model, id).is_some_and(|d| d < MAX_NODE_DEPTH)
            });
            let parent = pick(rng, &containers)?;
            let order_key = key_among(model, &index, parent, rng, None);
            Some(Command::InsertNode(InsertNode::single(NodeRecord {
                id: ids.fresh(),
                parent: Some(parent),
                order_key,
                visible: !rng.one_in(6),
                locked: rng.one_in(6),
                display_name: format!("새 {}", rng.below(1000)),
                payload: random_payload(rng),
            })))
        }
        CommandKind::DeleteSubtree => {
            let candidates = nodes_where(model, |id| non_root(model, id));
            Some(Command::DeleteSubtree(DeleteSubtree {
                root: pick(rng, &candidates)?,
            }))
        }
        CommandKind::MoveNode => {
            let movable = nodes_where(model, |id| non_root(model, id));
            let node = pick(rng, &movable)?;
            let containers = nodes_where(model, |id| {
                kind_of(model, id).is_some_and(can_have_children)
            });
            let new_parent = pick(rng, &containers)?;
            let order_key = key_among(model, &index, new_parent, rng, Some(node));
            Some(Command::MoveNode(MoveNode {
                node,
                new_parent,
                order_key,
            }))
        }
        CommandKind::SetProperty => {
            let node = pick(rng, &node_ids(model))?;
            let node_kind = kind_of(model, node)?;
            let mut keys = vec![
                node_keys::VISIBLE,
                node_keys::LOCKED,
                node_keys::DISPLAY_NAME,
            ];
            if node_kind == NodeKind::Frame {
                keys.extend(FRAME_PROPERTIES.iter().map(|m| m.key));
            }
            let key: PropertyKey = *rng.pick(&keys);
            let info = node_property_info(node_kind, key)?;
            let value = property_value(rng, info.value_type, model);
            Some(Command::SetProperty(SetProperty { node, key, value }))
        }
        CommandKind::InsertModifier => {
            let owners = nodes_where(model, |id| {
                kind_of(model, id).is_some_and(|k| k != NodeKind::Fragment)
            });
            let node = pick(rng, &owners)?;
            let kind = *rng.pick(&ModifierKind::ALL);
            let last = model
                .modifier_stack(node)
                .and_then(|s| s.entries().last().map(|e| e.order.clone()));
            Some(Command::InsertModifier(InsertModifier {
                node,
                entry: ModifierEntry {
                    id: ModifierId::new(ids.fresh()),
                    order: OrderKey::between(last.as_ref(), None),
                    kind,
                    enabled: !rng.one_in(4),
                    payload: ModifierPayload::default_for(kind),
                },
            }))
        }
        CommandKind::RemoveModifier
        | CommandKind::MoveModifier
        | CommandKind::SetModifierProperty => {
            let owners = nodes_where(model, |id| model.modifier_stack(id).is_some());
            let node = pick(rng, &owners)?;
            let stack = model.modifier_stack(node)?;
            let position = rng.below(stack.entries().len() as u64) as usize;
            let entry = stack.entries().get(position)?;
            match kind {
                CommandKind::RemoveModifier => Some(Command::RemoveModifier(RemoveModifier {
                    node,
                    modifier: entry.id,
                })),
                CommandKind::MoveModifier => {
                    let others: Vec<&OrderKey> = stack
                        .entries()
                        .iter()
                        .filter(|e| e.id != entry.id)
                        .map(|e| &e.order)
                        .collect();
                    let at = rng.below(others.len() as u64 + 1) as usize;
                    let left = at.checked_sub(1).and_then(|i| others.get(i).copied());
                    let right = others.get(at).copied();
                    Some(Command::MoveModifier(MoveModifier {
                        node,
                        modifier: entry.id,
                        order: OrderKey::between(left, right),
                    }))
                }
                _ => {
                    let keys: Vec<PropertyKey> = properties_of(entry.kind)
                        .into_iter()
                        .map(|m| m.key)
                        .filter(|key| *key != modifier_keys::ORDER)
                        .collect();
                    let key = *rng.pick(&keys);
                    let metadata = stellar_document::modifier_metadata::modifier_property(key)?;
                    let value_type = modifier_value_type(metadata.value_type)?;
                    Some(Command::SetModifierProperty(SetModifierProperty {
                        node,
                        modifier: entry.id,
                        key,
                        value: property_value(rng, value_type, model),
                    }))
                }
            }
        }
        CommandKind::ConvertFragmentToFrame => {
            let fragments = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Fragment));
            Some(Command::ConvertFragmentToFrame(ConvertFragmentToFrame {
                node: pick(rng, &fragments)?,
            }))
        }
        CommandKind::ConvertShapeToPath => {
            let shapes = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Shape));
            let node = pick(rng, &shapes)?;
            let NodePayload::Shape(geometry) = model.payload(node)? else {
                return None;
            };
            let count = ConvertShapeToPath::required_ids(geometry);
            Some(Command::ConvertShapeToPath(ConvertShapeToPath {
                node,
                element_ids: (0..count).map(|_| ids.fresh()).collect(),
            }))
        }
        CommandKind::RestoreNodeRecords => {
            let node = pick(rng, &node_ids(model))?;
            let mut record = model.record(node)?;
            record.display_name = format!("복원 {}", rng.below(1000));
            record.visible = !record.visible;
            // 종류 변경도 섞습니다(View root 종류 변경 등 불변식 위반 후보는 validate가 거부해야 함).
            if rng.one_in(3) {
                record.payload = random_payload(rng);
            }
            Some(Command::RestoreNodeRecords(RestoreNodeRecords {
                records: vec![record],
            }))
        }
        // command 계층 자체용 test double은 property 대상이 아닙니다.
        #[cfg(test)]
        CommandKind::TestProbe => None,
        CommandKind::DuplicateSubtree => {
            let sources = node_ids(model);
            let source = pick(rng, &sources)?;
            let containers = nodes_where(model, |id| {
                kind_of(model, id).is_some_and(can_have_children)
            });
            let parent = pick(rng, &containers)?;
            let order_key = key_among(model, &index, parent, rng, None);
            // 복제 id는 FreshIds와 다른 timestamp 대역(4.1e12 ms)에서 seed별로 만듭니다.
            let clock = ManualClock::new(
                (4_100_000_000_000 + ids.fresh().timestamp_ms() % 1_000_000) * 1_000_000,
            );
            let mut generator = StableIdGenerator::new(rng.next_u64());
            DuplicateSubtree::plan(model, source, parent, order_key, &mut generator, &clock)
                .ok()
                .map(Command::DuplicateSubtree)
        }
    }
}

fn modifier_value_type(
    value_type: stellar_document::modifier_metadata::ModifierPropertyType,
) -> Option<PropertyValueType> {
    use stellar_document::modifier_metadata::ModifierPropertyType as T;
    Some(match value_type {
        T::Bool => PropertyValueType::Bool,
        T::OrderKey => return None,
        T::Transform => PropertyValueType::Transform,
        T::BooleanOp => PropertyValueType::BooleanOp,
        T::NodeRefList => PropertyValueType::NodeRefList,
        T::NodeRef => PropertyValueType::NodeRef,
        T::NonNegative => PropertyValueType::NonNegative,
        T::Color => PropertyValueType::Color,
        T::Offset => PropertyValueType::Offset,
        T::UnitInterval => PropertyValueType::UnitInterval,
        T::BlendMode => PropertyValueType::BlendMode,
    })
}

/// 무작위 종류의 **유효한** command 요청(현재 revision 기준)을 만듭니다. `attempts` 안에 유효 후보를
/// 찾지 못하면 `None`입니다.
pub fn random_valid_request(
    state: &DocumentState,
    rng: &mut SplitMix64,
    ids: &mut FreshIds,
    attempts: usize,
) -> Option<CommandRequest> {
    for _ in 0..attempts {
        let kind = *rng.pick(CommandKind::ALL);
        let Some(command) = generate(kind, state.model(), rng, ids) else {
            continue;
        };
        if command.validate(state.model()).is_ok() {
            return Some(CommandRequest::at(state, command));
        }
    }
    None
}
