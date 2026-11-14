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
use crate::tree::{id_owners, path_element_ids, ChildIndex, IdOwner};

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
        // 평가할 수 없는 geometry(좌표가 f32 범위 밖)는 bounds 없음으로 둡니다(panic 금지).
        NodePayload::Shape(geometry) => evaluate_shape_path(geometry)
            .ok()
            .and_then(|commands| path_bounds(&commands)),
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
            // 옮긴 bounds도 끝 좌표가 유한해야 저장·재로드할 수 있습니다(`EllipseParams::new`).
            ShapeGeometry::Ellipse(params) => shift_rect(params.bounds, offset)
                .and_then(|bounds| EllipseParams::new(bounds).ok())
                .map(|params| NodePayload::Shape(ShapeGeometry::Ellipse(params))),
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
    ///
    /// geometry를 평가할 수 없으면(좌표가 f32 범위 밖) panic 대신
    /// [`CommandError::InvalidRequest`]입니다.
    pub fn required_ids(geometry: &ShapeGeometry) -> Result<usize, CommandError> {
        let commands = evaluate_shape_path(geometry).map_err(|_| CommandError::InvalidRequest {
            reason: "Shape geometry를 path로 만들 수 없습니다",
        })?;
        Ok(commands
            .iter()
            .map(|command| match command {
                PathCommand::MoveTo(_) | PathCommand::LineTo(_) => 1,
                PathCommand::CubicTo { .. } => 3,
                PathCommand::Close => 0,
            })
            .sum())
    }

    fn converted(&self, model: &DocumentModel) -> Result<NodeRecord, CommandError> {
        let mut record = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let NodePayload::Shape(geometry) = &record.payload else {
            return Err(kind_error(self.node, NodeKind::Shape, record.kind()));
        };
        if self.element_ids.len() != ConvertShapeToPath::required_ids(geometry)? {
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

/// `replacements`로 같은 id의 node 레코드를 바꾼 **결과 문서**가 전체 validator(VI.10.1)에서 새 위반을
/// 만들지 않는지 확인합니다. 문서를 바꾸지 않습니다.
///
/// 결과 레코드를 통째로 만들어(O(N), 모든 레코드 clone) 검사하므로 command 검증 경로에서는 쓰지 않고,
/// [`RestoreNodeRecords::validate`]의 범위 검사가 이 전체 검사와 같은 판정을 내리는지 확인하는 test oracle로만
/// 씁니다. 이미 있던 위반은 이 command의 책임이 아니므로, 결과에 위반이 있을 때만 변경 전 문서를 다시 검사해
/// **새로 생긴 위반**만 오류로 보고합니다.
#[cfg(test)]
pub(crate) fn ensure_no_new_violations(
    model: &DocumentModel,
    replacements: &[NodeRecord],
) -> Result<(), CommandError> {
    use stellar_document::validate::{
        validate_document, DocumentValidationError, DocumentViolation,
    };
    let by_id: BTreeMap<StableId, &NodeRecord> = replacements.iter().map(|r| (r.id, r)).collect();
    let mut records = model.to_records()?;
    let mut originals: Vec<(usize, NodeRecord)> = Vec::with_capacity(by_id.len());
    for (index, node) in records.nodes.iter_mut().enumerate() {
        if let Some(record) = by_id.get(&node.id) {
            originals.push((index, std::mem::replace(node, (*record).clone())));
        }
    }
    let Err(after) = validate_document(&records) else {
        return Ok(());
    };
    for (index, original) in originals {
        records.nodes[index] = original;
    }
    let existing = validate_document(&records)
        .err()
        .map(|error| error.violations)
        .unwrap_or_default();
    let introduced: Vec<DocumentViolation> = after
        .violations
        .into_iter()
        .filter(|violation| !existing.contains(violation))
        .collect();
    if introduced.is_empty() {
        Ok(())
    } else {
        Err(CommandError::InvalidResult(Box::new(
            DocumentValidationError {
                violations: introduced,
            },
        )))
    }
}

/// 지정 node 레코드(종류 포함)를 통째로 되돌립니다(변환 command의 역).
///
/// 각 레코드의 node가 있어야 하고 부모·OrderKey는 현재와 같아야 합니다. 바뀐 종류가 자식·modifier
/// stack·들어오는 clip 참조와 맞는지, 새 payload의 clip 대상과 path element id가 유효한지 검사합니다.
/// View root(부모 없는 node)는 Frame이 아닌 종류로 바꿀 수 없습니다. 새로 path element가 되는 id를 다른
/// 참조(clip·modifier, 교체 레코드의 clip — 바꾼 clip이든 그대로 둔 clip이든)가 가리키는 결과도 거부합니다.
///
/// 부모·순서·modifier stack은 바뀌지 않으므로, 전체 validator(VI.10.1)의 위반 중 이 command가 새로 만들 수
/// 있는 것은 위 대상 검사가 모두 다룹니다(종류 변경: root·자식·modifier owner·들어오는 clip, payload 변경:
/// clip 대상·path element id). 그래서 결과 문서 전체를 다시 만들어 검사하지 않습니다(undo·redo 비용).
/// 같은 판정인지는 test oracle([`ensure_no_new_violations`])과 비교하는 property test가 확인합니다.
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
        let replaced: BTreeSet<StableId> = self.records.iter().map(|r| r.id).collect();
        if replaced.len() != self.records.len() {
            return Err(CommandError::InvalidRequest {
                reason: "같은 node 레코드가 둘 이상입니다",
            });
        }
        let released: BTreeSet<StableId> = replaced
            .iter()
            .filter_map(|id| model.payload(*id))
            .flat_map(path_element_ids)
            .collect();
        // clip 대상이나 새 path element의 소유권을 조회할 때만 문서 전체 id 공간이 필요합니다.
        let owners = std::cell::LazyCell::new(|| {
            let mut owners = id_owners(model);
            for element in &released {
                owners.remove(element);
            }
            owners
        });
        let new_kinds: BTreeMap<StableId, NodeKind> =
            self.records.iter().map(|r| (r.id, r.kind())).collect();
        let kind_after = |id: StableId| -> Option<IdOwner> {
            new_kinds
                .get(&id)
                .map(|kind| IdOwner::Node(*kind))
                .or_else(|| owners.get(&id).copied())
        };
        let references = model.references();
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
            // View root는 Frame이어야 합니다(VI.10.1 root 규칙). 부모는 바꿀 수 없으므로 부모 없는
            // 레코드는 현재도 root입니다.
            if record.parent.is_none() && kind != NodeKind::Frame && current.kind() != kind {
                return Err(CommandError::RootKind {
                    node: record.id,
                    found: kind,
                });
            }
            if !can_have_children(kind)
                && model
                    .runtime_id(record.id)
                    .and_then(|handle| model.columns().child_count(handle))
                    .is_some_and(|count| count > 0)
            {
                return Err(CommandError::InvalidParent {
                    node: model.children_in_order(record.id)[0],
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
        // 새로 path element가 되는 id(교체 전에는 교체 node의 path element가 아니던 id)를 참조가 가리키면 결과
        // 문서에서 참조 대상 종류가 어긋납니다(VI.10.1 RefTypeMismatch: clip은 Path node, modifier는 node 기대).
        for &element in claimed.difference(&released) {
            for from in references.inbound(element) {
                // 교체되는 node의 clip은 새 payload의 clip으로 바뀌므로(clip을 유지해도 아래 반복에서 검사) 여기서는
                // modifier 참조만 남습니다.
                let still_refers = !replaced.contains(&from)
                    || model.modifier_stack(from).is_some_and(|stack| {
                        stack
                            .entries()
                            .iter()
                            .any(|entry| entry.payload.referenced_nodes().contains(&element))
                    });
                if still_refers {
                    return Err(CommandError::ReferenceTypeMismatch {
                        owner: from,
                        target: element,
                    });
                }
            }
        }
        for record in &self.records {
            let NodePayload::Frame(frame) = &record.payload else {
                continue;
            };
            let stellar_document::frame::Clipping::Path(target) = frame.clipping else {
                continue;
            };
            // 새 clip 대상이 이 command가 claim한 path element이면 결과에서 RefTypeMismatch입니다. clip을 그대로 둔
            // 레코드라도 대상이 새로 element가 된 id(교체 전에는 element가 아니던 id)면 새 위반입니다. clip을 유지했고
            // 대상이 교체 전에도 교체 node의 element였으면 이미 있던 위반이므로 이 command의 책임이 아닙니다.
            let unchanged = matches!(
                model.payload(record.id),
                Some(NodePayload::Frame(current)) if current.clipping == frame.clipping
            );
            let target = target.id();
            if claimed.contains(&target) && (!unchanged || !released.contains(&target)) {
                return Err(CommandError::ReferenceTypeMismatch {
                    owner: record.id,
                    target,
                });
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
        let needed = ConvertShapeToPath::required_ids(&geometry).unwrap();
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
            evaluate_shape_path(&geometry).unwrap().as_slice()
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
    fn unevaluable_shapes_are_rejected_without_panic() {
        // posthoc DOC-SHAPE-PANIC 회귀(종단): 크기 0 bounds + radius polygon과 끝 좌표가 f32 범위를 넘는
        // ellipse에서 `ConvertShapeToPath`(id 수 검사 전 `required_ids` 평가)와 부모 Fragment의
        // `ConvertFragmentToFrame`(leaf bounds 평가)이 panic했습니다(release는 abort).
        let mut polygon = vec![4u8];
        for value in [0.0f32; 4] {
            polygon.extend_from_slice(&value.to_le_bytes());
        }
        polygon.extend_from_slice(&3u32.to_le_bytes());
        polygon.extend_from_slice(&0.0f32.to_le_bytes());
        polygon.extend_from_slice(&1.0f32.to_le_bytes());
        let degenerate = ShapeGeometry::decode(&polygon).expect("디코딩은 수용");
        // 디코딩은 끝 좌표가 넘치는 bounds를 거부하므로, 필드를 직접 채운 in-memory geometry로 만듭니다.
        let hostile = ShapeGeometry::Ellipse(EllipseParams {
            bounds: Rect::new(v(f32::MAX, 0.0), Size2::new(f32::MAX, 1.0).unwrap()),
        });
        let mut state = DocumentState::new(small_model());
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![
                    record(30, 11, 0x40, NodePayload::Fragment(FragmentPayload)),
                    record(31, 30, 0x10, NodePayload::Shape(degenerate)),
                    record(32, 30, 0x20, NodePayload::Shape(hostile)),
                ],
                modifier_stacks: Vec::new(),
            }),
        )
        .unwrap();
        let convert = |node: u64, ids: Vec<StableId>| {
            Command::ConvertShapeToPath(ConvertShapeToPath {
                node: sid(node),
                element_ids: ids,
            })
        };
        assert!(matches!(
            ConvertShapeToPath::required_ids(&hostile),
            Err(CommandError::InvalidRequest { .. })
        ));
        assert!(matches!(
            run(&mut state, convert(32, Vec::new())),
            Err(CommandError::InvalidRequest { .. })
        ));
        // 평가할 수 없는 자식은 bounds 없음으로 빠지고, Fragment 변환은 남은 자식으로 진행합니다.
        let inverse = run(
            &mut state,
            Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: sid(30) }),
        )
        .expect("Fragment 변환");
        run(&mut state, inverse).unwrap();
        // 크기 0 polygon은 직선 꼭짓점 path(MoveTo + LineTo 2개 + Close)로 변환됩니다.
        let needed = ConvertShapeToPath::required_ids(&degenerate).unwrap();
        assert_eq!(needed, 3);
        assert!(matches!(
            run(&mut state, convert(31, Vec::new())),
            Err(CommandError::InvalidRequest { .. })
        ));
        run(&mut state, convert(31, element_ids(needed))).expect("변환");
        assert_eq!(
            state.model().payload(sid(31)).unwrap().kind(),
            NodeKind::Path
        );
    }

    #[test]
    fn shape_to_path_rejects_bad_ids_and_kinds() {
        let mut state = rectangle_state();
        let needed = {
            let NodePayload::Shape(geometry) = state.model().payload(sid(40)).unwrap() else {
                panic!("Shape");
            };
            ConvertShapeToPath::required_ids(geometry).unwrap()
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
            ConvertShapeToPath::required_ids(geometry).unwrap()
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
    #[test]
    fn restore_records_rejects_view_root_kind_change() {
        // 회귀(review RESTORE-ROOT-KIND): View root(10)를 Frame이 아닌 종류로 바꾸는 복원은 검증 단계에서
        // 거부되고 문서는 그대로입니다. 바이트로 디코딩한 command(agent·journal 재생 경로)도 같습니다.
        use stellar_document::image::ImagePayload;
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        let revision = state.revision();
        let payloads = [
            NodePayload::Fragment(FragmentPayload),
            NodePayload::Text(TextPayload::empty(0)),
            ellipse(0.0, 0.0, 4.0, 4.0),
            NodePayload::Image(ImagePayload::new(
                stellar_core_types::reference::AssetRef::new(sid(90)),
                Size2::new(4.0, 4.0).unwrap(),
            )),
        ];
        for payload in payloads {
            let found = payload.kind();
            let mut root = state.model().record(sid(10)).unwrap();
            root.payload = payload;
            let command = Command::RestoreNodeRecords(RestoreNodeRecords {
                records: vec![root],
            });
            let decoded = decode_command(&encode_command(&command)).unwrap();
            for candidate in [command, decoded] {
                assert_eq!(
                    candidate.validate(state.model()),
                    Err(CommandError::RootKind {
                        node: sid(10),
                        found
                    })
                );
                assert_eq!(
                    run(&mut state, candidate),
                    Err(CommandError::RootKind {
                        node: sid(10),
                        found
                    })
                );
            }
            assert_eq!(snapshot(&state), before, "{found:?}: 문서 불변");
            assert_eq!(state.revision(), revision);
            stellar_document::validate::validate_document(&state.model().to_records().unwrap())
                .expect("문서는 계속 유효");
        }
        // root의 공통 필드(이름·표시)만 바꾸는 복원은 허용됩니다.
        let mut renamed = state.model().record(sid(10)).unwrap();
        renamed.display_name = "root".into();
        renamed.visible = false;
        run(
            &mut state,
            Command::RestoreNodeRecords(RestoreNodeRecords {
                records: vec![renamed],
            }),
        )
        .unwrap();
        assert_eq!(state.model().record(sid(10)).unwrap().display_name, "root");
    }

    #[test]
    fn result_validator_backstop_rejects_new_violations_only() {
        // 결과 문서 전체 validator(VI.10.1) backstop: 대상 검사를 우회한 레코드 집합도 새 위반이 있으면
        // 거부합니다. 기존 위반을 그대로 두는 복원은 막지 않습니다(이미 잘못된 문서의 부분 수정 허용).
        let model = small_model();
        let mut root = model.record(sid(10)).unwrap();
        root.payload = NodePayload::Fragment(FragmentPayload);
        let Err(CommandError::InvalidResult(error)) =
            ensure_no_new_violations(&model, std::slice::from_ref(&root))
        else {
            panic!("RootNotFrame 위반으로 거부되어야 합니다");
        };
        assert!(error.violations.iter().any(|v| matches!(
            v,
            stellar_document::validate::DocumentViolation::RootNotFrame { .. }
        )));
        let mut renamed = model.record(sid(11)).unwrap();
        renamed.display_name = "ok".into();
        ensure_no_new_violations(&model, &[renamed]).unwrap();

        // root가 이미 Fragment인(잘못된) 문서: 다른 node 복원은 새 위반이 없어 허용.
        let mut broken = DocumentModel::new(crate::command::test_fixtures::definition());
        let mut fragment_root = frame(10, None, 0x10);
        fragment_root.payload = NodePayload::Fragment(FragmentPayload);
        broken.insert_node(fragment_root).unwrap();
        broken.insert_node(frame(11, Some(10), 0x10)).unwrap();
        let mut child = broken.record(sid(11)).unwrap();
        child.display_name = "수정".into();
        ensure_no_new_violations(&broken, &[child]).unwrap();
    }

    /// 점 2개짜리 Path payload. element id는 `ids`에서 차례로 씁니다(모자라면 9_000번대 fresh id).
    fn path_with(ids: &[u64]) -> NodePayload {
        path_with_ids(&ids.iter().map(|n| sid(*n)).collect::<Vec<_>>())
    }

    /// [`path_with`]와 같되 element id를 [`StableId`]로 받습니다.
    fn path_with_ids(ids: &[StableId]) -> NodePayload {
        let data = PathData::new(
            stellar_document::path::WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(Vec2::ZERO),
                PathCommand::LineTo(v(5.0, 5.0)),
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut queue = ids.to_vec();
        queue.reverse();
        let mut fresh = 9_000u64;
        NodePayload::Path(IdentifiedPath::assign(data, &mut || {
            queue.pop().unwrap_or_else(|| {
                fresh += 1;
                sid(fresh)
            })
        }))
    }

    fn clip_to(target: u64) -> NodePayload {
        NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(target))),
            ..FramePayload::default()
        })
    }

    fn restore(records: Vec<NodeRecord>) -> RestoreNodeRecords {
        RestoreNodeRecords { records }
    }

    #[test]
    #[ignore = "release 빌드에서 5만 leaf 레코드 복원 시간을 측정"]
    fn measure_restore_single_frame_50k() {
        let mut model = DocumentModel::new(crate::command::test_fixtures::definition());
        model.insert_node(frame(10, None, 1)).unwrap();
        let leaf_keys = OrderKey::evenly_spaced(500);
        for group in 0..100 {
            let parent = 100 + group;
            model
                .insert_node(frame(parent, Some(10), group as u8 + 1))
                .unwrap();
            for (offset, key) in leaf_keys.iter().enumerate() {
                let mut leaf = frame(1000 + group * 500 + offset as u64, Some(parent), 1);
                leaf.order_key = key.clone();
                model.insert_node(leaf).unwrap();
            }
        }
        let mut renamed = model.record(sid(25_000)).unwrap();
        renamed.display_name = "renamed".into();
        let command = restore(vec![renamed]);
        command.validate(&model).unwrap();
        let mut samples = Vec::new();
        for _ in 0..101 {
            let start = std::time::Instant::now();
            command.validate(std::hint::black_box(&model)).unwrap();
            samples.push(start.elapsed());
        }
        samples.sort_unstable();
        println!(
            "restore validate, 50,000 leaves, 101 samples: p50 {:?}, p95 {:?}, p99 {:?}",
            samples[50], samples[95], samples[99]
        );
        let mut state = DocumentState::new(model);
        let mut inverse = Command::RestoreNodeRecords(command);
        samples.clear();
        for _ in 0..101 {
            let start = std::time::Instant::now();
            inverse = run(&mut state, inverse).unwrap();
            samples.push(start.elapsed());
        }
        samples.sort_unstable();
        println!(
            "restore execute, 50,000 leaves, 101 samples: p50 {:?}, p95 {:?}, p99 {:?}",
            samples[50], samples[95], samples[99]
        );
    }

    #[test]
    fn restore_rejects_new_path_elements_that_references_point_to() {
        // 회귀(review RESTORE-BACKSTOP-PERF): 결과 문서 전체 재검사를 범위 검사로 바꾼 뒤에도, 대상 검사가 놓치던
        // 유일한 경우 — 새 path element id를 다른 참조가 이미 가리키는 결과(RefTypeMismatch) — 를 거부합니다.
        // 11은 없는 id 500을 clip, 13의 Boolean modifier는 없는 id 501을 참조합니다.
        use stellar_document::modifier::{BooleanOp, ModifierEntry, ModifierPayload};
        let mut model = small_model();
        model.set_payload(sid(11), clip_to(500)).unwrap();
        let mut stack = ModifierStack::new(sid(13));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(60)),
                order: OrderKey::from_bytes(vec![0x10]).unwrap(),
                kind: ModifierKind::Boolean,
                enabled: true,
                payload: ModifierPayload::Boolean {
                    op: BooleanOp::Union,
                    operands: vec![NodeRef::new(sid(501))],
                },
            })
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        stellar_document::validate::validate_document(&model.to_records().unwrap())
            .expect("미해결 참조만 있는 유효 문서");
        let as_path = |ids: &[u64]| {
            let mut record = model.record(sid(12)).unwrap();
            record.payload = path_with(ids);
            record
        };
        let renamed = |id: u64| {
            let mut record = model.record(sid(id)).unwrap();
            record.display_name = "이름만".into();
            record
        };
        let cases = [
            // 다른 Frame의 clip 대상이 path element가 됨.
            (
                restore(vec![as_path(&[500])]),
                CommandError::ReferenceTypeMismatch {
                    owner: sid(11),
                    target: sid(500),
                },
            ),
            // modifier 참조 대상이 path element가 됨.
            (
                restore(vec![as_path(&[501])]),
                CommandError::ReferenceTypeMismatch {
                    owner: sid(13),
                    target: sid(501),
                },
            ),
            // 같은 command 안 다른 교체 레코드의 새 clip이 새 path element를 가리킴.
            (
                restore(vec![as_path(&[502]), {
                    let mut root = model.record(sid(10)).unwrap();
                    root.payload = clip_to(502);
                    root
                }]),
                CommandError::ReferenceTypeMismatch {
                    owner: sid(10),
                    target: sid(502),
                },
            ),
            // 교체 레코드가 clip을 그대로 둔 채(이름만 변경) 그 미해결 clip 대상을 같은 command의 다른 레코드가 새
            // path element로 씀(사후 리뷰: clip 유지 + id 재사용 조합). 레코드 순서와 무관합니다.
            (
                restore(vec![renamed(11), as_path(&[500])]),
                CommandError::ReferenceTypeMismatch {
                    owner: sid(11),
                    target: sid(500),
                },
            ),
            (
                restore(vec![as_path(&[500]), renamed(11)]),
                CommandError::ReferenceTypeMismatch {
                    owner: sid(11),
                    target: sid(500),
                },
            ),
        ];
        for (command, expected) in cases {
            assert!(
                matches!(
                    ensure_no_new_violations(&model, &command.records),
                    Err(CommandError::InvalidResult(_))
                ),
                "전체 validator(oracle)도 거부하는 결과"
            );
            assert_eq!(command.validate(&model), Err(expected));
        }
        // 11 자신을 Path로 바꾸면 그 clip은 사라지므로 500을 element로 써도 됩니다(교체 node의 옛 clip은 무시).
        let mut own = model.record(sid(11)).unwrap();
        own.payload = path_with(&[500]);
        let command = restore(vec![own]);
        assert!(!matches!(
            command.validate(&model),
            Err(CommandError::ReferenceTypeMismatch { .. })
        ));
        // 아무도 참조하지 않는 fresh element는 허용합니다.
        let command = restore(vec![as_path(&[])]);
        command.validate(&model).unwrap();
        ensure_no_new_violations(&model, &command.records).unwrap();
    }

    #[test]
    fn restore_keeps_allowing_unchanged_clip_to_reclaimed_element() {
        // 이미 잘못된 문서(11의 clip이 12의 path element 500을 가리킴)에서 11의 이름만 바꾸고 12의 path를 그대로
        // 되돌리는 복원은 새 위반을 만들지 않으므로(500은 교체 전에도 element) 범위 검사·oracle 모두 허용합니다.
        let mut model = DocumentModel::new(crate::command::test_fixtures::definition());
        model.insert_node(frame(10, None, 0x10)).unwrap();
        model
            .insert_node(record(11, 10, 0x10, clip_to(500)))
            .unwrap();
        model
            .insert_node(record(12, 10, 0x20, path_with(&[500])))
            .unwrap();
        assert!(
            stellar_document::validate::validate_document(&model.to_records().unwrap()).is_err(),
            "clip이 path element를 가리키는 이미 잘못된 문서"
        );
        let mut clip_owner = model.record(sid(11)).unwrap();
        clip_owner.display_name = "이름만".into();
        let command = restore(vec![clip_owner, model.record(sid(12)).unwrap()]);
        ensure_no_new_violations(&model, &command.records).unwrap();
        command.validate(&model).unwrap();
    }

    #[test]
    fn scoped_restore_validation_never_accepts_what_the_full_validator_rejects() {
        // 범위 검사가 결과 문서 전체 validator(test oracle)보다 느슨하지 않은지: random 문서·random 복원 후보에서
        // validate가 통과하면 oracle도 새 위반이 없어야 합니다. 후보는 종류 변경, 기존·미해결 참조 대상을 element로
        // 재사용하는 Path, 기존·새 element·없는 id를 clip하는 Frame을 섞습니다. 우연히 나오기 어려운 조합 — payload를
        // 유지한(이름만 바꾼) Frame의 미해결 clip 대상을 같은 command의 다른 레코드가 path element로 재사용 — 은
        // 문서마다 의도적으로 만듭니다(사후 리뷰 회귀).
        use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
        let (mut accepted, mut rejected) = (0usize, 0usize);
        let mut reused_dangling = 0usize;
        let check = |model: &DocumentModel,
                     command: RestoreNodeRecords,
                     seed: u64,
                     accepted: &mut usize,
                     rejected: &mut usize| match command.validate(model) {
            Ok(()) => {
                *accepted += 1;
                assert_eq!(
                    ensure_no_new_violations(model, &command.records),
                    Ok(()),
                    "seed {seed}: 범위 검사가 통과시킨 후보 {:?}를 전체 validator가 거부",
                    command.records.iter().map(|r| r.id).collect::<Vec<_>>()
                );
            }
            Err(_) => *rejected += 1,
        };
        for seed in 0..150u64 {
            let model = random_valid_document(seed);
            let mut rng = SplitMix64::new(seed ^ 0xC0FF_EE00);
            let ids = crate::tree::node_ids(&model);
            if ids.is_empty() {
                continue;
            }
            let references = crate::tree::reference_index(&model);
            assert_eq!(model.references(), &references);
            // 참조 대상 후보: 기존 node, 다른 node의 path element, 미해결 참조 대상, fresh id.
            let mut targets: Vec<StableId> = ids.clone();
            for id in &ids {
                targets.extend(references.outbound(*id));
                if let Some(payload) = model.payload(*id) {
                    targets.extend(path_element_ids(payload));
                }
            }
            // payload를 유지한 Frame의 미해결 clip 대상을 다른 node의 새 Path element로 재사용하는 후보.
            let owners = id_owners(&model);
            for &frame_id in &ids {
                let Some(NodePayload::Frame(current)) = model.payload(frame_id) else {
                    continue;
                };
                let Clipping::Path(target) = current.clipping else {
                    continue;
                };
                if target.id() == StableId::NIL || owners.contains_key(&target.id()) {
                    continue;
                }
                let other = *rng.pick(&ids);
                if other == frame_id {
                    continue;
                }
                let mut kept = model.record(frame_id).unwrap();
                kept.display_name = format!("유지 {}", rng.below(100));
                let mut reuse = model.record(other).unwrap();
                reuse.payload = path_with_ids(&[target.id()]);
                let records = if rng.one_in(2) {
                    vec![kept, reuse]
                } else {
                    vec![reuse, kept]
                };
                reused_dangling += 1;
                check(&model, restore(records), seed, &mut accepted, &mut rejected);
            }
            for _ in 0..12 {
                let mut records = Vec::new();
                for _ in 0..(1 + rng.below(2)) {
                    let id = *rng.pick(&ids);
                    if records.iter().any(|r: &NodeRecord| r.id == id) {
                        continue;
                    }
                    let mut record = model.record(id).unwrap();
                    let pick = |rng: &mut SplitMix64| *rng.pick(&targets);
                    record.payload = match rng.below(5) {
                        0 => NodePayload::Frame(FramePayload {
                            clipping: Clipping::Path(NodeRef::new(pick(&mut rng))),
                            ..FramePayload::default()
                        }),
                        1 => {
                            let data = PathData::new(
                                stellar_document::path::WindingRule::NonZero,
                                vec![
                                    PathCommand::MoveTo(Vec2::ZERO),
                                    PathCommand::LineTo(v(1.0, 2.0)),
                                ],
                            )
                            .unwrap();
                            let mut fresh = 7_000_000 + rng.below(1_000);
                            let reuse = rng.one_in(2);
                            let first = pick(&mut rng);
                            let mut used = false;
                            NodePayload::Path(IdentifiedPath::assign(data, &mut || {
                                if reuse && !used {
                                    used = true;
                                    first
                                } else {
                                    fresh += 1;
                                    sid(fresh)
                                }
                            }))
                        }
                        2 => NodePayload::Fragment(FragmentPayload),
                        3 => NodePayload::Text(TextPayload::empty(0)),
                        _ => record.payload.clone(),
                    };
                    record.display_name = format!("후보 {}", rng.below(100));
                    records.push(record);
                }
                check(&model, restore(records), seed, &mut accepted, &mut rejected);
            }
        }
        assert!(
            accepted > 200 && rejected > 200,
            "통과 {accepted}, 거부 {rejected}"
        );
        assert!(
            reused_dangling > 50,
            "미해결 clip 대상 재사용 후보 {reused_dangling}"
        );
    }
}
