//! Context validator 시험(XXX.7.5).

use stellar_core_types::ids::StableId;
use stellar_expressions::types::TypeSpec;

use super::fixtures::world::*;
use super::fixtures::*;
use super::key::{ContextKey, ContextScope};
use super::provision::{ContextOwner, ContextProvision};
use super::resolve::{ContextOrigin, ContextResolver, ContextSite, NoInputs};
use super::set::ContextKeys;
use super::validate::{
    validate_context, ContextConsumer, ContextDiagnostic, ContextDiagnosticKind, ContextSeverity,
};

fn consumer(binding: u64, site: ContextSite, key: u64, expected: TypeSpec) -> ContextConsumer {
    ContextConsumer {
        binding: sid(binding),
        site,
        expected,
        source: context_read(binding, sid(key)),
    }
}

fn inside(chain: &[u64], node: u64) -> ContextSite {
    ContextSite::virtual_node(chain.iter().map(|c| sid(*c)).collect(), sid(node))
}

fn kinds(diagnostics: &[ContextDiagnostic]) -> Vec<ContextDiagnosticKind> {
    diagnostics.iter().map(|d| d.kind.clone()).collect()
}

#[test]
fn diagnostics_cover_declaration_types_orphans_unused_and_info() {
    // 검증 기준(XXX.7.5): diagnostic fixture 정확, View 최상위 소비 허용, View 최상위 slot 콘텐츠는 바깥 값, 정의 안
    // slot 콘텐츠의 선언 없는 소비 오류.
    let (mut model, library) = sample_world();
    {
        let set = model.context_mut();
        set.add_requirement(sid(CARD), sid(E)); // 쓰지 않는 requirement(경고).
        set.add_requirement(sid(CARD), sid(777)); // 없는 key(orphan).
        set.add_slot_key(sid(778), sid(N)); // 없는 Slot(orphan).
        set.insert_provision(
            ContextProvision::new(
                sid(3000),
                ContextOwner::View(sid(VIEW)),
                sid(FRAME),
                sid(779),
                number(3000, 1.0),
            )
            .unwrap(),
        )
        .unwrap(); // 없는 key provision(orphan).
        set.insert_provision(
            ContextProvision::new(
                sid(3001),
                ContextOwner::Component(sid(CARD)),
                sid(BUTTON_FRAME),
                sid(D),
                number(3001, 1.0),
            )
            .unwrap(),
        )
        .unwrap(); // Card가 Button 내부 node에 붙인 provision(범위 밖 오류).
    }
    let consumers = vec![
        // View 최상위: 선언 필요 없음, View root provision.
        consumer(4000, ContextSite::scene(sid(FRAME)), N, number_type()),
        // View 최상위, default 해석(정보).
        consumer(4001, ContextSite::scene(sid(FRAME)), D, number_type()),
        // Card 안: 선언된 N, Card root가 View root를 가림(정보).
        consumer(4002, inside(&[CARD_INSTANCE], CARD_FRAME), N, number_type()),
        // Card가 Button slot에 작성한 콘텐츠: Card가 D를 선언하지 않음(오류).
        consumer(4003, inside(&[CARD_INSTANCE], CARD_LABEL), D, number_type()),
        // View 최상위에서 작성한 slot 콘텐츠: 선언 필요 없음.
        consumer(4004, ContextSite::scene(sid(CONTENT)), N, number_type()),
        // 타입 불일치(Number key를 Color 속성에).
        consumer(4005, ContextSite::scene(sid(FRAME)), N, color_type()),
        // 없는 key 소비(orphan).
        consumer(4006, ContextSite::scene(sid(FRAME)), 780, number_type()),
    ];
    let keys = ContextKeys::new(model.context());
    let diagnostics = validate_context(&model, &library, &keys, &consumers, &NoInputs);
    let mut expected = vec![
        ContextDiagnosticKind::UndeclaredConsumption {
            component: sid(CARD),
            key: sid(D),
            reader: sid(4003),
        },
        ContextDiagnosticKind::TypeMismatch {
            reader: sid(4005),
            expected: color_type(),
            found: Some(number_type()),
        },
        ContextDiagnosticKind::ProvisionOutsideOwner {
            provision: sid(3001),
            error: format!(
                "provider {}는 component {}의 tree에 있지 않습니다",
                sid(BUTTON_FRAME),
                sid(CARD)
            ),
        },
        ContextDiagnosticKind::OrphanProvision {
            provision: sid(3000),
            key: sid(779),
        },
        ContextDiagnosticKind::OrphanSlotKey {
            slot: sid(778),
            key: sid(N),
        },
        ContextDiagnosticKind::OrphanRequirement {
            component: sid(CARD),
            key: sid(777),
        },
        ContextDiagnosticKind::OrphanConsumption {
            binding: sid(4006),
            key: sid(780),
        },
        ContextDiagnosticKind::UnusedRequirement {
            component: sid(CARD),
            key: sid(E),
        },
        ContextDiagnosticKind::DefaultResolved {
            binding: sid(4001),
            key: sid(D),
        },
        ContextDiagnosticKind::Shadowing {
            binding: sid(4002),
            key: sid(N),
            provision: sid(P_CARD_N),
            shadowed: vec![sid(P_ROOT_N)],
        },
    ];
    expected.sort();
    assert_eq!(kinds(&diagnostics), expected);
    let severities: Vec<ContextSeverity> = diagnostics
        .iter()
        .map(ContextDiagnostic::severity)
        .collect();
    assert!(severities.contains(&ContextSeverity::Error));
    assert!(severities.contains(&ContextSeverity::Orphan));
    assert!(severities.contains(&ContextSeverity::Warning));
    assert!(severities.contains(&ContextSeverity::Info));
    // 같은 입력은 같은 진단.
    assert_eq!(
        validate_context(&model, &library, &keys, &consumers, &NoInputs),
        diagnostics
    );
    // View 최상위에서 작성한 slot 콘텐츠의 N은 Card 내부(15) 대신 instance 바깥 View root provision으로 해석됩니다.
    let resolver = ContextResolver::new(&model, &library);
    assert_eq!(
        resolver
            .resolve(sid(N), &ContextSite::scene(sid(CONTENT)))
            .unwrap()
            .origin,
        ContextOrigin::Provision {
            provision: sid(P_ROOT_N),
            provider: sid(ROOT),
            instances: Vec::new(),
        }
    );
}

#[test]
fn clean_sample_world_has_no_errors_and_mutual_key_reads_are_a_cycle() {
    // 검증 기준(XXX.7.5): K1↔K2 cycle fixture 거부.
    let (mut model, library) = sample_world();
    let keys_set = model.context().clone();
    let keys = ContextKeys::new(&keys_set);
    let clean = validate_context(&model, &library, &keys, &[], &NoInputs);
    assert!(
        clean.iter().all(|d| d.severity() != ContextSeverity::Error),
        "{clean:?}"
    );
    let (x, y) = (sid(950), sid(951));
    {
        let set = model.context_mut();
        for (id, name) in [(x, "x"), (y, "y")] {
            set.insert_key(
                ContextKey::new(
                    id,
                    name,
                    ContextScope::Document(sid(DOC)),
                    number_type(),
                    number(id.to_bytes()[15] as u64 + 7000, 0.0),
                )
                .unwrap(),
            )
            .unwrap();
        }
        set.insert_provision(
            ContextProvision::new(
                sid(3100),
                ContextOwner::View(sid(VIEW)),
                sid(FRAME),
                x,
                context_plus(3100, y, 1.0),
            )
            .unwrap(),
        )
        .unwrap();
        set.insert_provision(
            ContextProvision::new(
                sid(3101),
                ContextOwner::View(sid(VIEW)),
                sid(ROOT),
                y,
                context_plus(3101, x, 1.0),
            )
            .unwrap(),
        )
        .unwrap();
    }
    let keys_set = model.context().clone();
    let keys = ContextKeys::new(&keys_set);
    let diagnostics = validate_context(&model, &library, &keys, &[], &NoInputs);
    // FRAME의 X는 FRAME의 Y가 없어 ROOT의 Y를 읽고, ROOT의 Y가 읽는 X는 ROOT 위에 없어 default라 cycle이 아닙니다.
    assert!(diagnostics
        .iter()
        .all(|d| !matches!(d.kind, ContextDiagnosticKind::ProvisionCycle { .. })));
    // 같은 node에서 서로 읽으면 cycle입니다.
    model.context_mut().remove_provision(sid(3101));
    model
        .context_mut()
        .insert_provision(
            ContextProvision::new(
                sid(3102),
                ContextOwner::View(sid(VIEW)),
                sid(FRAME),
                y,
                context_plus(3102, x, 1.0),
            )
            .unwrap(),
        )
        .unwrap();
    let keys_set = model.context().clone();
    let keys = ContextKeys::new(&keys_set);
    let diagnostics = validate_context(&model, &library, &keys, &[], &NoInputs);
    assert!(
        diagnostics.iter().any(|d| d.kind
            == ContextDiagnosticKind::ProvisionCycle {
                provisions: vec![sid(3100), sid(3102)]
            }),
        "{diagnostics:?}"
    );
    assert_ne!(x, StableId::NIL);
}
