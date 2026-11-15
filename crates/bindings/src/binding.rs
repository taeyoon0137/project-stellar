//! Value Binding (VIII.6.1).
//!
//! [`ValueBinding`]은 속성 하나의 값을 표현식으로 정합니다(아키텍처 §14.2 통합 Value Binding).
//!
//! - **target**: 값을 받는 속성 [`PropertyPath`](StableId + PropertyKey 경로).
//! - **source**: 값을 계산하는 표현식 AST. root node가 source expression root입니다.
//! - **expected type**: target 속성의 선언 타입([`TypeSpec`]). source 결과 타입은 이 타입에 대입 가능해야 합니다
//!   ([`assignable`]: `Required(T)` ← `Required(T)`, `Optional(T)` ← `Required(T)`·`Optional(T)`).
//! - **scope owner**: binding을 소유하는 node 또는 ComponentDefinition. Property Drilling 경계(§14.3)의 기준입니다.
//!
//! binding은 canonical binary 형식으로 round-trip합니다(ADR-0026). binding이 직접 쓰는 정수(version, blob 길이,
//! `enum_type`)는 V.1.1 규칙(little-endian, `u32` LE 길이 프리픽스)을 따릅니다. 담는 하위 bytes는 다시 인코딩하지
//! 않고 각 형식 그대로 둡니다. `PropertyPath` canonical bytes의 내부 정수는 big-endian이고, AST literal 안
//! `TypedValue` bytes는 정수(길이 프리픽스·Enum 필드)가 big-endian, `f32` payload가 little-endian입니다.
//!
//! ```text
//! binding   := version:u16 (= 1) ++ id[16] ++ blob(target) ++ type_spec ++ owner ++ blob(source)
//! target    := PropertyPath canonical bytes (III.2.2)
//! type_spec := nullable:u8 (0 = Required, 1 = Optional) ++ value_tag:u8 (0x01~0x0D) [++ enum_type:u32 (Enum)]
//! owner     := kind:u8 (0x01 = node, 0x02 = component) ++ id[16]
//! source    := canonical AST bytes (VIII.2.5)
//! blob      := len:u32 ++ bytes
//! ```
//!
//! 디코더는 미지 tag·잘림·잉여 bytes를 거부하고, 다시 인코딩한 bytes가 입력과 같을 때만 받아들입니다(한 binding에 한
//! 표현). 길이 프리픽스는 남은 입력보다 길면 할당 전에 거부합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_expressions::ast::{ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::codec::{value_type_tag, CodecError};
use stellar_expressions::typecheck::{type_check, ReferenceTypes, TypeError};
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

/// binding 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingId(StableId);

impl BindingId {
    /// `StableId`로 만듭니다.
    pub const fn new(id: StableId) -> BindingId {
        BindingId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

impl fmt::Display for BindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// binding 소유자(scope owner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeOwner {
    /// 일반 node(Frame·Fragment 등)가 소유합니다.
    Node(NodeRef),
    /// ComponentDefinition이 소유합니다(Property Drilling 경계).
    Component(ComponentRef),
}

impl ScopeOwner {
    const fn tag(&self) -> u8 {
        match self {
            ScopeOwner::Node(_) => 0x01,
            ScopeOwner::Component(_) => 0x02,
        }
    }

    const fn id(&self) -> StableId {
        match self {
            ScopeOwner::Node(node) => node.id(),
            ScopeOwner::Component(component) => component.id(),
        }
    }
}

/// `actual` 결과를 `expected` 위치에 둘 수 있는지 여부.
pub fn assignable(expected: TypeSpec, actual: TypeSpec) -> bool {
    match (expected, actual) {
        (TypeSpec::Required(e), TypeSpec::Required(a)) => e == a,
        (TypeSpec::Optional(e), a) => e.inner() == a.value_type(),
        (TypeSpec::Required(_), TypeSpec::Optional(_)) => false,
    }
}

/// binding type 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindingTypeError {
    /// source 표현식 안의 type 오류(오류 node StableId 포함).
    Expression(Vec<TypeError>),
    /// source 결과 타입을 target 타입에 둘 수 없습니다.
    Mismatch {
        /// target 기대 타입.
        expected: TypeSpec,
        /// source 결과 타입.
        actual: TypeSpec,
    },
}

impl fmt::Display for BindingTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingTypeError::Expression(errors) => {
                write!(f, "source 표현식 type 오류 {}개", errors.len())
            }
            BindingTypeError::Mismatch { expected, actual } => {
                write!(f, "source 결과 {actual}를 {expected} 속성에 둘 수 없습니다")
            }
        }
    }
}

impl std::error::Error for BindingTypeError {}

/// binding 직렬화 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingCodecError {
    /// 입력이 도중에 끊겼거나 길이 프리픽스가 남은 입력보다 깁니다.
    Truncated,
    /// 잉여 bytes.
    TrailingBytes,
    /// 길이가 `u32`를 넘습니다.
    LengthOverflow,
    /// 지원하지 않는 형식 버전.
    UnsupportedVersion(u16),
    /// 미지 nullable flag.
    UnknownNullability(u8),
    /// 미지 값 타입 tag.
    UnknownTypeTag(u8),
    /// 미지 owner 종류.
    UnknownOwner(u8),
    /// target PropertyPath bytes가 canonical이 아닙니다.
    InvalidTarget,
    /// source AST 디코드 실패.
    Source(CodecError),
    /// 다시 인코딩한 bytes가 입력과 다릅니다.
    NonCanonical,
}

impl fmt::Display for BindingCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingCodecError::Truncated => f.write_str("입력이 도중에 끊겼습니다"),
            BindingCodecError::TrailingBytes => f.write_str("잉여 bytes가 남았습니다"),
            BindingCodecError::LengthOverflow => f.write_str("길이가 u32 범위를 넘습니다"),
            BindingCodecError::UnsupportedVersion(v) => {
                write!(f, "지원하지 않는 binding 형식 버전: {v}")
            }
            BindingCodecError::UnknownNullability(v) => write!(f, "미지 nullable flag: 0x{v:02X}"),
            BindingCodecError::UnknownTypeTag(v) => write!(f, "미지 값 타입 tag: 0x{v:02X}"),
            BindingCodecError::UnknownOwner(v) => write!(f, "미지 owner 종류: 0x{v:02X}"),
            BindingCodecError::InvalidTarget => {
                f.write_str("target PropertyPath가 canonical이 아닙니다")
            }
            BindingCodecError::Source(error) => write!(f, "source AST: {error}"),
            BindingCodecError::NonCanonical => f.write_str("canonical 형식이 아닙니다"),
        }
    }
}

impl std::error::Error for BindingCodecError {}

/// Value Binding (VIII.6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueBinding {
    id: BindingId,
    target: PropertyPath,
    source: ExpressionAst,
    expected_type: TypeSpec,
    scope_owner: ScopeOwner,
}

/// binding 형식 버전.
pub const BINDING_FORMAT_VERSION: u16 = 1;

impl ValueBinding {
    /// binding을 만듭니다. type 검증은 [`ValueBinding::validate_types`]가 합니다.
    pub fn new(
        id: BindingId,
        target: PropertyPath,
        source: ExpressionAst,
        expected_type: TypeSpec,
        scope_owner: ScopeOwner,
    ) -> ValueBinding {
        ValueBinding {
            id,
            target,
            source,
            expected_type,
            scope_owner,
        }
    }

    /// binding ID.
    pub const fn id(&self) -> BindingId {
        self.id
    }

    /// target 속성 경로.
    pub fn target(&self) -> &PropertyPath {
        &self.target
    }

    /// source 표현식.
    pub fn source(&self) -> &ExpressionAst {
        &self.source
    }

    /// source expression root.
    pub const fn source_root(&self) -> ExprNodeId {
        self.source.root()
    }

    /// target 기대 타입.
    pub const fn expected_type(&self) -> TypeSpec {
        self.expected_type
    }

    /// 소유자.
    pub const fn scope_owner(&self) -> ScopeOwner {
        self.scope_owner
    }

    /// source가 참조하는 대상(중복 제거, 정렬).
    pub fn references(&self) -> BTreeSet<ReferenceTarget> {
        self.source
            .referenced_targets()
            .into_iter()
            .cloned()
            .collect()
    }

    /// source가 `target`을 참조하면 `true`. [`references`](Self::references)와 같은 판정(source AST의 모든 참조
    /// node)이지만 집합을 만들지 않고(할당 없이) 찾는 즉시 멈춥니다. 많은 binding 중 한 대상을 읽는 것만 고를 때
    /// 씁니다(토큰 삭제 inbound 검사).
    pub fn refers_to(&self, target: &ReferenceTarget) -> bool {
        self.source.nodes().iter().any(|node| {
            matches!(node.payload(), stellar_expressions::ast::NodePayload::Reference(found) if found == target)
        })
    }

    /// source 표현식을 type 검사하고 결과를 target 타입에 둘 수 있는지 확인합니다. 성공하면 source 결과 타입입니다.
    pub fn validate_types(
        &self,
        references: &dyn ReferenceTypes,
    ) -> Result<TypeSpec, BindingTypeError> {
        check_source(self.expected_type, &self.source, references)
    }

    /// 같은 ID·target·owner에 source와 expected type을 바꾼 binding.
    pub fn with_source(&self, source: ExpressionAst, expected_type: TypeSpec) -> ValueBinding {
        ValueBinding {
            source,
            expected_type,
            ..self.clone()
        }
    }

    /// canonical bytes로 직렬화합니다.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BindingCodecError> {
        let mut out = Vec::new();
        out.extend_from_slice(&BINDING_FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.id.0.to_bytes());
        write_blob(&mut out, &self.target.to_canonical_bytes())?;
        write_type_spec(&mut out, self.expected_type);
        out.push(self.scope_owner.tag());
        out.extend_from_slice(&self.scope_owner.id().to_bytes());
        let source = self
            .source
            .to_canonical_bytes()
            .map_err(BindingCodecError::Source)?;
        write_blob(&mut out, &source)?;
        Ok(out)
    }

    /// canonical bytes에서 복원합니다. 다시 인코딩한 bytes가 입력과 같아야 합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<ValueBinding, BindingCodecError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u16_le()?;
        if version != BINDING_FORMAT_VERSION {
            return Err(BindingCodecError::UnsupportedVersion(version));
        }
        let id = BindingId(reader.stable_id()?);
        let target_bytes = reader.blob()?;
        let target = decode_property_path(target_bytes)?;
        let expected_type = read_type_spec(&mut reader)?;
        let owner_tag = reader.u8()?;
        let owner_id = reader.stable_id()?;
        let scope_owner = match owner_tag {
            0x01 => ScopeOwner::Node(NodeRef::new(owner_id)),
            0x02 => ScopeOwner::Component(ComponentRef::new(owner_id)),
            other => return Err(BindingCodecError::UnknownOwner(other)),
        };
        let source = ExpressionAst::from_canonical_bytes(reader.blob()?)
            .map_err(BindingCodecError::Source)?;
        reader.finish()?;
        let binding = ValueBinding {
            id,
            target,
            source,
            expected_type,
            scope_owner,
        };
        if binding.to_bytes()? != bytes {
            return Err(BindingCodecError::NonCanonical);
        }
        Ok(binding)
    }
}

/// binding 모음 편집 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindingSetError {
    /// 같은 ID의 binding이 이미 있습니다.
    DuplicateId(BindingId),
    /// 같은 target 속성에 binding이 이미 있습니다(속성 하나에 binding 하나).
    DuplicateTarget {
        /// target 속성.
        target: PropertyPath,
        /// 이미 있는 binding.
        existing: BindingId,
    },
    /// binding이 없습니다.
    UnknownBinding(BindingId),
    /// 교체는 같은 target·owner를 유지해야 합니다.
    TargetChanged(BindingId),
}

impl fmt::Display for BindingSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingSetError::DuplicateId(id) => write!(f, "binding ID가 중복됩니다: {id}"),
            BindingSetError::DuplicateTarget { existing, .. } => {
                write!(f, "target 속성에 binding {existing}이 이미 있습니다")
            }
            BindingSetError::UnknownBinding(id) => write!(f, "binding이 없습니다: {id}"),
            BindingSetError::TargetChanged(id) => {
                write!(f, "binding {id} 교체가 target·owner를 바꿉니다")
            }
        }
    }
}

impl std::error::Error for BindingSetError {}

/// binding 모음(VIII.6.2). ID 색인과 target 색인을 함께 유지하며, 속성 하나에는 binding이 하나만 있습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingSet {
    bindings: BTreeMap<BindingId, ValueBinding>,
    by_target: BTreeMap<PropertyPath, BindingId>,
}

impl BindingSet {
    /// 빈 모음.
    pub fn new() -> BindingSet {
        BindingSet::default()
    }

    /// binding을 추가합니다. ID·target이 이미 있으면 거부합니다.
    pub fn insert(&mut self, binding: ValueBinding) -> Result<(), BindingSetError> {
        if self.bindings.contains_key(&binding.id) {
            return Err(BindingSetError::DuplicateId(binding.id));
        }
        if let Some(existing) = self.by_target.get(&binding.target) {
            return Err(BindingSetError::DuplicateTarget {
                target: binding.target.clone(),
                existing: *existing,
            });
        }
        self.by_target.insert(binding.target.clone(), binding.id);
        self.bindings.insert(binding.id, binding);
        Ok(())
    }

    /// binding을 제거하고 돌려줍니다.
    pub fn remove(&mut self, id: BindingId) -> Option<ValueBinding> {
        let binding = self.bindings.remove(&id)?;
        self.by_target.remove(&binding.target);
        Some(binding)
    }

    /// 같은 target·owner의 binding으로 교체하고 이전 binding을 돌려줍니다.
    pub fn replace(&mut self, binding: ValueBinding) -> Result<ValueBinding, BindingSetError> {
        let current = self
            .bindings
            .get_mut(&binding.id)
            .ok_or(BindingSetError::UnknownBinding(binding.id))?;
        if current.target != binding.target || current.scope_owner != binding.scope_owner {
            return Err(BindingSetError::TargetChanged(binding.id));
        }
        Ok(std::mem::replace(current, binding))
    }

    /// ID로 찾습니다.
    pub fn get(&self, id: BindingId) -> Option<&ValueBinding> {
        self.bindings.get(&id)
    }

    /// target 속성으로 찾습니다.
    pub fn by_target(&self, target: &PropertyPath) -> Option<&ValueBinding> {
        self.by_target
            .get(target)
            .and_then(|id| self.bindings.get(id))
    }

    /// 모든 binding(ID 순).
    pub fn iter(&self) -> impl Iterator<Item = &ValueBinding> {
        self.bindings.values()
    }

    /// binding 수.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// binding이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// `source`가 `expected` 속성의 source로 올바른지 검사합니다. 성공하면 source 결과 타입입니다.
pub fn check_source(
    expected: TypeSpec,
    source: &ExpressionAst,
    references: &dyn ReferenceTypes,
) -> Result<TypeSpec, BindingTypeError> {
    let report = type_check(source, references);
    if !report.is_ok() {
        return Err(BindingTypeError::Expression(report.errors().to_vec()));
    }
    let actual = report
        .root_type()
        .ok_or(BindingTypeError::Expression(Vec::new()))?;
    if assignable(expected, actual) {
        Ok(actual)
    } else {
        Err(BindingTypeError::Mismatch { expected, actual })
    }
}

fn write_blob(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), BindingCodecError> {
    let len = u32::try_from(bytes.len()).map_err(|_| BindingCodecError::LengthOverflow)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_type_spec(out: &mut Vec<u8>, spec: TypeSpec) {
    out.push(u8::from(spec.is_optional()));
    let value_type = spec.value_type();
    out.push(value_type_tag(value_type));
    if let ExpressionType::Enum(id) = value_type {
        out.extend_from_slice(&id.raw().to_le_bytes());
    }
}

fn read_type_spec(reader: &mut Reader<'_>) -> Result<TypeSpec, BindingCodecError> {
    let nullable = reader.u8()?;
    let value_type = match reader.u8()? {
        0x01 => ExpressionType::Boolean,
        0x02 => ExpressionType::Number,
        0x03 => ExpressionType::String,
        0x04 => ExpressionType::Enum(EnumTypeId::new(reader.u32_le()?)),
        0x05 => ExpressionType::Length,
        0x06 => ExpressionType::Percentage,
        0x07 => ExpressionType::Angle,
        0x08 => ExpressionType::Duration,
        0x09 => ExpressionType::Color,
        0x0A => ExpressionType::Gradient,
        0x0B => ExpressionType::Shadow,
        0x0C => ExpressionType::Typography,
        0x0D => ExpressionType::ResourceRef,
        other => return Err(BindingCodecError::UnknownTypeTag(other)),
    };
    match nullable {
        0 => Ok(TypeSpec::required(value_type)),
        1 => Ok(TypeSpec::optional(value_type)),
        other => Err(BindingCodecError::UnknownNullability(other)),
    }
}

/// `PropertyPath::to_canonical_bytes`(III.2.2, 내부 정수는 big-endian)의 역. 다시 직렬화한 bytes가 같아야 합니다.
pub(crate) fn decode_property_path(bytes: &[u8]) -> Result<PropertyPath, BindingCodecError> {
    let invalid = |_| BindingCodecError::InvalidTarget;
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
                .take(usize::try_from(len).map_err(|_| BindingCodecError::InvalidTarget)?)
                .map_err(invalid)?;
            let key = std::str::from_utf8(raw).map_err(|_| BindingCodecError::InvalidTarget)?;
            path = path.with_key_segment(key);
        }
        _ => return Err(BindingCodecError::InvalidTarget),
    }
    reader.finish().map_err(invalid)?;
    if path.to_canonical_bytes() != bytes {
        return Err(BindingCodecError::InvalidTarget);
    }
    Ok(path)
}

/// 앞에서부터 읽는 커서. 경계를 넘으면 `Truncated`.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], BindingCodecError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(BindingCodecError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(BindingCodecError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BindingCodecError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, BindingCodecError> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16_le(&mut self) -> Result<u16, BindingCodecError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32_le(&mut self) -> Result<u32, BindingCodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u32_be(&mut self) -> Result<u32, BindingCodecError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn stable_id(&mut self) -> Result<StableId, BindingCodecError> {
        Ok(StableId::from_bytes(self.array()?))
    }

    /// `u32` LE 길이 프리픽스 blob. 길이가 남은 입력보다 길면 할당 없이 `Truncated`.
    pub(crate) fn blob(&mut self) -> Result<&'a [u8], BindingCodecError> {
        let len = self.u32_le()?;
        let len = usize::try_from(len).map_err(|_| BindingCodecError::Truncated)?;
        self.take(len)
    }

    pub(crate) fn finish(&self) -> Result<(), BindingCodecError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(BindingCodecError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::Length;
    use stellar_expressions::ast::{EnvironmentKey, ExprNode};
    use stellar_expressions::ops::BinaryOp;

    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn node(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn len(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn token(n: u64) -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(sid(1_000 + n)))
    }

    fn scheme() -> ReferenceTarget {
        ReferenceTarget::Environment(EnvironmentKey::new(4))
    }

    /// `match scheme { E0 => token1, E1 => token1 + 2px } else 0px`.
    fn source() -> ExpressionAst {
        ExpressionAst::new(
            node(1),
            vec![
                ExprNode::match_on(
                    node(1),
                    node(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(9, 0)), node(3)),
                        (TypedValue::Enum(EnumValue::new(9, 1)), node(4)),
                    ],
                    node(5),
                ),
                ExprNode::reference(node(2), scheme()),
                ExprNode::reference(node(3), token(1)),
                ExprNode::binary(node(4), BinaryOp::Add, node(3), node(6)),
                ExprNode::literal(node(5), len(0.0)),
                ExprNode::literal(node(6), len(2.0)),
            ],
        )
        .expect("유효")
    }

    fn types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::from([
            (token(1), TypeSpec::required(ExpressionType::Length)),
            (
                scheme(),
                TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(9))),
            ),
            (token(2), TypeSpec::optional(ExpressionType::Length)),
        ])
    }

    fn binding(path: PropertyPath, expected: TypeSpec, owner: ScopeOwner) -> ValueBinding {
        ValueBinding::new(BindingId::new(sid(77)), path, source(), expected, owner)
    }

    fn samples() -> Vec<ValueBinding> {
        vec![
            binding(
                PropertyPath::new(sid(500)).push(PropertyKey(3)),
                TypeSpec::required(ExpressionType::Length),
                ScopeOwner::Node(NodeRef::new(sid(600))),
            ),
            binding(
                PropertyPath::with_keys(sid(501), vec![PropertyKey(1), PropertyKey(9)])
                    .with_index(4),
                TypeSpec::optional(ExpressionType::Length),
                ScopeOwner::Component(ComponentRef::new(sid(601))),
            ),
            binding(
                PropertyPath::new(sid(502)).with_key_segment("배경/hover"),
                TypeSpec::optional(ExpressionType::Enum(EnumTypeId::new(u32::MAX))),
                ScopeOwner::Node(NodeRef::new(sid(602))),
            ),
        ]
    }

    #[test]
    fn bindings_round_trip_through_canonical_bytes() {
        for binding in samples() {
            let bytes = binding.to_bytes().expect("직렬화");
            let decoded = ValueBinding::from_bytes(&bytes).expect("역직렬화");
            assert_eq!(decoded, binding);
            assert_eq!(decoded.to_bytes().expect("재직렬화"), bytes);
            assert_eq!(decoded.source_root(), node(1));
            assert_eq!(&bytes[..2], &BINDING_FORMAT_VERSION.to_le_bytes());
        }
        let first = &samples()[0];
        assert_eq!(first.references(), BTreeSet::from([token(1), scheme()]));
        // 할당 없는 단일 대상 검사는 references()와 같은 판정입니다.
        for binding in samples() {
            let references = binding.references();
            for target in [token(1), token(2), scheme()] {
                assert_eq!(binding.refers_to(&target), references.contains(&target));
            }
        }
        assert!(first.refers_to(&token(1)) && !first.refers_to(&token(2)));
    }

    fn fixed(byte: u8) -> StableId {
        StableId::from_bytes([byte; 16])
    }

    /// 버전 1 배치(ADR-0026 1절)를 고정합니다. 기대 bytes는 encoder를 쓰지 않고 형식 정의에서 직접
    /// 조립합니다. round-trip test는 배치가 바뀌어도 encoder·decoder가 함께 바뀌면 통과하므로, 이 test가
    /// 필드 순서·폭·엔디언을 지킵니다.
    #[test]
    fn canonical_bytes_golden() {
        // (1) 환경 참조 source, Required Enum target, node owner, key 경로 + index tail.
        let environment = ValueBinding::new(
            BindingId::new(fixed(0x77)),
            PropertyPath::with_keys(fixed(0x50), vec![PropertyKey(1), PropertyKey(0x0102_0304)])
                .with_index(5),
            ExpressionAst::new(
                ExprNodeId::new(fixed(0x01)),
                vec![ExprNode::reference(
                    ExprNodeId::new(fixed(0x01)),
                    ReferenceTarget::Environment(EnvironmentKey::new(4)),
                )],
            )
            .expect("유효"),
            TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(0xE000_0002))),
            ScopeOwner::Node(NodeRef::new(fixed(0x60))),
        );
        let mut expected = vec![0x01, 0x00]; // version u16 LE
        expected.extend([0x77; 16]); // BindingId
        expected.extend([0x21, 0x00, 0x00, 0x00]); // target blob 길이 33 (LE)
        expected.extend([0x50; 16]); // PropertyPath root
        expected.extend([0x00, 0x00, 0x00, 0x02]); // key 수 (BE)
        expected.extend([0x00, 0x00, 0x00, 0x01, 0x01, 0x02, 0x03, 0x04]); // key (BE)
        expected.extend([0x01, 0x00, 0x00, 0x00, 0x05]); // index tail (BE)
        expected.extend([0x00, 0x04, 0x02, 0x00, 0x00, 0xE0]); // Required, Enum, enum_type (LE)
        expected.push(0x01); // owner = Node
        expected.extend([0x60; 16]);
        expected.extend([0x22, 0x00, 0x00, 0x00]); // source blob 길이 34 (LE)
        expected.extend([0x01, 0x00, 0x01, 0x00, 0x00, 0x00]); // AST version, node 수 (LE)
        expected.extend([0x01; 16]);
        expected.extend([0x02, 0x00, 0x00]); // op = Reference
        expected.extend([0x04, 0x04, 0x00, 0x00, 0x00]); // environment key 4 (LE)
        expected.extend([0x00, 0x00, 0x00, 0x00]); // child 수
        assert_eq!(environment.to_bytes().expect("직렬화"), expected);
        assert_eq!(ValueBinding::from_bytes(&expected), Ok(environment));

        // (2) Enum literal source, Optional Enum target, component owner, key segment tail.
        // 같은 EnumTypeId가 type_spec에서는 LE, literal 값(TypedValue) 안에서는 BE입니다.
        let literal = ValueBinding::new(
            BindingId::new(fixed(0x78)),
            PropertyPath::new(fixed(0x51)).with_key_segment("가"),
            ExpressionAst::new(
                ExprNodeId::new(fixed(0x02)),
                vec![ExprNode::literal(
                    ExprNodeId::new(fixed(0x02)),
                    TypedValue::Enum(EnumValue::new(0xE000_0002, 1)),
                )],
            )
            .expect("유효"),
            TypeSpec::optional(ExpressionType::Enum(EnumTypeId::new(0xE000_0002))),
            ScopeOwner::Component(ComponentRef::new(fixed(0x61))),
        );
        let mut expected = vec![0x01, 0x00];
        expected.extend([0x78; 16]);
        expected.extend([0x1C, 0x00, 0x00, 0x00]); // target blob 길이 28 (LE)
        expected.extend([0x51; 16]);
        expected.extend([0x00, 0x00, 0x00, 0x00]); // key 수 0
        expected.extend([0x02, 0x00, 0x00, 0x00, 0x03, 0xEA, 0xB0, 0x80]); // key segment "가" (길이 BE)
        expected.extend([0x01, 0x04, 0x02, 0x00, 0x00, 0xE0]); // Optional, Enum, enum_type (LE)
        expected.push(0x02); // owner = Component
        expected.extend([0x61; 16]);
        expected.extend([0x2B, 0x00, 0x00, 0x00]); // source blob 길이 43 (LE)
        expected.extend([0x01, 0x00, 0x01, 0x00, 0x00, 0x00]);
        expected.extend([0x02; 16]);
        expected.extend([0x01, 0x00, 0x00]); // op = Literal
        expected.extend([0x01, 0x09, 0x00, 0x00, 0x00]); // literal 값, TypedValue blob 길이 9 (LE)
        expected.extend([0x04, 0xE0, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01]); // Enum (BE 필드)
        expected.extend([0x00, 0x00, 0x00, 0x00]); // child 수
        assert_eq!(literal.to_bytes().expect("직렬화"), expected);
        assert_eq!(ValueBinding::from_bytes(&expected), Ok(literal));
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = samples()[1].to_bytes().expect("직렬화");
        for cut in 0..bytes.len() {
            assert!(
                ValueBinding::from_bytes(&bytes[..cut]).is_err(),
                "잘린 입력 {cut}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            ValueBinding::from_bytes(&trailing),
            Err(BindingCodecError::TrailingBytes)
        );
        let mut version = bytes.clone();
        version[0] = 2;
        assert_eq!(
            ValueBinding::from_bytes(&version),
            Err(BindingCodecError::UnsupportedVersion(2))
        );
        // type_spec 위치: version 2 + id 16 + target blob(4 + len).
        let target_len = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        let spec = 22 + target_len;
        let mut nullable = bytes.clone();
        nullable[spec] = 2;
        assert_eq!(
            ValueBinding::from_bytes(&nullable),
            Err(BindingCodecError::UnknownNullability(2))
        );
        let mut tag = bytes.clone();
        tag[spec + 1] = 0x0E;
        assert_eq!(
            ValueBinding::from_bytes(&tag),
            Err(BindingCodecError::UnknownTypeTag(0x0E))
        );
        let mut owner = bytes.clone();
        owner[spec + 2] = 0x03;
        assert_eq!(
            ValueBinding::from_bytes(&owner),
            Err(BindingCodecError::UnknownOwner(0x03))
        );
        let mut target = bytes.clone();
        target[22 + 16 + 3] ^= 0x01; // key 수를 바꿔 PropertyPath bytes 손상.
        assert!(matches!(
            ValueBinding::from_bytes(&target),
            Err(BindingCodecError::InvalidTarget) | Err(BindingCodecError::Truncated)
        ));
        let mut huge = bytes.clone();
        huge[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            ValueBinding::from_bytes(&huge),
            Err(BindingCodecError::Truncated)
        );
        let mut source = bytes;
        let last = source.len() - 1;
        source[last] ^= 0xFF;
        assert!(matches!(
            ValueBinding::from_bytes(&source),
            Err(BindingCodecError::Source(_))
        ));
    }

    #[test]
    fn source_type_must_be_assignable_to_target() {
        let types = types();
        let required = &samples()[0];
        assert_eq!(
            required.validate_types(&types),
            Ok(TypeSpec::required(ExpressionType::Length))
        );
        // Optional 속성은 Required 결과를 받습니다.
        assert!(samples()[1].validate_types(&types).is_ok());
        // 타입이 다른 속성.
        let mismatch = binding(
            PropertyPath::new(sid(503)),
            TypeSpec::required(ExpressionType::Color),
            ScopeOwner::Node(NodeRef::new(sid(603))),
        );
        assert_eq!(
            mismatch.validate_types(&types),
            Err(BindingTypeError::Mismatch {
                expected: TypeSpec::required(ExpressionType::Color),
                actual: TypeSpec::required(ExpressionType::Length),
            })
        );
        // nullable 결과는 Required 속성에 둘 수 없습니다.
        let nullable_source =
            ExpressionAst::new(node(10), vec![ExprNode::reference(node(10), token(2))])
                .expect("유효");
        let nullable = required.with_source(
            nullable_source.clone(),
            TypeSpec::required(ExpressionType::Length),
        );
        assert!(matches!(
            nullable.validate_types(&types),
            Err(BindingTypeError::Mismatch { .. })
        ));
        let optional =
            required.with_source(nullable_source, TypeSpec::optional(ExpressionType::Length));
        assert!(optional.validate_types(&types).is_ok());
        // 모르는 참조는 표현식 오류.
        assert!(matches!(
            required.validate_types(&BTreeMap::new()),
            Err(BindingTypeError::Expression(errors)) if !errors.is_empty()
        ));
        assert!(assignable(
            TypeSpec::optional(ExpressionType::Number),
            TypeSpec::required(ExpressionType::Number)
        ));
        assert!(!assignable(
            TypeSpec::required(ExpressionType::Number),
            TypeSpec::optional(ExpressionType::Number)
        ));
    }

    #[test]
    fn binding_set_keeps_one_binding_per_target() {
        let mut set = BindingSet::new();
        let [a, b, c] = <[ValueBinding; 3]>::try_from(samples()).expect("3개");
        // samples는 모두 같은 ID라 ID를 바꿔 씁니다.
        let b = ValueBinding {
            id: BindingId::new(sid(78)),
            ..b
        };
        set.insert(a.clone()).expect("a");
        set.insert(b.clone()).expect("b");
        assert_eq!(
            set.insert(c.clone()),
            Err(BindingSetError::DuplicateId(a.id()))
        );
        let same_target = ValueBinding {
            id: BindingId::new(sid(79)),
            ..a.clone()
        };
        assert_eq!(
            set.insert(same_target),
            Err(BindingSetError::DuplicateTarget {
                target: a.target().clone(),
                existing: a.id()
            })
        );
        assert_eq!(
            set.by_target(b.target()).map(ValueBinding::id),
            Some(b.id())
        );
        // 교체는 target·owner 유지.
        let moved = ValueBinding {
            target: c.target().clone(),
            ..a.clone()
        };
        assert_eq!(
            set.replace(moved),
            Err(BindingSetError::TargetChanged(a.id()))
        );
        let retyped = a.with_source(source(), TypeSpec::optional(ExpressionType::Length));
        assert_eq!(set.replace(retyped.clone()), Ok(a.clone()));
        assert_eq!(set.get(a.id()), Some(&retyped));
        assert_eq!(set.remove(a.id()), Some(retyped));
        assert!(set.by_target(a.target()).is_none());
        assert_eq!(set.len(), 1);
    }
}
