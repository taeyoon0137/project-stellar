//! 의존성 정책 validator (I.3.2, license policy는 I.3.3, workspace 범위는 XXIX.1.2에서 확장).
//!
//! `config/dependencies.yaml`의 승인 목록을 파싱하고, workspace 모든 member의 외부 직접
//! 의존(normal·build·dev, 모든 target)이 승인되어 있는지 검사합니다.
//!
//! 아직 어떤 member도 쓰지 않는 승인 항목은 `planned_for`로 도입 예정 WBS 행을 적어야 합니다
//! (XXIX.1.2). 계획서 진행 상태가 주어지면 `planned_for` 행이 계획서에 있는지, 그 행이 모두
//! `[x]`인데도 쓰이지 않는 오래된 승인인지 검사합니다.

use std::collections::BTreeSet;

use crate::plan::markdown_wbs::extract_wbs;
use crate::task_status::model::TaskState;
use crate::task_status::wbs_id::WbsId;
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
    pub separate_approval: bool,
    /// 아직 쓰이지 않는 승인 의존을 처음 도입할 WBS 행(XXIX.1.2). 비어 있으면 미사용 승인은 오류입니다.
    pub planned_for: Vec<String>,
}

const FIELDS: [&str; 9] = [
    "name",
    "ecosystem",
    "version",
    "license",
    "approval",
    "purpose",
    "replaceable",
    "separate_approval",
    "planned_for",
];

/// 계획서 진행 상태(`planned_for` 검사용).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanProgress {
    /// 계획서에 정의된 WBS ID.
    pub known: BTreeSet<String>,
    /// `[x]` 상태인 WBS ID.
    pub verified: BTreeSet<String>,
}

impl PlanProgress {
    /// 계획서 Markdown의 WBS 표에서 진행 상태를 읽습니다.
    pub fn from_plan(markdown: &str) -> Self {
        let mut progress = PlanProgress::default();
        for entry in extract_wbs(markdown) {
            if entry.status == TaskState::Verified {
                progress.verified.insert(entry.id.clone());
            }
            progress.known.insert(entry.id);
        }
        progress
    }
}

/// 자동 허용 라이선스(SPDX).
const AUTO_ALLOW: [&str; 5] = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC"];
/// 자동 거부 라이선스 prefix.
const AUTO_REJECT_PREFIX: [&str; 3] = ["GPL", "AGPL", "SSPL"];

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
    let separate_approval = match get("separate_approval").as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => {
            return Err(DependencyError::new(
                "invalid-separate-approval",
                format!("separate_approval는 true/false여야 합니다: {other}"),
            ))
        }
    };
    let planned_for = match get("planned_for") {
        None => Vec::new(),
        Some(raw) => parse_planned_for(&name, &raw)?,
    };
    if !planned_for.is_empty() && approval != "approved" {
        return Err(DependencyError::new(
            "planned-for-not-approved",
            format!("{name}: planned_for는 approved 항목에만 쓸 수 있습니다"),
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
        separate_approval,
        planned_for,
    })
}

/// `planned_for`(쉼표로 구분한 WBS ID 목록)를 해석합니다.
fn parse_planned_for(name: &str, raw: &str) -> Result<Vec<String>, DependencyError> {
    let mut ids = Vec::new();
    for token in raw.split(',').map(str::trim) {
        if WbsId::parse(token).is_none() {
            return Err(DependencyError::new(
                "invalid-planned-for",
                format!("{name}: planned_for 항목이 WBS ID가 아닙니다: `{token}`"),
            ));
        }
        ids.push(token.to_string());
    }
    Ok(ids)
}

/// 라이선스 정책을 평가합니다. 위반 시 오류 코드를 반환합니다.
fn evaluate_license(license: &str, separate_approval: bool) -> Option<&'static str> {
    let tokens: Vec<&str> = license
        .split(['/', '(', ')'])
        .flat_map(str::split_whitespace)
        .filter(|t| !matches!(*t, "OR" | "AND" | "WITH"))
        .collect();
    if tokens.is_empty() {
        return Some("empty-license");
    }
    if tokens
        .iter()
        .any(|t| AUTO_REJECT_PREFIX.iter().any(|p| t.starts_with(p)))
    {
        return Some("rejected-license");
    }
    if tokens.iter().all(|t| AUTO_ALLOW.contains(t)) {
        return None;
    }
    if separate_approval {
        None
    } else {
        Some("license-needs-approval")
    }
}

/// dependencies.yaml과 cargo direct dependency 이름을 비교 검증합니다(빈 Vec = 통과).
///
/// 계획서 진행 상태 없이 검사하므로 `planned_for`가 있는 미사용 승인은 통과합니다.
pub fn validate_dependencies(content: &str, direct_dep_names: &[&str]) -> Vec<DependencyError> {
    validate_dependencies_with(content, direct_dep_names, None)
}

/// [`validate_dependencies`]에 계획서 진행 상태 검사를 더합니다(XXIX.1.2).
///
/// `plan`이 있으면 `planned_for` 행이 계획서에 없을 때(`unknown-planned-for`)와, 그 행이 모두
/// `[x]`인데도 쓰이지 않을 때(`stale-planned-approval`) 실패합니다.
pub fn validate_dependencies_with(
    content: &str,
    direct_dep_names: &[&str],
    plan: Option<&PlanProgress>,
) -> Vec<DependencyError> {
    let entries = match parse_dependencies(content) {
        Ok(entries) => entries,
        Err(e) => return vec![e],
    };
    let mut errors = Vec::new();

    let approved: Vec<&DepEntry> = entries
        .iter()
        .filter(|e| e.approval == "approved")
        .collect();

    for name in direct_dep_names {
        if !approved.iter().any(|e| e.name == *name) {
            errors.push(DependencyError::new(
                "unknown-dependency",
                format!("승인되지 않은 직접 의존성: {name}"),
            ));
        }
    }
    for entry in &approved {
        let name = entry.name.as_str();
        if let Some(plan) = plan {
            for id in &entry.planned_for {
                if !plan.known.contains(id) {
                    errors.push(DependencyError::new(
                        "unknown-planned-for",
                        format!("{name}: planned_for 행이 계획서에 없습니다: {id}"),
                    ));
                }
            }
        }
        if direct_dep_names.contains(&name) {
            continue;
        }
        if entry.planned_for.is_empty() {
            errors.push(DependencyError::new(
                "unused-approval",
                format!("승인되었으나 사용되지 않는 의존성: {name}"),
            ));
        } else if let Some(plan) = plan {
            if entry
                .planned_for
                .iter()
                .all(|id| plan.verified.contains(id))
            {
                errors.push(DependencyError::new(
                    "stale-planned-approval",
                    format!(
                        "{name}: planned_for 행({})이 모두 완료됐으나 사용되지 않습니다",
                        entry.planned_for.join(", ")
                    ),
                ));
            }
        }
    }

    for entry in &entries {
        if let Some(code) = evaluate_license(&entry.license, entry.separate_approval) {
            errors.push(DependencyError::new(
                code,
                format!("{}: 라이선스 정책 위반({})", entry.name, entry.license),
            ));
        }
    }

    errors
}

/// `cargo tree --workspace -e normal,build,dev --target all --depth 1 --prefix depth --no-dedupe`
/// 출력에서 workspace member가 아닌 직접 의존 이름을 모읍니다(정렬·중복 제거, XXIX.1.2).
///
/// 깊이 0 행은 workspace member이고, 깊이 1 행 중 member 행과 같은 package(이름·버전·출처)는
/// 내부 path 의존이므로 제외합니다. 예상한 형식이 아니면 오류로 거부합니다(fail closed).
pub fn parse_workspace_direct_deps(tree: &str) -> Result<Vec<String>, DependencyError> {
    let mut members: BTreeSet<&str> = BTreeSet::new();
    let mut direct: Vec<&str> = Vec::new();
    for (index, raw) in tree.lines().enumerate() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        let (depth, rest) = line.split_at(digits);
        let package = rest.strip_suffix(" (*)").unwrap_or(rest);
        let malformed = || {
            DependencyError::new(
                "tree-format",
                format!(
                    "cargo tree 출력 {}행 형식이 예상과 다릅니다: `{line}`",
                    index + 1
                ),
            )
        };
        let name = package.split_whitespace().next().ok_or_else(malformed)?;
        let valid_name = name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !valid_name {
            return Err(malformed());
        }
        match depth {
            "0" => {
                members.insert(package);
            }
            "1" => direct.push(package),
            _ => return Err(malformed()),
        }
    }
    if members.is_empty() {
        return Err(DependencyError::new(
            "tree-format",
            "cargo tree 출력에 workspace member가 없습니다",
        ));
    }
    let mut names = BTreeSet::new();
    for package in direct {
        if members.contains(package) {
            continue;
        }
        if let Some(name) = package.split_whitespace().next() {
            names.insert(name.to_string());
        }
    }
    Ok(names.into_iter().collect())
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
    fn license_policy_allow_reject_separate_approval() {
        assert_eq!(evaluate_license("MIT", false), None);
        assert_eq!(evaluate_license("MIT OR Apache-2.0", false), None);
        assert_eq!(evaluate_license("GPL-3.0", false), Some("rejected-license"));
        assert_eq!(
            evaluate_license("AGPL-3.0-only", false),
            Some("rejected-license")
        );
        assert_eq!(
            evaluate_license("Custom-1.0", false),
            Some("license-needs-approval")
        );
        assert_eq!(evaluate_license("Custom-1.0", true), None);
    }

    #[test]
    fn rejected_license_entry_fails_validation() {
        let yaml = "dependencies:\n  - name: bad\n    ecosystem: cargo\n    version: \"1.0\"\n    license: \"GPL-3.0\"\n    approval: approved\n    purpose: \"x\"\n    replaceable: true\n";
        let codes: Vec<&str> = validate_dependencies(yaml, &["bad"])
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"rejected-license"));
    }

    #[test]
    fn real_dependencies_file_parses() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let content = std::fs::read_to_string(format!("{root}/config/dependencies.yaml")).unwrap();
        assert!(validate_dependencies(&content, &["yaml-rust2"]).is_empty());
    }

    fn planned_yaml(planned_for: &str) -> String {
        format!(
            "{}  - name: serde_json\n    ecosystem: cargo\n    version: \"1.0.154\"\n    license: \"MIT OR Apache-2.0\"\n    approval: approved\n    purpose: \"JSON\"\n    replaceable: true\n    planned_for: \"{planned_for}\"\n",
            approved_yaml()
        )
    }

    fn progress(known: &[&str], verified: &[&str]) -> PlanProgress {
        PlanProgress {
            known: known.iter().map(|s| s.to_string()).collect(),
            verified: verified.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn codes(errors: &[DependencyError]) -> Vec<&'static str> {
        errors.iter().map(|e| e.code).collect()
    }

    #[test]
    fn planned_approval_passes_until_its_rows_are_verified() {
        let yaml = planned_yaml("XIX.9.1, XXIV.4.3");
        let entries = parse_dependencies(&yaml).unwrap();
        assert_eq!(entries[1].planned_for, ["XIX.9.1", "XXIV.4.3"]);
        // 아직 쓰지 않아도 도입 예정 행이 남아 있으면 통과합니다.
        assert!(validate_dependencies(&yaml, &["yaml-rust2"]).is_empty());
        let open = progress(&["XIX.9.1", "XXIV.4.3"], &["XIX.9.1"]);
        assert!(validate_dependencies_with(&yaml, &["yaml-rust2"], Some(&open)).is_empty());
        // 도입 예정 행이 모두 [x]인데 쓰이지 않으면 오래된 승인입니다.
        let done = progress(&["XIX.9.1", "XXIV.4.3"], &["XIX.9.1", "XXIV.4.3"]);
        assert_eq!(
            codes(&validate_dependencies_with(
                &yaml,
                &["yaml-rust2"],
                Some(&done)
            )),
            ["stale-planned-approval"]
        );
        // 쓰이기 시작하면 planned_for를 남겨 두어도 통과합니다.
        assert!(
            validate_dependencies_with(&yaml, &["yaml-rust2", "serde_json"], Some(&done))
                .is_empty()
        );
        // 계획서에 없는 행은 오타로 보고 거부합니다.
        let missing = progress(&["XIX.9.1"], &[]);
        assert_eq!(
            codes(&validate_dependencies_with(
                &yaml,
                &["yaml-rust2"],
                Some(&missing)
            )),
            ["unknown-planned-for"]
        );
    }

    #[test]
    fn malformed_planned_for_fails() {
        assert_eq!(
            parse_dependencies(&planned_yaml("XIX.9")).unwrap_err().code,
            "invalid-planned-for"
        );
        assert_eq!(
            parse_dependencies(&planned_yaml("")).unwrap_err().code,
            "invalid-planned-for"
        );
        let pending = planned_yaml("XIX.9.1").replace(
            "approval: approved\n    purpose: \"JSON\"",
            "approval: pending\n    purpose: \"JSON\"",
        );
        assert_eq!(
            parse_dependencies(&pending).unwrap_err().code,
            "planned-for-not-approved"
        );
    }

    const TREE: &str = "\
0stellar-core-types v0.0.0 (/repo/crates/core-types)

0stellar-project v0.0.0 (/repo/crates/project)
1stellar-core-types v0.0.0 (/repo/crates/core-types)
1yaml-rust2 v0.11.0
1stellar-document v0.0.0 (/repo/crates/document) (*)

0stellar-document v0.0.0 (/repo/crates/document)
1stellar-core-types v0.0.0 (/repo/crates/core-types) (*)

0xtask v0.0.0 (/repo/tools/xtask)
1yaml-rust2 v0.11.0 (*)
1stellar-storage v0.0.0 (/repo/crates/storage)

0stellar-storage v0.0.0 (/repo/crates/storage)
";

    #[test]
    fn workspace_tree_yields_external_direct_dependencies_only() {
        assert_eq!(parse_workspace_direct_deps(TREE).unwrap(), ["yaml-rust2"]);
    }

    #[test]
    fn unregistered_dependency_in_any_member_fails() {
        // xtask가 아닌 제품 crate의 미등록 의존도 잡습니다(이전 범위: xtask 직접 의존만).
        let tree = TREE.replace(
            "1yaml-rust2 v0.11.0\n",
            "1yaml-rust2 v0.11.0\n1hashbrown v0.16.1\n",
        );
        let direct = parse_workspace_direct_deps(&tree).unwrap();
        assert_eq!(direct, ["hashbrown", "yaml-rust2"]);
        let refs: Vec<&str> = direct.iter().map(String::as_str).collect();
        assert_eq!(
            codes(&validate_dependencies(approved_yaml(), &refs)),
            ["unknown-dependency"]
        );
        // 같은 이름이라도 member와 출처가 다르면 외부 의존입니다.
        let shadow = TREE.replace("1yaml-rust2 v0.11.0\n", "1stellar-document v9.9.9\n");
        assert_eq!(
            parse_workspace_direct_deps(&shadow).unwrap(),
            ["stellar-document", "yaml-rust2"]
        );
    }

    #[test]
    fn unexpected_tree_format_fails_closed() {
        for tree in [
            "",
            "stellar-a v0.0.0 (/repo/a)\n",
            "0stellar-a v0.0.0 (/repo/a)\n[dev-dependencies]\n",
            "0stellar-a v0.0.0 (/repo/a)\n2deep v1.0.0\n",
            "0stellar-a v0.0.0 (/repo/a)\n1\n",
        ] {
            assert_eq!(
                parse_workspace_direct_deps(tree).unwrap_err().code,
                "tree-format",
                "{tree:?}"
            );
        }
    }

    #[test]
    fn real_dependencies_file_matches_real_plan() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let content = std::fs::read_to_string(format!("{root}/config/dependencies.yaml")).unwrap();
        let plan = std::fs::read_to_string(format!(
            "{root}/docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md"
        ))
        .unwrap();
        let progress = PlanProgress::from_plan(&plan);
        assert!(progress.known.contains("XXIX.1.2"));
        let errors = validate_dependencies_with(&content, &["yaml-rust2"], Some(&progress));
        assert!(errors.is_empty(), "{errors:?}");
    }
}
