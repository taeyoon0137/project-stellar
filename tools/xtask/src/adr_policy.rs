//! ADR 보호 경로 정책 (I.3.5).
//!
//! storage format, public API, FFI, numeric registry, performance budget 변경은
//! 같은 diff에 새·수정 ADR이 없으면 실패합니다. docs-only 변경은 오탐을 내지 않습니다.

/// ADR 정책 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdrError {
    pub code: &'static str,
    pub message: String,
}

impl AdrError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        AdrError {
            code,
            message: message.into(),
        }
    }
}

/// 보호 경로 prefix(공개 API·numeric registry는 core-types, storage format, FFI, 스키마).
const PROTECTED_PREFIXES: [&str; 4] = [
    "crates/core-types/",
    "crates/storage/",
    "crates/ffi/",
    "schemas/",
];

/// 보호 경로(정확히 일치). performance budget과, crate 전체가 아니라 파일 하나에 모인 저장 형식·numeric
/// registry입니다(ADR-0026: `ValueBinding` 형식, canonical AST 형식, 환경 입력 key·enum registry).
const PROTECTED_EXACT: [&str; 4] = [
    "config/performance-budgets.yaml",
    "crates/bindings/src/binding.rs",
    "crates/expressions/src/codec.rs",
    "crates/expressions/src/environment.rs",
];

fn is_protected(path: &str) -> bool {
    PROTECTED_PREFIXES.iter().any(|p| path.starts_with(p)) || PROTECTED_EXACT.contains(&path)
}

/// `docs/adr/ADR-*.md`(TEMPLATE 제외)인지 확인합니다.
fn is_adr(path: &str) -> bool {
    path.starts_with("docs/adr/") && path.ends_with(".md") && path != "docs/adr/TEMPLATE.md"
}

/// 변경 경로 집합에 ADR 정책을 적용합니다(빈 Vec = 통과).
pub fn check_adr_policy(changed_paths: &[&str]) -> Vec<AdrError> {
    let protected: Vec<&str> = changed_paths
        .iter()
        .copied()
        .filter(|p| is_protected(p))
        .collect();
    let has_adr = changed_paths.iter().any(|p| is_adr(p));

    let mut errors = Vec::new();
    if !protected.is_empty() && !has_adr {
        errors.push(AdrError::new(
            "missing-adr",
            format!("보호 경로 변경에 ADR이 없습니다: {}", protected.join(", ")),
        ));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(paths: &[&str]) -> Vec<&'static str> {
        check_adr_policy(paths).iter().map(|e| e.code).collect()
    }

    #[test]
    fn protected_change_without_adr_fails() {
        assert!(codes(&["crates/storage/src/format.rs"]).contains(&"missing-adr"));
        assert!(codes(&["crates/ffi/src/lib.rs"]).contains(&"missing-adr"));
        assert!(codes(&["config/performance-budgets.yaml"]).contains(&"missing-adr"));
    }

    #[test]
    fn binding_and_expression_format_files_are_protected() {
        // ADR-0026: ValueBinding 형식, canonical AST 형식, 환경 입력 registry.
        for path in [
            "crates/bindings/src/binding.rs",
            "crates/expressions/src/codec.rs",
            "crates/expressions/src/environment.rs",
        ] {
            assert!(codes(&[path]).contains(&"missing-adr"), "{path}");
            assert!(check_adr_policy(&[path, "docs/adr/ADR-0026-x.md"]).is_empty());
        }
        // 형식·registry가 아닌 평가·command 코드는 보호 경로가 아닙니다.
        assert!(check_adr_policy(&[
            "crates/bindings/src/fallback.rs",
            "crates/expressions/src/vm.rs",
        ])
        .is_empty());
    }

    #[test]
    fn protected_change_with_adr_passes() {
        assert!(
            check_adr_policy(&["crates/ffi/src/lib.rs", "docs/adr/ADR-0001-ffi.md",]).is_empty()
        );
    }

    #[test]
    fn docs_only_change_has_no_false_positive() {
        assert!(check_adr_policy(&["docs/guide.md", "README.md"]).is_empty());
        // TEMPLATE 변경만으로는 보호 경로가 아니므로 정책 위반 아님.
        assert!(check_adr_policy(&["docs/adr/TEMPLATE.md"]).is_empty());
    }

    #[test]
    fn template_does_not_satisfy_adr_requirement() {
        assert!(
            codes(&["crates/storage/src/x.rs", "docs/adr/TEMPLATE.md"]).contains(&"missing-adr")
        );
    }
}
