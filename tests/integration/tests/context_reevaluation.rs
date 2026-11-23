//! Context 무효화 조건별 증분 재평가 종단 시험(XXX.7.6).
//!
//! 문서 변경 → `affected_consumers`(문서 crate)로 영향 소비를 고르고 → 그 소비만 경로표를 다시 써서 binding scheduler
//! (bindings crate)가 위상 순서로 재평가합니다. 결과를 전체 재해석·재평가와 비교합니다.

mod context_common;

use context_common::*;
use stellar_bindings::scheduler::StaticSources;
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::{ComponentLibrary, ComponentSlotId};
use stellar_document::component::instance::SlotOverride;
use stellar_document::context::fixtures::world::*;
use stellar_document::context::fixtures::*;
use stellar_document::context::invalidate::{affected_consumers, ContextChange};
use stellar_document::context::key::{ContextKey, ContextScope};
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::context::resolve::{ContextResolver, ContextSite};
use stellar_document::context::validate::ContextConsumer;
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};

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

fn consumers() -> Vec<ContextConsumer> {
    vec![
        consumer(1, ContextSite::scene(sid(FRAME)), N),
        consumer(2, ContextSite::scene(sid(CONTENT)), N),
        consumer(3, ContextSite::scene(sid(CONTENT)), D),
        consumer(4, inside(&[CARD_INSTANCE, CARD_BUTTON], BUTTON_FRAME), N),
        consumer(5, ContextSite::scene(sid(FRAME)), D),
        consumer(6, inside(&[CARD_INSTANCE], CARD_LABEL), D),
        consumer(7, ContextSite::scene(sid(CONTENT_CHILD)), D),
    ]
}

fn reparent(model: &mut DocumentModel, node: u64, parent: u64, payload: NodePayload) {
    model.remove_leaf(sid(node)).unwrap();
    insert(model, node, parent, payload);
}

fn insert(model: &mut DocumentModel, node: u64, parent: u64, payload: NodePayload) {
    let order = model.next_child_order_key(sid(parent));
    model
        .insert_node(NodeRecord {
            id: sid(node),
            parent: Some(sid(parent)),
            order_key: order,
            visible: true,
            locked: false,
            display_name: String::new(),
            payload,
        })
        .unwrap();
}

fn frame() -> NodePayload {
    NodePayload::Frame(FramePayload::default())
}

/// 증분 결과가 전체 재평가와 같고, 영향 밖 소비는 평가하지 않았는지 확인합니다. 평가 순서를 돌려줍니다.
fn check(
    before: &(DocumentModel, ComponentLibrary),
    after: &(DocumentModel, ComponentLibrary),
    change: ContextChange,
    consumers: &[ContextConsumer],
    expected_affected: &[u64],
) -> Vec<String> {
    let graph0 = context_graph(&before.0, &before.1, &[], consumers);
    let mut incremental = evaluator(&graph0, consumers);
    let resolver = ContextResolver::new(&before.0, &before.1);
    let affected = affected_consumers(&resolver, &change, consumers);
    let expected: Vec<StableId> = expected_affected.iter().map(|n| sid(*n)).collect();
    assert_eq!(affected, expected, "{change:?}");
    let graph1 = context_graph(&after.0, &after.1, &[], consumers);
    let report = incremental
        .apply_context_change(
            route_change(&graph0, &graph1, &affected),
            &StaticSources::new(),
        )
        .unwrap();
    let full = evaluator(&graph1, consumers);
    for consumer in consumers {
        assert_eq!(
            value(&incremental, consumer),
            value(&full, consumer),
            "{change:?} binding {}",
            consumer.binding
        );
        // 영향 밖 소비 binding은 평가하지 않습니다.
        if !affected.contains(&consumer.binding) {
            let index = incremental
                .graph()
                .index_of(&consumer_node(consumer))
                .unwrap();
            assert!(
                !report.order.contains(&index),
                "{change:?} {}",
                consumer.binding
            );
        }
    }
    report
        .order
        .iter()
        .map(|index| incremental.node_at(*index).to_string())
        .collect()
}

fn set_provision(
    world: &mut (DocumentModel, ComponentLibrary),
    id: u64,
    owner: ContextOwner,
    provider: u64,
    key: u64,
    value: f32,
) {
    let set = world.0.context_mut();
    set.remove_provision(sid(id));
    set.insert_provision(
        ContextProvision::new(
            sid(id),
            owner,
            sid(provider),
            sid(key),
            number(id + 50_000, value),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn provision_and_default_changes_reevaluate_only_affected_consumers() {
    let consumers = consumers();
    // provision 추가.
    let before = sample_world();
    let mut after = sample_world();
    set_provision(
        &mut after,
        5000,
        ContextOwner::View(sid(VIEW)),
        FRAME,
        N,
        20.0,
    );
    check(
        &before,
        &after,
        ContextChange::ProvisionAdded {
            provider: sid(FRAME),
            key: sid(N),
        },
        &consumers,
        &[1],
    );
    // provision 삭제(바깥 값을 다시 읽는 Card provision을 거치는 소비 포함).
    let mut after = sample_world();
    after.0.context_mut().remove_provision(sid(P_ROOT_N));
    check(
        &before,
        &after,
        ContextChange::ProvisionRemoved {
            provision: sid(P_ROOT_N),
        },
        &consumers,
        &[1, 2, 4],
    );
    // provision 값 변경.
    let mut after = sample_world();
    set_provision(
        &mut after,
        P_HOST_D,
        ContextOwner::Component(sid(CARD)),
        CARD_HOST,
        D,
        4.0,
    );
    check(
        &before,
        &after,
        ContextChange::ProvisionValueChanged {
            provision: sid(P_HOST_D),
        },
        &consumers,
        &[3, 7],
    );
    // ContextKey default 변경.
    let mut after = sample_world();
    let key = after.0.context().key(sid(D)).unwrap().clone();
    after
        .0
        .context_mut()
        .replace_key(
            ContextKey::new(
                key.id(),
                key.name(),
                ContextScope::Document(sid(DOC)),
                number_type(),
                number(77, 9.0),
            )
            .unwrap(),
        )
        .unwrap();
    check(
        &before,
        &after,
        ContextChange::KeyDefaultChanged { key: sid(D) },
        &consumers,
        &[5],
    );
}

#[test]
fn structural_changes_reevaluate_only_consumers_in_the_changed_subtree() {
    let consumers = consumers();
    let before = sample_world();
    // reparent: FRAME를 View 최상위 slot 콘텐츠 아래로(이제 body 공개 D를 봄).
    let mut after = sample_world();
    reparent(&mut after.0, FRAME, CONTENT, frame());
    check(
        &before,
        &after,
        ContextChange::NodesMoved {
            nodes: vec![sid(FRAME)],
        },
        &consumers,
        &[1, 5],
    );
    // slot 콘텐츠 삭제: CONTENT_CHILD를 slot 콘텐츠 밖으로.
    let mut after = sample_world();
    reparent(&mut after.0, CONTENT_CHILD, ROOT, frame());
    check(
        &before,
        &after,
        ContextChange::SlotContentChanged {
            content: vec![sid(CONTENT_CHILD)],
        },
        &consumers,
        &[7],
    );
    // Slot 공개 목록 변경: body가 D를 더는 공개하지 않음.
    let mut after = sample_world();
    after.0.context_mut().remove_slot_key(sid(BODY), sid(D));
    check(
        &before,
        &after,
        ContextChange::SlotKeysChanged { slot: sid(BODY) },
        &consumers,
        &[2, 3, 7],
    );
    // View Variant 구조 patch 적용(FRAME 이동)과 해제(되돌림).
    let mut patched = sample_world();
    reparent(&mut patched.0, FRAME, CONTENT, frame());
    let applied = check(
        &before,
        &patched,
        ContextChange::VariantStructure {
            nodes: vec![sid(FRAME)],
        },
        &consumers,
        &[1, 5],
    );
    let released = check(
        &patched,
        &before,
        ContextChange::VariantStructure {
            nodes: vec![sid(FRAME)],
        },
        &consumers,
        &[1, 5],
    );
    // 평가 순서는 결정적입니다.
    assert_eq!(
        applied,
        check(
            &before,
            &patched,
            ContextChange::VariantStructure {
                nodes: vec![sid(FRAME)],
            },
            &consumers,
            &[1, 5],
        )
    );
    assert!(!released.is_empty());
    // Fragment 묶기·풀기: FRAME을 새 Fragment(D = 6 공급)로 묶음.
    let mut wrapped = sample_world();
    insert(
        &mut wrapped.0,
        60,
        ROOT,
        NodePayload::Fragment(FragmentPayload),
    );
    reparent(&mut wrapped.0, FRAME, 60, frame());
    set_provision(
        &mut wrapped,
        5001,
        ContextOwner::View(sid(VIEW)),
        60,
        D,
        6.0,
    );
    let fragment = ContextChange::FragmentChanged {
        nodes: vec![sid(60), sid(FRAME)],
    };
    check(&before, &wrapped, fragment.clone(), &consumers, &[1, 5]);
    check(&wrapped, &before, fragment, &consumers, &[1, 5]);
}

#[test]
fn instance_component_swap_reevaluates_slot_content_consumers() {
    // instance ComponentRef 교체: Card 인스턴스를 Button으로(콘텐츠는 label slot으로). Card 내부 소비는 사라지므로 뺍니다.
    let consumers: Vec<ContextConsumer> = consumers()
        .into_iter()
        .filter(|c| c.site.instances.is_empty())
        .collect();
    let before = sample_world();
    let mut after = sample_world();
    let mut payload = instance_of(sid(BUTTON), &[]);
    payload.set_slot_override(
        ComponentSlotId::new(sid(LABEL)),
        SlotOverride::new(vec![sid(CONTENT)]).unwrap(),
    );
    after
        .0
        .set_payload(sid(CARD_INSTANCE), NodePayload::ComponentInstance(payload))
        .unwrap();
    check(
        &before,
        &after,
        ContextChange::InstanceSwapped {
            instance: sid(CARD_INSTANCE),
        },
        &consumers,
        &[2, 3, 7],
    );
}
