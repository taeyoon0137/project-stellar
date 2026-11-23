//! 코드 생성 lowering matrix (XXII.1.2, XXII.1.4, XXII.1.7, ADR-0106 결정 3).
//!
//! CSS 기반 속성 registry 항목 × profile kind의 capability 상태와 lowering 규칙을 한 곳에 둡니다. IR lowering과
//! adapter는 이 모듈만 참조합니다.
//!
//! - Web 항목은 IX.11.19 matrix(`stellar_target_profile::css_matrix::web_capability`)를 그대로 씁니다(다시 정의하지
//!   않음).

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::css_matrix::{web_capability, Basis, RegistryInput};
use stellar_target_profile::profile::TargetProfileKind;

/// matrix 항목 하나: 상태, lowering 규칙, 짝 진단.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixEntry {
    /// capability 상태(`None` = 이 kind에 등록되지 않음).
    pub status: Option<CapabilityStatus>,
    /// lowering 규칙 id.
    pub rule: &'static str,
    /// 규칙과 짝인 진단 code(1:1이면 `None`).
    pub diagnostic: Option<&'static str>,
}

impl MatrixEntry {
    /// 등록되지 않은 항목.
    pub const NOT_REGISTERED: MatrixEntry = MatrixEntry {
        status: None,
        rule: "not-registered",
        diagnostic: Some("mapping-not-registered"),
    };
}

/// Web 규칙 id: 같은 이름 CSS 속성.
pub const WEB_SAME_NAME: &str = "same-name-css";

/// Web 항목(IX.11.19 matrix 결과의 근거를 규칙·진단으로 옮김).
fn web_entry(input: RegistryInput<'_>) -> MatrixEntry {
    let capability = web_capability(input);
    let (rule, diagnostic) = match capability.basis {
        Basis::MappingTable => (
            WEB_SAME_NAME,
            match capability.status {
                CapabilityStatus::Native => None,
                _ => Some("css-approximated"),
            },
        ),
        Basis::Extension(id) => (extension_rule(id), Some("css-extension")),
        Basis::Capability(_) => ("css-capability-condition", None),
        Basis::StellarOnlyRule(_) => ("css-structural-output", Some("css-structural-output")),
        Basis::NoOutput => ("css-no-output", Some("css-no-output")),
    };
    MatrixEntry {
        status: Some(capability.status),
        rule,
        diagnostic,
    }
}

fn extension_rule(id: &'static str) -> &'static str {
    match id {
        "E1" => "css-extension-e1-linear-composite",
        "E2" => "css-extension-e2-wrapper-split",
        "E5" => "css-extension-e5-explicit-initial",
        "E7" => "css-extension-e7-svg-geometry",
        "E9" => "css-extension-e9-aspect-ratio",
        "E10" => "css-extension-e10-explicit-min",
        _ => "css-extension",
    }
}

/// CSS 기반 속성 registry 항목 하나의 profile kind 항목.
pub fn css_entry(input: RegistryInput<'_>, kind: TargetProfileKind) -> MatrixEntry {
    match kind {
        TargetProfileKind::Web => web_entry(input),
        TargetProfileKind::Apple | TargetProfileKind::Android => MatrixEntry::NOT_REGISTERED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_target_profile::css_matrix::MappingInput;

    fn input(
        scope: &'static str,
        name: &'static str,
        status: MappingInput,
    ) -> RegistryInput<'static> {
        RegistryInput {
            scope,
            name,
            status,
        }
    }

    #[test]
    fn web_entries_follow_the_ix_11_19_matrix() {
        for (scope, name, status) in [
            ("frame", "background-color", MappingInput::Exact),
            ("modifier", "opacity", MappingInput::Exact),
            ("modifier", "boolean-operation", MappingInput::StellarOnly),
            ("node", "locked", MappingInput::StellarOnly),
            ("frame", "clipping", MappingInput::Converted),
        ] {
            let entry = css_entry(input(scope, name, status), TargetProfileKind::Web);
            assert_eq!(
                entry.status,
                Some(web_capability(input(scope, name, status)).status),
                "{scope} {name}"
            );
        }
        let plain = css_entry(
            input("frame", "background-color", MappingInput::Exact),
            TargetProfileKind::Web,
        );
        assert_eq!(plain.rule, WEB_SAME_NAME);
        assert_eq!(plain.diagnostic, None);
        let none = css_entry(
            input("node", "locked", MappingInput::StellarOnly),
            TargetProfileKind::Web,
        );
        assert_eq!(none.status, Some(CapabilityStatus::Unsupported));
        assert_eq!(none.diagnostic, Some("css-no-output"));
    }
}
