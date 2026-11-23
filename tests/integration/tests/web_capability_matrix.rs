//! 속성 registry → Web capability matrix 대조 (IX.11.19, ADR-0025 결정 1.3).
//!
//! target-profile은 문서 registry를 읽지 못하므로(crate 그래프) 이 test가 registry 항목과 layout longhand를
//! `css_matrix::web_capability`에 넣어 봅니다.

use std::collections::BTreeSet;

use stellar_document::css::mapping::MappingStatus;
use stellar_document::css::registry;
use stellar_document::layout_record::{LayoutLonghand, LayoutRole};
use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::css_matrix::{
    mapping_table, web_capability, Basis, MappingInput, RegistryInput, CAPABILITY_CONDITIONS,
    EXTENSIONS, STELLAR_ONLY_OUTPUTS,
};

fn input_status(status: MappingStatus) -> MappingInput {
    match status {
        MappingStatus::Exact => MappingInput::Exact,
        MappingStatus::Converted => MappingInput::Converted,
        MappingStatus::Approximated => MappingInput::Approximated,
        MappingStatus::StellarOnly => MappingInput::StellarOnly,
    }
}

/// registry 항목과 layout longhand 전체: (범위, 이름, 상태).
fn subjects() -> Vec<(&'static str, &'static str, MappingInput)> {
    let mut out: Vec<_> = registry::entries()
        .into_iter()
        .map(|e| (e.scope.name(), e.name, input_status(e.css.status)))
        .collect();
    for longhand in LayoutLonghand::ALL {
        let scope = match longhand.role() {
            LayoutRole::Item => "node",
            LayoutRole::Container => "frame",
        };
        // payload 크기 key를 쓰는 width·height는 이미 registry 항목으로 있습니다.
        if !out
            .iter()
            .any(|(s, n, _)| *s == scope && *n == longhand.name())
        {
            out.push((scope, longhand.name(), MappingInput::Exact));
        }
    }
    out
}

#[test]
fn every_registry_item_has_a_web_status_derived_by_the_mapping_table() {
    let subjects = subjects();
    assert!(subjects.len() >= 90, "{}", subjects.len());
    for (scope, name, status) in &subjects {
        let capability = web_capability(RegistryInput {
            scope,
            name,
            status: *status,
        });
        assert_ne!(
            capability.status,
            CapabilityStatus::Deprecated,
            "{scope} {name}"
        );
        if capability.basis == Basis::MappingTable || capability.basis == Basis::NoOutput {
            assert_eq!(
                capability.status,
                mapping_table(*status, false),
                "{scope} {name}: 대응표와 다름"
            );
        }
        if let Basis::StellarOnlyRule(_) = capability.basis {
            assert_eq!(capability.status, mapping_table(*status, true));
        }
    }
}

#[test]
fn override_tables_name_only_existing_registry_items() {
    let names: BTreeSet<(&str, &str)> = subjects().iter().map(|(s, n, _)| (*s, *n)).collect();
    for (scope, name, id, status) in EXTENSIONS {
        assert!(
            names.contains(&(*scope, *name)),
            "{id}: {scope} {name}이 registry에 없습니다"
        );
        if ["E1", "E2", "E8", "E9"].contains(id) {
            assert_eq!(*status, CapabilityStatus::Approximated, "{id} {name}");
        }
    }
    for (scope, name, _, _) in CAPABILITY_CONDITIONS {
        assert!(names.contains(&(*scope, *name)), "{scope} {name}");
    }
    for (scope, name, _) in STELLAR_ONLY_OUTPUTS {
        let found = subjects()
            .into_iter()
            .find(|(s, n, _)| s == scope && n == name)
            .unwrap_or_else(|| panic!("{scope} {name}이 registry에 없습니다"));
        assert_eq!(found.2, MappingInput::StellarOnly, "{scope} {name}");
    }
}
