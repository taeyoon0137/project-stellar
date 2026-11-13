//! preview session별 runtime 상태 store (IX.5.4).
//!
//! 상태 머신의 현재 값은 **preview session 메모리**에만 있습니다(기술 아키텍처 16장: "프리뷰 중 사용자가
//! 클릭해 바뀐 상태는 세션 메모리에만 남고 프로젝트나 Git을 변경하지 않습니다"). 프로젝트 저장에는 상태
//! 머신 정의와 initial expression만 들어갑니다([`ComponentLibrary::encode`]).
//!
//! - 이 store에는 직렬화 API가 없습니다. 저장·복구 snapshot 경로가 store를 읽을 방법이 없습니다.
//! - 값은 (session, 인스턴스 chain, 상태 변수) 단위입니다. 인스턴스 chain은 scene 인스턴스부터 정의 tree
//!   안 nested instance node까지입니다(expansion의 `ExpandedNodeId::Virtual` chain과 같은 규칙).
//! - 초기화는 각 변수의 initial expression을 인스턴스의 **유효 Property 값**(override, 없으면 정의 default)
//!   으로 평가합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::eval::EvalError;
use stellar_expressions::types::TypeSpec;

use super::definition::{ComponentEntry, ComponentLibrary};
use super::expansion::{ExpansionError, SceneIndex};
use super::instance::{ComponentInstancePayload, OverrideKey};
use super::state_machine::StateVariableId;

/// preview session 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewSessionId(StableId);

impl PreviewSessionId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> PreviewSessionId {
        PreviewSessionId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// 상태를 가진 인스턴스 위치: scene 인스턴스부터 nested instance node까지의 chain.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceKey(Vec<StableId>);

impl InstanceKey {
    /// scene 인스턴스 하나.
    pub fn scene(instance: StableId) -> InstanceKey {
        InstanceKey(vec![instance])
    }

    /// chain(바깥 → 안쪽).
    pub fn chain(chain: Vec<StableId>) -> InstanceKey {
        InstanceKey(chain)
    }

    /// chain.
    pub fn ids(&self) -> &[StableId] {
        &self.0
    }
}

/// 인스턴스 하나의 상태 값(변수 id 오름차순). `None`은 nullable 변수의 값 없음입니다.
pub type InstanceState = BTreeMap<StateVariableId, Option<TypedValue>>;

/// store 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateStoreError {
    /// 같은 session이 이미 열려 있습니다.
    SessionExists {
        /// session.
        session: PreviewSessionId,
    },
    /// 열리지 않은 session입니다.
    UnknownSession {
        /// session.
        session: PreviewSessionId,
    },
    /// 컴포넌트가 library에 없습니다.
    ComponentNotFound {
        /// 컴포넌트.
        component: ComponentRef,
    },
    /// 인스턴스 해석 실패.
    Expansion(ExpansionError),
    /// 초기화되지 않은 인스턴스입니다.
    UnknownInstance {
        /// 인스턴스.
        instance: InstanceKey,
    },
    /// 상태 머신에 없는 변수입니다.
    UnknownVariable {
        /// 변수.
        variable: StateVariableId,
    },
    /// 값 타입이 변수 타입과 다릅니다.
    TypeMismatch {
        /// 변수.
        variable: StateVariableId,
        /// 선언 타입.
        expected: TypeSpec,
    },
    /// initial expression 평가 실패.
    Initial {
        /// 변수.
        variable: StateVariableId,
        /// 원인.
        error: EvalError,
    },
}

impl fmt::Display for StateStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateStoreError::SessionExists { session } => {
                write!(
                    f,
                    "preview session {}이 이미 열려 있습니다",
                    session.stable_id()
                )
            }
            StateStoreError::UnknownSession { session } => {
                write!(f, "preview session {}이 없습니다", session.stable_id())
            }
            StateStoreError::ComponentNotFound { component } => {
                write!(f, "컴포넌트 {}가 없습니다", component.id())
            }
            StateStoreError::Expansion(err) => write!(f, "{err}"),
            StateStoreError::UnknownInstance { instance } => {
                write!(f, "상태가 초기화되지 않은 인스턴스: {:?}", instance.ids())
            }
            StateStoreError::UnknownVariable { variable } => {
                write!(f, "상태 변수 {}가 없습니다", variable.stable_id())
            }
            StateStoreError::TypeMismatch { variable, expected } => write!(
                f,
                "상태 변수 {} 값이 선언 타입 {expected}와 맞지 않습니다",
                variable.stable_id()
            ),
            StateStoreError::Initial { variable, error } => write!(
                f,
                "상태 변수 {} initial 평가 실패: {error}",
                variable.stable_id()
            ),
        }
    }
}

impl std::error::Error for StateStoreError {}

impl From<ExpansionError> for StateStoreError {
    fn from(err: ExpansionError) -> StateStoreError {
        StateStoreError::Expansion(err)
    }
}

/// 인스턴스의 유효 Property 값: 타입이 맞는 direct override, 없으면 정의 default.
///
/// 표현식 참조 값 형식(`ReferenceTarget::ComponentProperty` → 값)으로 돌려줍니다.
pub fn effective_property_values(
    entry: &ComponentEntry,
    payload: &ComponentInstancePayload,
) -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
    entry
        .property_records()
        .map(|record| {
            let value = payload
                .property_override(&OverrideKey::direct(record.id()))
                .filter(|v| record.value_type().accepts(Some(v)))
                .or(record.default_value())
                .cloned();
            (
                ReferenceTarget::ComponentProperty(record.id().stable_id()),
                value,
            )
        })
        .collect()
}

/// 모든 preview session의 runtime 상태.
#[derive(Debug, Clone, Default)]
pub struct StateStore {
    sessions: BTreeMap<PreviewSessionId, BTreeMap<InstanceKey, InstanceState>>,
}

impl StateStore {
    /// 빈 store.
    pub fn new() -> StateStore {
        StateStore::default()
    }

    /// session을 엽니다.
    pub fn open_session(&mut self, session: PreviewSessionId) -> Result<(), StateStoreError> {
        if self.sessions.contains_key(&session) {
            return Err(StateStoreError::SessionExists { session });
        }
        self.sessions.insert(session, BTreeMap::new());
        Ok(())
    }

    /// session을 닫고 그 상태를 모두 버립니다. 열려 있지 않았으면 `false`.
    pub fn close_session(&mut self, session: PreviewSessionId) -> bool {
        self.sessions.remove(&session).is_some()
    }

    /// 열린 session 수.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// `component` 인스턴스 `key`의 상태를 initial expression으로 (다시) 초기화합니다.
    ///
    /// `properties`는 그 인스턴스의 유효 Property 값입니다([`effective_property_values`]). 한 변수라도
    /// 평가에 실패하면 기존 상태를 바꾸지 않습니다.
    pub fn initialize(
        &mut self,
        session: PreviewSessionId,
        library: &ComponentLibrary,
        component: ComponentRef,
        key: InstanceKey,
        properties: &BTreeMap<ReferenceTarget, Option<TypedValue>>,
    ) -> Result<&InstanceState, StateStoreError> {
        let entry = library
            .get(component)
            .ok_or(StateStoreError::ComponentNotFound { component })?;
        let mut state = InstanceState::new();
        for variable in entry.state_machine().variables() {
            let value = variable.initial().evaluate(properties).map_err(|error| {
                StateStoreError::Initial {
                    variable: variable.id(),
                    error,
                }
            })?;
            state.insert(variable.id(), value);
        }
        let instances = self
            .sessions
            .get_mut(&session)
            .ok_or(StateStoreError::UnknownSession { session })?;
        instances.insert(key.clone(), state);
        instances
            .get(&key)
            .ok_or(StateStoreError::UnknownInstance { instance: key })
    }

    /// scene 인스턴스의 상태를 초기화합니다(유효 Property 값은 scene payload에서 계산).
    pub fn initialize_scene_instance(
        &mut self,
        session: PreviewSessionId,
        library: &ComponentLibrary,
        scene: &SceneIndex<'_>,
        instance: StableId,
    ) -> Result<&InstanceState, StateStoreError> {
        let payload = scene.instance_payload(instance)?;
        let component = payload.component();
        let entry = library
            .get(component)
            .ok_or(StateStoreError::ComponentNotFound { component })?;
        let properties = effective_property_values(entry, &payload);
        self.initialize(
            session,
            library,
            component,
            InstanceKey::scene(instance),
            &properties,
        )
    }

    /// 인스턴스 상태.
    pub fn state(&self, session: PreviewSessionId, key: &InstanceKey) -> Option<&InstanceState> {
        self.sessions.get(&session).and_then(|s| s.get(key))
    }

    /// 상태 값 하나(`Some(None)`은 nullable 변수의 값 없음).
    pub fn value(
        &self,
        session: PreviewSessionId,
        key: &InstanceKey,
        variable: StateVariableId,
    ) -> Option<&Option<TypedValue>> {
        self.state(session, key).and_then(|s| s.get(&variable))
    }

    /// 여러 값을 **한꺼번에** 씁니다. 하나라도 변수가 없거나 타입이 맞지 않으면 아무것도 바꾸지 않습니다.
    pub fn commit(
        &mut self,
        session: PreviewSessionId,
        library: &ComponentLibrary,
        component: ComponentRef,
        key: &InstanceKey,
        values: &[(StateVariableId, Option<TypedValue>)],
    ) -> Result<(), StateStoreError> {
        let entry = library
            .get(component)
            .ok_or(StateStoreError::ComponentNotFound { component })?;
        for (variable, value) in values {
            let declared = entry
                .state_machine()
                .variable(*variable)
                .ok_or(StateStoreError::UnknownVariable {
                    variable: *variable,
                })?
                .value_type();
            if !declared.accepts(value.as_ref()) {
                return Err(StateStoreError::TypeMismatch {
                    variable: *variable,
                    expected: declared,
                });
            }
        }
        let state = self
            .sessions
            .get_mut(&session)
            .ok_or(StateStoreError::UnknownSession { session })?
            .get_mut(key)
            .ok_or_else(|| StateStoreError::UnknownInstance {
                instance: key.clone(),
            })?;
        for (variable, value) in values {
            state.insert(*variable, value.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::expansion::ScenePayloads;
    use crate::component::state_machine::fixtures::*;
    use crate::node_columns::{NodeColumns, NodeFields};
    use crate::node_kind::NodeKind;
    use stellar_core_types::order_key::OrderKey;

    fn session(ts: u64) -> PreviewSessionId {
        PreviewSessionId::new(stable(ts))
    }

    /// scene: 1000 ← 1001(Toggle, initiallyOn override = `on`).
    fn scene(on: Option<bool>) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let root = columns.insert(NodeFields {
            stable_id: stable(1000),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        columns.insert(NodeFields {
            stable_id: stable(1001),
            kind: NodeKind::ComponentInstance,
            parent: Some(root),
            order_key: OrderKey::from_bytes(vec![0x40]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        let mut payload = ComponentInstancePayload::new(toggle());
        if let Some(on) = on {
            payload.set_property_override(
                OverrideKey::direct(prop(INITIALLY_ON)),
                TypedValue::Boolean(on),
            );
        }
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        (columns, payloads)
    }

    fn library() -> ComponentLibrary {
        let mut library = toggle_library();
        let machine = toggle_machine(&library);
        library.set_state_machine(toggle(), machine).unwrap();
        library
    }

    #[test]
    fn initial_state_uses_instance_property_values() {
        let library = library();
        let mut store = StateStore::new();
        store.open_session(session(1)).unwrap();
        for (on, expected) in [(None, false), (Some(true), true)] {
            let (columns, payloads) = scene(on);
            let index = SceneIndex::build(&columns, &payloads);
            let state = store
                .initialize_scene_instance(session(1), &library, &index, stable(1001))
                .unwrap();
            assert_eq!(
                state.get(&var(IS_ON)),
                Some(&Some(TypedValue::Boolean(expected)))
            );
            assert_eq!(
                state.get(&var(TEXT)),
                Some(&Some(TypedValue::String(String::new())))
            );
        }
    }

    #[test]
    fn preview_changes_do_not_change_saved_bytes() {
        // 검증 기준(IX.5.4): preview 조작 후 save bytes 동일.
        let library = library();
        let (columns, payloads) = scene(None);
        let saved_library = library.encode();
        let saved_scene = payloads.clone();

        let mut store = StateStore::new();
        store.open_session(session(1)).unwrap();
        store.open_session(session(2)).unwrap();
        let index = SceneIndex::build(&columns, &payloads);
        for s in [session(1), session(2)] {
            store
                .initialize_scene_instance(s, &library, &index, stable(1001))
                .unwrap();
        }
        let key = InstanceKey::scene(stable(1001));
        store
            .commit(
                session(1),
                &library,
                toggle(),
                &key,
                &[
                    (var(IS_ON), Some(TypedValue::Boolean(true))),
                    (var(TEXT), Some(TypedValue::String("켜짐".into()))),
                ],
            )
            .unwrap();
        // session마다 독립.
        assert_eq!(
            store.value(session(1), &key, var(IS_ON)),
            Some(&Some(TypedValue::Boolean(true)))
        );
        assert_eq!(
            store.value(session(2), &key, var(IS_ON)),
            Some(&Some(TypedValue::Boolean(false)))
        );
        // 저장 대상(library 정의·scene payload)은 그대로.
        assert_eq!(library.encode(), saved_library);
        assert_eq!(payloads, saved_scene);
        // session을 닫으면 상태도 사라집니다.
        assert!(store.close_session(session(1)));
        assert_eq!(store.state(session(1), &key), None);
        assert_eq!(store.session_count(), 1);
    }

    #[test]
    fn commit_is_all_or_nothing() {
        let library = library();
        let (columns, payloads) = scene(None);
        let index = SceneIndex::build(&columns, &payloads);
        let mut store = StateStore::new();
        store.open_session(session(1)).unwrap();
        store
            .initialize_scene_instance(session(1), &library, &index, stable(1001))
            .unwrap();
        let key = InstanceKey::scene(stable(1001));
        let before = store.state(session(1), &key).cloned();
        // 두 번째 값 타입이 틀리면 첫 번째 값도 쓰지 않습니다.
        assert!(matches!(
            store.commit(
                session(1),
                &library,
                toggle(),
                &key,
                &[
                    (var(IS_ON), Some(TypedValue::Boolean(true))),
                    (var(TEXT), Some(TypedValue::Boolean(true))),
                ],
            ),
            Err(StateStoreError::TypeMismatch { .. })
        ));
        assert_eq!(store.state(session(1), &key).cloned(), before);
        assert!(matches!(
            store.commit(session(1), &library, toggle(), &key, &[(var(999), None)]),
            Err(StateStoreError::UnknownVariable { .. })
        ));
        assert!(matches!(
            store.open_session(session(1)),
            Err(StateStoreError::SessionExists { .. })
        ));
        assert!(matches!(
            store.commit(
                session(9),
                &library,
                toggle(),
                &key,
                &[(var(IS_ON), Some(TypedValue::Boolean(true)))]
            ),
            Err(StateStoreError::UnknownSession { .. })
        ));
    }
}
