//! 비례 확대(Scale) 명령 (XII.10.12, ADR-0105 결정 4).
//!
//! 크기 조절과 달리 상자 안의 길이 값까지 같은 비율 `k`로 바꿉니다. 결과는 CommandTransaction 하나(호출자가 묶음)이며
//! 쓰는 값은 다음과 같습니다.
//!
//! - 고른 node: 크기 longhand ×k([`place::set_size`]), 위치는 부모 공간 기준점 `anchor`를 고정한 확대(선형부 그대로,
//!   이동만 [`place::translate`]).
//! - 자손(Frame 아래 전부): 크기 ×k, 부모 local 원점 기준 위치 ×k. 그래서 subtree 전체가 한 번에 확대됩니다.
//! - 길이 값: Stroke modifier `stroke-width`, StrokeExpand 폭, Corner modifier 반경, Shadow `blur`·offset 두 longhand
//!   (`SHADOW_OFFSET_X`·`_Y`, 묶음 key 0x1092 아님), Blur 반경, Frame·Rectangle `border-*-radius` 네 longhand, Polygon
//!   반경.
//!
//! 지금 command로 쓸 수 없는 길이 값은 건너뛰고 [`ScalePlan::skipped`]에 이유 코드와 함께 남깁니다(조용히 잃지 않음).
//!
//! - `border-width`: Frame 테두리는 저장 형식이 아직 묶음 값(0x0105)이고 longhand command가 없습니다(IX.11.10 남은 범위).
//!   이 명령은 묶음 key를 쓰지 않습니다.
//! - `font-size`: Text 글자 크기는 문서 Typography resource에 있고 node 하나의 `font-size` command가 없습니다(IX.11.11 남은
//!   범위). Text 위치는 확대합니다.
//! - `instance-size`: 배치한 ComponentInstance는 정의 크기를 따릅니다. 위치만 확대합니다.
//! - `path-geometry`: Path point 좌표는 확대하지 않습니다(위치만).

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Vec2};
use stellar_document::frame::keys as frame_keys;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierPayload;
use stellar_document::modifier_metadata::keys as modifier_keys;
use stellar_document::payload_metadata::shape_keys;
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;

use crate::arrange::place::{self, PlaceError};
use crate::command::Command;
use crate::modifier_ops::{read_payload, SetModifierProperty};
use crate::property::{read_property, PropertyValue, SetProperty};

/// 확대 비율 허용 범위(배). 0 이하·비유한은 거부합니다.
pub const MIN_FACTOR: f32 = 1e-3;
/// 확대 비율 상한(배).
pub const MAX_FACTOR: f32 = 1e3;

/// 확대 대상 하나: node와 그 부모 공간 기준점(보통 선택 경계의 왼쪽 위 또는 중심).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleTarget {
    /// 고른 node.
    pub node: StableId,
    /// 부모 공간 기준점.
    pub anchor: [f32; 2],
}

/// 쓰지 못하고 건너뛴 값.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// node.
    pub node: StableId,
    /// 이유 코드(`border-width`, `font-size`, `instance-size`, `path-geometry`).
    pub code: &'static str,
}

/// 확대 계획.
#[derive(Clone, Debug, PartialEq)]
pub struct ScalePlan {
    /// 한 transaction에 넣을 command.
    pub commands: Vec<Command>,
    /// 건너뛴 값.
    pub skipped: Vec<Skipped>,
}

fn scaled_scalar(value: &PropertyValue, k: f32) -> Option<PropertyValue> {
    Some(match value {
        PropertyValue::NonNegative(v) => {
            PropertyValue::NonNegative(FiniteF32::new(v.get() * k).ok()?)
        }
        PropertyValue::Number(v) => PropertyValue::Number(FiniteF32::new(v.get() * k).ok()?),
        _ => return None,
    })
}

/// node 속성 key 목록을 ×k로 바꾸는 SetProperty(바뀐 것만).
fn scale_properties(
    model: &DocumentModel,
    node: StableId,
    keys: &[PropertyKey],
    k: f32,
    out: &mut Vec<Command>,
) -> Result<(), PlaceError> {
    let record = model.record(node).ok_or(PlaceError::Missing(node))?;
    for &key in keys {
        let Some(value) = read_property(&record, key) else {
            continue;
        };
        let next = scaled_scalar(&value, k).ok_or(PlaceError::NonFinite(node))?;
        if next != value {
            out.push(Command::SetProperty(SetProperty {
                node,
                key,
                value: next,
            }));
        }
    }
    Ok(())
}

/// modifier 길이 값 ×k.
fn scale_modifiers(
    model: &DocumentModel,
    node: StableId,
    k: f32,
    out: &mut Vec<Command>,
) -> Result<(), PlaceError> {
    let Some(stack) = model.modifier_stack(node) else {
        return Ok(());
    };
    for entry in stack.entries() {
        let keys: &[PropertyKey] = match entry.payload {
            ModifierPayload::Stroke { .. } => &[modifier_keys::STROKE_WIDTH],
            ModifierPayload::StrokeExpand { .. } => &[modifier_keys::STROKE_EXPAND_WIDTH],
            ModifierPayload::Corner { .. } => &[modifier_keys::CORNER_RADIUS],
            ModifierPayload::Shadow { .. } => &[
                modifier_keys::SHADOW_BLUR,
                modifier_keys::SHADOW_OFFSET_X,
                modifier_keys::SHADOW_OFFSET_Y,
            ],
            ModifierPayload::Blur { .. } => &[modifier_keys::BLUR_RADIUS],
            _ => &[],
        };
        // 앞 key의 변경이 다음 key의 읽기에 영향이 없도록 지금 값에서 계산합니다(offset 두 축은 서로 독립).
        for &key in keys {
            let Some(value) = read_payload(&entry.payload, key) else {
                continue;
            };
            let next = scaled_scalar(&value, k).ok_or(PlaceError::NonFinite(node))?;
            if next != value {
                out.push(Command::SetModifierProperty(SetModifierProperty {
                    node,
                    modifier: entry.id,
                    key,
                    value: next,
                }));
            }
        }
    }
    Ok(())
}

/// node 하나(자손 포함)의 확대. `anchor`는 부모 공간 고정점입니다(자손은 부모 local 원점).
fn scale_node(
    model: &DocumentModel,
    node: StableId,
    anchor: [f32; 2],
    k: f32,
    plan: &mut ScalePlan,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<(), PlaceError> {
    let payload = model.payload(node).ok_or(PlaceError::Missing(node))?;
    let skip = |plan: &mut ScalePlan, code| plan.skipped.push(Skipped { node, code });
    // 위치: 선형부는 그대로, local 상자 원점의 부모 공간 위치만 anchor 기준 ×k.
    if !matches!(payload, NodePayload::Fragment(_)) {
        let transform = place::local_transform(model, node).ok_or(PlaceError::Unsupported(node))?;
        let origin = place::local_box(model, node).map_or([0.0, 0.0], |b| [b[0], b[1]]);
        let at = transform
            .apply(Vec2::new(origin[0], origin[1]).map_err(|_| PlaceError::NonFinite(node))?)
            .map_err(|_| PlaceError::NonFinite(node))?;
        let dx = (k - 1.0) * (at.x() - anchor[0]);
        let dy = (k - 1.0) * (at.y() - anchor[1]);
        if let Some(command) = place::translate(model, node, dx, dy, new_modifier)? {
            plan.commands.push(command);
        }
    }
    if let Some(local) = place::local_box(model, node) {
        plan.commands
            .extend(place::set_size(model, node, local[2] * k, local[3] * k)?);
    }
    match payload {
        NodePayload::Frame(frame) => {
            scale_properties(
                model,
                node,
                &frame_keys::BORDER_RADIUS_LONGHANDS,
                k,
                &mut plan.commands,
            )?;
            if frame.border.is_some_and(|b| b.thickness() > 0.0) {
                skip(plan, "border-width");
            }
        }
        NodePayload::Shape(ShapeGeometry::Rectangle(_)) => {
            scale_properties(
                model,
                node,
                &shape_keys::BORDER_RADIUS_LONGHANDS,
                k,
                &mut plan.commands,
            )?;
        }
        NodePayload::Shape(ShapeGeometry::Polygon(_)) => {
            scale_properties(
                model,
                node,
                &[shape_keys::POLYGON_CORNER_RADIUS],
                k,
                &mut plan.commands,
            )?;
        }
        NodePayload::Text(_) => skip(plan, "font-size"),
        NodePayload::ComponentInstance(_) => skip(plan, "instance-size"),
        NodePayload::Path(_) => skip(plan, "path-geometry"),
        _ => {}
    }
    scale_modifiers(model, node, k, &mut plan.commands)?;
    for child in model.children_in_order(node) {
        scale_node(model, child, [0.0, 0.0], k, plan, new_modifier)?;
    }
    Ok(())
}

/// `targets`를 `factor`배 확대하는 계획. 고른 node끼리 조상·자손이면 조상만 확대합니다(자손은 subtree로 함께).
pub fn scale_commands(
    model: &DocumentModel,
    targets: &[ScaleTarget],
    factor: f32,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<ScalePlan, PlaceError> {
    let first = targets.first().map(|t| t.node).unwrap_or(StableId::NIL);
    if !factor.is_finite() {
        return Err(PlaceError::NonFinite(first));
    }
    if !(MIN_FACTOR..=MAX_FACTOR).contains(&factor) {
        return Err(PlaceError::OutOfRange(first));
    }
    let mut plan = ScalePlan {
        commands: Vec::new(),
        skipped: Vec::new(),
    };
    let selected: Vec<StableId> = targets.iter().map(|t| t.node).collect();
    for target in targets {
        let mut ancestor = crate::tree::parent_of(model, target.node).flatten();
        let mut covered = false;
        while let Some(id) = ancestor {
            if selected.contains(&id) {
                covered = true;
                break;
            }
            ancestor = crate::tree::parent_of(model, id).flatten();
        }
        if !covered {
            scale_node(
                model,
                target.node,
                target.anchor,
                factor,
                &mut plan,
                new_modifier,
            )?;
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::place::local_transform;
    use crate::arrange::test_fixtures::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::frame::{Border, CornerRadius};
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind};

    fn translation(m: &DocumentModel, node: u64) -> (f32, f32) {
        let (_, _, _, _, x, y) = local_transform(m, sid(node)).unwrap().components();
        (x, y)
    }

    fn styled() -> DocumentModel {
        let mut model = shapes();
        // 11(Frame) 반경 4, 테두리 2, 자식 Rectangle 30(위치 10, 5, 선 1, 그림자 offset 2,3 blur 4).
        if let Some(NodePayload::Frame(frame)) = model.payload(sid(11)).cloned() {
            let mut frame = frame;
            frame.corner_radius = CornerRadius::new(4., 4., 4., 4.).unwrap();
            frame.border = Some(
                Border::new(Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap(), 2.).unwrap(),
            );
            model
                .set_payload(sid(11), NodePayload::Frame(frame))
                .unwrap();
        }
        model
            .insert_node(record(30, Some(11), 0x10, rectangle(20., 10.)))
            .unwrap();
        let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
        with_stack(
            &mut model,
            30,
            vec![
                position(30, 0x10, 10., 5.),
                ModifierEntry {
                    id: ModifierId::new(sid(631)),
                    order: OrderKey::from_bytes(vec![0x20]).unwrap(),
                    kind: ModifierKind::Stroke,
                    enabled: true,
                    payload: ModifierPayload::Stroke {
                        color: black,
                        width: FiniteF32::new(1.).unwrap(),
                    },
                },
                ModifierEntry {
                    id: ModifierId::new(sid(632)),
                    order: OrderKey::from_bytes(vec![0x30]).unwrap(),
                    kind: ModifierKind::Shadow,
                    enabled: true,
                    payload: ModifierPayload::Shadow {
                        color: black,
                        offset: Vec2::new(2., 3.).unwrap(),
                        blur: FiniteF32::new(4.).unwrap(),
                    },
                },
            ],
        );
        model
    }

    #[test]
    fn two_hundred_percent_scales_sizes_positions_lengths_and_modifiers_in_one_undo() {
        let model = styled();
        let mut ids = ids();
        // 11을 왼쪽 위(10, 20) 기준 200%.
        let plan = scale_commands(
            &model,
            &[ScaleTarget {
                node: sid(11),
                anchor: [10.0, 20.0],
            }],
            2.0,
            &mut ids,
        )
        .unwrap();
        assert!(plan
            .commands
            .iter()
            .all(|c| !matches!(c, Command::SetProperty(s) if s.key == frame_keys::BORDER || s.key == frame_keys::CORNER_RADIUS)));
        assert_eq!(
            plan.skipped,
            vec![Skipped {
                node: sid(11),
                code: "border-width"
            }]
        );
        let (mut state, inverse) = apply(copy(&model), plan.commands);
        let m = state.model();
        assert_eq!(translation(m, 11), (10.0, 20.0));
        let Some(NodePayload::Frame(frame)) = m.payload(sid(11)) else {
            panic!()
        };
        assert_eq!((frame.size.width(), frame.size.height()), (200.0, 100.0));
        assert_eq!(
            frame.corner_radius,
            CornerRadius::new(8., 8., 8., 8.).unwrap()
        );
        // 자식은 부모 local 원점 기준 ×2.
        assert_eq!(translation(m, 30), (20.0, 10.0));
        let Some(NodePayload::Shape(ShapeGeometry::Rectangle(rect))) = m.payload(sid(30)) else {
            panic!()
        };
        assert_eq!((rect.size.width(), rect.size.height()), (40.0, 20.0));
        let stack = m.modifier_stack(sid(30)).unwrap();
        assert!(stack.entries().iter().any(|e| matches!(
            e.payload,
            ModifierPayload::Stroke { width, .. } if width.get() == 2.0
        )));
        assert!(stack.entries().iter().any(|e| matches!(
            e.payload,
            ModifierPayload::Shadow { offset, blur, .. }
                if (offset.x(), offset.y(), blur.get()) == (4.0, 6.0, 8.0)
        )));
        undo(&mut state, &inverse);
        assert_eq!(
            state.model().to_records().unwrap(),
            model.to_records().unwrap()
        );
    }

    #[test]
    fn anchors_text_instances_and_invalid_factors() {
        let model = shapes();
        let mut ids = ids();
        // Rectangle 13(30, 40, 40×20)을 중심(50, 50) 기준 50%: 원점 (40, 45), 크기 20×10.
        let plan = scale_commands(
            &model,
            &[
                ScaleTarget {
                    node: sid(13),
                    anchor: [50.0, 50.0],
                },
                ScaleTarget {
                    node: sid(16),
                    anchor: [0.0, 0.0],
                },
            ],
            0.5,
            &mut ids,
        )
        .unwrap();
        assert_eq!(
            plan.skipped,
            vec![Skipped {
                node: sid(16),
                code: "font-size"
            }]
        );
        let (state, _) = apply(copy(&model), plan.commands);
        assert_eq!(translation(state.model(), 13), (40.0, 45.0));
        assert!(matches!(
            scale_commands(&model, &[], 0.0, &mut ids),
            Err(PlaceError::OutOfRange(_))
        ));
        assert!(matches!(
            scale_commands(&model, &[], 1001.0, &mut ids),
            Err(PlaceError::OutOfRange(_))
        ));
        assert!(matches!(
            scale_commands(&model, &[], f32::NAN, &mut ids),
            Err(PlaceError::NonFinite(_))
        ));
    }
}
