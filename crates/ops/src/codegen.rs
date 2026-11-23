//! 문서 값 → 코드 생성 IR 입력 (XXII.1.2, XXII.1.4, ADR-0106 결정 1).
//!
//! codegen은 문서 crate를 읽지 못하므로(crate 그래프) 이 module이 문서 registry를 codegen 지역 입력으로 옮깁니다.
//! 새 판정·formatter를 두지 않고 문서 registry와 layout record 이름을 그대로 씁니다.

use stellar_codegen::ir::CssPropertyUse;
use stellar_document::css::mapping::MappingStatus;
use stellar_document::css::registry::{self, RegistryEntry};
use stellar_document::layout_record::{LayoutLonghand, LayoutRole};
use stellar_target_profile::css_matrix::MappingInput;

use crate::core_types::property::PropertyPath;

/// 문서 CSS 매핑 상태 → codegen 입력 상태.
pub fn mapping_input(status: MappingStatus) -> MappingInput {
    match status {
        MappingStatus::Exact => MappingInput::Exact,
        MappingStatus::Converted => MappingInput::Converted,
        MappingStatus::Approximated => MappingInput::Approximated,
        MappingStatus::StellarOnly => MappingInput::StellarOnly,
    }
}

/// registry 항목 하나의 사용(값 경로가 있으면 함께).
pub fn css_property_use(entry: &RegistryEntry, path: Option<PropertyPath>) -> CssPropertyUse {
    CssPropertyUse {
        scope: entry.scope.name(),
        name: entry.name,
        status: mapping_input(entry.css.status),
        path,
    }
}

/// layout longhand 하나의 사용. item longhand는 `node`, container longhand는 `frame` 범위입니다(IX.11.19 matrix와
/// 같은 대응). layout longhand는 CSS longhand 그대로이므로 매핑 상태는 `exact`입니다. registry 항목과 이름이 같으면
/// registry 상태를 씁니다.
pub fn layout_property_use(longhand: LayoutLonghand) -> CssPropertyUse {
    let scope = match longhand.role() {
        LayoutRole::Item => "node",
        LayoutRole::Container => "frame",
    };
    let status = registry::entries()
        .iter()
        .find(|e| e.scope.name() == scope && e.name == longhand.name())
        .map_or(MappingInput::Exact, |e| mapping_input(e.css.status));
    CssPropertyUse {
        scope,
        name: longhand.name(),
        status,
        path: None,
    }
}

/// registry 항목 전체와 layout longhand 전체(중복 없이, registry 순서 다음 longhand 순서).
pub fn all_css_subjects() -> Vec<CssPropertyUse> {
    let mut out: Vec<CssPropertyUse> = registry::entries()
        .iter()
        .map(|entry| css_property_use(entry, None))
        .collect();
    for longhand in LayoutLonghand::ALL {
        let item = layout_property_use(longhand);
        if !out
            .iter()
            .any(|u| u.scope == item.scope && u.name == item.name)
        {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_codegen::matrix::css_entry;
    use stellar_target_profile::css_matrix::{web_capability, RegistryInput};
    use stellar_target_profile::profile::TargetProfileKind;

    fn input(css: &CssPropertyUse) -> RegistryInput<'static> {
        RegistryInput {
            scope: css.scope,
            name: css.name,
            status: css.status,
        }
    }

    #[test]
    fn every_registry_item_has_apple_and_android_status_and_rule() {
        // 검증 기준(XXII.1.4): Apple·Android profile의 모든 registry 항목에 상태·규칙이 있는지 완전성 검사.
        let subjects = all_css_subjects();
        assert!(subjects.len() >= 110, "{}", subjects.len());
        let mut missing = Vec::new();
        for css in &subjects {
            for kind in [TargetProfileKind::Apple, TargetProfileKind::Android] {
                let entry = css_entry(input(css), kind);
                if entry.status.is_none() {
                    missing.push(format!("{}/{} {}", css.scope, css.name, kind.name()));
                }
            }
        }
        assert!(missing.is_empty(), "규칙 없는 항목: {missing:?}");
    }

    #[test]
    fn web_status_equals_the_ix_11_19_matrix_for_every_registry_item() {
        // 검증 기준(XXII.1.4): Web 항목 상태가 IX.11.19 matrix와 같음.
        for css in all_css_subjects() {
            assert_eq!(
                css_entry(input(&css), TargetProfileKind::Web).status,
                Some(web_capability(input(&css)).status),
                "{}/{}",
                css.scope,
                css.name
            );
        }
    }

    #[test]
    fn rule_table_names_only_existing_registry_items() {
        let subjects = all_css_subjects();
        for row in stellar_codegen::matrix::css::CSS_LOWERINGS {
            assert!(
                subjects
                    .iter()
                    .any(|u| u.scope == row.scope && u.name == row.name),
                "{}/{}는 registry에 없습니다",
                row.scope,
                row.name
            );
        }
    }
}
