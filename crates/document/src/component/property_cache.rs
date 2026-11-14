//! Component Property runtime 평가 값의 instance별 캐시 (IX.1.4).
//!
//! scene 인스턴스 하나의 Property 값은 "공개 property override가 있으면 그 값, 없으면 정의 default의 평가 값"입니다
//! (IX.1.2·IX.2.3). [`InstancePropertyCache`]는 그 결과를 **인스턴스별로** 저장하고 두 revision으로 무효화합니다:
//!
//! - **definition revision**([`PropertyRevisions::definition`]): 컴포넌트 정의(Property 선언·record·default)가 바뀔
//!   때 올립니다. 그 컴포넌트를 참조하는 모든 인스턴스 캐시가 무효가 됩니다.
//! - **override revision**([`PropertyRevisions::overrides`]): 인스턴스 payload(PropertyOverride map·참조 컴포넌트)가
//!   바뀔 때 올립니다. 그 인스턴스 캐시만 무효가 되고 다른 인스턴스 캐시는 그대로입니다.
//!
//! 캐시 항목은 계산 당시의 (컴포넌트, definition revision, override revision)을 기억하고, 조회 시 현재 revision과
//! 같을 때만 재사용합니다(hit에는 scene payload를 디코딩하지 않음). revision은 런타임 값이며 저장하지 않습니다.
//! [`PropertyRevisions::record_property_command`]·[`PropertyRevisions::record_override_command`]가 IX.1.3·IX.2.4
//! command에 맞는 revision을 올립니다.
//!
//! 값 규칙: 선언 순서의 property마다 직접 override가 있고 그 property가 공개이며 값 타입이 맞으면 override, 아니면
//! default 값입니다(정의에서 사라진 property의 orphan override, 규칙을 어긴 파일 값은 무시).

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;

use super::definition::{ComponentLibrary, ComponentPropertyId};
use super::expansion::{ExpansionError, SceneIndex};
use super::instance::OverrideKey;
use super::override_commands::PropertyOverrideCommand;
use super::property_commands::PropertyCommand;

/// 컴포넌트 정의·인스턴스 override revision(런타임 전용).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyRevisions {
    definitions: BTreeMap<ComponentRef, u64>,
    overrides: BTreeMap<StableId, u64>,
}

impl PropertyRevisions {
    /// 모든 revision이 0인 상태.
    pub fn new() -> PropertyRevisions {
        PropertyRevisions::default()
    }

    /// 컴포넌트 정의 revision(기록이 없으면 0).
    pub fn definition(&self, component: ComponentRef) -> u64 {
        self.definitions.get(&component).copied().unwrap_or(0)
    }

    /// 인스턴스 override revision(기록이 없으면 0).
    pub fn overrides(&self, instance: StableId) -> u64 {
        self.overrides.get(&instance).copied().unwrap_or(0)
    }

    /// 컴포넌트 정의 revision을 올리고 새 값을 돌려줍니다.
    pub fn bump_definition(&mut self, component: ComponentRef) -> u64 {
        let revision = self.definitions.entry(component).or_insert(0);
        *revision = revision.wrapping_add(1);
        *revision
    }

    /// 인스턴스 override revision을 올리고 새 값을 돌려줍니다.
    pub fn bump_overrides(&mut self, instance: StableId) -> u64 {
        let revision = self.overrides.entry(instance).or_insert(0);
        *revision = revision.wrapping_add(1);
        *revision
    }

    /// 적용한 Property command(IX.1.3)의 컴포넌트 정의 revision을 올립니다.
    pub fn record_property_command(&mut self, command: &PropertyCommand) -> u64 {
        self.bump_definition(command.component())
    }

    /// 적용한 override command(IX.2.4)의 인스턴스 override revision을 올립니다.
    pub fn record_override_command(&mut self, command: &PropertyOverrideCommand) -> u64 {
        self.bump_overrides(command.instance())
    }
}

/// 값 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrigin {
    /// 인스턴스 PropertyOverride.
    Override,
    /// 정의 default.
    Default,
}

/// 평가된 property 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyValue {
    /// 값(`None`이면 nullable property의 값 없음).
    pub value: Option<TypedValue>,
    /// 출처.
    pub origin: ValueOrigin,
}

/// 인스턴스 하나의 평가 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedProperties {
    component: ComponentRef,
    definition_revision: u64,
    override_revision: u64,
    values: BTreeMap<ComponentPropertyId, PropertyValue>,
}

impl EvaluatedProperties {
    /// 인스턴스가 참조한 컴포넌트.
    pub fn component(&self) -> ComponentRef {
        self.component
    }

    /// 계산 당시 definition revision.
    pub fn definition_revision(&self) -> u64 {
        self.definition_revision
    }

    /// 계산 당시 override revision.
    pub fn override_revision(&self) -> u64 {
        self.override_revision
    }

    /// property 값.
    pub fn get(&self, property: ComponentPropertyId) -> Option<&PropertyValue> {
        self.values.get(&property)
    }

    /// property id 오름차순 값.
    pub fn iter(&self) -> impl Iterator<Item = (ComponentPropertyId, &PropertyValue)> + '_ {
        self.values.iter().map(|(id, value)| (*id, value))
    }

    /// property 수.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// 캐시 조회 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyCacheError {
    /// 인스턴스·payload를 해석할 수 없습니다.
    Expansion(ExpansionError),
    /// 참조 컴포넌트가 library에 없습니다.
    UnresolvedComponent {
        /// 인스턴스.
        instance: StableId,
        /// 참조 컴포넌트.
        component: ComponentRef,
    },
}

impl fmt::Display for PropertyCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyCacheError::Expansion(err) => write!(f, "{err}"),
            PropertyCacheError::UnresolvedComponent {
                instance,
                component,
            } => write!(
                f,
                "인스턴스 {instance}의 컴포넌트 {}를 찾을 수 없습니다",
                component.id()
            ),
        }
    }
}

impl std::error::Error for PropertyCacheError {}

impl From<ExpansionError> for PropertyCacheError {
    fn from(err: ExpansionError) -> PropertyCacheError {
        PropertyCacheError::Expansion(err)
    }
}

/// 캐시 통계(테스트·계측용).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// 재사용 횟수.
    pub hits: u64,
    /// 인스턴스 평가(재계산) 횟수.
    pub evaluations: u64,
}

/// scene 인스턴스별 Property 평가 값 캐시.
#[derive(Debug, Clone, Default)]
pub struct InstancePropertyCache {
    entries: HashMap<StableId, EvaluatedProperties>,
    stats: CacheStats,
}

impl InstancePropertyCache {
    /// 빈 캐시.
    pub fn new() -> InstancePropertyCache {
        InstancePropertyCache::default()
    }

    /// 통계.
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// 캐시 항목 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 인스턴스 캐시가 현재 revision에서 유효한지(재계산 없이 판정).
    pub fn is_fresh(&self, revisions: &PropertyRevisions, instance: StableId) -> bool {
        self.entries
            .get(&instance)
            .is_some_and(|entry| is_fresh(entry, revisions, instance))
    }

    /// 인스턴스 캐시를 지웁니다(인스턴스 삭제 등).
    pub fn forget(&mut self, instance: StableId) -> bool {
        self.entries.remove(&instance).is_some()
    }

    /// 인스턴스의 Property 값. 유효한 캐시가 있으면 재사용하고, 없거나 revision이 바뀌었으면 다시 평가합니다.
    pub fn values(
        &mut self,
        library: &ComponentLibrary,
        scene: &SceneIndex<'_>,
        revisions: &PropertyRevisions,
        instance: StableId,
    ) -> Result<&EvaluatedProperties, PropertyCacheError> {
        if self.is_fresh(revisions, instance) {
            self.stats.hits += 1;
        } else {
            let evaluated = evaluate(library, scene, revisions, instance)?;
            self.stats.evaluations += 1;
            self.entries.insert(instance, evaluated);
        }
        self.entries
            .get(&instance)
            .ok_or(PropertyCacheError::Expansion(
                ExpansionError::NodeNotFound { id: instance },
            ))
    }

    /// 인스턴스의 property 하나의 값(property가 정의에 없으면 `None`).
    pub fn value(
        &mut self,
        library: &ComponentLibrary,
        scene: &SceneIndex<'_>,
        revisions: &PropertyRevisions,
        instance: StableId,
        property: ComponentPropertyId,
    ) -> Result<Option<PropertyValue>, PropertyCacheError> {
        Ok(self
            .values(library, scene, revisions, instance)?
            .get(property)
            .cloned())
    }
}

fn is_fresh(
    entry: &EvaluatedProperties,
    revisions: &PropertyRevisions,
    instance: StableId,
) -> bool {
    entry.definition_revision == revisions.definition(entry.component)
        && entry.override_revision == revisions.overrides(instance)
}

fn evaluate(
    library: &ComponentLibrary,
    scene: &SceneIndex<'_>,
    revisions: &PropertyRevisions,
    instance: StableId,
) -> Result<EvaluatedProperties, PropertyCacheError> {
    let payload = scene.instance_payload(instance)?;
    let component = payload.component();
    let entry = library
        .get(component)
        .ok_or(PropertyCacheError::UnresolvedComponent {
            instance,
            component,
        })?;
    let mut values = BTreeMap::new();
    for property in entry.definition().properties() {
        let Some(record) = entry.property(*property) else {
            continue;
        };
        let overridden = payload
            .property_override(&OverrideKey::direct(*property))
            .filter(|value| record.is_public() && record.value_type().accepts(Some(value)));
        let value = match overridden {
            Some(value) => PropertyValue {
                value: Some(value.clone()),
                origin: ValueOrigin::Override,
            },
            None => PropertyValue {
                value: record.default_value().cloned(),
                origin: ValueOrigin::Default,
            },
        };
        values.insert(*property, value);
    }
    Ok(EvaluatedProperties {
        component,
        definition_revision: revisions.definition(component),
        override_revision: revisions.overrides(instance),
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::drilling::DrillingSet;
    use crate::component::expansion::ScenePayloads;
    use crate::component::instance::ComponentInstancePayload;
    use crate::component::override_rules::fixtures::*;
    use crate::component::typed_expr::test_util::literal;
    use crate::node_columns::{NodeColumns, NodeFields};
    use crate::node_kind::NodeKind;
    use stellar_core_types::order_key::OrderKey;

    const FIRST: u64 = 1001;
    const SECOND: u64 = 1002;

    /// scene: 1000(Frame) ← 1001·1002(Card 인스턴스, override 없음).
    fn two_cards() -> (NodeColumns, ScenePayloads) {
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
        let mut payloads = ScenePayloads::new();
        for (id, key) in [(FIRST, 0x40u8), (SECOND, 0x60)] {
            columns.insert(NodeFields {
                stable_id: stable(id),
                kind: NodeKind::ComponentInstance,
                parent: Some(root),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: String::new(),
            });
            payloads.insert(stable(id), ComponentInstancePayload::new(card()).encode());
        }
        (columns, payloads)
    }

    fn title(
        cache: &mut InstancePropertyCache,
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &ScenePayloads,
        revisions: &PropertyRevisions,
        instance: u64,
    ) -> PropertyValue {
        let scene = SceneIndex::build(columns, payloads);
        cache
            .value(library, &scene, revisions, stable(instance), pid(TITLE))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn override_change_invalidates_only_that_instance() {
        // 검증 기준(IX.1.4): 한 instance override 변경 시 다른 instance cache 유지.
        let library = library();
        let (columns, mut payloads) = two_cards();
        let mut revisions = PropertyRevisions::new();
        let mut cache = InstancePropertyCache::new();
        for id in [FIRST, SECOND] {
            let value = title(&mut cache, &library, &columns, &payloads, &revisions, id);
            assert_eq!(value.value, Some(TypedValue::String("Card".into())));
            assert_eq!(value.origin, ValueOrigin::Default);
        }
        assert_eq!(cache.stats().evaluations, 2);

        // FIRST override 변경 → FIRST revision만 증가.
        let command = PropertyOverrideCommand::Set {
            instance: stable(FIRST),
            property: pid(TITLE),
            value: TypedValue::String("주문".into()),
        };
        let undo = command.apply(&library, &columns, &mut payloads).unwrap();
        revisions.record_override_command(&command);
        assert!(!cache.is_fresh(&revisions, stable(FIRST)));
        assert!(
            cache.is_fresh(&revisions, stable(SECOND)),
            "다른 인스턴스 유지"
        );

        let before = cache.stats();
        let second = title(
            &mut cache, &library, &columns, &payloads, &revisions, SECOND,
        );
        assert_eq!(second.value, Some(TypedValue::String("Card".into())));
        assert_eq!(
            cache.stats().evaluations,
            before.evaluations,
            "SECOND 재계산 없음"
        );
        assert_eq!(cache.stats().hits, before.hits + 1);
        let first = title(&mut cache, &library, &columns, &payloads, &revisions, FIRST);
        assert_eq!(first.value, Some(TypedValue::String("주문".into())));
        assert_eq!(first.origin, ValueOrigin::Override);
        assert_eq!(cache.stats().evaluations, before.evaluations + 1);

        // undo(reset) → 다시 default, SECOND는 여전히 hit.
        undo.apply(&library, &columns, &mut payloads).unwrap();
        revisions.record_override_command(&undo);
        let first = title(&mut cache, &library, &columns, &payloads, &revisions, FIRST);
        assert_eq!(first.origin, ValueOrigin::Default);
        let evaluations = cache.stats().evaluations;
        title(
            &mut cache, &library, &columns, &payloads, &revisions, SECOND,
        );
        assert_eq!(cache.stats().evaluations, evaluations);
    }

    #[test]
    fn definition_change_invalidates_all_instances_of_that_component() {
        let mut library = library();
        let (columns, payloads) = two_cards();
        let mut revisions = PropertyRevisions::new();
        let mut cache = InstancePropertyCache::new();
        for id in [FIRST, SECOND] {
            title(&mut cache, &library, &columns, &payloads, &revisions, id);
        }
        // 다른 컴포넌트(Button) 정의 변경은 Card 인스턴스 캐시를 건드리지 않습니다.
        revisions.bump_definition(button());
        assert!(cache.is_fresh(&revisions, stable(FIRST)));
        // Card default 변경(IX.1.3) → 두 인스턴스 모두 재계산.
        let mut drilling = DrillingSet::new();
        let command = PropertyCommand::SetDefault {
            component: card(),
            property: pid(TITLE),
            default: literal(TypedValue::String("카드".into())),
        };
        command.apply(&mut library, &mut drilling).unwrap();
        revisions.record_property_command(&command);
        assert!(!cache.is_fresh(&revisions, stable(FIRST)));
        assert!(!cache.is_fresh(&revisions, stable(SECOND)));
        let before = cache.stats().evaluations;
        for id in [FIRST, SECOND] {
            let value = title(&mut cache, &library, &columns, &payloads, &revisions, id);
            assert_eq!(value.value, Some(TypedValue::String("카드".into())));
        }
        assert_eq!(cache.stats().evaluations, before + 2);
        // revision이 그대로면 hit만.
        let before = cache.stats();
        for id in [FIRST, SECOND] {
            title(&mut cache, &library, &columns, &payloads, &revisions, id);
        }
        assert_eq!(cache.stats().evaluations, before.evaluations);
        assert_eq!(cache.stats().hits, before.hits + 2);
        assert_eq!(cache.len(), 2);
        assert!(cache.forget(stable(FIRST)));
        assert!(!cache.is_fresh(&revisions, stable(FIRST)));
    }

    #[test]
    fn invalid_or_orphan_overrides_fall_back_to_default() {
        let library = library();
        let (columns, mut payloads) = two_cards();
        let mut payload = ComponentInstancePayload::new(card());
        // 타입이 틀린 파일 값과 정의에 없는 property의 orphan override.
        payload.set_property_override(OverrideKey::direct(pid(TITLE)), TypedValue::Boolean(true));
        payload.set_property_override(
            OverrideKey::direct(pid(9_999)),
            TypedValue::String("orphan".into()),
        );
        payloads.insert(stable(FIRST), payload.encode());
        let revisions = PropertyRevisions::new();
        let mut cache = InstancePropertyCache::new();
        let scene = SceneIndex::build(&columns, &payloads);
        let values = cache
            .values(&library, &scene, &revisions, stable(FIRST))
            .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values.get(pid(TITLE)).unwrap().origin, ValueOrigin::Default);
        assert!(values.get(pid(9_999)).is_none());
        // 인스턴스가 아닌 node는 오류.
        assert!(matches!(
            cache.values(&library, &scene, &revisions, stable(1000)),
            Err(PropertyCacheError::Expansion(
                ExpansionError::NotAnInstance { .. }
            ))
        ));
    }
}
