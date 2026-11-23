//! Context provenance 종단 시험(XXX.7.7): 문서 해석 → binding graph → provenance chain.

mod context_common;

use context_common::*;
use stellar_bindings::context::ContextSource;
use stellar_bindings::graph::{ContextProvisionNode, DependencyNode};
use stellar_document::context::fixtures::world::*;
use stellar_document::context::fixtures::*;
use stellar_document::context::resolve::ContextSite;
use stellar_document::context::validate::ContextConsumer;

fn consumer(binding: u64, site: ContextSite, key: u64) -> ContextConsumer {
    ContextConsumer {
        binding: sid(binding),
        site,
        expected: number_type(),
        source: context_read(binding, sid(key)),
    }
}

#[test]
fn provenance_chain_lists_provisions_from_final_value_to_key() {
    // 검증 기준(XXX.7.7): 최종값 → provision → key chain, default 해석·slot 경유 표시.
    let (model, library) = sample_world();
    let deep = consumer(
        1,
        ContextSite::virtual_node(
            vec![sid(CARD_INSTANCE), sid(CARD_BUTTON)],
            sid(BUTTON_FRAME),
        ),
        N,
    );
    let default = consumer(2, ContextSite::scene(sid(FRAME)), D);
    let slot = consumer(3, ContextSite::scene(sid(CONTENT)), D);
    let consumers = vec![deep.clone(), default.clone(), slot.clone()];
    let graph = context_graph(&model, &library, &[], &consumers);
    let evaluator = evaluator(&graph, &consumers);

    let trace = evaluator
        .context_provenance(&consumer_node(&deep), &graph)
        .unwrap();
    // 최종값(15) ← Card root provision(인스턴스 chain [Card 인스턴스]) ← 같은 key 바깥 값 View root provision.
    assert_eq!(trace.providers(), vec![sid(CARD_ROOT), sid(ROOT)]);
    assert_eq!(trace.hops.len(), 2);
    assert_eq!(trace.hops[0].key, sid(N));
    assert_eq!(
        trace.hops[0].read,
        DependencyNode::ContextProvision(ContextProvisionNode {
            provision: sid(P_CARD_N),
            instances: vec![sid(CARD_INSTANCE)],
        })
    );
    assert_eq!(
        trace.hops[1].source,
        ContextSource::Provision {
            provision: sid(P_ROOT_N),
            provider: sid(ROOT),
            instances: Vec::new(),
        }
    );
    // chain의 마지막 단계는 View root provision의 리터럴(원본)입니다.
    assert!(trace
        .chain
        .nodes()
        .contains(&&DependencyNode::ContextProvision(ContextProvisionNode {
            provision: sid(P_ROOT_N),
            instances: Vec::new(),
        })));

    let trace = evaluator
        .context_provenance(&consumer_node(&default), &graph)
        .unwrap();
    assert_eq!(trace.hops[0].source, ContextSource::Default);
    assert_eq!(trace.hops[0].read, DependencyNode::ContextKey(sid(D)));
    assert!(trace.providers().is_empty());

    let trace = evaluator
        .context_provenance(&consumer_node(&slot), &graph)
        .unwrap();
    assert_eq!(trace.hops[0].via_slots, vec![sid(BODY)]);
    assert_eq!(trace.providers(), vec![sid(CARD_HOST)]);
}
