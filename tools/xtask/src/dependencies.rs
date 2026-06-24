//! 의존성 정책 validator (I.3.2, license policy는 I.3.3에서 확장).
//!
//! `config/dependencies.yaml`의 승인 목록을 파싱하고, cargo direct dependency가
//! 모두 승인되어 있는지 검사합니다.

use crate::yaml::restricted_loader::{load_restricted, YamlValue};

/// 의존성 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyError {
    pub code: &'static str,
    pub message: String,
}

impl DependencyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        DependencyError {
            code,
            message: message.into(),
        }
    }
}

/// 승인 목록의 한 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepEntry {
    pub name: String,
    pub ecosystem: String,
    pub version: String,
    pub license: String,
    pub approval: String,
    pub purpose: String,
    pub replaceable: bool,
}

const FIELDS: [&str; 7] = [
    "name",
    "ecosystem",
    "version",
    "license",
    "approval",
    "purpose",
    "replaceable",
];

/// dependencies.yaml을 파싱합니다.
pub fn parse_dependencies(content: &str) -> Result<Vec<DepEntry>, DependencyError> {
    let value = load_restricted(content)
        .map_err(|e| DependencyError::new("parse", format!("YAML 파싱 실패: {}", e.message)))?;
    let YamlValue::Mapping(top) = value else {
        return Err(DependencyError::new(
            "top-level",
            "최상위는 mapping이어야 합니다",
        ));
    };
    let Some((_, deps)) = top
        .iter()
        .find(|(k, _)| matches!(k, YamlValue::Scalar(s) if s == "dependencies"))
    else {
        return Err(DependencyError::new(
            "missing-dependencies",
            "`dependencies` key가 없습니다",
        ));
    };
    let YamlValue::Sequence(items) = deps else {
        return Err(DependencyError::new(
            "type-mismatch",
            "`dependencies`는 sequence여야 합니다",
        ));
    };

    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        entries.push(parse_entry(item)?);
    }
    Ok(entries)
}

fn parse_entry(value: &YamlValue) -> Result<DepEntry, DependencyError> {
    let YamlValue::Mapping(fields) = value else {
        return Err(DependencyError::new(
            "type-mismatch",
            "의존성 항목은 mapping이어야 합니다",
        ));
    };
    let get = |name: &str| -> Option<String> {
        fields.iter().find_map(|(k, v)| match (k, v) {
            (YamlValue::Scalar(key), YamlValue::Scalar(val)) if key == name => Some(val.clone()),
            _ => None,
        })
    };
    let mut missing = Vec::new();
    let mut value_of = |name: &'static str| -> String {
        match get(name) {
            Some(v) => v,
            None => {
                missing.push(name);
                String::new()
            }
        }
    };
    let name = value_of("name");
    let ecosystem = value_of("ecosystem");
    let version = value_of("version");
    let license = value_of("license");
    let approval = value_of("approval");
    let purpose = value_of("purpose");
    let replaceable_raw = value_of("replaceable");

    if !missing.is_empty() {
        return Err(DependencyError::new(
            "missing-field",
            format!("필수 필드 누락: {}", missing.join(", ")),
        ));
    }
    for (key, _) in fields {
        if let YamlValue::Scalar(k) = key {
            if !FIELDS.contains(&k.as_str()) {
                return Err(DependencyError::new(
                    "unknown-field",
                    format!("알 수 없는 필드: {k}"),
                ));
            }
        }
    }
    let replaceable = match replaceable_raw.as_str() {
        "true" => true,
        "false" => false,
        other => {
            return Err(DependencyError::new(
                "invalid-replaceable",
                format!("replaceable는 true/false여야 합니다: {other}"),
            ))
        }
    };
    if !matches!(approval.as_str(), "approved" | "pending" | "rejected") {
        return Err(DependencyError::new(
            "invalid-approval",
            format!("approval 값이 잘못됨: {approval}"),
        ));
    }

    Ok(DepEntry {
        name,
        ecosystem,
        version,
        license,
        approval,
        purpose,
        replaceable,
    })
}

/// dependencies.yaml과 cargo direct dependency 이름을 비교 검증합니다(빈 Vec = 통과).
pub fn validate_dependencies(content: &str, direct_dep_names: &[&str]) -> Vec<DependencyError> {
    let entries = match parse_dependencies(content) {
        Ok(entries) => entries,
        Err(e) => return vec![e],
    };
    let mut errors = Vec::new();

    let approved: Vec<&str> = entries
        .iter()
        .filter(|e| e.approval == "approved")
        .map(|e| e.name.as_str())
        .collect();

    for name in direct_dep_names {
        if !approved.contains(name) {
            errors.push(DependencyError::new(
                "unknown-dependency",
                format!("승인되지 않은 직접 의존성: {name}"),
            ));
        }
    }
    for name in &approved {
        if !direct_dep_names.contains(name) {
            errors.push(DependencyError::new(
                "unused-approval",
                format!("승인되었으나 사용되지 않는 의존성: {name}"),
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved_yaml() -> &'static str {
        "dependencies:\n  - name: yaml-rust2\n    ecosystem: cargo\n    version: \"0.11.0\"\n    license: \"MIT OR Apache-2.0\"\n    approval: approved\n    purpose: \"YAML\"\n    replaceable: true\n"
    }

    #[test]
    fn approved_set_matches_direct_set() {
        assert!(validate_dependencies(approved_yaml(), &["yaml-rust2"]).is_empty());
    }

    #[test]
    fn unknown_direct_dependency_fails() {
        let codes: Vec<&str> = validate_dependencies(approved_yaml(), &["yaml-rust2", "serde"])
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"unknown-dependency"));
    }

    #[test]
    fn empty_approval_list_with_no_direct_passes() {
        assert!(validate_dependencies("dependencies:\n  - name: yaml-rust2\n    ecosystem: cargo\n    version: \"0.11.0\"\n    license: \"MIT OR Apache-2.0\"\n    approval: pending\n    purpose: \"x\"\n    replaceable: true\n", &[]).is_empty());
    }

    #[test]
    fn missing_field_fails() {
        let yaml = "dependencies:\n  - name: x\n    ecosystem: cargo\n";
        assert_eq!(parse_dependencies(yaml).unwrap_err().code, "missing-field");
    }

    #[test]
    fn real_dependencies_file_parses() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let content = std::fs::read_to_string(format!("{root}/config/dependencies.yaml")).unwrap();
        assert!(validate_dependencies(&content, &["yaml-rust2"]).is_empty());
    }
}
