//! `ContextProvision`(XXX.7.2, ADR-0025 결정 3.2, ADR-0099 결정 1).
//!
//! provision 하나는 provider node 하나에 key 하나와 값 식 하나를 연결합니다. 효과 범위는 provider의 하위 트리이며 instance
//! expansion view 기준으로 펼친 트리를 따릅니다(해석은 [`super::resolve`]). provision은 소유 정의·View(owner) 안의 node에만
//! 붙습니다. 상위 정의는 자기 tree 안의 ComponentInstance node에 provision을 붙일 수 있습니다.
//!
//! canonical bytes:
//!
//! ```text
//! provision := id[16] ++ owner ++ provider[16] ++ key[16] ++ bytes(value canonical AST)
//! owner     := 0x01 ++ view_id[16] | 0x02 ++ component_id[16]
//! ```

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::{ExpressionAst, NodePayload, ReferenceTarget};
use stellar_expressions::codec::CodecError;

use crate::component::codec::{put_bytes, put_id, put_u8, Malformed, Reader};
use crate::component::definition::ComponentLibrary;
use crate::model::DocumentModel;
use crate::node_kind::NodeKind;

/// provision 값의 속성 key(ADR-0099 결정 9). 경로 `(provision StableId, [PROVISION_VALUE])`로 View Variant·Target Profile
/// override가 값을 덮어씁니다. `0xECxx` 대역(IX.12.1 예약)의 값입니다.
pub const PROVISION_VALUE: PropertyKey = PropertyKey(0xEC10);

/// provision을 소유한 작성 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextOwner {
    /// View 최상위 트리(값은 View StableId).
    View(StableId),
    /// 컴포넌트 정의 tree(값은 컴포넌트 StableId).
    Component(StableId),
}

impl ContextOwner {
    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ContextOwner::View(id) => {
                put_u8(buf, 0x01);
                put_id(buf, *id);
            }
            ContextOwner::Component(id) => {
                put_u8(buf, 0x02);
                put_id(buf, *id);
            }
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<ContextOwner, Malformed> {
        match reader.u8()? {
            0x01 => Ok(ContextOwner::View(reader.id()?)),
            0x02 => Ok(ContextOwner::Component(reader.id()?)),
            _ => Err(Malformed("알 수 없는 provision owner tag")),
        }
    }
}

impl fmt::Display for ContextOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextOwner::View(id) => write!(f, "view {id}"),
            ContextOwner::Component(id) => write!(f, "component {id}"),
        }
    }
}

/// provision 생성·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextProvisionError {
    /// id·provider·key에 NIL을 썼습니다.
    NilId,
    /// 값 식을 canonical bytes로 쓰거나 읽지 못했습니다.
    Value(CodecError),
    /// canonical bytes가 잘못됐습니다.
    Malformed(&'static str),
}

impl fmt::Display for ContextProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextProvisionError::NilId => {
                f.write_str("provision의 id·provider·key에는 NIL을 쓸 수 없습니다")
            }
            ContextProvisionError::Value(error) => write!(f, "provision 값 식: {error}"),
            ContextProvisionError::Malformed(reason) => write!(f, "provision bytes: {reason}"),
        }
    }
}

impl std::error::Error for ContextProvisionError {}

impl From<Malformed> for ContextProvisionError {
    fn from(value: Malformed) -> ContextProvisionError {
        ContextProvisionError::Malformed(value.0)
    }
}

/// `ContextProvision`.
///
/// 값 식은 저장된 AST 그대로입니다. 타입 검사는 key·참조 타입을 아는 validator(XXX.7.5)가 합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextProvision {
    id: StableId,
    owner: ContextOwner,
    provider: StableId,
    key: StableId,
    value: ExpressionAst,
    canonical: Vec<u8>,
}

impl ContextProvision {
    /// provision을 만듭니다.
    pub fn new(
        id: StableId,
        owner: ContextOwner,
        provider: StableId,
        key: StableId,
        value: ExpressionAst,
    ) -> Result<ContextProvision, ContextProvisionError> {
        let owner_id = match owner {
            ContextOwner::View(id) | ContextOwner::Component(id) => id,
        };
        if [id, provider, key, owner_id].contains(&StableId::NIL) {
            return Err(ContextProvisionError::NilId);
        }
        let canonical = value
            .to_canonical_bytes()
            .map_err(ContextProvisionError::Value)?;
        Ok(ContextProvision {
            id,
            owner,
            provider,
            key,
            value,
            canonical,
        })
    }

    /// StableId.
    pub fn id(&self) -> StableId {
        self.id
    }

    /// 소유 scope.
    pub fn owner(&self) -> ContextOwner {
        self.owner
    }

    /// provider node StableId.
    pub fn provider(&self) -> StableId {
        self.provider
    }

    /// 공급하는 key StableId.
    pub fn key(&self) -> StableId {
        self.key
    }

    /// 값 식.
    pub fn value(&self) -> &ExpressionAst {
        &self.value
    }

    /// 값 식 canonical bytes(VIII.2.5).
    pub fn value_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// 값 식이 리터럴 하나뿐이면 그 값(override 공통값과 텍스트 원본 리터럴 문법에 씀).
    pub fn literal_value(&self) -> Option<&stellar_core_types::typed_value::TypedValue> {
        match self.value.node(self.value.root())?.payload() {
            NodePayload::Literal(stellar_expressions::ast::Literal::Value(value)) => Some(value),
            _ => None,
        }
    }

    /// 값 식이 읽는 Context key(정렬, 중복 없음).
    pub fn reads(&self) -> BTreeSet<StableId> {
        context_reads(&self.value)
    }

    /// 값 식만 바꾼 provision.
    pub fn with_value(
        &self,
        value: ExpressionAst,
    ) -> Result<ContextProvision, ContextProvisionError> {
        ContextProvision::new(self.id, self.owner, self.provider, self.key, value)
    }

    /// 값 경로(ADR-0099 결정 9).
    pub fn value_path(&self) -> PropertyPath {
        PropertyPath::with_keys(self.id, vec![PROVISION_VALUE])
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id);
        self.owner.encode_into(buf);
        put_id(buf, self.provider);
        put_id(buf, self.key);
        put_bytes(buf, &self.canonical);
    }

    pub(crate) fn decode_from(
        reader: &mut Reader<'_>,
    ) -> Result<ContextProvision, ContextProvisionError> {
        let id = reader.id()?;
        let owner = ContextOwner::decode_from(reader)?;
        let provider = reader.id()?;
        let key = reader.id()?;
        let value = ExpressionAst::from_canonical_bytes(reader.bytes()?)
            .map_err(ContextProvisionError::Value)?;
        ContextProvision::new(id, owner, provider, key, value)
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// canonical bytes에서 디코딩합니다.
    pub fn decode(bytes: &[u8]) -> Result<ContextProvision, ContextProvisionError> {
        let mut reader = Reader::new(bytes);
        let provision = ContextProvision::decode_from(&mut reader)?;
        reader.expect_end()?;
        if provision.encode() != bytes {
            return Err(ContextProvisionError::Malformed(
                "canonical bytes가 아닙니다",
            ));
        }
        Ok(provision)
    }
}

/// 표현식이 읽는 Context key(정렬, 중복 없음).
pub fn context_reads(ast: &ExpressionAst) -> BTreeSet<StableId> {
    ast.referenced_targets()
        .into_iter()
        .filter_map(|target| match target {
            ReferenceTarget::Context(key) => Some(*key),
            _ => None,
        })
        .collect()
}

/// provision이 owner 범위 밖 node를 가리키는 이유(XXX.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionScopeError {
    /// owner View가 문서에 없습니다.
    UnknownView {
        /// View.
        view: StableId,
    },
    /// owner 컴포넌트가 library에 없습니다.
    UnknownComponent {
        /// 컴포넌트.
        component: StableId,
    },
    /// provider node가 owner의 tree에 없습니다(다른 View·다른 컴포넌트 내부·없는 node).
    OutsideOwner {
        /// provider.
        provider: StableId,
        /// owner.
        owner: ContextOwner,
    },
}

impl fmt::Display for ProvisionScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvisionScopeError::UnknownView { view } => write!(f, "View {view}가 없습니다"),
            ProvisionScopeError::UnknownComponent { component } => {
                write!(f, "컴포넌트 {component}가 없습니다")
            }
            ProvisionScopeError::OutsideOwner { provider, owner } => {
                write!(f, "provider {provider}는 {owner}의 tree에 있지 않습니다")
            }
        }
    }
}

impl std::error::Error for ProvisionScopeError {}

/// scene node의 View root까지 올라가 그 root를 돌려줍니다.
pub(crate) fn scene_root(model: &DocumentModel, node: StableId) -> Option<StableId> {
    let mut current = node;
    let mut steps = 0usize;
    loop {
        let record = model.record(current)?;
        match record.parent {
            Some(parent) => current = parent,
            None => return Some(current),
        }
        steps += 1;
        if steps > model.node_count() {
            return None;
        }
    }
}

/// provider가 owner 범위 안 node인지 확인합니다(XXX.7.2). 자기 tree 안의 ComponentInstance node는 허용합니다.
pub fn check_provision_scope(
    provision: &ContextProvision,
    model: &DocumentModel,
    library: &ComponentLibrary,
) -> Result<(), ProvisionScopeError> {
    let outside = || ProvisionScopeError::OutsideOwner {
        provider: provision.provider,
        owner: provision.owner,
    };
    match provision.owner {
        ContextOwner::View(view) => {
            let root = model
                .definition()
                .pages
                .iter()
                .flat_map(|page| page.views.iter())
                .find(|candidate| candidate.id == view)
                .map(|candidate| candidate.root_frame)
                .ok_or(ProvisionScopeError::UnknownView { view })?;
            if scene_root(model, provision.provider) == Some(root) {
                Ok(())
            } else {
                Err(outside())
            }
        }
        ContextOwner::Component(component) => {
            library
                .get(ComponentRef::new(component))
                .ok_or(ProvisionScopeError::UnknownComponent { component })?;
            if library.owner_of(provision.provider) == Some(ComponentRef::new(component)) {
                Ok(())
            } else {
                Err(outside())
            }
        }
    }
}

/// provider node 종류(scene 또는 정의 tree). 없으면 `None`.
pub fn provider_kind(
    provision: &ContextProvision,
    model: &DocumentModel,
    library: &ComponentLibrary,
) -> Option<NodeKind> {
    match provision.owner {
        ContextOwner::View(_) => model.payload(provision.provider).map(|p| p.kind()),
        ContextOwner::Component(component) => library
            .get(ComponentRef::new(component))?
            .tree()
            .node(provision.provider)
            .map(|node| node.kind),
    }
}
