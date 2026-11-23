//! 무효화 조건별 영향 소비 시험(XXX.7.6).

use stellar_core_types::ids::StableId;

use super::fixtures::world::*;
use super::fixtures::*;
use super::invalidate::{affected_consumers, ContextChange};
use super::resolve::{ContextResolver, ContextSite};
use super::validate::ContextConsumer;

fn consumer(binding: u64, site: ContextSite, key: u64) -> ContextConsumer {
    ContextConsumer {
        binding: sid(binding),
        site,
        expected: number_type(),
        source: context_read(binding, sid(key)),
    }
}

fn inside(chain: &[u64], node: u64) -> ContextSite {
    ContextSite::virtual_node(chain.iter().map(|c| sid(*c)).collect(), sid(node))
}

/// c1 FRAME·N, c2 CONTENT·N, c3 CONTENT·D, c4 Button Frame·N, c5 FRAME·D(default), c6 CARD_LABEL·D.
pub(crate) fn consumers() -> Vec<ContextConsumer> {
    vec![
        consumer(1, ContextSite::scene(sid(FRAME)), N),
        consumer(2, ContextSite::scene(sid(CONTENT)), N),
        consumer(3, ContextSite::scene(sid(CONTENT)), D),
        consumer(4, inside(&[CARD_INSTANCE, CARD_BUTTON], BUTTON_FRAME), N),
        consumer(5, ContextSite::scene(sid(FRAME)), D),
        consumer(6, inside(&[CARD_INSTANCE], CARD_LABEL), D),
    ]
}

#[test]
fn each_invalidation_condition_selects_only_consumers_whose_resolution_it_can_change() {
    // 검증 기준(XXX.7.6): 조건별 영향 소비 binding, 무관 subtree 제외.
    let (model, library) = sample_world();
    let resolver = ContextResolver::new(&model, &library);
    let consumers = consumers();
    let cases: Vec<(ContextChange, Vec<u64>)> = vec![
        (
            ContextChange::ProvisionAdded {
                provider: sid(FRAME),
                key: sid(N),
            },
            vec![1],
        ),
        (
            ContextChange::ProvisionAdded {
                provider: sid(CARD_BUTTON),
                key: sid(N),
            },
            vec![4],
        ),
        (
            ContextChange::ProvisionRemoved {
                provision: sid(P_ROOT_N),
            },
            vec![1, 2, 4],
        ),
        (
            ContextChange::ProvisionValueChanged {
                provision: sid(P_HOST_D),
            },
            vec![3],
        ),
        (ContextChange::KeyDefaultChanged { key: sid(D) }, vec![5]),
        (
            ContextChange::NodesMoved {
                nodes: vec![sid(FRAME)],
            },
            vec![1, 5],
        ),
        (
            ContextChange::InstanceSwapped {
                instance: sid(CARD_INSTANCE),
            },
            vec![2, 3, 4, 6],
        ),
        (
            ContextChange::SlotContentChanged {
                content: vec![sid(CONTENT)],
            },
            vec![2, 3],
        ),
        (
            ContextChange::SlotKeysChanged { slot: sid(BODY) },
            vec![2, 3],
        ),
        (ContextChange::SlotKeysChanged { slot: sid(LABEL) }, vec![6]),
        (
            ContextChange::VariantStructure {
                nodes: vec![sid(CARD_FRAME)],
            },
            vec![],
        ),
        (
            ContextChange::FragmentChanged {
                nodes: vec![sid(ROOT)],
            },
            vec![1, 2, 4, 5],
        ),
    ];
    for (change, expected) in cases {
        let affected = affected_consumers(&resolver, &change, &consumers);
        let expected: Vec<StableId> = expected.into_iter().map(sid).collect();
        assert_eq!(affected, expected, "{change:?}");
    }
}
