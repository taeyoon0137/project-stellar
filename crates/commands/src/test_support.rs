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
use crate::page_view::{AddPage, AddView, RemovePage, RemoveView, RenamePage, RenameView};
use crate::path_edit::EditPathPoint;
use crate::property::{node_property_info, PropertyValue, PropertyValueType, SetProperty};
use crate::request::CommandRequest;
use crate::snapshot_apply::ApplyExternalSnapshot;
use crate::state::DocumentState;
use crate::tree::{depth_of, node_ids, order_key_of, parent_of, ChildIndex};
use stellar_core_types::reference::ComponentRef;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::path::{HandleMode, PathCommand, PathEdit, PathHandleId, PathPointId};

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
    index: &ChildIndex<'_>,
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
        PropertyValueType::Number => PropertyValue::Number(finite(grid(rng, 20))),
        PropertyValueType::Keyword => PropertyValue::Keyword(rng.below(3) as u8),
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
        CommandKind::ReplaceText => {
            let candidates = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Text));
            let node = pick(rng, &candidates)?;
            let NodePayload::Text(text) = model.payload(node)? else {
                return None;
            };
            let len = stellar_document::text::scalar_len(text.content());
            let start = rng.below(u64::from(len) + 1) as u32;
            let end = start + rng.below(u64::from(len - start) + 1) as u32;
            let replacement = rng.pick(&["한글", "e\u{301}", "🎨\n"]).to_string();
            crate::text_edit::ReplaceText::new(node, start, end, replacement)
                .map(Command::ReplaceText)
        }
        CommandKind::SetTextRangeStyle => {
            let candidates = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Text));
            let node = pick(rng, &candidates)?;
            let NodePayload::Text(text) = model.payload(node)? else {
                return None;
            };
            let len = stellar_document::text::scalar_len(text.content());
            if len == 0 {
                return None;
            }
            let start = rng.below(u64::from(len)) as u32;
            let end = start + 1 + rng.below(u64::from(len - start)) as u32;
            let styles: Vec<u32> = model
                .definition()
                .text_styles
                .runs
                .keys()
                .copied()
                .collect();
            let style = if rng.one_in(3) {
                None
            } else if styles.is_empty() {
                Some(rng.below(3) as u32)
            } else {
                Some(*rng.pick(&styles))
            };
            Some(Command::SetTextRangeStyle(
                crate::text_edit::SetTextRangeStyle {
                    node,
                    range: stellar_document::text::TextRange::new(start, end)?,
                    style,
                },
            ))
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
            let count = ConvertShapeToPath::required_ids(geometry).ok()?;
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
        CommandKind::AddPage => {
            let index = rng.below(model.definition().pages.len() as u64 + 1) as u32;
            Some(Command::AddPage(AddPage::with_empty_view(
                index,
                ids.fresh(),
                format!("페이지 {}", rng.below(100)),
                ids.fresh(),
                String::new(),
                ids.fresh(),
            )))
        }
        CommandKind::RemovePage | CommandKind::RenamePage => {
            let pages: Vec<StableId> = model.definition().pages.iter().map(|p| p.id).collect();
            let page = pick(rng, &pages)?;
            Some(match kind {
                CommandKind::RemovePage => Command::RemovePage(RemovePage { page }),
                _ => Command::RenamePage(RenamePage {
                    page,
                    name: rng.pick(&["표지", "", "🎨 화면"]).to_string(),
                }),
            })
        }
        CommandKind::AddView => {
            let pages: Vec<(StableId, usize)> = model
                .definition()
                .pages
                .iter()
                .map(|p| (p.id, p.views.len()))
                .collect();
            let (page, count) = pick(rng, &pages)?;
            Some(Command::AddView(AddView::empty(
                page,
                rng.below(count as u64 + 1) as u32,
                ids.fresh(),
                format!("View {}", rng.below(100)),
                ids.fresh(),
            )))
        }
        CommandKind::ApplyExternalSnapshot => {
            // 현재 문서에 무작위 편집 하나를 가한 결과를 외부 원본으로 삼습니다.
            let mut edited = DocumentState::new(
                DocumentModel::from_records(model.to_records().ok()?)
                    .ok()?
                    .0,
            );
            for _ in 0..8 {
                let other = *rng.pick(CommandKind::ALL);
                if other == CommandKind::ApplyExternalSnapshot {
                    continue;
                }
                let Some(command) = generate(other, edited.model(), rng, ids) else {
                    continue;
                };
                if edited
                    .execute(&CommandRequest::at(&edited, command))
                    .is_ok()
                {
                    break;
                }
            }
            let incoming = edited.model().to_records().ok()?;
            let command = ApplyExternalSnapshot::between(model, &incoming).ok()?;
            (!command.is_empty()).then_some(Command::ApplyExternalSnapshot(command))
        }
        CommandKind::SwapInstance => {
            let instances = nodes_where(model, |id| {
                kind_of(model, id) == Some(NodeKind::ComponentInstance)
            });
            let node = pick(rng, &instances)?;
            let NodePayload::ComponentInstance(current) = model.payload(node)? else {
                return None;
            };
            // override를 일부만 옮긴 다른 컴포넌트 참조.
            let mut payload = ComponentInstancePayload::new(ComponentRef::new(ids.fresh()));
            for (key, value) in current.property_overrides() {
                if rng.one_in(2) {
                    payload.set_property_override(key.clone(), value.clone());
                }
            }
            Some(Command::SwapInstance(crate::node_ops::SwapInstance {
                node,
                payload,
            }))
        }
        CommandKind::DetachInstance | CommandKind::PlaceSlotContent => {
            // 인스턴스 구조 command(ADR-0129). Detach는 정의 library가 있어야 계획되므로 여기서는 구조 규칙만 쓰는
            // 후보를 만듭니다: 인스턴스 → 빈 Frame(Detach), 다른 node 하나를 새 slot에 투영(Slot 배치).
            let instances = nodes_where(model, |id| {
                kind_of(model, id) == Some(NodeKind::ComponentInstance)
            });
            let node = pick(rng, &instances)?;
            let mut record = model.record(node)?;
            let NodePayload::ComponentInstance(current) = record.payload.clone() else {
                return None;
            };
            let layout = model
                .layout_record(node)
                .cloned()
                .unwrap_or_else(|| stellar_document::layout_record::LayoutRecord::new(node));
            let mut moves = Vec::new();
            if kind == CommandKind::DetachInstance {
                record.payload = NodePayload::Frame(FramePayload::default());
            } else {
                let movable = nodes_where(model, |id| non_root(model, id) && id != node);
                let content = pick(rng, &movable)?;
                let mut payload = current;
                payload.set_slot_override(
                    stellar_document::component::definition::ComponentSlotId::new(ids.fresh()),
                    stellar_document::component::instance::SlotOverride::new(vec![content]).ok()?,
                );
                record.payload = NodePayload::ComponentInstance(payload);
                moves.push(crate::detach::NodePlacement {
                    node: content,
                    parent: node,
                    order_key: model.next_child_order_key(node),
                });
            }
            let restructure = crate::detach::InstanceRestructure {
                node,
                record,
                layout,
                stack: None,
                insert: Vec::new(),
                insert_stacks: Vec::new(),
                insert_layouts: Vec::new(),
                moves,
                remove: Vec::new(),
            };
            Some(if kind == CommandKind::DetachInstance {
                Command::DetachInstance(Box::new(restructure))
            } else {
                Command::PlaceSlotContent(Box::new(restructure))
            })
        }
        CommandKind::SetStyleDefaults => {
            let name = if model.style_defaults() == "css" {
                "react-native"
            } else {
                "css"
            };
            Some(Command::SetStyleDefaults(
                crate::style_defaults::SetStyleDefaults { name: name.into() },
            ))
        }
        CommandKind::SetLayoutProperty => {
            use stellar_document::layout_record::{longhand_violation, LayoutLonghand};
            let boxed = nodes_where(model, |id| kind_of(model, id) != Some(NodeKind::Fragment));
            let node = pick(rng, &boxed)?;
            let payload = model.payload(node)?;
            let longhands: Vec<LayoutLonghand> = LayoutLonghand::ALL
                .into_iter()
                .filter(|l| longhand_violation(node, *l, payload).is_none())
                .collect();
            let longhand = pick(rng, &longhands)?;
            let samples: &[&str] = match longhand.grammar() {
                stellar_document::layout_record::LayoutGrammar::Keyword(choices) => choices,
                stellar_document::layout_record::LayoutGrammar::NonNegativeNumber => {
                    &["0", "1", "2.5"]
                }
                stellar_document::layout_record::LayoutGrammar::NonNegativeLength => {
                    &["0px", "8px"]
                }
                stellar_document::layout_record::LayoutGrammar::Gap => &["normal", "4px"],
                stellar_document::layout_record::LayoutGrammar::GridLine => {
                    &["auto", "2", "span 2"]
                }
                stellar_document::layout_record::LayoutGrammar::AspectRatio => &["auto", "16 / 9"],
                stellar_document::layout_record::LayoutGrammar::TrackList => &["none", "1fr 100px"],
                stellar_document::layout_record::LayoutGrammar::AutoTracks => {
                    &["auto", "minmax(0px, 1fr)"]
                }
                stellar_document::layout_record::LayoutGrammar::MaxSize => {
                    &["none", "50%", "max-content"]
                }
                stellar_document::layout_record::LayoutGrammar::FlexBasis => {
                    &["auto", "content", "10px"]
                }
                stellar_document::layout_record::LayoutGrammar::Inset => &["auto", "-4px", "10%"],
                stellar_document::layout_record::LayoutGrammar::Size => {
                    &["auto", "20px", "fit-content"]
                }
            };
            let text = samples[rng.below(samples.len() as u64) as usize];
            Some(Command::SetLayoutProperty(
                crate::style_defaults::SetLayoutProperty {
                    node,
                    longhand,
                    value: longhand.parse(text).ok()?,
                },
            ))
        }
        CommandKind::UnsetProperty => {
            let explicit = nodes_where(model, |id| model.layout_record(id).is_some());
            let node = pick(rng, &explicit)?;
            let longhands: Vec<_> = model.layout_record(node)?.iter().map(|(l, _)| l).collect();
            Some(Command::UnsetProperty(
                crate::style_defaults::UnsetProperty {
                    node,
                    longhand: pick(rng, &longhands)?,
                },
            ))
        }
        CommandKind::EditContext => crate::context_test_support::random_edit(model, rng, ids),
        CommandKind::SetNodeGuides => {
            crate::guides_test_support::random_guides_edit(model, rng, ids)
        }
        CommandKind::EditViewVariants => {
            crate::guides_test_support::random_view_variants_edit(model, rng, ids)
        }
        CommandKind::EditPathPoint => {
            let paths = nodes_where(model, |id| kind_of(model, id) == Some(NodeKind::Path));
            let node = pick(rng, &paths)?;
            let NodePayload::Path(path) = model.payload(node)? else {
                return None;
            };
            let commands = path.data().commands();
            let points = path.point_ids();
            let handles: Vec<PathHandleId> = path
                .ids()
                .iter()
                .filter_map(|ids| ids.handles)
                .flat_map(|(a, b)| [a, b])
                .collect();
            let segments: Vec<(PathPointId, bool)> = path
                .ids()
                .iter()
                .zip(commands)
                .filter_map(|(ids, command)| match command {
                    PathCommand::LineTo(_) => ids.point.map(|p| (p, false)),
                    PathCommand::CubicTo { .. } => ids.point.map(|p| (p, true)),
                    _ => None,
                })
                .collect();
            let starts: Vec<PathPointId> = path
                .ids()
                .iter()
                .zip(commands)
                .filter_map(|(ids, command)| match command {
                    PathCommand::MoveTo(_) => ids.point,
                    _ => None,
                })
                .collect();
            let to = Vec2::new(grid(rng, 400), grid(rng, 400)).ok()?;
            let edit = match rng.below(7) {
                0 => {
                    let (segment, cubic) = pick(rng, &segments)?;
                    PathEdit::InsertPoint {
                        segment,
                        t: finite(*rng.pick(&[0.25, 0.5, 0.75])),
                        point: PathPointId(ids.fresh()),
                        handles: cubic
                            .then(|| (PathHandleId(ids.fresh()), PathHandleId(ids.fresh()))),
                    }
                }
                1 => PathEdit::MovePoint {
                    point: pick(rng, &points)?,
                    to,
                },
                2 => PathEdit::DeletePoint {
                    point: pick(rng, &points)?,
                },
                3 => PathEdit::MoveHandle {
                    handle: pick(rng, &handles)?,
                    to,
                },
                4 => PathEdit::SetHandleMode {
                    point: pick(rng, &points)?,
                    mode: *rng.pick(&[
                        HandleMode::Independent,
                        HandleMode::Aligned,
                        HandleMode::Mirrored,
                    ]),
                },
                5 => PathEdit::CloseSubpath {
                    start: pick(rng, &starts)?,
                },
                _ => PathEdit::OpenSubpath {
                    start: pick(rng, &starts)?,
                },
            };
            Some(Command::EditPathPoint(EditPathPoint { node, edit }))
        }
        CommandKind::RemoveView | CommandKind::RenameView => {
            let views: Vec<StableId> = model
                .definition()
                .pages
                .iter()
                .flat_map(|p| p.views.iter().map(|v| v.id))
                .collect();
            let view = pick(rng, &views)?;
            Some(match kind {
                CommandKind::RemoveView => Command::RemoveView(RemoveView { view }),
                _ => Command::RenameView(RenameView {
                    view,
                    name: rng.pick(&["홈", "", "설정 e\u{301}"]).to_string(),
                }),
            })
        }
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
        T::Coordinate => PropertyValueType::Number,
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
