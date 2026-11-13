//! Expression AST 직렬화 — node 형식(VIII.2.2)과 canonical AST 형식(VIII.2.5).
//!
//! canonical primitive 규칙(V.1.1: 다중 바이트 little-endian, 고정 폭 정수, `u32` LE 길이 프리픽스)을
//! 따르는 binary 형식입니다. 리터럴 값은 `TypedValue`의 안정 직렬화(III.3.5)를 길이 프리픽스 blob으로
//! 감싸 그대로 씁니다.
//!
//! **canonical AST**는 node를 map·삽입 순서가 아니라 root에서 시작하는 **child slot 순서 전위 순회**
//! 순서로 기록합니다. 공유(DAG) node는 처음 방문할 때 한 번만 기록하고, 이후에는 child 참조의 ID로만
//! 가리킵니다. 따라서 GUI가 node를 만든 순서·child를 연결한 순서와 무관하게 같은 AST는 같은 bytes가
//! 됩니다. 디코더는 다시 직렬화한 bytes가 입력과 같을 때만 받아들여(비-canonical 순서 거부) 한 AST에
//! 한 표현만 존재하게 합니다.
//!
//! ```text
//! ast       := version:u16 (= 1) ++ node_count:u32 ++ node*   (첫 node = root, 전위 slot 순서)
//! node      := id[16] ++ op[3] ++ payload ++ child_count:u32 ++ child*
//! op        := kind:u8 ++ operand:u16            (ExpressionOp::encode, VIII.1.5)
//! child     := slot_tag:u8 ++ slot_index:u16 ++ target_id[16]   (slot 오름차순, 중복 없음)
//! payload   := Literal   : 0x01 ++ blob(TypedValue) | 0x02 ++ type
//!            | Reference : 0x01 ++ token_id[16]
//!                        | 0x02 ++ blob(PropertyPath canonical bytes)
//!                        | 0x03 ++ component_property_id[16]
//!                        | 0x04 ++ environment_key:u32
//!            | Match     : pattern_count:u32 ++ blob(TypedValue)*
//!            | 그 외      : 없음
//! type      := value_tag:u8 (TypedValue type tag 0x01~0x0D) [++ enum_type:u32 (Enum일 때)]
//! blob      := len:u32 ++ bytes
//! ```
//!
//! 디코더는 닫힌 연산 집합(VIII.1.5) 밖의 연산, 미지 tag, 길이 초과·잉여 bytes, 비정렬 child를 모두
//! 오류로 거부하며 panic하지 않습니다. 길이 프리픽스는 남은 입력 길이를 넘으면 할당 전에 거부합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::{TypedValue, ValueDecodeError};

use crate::ast::{
    AstError, ChildRef, ChildSlot, EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, Literal,
    NodePayload, ReferenceTarget,
};
use crate::ops::{ClosedOpError, ExpressionOp};
use crate::types::{EnumTypeId, ExpressionType};

/// 직렬화·역직렬화 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// 입력이 도중에 끊겼거나 길이 프리픽스가 남은 입력보다 깁니다.
    Truncated,
    /// 값을 다 읽은 뒤 잉여 bytes가 있습니다.
    TrailingBytes,
    /// 길이가 `u32` 범위를 넘어 기록할 수 없습니다.
    LengthOverflow,
    /// 닫힌 연산 집합 밖의 연산(VIII.1.5).
    Op(ClosedOpError),
    /// 미지 리터럴 tag.
    UnknownLiteralTag(u8),
    /// 미지 값 타입 tag.
    UnknownTypeTag(u8),
    /// 미지 참조 종류 tag.
    UnknownReferenceKind(u8),
    /// 미지 child slot.
    UnknownSlot {
        /// slot tag.
        tag: u8,
        /// slot index.
        index: u16,
    },
    /// 리터럴·pattern 값 디코드 실패.
    Value(ValueDecodeError),
    /// PropertyPath bytes가 canonical 형식이 아닙니다.
    InvalidPropertyPath,
    /// child가 slot 오름차순이 아니거나 같은 slot이 반복되거나, 값 bytes가 canonical 형식이 아닙니다
    /// (예: `-0.0`). AST 형식에서는 node 기록 순서가 canonical 전위 순서가 아닌 경우도 포함합니다.
    NonCanonical,
    /// 지원하지 않는 AST 형식 버전(VIII.2.5).
    UnsupportedVersion(u16),
    /// node가 없는 AST(VIII.2.5).
    EmptyAst,
    /// 디코드한 node들이 AST 구조 검증을 통과하지 못했습니다(VIII.2.5).
    Ast(AstError),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Truncated => f.write_str("입력이 도중에 끊겼습니다"),
            CodecError::TrailingBytes => f.write_str("잉여 bytes가 남았습니다"),
            CodecError::LengthOverflow => f.write_str("길이가 u32 범위를 넘습니다"),
            CodecError::Op(err) => write!(f, "연산 디코드 실패: {err}"),
            CodecError::UnknownLiteralTag(tag) => write!(f, "미지 리터럴 tag: 0x{tag:02X}"),
            CodecError::UnknownTypeTag(tag) => write!(f, "미지 값 타입 tag: 0x{tag:02X}"),
            CodecError::UnknownReferenceKind(tag) => write!(f, "미지 참조 종류: 0x{tag:02X}"),
            CodecError::UnknownSlot { tag, index } => {
                write!(f, "미지 child slot: tag 0x{tag:02X}, index {index}")
            }
            CodecError::Value(err) => write!(f, "값 디코드 실패: {err}"),
            CodecError::InvalidPropertyPath => {
                f.write_str("PropertyPath bytes가 canonical이 아닙니다")
            }
            CodecError::NonCanonical => {
                f.write_str("canonical 형식이 아닙니다(child 순서·값 bytes)")
            }
            CodecError::UnsupportedVersion(version) => {
                write!(f, "지원하지 않는 AST 형식 버전: {version}")
            }
            CodecError::EmptyAst => f.write_str("node가 없는 AST입니다"),
            CodecError::Ast(err) => write!(f, "AST 구조 오류: {err}"),
        }
    }
}

impl std::error::Error for CodecError {}

const LITERAL_VALUE: u8 = 0x01;
const LITERAL_NULL: u8 = 0x02;

/// 값 타입의 안정 tag. `TypedValue` type tag(III.3.5)와 같은 번호입니다.
pub const fn value_type_tag(value_type: ExpressionType) -> u8 {
    match value_type {
        ExpressionType::Boolean => 0x01,
        ExpressionType::Number => 0x02,
        ExpressionType::String => 0x03,
        ExpressionType::Enum(_) => 0x04,
        ExpressionType::Length => 0x05,
        ExpressionType::Percentage => 0x06,
        ExpressionType::Angle => 0x07,
        ExpressionType::Duration => 0x08,
        ExpressionType::Color => 0x09,
        ExpressionType::Gradient => 0x0A,
        ExpressionType::Shadow => 0x0B,
        ExpressionType::Typography => 0x0C,
        ExpressionType::ResourceRef => 0x0D,
    }
}

impl ExprNode {
    /// node를 binary로 직렬화합니다(VIII.2.2).
    pub fn to_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        write_node(&mut out, self)?;
        Ok(out)
    }

    /// [`ExprNode::to_bytes`]의 역. 입력 전체를 정확히 소비해야 합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<ExprNode, CodecError> {
        let mut reader = Reader::new(bytes);
        let node = read_node(&mut reader)?;
        reader.finish()?;
        Ok(node)
    }
}

/// canonical AST 형식 버전(VIII.2.5).
pub const AST_FORMAT_VERSION: u16 = 1;

impl ExpressionAst {
    /// canonical 순서: root에서 child slot 순서로 전위 순회하며 처음 방문한 순서(VIII.2.5).
    ///
    /// 공유 node는 한 번만 나타납니다. 명시적 stack을 쓰므로 깊은 AST에서도 재귀하지 않습니다.
    pub fn canonical_order(&self) -> Vec<ExprNodeId> {
        let mut order = Vec::with_capacity(self.len());
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![self.root()];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            order.push(id);
            if let Some(node) = self.node(id) {
                stack.extend(
                    node.children()
                        .iter()
                        .rev()
                        .map(|child| child.target)
                        .filter(|target| !seen.contains(target)),
                );
            }
        }
        order
    }

    /// canonical AST bytes(VIII.2.5). 삽입·연결 순서와 무관하게 같은 AST는 같은 bytes입니다.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        out.extend_from_slice(&AST_FORMAT_VERSION.to_le_bytes());
        let order = self.canonical_order();
        write_len(&mut out, order.len())?;
        for id in order {
            if let Some(node) = self.node(id) {
                write_node(&mut out, node)?;
            }
        }
        Ok(out)
    }

    /// [`ExpressionAst::to_canonical_bytes`]의 역(VIII.2.5).
    ///
    /// 각 node는 닫힌 연산 집합·payload 규칙으로 디코드하고(VIII.2.2), 전체는 AST 구조 검증(ID 유일성,
    /// slot 구성, 도달성, cycle 금지)을 거칩니다. 다시 직렬화한 bytes가 입력과 다르면(비-canonical node
    /// 순서) 거부합니다.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<ExpressionAst, CodecError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u16()?;
        if version != AST_FORMAT_VERSION {
            return Err(CodecError::UnsupportedVersion(version));
        }
        let count = reader.u32()?;
        if count == 0 {
            return Err(CodecError::EmptyAst);
        }
        let mut nodes = Vec::new();
        for _ in 0..count {
            nodes.push(read_node(&mut reader)?);
        }
        reader.finish()?;
        let root = nodes
            .first()
            .map(ExprNode::id)
            .ok_or(CodecError::EmptyAst)?;
        let ast = ExpressionAst::new(root, nodes).map_err(CodecError::Ast)?;
        if ast.to_canonical_bytes()? != bytes {
            return Err(CodecError::NonCanonical);
        }
        Ok(ast)
    }
}

/// node 하나를 기록합니다.
pub(crate) fn write_node(out: &mut Vec<u8>, node: &ExprNode) -> Result<(), CodecError> {
    out.extend_from_slice(&node.id().stable_id().to_bytes());
    out.extend_from_slice(&node.op().encode());
    match node.payload() {
        NodePayload::Literal(Literal::Value(value)) => {
            out.push(LITERAL_VALUE);
            write_blob(out, &value.to_bytes())?;
        }
        NodePayload::Literal(Literal::Null(value_type)) => {
            out.push(LITERAL_NULL);
            write_type(out, *value_type);
        }
        NodePayload::Reference(target) => write_reference(out, target)?,
        NodePayload::Match(patterns) => {
            write_len(out, patterns.len())?;
            for pattern in patterns {
                write_blob(out, &pattern.to_bytes())?;
            }
        }
        NodePayload::Unary(_) | NodePayload::Binary(_) | NodePayload::Call(_) | NodePayload::If => {
        }
    }
    write_len(out, node.children().len())?;
    for child in node.children() {
        out.push(child.slot.tag());
        out.extend_from_slice(&child.slot.index().to_le_bytes());
        out.extend_from_slice(&child.target.stable_id().to_bytes());
    }
    Ok(())
}

/// node 하나를 읽습니다.
pub(crate) fn read_node(reader: &mut Reader<'_>) -> Result<ExprNode, CodecError> {
    let id = ExprNodeId::new(reader.stable_id()?);
    let op =
        ExpressionOp::decode(reader.take(ExpressionOp::ENCODED_LEN)?).map_err(CodecError::Op)?;
    let payload = match op {
        ExpressionOp::Literal => match reader.u8()? {
            LITERAL_VALUE => NodePayload::Literal(Literal::Value(read_value(reader)?)),
            LITERAL_NULL => NodePayload::Literal(Literal::Null(read_type(reader)?)),
            other => return Err(CodecError::UnknownLiteralTag(other)),
        },
        ExpressionOp::Reference => NodePayload::Reference(read_reference(reader)?),
        ExpressionOp::Unary(op) => NodePayload::Unary(op),
        ExpressionOp::Binary(op) => NodePayload::Binary(op),
        ExpressionOp::Call(function) => NodePayload::Call(function),
        ExpressionOp::If => NodePayload::If,
        ExpressionOp::Match => {
            let count = reader.u32()?;
            let mut patterns = Vec::new();
            for _ in 0..count {
                patterns.push(read_value(reader)?);
            }
            NodePayload::Match(patterns)
        }
    };
    let child_count = reader.u32()?;
    let mut children: Vec<ChildRef> = Vec::new();
    for _ in 0..child_count {
        let tag = reader.u8()?;
        let index = reader.u16()?;
        let slot =
            ChildSlot::from_parts(tag, index).ok_or(CodecError::UnknownSlot { tag, index })?;
        let target = ExprNodeId::new(reader.stable_id()?);
        if children.last().is_some_and(|last| last.slot >= slot) {
            return Err(CodecError::NonCanonical);
        }
        children.push(ChildRef::new(slot, target));
    }
    Ok(ExprNode::new(id, payload, children))
}

fn write_type(out: &mut Vec<u8>, value_type: ExpressionType) {
    out.push(value_type_tag(value_type));
    if let ExpressionType::Enum(enum_type) = value_type {
        out.extend_from_slice(&enum_type.raw().to_le_bytes());
    }
}

fn read_type(reader: &mut Reader<'_>) -> Result<ExpressionType, CodecError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0x01 => ExpressionType::Boolean,
        0x02 => ExpressionType::Number,
        0x03 => ExpressionType::String,
        0x04 => ExpressionType::Enum(EnumTypeId::new(reader.u32()?)),
        0x05 => ExpressionType::Length,
        0x06 => ExpressionType::Percentage,
        0x07 => ExpressionType::Angle,
        0x08 => ExpressionType::Duration,
        0x09 => ExpressionType::Color,
        0x0A => ExpressionType::Gradient,
        0x0B => ExpressionType::Shadow,
        0x0C => ExpressionType::Typography,
        0x0D => ExpressionType::ResourceRef,
        other => return Err(CodecError::UnknownTypeTag(other)),
    })
}

fn write_reference(out: &mut Vec<u8>, target: &ReferenceTarget) -> Result<(), CodecError> {
    out.push(target.tag());
    match target {
        ReferenceTarget::Token(token) => out.extend_from_slice(&token.id().to_bytes()),
        ReferenceTarget::Property(path) => write_blob(out, &path.to_canonical_bytes())?,
        ReferenceTarget::ComponentProperty(id) => out.extend_from_slice(&id.to_bytes()),
        ReferenceTarget::Environment(key) => out.extend_from_slice(&key.raw().to_le_bytes()),
    }
    Ok(())
}

fn read_reference(reader: &mut Reader<'_>) -> Result<ReferenceTarget, CodecError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0x01 => ReferenceTarget::Token(TokenRef::new(reader.stable_id()?)),
        0x02 => ReferenceTarget::Property(decode_property_path(reader.blob()?)?),
        0x03 => ReferenceTarget::ComponentProperty(reader.stable_id()?),
        0x04 => ReferenceTarget::Environment(EnvironmentKey::new(reader.u32()?)),
        other => return Err(CodecError::UnknownReferenceKind(other)),
    })
}

/// 값 blob을 읽습니다. 다시 직렬화한 bytes가 입력과 같아야 합니다(예: `-0.0` 거부).
fn read_value(reader: &mut Reader<'_>) -> Result<TypedValue, CodecError> {
    let blob = reader.blob()?;
    let value = TypedValue::from_bytes(blob).map_err(CodecError::Value)?;
    if value.to_bytes() != blob {
        return Err(CodecError::NonCanonical);
    }
    Ok(value)
}

/// `PropertyPath::to_canonical_bytes`(III.2.2)의 역. 다시 직렬화한 bytes가 입력과 같아야 합니다.
fn decode_property_path(bytes: &[u8]) -> Result<PropertyPath, CodecError> {
    let invalid = |_| CodecError::InvalidPropertyPath;
    let mut reader = Reader::new(bytes);
    let root = reader.stable_id().map_err(invalid)?;
    let key_count = reader.u32_be().map_err(invalid)?;
    let mut keys = Vec::new();
    for _ in 0..key_count {
        keys.push(PropertyKey(reader.u32_be().map_err(invalid)?));
    }
    let mut path = PropertyPath::with_keys(root, keys);
    match reader.u8().map_err(invalid)? {
        0 => {}
        1 => path = path.with_index(reader.u32_be().map_err(invalid)?),
        2 => {
            let len = reader.u32_be().map_err(invalid)?;
            let raw = reader
                .take(usize::try_from(len).map_err(|_| CodecError::InvalidPropertyPath)?)
                .map_err(invalid)?;
            let key = std::str::from_utf8(raw).map_err(|_| CodecError::InvalidPropertyPath)?;
            path = path.with_key_segment(key);
        }
        _ => return Err(CodecError::InvalidPropertyPath),
    }
    reader.finish().map_err(invalid)?;
    if path.to_canonical_bytes() != bytes {
        return Err(CodecError::InvalidPropertyPath);
    }
    Ok(path)
}

/// `u32` LE 길이를 기록합니다.
pub(crate) fn write_len(out: &mut Vec<u8>, len: usize) -> Result<(), CodecError> {
    let len = u32::try_from(len).map_err(|_| CodecError::LengthOverflow)?;
    out.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn write_blob(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CodecError> {
    write_len(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

/// 슬라이스 앞에서부터 읽는 커서. 경계를 넘으면 `Truncated`.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u32_be(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn stable_id(&mut self) -> Result<StableId, CodecError> {
        Ok(StableId::from_bytes(self.array()?))
    }

    /// `u32` LE 길이 프리픽스 blob. 길이가 남은 입력보다 길면 할당 없이 `Truncated`.
    fn blob(&mut self) -> Result<&'a [u8], CodecError> {
        let len = self.u32()?;
        let len = usize::try_from(len).map_err(|_| CodecError::Truncated)?;
        self.take(len)
    }

    pub(crate) fn finish(&self) -> Result<(), CodecError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::EnumValue;
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(1_700_000_000_000 + n, 0x0ABC, n))
    }

    fn number(value: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(value).expect("유한"))
    }

    /// 7개 node 종류와 payload 변형을 모두 덮는 fixture.
    fn fixtures() -> Vec<ExprNode> {
        let path = PropertyPath::with_keys(StableId::new_v7(9, 1, 1), vec![PropertyKey(3)])
            .with_key_segment("키");
        let indexed = PropertyPath::new(StableId::new_v7(9, 1, 2)).with_index(4);
        vec![
            ExprNode::literal(id(1), number(-2.5)),
            ExprNode::literal(id(2), TypedValue::String("텍스트".to_string())),
            ExprNode::literal(
                id(3),
                TypedValue::Color(
                    Color::new(ColorSpace::DisplayP3, 1.0, 0.5, 0.0, 0.25).expect("색"),
                ),
            ),
            ExprNode::literal(id(4), TypedValue::Length(Length::new(8.0).expect("유한"))),
            ExprNode::null(id(5), ExpressionType::Length),
            ExprNode::null(id(6), ExpressionType::Enum(EnumTypeId::new(0xDEAD_BEEF))),
            ExprNode::reference(
                id(7),
                ReferenceTarget::Token(TokenRef::new(StableId::new_v7(1, 2, 3))),
            ),
            ExprNode::reference(id(8), ReferenceTarget::Property(path)),
            ExprNode::reference(id(9), ReferenceTarget::Property(indexed)),
            ExprNode::reference(
                id(10),
                ReferenceTarget::ComponentProperty(StableId::new_v7(4, 5, 6)),
            ),
            ExprNode::reference(id(11), ReferenceTarget::Environment(EnvironmentKey::new(7))),
            ExprNode::unary(id(12), UnaryOp::Negate, id(1)),
            ExprNode::unary(id(13), UnaryOp::Not, id(14)),
            ExprNode::binary(id(15), BinaryOp::Add, id(4), id(4)),
            ExprNode::binary(id(16), BinaryOp::GreaterEqual, id(1), id(12)),
            ExprNode::call(
                id(17),
                FunctionId::Remap,
                &[id(1), id(2), id(3), id(4), id(5)],
            ),
            ExprNode::call(id(18), FunctionId::ToSrgb, &[id(3)]),
            ExprNode::if_else(id(19), id(16), id(1), id(12)),
            ExprNode::match_on(
                id(20),
                id(7),
                vec![
                    (TypedValue::Enum(EnumValue::new(2, 0)), id(1)),
                    (TypedValue::Enum(EnumValue::new(2, 1)), id(12)),
                ],
                id(1),
            ),
            ExprNode::match_on(
                id(21),
                id(2),
                vec![(TypedValue::String(String::new()), id(1))],
                id(1),
            ),
        ]
    }

    #[test]
    fn each_node_kind_round_trips() {
        let nodes = fixtures();
        let kinds: std::collections::BTreeSet<_> = nodes.iter().map(ExprNode::kind).collect();
        assert_eq!(kinds.len(), 7, "7개 node 종류 모두 포함");
        for node in nodes {
            let bytes = node.to_bytes().expect("직렬화");
            let decoded = ExprNode::from_bytes(&bytes).expect("역직렬화");
            assert_eq!(decoded, node, "{:?}", node.kind());
            // 재직렬화도 동일 bytes(결정론).
            assert_eq!(decoded.to_bytes().expect("재직렬화"), bytes);
        }
    }

    #[test]
    fn layout_golden() {
        let node = ExprNode::binary(id(15), BinaryOp::Add, id(4), id(4));
        let bytes = node.to_bytes().expect("직렬화");
        assert_eq!(&bytes[..16], &id(15).stable_id().to_bytes());
        assert_eq!(&bytes[16..19], &[0x04, 0x01, 0x00], "Binary(+)");
        assert_eq!(&bytes[19..23], &[2, 0, 0, 0], "child 2개(LE)");
        assert_eq!(&bytes[23..26], &[0x02, 0x00, 0x00], "Lhs");
        assert_eq!(&bytes[42..45], &[0x03, 0x00, 0x00], "Rhs");
        assert_eq!(bytes.len(), 16 + 3 + 4 + 2 * 19);
        let null = ExprNode::null(id(6), ExpressionType::Enum(EnumTypeId::new(1)));
        let bytes = null.to_bytes().expect("직렬화");
        assert_eq!(
            &bytes[16..],
            &[0x01, 0, 0, 0x02, 0x04, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn value_type_tags_match_typed_value_tags() {
        let samples = [
            TypedValue::Boolean(false),
            number(1.0),
            TypedValue::String(String::new()),
            TypedValue::Enum(EnumValue::new(3, 1)),
            TypedValue::Length(Length::new(1.0).expect("유한")),
        ];
        for value in samples {
            assert_eq!(value_type_tag(ExpressionType::of(&value)), value.type_tag());
        }
    }

    #[test]
    fn every_truncation_is_an_error() {
        for node in fixtures() {
            let bytes = node.to_bytes().expect("직렬화");
            for cut in 0..bytes.len() {
                assert!(
                    ExprNode::from_bytes(&bytes[..cut]).is_err(),
                    "{cut}바이트에서 끊김"
                );
            }
            let mut trailing = bytes.clone();
            trailing.push(0);
            assert_eq!(
                ExprNode::from_bytes(&trailing),
                Err(CodecError::TrailingBytes)
            );
        }
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        let literal = ExprNode::literal(id(1), number(1.0))
            .to_bytes()
            .expect("직렬화");
        let mut bad_tag = literal.clone();
        bad_tag[19] = 0x07;
        assert_eq!(
            ExprNode::from_bytes(&bad_tag),
            Err(CodecError::UnknownLiteralTag(0x07))
        );

        let reference =
            ExprNode::reference(id(1), ReferenceTarget::Environment(EnvironmentKey::new(1)))
                .to_bytes()
                .expect("직렬화");
        let mut bad_kind = reference.clone();
        bad_kind[19] = 0x09;
        assert_eq!(
            ExprNode::from_bytes(&bad_kind),
            Err(CodecError::UnknownReferenceKind(0x09))
        );

        let null = ExprNode::null(id(1), ExpressionType::Length)
            .to_bytes()
            .expect("직렬화");
        let mut bad_type = null.clone();
        bad_type[20] = 0x0E;
        assert_eq!(
            ExprNode::from_bytes(&bad_type),
            Err(CodecError::UnknownTypeTag(0x0E))
        );

        // 미지 연산(VIII.1.5 디코더 재사용).
        let mut bad_op = literal.clone();
        bad_op[16] = 0x08;
        assert_eq!(
            ExprNode::from_bytes(&bad_op),
            Err(CodecError::Op(ClosedOpError::UnknownNodeKind(0x08)))
        );

        // child slot 역순.
        let mut reversed = ExprNode::binary(id(15), BinaryOp::Add, id(4), id(5))
            .to_bytes()
            .expect("직렬화");
        let (lhs, rhs) = (reversed[23..42].to_vec(), reversed[42..61].to_vec());
        reversed[23..42].copy_from_slice(&rhs);
        reversed[42..61].copy_from_slice(&lhs);
        assert_eq!(
            ExprNode::from_bytes(&reversed),
            Err(CodecError::NonCanonical)
        );

        // 미지 slot tag.
        let mut bad_slot = ExprNode::unary(id(12), UnaryOp::Not, id(1))
            .to_bytes()
            .expect("직렬화");
        bad_slot[23] = 0x0B;
        assert_eq!(
            ExprNode::from_bytes(&bad_slot),
            Err(CodecError::UnknownSlot {
                tag: 0x0B,
                index: 0
            })
        );

        // 과대 길이 프리픽스는 할당 없이 Truncated.
        let mut huge = literal.clone();
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(ExprNode::from_bytes(&huge), Err(CodecError::Truncated));

        // 리터럴 값의 -0.0은 비-canonical(정규화하면 bytes가 달라짐).
        let mut negative_zero = literal.clone();
        negative_zero[25..29].copy_from_slice(&(-0.0f32).to_le_bytes());
        assert_eq!(
            ExprNode::from_bytes(&negative_zero),
            Err(CodecError::NonCanonical)
        );

        // 리터럴 값의 비유한 float.
        let mut nan = literal;
        nan[25..29].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            ExprNode::from_bytes(&nan),
            Err(CodecError::Value(ValueDecodeError::NonFiniteFloat))
        );
    }

    #[test]
    fn byte_mutations_never_panic() {
        // 결정적 mutation(SplitMix64 유사): 모든 fixture의 임의 byte를 바꿔도 panic 없이 Ok/Err.
        let mut state: u64 = 0x5EED_1234;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for node in fixtures() {
            let bytes = node.to_bytes().expect("직렬화");
            for _ in 0..200 {
                let mut mutated = bytes.clone();
                let position = (next() % mutated.len() as u64) as usize;
                mutated[position] = (next() & 0xFF) as u8;
                if let Ok(decoded) = ExprNode::from_bytes(&mutated) {
                    // 성공했다면 canonical: 재직렬화가 입력과 같아야 함.
                    assert_eq!(decoded.to_bytes().expect("재직렬화"), mutated);
                }
            }
        }
    }
}

#[cfg(test)]
mod ast_tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::EnumValue;
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn number(value: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(value).expect("유한"))
    }

    fn color(r: f32) -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, r, 0.2, 0.3, 1.0).expect("색"))
    }

    /// if/and/less/match/call과 공유 node(11, 13, 14)를 포함한 AST의 node 목록(생성 순서 임의).
    fn fixture_nodes() -> Vec<ExprNode> {
        let token_len = ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xA1; 16])));
        let token_enum = ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xA2; 16])));
        vec![
            ExprNode::if_else(id(1), id(2), id(3), id(4)),
            ExprNode::binary(id(2), BinaryOp::And, id(5), id(6)),
            ExprNode::binary(id(5), BinaryOp::Less, id(7), id(8)),
            ExprNode::reference(id(7), token_len),
            ExprNode::literal(id(8), TypedValue::Length(Length::new(100.0).expect("유한"))),
            ExprNode::reference(id(6), ReferenceTarget::Environment(EnvironmentKey::new(1))),
            ExprNode::match_on(
                id(3),
                id(9),
                vec![
                    (TypedValue::Enum(EnumValue::new(5, 0)), id(10)),
                    (TypedValue::Enum(EnumValue::new(5, 1)), id(11)),
                ],
                id(12),
            ),
            ExprNode::reference(id(9), token_enum),
            ExprNode::call(id(10), FunctionId::Mix, &[id(11), id(13), id(14)]),
            ExprNode::literal(id(11), color(0.1)),
            ExprNode::literal(id(13), color(0.9)),
            ExprNode::literal(id(14), number(0.5)),
            ExprNode::call(id(12), FunctionId::WithAlpha, &[id(13), id(14)]),
            ExprNode::call(id(4), FunctionId::ToSrgb, &[id(11)]),
        ]
    }

    fn fixture() -> ExpressionAst {
        ExpressionAst::new(id(1), fixture_nodes()).expect("유효한 AST")
    }

    /// 결정적 PRNG(SplitMix64).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn shuffle<T>(&mut self, items: &mut [T]) {
            for i in (1..items.len()).rev() {
                let j = (self.next() % (i as u64 + 1)) as usize;
                items.swap(i, j);
            }
        }
    }

    #[test]
    fn gui_creation_order_does_not_change_bytes() {
        let expected = fixture().to_canonical_bytes().expect("직렬화");
        let mut rng = Rng(0x0005_EED5);
        let mut distinct_orders = std::collections::BTreeSet::new();
        for _ in 0..1_000 {
            // node 생성(삽입) 순서와 각 node의 child 연결 순서를 모두 섞습니다.
            let mut nodes: Vec<ExprNode> = fixture_nodes()
                .into_iter()
                .map(|node| {
                    let mut children = node.children().to_vec();
                    rng.shuffle(&mut children);
                    ExprNode::new(node.id(), node.payload().clone(), children)
                })
                .collect();
            rng.shuffle(&mut nodes);
            distinct_orders.insert(nodes.iter().map(ExprNode::id).collect::<Vec<_>>());
            let ast = ExpressionAst::new(id(1), nodes).expect("유효한 AST");
            assert_eq!(ast.to_canonical_bytes().expect("직렬화"), expected);
        }
        assert!(
            distinct_orders.len() > 900,
            "실제로 서로 다른 생성 순서를 시험"
        );
    }

    #[test]
    fn canonical_order_follows_child_slots() {
        let order = fixture().canonical_order();
        // 전위 slot 순서: if → cond(and → less → token, 100px → env) → then(match → scrutinee →
        // arm0(mix → c1 → c2 → 0.5) → arm1(c1 공유, 생략) → default(with_alpha)) → else(to_srgb).
        let expected: Vec<ExprNodeId> = [1, 2, 5, 7, 8, 6, 3, 9, 10, 11, 13, 14, 12, 4]
            .into_iter()
            .map(id)
            .collect();
        assert_eq!(order, expected);
    }

    #[test]
    fn canonical_bytes_golden() {
        // 1 + 2. 기대 bytes는 encoder를 쓰지 않고 형식 정의에서 직접 조립합니다.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::literal(id(3), number(2.0)),
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                ExprNode::literal(id(2), number(1.0)),
            ],
        )
        .expect("유효");
        let mut expected = vec![0x01, 0x00, 0x03, 0x00, 0x00, 0x00];
        expected.extend([0x01; 16]);
        expected.extend([0x04, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00]);
        expected.extend([0x02, 0x00, 0x00]);
        expected.extend([0x02; 16]);
        expected.extend([0x03, 0x00, 0x00]);
        expected.extend([0x03; 16]);
        expected.extend([0x02; 16]);
        expected.extend([0x01, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00, 0x00]);
        expected.extend([0x02, 0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x00]);
        expected.extend([0x03; 16]);
        expected.extend([0x01, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00, 0x00]);
        expected.extend([0x02, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(ast.to_canonical_bytes().expect("직렬화"), expected);
    }

    #[test]
    fn canonical_bytes_round_trip() {
        let ast = fixture();
        let bytes = ast.to_canonical_bytes().expect("직렬화");
        let decoded = ExpressionAst::from_canonical_bytes(&bytes).expect("역직렬화");
        assert_eq!(decoded, ast);
        assert_eq!(decoded.canonical_order(), ast.canonical_order());
        assert_eq!(decoded.to_canonical_bytes().expect("재직렬화"), bytes);
    }

    #[test]
    fn different_asts_have_different_bytes() {
        let base = fixture().to_canonical_bytes().expect("직렬화");
        // 연산자 하나만 바꿈(and → or).
        let changed: Vec<ExprNode> = fixture_nodes()
            .into_iter()
            .map(|node| {
                if node.id() == id(2) {
                    ExprNode::binary(id(2), BinaryOp::Or, id(5), id(6))
                } else {
                    node
                }
            })
            .collect();
        let changed = ExpressionAst::new(id(1), changed).expect("유효");
        assert_ne!(changed.to_canonical_bytes().expect("직렬화"), base);
        // Lhs/Rhs 교환은 다른 의미 → 다른 bytes.
        let a = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Subtract, id(2), id(3)),
                ExprNode::literal(id(2), number(1.0)),
                ExprNode::literal(id(3), number(2.0)),
            ],
        )
        .expect("유효");
        let b = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Subtract, id(3), id(2)),
                ExprNode::literal(id(2), number(1.0)),
                ExprNode::literal(id(3), number(2.0)),
            ],
        )
        .expect("유효");
        assert_ne!(
            a.to_canonical_bytes().expect("직렬화"),
            b.to_canonical_bytes().expect("직렬화")
        );
    }

    /// node 기록 순서를 직접 정해 AST bytes를 조립합니다(비-canonical·악성 입력 생성용).
    fn raw_ast(nodes: &[ExprNode]) -> Vec<u8> {
        let mut out = AST_FORMAT_VERSION.to_le_bytes().to_vec();
        out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        for node in nodes {
            out.extend(node.to_bytes().expect("직렬화"));
        }
        out
    }

    #[test]
    fn non_canonical_and_malicious_bytes_are_rejected() {
        let add = ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3));
        let one = ExprNode::literal(id(2), number(1.0));
        let two = ExprNode::literal(id(3), number(2.0));
        // 정상 순서는 통과.
        assert!(ExpressionAst::from_canonical_bytes(&raw_ast(&[
            add.clone(),
            one.clone(),
            two.clone()
        ]))
        .is_ok());
        // Rhs를 Lhs보다 먼저 기록 → 비-canonical.
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&raw_ast(&[add.clone(), two.clone(), one.clone()])),
            Err(CodecError::NonCanonical)
        );
        // AST 안의 미지 연산(가상 반복 kind 0x08).
        let mut malicious = raw_ast(&[add.clone(), one.clone(), two.clone()]);
        malicious[6 + 16] = 0x08;
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&malicious),
            Err(CodecError::Op(ClosedOpError::UnknownNodeKind(0x08)))
        );
        // AST 안의 미등록 함수 ID.
        let call = ExprNode::call(id(1), FunctionId::Abs, &[id(2)]);
        let mut unknown_fn = raw_ast(&[call, one.clone()]);
        unknown_fn[6 + 17..6 + 19].copy_from_slice(&0x0F00u16.to_le_bytes());
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&unknown_fn),
            Err(CodecError::Op(ClosedOpError::UnknownFunction(0x0F00)))
        );
        // cycle bytes.
        let cyclic = raw_ast(&[
            ExprNode::unary(id(1), UnaryOp::Negate, id(2)),
            ExprNode::unary(id(2), UnaryOp::Negate, id(1)),
        ]);
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&cyclic),
            Err(CodecError::Ast(AstError::Cycle {
                path: vec![id(1), id(2), id(1)]
            }))
        );
        // 중복 node ID.
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&raw_ast(&[add.clone(), one.clone(), one.clone()])),
            Err(CodecError::Ast(AstError::DuplicateNodeId(id(2))))
        );
        // 버전·빈 AST·잉여·절단.
        let mut version = raw_ast(std::slice::from_ref(&one));
        version[0] = 2;
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&version),
            Err(CodecError::UnsupportedVersion(2))
        );
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&raw_ast(&[])),
            Err(CodecError::EmptyAst)
        );
        let mut trailing = raw_ast(std::slice::from_ref(&one));
        trailing.push(0);
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&trailing),
            Err(CodecError::TrailingBytes)
        );
        let full = fixture().to_canonical_bytes().expect("직렬화");
        for cut in 0..full.len() {
            assert!(ExpressionAst::from_canonical_bytes(&full[..cut]).is_err());
        }
        // node 수를 과장해도 할당 없이 Truncated.
        let mut inflated = raw_ast(&[one]);
        inflated[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            ExpressionAst::from_canonical_bytes(&inflated),
            Err(CodecError::Truncated)
        );
    }

    #[test]
    fn deep_ast_serializes_without_recursion() {
        let depth: u64 = 100_000;
        let node_id = |n: u64| ExprNodeId::new(StableId::new_v7(n, 0, n));
        let mut nodes: Vec<ExprNode> = (1..depth)
            .map(|n| ExprNode::unary(node_id(n), UnaryOp::Negate, node_id(n + 1)))
            .collect();
        nodes.push(ExprNode::literal(node_id(depth), number(1.0)));
        let ast = ExpressionAst::new(node_id(1), nodes).expect("유효");
        let bytes = ast.to_canonical_bytes().expect("직렬화");
        let decoded = ExpressionAst::from_canonical_bytes(&bytes).expect("역직렬화");
        assert_eq!(decoded.len(), depth as usize);
    }
}
