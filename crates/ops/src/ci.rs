//! CI 검사 (XXIV.3.1).
//!
//! [`run_ci`]는 프로젝트 하나에 세 검사를 실행하고 모두 통과해야 성공입니다. 네트워크·Git·cache를
//! 쓰지 않는 읽기 전용 검사이며 파일을 쓰지 않습니다.
//!
//! | 검사 | 실패 조건 |
//! |---|---|
//! | `lockfile` | `stellar.lock.yaml`이 없거나(의존성이 있을 때) 형식이 틀림, 매니페스트 의존성의 최상위 lock entry 없음·내용 불일치(library id·source·repository·version 선택자), 매니페스트에 없는 최상위 entry, 부모 경로가 없는 전이 entry |
//! | `validation` | 문서 validator 오류(XXIV.1.3 `validate`와 같음) |
//! | `migration` | generation 스키마가 이 도구의 스키마와 다름(낮으면 migration 필요, 높으면 지원하지 않는 새 버전) |
//!
//! lock 검사는 resolver를 다시 실행하지 않습니다. CI에서 fetch 없이 확인할 수 있는 "매니페스트와
//! 잠금 파일이 맞물리는가"만 봅니다. 진단 메시지에는 repository URL을 넣지 않고 logical name만 씁니다
//! (XV.1.4 민감값).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use stellar_project::lock::Lockfile;
use stellar_project::lockfile_write::LOCK_FILE;
use stellar_project::yaml::load_restricted;
use stellar_project::yaml::loader::MAX_INPUT_BYTES;

use crate::error::OpenError;
use crate::migrate::{SchemaState, CURRENT_SCHEMA_VERSION};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};
use crate::validate::{validate_snapshot, FindingSeverity};

/// 검사 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// 통과.
    Pass,
    /// 실패.
    Fail,
}

impl CheckStatus {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
        }
    }
}

/// 검사 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFinding {
    /// 고정 진단 코드.
    pub code: &'static str,
    /// 사람이 읽는 설명(경로·URL 미포함).
    pub message: String,
}

/// 검사 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// 검사 이름(`lockfile`, `validation`, `migration`).
    pub name: &'static str,
    /// 결과.
    pub status: CheckStatus,
    /// 실패 원인(통과면 비어 있거나 정보성 내용).
    pub findings: Vec<CheckFinding>,
}

/// CI 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiReport {
    /// 검사한 revision.
    pub revision: ProjectRevision,
    /// 검사(고정 순서).
    pub checks: Vec<CheckResult>,
}

impl CiReport {
    /// 모든 검사가 통과했는지.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.status == CheckStatus::Pass)
    }
}

fn finding(code: &'static str, message: impl Into<String>) -> CheckFinding {
    CheckFinding {
        code,
        message: message.into(),
    }
}

fn result(name: &'static str, findings: Vec<CheckFinding>) -> CheckResult {
    CheckResult {
        name,
        status: if findings.is_empty() {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        findings,
    }
}

/// 잠금 파일을 loader 상한까지만 읽습니다.
fn read_lock(root: &Path) -> Result<Option<String>, CheckFinding> {
    let path = root.join(LOCK_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(finding(
                "lockfile-read",
                format!("{LOCK_FILE}을 읽을 수 없습니다: {:?}", e.kind()),
            ))
        }
    };
    let mut text = String::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| {
            finding(
                "lockfile-read",
                format!("{LOCK_FILE}을 읽을 수 없습니다: {:?}", e.kind()),
            )
        })?;
    if text.len() > MAX_INPUT_BYTES {
        return Err(finding(
            "lockfile-invalid",
            format!("{LOCK_FILE}이 {MAX_INPUT_BYTES} bytes를 넘습니다"),
        ));
    }
    Ok(Some(text))
}

/// 매니페스트와 잠금 파일이 맞물리는지 검사합니다.
pub fn check_lockfile(snapshot: &ProjectSnapshot) -> CheckResult {
    let manifest = snapshot.manifest();
    let text = match read_lock(snapshot.root()) {
        Ok(text) => text,
        Err(f) => return result("lockfile", vec![f]),
    };
    let Some(text) = text else {
        let findings = if manifest.dependencies.is_empty() {
            Vec::new()
        } else {
            vec![finding(
                "lockfile-missing",
                format!(
                    "의존성 {}개가 선언됐지만 {LOCK_FILE}이 없습니다",
                    manifest.dependencies.len()
                ),
            )]
        };
        return result("lockfile", findings);
    };
    let lock = match load_restricted(&text)
        .map_err(|e| e.to_string())
        .and_then(|value| Lockfile::from_yaml(&value).map_err(|e| e.to_string()))
    {
        Ok(lock) => lock,
        Err(e) => {
            return result(
                "lockfile",
                vec![finding(
                    "lockfile-invalid",
                    format!("{LOCK_FILE} 형식 오류: {e}"),
                )],
            )
        }
    };
    let mut findings = Vec::new();
    for dependency in &manifest.dependencies {
        let name = dependency.name.as_str();
        let top = lock.resolved.iter().find(|entry| {
            let names = entry.dependency_path.names();
            names.len() == 1 && names[0].as_str() == name
        });
        match top {
            None => findings.push(finding(
                "lockfile-missing-entry",
                format!("의존성 {name:?}의 lock entry가 없습니다(resolve 필요)"),
            )),
            Some(entry) => {
                let mut stale = Vec::new();
                if entry.library_id != dependency.library_id {
                    stale.push("library_id");
                }
                if entry.source != dependency.source {
                    stale.push("source");
                }
                if entry.repository != dependency.repository {
                    stale.push("repository");
                }
                if entry.requested != dependency.version {
                    stale.push("version");
                }
                if !stale.is_empty() {
                    findings.push(finding(
                        "lockfile-stale-entry",
                        format!(
                            "의존성 {name:?}의 lock entry가 매니페스트와 다릅니다({})",
                            stale.join(", ")
                        ),
                    ));
                }
            }
        }
    }
    let declared: BTreeSet<&str> = manifest
        .dependencies
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    let paths: BTreeSet<Vec<&str>> = lock
        .resolved
        .iter()
        .map(|e| {
            e.dependency_path
                .names()
                .iter()
                .map(|n| n.as_str())
                .collect()
        })
        .collect();
    for path in &paths {
        match path.as_slice() {
            [] => {}
            [top] => {
                if !declared.contains(top) {
                    findings.push(finding(
                        "lockfile-extra-entry",
                        format!("매니페스트에 없는 의존성 {top:?}의 lock entry가 있습니다"),
                    ));
                }
            }
            [parent @ .., _] => {
                if !paths.contains(parent) {
                    findings.push(finding(
                        "lockfile-orphan-entry",
                        format!("전이 lock entry {}의 부모 경로가 없습니다", path.join("/")),
                    ));
                }
            }
        }
    }
    result("lockfile", findings)
}

/// CI 검사를 실행합니다. 프로젝트를 열 수 없으면 검사 결과가 아니라 오류입니다.
pub fn run_ci(path: &Path) -> Result<CiReport, OpenError> {
    let snapshot = open_snapshot(path)?;
    let lockfile = check_lockfile(&snapshot);
    let validation = validate_snapshot(&snapshot);
    let validation = result(
        "validation",
        validation
            .documents
            .iter()
            .flat_map(|d| d.findings.iter())
            .filter(|f| f.severity == FindingSeverity::Error)
            .map(|f| finding(f.code, format!("문서 {}: {}", f.document, f.message)))
            .collect(),
    );
    let schema = snapshot.generation().map(|g| g.manifest().schema_version);
    let state = match schema {
        None => SchemaState::Unsaved,
        Some(v) if v == CURRENT_SCHEMA_VERSION => SchemaState::UpToDate,
        Some(v) if v < CURRENT_SCHEMA_VERSION => SchemaState::MigrationRequired,
        Some(_) => SchemaState::UnsupportedNewer,
    };
    let migration = result(
        "migration",
        match state {
            SchemaState::Unsaved | SchemaState::UpToDate => Vec::new(),
            SchemaState::MigrationRequired => vec![finding(
                "migration-required",
                format!(
                    "generation 스키마 {}를 {CURRENT_SCHEMA_VERSION}로 migration해야 합니다(`stellar migrate`)",
                    schema.unwrap_or_default()
                ),
            )],
            SchemaState::UnsupportedNewer => vec![finding(
                "unsupported-schema",
                format!(
                    "generation 스키마 {}가 이 도구의 스키마 {CURRENT_SCHEMA_VERSION}보다 새 버전입니다",
                    schema.unwrap_or_default()
                ),
            )],
        },
    );
    Ok(CiReport {
        revision: snapshot.revision(),
        checks: vec![lockfile, validation, migration],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        corrupt_view_tree_chunks, file_digest, random_valid_document, save_documents,
        save_documents_with_schema, scaffold_project, TempDir,
    };

    fn statuses(report: &CiReport) -> Vec<(&'static str, CheckStatus)> {
        report.checks.iter().map(|c| (c.name, c.status)).collect()
    }

    #[test]
    fn valid_project_passes_every_check_without_writing() {
        let dir = TempDir::new("ci");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&random_valid_document(500)]);
        let before = file_digest(&dir.path);
        let report = run_ci(&dir.path).unwrap();
        assert!(report.passed(), "{report:?}");
        assert_eq!(
            statuses(&report),
            [
                ("lockfile", CheckStatus::Pass),
                ("validation", CheckStatus::Pass),
                ("migration", CheckStatus::Pass)
            ]
        );
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn invalid_projects_fail_the_matching_check() {
        // 손상 문서 → validation 실패.
        let dir = TempDir::new("ci-invalid");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&random_valid_document(7)]);
        assert!(corrupt_view_tree_chunks(&dir.path) > 0);
        let report = run_ci(&dir.path).unwrap();
        assert!(!report.passed());
        assert_eq!(report.checks[1].status, CheckStatus::Fail);
        assert_eq!(report.checks[0].status, CheckStatus::Pass);

        // 새 스키마 → migration 실패.
        let newer = TempDir::new("ci-newer");
        scaffold_project(&newer.path);
        save_documents_with_schema(&newer.path, &[&random_valid_document(7)], 2);
        let report = run_ci(&newer.path).unwrap();
        assert_eq!(report.checks[2].status, CheckStatus::Fail);
        assert_eq!(report.checks[2].findings[0].code, "unsupported-schema");

        // 의존성 선언 + 빈 lock → lockfile 실패(entry 없음), lock 삭제 → 없음.
        let deps = TempDir::new("ci-lock");
        scaffold_project(&deps.path);
        let manifest = "schema_version: 1\nproject:\n  id: 0195c530-3acd-7bc1-8414-c213f61cb965\n  name: Deps\ntarget_profiles:\n  - id: apple-desktop\n    platform: apple\n    minimum_os: \"26.0\"\ndependencies:\n  design-system:\n    source: git\n    library_id: 0195c530-3acd-7bc1-8414-c213f61cb966\n    repository: https://example.com/design-system.git\n    version: v2.4.0\n";
        std::fs::write(deps.path.join("stellar.yaml"), manifest).unwrap();
        let report = run_ci(&deps.path).unwrap();
        assert_eq!(report.checks[0].status, CheckStatus::Fail);
        assert_eq!(report.checks[0].findings[0].code, "lockfile-missing-entry");
        assert!(!report.checks[0].findings[0].message.contains("example.com"));
        std::fs::remove_file(deps.path.join(LOCK_FILE)).unwrap();
        let report = run_ci(&deps.path).unwrap();
        assert_eq!(report.checks[0].findings[0].code, "lockfile-missing");
        std::fs::write(deps.path.join(LOCK_FILE), "schema_version: 9\n").unwrap();
        let report = run_ci(&deps.path).unwrap();
        assert_eq!(report.checks[0].findings[0].code, "lockfile-invalid");

        // 매니페스트와 맞는 lock은 통과, 선택자가 다르면 stale, 매니페스트에 없는 entry는 extra,
        // 부모 경로가 없는 전이 entry는 orphan입니다.
        let entry = |name: &str, requested: &str, path: &[&str]| {
            format!(
                "  -\n    logical_id: {name}\n    library_id: \"0195c530-3acd-7bc1-8414-c213f61cb966\"\n    source: git\n    repository: \"https://example.com/design-system.git\"\n    requested: {requested}\n    exact_commit: a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1\n    content_hash: \"sha256:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1\"\n    schema_version: \"1\"\n    dependency_path:\n{}    dependencies: []\n",
                path.iter().map(|p| format!("      - {p}\n")).collect::<String>()
            )
        };
        let lock =
            |entries: &[String]| format!("schema_version: 1\nresolved:\n{}", entries.concat());
        std::fs::write(
            deps.path.join(LOCK_FILE),
            lock(&[entry("design-system", "v2.4.0", &["design-system"])]),
        )
        .unwrap();
        let report = run_ci(&deps.path).unwrap();
        assert_eq!(report.checks[0].status, CheckStatus::Pass, "{report:?}");
        std::fs::write(
            deps.path.join(LOCK_FILE),
            lock(&[entry("design-system", "v2.3.0", &["design-system"])]),
        )
        .unwrap();
        let report = run_ci(&deps.path).unwrap();
        assert_eq!(report.checks[0].findings[0].code, "lockfile-stale-entry");
        assert!(report.checks[0].findings[0].message.contains("version"));
        std::fs::write(
            deps.path.join(LOCK_FILE),
            lock(&[
                entry("design-system", "v2.4.0", &["design-system"]),
                entry("icons", "v1", &["ghost", "icons"]),
            ]),
        )
        .unwrap();
        let report = run_ci(&deps.path).unwrap();
        let codes: Vec<&str> = report.checks[0].findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, ["lockfile-orphan-entry"], "{report:?}");
    }
}
