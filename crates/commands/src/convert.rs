//! node 종류 변환 command (VII.2.7).
//!
//! Phase VI의 **순수 변환 계획**을 문서에 적용합니다. 두 변환 모두 StableId를 유지하므로 외부
//! 참조가 그대로 유효합니다.
//!
//! - [`ConvertFragmentToFrame`]: [`plan_fragment_to_frame`](VI.4.4)이 만든 Frame payload(가시 자식
//!   bounds 크기·위치, clip 없음)로 Fragment를 Frame으로 바꿉니다. 새 Frame이 좌표계를 만들므로
//!   자식(중첩 Fragment를 거친 자손 포함)의 local 위치에 `child_offset`을 더해 시각 결과를
//!   유지합니다. 위치 필드가 없어 옮길 수 없는 종류(Rectangle·Text·Image)가 있는데 offset이 0이
//!   아니면 시각 결과를 보존할 수 없으므로 거부합니다.
//! - [`ConvertShapeToPath`]: [`plan_shape_to_path`](VI.6.5)로 Shape를 같은 렌더 geometry의 Path로
//!   바꿉니다. 새 point·handle id는 caller가 제공합니다(재실행 결정론).
//! - [`RestoreNodeRecords`]: 두 변환의 역 command입니다. 지정 node들의 레코드(종류 포함)를 통째로
//!   되돌립니다. 부모·순서는 바꾸지 않습니다.
//!
//! leaf bounds(Fragment 변환 계획 입력)는 layout 이전 단계라 payload에서 계산합니다: Frame은
//! transform을 적용한 크기 상자, Shape·Path는 path geometry bounds, Image는 배치 상자, Text는 layout
//! 전까지 bounds 없음(합집합에서 제외).

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Angle, Rect, Size2, Transform2D, Vec2};
use stellar_document::fragment::plan_fragment_to_frame;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::validate_modifier_owner;
use stellar_document::node_kind::NodeKind;
use stellar_document::path::{
    path_bounds, plan_shape_to_path, IdentifiedPath, PathCommand, PathData,
};
use stellar_document::record_codec::{decode_node_record, encode_node_record};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    evaluate_shape_path, EllipseParams, LineParams, PolygonParams, ShapeGeometry,
};
use stellar_document::validate::can_have_children;

use crate::change::{node_field_invalidation, node_keys};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::{id_owners, path_element_ids, reference_index, ChildIndex, IdOwner};

fn kind_error(node: StableId, expected: NodeKind, found: NodeKind) -> CommandError {
    CommandError::ConversionKind {
        node,
        expected,
        found,
    }
}

/// payload의 containing Frame 좌표계 bounds(layout 이전 근사). 없으면 `None`.
fn leaf_bounds(payload: &NodePayload) -> Option<Rect> {
    match payload {
        NodePayload::Frame(frame) => {
            let (w, h) = (frame.size.width(), frame.size.height());
            let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
            let mut min = (f32::INFINITY, f32::INFINITY);
            let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            for (x, y) in corners {
                let point = frame.transform.apply(Vec2::new(x, y).ok()?).ok()?;
                min = (min.0.min(point.x()), min.1.min(point.y()));
                max = (max.0.max(point.x()), max.1.max(point.y()));
            }
            Some(Rect::new(
                Vec2::new(min.0, min.1).ok()?,
                Size2::new(max.0 - min.0, max.1 - min.1).ok()?,
            ))
        }
        NodePayload::Shape(geometry) => path_bounds(&evaluate_shape_path(geometry)),
        NodePayload::Path(path) => path_bounds(path.data().commands()),
        NodePayload::Image(image) => Some(image.layout_bounds()),
        NodePayload::Text(_) | NodePayload::Fragment(_) => None,
    }
}

fn shift(point: Vec2, offset: Vec2) -> Option<Vec2> {
    Vec2::new(point.x() + offset.x(), point.y() + offset.y()).ok()
}

fn shift_rect(rect: Rect, offset: Vec2) -> Option<Rect> {
    Some(Rect::new(shift(rect.origin(), offset)?, rect.size()))
}

/// local 위치를 `offset`만큼 옮긴 payload. 위치 필드가 없는 종류는 `Err(())`, 비유한 결과는
/// `Ok(None)`입니다.
fn translate(payload: &NodePayload, offset: Vec2) -> Result<Option<NodePayload>, ()> {
    Ok(match payload {
        NodePayload::Frame(frame) => {
            let (a, b, c, d, tx, ty) = frame.transform.components();
            Transform2D::new(a, b, c, d, tx + offset.x(), ty + offset.y())
                .ok()
                .map(|transform| {
                    let mut frame = frame.clone();
                    frame.transform = transform;
                    NodePayload::Frame(frame)
                })
        }
        NodePayload::Shape(geometry) => match geometry {
            ShapeGeometry::Rectangle(_) => return Err(()),
            ShapeGeometry::Ellipse(params) => shift_rect(params.bounds, offset)
                .map(|bounds| NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams { bounds }))),
            ShapeGeometry::Line(params) => {
                match (shift(params.start, offset), shift(params.end, offset)) {
                    (Some(start), Some(end)) => {
                        Some(NodePayload::Shape(ShapeGeometry::Line(LineParams {
                            start,
                            end,
                            cap: params.cap,
                        })))
                    }
                    _ => None,
                }
            }
            ShapeGeometry::Polygon(params) => {
                shift_rect(params.bounds(), offset).and_then(|bounds| {
                    let rotation = Angle::new(params.rotation_degrees()).ok()?;
                    PolygonParams::new(bounds, params.sides(), rotation, params.corner_radius())
                        .ok()
                        .map(|p| NodePayload::Shape(ShapeGeometry::Polygon(p)))
                })
            }
        },
        NodePayload::Path(path) => {
            let mut commands = Vec::with_capacity(path.data().commands().len());
            for command in path.data().commands() {
                let moved = match *command {
                    PathCommand::MoveTo(p) => shift(p, offset).map(PathCommand::MoveTo),
                    PathCommand::LineTo(p) => shift(p, offset).map(PathCommand::LineTo),
                    PathCommand::CubicTo { c1, c2, to } => {
                        match (shift(c1, offset), shift(c2, offset), shift(to, offset)) {
                            (Some(c1), Some(c2), Some(to)) => {
                                Some(PathCommand::CubicTo { c1, c2, to })
                            }
                            _ => None,
                        }
                    }
                    PathCommand::Close => Some(PathCommand::Close),
                };
                match moved {
                    Some(command) => commands.push(command),
                    None => return Ok(None),
                }
            }
            PathData::new(path.data().winding(), commands)
                .ok()
                .and_then(|data| IdentifiedPath::new(data, path.ids().to_vec()).ok())
                .map(NodePayload::Path)
        }
        NodePayload::Fragment(_) | NodePayload::Text(_) | NodePayload::Image(_) => return Err(()),
    })
}

/// Fragment → Frame 변환 결과 레코드(Fragment 자신 + 위치가 바뀌는 자손).
fn fragment_conversion(
    model: &DocumentModel,
    node: StableId,
) -> Result<Vec<NodeRecord>, CommandError> {
    let handle = model
        .runtime_id(node)
        .ok_or(CommandError::UnknownNode { id: node })?;
    let mut record = model
        .record(node)
        .ok_or(CommandError::UnknownNode { id: node })?;
    if record.kind() != NodeKind::Fragment {
        return Err(kind_error(node, NodeKind::Fragment, record.kind()));
    }
    let bounds_of = |child| {
        model
            .stable_id(child)
            .and_then(|id| model.payload(id))
            .and_then(leaf_bounds)
    };
    let plan = plan_fragment_to_frame(model.columns(), handle, &bounds_of)
        .map_err(|_| CommandError::UnknownNode { id: node })?;
    record.payload = NodePayload::Frame(plan.frame_payload);
    let mut out = vec![record];
    if plan.child_offset == Vec2::ZERO {
        return Ok(out);
    }
    // 자식과(중첩 Fragment를 거친) 자손을 옮깁니다. Fragment 자신은 좌표계가 없어 그대로 둡니다.
    let index = ChildIndex::build(model);
    let mut stack: Vec<StableId> = plan.children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let mut child = model.record(id).ok_or(CommandError::UnknownNode { id })?;
        if child.kind() == NodeKind::Fragment {
            stack.extend(index.children(id).iter().rev().copied());
            continue;
        }
        // 위치 필드가 없거나(Err) 옮긴 값이 유한하지 않으면(None) 시각 결과를 보존할 수 없습니다.
        let Ok(Some(payload)) = translate(&child.payload, plan.child_offset) else {
            return Err(CommandError::ConversionNotRepresentable { node, child: id });
        };
        child.payload = payload;
        out.push(child);
    }
    Ok(out)
}

/// 레코드들을 적용하고 변경을 기록합니다(종류 변환·복원 공용).
fn apply_records(ctx: &mut EditContext<'_>, records: &[NodeRecord]) -> Result<(), CommandError> {
    for record in records {
        let previous = ctx.replace_record(record.clone())?;
        let id = record.id;
        if previous.kind() != record.kind() || previous.payload != record.payload {
            ctx.changes().structure(id);
        }
        for (key, changed) in [
            (node_keys::VISIBLE, previous.visible != record.visible),
            (node_keys::LOCKED, previous.locked != record.locked),
            (
                node_keys::DISPLAY_NAME,
                previous.display_name != record.display_name,
            ),
        ] {
            if changed {
                let invalidation = node_field_invalidation(key).unwrap_or_default();
                ctx.changes().node_property(id, key, invalidation);
            }
        }
    }
    Ok(())
}

fn encode_records(out: &mut Writer, records: &[NodeRecord]) {
    out.len(records.len());
    for record in records {
        out.bytes(&encode_node_record(record));
    }
}

fn decode_records(input: &mut Reader<'_>) -> Result<Vec<NodeRecord>, CommandCodecError> {
    let count = input.len()?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(decode_node_record(input.bytes()?).map_err(|_| {
            CommandCodecError::Malformed {
                reason: "잘못된 node 레코드",
            }
        })?);
    }
    Ok(records)
}

/// Fragment → Frame 변환.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertFragmentToFrame {
    /// 대상 Fragment.
    pub node: StableId,
}

impl ConvertFragmentToFrame {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        fragment_conversion(model, self.node).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        let changed = fragment_conversion(model, self.node)?;
        let mut records = Vec::with_capacity(changed.len());
        for record in changed {
            records.push(
                model
                    .record(record.id)
                    .ok_or(CommandError::UnknownNode { id: record.id })?,
            );
        }
        Ok(RestoreNodeRecords { records })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let records = fragment_conversion(ctx.model(), self.node)?;
        apply_records(ctx, &records)
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<ConvertFragmentToFrame, CommandCodecError> {
        Ok(ConvertFragmentToFrame { node: input.id()? })
    }
}

/// Shape → Path 파괴 변환.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertShapeToPath {
    /// 대상 Shape.
    pub node: StableId,
    /// 새 path point·handle id(command 순서대로 소비, 개수는 [`ConvertShapeToPath::required_ids`]).
    pub element_ids: Vec<StableId>,
}

impl ConvertShapeToPath {
    /// geometry를 path로 바꿀 때 필요한 point·handle id 수.
    pub fn required_ids(geometry: &ShapeGeometry) -> usize {
        evaluate_shape_path(geometry)
            .iter()
            .map(|command| match command {
                PathCommand::MoveTo(_) | PathCommand::LineTo(_) => 1,
                PathCommand::CubicTo { .. } => 3,
                PathCommand::Close => 0,
            })
            .sum()
    }

    fn converted(&self, model: &DocumentModel) -> Result<NodeRecord, CommandError> {
        let mut record = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let NodePayload::Shape(geometry) = &record.payload else {
            return Err(kind_error(self.node, NodeKind::Shape, record.kind()));
        };
        if self.element_ids.len() != ConvertShapeToPath::required_ids(geometry) {
            return Err(CommandError::InvalidRequest {
                reason: "path point·handle id 수가 변환 결과와 다릅니다",
            });
        }
        let owners = id_owners(model);
        let mut seen = BTreeSet::new();
        for id in &self.element_ids {
            if *id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if owners.contains_key(id) || !seen.insert(*id) {
                return Err(CommandError::DuplicateId { id: *id });
            }
        }
        let mut ids = self.element_ids.iter().copied();
        let plan = plan_shape_to_path(self.node, geometry, &mut || {
            ids.next().unwrap_or(StableId::NIL)
        })
        .map_err(|_| CommandError::InvalidRequest {
            reason: "Shape geometry를 path로 만들 수 없습니다",
        })?;
        record.payload = NodePayload::Path(plan.path);
        Ok(record)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.converted(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        self.converted(model)?;
        Ok(RestoreNodeRecords {
            records: vec![model
                .record(self.node)
                .ok_or(CommandError::UnknownNode { id: self.node })?],
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let record = self.converted(ctx.model())?;
        apply_records(ctx, &[record])
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.len(self.element_ids.len());
        for id in &self.element_ids {
            out.id(*id);
        }
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<ConvertShapeToPath, CommandCodecError> {
        let node = input.id()?;
        let count = input.len()?;
        let mut element_ids = Vec::with_capacity(count);
        for _ in 0..count {
            element_ids.push(input.id()?);
        }
        Ok(ConvertShapeToPath { node, element_ids })
    }
}

/// 지정 node 레코드(종류 포함)를 통째로 되돌립니다(변환 command의 역).
///
/// 각 레코드의 node가 있어야 하고 부모·OrderKey는 현재와 같아야 합니다. 바뀐 종류가 자식·modifier
/// stack·들어오는 clip 참조와 맞는지, 새 payload의 clip 대상과 path element id가 유효한지 검사합니다.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreNodeRecords {
    /// 되돌릴 레코드(같은 id의 기존 node를 대체).
    pub records: Vec<NodeRecord>,
}

impl RestoreNodeRecords {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        if self.records.is_empty() {
            return Err(CommandError::InvalidRequest {
                reason: "되돌릴 레코드가 없습니다",
            });
        }
        let mut owners = id_owners(model);
        let replaced: BTreeSet<StableId> = self.records.iter().map(|r| r.id).collect();
        if replaced.len() != self.records.len() {
            return Err(CommandError::InvalidRequest {
                reason: "같은 node 레코드가 둘 이상입니다",
            });
        }
        // 교체되는 node의 기존 path element id는 해제됩니다.
        for id in &replaced {
            if let Some(payload) = model.payload(*id) {
                for element in path_element_ids(payload) {
                    owners.remove(&element);
                }
            }
        }
        let new_kinds: BTreeMap<StableId, NodeKind> =
            self.records.iter().map(|r| (r.id, r.kind())).collect();
        let kind_after = |id: StableId| -> Option<IdOwner> {
            new_kinds
                .get(&id)
                .map(|kind| IdOwner::Node(*kind))
                .or_else(|| owners.get(&id).copied())
        };
        let index = ChildIndex::build(model);
        let references = reference_index(model);
        let mut claimed = BTreeSet::new();
        for record in &self.records {
            let current = model
                .record(record.id)
                .ok_or(CommandError::UnknownNode { id: record.id })?;
            if current.parent != record.parent || current.order_key != record.order_key {
                return Err(CommandError::InvalidRequest {
                    reason: "레코드 복원은 부모·순서를 바꿀 수 없습니다",
                });
            }
            let kind = record.kind();
            if !index.children(record.id).is_empty() && !can_have_children(kind) {
                return Err(CommandError::InvalidParent {
                    node: index.children(record.id)[0],
                    parent: Some(record.id),
                });
            }
            if let Some(stack) = model.modifier_stack(record.id) {
                for entry in stack.entries() {
                    validate_modifier_owner(kind, entry.kind).map_err(|error| {
                        CommandError::Modifier {
                            owner: record.id,
                            error,
                        }
                    })?;
                }
            }
            if kind != NodeKind::Path {
                // 다른 Frame의 clip 대상이면 Path여야 합니다.
                for from in references.inbound(record.id) {
                    let clips_here = match model.payload(from) {
                        Some(NodePayload::Frame(frame)) => matches!(
                            frame.clipping,
                            stellar_document::frame::Clipping::Path(t) if t.id() == record.id
                        ),
                        _ => false,
                    };
                    if clips_here {
                        return Err(CommandError::ReferenceTypeMismatch {
                            owner: from,
                            target: record.id,
                        });
                    }
                }
            }
            if let NodePayload::Frame(frame) = &record.payload {
                if let stellar_document::frame::Clipping::Path(target) = frame.clipping {
                    match kind_after(target.id()) {
                        None | Some(IdOwner::Node(NodeKind::Path)) => {}
                        Some(_) => {
                            return Err(CommandError::ReferenceTypeMismatch {
                                owner: record.id,
                                target: target.id(),
                            })
                        }
                    }
                }
            }
            for element in path_element_ids(&record.payload) {
                if element == StableId::NIL {
                    return Err(CommandError::NilId);
                }
                if owners.contains_key(&element) || !claimed.insert(element) {
                    return Err(CommandError::DuplicateId { id: element });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        let mut records = Vec::with_capacity(self.records.len());
        for record in &self.records {
            records.push(
                model
                    .record(record.id)
                    .ok_or(CommandError::UnknownNode { id: record.id })?,
            );
        }
        Ok(RestoreNodeRecords { records })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        apply_records(ctx, &self.records)
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        encode_records(out, &self.records);
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<RestoreNodeRecords, CommandCodecError> {
        Ok(RestoreNodeRecords {
            records: decode_records(input)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::node_ops::InsertNode;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::frame::{Clipping, CornerRadius, FramePayload};
    use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};
    use stellar_document::shape::RectangleParams;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_document::text::TextPayload;

    fn snapshot(state: &DocumentState) -> String {
        model_snapshot_json(state.model()).expect("snapshot")
    }

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    fn record(id: u64, parent: u64, key: u8, payload: NodePayload) -> NodeRecord {
        let mut record = frame(id, Some(parent), key);
        record.payload = payload;
        record
    }

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    fn placed_frame(x: f32, y: f32, w: f32, h: f32) -> NodePayload {
        NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
            ..FramePayload::default()
        })
    }

    fn ellipse(x: f32, y: f32, w: f32, h: f32) -> NodePayload {
        NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
            bounds: Rect::new(v(x, y), Size2::new(w, h).unwrap()),
        }))
    }

    /// 11 아래 Fragment(20): Frame(21, (10,20) 30×10), 중첩 Fragment(22) 안 Ellipse(23, (40,30) 20×20),
    /// 숨김 Frame(24, (0,0)).
    fn fragment_state() -> DocumentState {
        let mut state = DocumentState::new(small_model());
        let mut hidden = record(24, 20, 0x30, placed_frame(0.0, 0.0, 5.0, 5.0));
        hidden.visible = false;
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![
                    record(20, 11, 0x40, NodePayload::Fragment(FragmentPayload)),
                    record(21, 20, 0x10, placed_frame(10.0, 20.0, 30.0, 10.0)),
                    record(22, 20, 0x20, NodePayload::Fragment(FragmentPayload)),
                    record(23, 22, 0x10, ellipse(40.0, 30.0, 20.0, 20.0)),
                    hidden,
                ],
                modifier_stacks: Vec::new(),
            }),
        )
        .unwrap();
        state
    }

    fn encoded(state: &DocumentState, ids: &[u64]) -> Vec<Vec<u8>> {
        ids.iter()
            .map(|id| encode_node_record(&state.model().record(sid(*id)).unwrap()))
            .collect()
    }

    #[test]
    fn fragment_to_frame_applies_plan_and_undo_restores_bytes() {
        // 검증 기준(VII.2.7): undo 후 원본 bytes 의미 복원.
        let mut state = fragment_state();
        let before = snapshot(&state);
        let ids = [20, 21, 22, 23, 24];
        let original = encoded(&state, &ids);
        let request = CommandRequest::at(
            &state,
            Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: sid(20) }),
        );
        let outcome = state.execute(&request).unwrap();
        // 가시 자식 bounds = (10,20)~(60,50) → Frame 50×30, 위치 (10,20), clip 없음.
        let NodePayload::Frame(converted) = state.model().payload(sid(20)).unwrap() else {
            panic!("Frame이어야 함");
        };
        assert_eq!(converted.size, Size2::new(50.0, 30.0).unwrap());
        assert_eq!(
            converted.transform.components(),
            (1.0, 0.0, 0.0, 1.0, 10.0, 20.0)
        );
        assert_eq!(converted.clipping, Clipping::Disabled);
        // StableId·자식 순서 유지, 자식 위치는 새 좌표계로 옮겨 시각 위치 동일.
        assert_eq!(
            state.model().children_in_order(sid(20)),
            vec![sid(21), sid(22), sid(24)]
        );
        let NodePayload::Frame(child) = state.model().payload(sid(21)).unwrap() else {
            panic!("Frame");
        };
        assert_eq!(child.transform.components().4, 0.0);
        assert_eq!(child.transform.components().5, 0.0);
        let NodePayload::Shape(ShapeGeometry::Ellipse(e)) = state.model().payload(sid(23)).unwrap()
        else {
            panic!("Ellipse");
        };
        assert_eq!(
            e.bounds.origin(),
            v(30.0, 10.0),
            "중첩 Fragment 자손도 이동"
        );
        let NodePayload::Frame(hidden) = state.model().payload(sid(24)).unwrap() else {
            panic!("Frame");
        };
        assert_eq!(
            hidden.transform.components().4,
            -10.0,
            "숨김 자식도 같은 offset"
        );
        assert!(outcome.changes.changed_nodes().contains(&sid(20)));
        stellar_document::validate::validate_document(&state.model().to_records().unwrap())
            .expect("변환 후 문서 유효");

        // undo: 원본 레코드 bytes와 semantic snapshot 복원.
        let Command::RestoreNodeRecords(restore) = &outcome.inverse else {
            panic!("RestoreNodeRecords여야 함");
        };
        assert_eq!(restore.records.len(), 4, "Fragment + 옮긴 자손 3");
        run(&mut state, outcome.inverse).unwrap();
        assert_eq!(encoded(&state, &ids), original);
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn fragment_conversion_rejects_wrong_kind_and_unmovable_children() {
        let mut state = fragment_state();
        // Text는 위치 필드가 없어 offset (−10,−20)을 적용할 수 없습니다.
        run(
            &mut state,
            Command::InsertNode(InsertNode::single(record(
                25,
                20,
                0x50,
                NodePayload::Text(TextPayload::empty(0)),
            ))),
        )
        .unwrap();
        let before = snapshot(&state);
        assert_eq!(
            run(
                &mut state,
                Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: sid(20) })
            ),
            Err(CommandError::ConversionNotRepresentable {
                node: sid(20),
                child: sid(25)
            })
        );
        assert_eq!(
            run(
                &mut state,
                Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: sid(11) })
            ),
            Err(CommandError::ConversionKind {
                node: sid(11),
                expected: NodeKind::Fragment,
                found: NodeKind::Frame
            })
        );
        assert_eq!(snapshot(&state), before);

        // offset이 0이면(가시 bounds 원점이 (0,0)) 옮길 필요가 없어 Text가 있어도 변환됩니다.
        let mut state = DocumentState::new(small_model());
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![
                    record(30, 11, 0x40, NodePayload::Fragment(FragmentPayload)),
                    record(31, 30, 0x10, placed_frame(0.0, 0.0, 8.0, 8.0)),
                    record(32, 30, 0x20, NodePayload::Text(TextPayload::empty(0))),
                ],
                modifier_stacks: Vec::new(),
            }),
        )
        .unwrap();
        let before = snapshot(&state);
        let inverse = run(
            &mut state,
            Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: sid(30) }),
        )
        .unwrap();
        assert_eq!(
            state.model().payload(sid(30)).unwrap().kind(),
            NodeKind::Frame
        );
        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), before);
    }

    fn rectangle_state() -> DocumentState {
        let mut state = DocumentState::new(small_model());
        let rect = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(40.0, 20.0).unwrap(),
            corner_radius: CornerRadius::new(4.0, 4.0, 0.0, 0.0).unwrap(),
        }));
        let mut stack = ModifierStack::new(sid(40));
        stack
            .push_back(ModifierId::new(sid(41)), ModifierKind::Fill)
            .unwrap();
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![record(40, 12, 0x10, rect)],
                modifier_stacks: vec![stack],
            }),
        )
        .unwrap();
        state
    }

    fn element_ids(count: usize) -> Vec<StableId> {
        (0..count as u64).map(|i| sid(5_000 + i)).collect()
    }

    #[test]
    fn shape_to_path_keeps_id_geometry_and_undo_restores_bytes() {
        let mut state = rectangle_state();
        let before = snapshot(&state);
        let original = encoded(&state, &[40]);
        let NodePayload::Shape(geometry) = state.model().payload(sid(40)).unwrap().clone() else {
            panic!("Shape");
        };
        let needed = ConvertShapeToPath::required_ids(&geometry);
        assert!(needed > 4);
        let command = Command::ConvertShapeToPath(ConvertShapeToPath {
            node: sid(40),
            element_ids: element_ids(needed),
        });
        let request = CommandRequest::at(&state, command.clone());
        let outcome = state.execute(&request).unwrap();
        let NodePayload::Path(path) = state.model().payload(sid(40)).unwrap() else {
            panic!("Path여야 함");
        };
        // 변환 전 렌더 geometry와 같은 command, StableId·stack 유지.
        assert_eq!(
            path.data().commands(),
            evaluate_shape_path(&geometry).as_slice()
        );
        assert!(state.model().modifier_stack(sid(40)).is_some());
        assert_eq!(state.model().children_in_order(sid(12)), vec![sid(40)]);
        stellar_document::validate::validate_document(&state.model().to_records().unwrap())
            .expect("변환 후 문서 유효");
        run(&mut state, outcome.inverse).unwrap();
        assert_eq!(encoded(&state, &[40]), original);
        assert_eq!(snapshot(&state), before);
        // codec.
        assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
    }

    #[test]
    fn shape_to_path_rejects_bad_ids_and_kinds() {
        let mut state = rectangle_state();
        let needed = {
            let NodePayload::Shape(geometry) = state.model().payload(sid(40)).unwrap() else {
                panic!("Shape");
            };
            ConvertShapeToPath::required_ids(geometry)
        };
        let before = snapshot(&state);
        let convert = |ids: Vec<StableId>| {
            Command::ConvertShapeToPath(ConvertShapeToPath {
                node: sid(40),
                element_ids: ids,
            })
        };
        assert!(matches!(
            run(&mut state, convert(element_ids(needed - 1))),
            Err(CommandError::InvalidRequest { .. })
        ));
        let mut dup = element_ids(needed);
        dup[1] = dup[0];
        assert_eq!(
            run(&mut state, convert(dup)),
            Err(CommandError::DuplicateId { id: sid(5_000) })
        );
        let mut used = element_ids(needed);
        used[2] = sid(41);
        assert_eq!(
            run(&mut state, convert(used)),
            Err(CommandError::DuplicateId { id: sid(41) })
        );
        assert_eq!(
            run(
                &mut state,
                Command::ConvertShapeToPath(ConvertShapeToPath {
                    node: sid(12),
                    element_ids: Vec::new(),
                })
            ),
            Err(CommandError::ConversionKind {
                node: sid(12),
                expected: NodeKind::Shape,
                found: NodeKind::Frame
            })
        );
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn restore_records_checks_kind_compatibility() {
        let mut state = rectangle_state();
        let needed = {
            let NodePayload::Shape(geometry) = state.model().payload(sid(40)).unwrap() else {
                panic!("Shape");
            };
            ConvertShapeToPath::required_ids(geometry)
        };
        run(
            &mut state,
            Command::ConvertShapeToPath(ConvertShapeToPath {
                node: sid(40),
                element_ids: element_ids(needed),
            }),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        // 13 Frame이 40(Path)을 clip으로 참조.
        let request = CommandRequest::at(
            &state,
            Command::SetProperty(crate::property::SetProperty {
                node: sid(13),
                key: stellar_document::frame::keys::CLIPPING,
                value: crate::property::PropertyValue::Clipping(Clipping::Path(NodeRef::new(sid(
                    40,
                )))),
            }),
        );
        state.execute(&request).unwrap();
        let before = snapshot(&state);
        // 40을 Shape로 되돌리면 clip 대상이 Path가 아니게 됩니다.
        let mut shape = state.model().record(sid(40)).unwrap();
        shape.payload = NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
            bounds: Rect::new(v(0.0, 0.0), Size2::new(1.0, 1.0).unwrap()),
        }));
        assert_eq!(
            run(
                &mut state,
                Command::RestoreNodeRecords(RestoreNodeRecords {
                    records: vec![shape.clone()]
                })
            ),
            Err(CommandError::ReferenceTypeMismatch {
                owner: sid(13),
                target: sid(40)
            })
        );
        // 자식 있는 Frame을 Text로, 부모 변경, 빈 요청은 거부.
        let mut text = state.model().record(sid(11)).unwrap();
        text.payload = NodePayload::Text(TextPayload::empty(0));
        assert!(matches!(
            run(
                &mut state,
                Command::RestoreNodeRecords(RestoreNodeRecords {
                    records: vec![text]
                })
            ),
            Err(CommandError::InvalidParent { .. })
        ));
        let mut moved = state.model().record(sid(12)).unwrap();
        moved.order_key = OrderKey::from_bytes(vec![0x77]).unwrap();
        assert!(matches!(
            run(
                &mut state,
                Command::RestoreNodeRecords(RestoreNodeRecords {
                    records: vec![moved]
                })
            ),
            Err(CommandError::InvalidRequest { .. })
        ));
        assert!(matches!(
            run(
                &mut state,
                Command::RestoreNodeRecords(RestoreNodeRecords {
                    records: Vec::new()
                })
            ),
            Err(CommandError::InvalidRequest { .. })
        ));
        assert_eq!(snapshot(&state), before);
        let command = Command::RestoreNodeRecords(RestoreNodeRecords {
            records: vec![shape],
        });
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0x0032u16.to_le_bytes());
        assert_eq!(decode_command(&bytes).unwrap(), command);
    }
}
