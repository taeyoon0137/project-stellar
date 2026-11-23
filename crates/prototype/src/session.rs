//! 프리뷰 세션과 tick 처리 순서 (XXX.3.3~XXX.3.6, ADR-0025 결정 2.7~2.9, ADR-0094 결정 6).
//!
//! [`PreviewSession`]은 컴포넌트 인스턴스 하나(와 그 nested instance)의 프리뷰 상태를 **세션 메모리에만** 둡니다. 문서
//! 원본(library·binding·MotionSpec)은 공유 참조로만 읽으므로 프리뷰 조작이 저장 bytes를 바꿀 수 없습니다(XXX.3.6).
//!
//! tick 하나의 처리 순서는 고정입니다([`PreviewSession::advance`]).
//!
//! 1. **입력 이벤트**: 입력 순서대로 포인터 입력을 제스처 판정기(이벤트 원천 node별)에 넣어 이벤트로 바꾸고, 직접 이벤트를
//!    더합니다. 그 뒤 판정기마다(원천 StableId 순) tick 끝 판정(long press)을 더합니다.
//! 2. **펄스 dispatch**: 이벤트를 차례로 dispatch(IX.5.5)하고, emit된 semantic action을 [`PulseRouter`]가 정한 다음 펄스로
//!    이어 너비 우선으로 처리합니다. 모든 dispatch는 작업 사본에 하고, 끝나면 한꺼번에 commit합니다(원자). 연쇄 깊이가
//!    [`SessionConfig::max_dispatch_depth`]를 넘으면 [`SessionDiagnostic::PulseDepthExceeded`]를 내고 그 tick의 상태 변화를
//!    모두 버립니다.
//! 3. **MotionSpec 시작·재지정**: 바뀐 상태 변수에 대해 시작 조건이 성립한 MotionSpec(id 순)의 대상마다, 지금 표시 중인 값을
//!    시작값으로 잡습니다(진행 중인 motion이면 그 현재 값에서 재지정).
//! 4. **binding 재평가**: 바뀐 상태 변수에 영향받는 binding만 scheduler(VIII.6.4)로 재평가하고, 그 값을 3의 끝값으로
//!    씁니다. 상태 변화가 없는 tick은 binding을 하나도 평가하지 않습니다(XXX.3.4).
//! 5. **보간**: 진행 중인 motion만 그 tick의 값으로 표본을 뽑습니다(XXX.3.2). 끝난 motion은 지웁니다.
//!
//! 환경 key `reduced_motion`이 켜져 있으면 대체 동작이 `instant`인 MotionSpec은 끝값으로 바로 바꿉니다(XXX.3.5).

use std::collections::{BTreeMap, VecDeque};

use stellar_bindings::binding::BindingSet;
use stellar_bindings::graph::DependencyNode;
use stellar_bindings::registry::TokenRegistry;
use stellar_bindings::scheduler::{BindingEvaluator, SchedulerError, StaticSources};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::state_dispatch::{dispatch, DispatchError};
use stellar_document::component::state_machine::{
    state_value_path, SemanticActionId, StateVariableId, Trigger,
};
use stellar_document::component::state_store::{
    InstanceKey, InstanceState, PreviewSessionId, StateStore, StateStoreError,
};
use stellar_document::motion::{
    check_endpoints, EndpointError, MotionOwner, MotionSet, MotionSpec, MotionSpecId, ReducedMotion,
};
use stellar_expressions::ast::{EnvironmentKey, ReferenceTarget};
use stellar_expressions::environment::key_by_name;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::clock::{ticks_to_ms, PreviewClock, Tick};
use crate::gesture::{GestureRecognizer, OutOfOrder, PointerInput, PointerSample};
use crate::timing::sample;

/// 세션 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// 펄스 연쇄 dispatch 깊이 상한(첫 이벤트가 깊이 0).
    pub max_dispatch_depth: u32,
    /// 환경 key `reduced_motion`.
    pub reduced_motion: bool,
}

impl Default for SessionConfig {
    fn default() -> SessionConfig {
        SessionConfig {
            max_dispatch_depth: 16,
            reduced_motion: false,
        }
    }
}

/// tick 하나의 입력.
#[derive(Debug, Clone, PartialEq)]
pub enum TickInput {
    /// 이벤트 원천 node의 포인터 입력(제스처 판정을 거칩니다).
    Pointer {
        /// 원천 node.
        source: StableId,
        /// 입력.
        input: PointerInput,
    },
    /// 루트 인스턴스에 바로 보내는 이벤트.
    Event {
        /// trigger.
        trigger: Trigger,
        /// payload.
        payload: Option<TypedValue>,
    },
}

/// 펄스 하나: 어느 인스턴스의 어떤 trigger인지.
#[derive(Debug, Clone, PartialEq)]
pub struct Pulse {
    /// 인스턴스.
    pub instance: InstanceKey,
    /// trigger.
    pub trigger: Trigger,
    /// payload.
    pub payload: Option<TypedValue>,
}

/// emit된 semantic action을 다음 펄스로 잇는 규칙.
pub trait PulseRouter {
    /// `instance`가 `action`을 emit했을 때 이어지는 펄스(순서대로 처리).
    fn route(&self, instance: &InstanceKey, action: SemanticActionId) -> Vec<Pulse>;
}

/// 기본 규칙: nested instance가 emit한 action은 부모 인스턴스의 `NestedAction` trigger가 됩니다.
#[derive(Debug, Clone, Default)]
pub struct ParentRouter {
    parents: BTreeMap<InstanceKey, (InstanceKey, StableId)>,
}

impl PulseRouter for ParentRouter {
    fn route(&self, instance: &InstanceKey, action: SemanticActionId) -> Vec<Pulse> {
        match self.parents.get(instance) {
            Some((parent, node)) => vec![Pulse {
                instance: parent.clone(),
                trigger: Trigger::NestedAction {
                    instance: *node,
                    action,
                },
                payload: None,
            }],
            None => Vec::new(),
        }
    }
}

/// 세션 진단.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionDiagnostic {
    /// 펄스 연쇄 깊이 상한 초과. 그 tick의 상태 변화는 버렸습니다.
    PulseDepthExceeded {
        /// 상한.
        limit: u32,
    },
    /// 이벤트 하나의 dispatch 실패(그 이벤트의 상태 변화 없음).
    Dispatch(DispatchError),
    /// 보간할 수 없는 끝값. 끝값으로 바로 바꿨습니다.
    Endpoint {
        /// MotionSpec.
        motion: MotionSpecId,
        /// 대상.
        target: PropertyPath,
        /// 원인.
        error: EndpointError,
    },
    /// 포인터 입력 tick 순서 오류.
    OutOfOrder(OutOfOrder),
}

/// 진행 중인 motion.
#[derive(Debug, Clone, PartialEq)]
struct ActiveMotion {
    motion: MotionSpecId,
    start: TypedValue,
    end: TypedValue,
    start_tick: Tick,
}

/// tick 하나의 결과.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TickReport {
    /// tick.
    pub tick: Tick,
    /// dispatch한 펄스 수.
    pub dispatched: usize,
    /// commit한 상태 변화(변수 id 순).
    pub state_changes: Vec<(StateVariableId, Option<TypedValue>)>,
    /// 이 tick에 평가한 binding 수.
    pub binding_evaluations: usize,
    /// 시작(또는 재지정)한 motion.
    pub motions_started: Vec<(MotionSpecId, PropertyPath)>,
    /// 진행 중인 motion 수(보간 후).
    pub active_motions: usize,
    /// 진단.
    pub diagnostics: Vec<SessionDiagnostic>,
}

/// 세션 생성 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// 상태 store 오류.
    Store(StateStoreError),
    /// binding scheduler 오류.
    Scheduler(SchedulerError),
}

/// 프리뷰 세션(문서를 쓰지 않음).
pub struct PreviewSession<'a> {
    library: &'a ComponentLibrary,
    motions: Vec<&'a MotionSpec>,
    session: PreviewSessionId,
    root: InstanceKey,
    instances: BTreeMap<InstanceKey, ComponentRef>,
    properties: BTreeMap<InstanceKey, BTreeMap<ReferenceTarget, Option<TypedValue>>>,
    parents: ParentRouter,
    router: Option<Box<dyn PulseRouter + 'a>>,
    store: StateStore,
    evaluator: BindingEvaluator,
    targets: Vec<PropertyPath>,
    clock: PreviewClock,
    recognizers: BTreeMap<StableId, GestureRecognizer>,
    active: BTreeMap<PropertyPath, ActiveMotion>,
    displayed: BTreeMap<PropertyPath, Option<TypedValue>>,
    config: SessionConfig,
    input_log: Vec<(Tick, TickInput)>,
    trace: Vec<u8>,
}

fn reduced_motion_key() -> Option<EnvironmentKey> {
    key_by_name("reduced_motion")
}

fn property_values(
    library: &ComponentLibrary,
    component: ComponentRef,
) -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
    library
        .get(component)
        .map(|entry| {
            entry
                .property_records()
                .map(|record| {
                    (
                        ReferenceTarget::ComponentProperty(record.id().stable_id()),
                        record.default_value().cloned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

impl<'a> PreviewSession<'a> {
    /// 컴포넌트 `component` 인스턴스 하나의 세션을 엽니다. binding은 그 정의 tree를 대상으로 하는 것만 씁니다.
    pub fn new(
        library: &'a ComponentLibrary,
        tokens: &'a TokenRegistry,
        bindings: &'a BindingSet,
        motions: &'a MotionSet,
        component: ComponentRef,
        config: SessionConfig,
    ) -> Result<PreviewSession<'a>, SessionError> {
        let session = PreviewSessionId::new(component.id());
        let root = InstanceKey::scene(component.id());
        let mut store = StateStore::new();
        store.open_session(session).map_err(SessionError::Store)?;
        let properties = property_values(library, component);
        let state = store
            .initialize(session, library, component, root.clone(), &properties)
            .map_err(SessionError::Store)?
            .clone();
        let entry = library.get(component).ok_or(SessionError::Store(
            StateStoreError::ComponentNotFound { component },
        ))?;
        let mut scoped = BindingSet::new();
        for binding in bindings.iter() {
            if entry.tree().contains(binding.target().root()) {
                // 같은 모음에서 왔으므로 id·target 중복이 없습니다.
                let _ = scoped.insert(binding.clone());
            }
        }
        let mut sources = StaticSources::new();
        for variable in entry.state_machine().variables() {
            sources.insert(
                DependencyNode::Property(state_value_path(variable.id())),
                (
                    variable.value_type(),
                    state.get(&variable.id()).cloned().flatten(),
                ),
            );
        }
        for record in entry.property_records() {
            sources.insert(
                DependencyNode::ComponentProperty(record.id().stable_id()),
                (record.value_type(), record.default_value().cloned()),
            );
        }
        if let Some(key) = reduced_motion_key() {
            sources.insert(
                DependencyNode::Environment(key),
                (
                    TypeSpec::required(ExpressionType::Boolean),
                    Some(TypedValue::Boolean(config.reduced_motion)),
                ),
            );
        }
        let evaluator =
            BindingEvaluator::new(tokens, &scoped, &sources).map_err(SessionError::Scheduler)?;
        let mut targets: Vec<PropertyPath> = scoped.iter().map(|b| b.target().clone()).collect();
        targets.sort();
        let mut displayed = BTreeMap::new();
        for target in &targets {
            displayed.insert(target.clone(), base_value(&evaluator, target));
        }
        let mut instances = BTreeMap::new();
        instances.insert(root.clone(), component);
        let mut by_instance = BTreeMap::new();
        by_instance.insert(root.clone(), properties);
        let mut motions: Vec<&MotionSpec> = motions
            .owned_by(MotionOwner::Component(component))
            .collect();
        motions.sort_by_key(|m| m.id());
        Ok(PreviewSession {
            library,
            motions,
            session,
            root,
            instances,
            properties: by_instance,
            parents: ParentRouter::default(),
            router: None,
            store,
            evaluator,
            targets,
            clock: PreviewClock::new(),
            recognizers: BTreeMap::new(),
            active: BTreeMap::new(),
            displayed,
            config,
            input_log: Vec::new(),
            trace: Vec::new(),
        })
    }

    /// nested instance를 더합니다. 그 인스턴스가 emit한 action은 부모의 `NestedAction`이 됩니다.
    pub fn add_nested_instance(
        &mut self,
        parent: InstanceKey,
        node: StableId,
        component: ComponentRef,
    ) -> Result<InstanceKey, SessionError> {
        let mut chain = parent.ids().to_vec();
        chain.push(node);
        let key = InstanceKey::chain(chain);
        let properties = property_values(self.library, component);
        self.store
            .initialize(
                self.session,
                self.library,
                component,
                key.clone(),
                &properties,
            )
            .map_err(SessionError::Store)?;
        self.instances.insert(key.clone(), component);
        self.properties.insert(key.clone(), properties);
        self.parents.parents.insert(key.clone(), (parent, node));
        Ok(key)
    }

    /// 펄스 연결 규칙을 바꿉니다(기본은 [`ParentRouter`]).
    pub fn set_router(&mut self, router: Box<dyn PulseRouter + 'a>) {
        self.router = Some(router);
    }

    /// 루트 인스턴스 key.
    pub fn root(&self) -> &InstanceKey {
        &self.root
    }

    /// 지금 tick.
    pub fn tick(&self) -> Tick {
        self.clock.tick()
    }

    /// 인스턴스 상태.
    pub fn state(&self, instance: &InstanceKey) -> Option<&InstanceState> {
        self.store.state(self.session, instance)
    }

    /// 표시 중인 속성 값.
    pub fn displayed(&self, target: &PropertyPath) -> Option<&Option<TypedValue>> {
        self.displayed.get(target)
    }

    /// 입력 로그(세션 메모리).
    pub fn input_log(&self) -> &[(Tick, TickInput)] {
        &self.input_log
    }

    /// 값 trace canonical bytes(tick마다 표시 값과 루트 상태).
    pub fn trace_bytes(&self) -> &[u8] {
        &self.trace
    }

    /// 환경 key `reduced_motion`을 바꿉니다. 그 key를 읽는 binding만 재평가합니다.
    pub fn set_reduced_motion(&mut self, on: bool) -> usize {
        self.config.reduced_motion = on;
        let Some(key) = reduced_motion_key() else {
            return 0;
        };
        let node = DependencyNode::Environment(key);
        if self.evaluator.graph().index_of(&node).is_none() {
            return 0;
        }
        let evaluated = self
            .evaluator
            .update_sources(vec![(node, Some(TypedValue::Boolean(on)))])
            .map(|r| r.evaluated)
            .unwrap_or(0);
        self.refresh_unanimated();
        evaluated
    }

    fn refresh_unanimated(&mut self) {
        for target in &self.targets {
            if !self.active.contains_key(target) {
                self.displayed
                    .insert(target.clone(), base_value(&self.evaluator, target));
            }
        }
    }

    fn route(&self, instance: &InstanceKey, action: SemanticActionId) -> Vec<Pulse> {
        match &self.router {
            Some(router) => router.route(instance, action),
            None => self.parents.route(instance, action),
        }
    }

    /// 한 tick 나아갑니다(처리 순서는 module 문서).
    pub fn advance(&mut self, inputs: Vec<TickInput>) -> TickReport {
        let tick = self.clock.advance();
        let mut report = TickReport {
            tick,
            ..TickReport::default()
        };

        // 1. 입력 이벤트.
        let mut pulses: Vec<Pulse> = Vec::new();
        for input in inputs {
            self.input_log.push((tick, input.clone()));
            match input {
                TickInput::Pointer { source, input } => {
                    let recognizer = self.recognizers.entry(source).or_default();
                    match recognizer.push(PointerSample { tick, input }) {
                        Ok(events) => pulses.extend(events.into_iter().map(|e| Pulse {
                            instance: self.root.clone(),
                            trigger: Trigger::Event {
                                kind: e.kind,
                                source,
                            },
                            payload: e.payload,
                        })),
                        Err(e) => report.diagnostics.push(SessionDiagnostic::OutOfOrder(e)),
                    }
                }
                TickInput::Event { trigger, payload } => pulses.push(Pulse {
                    instance: self.root.clone(),
                    trigger,
                    payload,
                }),
            }
        }
        for (source, recognizer) in self.recognizers.iter_mut() {
            for e in recognizer.finish_tick(tick) {
                pulses.push(Pulse {
                    instance: self.root.clone(),
                    trigger: Trigger::Event {
                        kind: e.kind,
                        source: *source,
                    },
                    payload: e.payload,
                });
            }
        }

        // 2. 펄스 dispatch(작업 사본, 원자 commit).
        let before = self.state(&self.root).cloned().unwrap_or_default();
        let mut work = self.store.clone();
        let mut queue: VecDeque<(Pulse, u32)> = pulses.into_iter().map(|p| (p, 0)).collect();
        let mut exceeded = false;
        while let Some((pulse, depth)) = queue.pop_front() {
            if depth > self.config.max_dispatch_depth {
                exceeded = true;
                break;
            }
            let Some(component) = self.instances.get(&pulse.instance).copied() else {
                continue;
            };
            let empty = BTreeMap::new();
            let properties = self.properties.get(&pulse.instance).unwrap_or(&empty);
            report.dispatched += 1;
            match dispatch(
                &mut work,
                self.session,
                self.library,
                component,
                &pulse.instance,
                properties,
                pulse.trigger,
                pulse.payload,
            ) {
                Ok(outcome) => {
                    for action in outcome.emitted {
                        for next in self.route(&pulse.instance, action) {
                            queue.push_back((next, depth + 1));
                        }
                    }
                }
                Err(error) => report.diagnostics.push(SessionDiagnostic::Dispatch(error)),
            }
        }
        if exceeded {
            report
                .diagnostics
                .push(SessionDiagnostic::PulseDepthExceeded {
                    limit: self.config.max_dispatch_depth,
                });
        } else {
            self.store = work;
        }
        let after = self.state(&self.root).cloned().unwrap_or_default();
        let changed: Vec<(StateVariableId, Option<TypedValue>)> = after
            .iter()
            .filter(|(var, value)| before.get(var) != Some(value))
            .map(|(var, value)| (*var, value.clone()))
            .collect();
        report.state_changes = changed.clone();

        // 3. MotionSpec 시작·재지정: 시작값은 지금 표시 중인 값.
        let mut pending: Vec<(MotionSpecId, PropertyPath, Option<TypedValue>)> = Vec::new();
        for spec in &self.motions {
            let variable = spec.start().variable();
            if !changed.iter().any(|(v, _)| *v == variable) {
                continue;
            }
            let old = before.get(&variable).cloned().flatten();
            let new = after.get(&variable).cloned().flatten();
            if !spec.start().fires(old.as_ref(), new.as_ref()) {
                continue;
            }
            for target in spec.targets() {
                if pending.iter().any(|(_, t, _)| t == target) {
                    // 같은 대상을 둘 이상의 MotionSpec이 잡으면 id가 앞선 것이 이깁니다.
                    continue;
                }
                let current = self.displayed.get(target).cloned().flatten();
                pending.push((spec.id(), target.clone(), current));
            }
        }

        // 4. binding 재평가(바뀐 상태 변수에 영향받는 binding만).
        let updates: Vec<(DependencyNode, Option<TypedValue>)> = changed
            .iter()
            .map(|(var, value)| {
                (
                    DependencyNode::Property(state_value_path(*var)),
                    value.clone(),
                )
            })
            .filter(|(node, _)| self.evaluator.graph().index_of(node).is_some())
            .collect();
        if !updates.is_empty() {
            if let Ok(update) = self.evaluator.update_sources(updates) {
                report.binding_evaluations = update.evaluated;
            }
        }
        for (motion, target, start) in pending {
            let end = base_value(&self.evaluator, &target);
            let spec = self.motions.iter().find(|m| m.id() == motion).copied();
            let instant = self.config.reduced_motion
                && spec.is_some_and(|s| s.reduced_motion() == ReducedMotion::Instant);
            report.motions_started.push((motion, target.clone()));
            if instant || start == end {
                self.active.remove(&target);
                self.displayed.insert(target, end);
                continue;
            }
            match check_endpoints(start.as_ref(), end.as_ref()) {
                Ok(_) => {
                    let (Some(start), Some(end)) = (start, end) else {
                        continue;
                    };
                    self.active.insert(
                        target,
                        ActiveMotion {
                            motion,
                            start,
                            end,
                            start_tick: tick,
                        },
                    );
                }
                Err(error) => {
                    report.diagnostics.push(SessionDiagnostic::Endpoint {
                        motion,
                        target: target.clone(),
                        error,
                    });
                    self.active.remove(&target);
                    self.displayed.insert(target, end);
                }
            }
        }
        // motion 없이 값이 바뀐 대상은 바로 바꿉니다.
        if report.binding_evaluations > 0 {
            self.refresh_unanimated();
        }

        // 5. 진행 중인 motion 보간.
        let mut finished = Vec::new();
        for (target, motion) in &self.active {
            let Some(spec) = self.motions.iter().find(|m| m.id() == motion.motion) else {
                finished.push(target.clone());
                continue;
            };
            let elapsed = ticks_to_ms(tick - motion.start_tick);
            match sample(spec, &motion.start, &motion.end, elapsed) {
                Ok((value, done)) => {
                    self.displayed.insert(target.clone(), Some(value));
                    if done {
                        finished.push(target.clone());
                    }
                }
                Err(error) => {
                    report.diagnostics.push(SessionDiagnostic::Endpoint {
                        motion: motion.motion,
                        target: target.clone(),
                        error,
                    });
                    self.displayed
                        .insert(target.clone(), Some(motion.end.clone()));
                    finished.push(target.clone());
                }
            }
        }
        for target in finished {
            self.active.remove(&target);
        }
        report.active_motions = self.active.len();
        self.record_trace(tick);
        report
    }

    fn record_trace(&mut self, tick: Tick) {
        let out = &mut self.trace;
        out.extend_from_slice(&tick.to_le_bytes());
        out.extend_from_slice(&(self.displayed.len() as u32).to_le_bytes());
        for (target, value) in &self.displayed {
            out.extend_from_slice(&target.to_canonical_bytes());
            put_value(out, value.as_ref());
        }
        let state = self
            .store
            .state(self.session, &self.root)
            .cloned()
            .unwrap_or_default();
        out.extend_from_slice(&(state.len() as u32).to_le_bytes());
        for (var, value) in &state {
            out.extend_from_slice(&var.stable_id().to_bytes());
            put_value(out, value.as_ref());
        }
    }
}

fn put_value(out: &mut Vec<u8>, value: Option<&TypedValue>) {
    match value {
        Some(v) => {
            let bytes = v.to_bytes();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        None => out.extend_from_slice(&u32::MAX.to_le_bytes()),
    }
}

fn base_value(evaluator: &BindingEvaluator, target: &PropertyPath) -> Option<TypedValue> {
    evaluator
        .value(&DependencyNode::Property(target.clone()))
        .and_then(|v| v.as_ref().ok().cloned())
        .flatten()
}
