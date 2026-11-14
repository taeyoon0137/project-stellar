//! 컴포넌트 경계를 넘는 drilling binding (IX.4.1).
//!
//! 컴포넌트 경계를 넘는 값은 공개 Component Property를 통해서만 전달합니다(기술 아키텍처 14.3). 상위
//! ComponentDefinition이 자기 Property를 만들고, 정의 tree 안 nested instance의 Property에 **명시적으로**
//! binding합니다. [`DrillingBinding`]이 그 binding입니다:
//!
//! - **owner**: binding을 소유하는 상위 컴포넌트(scope owner). 값 원천 **source**는 owner의 Property입니다.
//! - **path**: owner 정의 tree에서 시작하는 nested instance chain(바깥 → 안쪽). `path[0]`은 owner 정의 tree의
//!   ComponentInstance node, `path[k]`는 `path[k-1]`이 참조하는 컴포넌트의 정의 tree node입니다. path 길이가
//!   넘는 **경계 수**입니다.
//! - **target**: 가장 안쪽 인스턴스가 참조하는 컴포넌트의 Property.
//! - **chain**: 경계마다 값이 지나가는 ComponentProperty StableId입니다. `chain[k]`는 `path[k]`가 참조하는
//!   컴포넌트의 Property이며 `chain`의 마지막은 `target`입니다. **chain은 필수**입니다 — 경계 수만큼 property가
//!   없으면 만들 수도, 디코딩할 수도 없습니다([`DrillingError::MissingChain`]).
//!
//! 경계 하나를 넘는 binding(`path.len() == 1`)이 기본 단위 **link**입니다. 둘 이상을 넘는 binding은 중간 경계마다
//! 그 컴포넌트가 소유한 link(`chain[k-1]` → `path[k]`의 `chain[k]`)가 이미 있어야 합니다
//! ([`DrillingError::MissingIntermediateLink`]) — 모든 경계에 Property와 binding이 명시적으로 존재해야 한다는
//! 규칙(14.3)을 저장 모델이 그대로 표현합니다.
//!
//! [`DrillingSet`]은 문서의 drilling binding 모음입니다. 첫 경계의 값 자리(owner, `path[0]`, `chain[0]`)마다
//! binding은 하나뿐입니다. 저장은 canonical binary(id 오름차순)이며, library에 대한 의미 검증은
//! [`DrillingSet::validate`]가 합니다(디코딩은 구조 규칙만 검사).
//!
//! 일반 Frame·Fragment는 drilling 경계가 아닙니다. 경계 안쪽 binding(경계 0개)은 값 binding 계층
//! (`stellar-bindings`, VIII.6)의 몫이라 여기서는 표현하지 않습니다([`DrillingError::NoBoundary`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::types::TypeSpec;

use super::codec::{capacity_hint, put_count, put_id, Malformed, Reader};
use super::definition::{ComponentEntry, ComponentLibrary, ComponentPropertyId};
use super::expansion::MAX_INSTANCE_NESTING;
use super::instance::{ComponentInstancePayload, InstancePayloadError, OverrideKey};
use super::typed_expr::assignable;
use crate::node_kind::NodeKind;

/// drilling binding이 넘을 수 있는 경계 수 상한(인스턴스 중첩 상한과 같음).
pub const MAX_DRILLING_DEPTH: usize = MAX_INSTANCE_NESTING;

/// drilling binding 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrillingBindingId(StableId);

impl DrillingBindingId {
    /// `StableId`로 만듭니다.
    pub const fn new(id: StableId) -> DrillingBindingId {
        DrillingBindingId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

impl fmt::Display for DrillingBindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// drilling binding·모음 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillingError {
    /// 경계를 넘지 않습니다(path가 비었음). 경계 안쪽 binding은 drilling이 아닙니다.
    NoBoundary,
    /// 경계 수가 상한을 넘습니다.
    TooDeep {
        /// 경계 수.
        depth: usize,
    },
    /// path에 같은 인스턴스가 두 번 나옵니다.
    RepeatedInstance {
        /// 인스턴스.
        instance: StableId,
    },
    /// 경계마다 property가 있어야 하는데 chain이 짧습니다(chain 누락).
    MissingChain {
        /// 경계 수(path 길이).
        boundaries: usize,
        /// chain property 수.
        properties: usize,
    },
    /// chain이 경계 수보다 깁니다.
    ExtraChain {
        /// 경계 수(path 길이).
        boundaries: usize,
        /// chain property 수.
        properties: usize,
    },
    /// chain 마지막 property가 target과 다릅니다.
    ChainTargetMismatch {
        /// target.
        target: ComponentPropertyId,
        /// chain 마지막.
        last: ComponentPropertyId,
    },
    /// 컴포넌트가 library에 없습니다.
    UnknownComponent {
        /// 컴포넌트.
        component: ComponentRef,
    },
    /// 컴포넌트에 선언되지 않은 property입니다.
    UnknownProperty {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 경계 안쪽 property가 private입니다(공개 property만 drilling 대상).
    PrivateProperty {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 경계 `boundary`의 인스턴스가 바로 바깥 컴포넌트 정의 tree에 없습니다.
    InstanceOutOfScope {
        /// 경계 index(0 = owner 정의 tree).
        boundary: usize,
        /// 인스턴스.
        instance: StableId,
        /// 인스턴스가 있어야 하는 컴포넌트.
        scope: ComponentRef,
    },
    /// path node가 ComponentInstance가 아닙니다.
    NotAnInstance {
        /// node.
        instance: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// nested 인스턴스 payload가 잘못됐습니다.
    InstancePayload {
        /// 인스턴스.
        instance: StableId,
        /// 원인.
        error: InstancePayloadError,
    },
    /// nested 인스턴스가 참조하는 컴포넌트가 library에 없습니다.
    UnresolvedComponent {
        /// 인스턴스.
        instance: StableId,
        /// 참조 컴포넌트.
        component: ComponentRef,
    },
    /// 경계 `boundary`로 들어가는 값의 타입이 그 경계 property 타입에 대입되지 않습니다.
    TypeMismatch {
        /// 경계 index.
        boundary: usize,
        /// 경계 property 선언 타입.
        expected: TypeSpec,
        /// 들어오는 값 타입.
        found: TypeSpec,
    },
    /// 중간 경계 `boundary`(≥ 1)에 그 바깥 컴포넌트가 소유한 link가 없습니다.
    MissingIntermediateLink {
        /// 경계 index.
        boundary: usize,
    },
    /// 첫 경계 인스턴스가 같은 property를 정적 override로도 지정합니다(값 원천 두 개).
    StaticOverrideConflict {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// 같은 id의 binding이 이미 있습니다.
    DuplicateBinding {
        /// binding.
        binding: DrillingBindingId,
    },
    /// 같은 값 자리(owner, 첫 인스턴스, 첫 경계 property)를 다른 binding이 이미 채웁니다.
    DuplicateTarget {
        /// 기존 binding.
        existing: DrillingBindingId,
    },
    /// binding이 없습니다.
    UnknownBinding {
        /// binding.
        binding: DrillingBindingId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for DrillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrillingError::NoBoundary => {
                f.write_str("drilling binding은 컴포넌트 경계를 하나 이상 넘어야 합니다")
            }
            DrillingError::TooDeep { depth } => write!(
                f,
                "drilling 경계 수 {depth}가 상한 {MAX_DRILLING_DEPTH}을 넘습니다"
            ),
            DrillingError::RepeatedInstance { instance } => {
                write!(f, "drilling path에 인스턴스 {instance}가 반복됩니다")
            }
            DrillingError::MissingChain {
                boundaries,
                properties,
            } => write!(
                f,
                "경계 {boundaries}개를 넘는 binding에 경계 property가 {properties}개뿐입니다(chain 누락)"
            ),
            DrillingError::ExtraChain {
                boundaries,
                properties,
            } => write!(
                f,
                "경계 {boundaries}개를 넘는 binding의 chain이 {properties}개로 깁니다"
            ),
            DrillingError::ChainTargetMismatch { target, last } => write!(
                f,
                "chain 마지막 property {}가 target {}와 다릅니다",
                last.stable_id(),
                target.stable_id()
            ),
            DrillingError::UnknownComponent { component } => {
                write!(f, "컴포넌트 {}가 없습니다", component.id())
            }
            DrillingError::UnknownProperty {
                component,
                property,
            } => write!(
                f,
                "컴포넌트 {}에 property {}가 없습니다",
                component.id(),
                property.stable_id()
            ),
            DrillingError::PrivateProperty {
                component,
                property,
            } => write!(
                f,
                "컴포넌트 {}의 private property {}는 drilling 대상이 아닙니다",
                component.id(),
                property.stable_id()
            ),
            DrillingError::InstanceOutOfScope {
                boundary,
                instance,
                scope,
            } => write!(
                f,
                "경계 {boundary}의 인스턴스 {instance}가 컴포넌트 {} 정의 tree에 없습니다",
                scope.id()
            ),
            DrillingError::NotAnInstance { instance, kind } => write!(
                f,
                "drilling path node {instance}는 ComponentInstance가 아닙니다({})",
                kind.name()
            ),
            DrillingError::InstancePayload { instance, error } => {
                write!(f, "인스턴스 {instance} payload 오류: {error}")
            }
            DrillingError::UnresolvedComponent {
                instance,
                component,
            } => write!(
                f,
                "인스턴스 {instance}의 컴포넌트 {}를 찾을 수 없습니다",
                component.id()
            ),
            DrillingError::TypeMismatch {
                boundary,
                expected,
                found,
            } => write!(
                f,
                "경계 {boundary}로 들어가는 값 타입 {found}가 property 타입 {expected}에 맞지 않습니다"
            ),
            DrillingError::MissingIntermediateLink { boundary } => {
                write!(f, "중간 경계 {boundary}에 drilling link가 없습니다")
            }
            DrillingError::StaticOverrideConflict { instance, property } => write!(
                f,
                "인스턴스 {instance}가 property {}를 정적 override로도 지정합니다",
                property.stable_id()
            ),
            DrillingError::DuplicateBinding { binding } => {
                write!(f, "drilling binding {binding}가 이미 있습니다")
            }
            DrillingError::DuplicateTarget { existing } => write!(
                f,
                "같은 nested property 자리를 drilling binding {existing}가 이미 채웁니다"
            ),
            DrillingError::UnknownBinding { binding } => {
                write!(f, "drilling binding {binding}가 없습니다")
            }
            DrillingError::Malformed { reason } => {
                write!(f, "drilling binding binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for DrillingError {}

impl From<Malformed> for DrillingError {
    fn from(err: Malformed) -> DrillingError {
        DrillingError::Malformed { reason: err.0 }
    }
}

/// 첫 경계의 값 자리: owner 컴포넌트, 첫 nested 인스턴스, 그 인스턴스 컴포넌트의 property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrillingSlot {
    /// owner 컴포넌트.
    pub owner: ComponentRef,
    /// owner 정의 tree 안 nested 인스턴스.
    pub instance: StableId,
    /// 인스턴스 컴포넌트의 property.
    pub property: ComponentPropertyId,
}

/// 컴포넌트 경계를 넘는 binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrillingBinding {
    id: DrillingBindingId,
    owner: ComponentRef,
    source: ComponentPropertyId,
    path: Vec<StableId>,
    target: ComponentPropertyId,
    chain: Vec<ComponentPropertyId>,
}

impl DrillingBinding {
    /// binding을 만듭니다. path가 비었거나, 너무 깊거나, 인스턴스가 반복되거나, chain이 경계 수와 다르거나
    /// chain 마지막이 target이 아니면 거부합니다.
    pub fn new(
        id: DrillingBindingId,
        owner: ComponentRef,
        source: ComponentPropertyId,
        path: Vec<StableId>,
        target: ComponentPropertyId,
        chain: Vec<ComponentPropertyId>,
    ) -> Result<DrillingBinding, DrillingError> {
        if path.is_empty() {
            return Err(DrillingError::NoBoundary);
        }
        if path.len() > MAX_DRILLING_DEPTH {
            return Err(DrillingError::TooDeep { depth: path.len() });
        }
        let mut seen = BTreeSet::new();
        if let Some(instance) = path.iter().find(|id| !seen.insert(**id)) {
            return Err(DrillingError::RepeatedInstance {
                instance: *instance,
            });
        }
        if chain.len() < path.len() {
            return Err(DrillingError::MissingChain {
                boundaries: path.len(),
                properties: chain.len(),
            });
        }
        if chain.len() > path.len() {
            return Err(DrillingError::ExtraChain {
                boundaries: path.len(),
                properties: chain.len(),
            });
        }
        match chain.last() {
            Some(last) if *last == target => {}
            Some(last) => {
                return Err(DrillingError::ChainTargetMismatch {
                    target,
                    last: *last,
                })
            }
            None => {
                return Err(DrillingError::MissingChain {
                    boundaries: path.len(),
                    properties: 0,
                })
            }
        }
        Ok(DrillingBinding {
            id,
            owner,
            source,
            path,
            target,
            chain,
        })
    }

    /// 경계 하나를 넘는 link: owner의 `source` → owner 정의 tree 안 `instance`의 `target`.
    pub fn link(
        id: DrillingBindingId,
        owner: ComponentRef,
        source: ComponentPropertyId,
        instance: StableId,
        target: ComponentPropertyId,
    ) -> DrillingBinding {
        DrillingBinding {
            id,
            owner,
            source,
            path: vec![instance],
            target,
            chain: vec![target],
        }
    }

    /// 식별자.
    pub fn id(&self) -> DrillingBindingId {
        self.id
    }

    /// owner 컴포넌트(scope owner).
    pub fn owner(&self) -> ComponentRef {
        self.owner
    }

    /// 값 원천: owner의 property.
    pub fn source(&self) -> ComponentPropertyId {
        self.source
    }

    /// nested instance chain(바깥 → 안쪽).
    pub fn path(&self) -> &[StableId] {
        &self.path
    }

    /// 가장 안쪽 컴포넌트의 대상 property.
    pub fn target(&self) -> ComponentPropertyId {
        self.target
    }

    /// 경계별 property chain(`chain[k]`는 `path[k]` 인스턴스 컴포넌트의 property).
    pub fn chain(&self) -> &[ComponentPropertyId] {
        &self.chain
    }

    /// 넘는 경계 수.
    pub fn boundaries(&self) -> usize {
        self.path.len()
    }

    /// 경계 하나만 넘는 link인지.
    pub fn is_link(&self) -> bool {
        self.path.len() == 1
    }

    /// 첫 경계의 값 자리.
    pub fn slot(&self) -> DrillingSlot {
        DrillingSlot {
            owner: self.owner,
            instance: self.path[0],
            property: self.chain[0],
        }
    }

    /// path 각 경계 인스턴스가 참조하는 컴포넌트(바깥 → 안쪽)를 해석합니다.
    ///
    /// `path[0]`은 owner 정의 tree의 ComponentInstance여야 하고, `path[k]`는 `path[k-1]` 컴포넌트 정의 tree의
    /// ComponentInstance여야 합니다.
    pub fn resolve_components(
        &self,
        library: &ComponentLibrary,
    ) -> Result<Vec<ComponentRef>, DrillingError> {
        resolve_path(library, self.owner, &self.path)
    }

    /// binding을 library와 같은 모음의 다른 binding(중간 link)에 대해 검증합니다.
    ///
    /// - owner·source가 있어야 합니다(source는 owner 내부 property라 private이어도 됩니다).
    /// - path가 해석돼야 하고 `chain[k]`는 `path[k]` 컴포넌트의 **공개** property여야 합니다.
    /// - 값이 source → `chain[0]` → … → `chain[last]`로 흐르며 각 단계 타입이 대입 가능해야 합니다.
    /// - 중간 경계 `k ≥ 1`마다 `path[k-1]` 컴포넌트가 소유한 link(`chain[k-1]` → `path[k]`의 `chain[k]`)가
    ///   `links`에 있어야 합니다.
    /// - 첫 경계 인스턴스 payload가 같은 property를 정적 override로도 지정하면 거부합니다.
    pub fn validate(
        &self,
        library: &ComponentLibrary,
        links: &DrillingSet,
    ) -> Result<(), DrillingError> {
        let owner = library
            .get(self.owner)
            .ok_or(DrillingError::UnknownComponent {
                component: self.owner,
            })?;
        let source = declared_property(owner, self.source)?;
        let components = self.resolve_components(library)?;
        let mut incoming = source.value_type();
        for (boundary, (component, property)) in components.iter().zip(&self.chain).enumerate() {
            let entry = library
                .get(*component)
                .ok_or(DrillingError::UnknownComponent {
                    component: *component,
                })?;
            let record = declared_property(entry, *property)?;
            if !record.is_public() {
                return Err(DrillingError::PrivateProperty {
                    component: *component,
                    property: *property,
                });
            }
            if !assignable(incoming, record.value_type()) {
                return Err(DrillingError::TypeMismatch {
                    boundary,
                    expected: record.value_type(),
                    found: incoming,
                });
            }
            if boundary >= 1 {
                let link_owner = components[boundary - 1];
                let has_link = links
                    .link_into(link_owner, self.path[boundary], *property)
                    .is_some_and(|link| link.source == self.chain[boundary - 1]);
                if !has_link {
                    return Err(DrillingError::MissingIntermediateLink { boundary });
                }
            }
            incoming = record.value_type();
        }
        let first = nested_payload(owner, self.path[0])?;
        if first
            .property_override(&OverrideKey::direct(self.chain[0]))
            .is_some()
        {
            return Err(DrillingError::StaticOverrideConflict {
                instance: self.path[0],
                property: self.chain[0],
            });
        }
        Ok(())
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        put_id(buf, self.owner.id());
        put_id(buf, self.source.stable_id());
        put_count(buf, self.path.len());
        for instance in &self.path {
            put_id(buf, *instance);
        }
        put_id(buf, self.target.stable_id());
        put_count(buf, self.chain.len());
        for property in &self.chain {
            put_id(buf, property.stable_id());
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<DrillingBinding, DrillingError> {
        let id = DrillingBindingId::new(reader.id()?);
        let owner = ComponentRef::new(reader.id()?);
        let source = ComponentPropertyId::new(reader.id()?);
        let path_len = reader.count()?;
        if path_len > MAX_DRILLING_DEPTH {
            return Err(DrillingError::TooDeep { depth: path_len });
        }
        let mut path = Vec::with_capacity(capacity_hint(path_len));
        for _ in 0..path_len {
            path.push(reader.id()?);
        }
        let target = ComponentPropertyId::new(reader.id()?);
        let chain_len = reader.count()?;
        if chain_len > MAX_DRILLING_DEPTH {
            return Err(DrillingError::ExtraChain {
                boundaries: path_len,
                properties: chain_len,
            });
        }
        let mut chain = Vec::with_capacity(capacity_hint(chain_len));
        for _ in 0..chain_len {
            chain.push(ComponentPropertyId::new(reader.id()?));
        }
        DrillingBinding::new(id, owner, source, path, target, chain)
    }

    /// canonical binary bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩하고 생성 규칙(chain 필수 포함)으로 재검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<DrillingBinding, DrillingError> {
        let mut reader = Reader::new(bytes);
        let binding = DrillingBinding::decode_from(&mut reader)?;
        reader.expect_end()?;
        Ok(binding)
    }
}

fn declared_property(
    entry: &ComponentEntry,
    property: ComponentPropertyId,
) -> Result<&super::property::ComponentProperty, DrillingError> {
    entry
        .property(property)
        .filter(|_| entry.definition().properties().contains(&property))
        .ok_or(DrillingError::UnknownProperty {
            component: entry.definition().reference(),
            property,
        })
}

fn nested_payload(
    scope: &ComponentEntry,
    instance: StableId,
) -> Result<ComponentInstancePayload, DrillingError> {
    let node = scope
        .tree()
        .node(instance)
        .ok_or(DrillingError::InstanceOutOfScope {
            boundary: 0,
            instance,
            scope: scope.definition().reference(),
        })?;
    ComponentInstancePayload::decode_for(node.kind, &node.payload).map_err(|error| match error {
        InstancePayloadError::KindMismatch { kind } => {
            DrillingError::NotAnInstance { instance, kind }
        }
        error => DrillingError::InstancePayload { instance, error },
    })
}

/// owner 정의 tree에서 시작하는 nested instance `path`의 경계별 컴포넌트(바깥 → 안쪽)를 해석합니다.
pub fn resolve_path(
    library: &ComponentLibrary,
    owner: ComponentRef,
    path: &[StableId],
) -> Result<Vec<ComponentRef>, DrillingError> {
    if path.len() > MAX_DRILLING_DEPTH {
        return Err(DrillingError::TooDeep { depth: path.len() });
    }
    let mut scope = library
        .get(owner)
        .ok_or(DrillingError::UnknownComponent { component: owner })?;
    let mut components = Vec::with_capacity(path.len());
    for (boundary, instance) in path.iter().enumerate() {
        let payload = nested_payload(scope, *instance).map_err(|error| match error {
            DrillingError::InstanceOutOfScope {
                instance, scope, ..
            } => DrillingError::InstanceOutOfScope {
                boundary,
                instance,
                scope,
            },
            other => other,
        })?;
        let component = payload.component();
        scope = library
            .get(component)
            .ok_or(DrillingError::UnresolvedComponent {
                instance: *instance,
                component,
            })?;
        components.push(component);
    }
    Ok(components)
}

/// 문서의 drilling binding 모음.
///
/// id 오름차순으로 순회·직렬화합니다. 첫 경계 값 자리([`DrillingSlot`])마다 binding은 하나뿐입니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrillingSet {
    bindings: BTreeMap<DrillingBindingId, DrillingBinding>,
    slots: BTreeMap<DrillingSlot, DrillingBindingId>,
}

impl DrillingSet {
    /// 빈 모음.
    pub fn new() -> DrillingSet {
        DrillingSet::default()
    }

    /// binding을 추가합니다. 같은 id나 같은 값 자리가 있으면 거부하고 모음은 바뀌지 않습니다.
    pub fn insert(&mut self, binding: DrillingBinding) -> Result<(), DrillingError> {
        if self.bindings.contains_key(&binding.id) {
            return Err(DrillingError::DuplicateBinding {
                binding: binding.id,
            });
        }
        let slot = binding.slot();
        if let Some(existing) = self.slots.get(&slot) {
            return Err(DrillingError::DuplicateTarget {
                existing: *existing,
            });
        }
        self.slots.insert(slot, binding.id);
        self.bindings.insert(binding.id, binding);
        Ok(())
    }

    /// binding을 제거하고 반환합니다.
    pub fn remove(&mut self, id: DrillingBindingId) -> Result<DrillingBinding, DrillingError> {
        let binding = self
            .bindings
            .remove(&id)
            .ok_or(DrillingError::UnknownBinding { binding: id })?;
        self.slots.remove(&binding.slot());
        Ok(binding)
    }

    /// binding.
    pub fn get(&self, id: DrillingBindingId) -> Option<&DrillingBinding> {
        self.bindings.get(&id)
    }

    /// 값 자리를 채우는 binding.
    pub fn at_slot(&self, slot: &DrillingSlot) -> Option<&DrillingBinding> {
        self.slots.get(slot).and_then(|id| self.bindings.get(id))
    }

    /// `owner`가 소유한, 정의 tree 안 `instance`의 `property`로 들어가는 link(경계 하나).
    pub fn link_into(
        &self,
        owner: ComponentRef,
        instance: StableId,
        property: ComponentPropertyId,
    ) -> Option<&DrillingBinding> {
        self.at_slot(&DrillingSlot {
            owner,
            instance,
            property,
        })
        .filter(|binding| binding.is_link())
    }

    /// id 오름차순 순회.
    pub fn iter(&self) -> impl Iterator<Item = &DrillingBinding> + '_ {
        self.bindings.values()
    }

    /// binding 수.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// 모든 binding을 library에 대해 검증합니다(id 오름차순, 첫 위반 반환).
    pub fn validate(
        &self,
        library: &ComponentLibrary,
    ) -> Result<(), (DrillingBindingId, DrillingError)> {
        for binding in self.bindings.values() {
            binding
                .validate(library, self)
                .map_err(|error| (binding.id, error))?;
        }
        Ok(())
    }

    /// canonical binary bytes(id 오름차순).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_count(&mut buf, self.bindings.len());
        for binding in self.bindings.values() {
            binding.encode_into(&mut buf);
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. 생성 규칙·id 오름차순·값 자리 유일성을 검사합니다.
    pub fn decode(bytes: &[u8]) -> Result<DrillingSet, DrillingError> {
        let mut reader = Reader::new(bytes);
        let count = reader.count()?;
        let mut set = DrillingSet::new();
        let mut previous: Option<DrillingBindingId> = None;
        for _ in 0..count {
            let binding = DrillingBinding::decode_from(&mut reader)?;
            if previous.is_some_and(|p| p >= binding.id) {
                return Err(DrillingError::Malformed {
                    reason: "drilling binding이 id 오름차순이 아님",
                });
            }
            previous = Some(binding.id);
            set.insert(binding)?;
        }
        reader.expect_end()?;
        Ok(set)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! `depth`단계 nested component chain fixture(IX.4.x 테스트 공용).
    //!
    //! 컴포넌트 `C0`(최상위) … `C{depth}`(가장 안쪽). `Ck`(k < depth)의 정의 tree는 root Frame 아래 `C{k+1}`
    //! 인스턴스 `I{k}` 하나를 갖습니다. 가장 안쪽 컴포넌트는 공개 `label`(String, "확인")과 private
    //! `secret`(Number) property를 갖습니다.

    use super::super::typed_expr::test_util::literal;
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::property::{ComponentProperty, PropertyVisibility};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::types::ExpressionType;

    pub(crate) fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0d71, 0x0123_4567_89ab)
    }

    pub(crate) fn component(k: usize) -> ComponentRef {
        ComponentRef::new(stable(10_000 + k as u64))
    }

    pub(crate) fn root(k: usize) -> StableId {
        stable(20_000 + k as u64)
    }

    pub(crate) fn instance(k: usize) -> StableId {
        stable(30_000 + k as u64)
    }

    pub(crate) fn pid(ts: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(stable(ts))
    }

    pub(crate) const LABEL: u64 = 40_000;
    pub(crate) const SECRET: u64 = 40_001;

    pub(crate) fn bid(ts: u64) -> DrillingBindingId {
        DrillingBindingId::new(stable(ts))
    }

    /// `id`·`name`의 공개 String property(default "확인").
    pub(crate) fn string_property(id: u64, name: &str) -> ComponentProperty {
        ComponentProperty::new(
            pid(id),
            name.to_string(),
            TypeSpec::required(ExpressionType::String),
            literal(TypedValue::String("확인".into())),
            PropertyVisibility::Public,
        )
        .unwrap()
    }

    /// `id`·`name`의 Number property.
    pub(crate) fn number_property(
        id: u64,
        name: &str,
        visibility: PropertyVisibility,
    ) -> ComponentProperty {
        ComponentProperty::new(
            pid(id),
            name.to_string(),
            TypeSpec::required(ExpressionType::Number),
            literal(TypedValue::Number(FiniteF32::new(1.0).unwrap())),
            visibility,
        )
        .unwrap()
    }

    /// `C0`…`C{depth}` library.
    pub(crate) fn nested_library(depth: usize) -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        for k in 0..=depth {
            let mut tree = DefinitionTree::new(component(k));
            tree.insert(DefinitionNode {
                id: root(k),
                kind: NodeKind::Frame,
                parent: None,
                order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
                visible: true,
                locked: false,
                display_name: format!("C{k} root"),
                payload: Vec::new(),
            })
            .unwrap();
            if k < depth {
                tree.insert(DefinitionNode {
                    id: instance(k),
                    kind: NodeKind::ComponentInstance,
                    parent: Some(root(k)),
                    order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
                    visible: true,
                    locked: false,
                    display_name: format!("C{} 인스턴스", k + 1),
                    payload: ComponentInstancePayload::new(component(k + 1)).encode(),
                })
                .unwrap();
            }
            let mut definition =
                ComponentDefinition::new(component(k).id(), format!("C{k}"), NodeRef::new(root(k)));
            if k == depth {
                definition.add_property(pid(LABEL)).unwrap();
                definition.add_property(pid(SECRET)).unwrap();
            }
            library.insert(definition, tree).unwrap();
            if k == depth {
                library
                    .add_property(component(k), string_property(LABEL, "label"))
                    .unwrap();
                library
                    .add_property(
                        component(k),
                        number_property(SECRET, "secret", PropertyVisibility::Private),
                    )
                    .unwrap();
            }
        }
        library
    }

    /// 컴포넌트 `k`에 property를 선언하고 record를 추가합니다.
    pub(crate) fn declare(library: &mut ComponentLibrary, k: usize, property: ComponentProperty) {
        let entry = library.entry_mut(component(k)).unwrap();
        entry.definition_mut().add_property(property.id()).unwrap();
        library.add_property(component(k), property).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::component::property::PropertyVisibility;
    use stellar_core_types::typed_value::TypedValue;

    /// 2단계(C0 ⊃ C1 ⊃ C2) library + C1.label(41_001) · C0.label(41_000) property와 C1의 link.
    fn two_level() -> (ComponentLibrary, DrillingSet) {
        let mut library = nested_library(2);
        declare(&mut library, 1, string_property(41_001, "label"));
        declare(&mut library, 0, string_property(41_000, "label"));
        let mut set = DrillingSet::new();
        set.insert(DrillingBinding::link(
            bid(1),
            component(1),
            pid(41_001),
            instance(1),
            pid(LABEL),
        ))
        .unwrap();
        (library, set)
    }

    #[test]
    fn binding_without_boundary_chain_is_rejected() {
        // 검증 기준(IX.4.1): chain 누락 fixture 거부.
        let path = vec![instance(0), instance(1)];
        // 경계 2개, chain 0개.
        assert_eq!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(41_000),
                path.clone(),
                pid(LABEL),
                Vec::new(),
            )
            .unwrap_err(),
            DrillingError::MissingChain {
                boundaries: 2,
                properties: 0
            }
        );
        // 경계 2개, chain에 안쪽 property 하나뿐(C1 경계 property 누락).
        assert_eq!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(41_000),
                path.clone(),
                pid(LABEL),
                vec![pid(LABEL)],
            )
            .unwrap_err(),
            DrillingError::MissingChain {
                boundaries: 2,
                properties: 1
            }
        );
        // chain 마지막이 target이 아님.
        assert_eq!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(41_000),
                path.clone(),
                pid(LABEL),
                vec![pid(41_001), pid(41_002)],
            )
            .unwrap_err(),
            DrillingError::ChainTargetMismatch {
                target: pid(LABEL),
                last: pid(41_002)
            }
        );
        // 경계가 없거나 chain이 더 길면 거부.
        assert_eq!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(41_000),
                Vec::new(),
                pid(LABEL),
                Vec::new()
            )
            .unwrap_err(),
            DrillingError::NoBoundary
        );
        assert!(matches!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(41_000),
                vec![instance(0)],
                pid(LABEL),
                vec![pid(41_001), pid(LABEL)],
            ),
            Err(DrillingError::ExtraChain { .. })
        ));
        // 저장 bytes로 chain 없는 binding을 만들어도 디코딩이 거부합니다.
        let mut bytes = Vec::new();
        put_count(&mut bytes, 1);
        put_id(&mut bytes, bid(9).stable_id());
        put_id(&mut bytes, component(0).id());
        put_id(&mut bytes, pid(41_000).stable_id());
        put_count(&mut bytes, 2);
        put_id(&mut bytes, instance(0));
        put_id(&mut bytes, instance(1));
        put_id(&mut bytes, pid(LABEL).stable_id());
        put_count(&mut bytes, 0);
        assert_eq!(
            DrillingSet::decode(&bytes).unwrap_err(),
            DrillingError::MissingChain {
                boundaries: 2,
                properties: 0
            }
        );
    }

    #[test]
    fn full_chain_validates_with_intermediate_links() {
        let (library, mut set) = two_level();
        set.validate(&library).unwrap();
        // C0 → C1.label → C2.label: chain 완비, C1 link 존재.
        let deep = DrillingBinding::new(
            bid(2),
            component(0),
            pid(41_000),
            vec![instance(0), instance(1)],
            pid(LABEL),
            vec![pid(41_001), pid(LABEL)],
        )
        .unwrap();
        deep.validate(&library, &set).unwrap();
        // 중간 link가 없으면 거부.
        let empty = DrillingSet::new();
        assert_eq!(
            deep.validate(&library, &empty).unwrap_err(),
            DrillingError::MissingIntermediateLink { boundary: 1 }
        );
        // 중간 경계 property가 그 컴포넌트에 없으면 거부.
        let wrong_middle = DrillingBinding::new(
            bid(3),
            component(0),
            pid(41_000),
            vec![instance(0), instance(1)],
            pid(LABEL),
            vec![pid(41_000), pid(LABEL)],
        )
        .unwrap();
        assert_eq!(
            wrong_middle.validate(&library, &set).unwrap_err(),
            DrillingError::UnknownProperty {
                component: component(1),
                property: pid(41_000)
            }
        );
        set.insert(deep).unwrap();
        set.validate(&library).unwrap();
        let bytes = set.encode();
        let decoded = DrillingSet::decode(&bytes).unwrap();
        assert_eq!(decoded, set);
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn semantic_rules_reject_scope_visibility_type_and_duplicates() {
        let (mut library, mut set) = two_level();
        // C2 인스턴스를 C0 정의 tree에서 바로 가리키면(경계 건너뛰기) 범위 밖.
        assert_eq!(
            DrillingBinding::link(bid(5), component(0), pid(41_000), instance(1), pid(LABEL))
                .validate(&library, &set)
                .unwrap_err(),
            DrillingError::InstanceOutOfScope {
                boundary: 0,
                instance: instance(1),
                scope: component(0)
            }
        );
        // root Frame은 인스턴스가 아님.
        assert!(matches!(
            DrillingBinding::link(bid(5), component(0), pid(41_000), root(0), pid(LABEL))
                .validate(&library, &set),
            Err(DrillingError::NotAnInstance { .. })
        ));
        // private property는 drilling 대상이 아님.
        declare(
            &mut library,
            1,
            number_property(41_101, "count", PropertyVisibility::Public),
        );
        assert_eq!(
            DrillingBinding::link(bid(5), component(1), pid(41_101), instance(1), pid(SECRET))
                .validate(&library, &set)
                .unwrap_err(),
            DrillingError::PrivateProperty {
                component: component(2),
                property: pid(SECRET)
            }
        );
        // Number → String 타입 불일치.
        assert_eq!(
            DrillingBinding::new(
                bid(5),
                component(1),
                pid(41_101),
                vec![instance(1)],
                pid(LABEL),
                vec![pid(LABEL)],
            )
            .unwrap()
            .validate(&library, &DrillingSet::new())
            .unwrap_err(),
            DrillingError::TypeMismatch {
                boundary: 0,
                expected: TypeSpec::required(stellar_expressions::types::ExpressionType::String),
                found: TypeSpec::required(stellar_expressions::types::ExpressionType::Number),
            }
        );
        // 같은 값 자리 중복.
        assert_eq!(
            set.insert(DrillingBinding::link(
                bid(6),
                component(1),
                pid(41_101),
                instance(1),
                pid(LABEL)
            ))
            .unwrap_err(),
            DrillingError::DuplicateTarget { existing: bid(1) }
        );
        assert_eq!(
            set.insert(DrillingBinding::link(
                bid(1),
                component(0),
                pid(41_000),
                instance(0),
                pid(41_001)
            ))
            .unwrap_err(),
            DrillingError::DuplicateBinding { binding: bid(1) }
        );
        assert_eq!(set.len(), 1);
        // 정적 override와 binding이 같은 자리를 채우면 거부.
        let mut payload = ComponentInstancePayload::new(component(2));
        payload.set_property_override(
            OverrideKey::direct(pid(LABEL)),
            TypedValue::String("정적".into()),
        );
        let entry = library.entry_mut(component(1)).unwrap();
        entry.tree_mut().set_payload(instance(1), payload.encode());
        assert_eq!(
            set.validate(&library).unwrap_err(),
            (
                bid(1),
                DrillingError::StaticOverrideConflict {
                    instance: instance(1),
                    property: pid(LABEL)
                }
            )
        );
        // 제거 후 같은 자리 재사용 가능.
        set.remove(bid(1)).unwrap();
        assert!(set.is_empty());
        assert!(matches!(
            set.remove(bid(1)),
            Err(DrillingError::UnknownBinding { .. })
        ));
    }

    #[test]
    fn decode_rejects_unsorted_trailing_and_deep_input() {
        let (_, mut set) = two_level();
        set.insert(DrillingBinding::link(
            bid(0),
            component(0),
            pid(41_000),
            instance(0),
            pid(41_001),
        ))
        .unwrap();
        let bytes = set.encode();
        assert_eq!(DrillingSet::decode(&bytes).unwrap(), set);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(DrillingSet::decode(&trailing).is_err());
        for cut in [0, 3, 20, bytes.len() - 1] {
            assert!(DrillingSet::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        // 순서 뒤집기.
        let first = set.get(bid(0)).unwrap().encode();
        let second = set.get(bid(1)).unwrap().encode();
        let mut swapped = Vec::new();
        put_count(&mut swapped, 2);
        swapped.extend_from_slice(&second);
        swapped.extend_from_slice(&first);
        assert!(matches!(
            DrillingSet::decode(&swapped),
            Err(DrillingError::Malformed { .. })
        ));
        // 상한을 넘는 path 길이는 할당 전에 거부.
        let mut deep = Vec::new();
        put_count(&mut deep, 1);
        put_id(&mut deep, bid(9).stable_id());
        put_id(&mut deep, component(0).id());
        put_id(&mut deep, pid(1).stable_id());
        put_count(&mut deep, u32::MAX as usize);
        assert!(matches!(
            DrillingSet::decode(&deep),
            Err(DrillingError::TooDeep { .. })
        ));
        // 반복 인스턴스.
        assert_eq!(
            DrillingBinding::new(
                bid(9),
                component(0),
                pid(1),
                vec![instance(0), instance(0)],
                pid(LABEL),
                vec![pid(2), pid(LABEL)],
            )
            .unwrap_err(),
            DrillingError::RepeatedInstance {
                instance: instance(0)
            }
        );
    }
}
