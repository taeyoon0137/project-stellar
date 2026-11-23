//! 영속 레코드 canonical 인코딩과 View subtree 분할 (VI.10.2).
//!
//! chunk model 직렬화(project crate의 document I/O)가 소비하는 **storage 독립** 형식입니다.
//! document crate는 storage에 의존하지 않으므로 canonical 규칙(V.1: LE 고정 폭, `u32`
//! 길이 프리픽스, `-0.0` 정규화는 각 payload 인코더)을 직접 따릅니다. runtime 핸들은
//! 레코드에 존재하지 않으므로 이 형식에 기록될 수 없습니다.
//!
//! node 레코드 형식:
//!
//! | 필드 | 형식 |
//! |---|---|
//! | id | 16바이트 StableId |
//! | kind | `u16` LE ([`NodeKind`] numeric) |
//! | parent | flag `u8`(0/1) + 16바이트 StableId |
//! | order key | `u32` 길이 + bytes |
//! | visible, locked | `u8` 0/1 각각 |
//! | display name | `u32` 길이 + UTF-8 |
//! | payload | `u32` 길이 + 종류별 canonical payload |
//!
//! subtree 형식: root id 16 + node 수 `u32` + (`u32` 길이 + node 레코드)* + stack 수 `u32` +
//! (`u32` 길이 + [`ModifierStack::encode`])* + [선택 layout 절]. node는 root부터 pre-order(형제는 OrderKey,
//! StableId 순), stack은 소유 node StableId 순입니다.
//!
//! layout 절(IX.11.6, ADR-0086)은 record 수 `u32`(1 이상) + (`u32` 길이 + [`LayoutRecord::encode`])*이고, 소유
//! node StableId 순입니다. 명시 layout longhand가 없는 subtree는 이 절을 쓰지 않으므로 이전 형식의 bytes와
//! 같습니다. 읽을 때 절이 없으면 record가 없는 것입니다. 수 0으로 쓴 절은 정규형이 아니라 거부합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

use crate::component::instance::{ComponentInstancePayload, InstancePayloadError};
use crate::definitions::{DefinitionError, DocumentDefinition};
use crate::fragment::{FragmentError, FragmentPayload};
use crate::frame::{FrameError, FramePayload};
use crate::image::{ImageError, ImagePayload};
use crate::layout_record::{LayoutRecord, LayoutRecordError};
use crate::modifier::{ModifierError, ModifierStack};
use crate::node_kind::NodeKind;
use crate::path::{IdentifiedPath, PathError};
use crate::records::{DocumentRecords, NodePayload, NodeRecord};
use crate::shape::{ShapeError, ShapeGeometry};
use crate::text::{TextError, TextPayload};

/// 레코드 인코딩·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordCodecError {
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 알 수 없는 node kind 값.
    UnknownNodeKind {
        /// 발견된 값.
        value: u16,
    },
    /// 인스턴스 참조 또는 희소 override 형식 오류.
    ComponentInstance(InstancePayloadError),
    /// subtree 첫 node가 선언된 root가 아닙니다.
    SubtreeRootMismatch {
        /// 선언된 root.
        declared: StableId,
        /// 첫 node(없으면 NIL).
        found: StableId,
    },
    /// subtree 안 stack의 소유 node가 그 subtree에 없습니다.
    ForeignModifierStack {
        /// 소유 node.
        owner: StableId,
    },
    /// Frame payload 오류.
    Frame(FrameError),
    /// Fragment payload 오류.
    Fragment(FragmentError),
    /// Shape payload 오류.
    Shape(ShapeError),
    /// Path payload 오류.
    Path(PathError),
    /// Text payload 오류.
    Text(TextError),
    /// Image payload 오류.
    Image(ImageError),
    /// modifier stack 오류.
    Modifier(ModifierError),
    /// 문서 정의 오류.
    Definition(DefinitionError),
    /// layout record 오류(IX.11.6).
    Layout(LayoutRecordError),
    /// subtree 안 layout record의 소유 node가 그 subtree에 없습니다.
    ForeignLayoutRecord {
        /// 소유 node.
        owner: StableId,
    },
}

impl fmt::Display for RecordCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordCodecError::Malformed { reason } => write!(f, "레코드 형식 위반: {reason}"),
            RecordCodecError::UnknownNodeKind { value } => {
                write!(f, "알 수 없는 node kind: {value}")
            }
            RecordCodecError::ComponentInstance(e) => write!(f, "ComponentInstance payload: {e}"),
            RecordCodecError::SubtreeRootMismatch { declared, found } => {
                write!(f, "subtree root {declared}와 첫 node {found}가 다릅니다")
            }
            RecordCodecError::ForeignModifierStack { owner } => {
                write!(f, "subtree 밖 node {owner}의 modifier stack입니다")
            }
            RecordCodecError::Frame(e) => write!(f, "Frame payload: {e}"),
            RecordCodecError::Fragment(e) => write!(f, "Fragment payload: {e}"),
            RecordCodecError::Shape(e) => write!(f, "Shape payload: {e}"),
            RecordCodecError::Path(e) => write!(f, "Path payload: {e}"),
            RecordCodecError::Text(e) => write!(f, "Text payload: {e}"),
            RecordCodecError::Image(e) => write!(f, "Image payload: {e}"),
            RecordCodecError::Modifier(e) => write!(f, "modifier stack: {e}"),
            RecordCodecError::Definition(e) => write!(f, "문서 정의: {e}"),
            RecordCodecError::Layout(e) => write!(f, "layout record: {e}"),
            RecordCodecError::ForeignLayoutRecord { owner } => {
                write!(f, "subtree 밖 node {owner}의 layout record입니다")
            }
        }
    }
}

impl std::error::Error for RecordCodecError {}

fn put_u32(buf: &mut Vec<u8>, value: usize) {
    // 길이·개수는 u32 범위입니다(chunk 크기 상한이 훨씬 작음).
    buf.extend_from_slice(&(value as u32).to_le_bytes());
}

fn put_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(buf, bytes.len());
    buf.extend_from_slice(bytes);
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], RecordCodecError> {
        if self.input.len() - self.pos < n {
            return Err(RecordCodecError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_u8(&mut self) -> Result<u8, RecordCodecError> {
        Ok(self.take(1)?[0])
    }

    fn take_bool(&mut self) -> Result<bool, RecordCodecError> {
        match self.take_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(RecordCodecError::Malformed {
                reason: "bool은 0 또는 1",
            }),
        }
    }

    fn take_u16(&mut self) -> Result<u16, RecordCodecError> {
        let raw = self.take(2)?;
        Ok(u16::from_le_bytes([raw[0], raw[1]]))
    }

    fn take_u32(&mut self) -> Result<u32, RecordCodecError> {
        let raw = self.take(4)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn take_id(&mut self) -> Result<StableId, RecordCodecError> {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(raw))
    }

    fn take_bytes(&mut self) -> Result<&'a [u8], RecordCodecError> {
        let len = self.take_u32()? as usize;
        self.take(len)
    }

    fn is_at_end(&self) -> bool {
        self.pos == self.input.len()
    }

    fn expect_end(&self) -> Result<(), RecordCodecError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(RecordCodecError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

/// payload canonical bytes.
pub fn encode_payload(payload: &NodePayload) -> Vec<u8> {
    match payload {
        NodePayload::Frame(frame) => frame.encode(),
        NodePayload::Fragment(fragment) => fragment.encode(),
        NodePayload::Shape(shape) => shape.encode(),
        NodePayload::Path(path) => path.encode(),
        NodePayload::Text(text) => text.encode(),
        NodePayload::Image(image) => image.encode(),
        NodePayload::ComponentInstance(instance) => instance.encode(),
    }
}

/// `kind` payload bytes를 복원하고 검증합니다.
pub fn decode_payload(kind: NodeKind, bytes: &[u8]) -> Result<NodePayload, RecordCodecError> {
    Ok(match kind {
        NodeKind::Frame => {
            NodePayload::Frame(FramePayload::decode(bytes).map_err(RecordCodecError::Frame)?)
        }
        NodeKind::Fragment => NodePayload::Fragment(
            FragmentPayload::decode(bytes).map_err(RecordCodecError::Fragment)?,
        ),
        NodeKind::Shape => {
            NodePayload::Shape(ShapeGeometry::decode(bytes).map_err(RecordCodecError::Shape)?)
        }
        NodeKind::Path => {
            NodePayload::Path(IdentifiedPath::decode(bytes).map_err(RecordCodecError::Path)?)
        }
        NodeKind::Text => {
            NodePayload::Text(TextPayload::decode(bytes).map_err(RecordCodecError::Text)?)
        }
        NodeKind::Image => {
            NodePayload::Image(ImagePayload::decode(bytes).map_err(RecordCodecError::Image)?)
        }
        NodeKind::ComponentInstance => NodePayload::ComponentInstance(
            ComponentInstancePayload::decode(bytes).map_err(RecordCodecError::ComponentInstance)?,
        ),
    })
}

/// node 레코드 하나를 canonical bytes로 인코딩합니다.
pub fn encode_node_record(record: &NodeRecord) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&record.id.to_bytes());
    buf.extend_from_slice(&record.kind().as_u16().to_le_bytes());
    match record.parent {
        Some(parent) => {
            buf.push(1);
            buf.extend_from_slice(&parent.to_bytes());
        }
        None => buf.push(0),
    }
    put_bytes(&mut buf, record.order_key.as_bytes());
    buf.push(u8::from(record.visible));
    buf.push(u8::from(record.locked));
    put_bytes(&mut buf, record.display_name.as_bytes());
    put_bytes(&mut buf, &encode_payload(&record.payload));
    buf
}

fn decode_node_from(reader: &mut Reader<'_>) -> Result<NodeRecord, RecordCodecError> {
    let id = reader.take_id()?;
    let kind_value = reader.take_u16()?;
    let kind = NodeKind::from_u16(kind_value)
        .ok_or(RecordCodecError::UnknownNodeKind { value: kind_value })?;
    let parent = match reader.take_u8()? {
        0 => None,
        1 => Some(reader.take_id()?),
        _ => {
            return Err(RecordCodecError::Malformed {
                reason: "parent flag",
            })
        }
    };
    let order_key = OrderKey::from_bytes(reader.take_bytes()?.to_vec()).map_err(|_| {
        RecordCodecError::Malformed {
            reason: "잘못된 OrderKey",
        }
    })?;
    let visible = reader.take_bool()?;
    let locked = reader.take_bool()?;
    let display_name = std::str::from_utf8(reader.take_bytes()?)
        .map_err(|_| RecordCodecError::Malformed {
            reason: "표시 이름 UTF-8 아님",
        })?
        .to_string();
    let payload = decode_payload(kind, reader.take_bytes()?)?;
    Ok(NodeRecord {
        id,
        parent,
        order_key,
        visible,
        locked,
        display_name,
        payload,
    })
}

/// [`encode_node_record`] bytes에서 레코드를 복원합니다.
pub fn decode_node_record(bytes: &[u8]) -> Result<NodeRecord, RecordCodecError> {
    let mut reader = Reader::new(bytes);
    let record = decode_node_from(&mut reader)?;
    reader.expect_end()?;
    Ok(record)
}

/// 한 subtree chunk의 레코드: root부터 pre-order node와 그 node들의 modifier stack.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtreeRecords {
    /// subtree root node.
    pub root: StableId,
    /// root부터 pre-order(형제는 OrderKey, StableId 순).
    pub nodes: Vec<NodeRecord>,
    /// subtree 안 node의 modifier stack(소유 node StableId 순).
    pub modifier_stacks: Vec<ModifierStack>,
    /// subtree 안 node의 명시 layout record(소유 node StableId 순, IX.11.6).
    pub layout_records: Vec<LayoutRecord>,
}

impl SubtreeRecords {
    /// canonical bytes(모듈 문서의 subtree 형식).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.root.to_bytes());
        put_u32(&mut buf, self.nodes.len());
        for node in &self.nodes {
            put_bytes(&mut buf, &encode_node_record(node));
        }
        put_u32(&mut buf, self.modifier_stacks.len());
        for stack in &self.modifier_stacks {
            put_bytes(&mut buf, &stack.encode());
        }
        if !self.layout_records.is_empty() {
            put_u32(&mut buf, self.layout_records.len());
            for record in &self.layout_records {
                put_bytes(&mut buf, &record.encode());
            }
        }
        buf
    }

    /// bytes에서 복원합니다. 첫 node가 root가 아니거나 subtree 밖 node의 stack이 있으면
    /// 거부합니다. 문서 불변식(부모·순서·참조 등)은 validator(VI.10.1)가 검사합니다.
    pub fn decode(bytes: &[u8]) -> Result<SubtreeRecords, RecordCodecError> {
        let mut reader = Reader::new(bytes);
        let root = reader.take_id()?;
        let node_count = reader.take_u32()? as usize;
        let mut nodes = Vec::with_capacity(node_count.min(4096));
        for _ in 0..node_count {
            let mut inner = Reader::new(reader.take_bytes()?);
            nodes.push(decode_node_from(&mut inner)?);
            inner.expect_end()?;
        }
        let found = nodes.first().map_or(StableId::NIL, |n| n.id);
        if found != root {
            return Err(RecordCodecError::SubtreeRootMismatch {
                declared: root,
                found,
            });
        }
        let members: BTreeSet<StableId> = nodes.iter().map(|n| n.id).collect();
        let stack_count = reader.take_u32()? as usize;
        let mut modifier_stacks = Vec::with_capacity(stack_count.min(4096));
        for _ in 0..stack_count {
            let stack =
                ModifierStack::decode(reader.take_bytes()?).map_err(RecordCodecError::Modifier)?;
            if !members.contains(&stack.node) {
                return Err(RecordCodecError::ForeignModifierStack { owner: stack.node });
            }
            modifier_stacks.push(stack);
        }
        let mut layout_records = Vec::new();
        if !reader.is_at_end() {
            let count = reader.take_u32()? as usize;
            if count == 0 {
                return Err(RecordCodecError::Malformed {
                    reason: "빈 layout 절은 쓰지 않습니다",
                });
            }
            for _ in 0..count {
                let record =
                    LayoutRecord::decode(reader.take_bytes()?).map_err(RecordCodecError::Layout)?;
                if !members.contains(&record.node) {
                    return Err(RecordCodecError::ForeignLayoutRecord { owner: record.node });
                }
                if layout_records
                    .last()
                    .is_some_and(|last: &LayoutRecord| last.node >= record.node)
                {
                    return Err(RecordCodecError::Malformed {
                        reason: "layout record는 소유 node 순서로 한 번씩 씁니다",
                    });
                }
                layout_records.push(record);
            }
        }
        reader.expect_end()?;
        Ok(SubtreeRecords {
            root,
            nodes,
            modifier_stacks,
            layout_records,
        })
    }
}

/// View별 subtree 분할 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionError {
    /// 어느 View root에서도 닿지 않는 node가 있습니다(분할하면 데이터 손실).
    UnreachableNodes {
        /// 해당 node들(StableId 순).
        nodes: Vec<StableId>,
    },
    /// 어느 subtree에도 속하지 않는 modifier stack이 있습니다.
    UnassignedModifierStacks {
        /// 소유 node들.
        owners: Vec<StableId>,
    },
    /// 어느 subtree에도 속하지 않는 layout record가 있습니다.
    UnassignedLayoutRecords {
        /// 소유 node들.
        owners: Vec<StableId>,
    },
    /// 한 node가 두 View subtree에 나타납니다.
    SharedNode {
        /// 해당 node.
        node: StableId,
    },
}

impl fmt::Display for PartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartitionError::UnreachableNodes { nodes } => {
                write!(f, "View root에서 닿지 않는 node {}개", nodes.len())
            }
            PartitionError::UnassignedModifierStacks { owners } => {
                write!(f, "subtree에 속하지 않는 modifier stack {}개", owners.len())
            }
            PartitionError::UnassignedLayoutRecords { owners } => {
                write!(f, "subtree에 속하지 않는 layout record {}개", owners.len())
            }
            PartitionError::SharedNode { node } => {
                write!(f, "node {node}가 두 View subtree에 있습니다")
            }
        }
    }
}

impl std::error::Error for PartitionError {}

impl DocumentRecords {
    /// View마다 root부터의 subtree 하나로 레코드를 나눕니다(정의 순서).
    ///
    /// 초기 정책은 **View당 subtree 하나**입니다. chunk 크기 기반 분할(V.6.4)은 임계치 ADR
    /// 승인 후 추가합니다. 모든 node와 stack이 정확히 한 subtree에 들어가지 않으면 데이터
    /// 손실을 막기 위해 거부합니다 — 직렬화 전에 validator(VI.10.1)를 통과해야 합니다.
    pub fn view_subtrees(&self) -> Result<Vec<(StableId, SubtreeRecords)>, PartitionError> {
        let mut children: BTreeMap<StableId, Vec<&NodeRecord>> = BTreeMap::new();
        let mut by_id: BTreeMap<StableId, &NodeRecord> = BTreeMap::new();
        for node in &self.nodes {
            by_id.insert(node.id, node);
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(node);
            }
        }
        for kids in children.values_mut() {
            kids.sort_by(|a, b| (&a.order_key, a.id).cmp(&(&b.order_key, b.id)));
        }
        let stacks: BTreeMap<StableId, &ModifierStack> =
            self.modifier_stacks.iter().map(|s| (s.node, s)).collect();
        let layouts: BTreeMap<StableId, &LayoutRecord> =
            self.layout_records.iter().map(|r| (r.node, r)).collect();

        let mut assigned: BTreeSet<StableId> = BTreeSet::new();
        let mut out = Vec::new();
        for view in self.views() {
            let mut nodes = Vec::new();
            if let Some(root) = by_id.get(&view.root_frame) {
                let mut stack: Vec<&NodeRecord> = vec![root];
                while let Some(node) = stack.pop() {
                    if !assigned.insert(node.id) {
                        return Err(PartitionError::SharedNode { node: node.id });
                    }
                    nodes.push(node.clone());
                    if let Some(kids) = children.get(&node.id) {
                        stack.extend(kids.iter().rev());
                    }
                }
            }
            let mut members: Vec<StableId> = nodes.iter().map(|n| n.id).collect();
            members.sort();
            let modifier_stacks = members
                .iter()
                .filter_map(|id| stacks.get(id).map(|s| (*s).clone()))
                .collect();
            let layout_records = members
                .iter()
                .filter_map(|id| layouts.get(id).map(|r| (*r).clone()))
                .collect();
            out.push((
                view.id,
                SubtreeRecords {
                    root: view.root_frame,
                    nodes,
                    modifier_stacks,
                    layout_records,
                },
            ));
        }
        let unreachable: Vec<StableId> = by_id
            .keys()
            .filter(|id| !assigned.contains(id))
            .copied()
            .collect();
        if !unreachable.is_empty() {
            return Err(PartitionError::UnreachableNodes { nodes: unreachable });
        }
        if by_id.len() != self.nodes.len() {
            // 중복 id 레코드는 하나만 배치되어 손실됩니다.
            let mut seen = BTreeSet::new();
            let duplicate = self
                .nodes
                .iter()
                .find(|n| !seen.insert(n.id))
                .map_or(StableId::NIL, |n| n.id);
            return Err(PartitionError::SharedNode { node: duplicate });
        }
        let unassigned: Vec<StableId> = self
            .modifier_stacks
            .iter()
            .map(|s| s.node)
            .filter(|owner| !assigned.contains(owner))
            .collect();
        if !unassigned.is_empty() || stacks.len() != self.modifier_stacks.len() {
            return Err(PartitionError::UnassignedModifierStacks { owners: unassigned });
        }
        let unassigned: Vec<StableId> = self
            .layout_records
            .iter()
            .map(|r| r.node)
            .filter(|owner| !assigned.contains(owner))
            .collect();
        if !unassigned.is_empty() || layouts.len() != self.layout_records.len() {
            return Err(PartitionError::UnassignedLayoutRecords { owners: unassigned });
        }
        Ok(out)
    }

    /// 문서 정의와 subtree들로 레코드를 다시 합칩니다(canonical 순서로 정렬).
    pub fn from_subtrees(
        definition: DocumentDefinition,
        subtrees: Vec<SubtreeRecords>,
    ) -> DocumentRecords {
        let mut records = DocumentRecords::new(definition);
        for subtree in subtrees {
            records.nodes.extend(subtree.nodes);
            records.modifier_stacks.extend(subtree.modifier_stacks);
            records.layout_records.extend(subtree.layout_records);
        }
        records.sort_canonical();
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{PageDefinition, ViewDefinition};
    use crate::frame::{Border, Clipping, CornerRadius};
    use crate::image::{CropRect, FitMode, SamplingMode};
    use crate::modifier::{ModifierId, ModifierKind, ModifierPayload};
    use crate::path::{PathCommand, PathData, WindingRule};
    use crate::shape::{LineCap, LineParams, RectangleParams};
    use crate::text::{Paragraph, StyleRun, TextRange};
    use crate::validate::validate_document;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::reference::{AssetRef, NodeRef};
    use stellar_core_types::text::UserText;
    use stellar_core_types::value::{Size2, Transform2D, Vec2};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0abc, n)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    fn record(id: u64, parent: Option<u64>, k: u8, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: key(k),
            visible: id.is_multiple_of(2),
            locked: id.is_multiple_of(3),
            display_name: format!("이름 {id} 🎨"),
            payload,
        }
    }

    fn every_payload() -> Vec<NodePayload> {
        let color = Color::new(ColorSpace::DisplayP3, 0.25, 0.5, 1.0, 0.75).unwrap();
        let frame = FramePayload {
            size: Size2::new(100.0, 50.0).unwrap(),
            transform: Transform2D::new(-1.0, 0.0, 0.0, 1.0, 10.0, -3.5).unwrap(),
            clipping: Clipping::Path(NodeRef::new(sid(77))),
            background: Some(color),
            border: Some(Border::new(color, 2.0).unwrap()),
            corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap(),
            layout_mode: Some(sid(88)),
        };
        let path = PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(Vec2::ZERO),
                PathCommand::CubicTo {
                    c1: Vec2::new(1.0, 1.0).unwrap(),
                    c2: Vec2::new(2.0, 1.0).unwrap(),
                    to: Vec2::new(3.0, 0.0).unwrap(),
                },
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut next = 500u64;
        let text = TextPayload::new(
            UserText::new("안녕\n👩‍💻"),
            vec![Paragraph { style: 1 }, Paragraph { style: 2 }],
            vec![StyleRun {
                range: TextRange::new(0, 2).unwrap(),
                style: 3,
            }],
        )
        .unwrap();
        let mut image = ImagePayload::new(AssetRef::new(sid(99)), Size2::new(8.0, 6.0).unwrap());
        image.fit = FitMode::Cover;
        image.crop = CropRect::from_components(0.1, 0.2, 0.5, 0.5).unwrap();
        image.sampling = SamplingMode::Nearest;
        image.color_space_override = Some(ColorSpace::Srgb);
        vec![
            NodePayload::Frame(frame),
            NodePayload::Frame(FramePayload::default()),
            NodePayload::Fragment(FragmentPayload),
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(3.0, 4.0).unwrap(),
                corner_radius: CornerRadius::ZERO,
            })),
            NodePayload::Shape(ShapeGeometry::Line(LineParams {
                start: Vec2::ZERO,
                end: Vec2::new(5.0, 5.0).unwrap(),
                cap: LineCap::Round,
            })),
            NodePayload::Path(IdentifiedPath::assign(path, &mut || {
                next += 1;
                sid(next)
            })),
            NodePayload::Text(text),
            NodePayload::Text(TextPayload::empty(0)),
            NodePayload::Image(image),
            NodePayload::ComponentInstance(ComponentInstancePayload::new(
                stellar_core_types::reference::ComponentRef::new(sid(1000)),
            )),
        ]
    }

    #[test]
    fn node_record_round_trips_every_payload_kind() {
        for (i, payload) in every_payload().into_iter().enumerate() {
            let parent = if i == 0 { None } else { Some(1) };
            let original = record(10 + i as u64, parent, 0x10 + i as u8, payload);
            let bytes = encode_node_record(&original);
            let decoded = decode_node_record(&bytes).unwrap();
            assert_eq!(decoded, original, "#{i}");
            assert_eq!(encode_node_record(&decoded), bytes, "#{i} 재인코딩");
        }
    }

    #[test]
    fn component_instance_conformance_corpus() {
        fn bytes(hex: &str) -> Vec<u8> {
            let joined: String = hex
                .lines()
                .filter(|line| !line.starts_with('#'))
                .flat_map(str::split_whitespace)
                .collect();
            (0..joined.len())
                .step_by(2)
                .map(|n| u8::from_str_radix(&joined[n..n + 2], 16).unwrap())
                .collect()
        }
        let good = bytes(include_str!(
            "../../../docs/spec/conformance/binary/subtree_component_instance_valid.hex"
        ));
        let tree = SubtreeRecords::decode(&good).unwrap();
        assert!(matches!(
            tree.nodes[1].payload,
            NodePayload::ComponentInstance(_)
        ));
        assert_eq!(tree.encode(), good);
        let bad = bytes(include_str!(
            "../../../docs/spec/conformance/binary/subtree_component_instance.hex"
        ));
        assert_eq!(
            SubtreeRecords::decode(&bad).unwrap_err(),
            RecordCodecError::ComponentInstance(InstancePayloadError::Malformed {
                reason: "입력 부족"
            })
        );
    }

    #[test]
    fn malformed_node_records_are_rejected() {
        let original = record(10, Some(1), 0x10, NodePayload::Text(TextPayload::empty(0)));
        let bytes = encode_node_record(&original);
        for cut in [0, 15, 17, 18, 34, bytes.len() - 1] {
            assert!(decode_node_record(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(decode_node_record(&extended).is_err());
        // kind 위치(16..18): 알 수 없는 값, ComponentInstance.
        let mut unknown = bytes.clone();
        unknown[16..18].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            decode_node_record(&unknown).unwrap_err(),
            RecordCodecError::UnknownNodeKind { value: 99 }
        );
        let mut component = bytes.clone();
        component[16..18].copy_from_slice(&NodeKind::ComponentInstance.as_u16().to_le_bytes());
        assert!(matches!(
            decode_node_record(&component),
            Err(RecordCodecError::ComponentInstance(_))
        ));
        // kind와 payload가 어긋나면 payload decoder가 거부(Text bytes를 Frame으로).
        let mut mismatched = bytes.clone();
        mismatched[16..18].copy_from_slice(&NodeKind::Frame.as_u16().to_le_bytes());
        assert!(matches!(
            decode_node_record(&mismatched),
            Err(RecordCodecError::Frame(_))
        ));
        // parent flag 2.
        let mut flag = bytes;
        flag[18] = 2;
        assert!(decode_node_record(&flag).is_err());
    }

    /// 정의 1개, View 2개(root 10·20), 노드 6개, stack 2개.
    fn sample() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(4), String::new(), sid(20)))
            .unwrap();
        definition.add_page(page).unwrap();
        let frame = || NodePayload::Frame(FramePayload::default());
        let shape = || {
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(1.0, 1.0).unwrap(),
                corner_radius: CornerRadius::ZERO,
            }))
        };
        let mut records = DocumentRecords::new(definition);
        // 삽입 순서를 일부러 섞습니다.
        records.nodes = vec![
            record(13, Some(11), 0x10, shape()),
            record(20, None, 0x10, frame()),
            record(12, Some(10), 0x10, shape()),
            record(11, Some(10), 0x20, NodePayload::Fragment(FragmentPayload)),
            record(10, None, 0x10, frame()),
            record(21, Some(20), 0x10, shape()),
        ];
        let mut a = ModifierStack::new(sid(21));
        a.push_back(ModifierId::new(sid(31)), ModifierKind::Opacity)
            .unwrap();
        let mut b = ModifierStack::new(sid(12));
        b.push_back(ModifierId::new(sid(30)), ModifierKind::Mask)
            .unwrap();
        b.set_payload(
            ModifierId::new(sid(30)),
            ModifierPayload::Mask {
                source: NodeRef::new(sid(13)),
            },
        )
        .unwrap();
        records.modifier_stacks = vec![a, b];
        validate_document(&records).unwrap();
        records
    }

    #[test]
    fn view_subtrees_are_preorder_and_round_trip() {
        let records = sample();
        let subtrees = records.view_subtrees().unwrap();
        assert_eq!(subtrees.len(), 2);
        let (view, first) = &subtrees[0];
        assert_eq!(*view, sid(3));
        // pre-order: 10, 12(key 0x10), 11(key 0x20), 13.
        let order: Vec<StableId> = first.nodes.iter().map(|n| n.id).collect();
        assert_eq!(order, vec![sid(10), sid(12), sid(11), sid(13)]);
        assert_eq!(first.modifier_stacks.len(), 1);
        assert_eq!(first.modifier_stacks[0].node, sid(12));
        assert_eq!(subtrees[1].1.nodes.len(), 2);

        // subtree codec round-trip과 재결합.
        let mut decoded = Vec::new();
        for (_, subtree) in &subtrees {
            let bytes = subtree.encode();
            let back = SubtreeRecords::decode(&bytes).unwrap();
            assert_eq!(&back, subtree);
            assert_eq!(back.encode(), bytes);
            decoded.push(back);
        }
        let merged = DocumentRecords::from_subtrees(records.definition.clone(), decoded);
        let mut expected = records.clone();
        expected.sort_canonical();
        assert_eq!(merged, expected, "분할·재결합 후 의미 동일");
    }

    #[test]
    fn partition_refuses_data_loss() {
        let mut records = sample();
        records.nodes.push(record(
            40,
            None,
            0x10,
            NodePayload::Frame(FramePayload::default()),
        ));
        assert_eq!(
            records.view_subtrees().unwrap_err(),
            PartitionError::UnreachableNodes {
                nodes: vec![sid(40)]
            }
        );
        let mut records = sample();
        records.modifier_stacks.push(ModifierStack::new(sid(99)));
        assert!(matches!(
            records.view_subtrees().unwrap_err(),
            PartitionError::UnassignedModifierStacks { .. }
        ));
    }

    #[test]
    fn subtree_decode_checks_root_and_stack_owners() {
        let (_, subtree) = sample().view_subtrees().unwrap().remove(0);
        let mut wrong_root = subtree.clone();
        wrong_root.root = sid(12);
        assert!(matches!(
            SubtreeRecords::decode(&wrong_root.encode()),
            Err(RecordCodecError::SubtreeRootMismatch { .. })
        ));
        let mut foreign = subtree.clone();
        foreign.modifier_stacks.push(ModifierStack::new(sid(21)));
        assert_eq!(
            SubtreeRecords::decode(&foreign.encode()).unwrap_err(),
            RecordCodecError::ForeignModifierStack { owner: sid(21) }
        );
        let bytes = subtree.encode();
        for cut in [0, 16, 19, 20, bytes.len() - 1] {
            assert!(SubtreeRecords::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut extended = bytes;
        extended.push(0);
        assert!(SubtreeRecords::decode(&extended).is_err());
    }
}
