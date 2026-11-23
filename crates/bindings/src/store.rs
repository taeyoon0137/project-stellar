//! live·orphan binding 저장소와 역참조 index (VIII.8.1, XVI.4.8).
//!
//! [`BindingStore`]는 live binding([`BindingSet`])과 orphan binding을 함께 들고, 편집할 때마다 두 index를 함께
//! 갱신합니다(ADR-0108 결정 2).
//!
//! - **토큰 역참조**: 토큰 → 그 토큰을 source에서 읽는 binding(live·orphan). 토큰 삭제의 inbound 검사
//!   ([`TokenOrphanPlan::current`](crate::token_commands::TokenOrphanPlan::current))가 모든 binding source를 훑지
//!   않고 그 토큰을 읽는 binding만 봅니다.
//! - **orphan target**: target 속성 → 그 target의 orphan(ID 순). 참조 타입 조회
//!   ([`StoreTypes`](crate::fallback))가 Property 참조마다 orphan을 선형 검색하지 않습니다.
//!
//! 두 index는 저장소 내용에서 결정적으로 정해지므로 같은 내용의 저장소는 index도 같습니다(`PartialEq`).
//! 바꾸는 경로는 이 타입의 메서드뿐이라 index가 내용과 어긋날 수 없습니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::ReferenceTarget;

use crate::binding::{BindingId, BindingSet, BindingSetError, ValueBinding};
use crate::orphan::{break_cause, BindingWorld, OrphanCause, OrphanedBinding};

/// binding이 저장소의 어느 쪽에 있는지(같은 ID가 두 쪽에 겹친 경우에도 index를 정확히 지우기 위한 구분).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Slot {
    Live,
    Orphan,
}

/// live binding과 orphan binding 저장소 (VIII.8.1). 같은 ID는 둘 중 한 곳에만 있습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingStore {
    live: BindingSet,
    orphans: BTreeMap<BindingId, OrphanedBinding>,
    /// 토큰 → 그 토큰을 읽는 binding(ID, 위치).
    token_readers: BTreeMap<TokenRef, BTreeSet<(BindingId, Slot)>>,
    /// orphan target → 그 target의 orphan ID.
    orphan_targets: BTreeMap<PropertyPath, BTreeSet<BindingId>>,
}

/// binding source가 읽는 토큰(중복 제거, 정렬).
fn read_tokens(binding: &ValueBinding) -> BTreeSet<TokenRef> {
    binding
        .source()
        .referenced_targets()
        .into_iter()
        .filter_map(|target| match target {
            ReferenceTarget::Token(token) => Some(*token),
            _ => None,
        })
        .collect()
}

impl BindingStore {
    /// live binding으로 만듭니다.
    pub fn new(live: BindingSet) -> BindingStore {
        let mut store = BindingStore::default();
        for binding in live.iter() {
            store.index_tokens(binding, Slot::Live);
        }
        store.live = live;
        store
    }

    /// live binding.
    pub fn live(&self) -> &BindingSet {
        &self.live
    }

    /// orphan binding(ID 순).
    pub fn orphans(&self) -> impl Iterator<Item = &OrphanedBinding> {
        self.orphans.values()
    }

    /// orphan 하나.
    pub fn orphan(&self, id: BindingId) -> Option<&OrphanedBinding> {
        self.orphans.get(&id)
    }

    /// `target` 속성의 orphan 중 ID가 가장 작은 것(orphan target index, XVI.4.8).
    pub fn orphan_by_target(&self, target: &PropertyPath) -> Option<&OrphanedBinding> {
        self.orphan_targets
            .get(target)
            .and_then(|ids| ids.first())
            .and_then(|id| self.orphans.get(id))
    }

    /// `token`을 source에서 읽는 binding(live·orphan, ID 순, 토큰 역참조 index, XVI.4.8).
    pub fn bindings_reading(&self, token: TokenRef) -> BTreeSet<BindingId> {
        self.token_readers
            .get(&token)
            .map(|readers| readers.iter().map(|(id, _)| *id).collect())
            .unwrap_or_default()
    }

    /// 전체 binding 수(live + orphan).
    pub fn len(&self) -> usize {
        self.live.len() + self.orphans.len()
    }

    /// binding이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// live binding을 추가합니다. 같은 ID의 orphan이 있거나 live 모음 규칙(ID·target 유일)을 어기면 거부합니다.
    pub fn insert_live(&mut self, binding: ValueBinding) -> Result<(), BindingSetError> {
        if self.orphans.contains_key(&binding.id()) {
            return Err(BindingSetError::DuplicateId(binding.id()));
        }
        let tokens = read_tokens(&binding);
        let id = binding.id();
        self.live.insert(binding)?;
        for token in tokens {
            self.token_readers
                .entry(token)
                .or_default()
                .insert((id, Slot::Live));
        }
        Ok(())
    }

    /// live binding을 꺼냅니다.
    pub(crate) fn remove_live(&mut self, id: BindingId) -> Option<ValueBinding> {
        let binding = self.live.remove(id)?;
        self.unindex_tokens(&binding, Slot::Live);
        Some(binding)
    }

    pub(crate) fn insert_orphan(&mut self, orphan: OrphanedBinding) {
        self.remove_orphan(orphan.id());
        self.index_tokens(orphan.binding(), Slot::Orphan);
        self.orphan_targets
            .entry(orphan.target().clone())
            .or_default()
            .insert(orphan.id());
        self.orphans.insert(orphan.id(), orphan);
    }

    pub(crate) fn remove_orphan(&mut self, id: BindingId) -> Option<OrphanedBinding> {
        let orphan = self.orphans.remove(&id)?;
        self.unindex_tokens(orphan.binding(), Slot::Orphan);
        if let Some(ids) = self.orphan_targets.get_mut(orphan.target()) {
            ids.remove(&id);
            if ids.is_empty() {
                self.orphan_targets.remove(orphan.target());
            }
        }
        Some(orphan)
    }

    /// live binding을 검사해 깨진 binding을 orphan으로 옮기고, 새로 orphan이 된 ID를 돌려줍니다(ID 순). 아무것도
    /// 삭제하지 않습니다.
    pub fn orphan_broken(&mut self, world: &dyn BindingWorld) -> Vec<BindingId> {
        let broken: Vec<(BindingId, OrphanCause)> = self
            .live
            .iter()
            .filter_map(|binding| break_cause(binding, world).map(|cause| (binding.id(), cause)))
            .collect();
        let mut moved = Vec::with_capacity(broken.len());
        for (id, cause) in broken {
            if let Some(binding) = self.remove_live(id) {
                self.insert_orphan(OrphanedBinding::new(binding, cause));
                moved.push(id);
            }
        }
        moved
    }

    fn index_tokens(&mut self, binding: &ValueBinding, slot: Slot) {
        for token in read_tokens(binding) {
            self.token_readers
                .entry(token)
                .or_default()
                .insert((binding.id(), slot));
        }
    }

    fn unindex_tokens(&mut self, binding: &ValueBinding, slot: Slot) {
        for token in read_tokens(binding) {
            if let Some(readers) = self.token_readers.get_mut(&token) {
                readers.remove(&(binding.id(), slot));
                if readers.is_empty() {
                    self.token_readers.remove(&token);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
    use stellar_expressions::functions::FunctionId;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use crate::binding::ScopeOwner;

    /// 결정적 난수(SplitMix64).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    fn prop(n: u64) -> PropertyPath {
        PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1))
    }

    fn number() -> TypeSpec {
        TypeSpec::required(ExpressionType::Number)
    }

    /// 토큰 0~2개의 `max`(없으면 상수, 하나면 그 참조).
    fn source(base: u64, tokens: &[TokenRef]) -> ExpressionAst {
        let node = |k: u64| ExprNodeId::new(sid(base + k));
        let reference =
            |k: u64, token: TokenRef| ExprNode::reference(node(k), ReferenceTarget::Token(token));
        let nodes = match tokens {
            [] => vec![ExprNode::literal(
                node(0),
                TypedValue::Number(FiniteF32::new(1.0).expect("유한")),
            )],
            [only] => vec![reference(0, *only)],
            [first, second, ..] => vec![
                ExprNode::call(node(0), FunctionId::Max, &[node(1), node(2)]),
                reference(1, *first),
                reference(2, *second),
            ],
        };
        ExpressionAst::new(node(0), nodes).expect("유효")
    }

    fn random_binding(rng: &mut Rng, id: u64, target: u64) -> ValueBinding {
        let count = rng.below(3) as usize;
        let mut tokens: Vec<TokenRef> = (0..count).map(|_| token(rng.below(8))).collect();
        tokens.sort();
        tokens.dedup();
        ValueBinding::new(
            BindingId::new(sid(4_000 + id)),
            prop(target),
            source(100_000 + id * 10, &tokens),
            number(),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + target))),
        )
    }

    /// 저장소 전체를 훑어 index와 같은 답을 만듭니다(oracle).
    fn scan_readers(store: &BindingStore, token: TokenRef) -> BTreeSet<BindingId> {
        let target = ReferenceTarget::Token(token);
        store
            .live()
            .iter()
            .chain(store.orphans().map(OrphanedBinding::binding))
            .filter(|binding| binding.refers_to(&target))
            .map(ValueBinding::id)
            .collect()
    }

    fn assert_matches_scan(store: &BindingStore) {
        for n in 0..10 {
            assert_eq!(
                store.bindings_reading(token(n)),
                scan_readers(store, token(n))
            );
        }
        for n in 0..24 {
            let target = prop(n);
            assert_eq!(
                store.orphan_by_target(&target).map(OrphanedBinding::id),
                store
                    .orphans()
                    .find(|orphan| orphan.target() == &target)
                    .map(OrphanedBinding::id)
            );
        }
        // index는 내용에서 결정적으로 정해집니다.
        let mut rebuilt = BindingStore::new(store.live().clone());
        for orphan in store.orphans() {
            rebuilt.insert_orphan(orphan.clone());
        }
        assert_eq!(&rebuilt, store);
    }

    #[test]
    fn indexes_match_full_scans_after_random_edits() {
        let mut checked = 0;
        for seed in 0..60u64 {
            let mut rng = Rng(seed);
            let mut store = BindingStore::default();
            let mut next_id = 0u64;
            for _ in 0..80 {
                match rng.below(5) {
                    0 | 1 => {
                        let target = rng.below(24);
                        let binding = random_binding(&mut rng, next_id, target);
                        next_id += 1;
                        let _ = store.insert_live(binding);
                    }
                    2 => {
                        // live → orphan (VIII.8.4 Orphanize와 같은 이동).
                        let ids: Vec<BindingId> = store.live().iter().map(|b| b.id()).collect();
                        if let Some(&id) = ids.get(rng.below(ids.len().max(1) as u64) as usize) {
                            let binding = store.remove_live(id).expect("live");
                            store.insert_orphan(OrphanedBinding::new(
                                binding,
                                OrphanCause::TargetMissing,
                            ));
                        }
                    }
                    3 => {
                        // orphan → live 재연결 또는 삭제.
                        let ids: Vec<BindingId> = store.orphans().map(|o| o.id()).collect();
                        if let Some(&id) = ids.get(rng.below(ids.len().max(1) as u64) as usize) {
                            let orphan = store.remove_orphan(id).expect("orphan");
                            if rng.below(2) == 0 {
                                let tokens = [token(rng.below(8))];
                                let next = orphan
                                    .binding()
                                    .with_source(source(900_000 + next_id * 10, &tokens), number());
                                next_id += 1;
                                if store.insert_live(next).is_err() {
                                    store.insert_orphan(orphan);
                                }
                            }
                        }
                    }
                    _ => {
                        // 같은 ID의 orphan을 다시 넣어도(교체) index가 남지 않아야 합니다.
                        let first = store.orphans().next().cloned();
                        if let Some(orphan) = first {
                            store.insert_orphan(orphan);
                        }
                    }
                }
                assert_matches_scan(&store);
                checked += 1;
            }
        }
        assert!(checked >= 4_000);
    }
}
