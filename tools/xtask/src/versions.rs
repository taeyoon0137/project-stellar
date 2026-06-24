//! 버전 manifest validator (I.3.1).
//!
//! `config/versions.yaml`의 도구 항목을 검사합니다.
//! Rust는 `exact`만 허용하고 `rust-toolchain.toml`의 channel과 일치해야 합니다.
//! 그 밖의 도구는 `exact`와 `range` 중 정확히 하나만 허용합니다.

use crate::yaml::restricted_loader::{load_restricted, YamlValue};

/// 버전 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionError {
    pub code: &'static str,
    pub message: String,
}

impl VersionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        VersionError {
            code,
            message: message.into(),
        }
    }
}

/// versions.yaml 내용을 검증합니다(빈 Vec = 통과).
pub fn validate_versions(content: &str, toolchain_channel: &str) -> Vec<VersionError> {
    let value = match load_restricted(content) {
        Ok(value) => value,
        Err(e) => {
            return vec![VersionError::new(
                "parse",
                format!("YAML 파싱 실패: {}", e.message),
            )]
        }
    };
    let YamlValue::Mapping(pairs) = value else {
        return vec![VersionError::new(
            "top-level",
            "최상위는 도구 mapping이어야 합니다",
        )];
    };

    let mut errors = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut rust_seen = false;

    for (key, spec) in &pairs {
        let YamlValue::Scalar(tool) = key else {
            errors.push(VersionError::new(
                "top-level",
                "도구 이름은 scalar여야 합니다",
            ));
            continue;
        };
        if seen.contains(&tool.as_str()) {
            errors.push(VersionError::new(
                "duplicate-tool",
                format!("중복 도구: {tool}"),
            ));
            continue;
        }
        seen.push(tool);

        let YamlValue::Mapping(fields) = spec else {
            errors.push(VersionError::new(
                "type-mismatch",
                format!("{tool}: 값은 mapping이어야 합니다"),
            ));
            continue;
        };

        let mut exact: Option<String> = None;
        let mut range: Option<String> = None;
        let mut field_problem = false;
        for (fk, fv) in fields {
            let (YamlValue::Scalar(name), YamlValue::Scalar(text)) = (fk, fv) else {
                errors.push(VersionError::new(
                    "type-mismatch",
                    format!("{tool}: 필드는 scalar여야 합니다"),
                ));
                field_problem = true;
                continue;
            };
            match name.as_str() {
                "exact" => exact = Some(text.clone()),
                "range" => range = Some(text.clone()),
                other => {
                    errors.push(VersionError::new(
                        "unknown-field",
                        format!("{tool}: 알 수 없는 필드 {other}"),
                    ));
                    field_problem = true;
                }
            }
        }
        if field_problem {
            continue;
        }

        match (&exact, &range) {
            (Some(_), Some(_)) => errors.push(VersionError::new(
                "exact-and-range",
                format!("{tool}: exact와 range를 동시에 지정할 수 없습니다"),
            )),
            (None, None) => errors.push(VersionError::new(
                "neither-exact-nor-range",
                format!("{tool}: exact 또는 range가 필요합니다"),
            )),
            (Some(value), None) | (None, Some(value)) => {
                if value.trim().is_empty() {
                    errors.push(VersionError::new(
                        "empty-version",
                        format!("{tool}: 버전 값이 비어 있습니다"),
                    ));
                }
            }
        }

        if tool == "rust" {
            rust_seen = true;
            if range.is_some() {
                errors.push(VersionError::new(
                    "rust-requires-exact",
                    "rust는 exact만 허용합니다",
                ));
            }
            if let Some(value) = &exact {
                if value != toolchain_channel {
                    errors.push(VersionError::new(
                        "rust-mismatch",
                        format!(
                            "rust.exact({value}) != rust-toolchain channel({toolchain_channel})"
                        ),
                    ));
                }
            }
        }
    }

    if !rust_seen {
        errors.push(VersionError::new("missing-rust", "rust 항목이 필요합니다"));
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_versions_pass() {
        let content = "rust:\n  exact: \"1.96.0\"\n";
        assert!(validate_versions(content, "1.96.0").is_empty());
    }

    #[test]
    fn rust_mismatch_fails() {
        let content = "rust:\n  exact: \"1.95.0\"\n";
        let codes: Vec<&str> = validate_versions(content, "1.96.0")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"rust-mismatch"));
    }

    #[test]
    fn duplicate_tool_fails() {
        let content = "rust:\n  exact: \"1.96.0\"\nrust:\n  exact: \"1.96.0\"\n";
        let codes: Vec<&str> = validate_versions(content, "1.96.0")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"duplicate-tool"));
    }

    #[test]
    fn empty_version_fails() {
        let content = "rust:\n  exact: \"1.96.0\"\ncmake:\n  exact: \"\"\n";
        let codes: Vec<&str> = validate_versions(content, "1.96.0")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"empty-version"));
    }

    #[test]
    fn exact_and_range_together_fails() {
        let content =
            "rust:\n  exact: \"1.96.0\"\ncmake:\n  exact: \"3.30\"\n  range: \">=3.30\"\n";
        let codes: Vec<&str> = validate_versions(content, "1.96.0")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"exact-and-range"));
    }

    #[test]
    fn real_versions_file_passes() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let content =
            std::fs::read_to_string(format!("{root}/config/versions.yaml")).expect("versions.yaml");
        let toolchain =
            std::fs::read_to_string(format!("{root}/rust-toolchain.toml")).expect("toolchain");
        let channel = toolchain
            .lines()
            .find_map(|l| l.trim().strip_prefix("channel = "))
            .map(|v| v.trim().trim_matches('"').to_string())
            .expect("channel");
        assert!(validate_versions(&content, &channel).is_empty());
    }
}
