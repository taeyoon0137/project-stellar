//! node 배치 쓰기: 부모 공간 affine 변경과 크기 longhand를 기존 command로 만듭니다(ADR-0105 결정 1).
//!
//! 캔버스 변형(크기·회전·뒤집기·비례 확대·정렬·분배·키보드 이동)은 모두 "node의 local → 부모 공간 변환을 부모 공간
//! affine `change`만큼 바꾸기"와 "크기 longhand 바꾸기" 두 가지로 줄입니다. 이 module이 그 두 가지를 저장 위치에 맞는
//! command로 바꾸는 한 곳입니다.
//!
//! # 지금 저장 위치(IX.11.9 step 등록 전)
//!
//! 위치·transform 분리 migration(IX.11.9)의 step은 아직 제품 step 목록에 없고 scene은 layout 결과 위치를 그리지
//! 않습니다(ADR-0086 결정 3). 그래서 화면 위치를 실제로 바꾸는 저장 값은 다음과 같고, 이 module은 그 값만 씁니다.
//!
//! - Frame: CSS `transform`(`matrix(a, b, c, d, tx, ty)`, key 0x0102). 새 값 = `change ∘ 지금`.
//! - 배치한 ComponentInstance: 배치가 기록한 Transform modifier 하나(선형부 항등, override 없음). 이동만 됩니다.
//! - 그 밖의 node(Shape·Path·Text·Image): 위치 Transform modifier(켜진 modifier 앞부분 Transform 가운데 마지막 것,
//!   [`crate::inspector::position_modifier`], 속성 탭 `left`·`top`과 같은 규칙). 새 값 = `change ∘ 지금 위치 modifier`.
//!   앞부분 Transform이 여럿이어도 합성 결과(`위치 ∘ 나머지`)가 정확히 `change ∘ 지금 합성`이 됩니다. 없으면 맨 앞에
//!   새 Transform modifier를 넣습니다.
//!
//! IX.11.9 step이 등록되면(위치는 layout record `left`·`top` 또는 `translate`, 선형부는 `rotate`·`scale`·`transform`)
//! 이 module만 바꿉니다. 캔버스 변형의 계산은 저장 위치를 모릅니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Transform2D};
use stellar_document::frame::keys as frame_keys;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;
use stellar_document::payload_metadata::{image_keys, shape_keys};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;

use crate::command::Command;
use crate::lock::{edit_permission, EditIntent};
use crate::modifier_ops::{InsertModifier, SetModifierProperty};
use crate::property::{read_property, PropertyValue, SetProperty};

/// 배치 쓰기를 만들 수 없는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceError {
    /// node가 없습니다.
    Missing(StableId),
    /// 자신·조상 잠금이나 흐름 부모가 막습니다(D8, [`crate::lock`]).
    Blocked(StableId),
    /// 이 node 종류·modifier 구성으로는 표현할 수 없는 변경입니다(예: 인스턴스 회전, Text 크기).
    Unsupported(StableId),
    /// 결과가 유한하지 않거나 크기가 음수입니다.
    NonFinite(StableId),
}

impl PlaceError {
    /// 진단 코드.
    pub fn code(self) -> &'static str {
        match self {
            PlaceError::Missing(_) => "missing-node",
            PlaceError::Blocked(_) => "blocked",
            PlaceError::Unsupported(_) => "unsupported",
            PlaceError::NonFinite(_) => "non-finite",
        }
    }

    /// 대상 node.
    pub fn node(self) -> StableId {
        match self {
            PlaceError::Missing(id)
            | PlaceError::Blocked(id)
            | PlaceError::Unsupported(id)
            | PlaceError::NonFinite(id) => id,
        }
    }
}

/// x, y, width, height.
pub type Box4 = [f32; 4];

/// 부모 공간 평행 이동 `(dx, dy)`.
pub fn translation(dx: f32, dy: f32) -> Option<Transform2D> {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, dx, dy).ok()
}

/// node의 local → 부모 공간 변환(위 "지금 저장 위치"의 합성 값). Fragment는 상자가 없어 `None`입니다.
pub fn local_transform(model: &DocumentModel, node: StableId) -> Option<Transform2D> {
    match model.payload(node)? {
        NodePayload::Frame(frame) => Some(frame.transform),
        NodePayload::Fragment(_) => None,
        _ => {
            let mut total = Transform2D::identity();
            if let Some(stack) = model.modifier_stack(node) {
                for entry in stack.active_entries() {
                    let ModifierPayload::Transform(next) = entry.payload else {
                        break;
                    };
                    total = total.then(&next).ok()?;
                }
            }
            Some(total)
        }
    }
}

/// node 자신의 local 좌표 상자(문서 값만으로 정해지는 종류). Text·Path·인스턴스·Fragment는 `None`입니다.
///
/// Frame·Rectangle·Image는 원점 0, Ellipse·Polygon은 저장된 bounds, Line은 두 끝점을 감싸는 상자입니다.
pub fn local_box(model: &DocumentModel, node: StableId) -> Option<Box4> {
    let rect = |x: f32, y: f32, w: f32, h: f32| [x, y, w, h];
    Some(match model.payload(node)? {
        NodePayload::Frame(frame) => rect(0.0, 0.0, frame.size.width(), frame.size.height()),
        NodePayload::Image(image) => rect(0.0, 0.0, image.size.width(), image.size.height()),
        NodePayload::Shape(shape) => match shape {
            ShapeGeometry::Rectangle(r) => rect(0.0, 0.0, r.size.width(), r.size.height()),
            ShapeGeometry::Ellipse(e) => rect(
                e.bounds.origin().x(),
                e.bounds.origin().y(),
                e.bounds.size().width(),
                e.bounds.size().height(),
            ),
            ShapeGeometry::Polygon(p) => {
                let b = p.bounds();
                rect(
                    b.origin().x(),
                    b.origin().y(),
                    b.size().width(),
                    b.size().height(),
                )
            }
            ShapeGeometry::Line(l) => rect(
                l.start.x().min(l.end.x()),
                l.start.y().min(l.end.y()),
                (l.end.x() - l.start.x()).abs(),
                (l.end.y() - l.start.y()).abs(),
            ),
        },
        _ => return None,
    })
}

/// 크기 longhand를 쓸 수 있는 종류인지(Frame·Rectangle·Ellipse·Polygon·Line·Image).
pub fn has_size(model: &DocumentModel, node: StableId) -> bool {
    local_box(model, node).is_some()
}

fn set_property(node: StableId, key: PropertyKey, value: PropertyValue) -> Command {
    Command::SetProperty(SetProperty { node, key, value })
}

/// 값이 바뀐 `(key, value)`만 `SetProperty`로 만듭니다(속성 탭과 같은 key).
pub fn changed_properties(
    model: &DocumentModel,
    node: StableId,
    values: impl IntoIterator<Item = (PropertyKey, PropertyValue)>,
) -> Result<Vec<Command>, PlaceError> {
    let record = model.record(node).ok_or(PlaceError::Missing(node))?;
    Ok(values
        .into_iter()
        .filter(|(key, value)| read_property(&record, *key).as_ref() != Some(value))
        .map(|(key, value)| set_property(node, key, value))
        .collect())
}

fn non_negative(node: StableId, value: f32) -> Result<PropertyValue, PlaceError> {
    FiniteF32::new(value)
        .ok()
        .filter(|v| v.get() >= 0.0)
        .map(PropertyValue::NonNegative)
        .ok_or(PlaceError::NonFinite(node))
}

fn number(node: StableId, value: f32) -> Result<PropertyValue, PlaceError> {
    FiniteF32::new(value)
        .map(PropertyValue::Number)
        .map_err(|_| PlaceError::NonFinite(node))
}

/// node 크기를 `width × height`(local px, ≥ 0)로 바꾸는 command(바뀐 longhand만). local 상자 원점은 그대로입니다.
///
/// Frame·Rectangle·Ellipse·Polygon·Image는 `width`·`height`, Line은 상자 원점(두 끝점의 최솟값)을 고정하고 끝점을
/// 같은 비율로 옮긴 `x1`·`y1`·`x2`·`y2`입니다(길이 0인 축은 그대로). Text·Path·인스턴스는 저장 크기가 없어
/// [`PlaceError::Unsupported`]입니다.
pub fn set_size(
    model: &DocumentModel,
    node: StableId,
    width: f32,
    height: f32,
) -> Result<Vec<Command>, PlaceError> {
    let payload = model.payload(node).ok_or(PlaceError::Missing(node))?;
    let sized = |w: PropertyKey, h: PropertyKey| -> Result<Vec<(PropertyKey, PropertyValue)>, _> {
        Ok(vec![
            (w, non_negative(node, width)?),
            (h, non_negative(node, height)?),
        ])
    };
    let values = match payload {
        NodePayload::Frame(_) => sized(frame_keys::WIDTH, frame_keys::HEIGHT)?,
        NodePayload::Image(_) => sized(image_keys::WIDTH, image_keys::HEIGHT)?,
        NodePayload::Shape(ShapeGeometry::Line(line)) => {
            non_negative(node, width)?;
            non_negative(node, height)?;
            let min_x = line.start.x().min(line.end.x());
            let min_y = line.start.y().min(line.end.y());
            let old_w = (line.end.x() - line.start.x()).abs();
            let old_h = (line.end.y() - line.start.y()).abs();
            let sx = if old_w > 0.0 { width / old_w } else { 1.0 };
            let sy = if old_h > 0.0 { height / old_h } else { 1.0 };
            let x = |v: f32| number(node, min_x + (v - min_x) * sx);
            let y = |v: f32| number(node, min_y + (v - min_y) * sy);
            vec![
                (shape_keys::X1, x(line.start.x())?),
                (shape_keys::Y1, y(line.start.y())?),
                (shape_keys::X2, x(line.end.x())?),
                (shape_keys::Y2, y(line.end.y())?),
            ]
        }
        NodePayload::Shape(_) => sized(shape_keys::WIDTH, shape_keys::HEIGHT)?,
        _ => return Err(PlaceError::Unsupported(node)),
    };
    edit_permission(model, node, EditIntent::Edit).map_err(|_| PlaceError::Blocked(node))?;
    changed_properties(model, node, values)
}

/// 선형부가 항등인지.
fn is_translation(change: &Transform2D) -> bool {
    let (a, b, c, d, _, _) = change.components();
    (a, b, c, d) == (1.0, 0.0, 0.0, 1.0)
}

/// node의 local → 부모 공간 변환을 `change ∘ 지금`으로 바꾸는 command. 바뀌지 않으면 `None`입니다.
///
/// `new_modifier`는 위치 Transform modifier를 새로 넣을 때만 부릅니다(제품 경로는 UUIDv7). 이동 판정은
/// [`EditIntent::Move`](잠금·흐름 부모)입니다. 배치한 인스턴스는 평행 이동만 받습니다.
pub fn apply_affine(
    model: &DocumentModel,
    node: StableId,
    change: Transform2D,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Option<Command>, PlaceError> {
    let payload = model.payload(node).ok_or(PlaceError::Missing(node))?;
    edit_permission(model, node, EditIntent::Move).map_err(|_| PlaceError::Blocked(node))?;
    let compose = |current: Transform2D| {
        current
            .then(&change)
            .map_err(|_| PlaceError::NonFinite(node))
    };
    match payload {
        NodePayload::Frame(frame) => {
            let next = compose(frame.transform)?;
            Ok((next != frame.transform)
                .then(|| set_property(node, frame_keys::TRANSFORM, PropertyValue::Transform(next))))
        }
        NodePayload::Fragment(_) => Err(PlaceError::Unsupported(node)),
        NodePayload::ComponentInstance(instance) => {
            // 배치가 기록한 이동 하나만 바꿉니다(캔버스 이동과 같은 판정). 별도 변환·override가 있으면 거부합니다.
            let stack = model
                .modifier_stack(node)
                .ok_or(PlaceError::Unsupported(node))?;
            let [entry] = stack.entries() else {
                return Err(PlaceError::Unsupported(node));
            };
            let ModifierPayload::Transform(matrix) = entry.payload else {
                return Err(PlaceError::Unsupported(node));
            };
            if !instance.has_no_overrides()
                || !entry.enabled
                || !is_translation(&matrix)
                || !is_translation(&change)
            {
                return Err(PlaceError::Unsupported(node));
            }
            let next = compose(matrix)?;
            Ok((next != matrix).then_some({
                Command::SetModifierProperty(SetModifierProperty {
                    node,
                    modifier: entry.id,
                    key: TRANSFORM_MATRIX,
                    value: PropertyValue::Transform(next),
                })
            }))
        }
        NodePayload::Shape(_)
        | NodePayload::Path(_)
        | NodePayload::Text(_)
        | NodePayload::Image(_) => match crate::inspector::position_modifier(model, node) {
            Some(entry) => {
                let ModifierPayload::Transform(current) = entry.payload else {
                    return Err(PlaceError::Unsupported(node));
                };
                let next = compose(current)?;
                Ok((next != current).then_some({
                    Command::SetModifierProperty(SetModifierProperty {
                        node,
                        modifier: entry.id,
                        key: TRANSFORM_MATRIX,
                        value: PropertyValue::Transform(next),
                    })
                }))
            }
            None => {
                if change == Transform2D::identity() {
                    return Ok(None);
                }
                let order = OrderKey::between(
                    None,
                    model
                        .modifier_stack(node)
                        .and_then(|s| s.entries().first())
                        .map(|e| &e.order),
                );
                Ok(Some(Command::InsertModifier(InsertModifier {
                    node,
                    entry: ModifierEntry {
                        id: ModifierId::new(new_modifier()),
                        order,
                        kind: ModifierKind::Transform,
                        enabled: true,
                        payload: ModifierPayload::Transform(change),
                    },
                })))
            }
        },
    }
}

/// 부모 공간 `(dx, dy)` 평행 이동([`apply_affine`]의 이동 판).
pub fn translate(
    model: &DocumentModel,
    node: StableId,
    dx: f32,
    dy: f32,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Option<Command>, PlaceError> {
    let change = translation(dx, dy).ok_or(PlaceError::NonFinite(node))?;
    apply_affine(model, node, change, new_modifier)
}

/// 선형부를 바꾸는 변경(회전·뒤집기)을 받을 수 있는지. 인스턴스·Fragment는 받지 못합니다.
pub fn accepts_linear(model: &DocumentModel, node: StableId) -> bool {
    matches!(
        model.payload(node),
        Some(
            NodePayload::Frame(_)
                | NodePayload::Shape(_)
                | NodePayload::Path(_)
                | NodePayload::Text(_)
                | NodePayload::Image(_)
        )
    )
}

/// `rotate(<degrees>)`의 선형부 `[cos, sin, -sin, cos]`(y 아래 방향 좌표에서 시계 방향 양수).
///
/// 속성 모델 15장 O13 규칙(IX.11.9, `stellar_layout::transform::rotate_matrix`)과 같습니다: 직각 배수(0·90·180·270도와
/// ±360k)는 sin·cos를 정확한 0·±1로 두고, 그 밖은 f64 `sin_cos`를 f32로 반올림합니다. crate 그래프가 commands →
/// layout 의존을 허용하지 않아 같은 규칙을 여기 둡니다(두 함수의 결과가 같은지 IX.11.9 step 등록 때 함께 맞춤).
pub fn rotation(degrees: f32) -> Option<[f32; 4]> {
    if !degrees.is_finite() {
        return None;
    }
    let turns = degrees.rem_euclid(360.0);
    let (sin, cos) = if turns == 0.0 {
        (0.0, 1.0)
    } else if turns == 90.0 {
        (1.0, 0.0)
    } else if turns == 180.0 {
        (0.0, -1.0)
    } else if turns == 270.0 {
        (-1.0, 0.0)
    } else {
        let (s, c) = f64::from(degrees).to_radians().sin_cos();
        (s as f32, c as f32)
    };
    Some([cos, sin, -sin, cos])
}

/// 부모 공간 점 `(x, y)`를 고정한 회전·뒤집기·확대 행렬 `pivot ∘ linear ∘ pivot⁻¹`.
pub fn about(linear: [f32; 4], x: f32, y: f32) -> Option<Transform2D> {
    let [a, b, c, d] = linear;
    Transform2D::new(a, b, c, d, x - (a * x + c * y), y - (b * x + d * y)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::test_fixtures::*;
    use stellar_core_types::value::Vec2;

    #[test]
    fn frame_transform_composes_the_change_after_the_stored_matrix() {
        let model = shapes();
        let rotate = about([0.0, 1.0, -1.0, 0.0], 0.0, 0.0).unwrap();
        let mut ids = ids();
        let command = apply_affine(&model, sid(11), rotate, &mut ids)
            .unwrap()
            .unwrap();
        let Command::SetProperty(set) = command else {
            panic!("Frame은 transform SetProperty");
        };
        assert_eq!(set.key, frame_keys::TRANSFORM);
        // 지금 translate(10, 20)을 90° 회전: (10, 20) → (-20, 10).
        assert_eq!(
            set.value,
            PropertyValue::Transform(Transform2D::new(0.0, 1.0, -1.0, 0.0, -20.0, 10.0).unwrap())
        );
    }

    #[test]
    fn shapes_change_the_position_modifier_or_insert_one_in_front() {
        let model = shapes();
        let mut ids = ids();
        // 13은 위치 modifier가 있습니다.
        let command = translate(&model, sid(13), 5.0, 0.0, &mut ids)
            .unwrap()
            .unwrap();
        assert!(matches!(command, Command::SetModifierProperty(_)));
        // 14는 modifier가 없어 맨 앞에 새로 넣습니다.
        let command = translate(&model, sid(14), 5.0, 0.0, &mut ids)
            .unwrap()
            .unwrap();
        let Command::InsertModifier(insert) = command else {
            panic!("새 위치 modifier");
        };
        assert_eq!(
            insert.entry.payload,
            ModifierPayload::Transform(translation(5.0, 0.0).unwrap())
        );
        assert!(translate(&model, sid(14), 0.0, 0.0, &mut ids)
            .unwrap()
            .is_none());
    }

    #[test]
    fn size_writes_changed_longhands_and_rejects_unsized_kinds() {
        let model = shapes();
        let commands = set_size(&model, sid(13), 40.0, 30.0).unwrap();
        // Rectangle 40×30 → 너비는 그대로, 높이만 바뀜.
        assert_eq!(commands.len(), 1);
        let Command::SetProperty(set) = &commands[0] else {
            panic!()
        };
        assert_eq!(set.key, shape_keys::HEIGHT);
        assert!(matches!(
            set_size(&model, sid(16), 1.0, 1.0),
            Err(PlaceError::Unsupported(_))
        ));
        // Line은 최솟값 꼭짓점을 고정하고 끝점을 같은 비율로 옮깁니다.
        let commands = set_size(&model, sid(15), 20.0, 0.0).unwrap();
        assert_eq!(commands.len(), 1, "{commands:?}");
        let local = local_box(&model, sid(15)).unwrap();
        assert_eq!(local, [0.0, 0.0, 10.0, 0.0]);
        let _ = Vec2::ZERO;
    }

    #[test]
    fn right_angle_rotations_are_exact() {
        assert_eq!(rotation(90.0), Some([0.0, 1.0, -1.0, 0.0]));
        assert_eq!(rotation(-90.0), Some([0.0, -1.0, 1.0, 0.0]));
        assert_eq!(rotation(450.0), Some([0.0, 1.0, -1.0, 0.0]));
        assert_eq!(rotation(180.0), Some([-1.0, 0.0, -0.0, -1.0]));
        assert!(rotation(f32::NAN).is_none());
        let [c, s, _, _] = rotation(30.0).unwrap();
        assert!((c - 0.866_025_4).abs() < 1e-6 && (s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn locked_nodes_and_instance_rotation_are_rejected() {
        let mut model = shapes();
        model.replace_flags(sid(13), true, true).unwrap();
        let mut ids = ids();
        assert!(matches!(
            translate(&model, sid(13), 1.0, 0.0, &mut ids),
            Err(PlaceError::Blocked(_))
        ));
        assert!(!accepts_linear(&model, sid(12)));
    }
}
