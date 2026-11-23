//! Context 해석기 (XXX.7.4, ADR-0025 결정 3.5·3.11, ADR-0099 결정 4).
//!
//! 소비 지점 [`ContextSite`]에서 **작성 scope** 기준으로 위로 올라가며 가장 가까운 provision을 찾고, 없으면 key default를
//! 씁니다.
//!
//! 1. node 자신 → 작성 부모(scene 부모 또는 정의 tree 부모).
//! 2. 정의 root에 이르면 그 정의를 펼친 instance node(chain 마지막 원소, 한 단계 바깥 scope)로 이어 갑니다.
//! 3. 작성 부모가 ComponentInstance이고 node가 그 인스턴스의 slot override 콘텐츠이면 slot 경계입니다. 찾는 key가 그 Slot
//!    공개 목록에 있으면 slot host(인스턴스 chain을 한 단계 늘린 site)부터 컴포넌트 내부 조상을 찾고 — 내부에 없으면 2번
//!    규칙으로 instance node에 닿습니다 —, 공개하지 않은 key는 내부를 건너뛰고 instance node로 갑니다.
//! 4. provision 값 식이 같은 key를 읽으면 provider의 작성 부모부터(바깥 조상 값), 다른 key는 provider 자신부터 찾습니다.
//!
//! 결과는 (site, key, 시작 포함 여부)별로 캐시합니다(runtime index, ADR-0025 Performance). 같은 입력은 같은 결과입니다.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::bytecode::Program;
use stellar_expressions::eval::ReferenceValues;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

use super::provision::ContextProvision;
use super::set::{ContextKeys, ContextSet};
use crate::component::definition::{ComponentEntry, ComponentLibrary, ComponentSlotId};
use crate::component::expansion::{ExpandedNodeId, MAX_INSTANCE_NESTING};
use crate::component::instance::ComponentInstancePayload;
use crate::model::DocumentModel;
use crate::node_kind::NodeKind;
use crate::records::NodePayload;

/// 소비·provision 위치: 인스턴스 chain(scene 인스턴스부터 바깥 → 안쪽)과 그 chain 마지막 인스턴스가 펼치는 정의의 node.
/// chain이 비면 scene node입니다. instance expansion view의 `ExpandedNodeId`와 같은 식별입니다(IX.2.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextSite {
    /// 인스턴스 chain.
    pub instances: Vec<StableId>,
    /// node.
    pub node: StableId,
}

impl ContextSite {
    /// scene node site.
    pub fn scene(node: StableId) -> ContextSite {
        ContextSite {
            instances: Vec::new(),
            node,
        }
    }

    /// chain 안 정의 node site.
    pub fn virtual_node(instances: Vec<StableId>, node: StableId) -> ContextSite {
        ContextSite { instances, node }
    }
}

impl From<&ExpandedNodeId> for ContextSite {
    fn from(id: &ExpandedNodeId) -> ContextSite {
        match id {
            ExpandedNodeId::Virtual { instances, source } => {
                ContextSite::virtual_node(instances.clone(), *source)
            }
            ExpandedNodeId::Owned(node) => ContextSite::scene(*node),
        }
    }
}

/// 해석 출처.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextOrigin {
    /// 조상 provision이 없어 key default.
    Default,
    /// provision.
    Provision {
        /// provision StableId.
        provision: StableId,
        /// provider node.
        provider: StableId,
        /// provider를 펼친 인스턴스 chain.
        instances: Vec<StableId>,
    },
}

/// key 하나의 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextResolution {
    /// key.
    pub key: StableId,
    /// 출처.
    pub origin: ContextOrigin,
    /// 소비 지점부터 provider(또는 최상위)까지 지나간 site(찾은 순서).
    pub path: Vec<ContextSite>,
    /// 공개 목록으로 들어가 컴포넌트 내부를 찾은 Slot(지난 순서).
    pub via_slots: Vec<StableId>,
    /// key를 공개하지 않아 내부를 건너뛴 Slot(지난 순서).
    pub skipped_slots: Vec<StableId>,
    /// 이 해석이 가린(더 바깥에 있던) 같은 key provision. shadowing 정보 진단에 씁니다.
    pub shadowed: Vec<StableId>,
}

impl ContextResolution {
    /// default로 해석됐는지.
    pub fn is_default(&self) -> bool {
        self.origin == ContextOrigin::Default
    }

    /// 제공한 provision(default면 `None`).
    pub fn provision(&self) -> Option<StableId> {
        match &self.origin {
            ContextOrigin::Provision { provision, .. } => Some(*provision),
            ContextOrigin::Default => None,
        }
    }
}

/// 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// site node가 scene·정의 tree에 없습니다.
    UnknownSite(ContextSite),
    /// chain의 인스턴스가 가리키는 컴포넌트를 찾을 수 없거나 인스턴스가 아닙니다.
    BadInstance {
        /// 인스턴스 node.
        instance: StableId,
    },
    /// chain이 중첩 상한을 넘거나 같은 컴포넌트를 다시 펼칩니다.
    Nesting {
        /// chain 길이.
        depth: usize,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::UnknownSite(site) => {
                write!(f, "Context site {site:?}를 찾을 수 없습니다")
            }
            ResolveError::BadInstance { instance } => {
                write!(f, "인스턴스 {instance}의 컴포넌트를 펼칠 수 없습니다")
            }
            ResolveError::Nesting { depth } => {
                write!(f, "인스턴스 chain 깊이 {depth}가 상한을 넘거나 재귀입니다")
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// 위로 한 걸음.
enum Up {
    /// 최상위(View root).
    Top,
    /// 작성 부모.
    Parent(ContextSite),
    /// slot 경계: 콘텐츠의 작성 부모 instance node와 그 인스턴스 안 slot host.
    Slot {
        slot: ComponentSlotId,
        instance: ContextSite,
        host: ContextSite,
    },
}

/// Context 해석기(캐시 포함).
#[derive(Debug)]
pub struct ContextResolver<'a> {
    model: &'a DocumentModel,
    library: &'a ComponentLibrary,
    context: &'a ContextSet,
    cache: RefCell<HashMap<(ContextSite, StableId, bool), ContextResolution>>,
    components: RefCell<HashMap<Vec<StableId>, StableId>>,
}

impl<'a> ContextResolver<'a> {
    /// 모델의 Context로 해석기를 만듭니다.
    pub fn new(model: &'a DocumentModel, library: &'a ComponentLibrary) -> ContextResolver<'a> {
        ContextResolver::with_context(model, library, model.context())
    }

    /// 다른 ContextSet(예: override를 적용한 set)으로 해석기를 만듭니다.
    pub fn with_context(
        model: &'a DocumentModel,
        library: &'a ComponentLibrary,
        context: &'a ContextSet,
    ) -> ContextResolver<'a> {
        ContextResolver {
            model,
            library,
            context,
            cache: RefCell::new(HashMap::new()),
            components: RefCell::new(HashMap::new()),
        }
    }

    /// 문서 모델.
    pub fn model(&self) -> &'a DocumentModel {
        self.model
    }

    /// 컴포넌트 library.
    pub fn library(&self) -> &'a ComponentLibrary {
        self.library
    }

    /// ContextSet.
    pub fn context(&self) -> &'a ContextSet {
        self.context
    }

    /// 캐시한 해석 수(runtime index 크기).
    pub fn cached(&self) -> usize {
        self.cache.borrow().len()
    }

    /// node StableId의 인스턴스 payload(scene 또는 정의 tree node).
    fn instance_payload(&self, site: &ContextSite) -> Option<ComponentInstancePayload> {
        if site.instances.is_empty() {
            match self.model.payload(site.node)? {
                NodePayload::ComponentInstance(payload) => Some(payload.clone()),
                _ => None,
            }
        } else {
            let entry = self.entry_of(&site.instances).ok()?;
            let node = entry.tree().node(site.node)?;
            ComponentInstancePayload::decode_for(node.kind, &node.payload).ok()
        }
    }

    /// chain 마지막 인스턴스가 펼치는 컴포넌트.
    pub fn component_of(&self, instances: &[StableId]) -> Result<StableId, ResolveError> {
        if let Some(found) = self.components.borrow().get(instances) {
            return Ok(*found);
        }
        let Some((last, outer)) = instances.split_last() else {
            return Err(ResolveError::Nesting { depth: 0 });
        };
        if instances.len() > MAX_INSTANCE_NESTING {
            return Err(ResolveError::Nesting {
                depth: instances.len(),
            });
        }
        let payload = self
            .instance_payload(&ContextSite::virtual_node(outer.to_vec(), *last))
            .ok_or(ResolveError::BadInstance { instance: *last })?;
        let component = payload.component().id();
        if self.library.get(payload.component()).is_none() {
            return Err(ResolveError::BadInstance { instance: *last });
        }
        // 같은 컴포넌트를 chain 안에서 다시 펼치면 재귀입니다.
        for prefix in 1..instances.len() {
            if self.component_of(&instances[..prefix])? == component {
                return Err(ResolveError::Nesting {
                    depth: instances.len(),
                });
            }
        }
        self.components
            .borrow_mut()
            .insert(instances.to_vec(), component);
        Ok(component)
    }

    fn entry_of(&self, instances: &[StableId]) -> Result<&'a ComponentEntry, ResolveError> {
        let component = self.component_of(instances)?;
        self.library
            .get(ComponentRef::new(component))
            .ok_or(ResolveError::BadInstance {
                instance: *instances.last().unwrap_or(&StableId::NIL),
            })
    }

    /// site의 작성 부모(slot 경계 포함).
    fn up(&self, site: &ContextSite) -> Result<Up, ResolveError> {
        let (parent, parent_kind) = if site.instances.is_empty() {
            let record = self
                .model
                .record(site.node)
                .ok_or_else(|| ResolveError::UnknownSite(site.clone()))?;
            match record.parent {
                None => return Ok(Up::Top),
                Some(parent) => (parent, self.model.payload(parent).map(NodePayload::kind)),
            }
        } else {
            let entry = self.entry_of(&site.instances)?;
            let node = entry
                .tree()
                .node(site.node)
                .ok_or_else(|| ResolveError::UnknownSite(site.clone()))?;
            match node.parent {
                None => {
                    let (last, outer) = site.instances.split_last().expect("non-empty chain");
                    return Ok(Up::Parent(ContextSite::virtual_node(outer.to_vec(), *last)));
                }
                Some(parent) => (parent, entry.tree().node(parent).map(|n| n.kind)),
            }
        };
        let parent_site = ContextSite::virtual_node(site.instances.clone(), parent);
        if parent_kind == Some(NodeKind::ComponentInstance) {
            if let Some(payload) = self.instance_payload(&parent_site) {
                if let Some((slot, _)) = payload
                    .slot_overrides()
                    .find(|(_, content)| content.position(site.node).is_some())
                {
                    let mut chain = site.instances.clone();
                    chain.push(parent);
                    let entry = self.entry_of(&chain)?;
                    let host = entry
                        .slot(slot)
                        .map(|s| s.host.id())
                        .ok_or(ResolveError::BadInstance { instance: parent })?;
                    return Ok(Up::Slot {
                        slot,
                        instance: parent_site,
                        host: ContextSite::virtual_node(chain, host),
                    });
                }
            }
        }
        Ok(Up::Parent(parent_site))
    }

    fn lookup(&self, site: &ContextSite, key: StableId) -> Option<&'a ContextProvision> {
        self.context.provision_at(site.node, key)
    }

    /// `site`에서 key를 해석합니다(site 자신의 provision 포함).
    pub fn resolve(
        &self,
        key: StableId,
        site: &ContextSite,
    ) -> Result<ContextResolution, ResolveError> {
        self.resolve_from(key, site, true)
    }

    /// `site`의 바깥(작성 부모)부터 key를 해석합니다. provision 값 식의 같은 key 재읽기에 씁니다.
    pub fn resolve_outer(
        &self,
        key: StableId,
        site: &ContextSite,
    ) -> Result<ContextResolution, ResolveError> {
        self.resolve_from(key, site, false)
    }

    /// provider site에 붙은 provision의 값 식이 `read`를 읽을 때의 해석(같은 key는 바깥, 다른 key는 자신부터).
    pub fn resolve_provision_read(
        &self,
        provision: &ContextProvision,
        provider: &ContextSite,
        read: StableId,
    ) -> Result<ContextResolution, ResolveError> {
        if read == provision.key() {
            self.resolve_outer(read, provider)
        } else {
            self.resolve(read, provider)
        }
    }

    fn resolve_from(
        &self,
        key: StableId,
        start: &ContextSite,
        include_start: bool,
    ) -> Result<ContextResolution, ResolveError> {
        let cache_key = (start.clone(), key, include_start);
        if let Some(hit) = self.cache.borrow().get(&cache_key) {
            return Ok(hit.clone());
        }
        let mut path = Vec::new();
        let mut via_slots = Vec::new();
        let mut skipped_slots = Vec::new();
        let mut origin = ContextOrigin::Default;
        let mut current = start.clone();
        let mut check = include_start;
        let limit = self.model.node_count()
            + self.library.iter().map(|e| e.tree().len()).sum::<usize>()
                * (MAX_INSTANCE_NESTING + 1)
            + 16;
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > limit {
                return Err(ResolveError::Nesting {
                    depth: current.instances.len(),
                });
            }
            path.push(current.clone());
            if check {
                if let Some(provision) = self.lookup(&current, key) {
                    origin = ContextOrigin::Provision {
                        provision: provision.id(),
                        provider: provision.provider(),
                        instances: current.instances.clone(),
                    };
                    break;
                }
            }
            check = true;
            match self.up(&current)? {
                Up::Top => break,
                Up::Parent(parent) => current = parent,
                Up::Slot {
                    slot,
                    instance,
                    host,
                } => {
                    if self.context.slot_keys(slot.stable_id()).contains(&key) {
                        via_slots.push(slot.stable_id());
                        current = host;
                    } else {
                        skipped_slots.push(slot.stable_id());
                        current = instance;
                    }
                }
            }
        }
        let shadowed = match &origin {
            ContextOrigin::Provision {
                instances,
                provider,
                ..
            } => {
                let outer = self.resolve_outer(
                    key,
                    &ContextSite::virtual_node(instances.clone(), *provider),
                )?;
                let mut list = outer.provision().into_iter().collect::<Vec<_>>();
                list.extend(outer.shadowed);
                list
            }
            ContextOrigin::Default => Vec::new(),
        };
        let resolution = ContextResolution {
            key,
            origin,
            path,
            via_slots,
            skipped_slots,
            shadowed,
        };
        self.cache
            .borrow_mut()
            .insert(cache_key, resolution.clone());
        Ok(resolution)
    }

    /// provision의 provider site(scene node면 빈 chain). 정의 node provision은 인스턴스마다 site가 다르므로 chain을 받습니다.
    pub fn provider_site(provision: &ContextProvision, instances: &[StableId]) -> ContextSite {
        ContextSite::virtual_node(instances.to_vec(), provision.provider())
    }
}

/// Context 밖 참조(token·속성·Component Property·환경 입력)의 타입·값. 소비 site를 함께 받습니다.
pub trait ContextInputs {
    /// 참조 타입.
    fn input_type(&self, target: &ReferenceTarget, site: &ContextSite) -> Option<TypeSpec>;
    /// 참조 값.
    fn input_value(
        &self,
        target: &ReferenceTarget,
        site: &ContextSite,
    ) -> Option<Option<TypedValue>>;
}

/// Context 밖 참조가 없는 입력.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoInputs;

impl ContextInputs for NoInputs {
    fn input_type(&self, _: &ReferenceTarget, _: &ContextSite) -> Option<TypeSpec> {
        None
    }
    fn input_value(&self, _: &ReferenceTarget, _: &ContextSite) -> Option<Option<TypedValue>> {
        None
    }
}

/// 값 평가 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextEvalError {
    /// 해석 실패.
    Resolve(ResolveError),
    /// key가 없습니다(orphan).
    UnknownKey(StableId),
    /// provision 값 식 컴파일·평가 실패.
    Expression {
        /// provision.
        provision: StableId,
        /// 원인.
        message: String,
    },
    /// provision 값 식끼리의 읽기가 순환합니다.
    Cycle(Vec<StableId>),
}

impl fmt::Display for ContextEvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextEvalError::Resolve(error) => write!(f, "{error}"),
            ContextEvalError::UnknownKey(key) => write!(f, "ContextKey {key}가 없습니다"),
            ContextEvalError::Expression { provision, message } => {
                write!(f, "provision {provision} 값 식: {message}")
            }
            ContextEvalError::Cycle(path) => write!(f, "provision 읽기 순환: {path:?}"),
        }
    }
}

impl std::error::Error for ContextEvalError {}

/// 해석 결과로 Context 값을 계산합니다(XXX.7.4). key 정의는 `keys`(문서 set + 외부 library set)에서 찾습니다.
pub struct ContextEvaluator<'r, 'a> {
    resolver: &'r ContextResolver<'a>,
    keys: &'r ContextKeys<'r>,
    inputs: &'r dyn ContextInputs,
    stack: RefCell<Vec<(StableId, ContextSite)>>,
    values: RefCell<BTreeMap<(StableId, ContextSite), Option<TypedValue>>>,
    error: RefCell<Option<ContextEvalError>>,
}

impl<'r, 'a> ContextEvaluator<'r, 'a> {
    /// 평가기를 만듭니다.
    pub fn new(
        resolver: &'r ContextResolver<'a>,
        keys: &'r ContextKeys<'r>,
        inputs: &'r dyn ContextInputs,
    ) -> ContextEvaluator<'r, 'a> {
        ContextEvaluator {
            resolver,
            keys,
            inputs,
            stack: RefCell::new(Vec::new()),
            values: RefCell::new(BTreeMap::new()),
            error: RefCell::new(None),
        }
    }

    /// `site`에서 key의 값.
    pub fn value(
        &self,
        key: StableId,
        site: &ContextSite,
    ) -> Result<Option<TypedValue>, ContextEvalError> {
        let resolution = self
            .resolver
            .resolve(key, site)
            .map_err(ContextEvalError::Resolve)?;
        self.value_of(&resolution)
    }

    /// 해석 결과의 값.
    pub fn value_of(
        &self,
        resolution: &ContextResolution,
    ) -> Result<Option<TypedValue>, ContextEvalError> {
        match &resolution.origin {
            ContextOrigin::Default => {
                let key = self
                    .keys
                    .key(resolution.key)
                    .ok_or(ContextEvalError::UnknownKey(resolution.key))?;
                key.default_expression()
                    .evaluate(&BTreeMap::<ReferenceTarget, Option<TypedValue>>::new())
                    .map_err(|error| ContextEvalError::Expression {
                        provision: resolution.key,
                        message: error.to_string(),
                    })
            }
            ContextOrigin::Provision {
                provision,
                instances,
                ..
            } => {
                let provision = self
                    .resolver
                    .context()
                    .provision(*provision)
                    .expect("해석기는 set의 provision만 돌려줍니다");
                self.provision_value(provision, instances)
            }
        }
    }

    /// provision 하나가 인스턴스 chain `instances`에서 내는 값.
    pub fn provision_value(
        &self,
        provision: &ContextProvision,
        instances: &[StableId],
    ) -> Result<Option<TypedValue>, ContextEvalError> {
        let site = ContextResolver::provider_site(provision, instances);
        let memo = (provision.id(), site.clone());
        if let Some(value) = self.values.borrow().get(&memo) {
            return Ok(value.clone());
        }
        if self.stack.borrow().contains(&memo) {
            let mut path: Vec<StableId> = self.stack.borrow().iter().map(|(id, _)| *id).collect();
            path.push(provision.id());
            return Err(ContextEvalError::Cycle(path));
        }
        let key = self
            .keys
            .key(provision.key())
            .ok_or(ContextEvalError::UnknownKey(provision.key()))?;
        self.stack.borrow_mut().push(memo.clone());
        let result = self.compute(provision, &site, key.value_type());
        self.stack.borrow_mut().pop();
        let value = result?;
        self.values.borrow_mut().insert(memo, value.clone());
        Ok(value)
    }

    fn compute(
        &self,
        provision: &ContextProvision,
        site: &ContextSite,
        declared: TypeSpec,
    ) -> Result<Option<TypedValue>, ContextEvalError> {
        let view = ProvisionView {
            evaluator: self,
            provision,
            site,
        };
        let expression_error = |message: String| ContextEvalError::Expression {
            provision: provision.id(),
            message,
        };
        let program = Program::compile(provision.value(), &view)
            .map_err(|error| expression_error(error.to_string()))?;
        if !crate::component::typed_expr::assignable(program.result_type(), declared) {
            return Err(expression_error(format!(
                "값 타입 {}는 key 타입 {declared}에 맞지 않습니다",
                program.result_type()
            )));
        }
        let result = program.evaluate_with_limits(&view, EvalLimits::DEFAULT);
        if let Some(error) = self.error.borrow_mut().take() {
            return Err(error);
        }
        result
            .map(|(value, _)| value)
            .map_err(|error| expression_error(error.to_string()))
    }
}

struct ProvisionView<'v, 'r, 'a> {
    evaluator: &'v ContextEvaluator<'r, 'a>,
    provision: &'v ContextProvision,
    site: &'v ContextSite,
}

impl ReferenceTypes for ProvisionView<'_, '_, '_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Context(key) => self.evaluator.keys.key(*key).map(|k| k.value_type()),
            other => self.evaluator.inputs.input_type(other, self.site),
        }
    }
}

impl ReferenceValues for ProvisionView<'_, '_, '_> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        match target {
            ReferenceTarget::Context(key) => {
                let result = self
                    .evaluator
                    .resolver
                    .resolve_provision_read(self.provision, self.site, *key)
                    .map_err(ContextEvalError::Resolve)
                    .and_then(|resolution| self.evaluator.value_of(&resolution));
                match result {
                    Ok(value) => Some(value),
                    Err(error) => {
                        self.evaluator.error.borrow_mut().get_or_insert(error);
                        None
                    }
                }
            }
            other => self.evaluator.inputs.input_value(other, self.site),
        }
    }
}
