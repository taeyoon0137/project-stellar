//! Context 해석을 바꾸는 변경과 영향받는 소비 binding (XXX.7.6, ADR-0099 결정 6).
//!
//! 소비 binding 하나의 **footprint**는 그 해석이 지나간 (key, node) 쌍, 인스턴스 chain의 instance node, 해석 결과로 쓴
//! provision, default로 끝난 key, 지나간 Slot입니다. provision 값 식이 다른 key를 읽으면 그 해석도 footprint에 더합니다(전이).
//! 변경마다 footprint와 겹치는 소비 binding만 다시 해석·평가 대상입니다. 무관한 subtree의 소비는 footprint가 겹치지 않아
//! 대상에서 빠집니다.
//!
//! | 변경 | 겹치는 조건 |
//! |---|---|
//! | provision 추가(provider X, key K) | (K, X)를 지나감 |
//! | provision 삭제·값 변경 | 그 provision을 씀 |
//! | ContextKey default 변경 | 그 key가 default로 끝남 |
//! | node reparent·이동·삭제, instance ComponentRef 교체, slot 콘텐츠 삽입·삭제·재정렬, View Variant 구조 patch 적용·해제, Fragment 묶기·풀기 | 바뀐 node를 지나가거나 chain에 가짐 |
//! | Slot 공개 목록 변경 | 그 Slot을 지나감(공개 경유 또는 건너뜀) |

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;

use super::resolve::{
    ContextOrigin, ContextResolution, ContextResolver, ContextSite, ResolveError,
};
use super::validate::ContextConsumer;

/// Context 해석을 바꿀 수 있는 변경(XXX.7.6 무효화 조건).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextChange {
    /// provision 추가.
    ProvisionAdded {
        /// provider node.
        provider: StableId,
        /// key.
        key: StableId,
    },
    /// provision 삭제.
    ProvisionRemoved {
        /// provision.
        provision: StableId,
    },
    /// provision 값 변경.
    ProvisionValueChanged {
        /// provision.
        provision: StableId,
    },
    /// ContextKey default 변경.
    KeyDefaultChanged {
        /// key.
        key: StableId,
    },
    /// 소비 node나 그 조상의 reparent·이동·삭제.
    NodesMoved {
        /// 바뀐 node.
        nodes: Vec<StableId>,
    },
    /// instance의 ComponentRef 교체.
    InstanceSwapped {
        /// instance node.
        instance: StableId,
    },
    /// slot 콘텐츠 삽입·삭제·재정렬.
    SlotContentChanged {
        /// 바뀐 콘텐츠 node.
        content: Vec<StableId>,
    },
    /// Slot 공개 목록 변경.
    SlotKeysChanged {
        /// Slot.
        slot: StableId,
    },
    /// View Variant 구조 patch(node add·delete·move·replace, slot change)의 적용·해제.
    VariantStructure {
        /// patch 대상 node.
        nodes: Vec<StableId>,
    },
    /// Fragment 묶기·풀기.
    FragmentChanged {
        /// Fragment와 그 자식.
        nodes: Vec<StableId>,
    },
}

/// 소비 binding 하나의 해석 footprint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextFootprint {
    /// 지나간 (key, node).
    pub visited: BTreeSet<(StableId, StableId)>,
    /// 지나간 node.
    pub nodes: BTreeSet<StableId>,
    /// 쓴 provision.
    pub provisions: BTreeSet<StableId>,
    /// default로 끝난 key.
    pub defaults: BTreeSet<StableId>,
    /// 지나간 Slot.
    pub slots: BTreeSet<StableId>,
}

impl ContextFootprint {
    fn add(&mut self, resolution: &ContextResolution) {
        for site in &resolution.path {
            self.visited.insert((resolution.key, site.node));
            self.nodes.insert(site.node);
            self.nodes.extend(site.instances.iter().copied());
        }
        self.slots.extend(resolution.via_slots.iter().copied());
        self.slots.extend(resolution.skipped_slots.iter().copied());
        match &resolution.origin {
            ContextOrigin::Default => {
                self.defaults.insert(resolution.key);
            }
            ContextOrigin::Provision { provision, .. } => {
                self.provisions.insert(*provision);
            }
        }
    }

    /// 변경과 겹치는지.
    pub fn touched_by(&self, change: &ContextChange) -> bool {
        let any = |nodes: &[StableId]| nodes.iter().any(|n| self.nodes.contains(n));
        match change {
            ContextChange::ProvisionAdded { provider, key } => {
                self.visited.contains(&(*key, *provider))
            }
            ContextChange::ProvisionRemoved { provision }
            | ContextChange::ProvisionValueChanged { provision } => {
                self.provisions.contains(provision)
            }
            ContextChange::KeyDefaultChanged { key } => self.defaults.contains(key),
            ContextChange::NodesMoved { nodes }
            | ContextChange::SlotContentChanged { content: nodes }
            | ContextChange::VariantStructure { nodes }
            | ContextChange::FragmentChanged { nodes } => any(nodes),
            ContextChange::InstanceSwapped { instance } => self.nodes.contains(instance),
            ContextChange::SlotKeysChanged { slot } => self.slots.contains(slot),
        }
    }
}

/// 소비 binding의 footprint(provision 값 식의 Context 읽기까지 전이).
pub fn footprint(
    resolver: &ContextResolver<'_>,
    consumer: &ContextConsumer,
) -> Result<ContextFootprint, ResolveError> {
    let mut out = ContextFootprint::default();
    let mut pending: Vec<ContextResolution> = Vec::new();
    for key in consumer.reads() {
        pending.push(resolver.resolve(key, &consumer.site)?);
    }
    let mut seen: BTreeSet<(StableId, Vec<StableId>)> = BTreeSet::new();
    while let Some(resolution) = pending.pop() {
        out.add(&resolution);
        if let ContextOrigin::Provision {
            provision,
            provider,
            instances,
        } = &resolution.origin
        {
            if !seen.insert((*provision, instances.clone())) {
                continue;
            }
            let Some(provision) = resolver.context().provision(*provision) else {
                continue;
            };
            let site = ContextSite::virtual_node(instances.clone(), *provider);
            for read in provision.reads() {
                pending.push(resolver.resolve_provision_read(provision, &site, read)?);
            }
        }
    }
    Ok(out)
}

/// 변경 전 해석기 기준으로 변경과 footprint가 겹치는 소비 binding(입력 순서, 중복 없음). 해석에 실패한 소비는 대상에
/// 넣습니다(다시 해석해 진단을 갱신해야 하므로).
pub fn affected_consumers(
    before: &ContextResolver<'_>,
    change: &ContextChange,
    consumers: &[ContextConsumer],
) -> Vec<StableId> {
    let mut out = Vec::new();
    for consumer in consumers {
        let touched = match footprint(before, consumer) {
            Ok(print) => print.touched_by(change),
            Err(_) => true,
        };
        if touched && !out.contains(&consumer.binding) {
            out.push(consumer.binding);
        }
    }
    out
}
