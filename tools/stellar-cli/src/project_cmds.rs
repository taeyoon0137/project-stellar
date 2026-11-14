//! `validate`, `semantic-diff`, `semantic-merge`, `migrate` (XXIV.1.3).
//!
//! 모두 `stellar-ops`를 호출합니다. 결과가 부정적인 명령(검증 오류, 미해결 semantic conflict,
//! migration 필요)은 종료 코드 1(도메인 실패)로 끝나며, 그때도 판정 근거를 잃지 않도록 결과
//! object를 오류 envelope의 `result`에 함께 넣습니다(XXIV.1.2 envelope: `result`는 object 또는 null).
//!
//! | 명령 | 읽기·쓰기 | 성공 | 도메인 실패(1) |
//! |---|---|---|---|
//! | `validate <경로>` | 읽기 | 오류 0(경고 가능) | `validation-failed` |
//! | `semantic-diff <base> <target>` | 읽기 | 항상(변경 목록) | — |
//! | `semantic-merge <base> <ours> <theirs>` | 읽기 | conflict 0 | `unresolved-conflicts` |
//! | `migrate <경로> --check` | 읽기 | 최신·저장 전 | `migration-required` |
//! | `migrate <경로>` | 쓰기(저장 잠금) | 최신이거나 migration 완료 | — |
//!
//! | `ci <경로>` | 읽기 | lockfile·검증·migration 모두 통과 | `ci-failed` (XXIV.3.1) |
//!
//! 스키마가 이 도구보다 새 버전이면 `migrate`는 `unsupported-schema`(65)입니다.

use std::path::Path;

use stellar_ops::ci::{run_ci, CheckStatus, CiReport};
use stellar_ops::compare::{
    semantic_diff_projects, semantic_merge_projects, CompareError, ConflictEntry, SideView,
};
use stellar_ops::lock::{default_lock_dir, LockError, LockPolicy};
use stellar_ops::migrate::{
    migrate, product_registry, schema_status, MigrateError, SchemaState, SchemaStatus,
    CURRENT_SCHEMA_VERSION,
};
use stellar_ops::validate::{validate_project, FindingSeverity, ValidationReport};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;

fn paths<'a>(args: &'a [String], names: &[&str], command: &str) -> Result<Vec<&'a Path>, CliError> {
    if let Some(option) = args
        .iter()
        .find(|a| a.starts_with('-') && a.as_str() != "-")
    {
        return Err(CliError::usage(format!("알 수 없는 옵션: {option}")));
    }
    if args.len() != names.len() {
        return Err(CliError::usage(format!(
            "{command}에는 {}가 필요합니다",
            names.join(" ")
        )));
    }
    Ok(args.iter().map(|a| Path::new(a.as_str())).collect())
}

fn validation_json(report: &ValidationReport) -> JsonValue {
    JsonValue::object(vec![
        ("revision", views::revision(report.revision)),
        ("valid", JsonValue::Bool(report.is_valid())),
        ("error_count", views::count(report.error_count())),
        ("warning_count", views::count(report.warning_count())),
        (
            "documents",
            JsonValue::Array(
                report
                    .documents
                    .iter()
                    .map(|doc| {
                        JsonValue::object(vec![
                            ("id", views::id(doc.document)),
                            (
                                "node_count",
                                doc.node_count.map_or(JsonValue::Null, views::count),
                            ),
                            (
                                "findings",
                                JsonValue::Array(
                                    doc.findings
                                        .iter()
                                        .map(|f| {
                                            JsonValue::object(vec![
                                                (
                                                    "severity",
                                                    JsonValue::string(match f.severity {
                                                        FindingSeverity::Error => "error",
                                                        FindingSeverity::Warning => "warning",
                                                    }),
                                                ),
                                                ("code", JsonValue::string(f.code)),
                                                ("message", JsonValue::string(f.message.clone())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn finding_diagnostics(report: &ValidationReport) -> Vec<Diagnostic> {
    report
        .documents
        .iter()
        .flat_map(|doc| doc.findings.iter())
        .map(|f| Diagnostic {
            severity: match f.severity {
                FindingSeverity::Error => Severity::Error,
                FindingSeverity::Warning => Severity::Warning,
            },
            code: f.code,
            message: format!("문서 {}: {}", f.document, f.message),
        })
        .collect()
}

/// `validate <경로>`.
pub fn validate(args: &[String]) -> Result<Success, CliError> {
    let path = paths(args, &["<프로젝트 경로>"], "validate")?[0];
    let context = path.display().to_string();
    let report =
        validate_project(path).map_err(|e| views::open_error(&e).with_context(context.clone()))?;
    let result = validation_json(&report);
    let diagnostics = finding_diagnostics(&report);
    if report.is_valid() {
        return Ok(Success {
            result,
            diagnostics,
        });
    }
    let mut error = CliError::new(
        ExitStatus::Failure,
        "validation-failed",
        format!("검증 오류 {}건", report.error_count()),
    )
    .with_result(result)
    .with_context(context);
    error.diagnostics = diagnostics;
    Err(error)
}

/// 비교·병합 실패 분류(`diff-summary`와 공유).
pub fn compare_error_public(error: &CompareError) -> CliError {
    compare_error(error)
}

fn compare_error(error: &CompareError) -> CliError {
    match error {
        CompareError::Open { error: e, .. } => {
            let mut cli = views::open_error(e);
            cli.message = format!("{}", error);
            cli
        }
        CompareError::Document { error: e, .. } => views::document_error(e),
        CompareError::ProjectMismatch { .. } => {
            CliError::new(ExitStatus::DataErr, "project-mismatch", error.to_string())
        }
        CompareError::Projection(_) => {
            CliError::new(ExitStatus::Software, "projection", error.to_string())
        }
        CompareError::Merge(_) => CliError::new(ExitStatus::Software, "merge", error.to_string()),
    }
}

/// `semantic-diff <base 경로> <target 경로>`.
pub fn semantic_diff(args: &[String]) -> Result<Success, CliError> {
    let p = paths(args, &["<base 경로>", "<target 경로>"], "semantic-diff")?;
    let context = format!("{} -> {}", p[0].display(), p[1].display());
    let diff =
        semantic_diff_projects(p[0], p[1]).map_err(|e| compare_error(&e).with_context(context))?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("base_revision", views::revision(diff.base_revision)),
            ("target_revision", views::revision(diff.target_revision)),
            ("changes", views::diff_report(&diff.report)),
        ]),
        diagnostics: Vec::new(),
    })
}

fn side_json(side: &SideView) -> JsonValue {
    match side {
        SideView::Missing => JsonValue::object(vec![("state", JsonValue::string("missing"))]),
        SideView::Object(kind) => JsonValue::object(vec![
            ("state", JsonValue::string("object")),
            ("object_kind", JsonValue::string(kind.clone())),
        ]),
        SideView::Position(position) => JsonValue::object(vec![
            ("state", JsonValue::string("position")),
            ("parent", views::opt_id(position.parent)),
            (
                "order_key",
                JsonValue::opt_string(position.order_key_hex.as_deref()),
            ),
        ]),
        SideView::Value(value) => JsonValue::object(vec![
            ("state", JsonValue::string("value")),
            (
                "value",
                value.as_ref().map_or(JsonValue::Null, |v| {
                    JsonValue::object(vec![
                        ("type", JsonValue::string(v.type_name())),
                        ("value", views::value_inner(v)),
                    ])
                }),
            ),
        ]),
    }
}

/// semantic conflict JSON.
pub fn conflict_json(conflict: &ConflictEntry) -> JsonValue {
    JsonValue::object(vec![
        ("kind", JsonValue::string(conflict.kind)),
        ("document", views::opt_id(conflict.document)),
        ("object", views::id(conflict.object)),
        ("target", JsonValue::string(conflict.target)),
        ("path", JsonValue::opt_string(conflict.path.as_deref())),
        ("base", side_json(&conflict.base)),
        ("ours", side_json(&conflict.ours)),
        ("theirs", side_json(&conflict.theirs)),
    ])
}

/// `semantic-merge <base> <ours> <theirs>`.
pub fn semantic_merge(args: &[String]) -> Result<Success, CliError> {
    let p = paths(
        args,
        &["<base 경로>", "<ours 경로>", "<theirs 경로>"],
        "semantic-merge",
    )?;
    let context = format!(
        "{} / {} / {}",
        p[0].display(),
        p[1].display(),
        p[2].display()
    );
    let merged = semantic_merge_projects(p[0], p[1], p[2])
        .map_err(|e| compare_error(&e).with_context(context.clone()))?;
    let result = JsonValue::object(vec![
        ("base_revision", views::revision(merged.base_revision)),
        ("ours_revision", views::revision(merged.ours_revision)),
        ("theirs_revision", views::revision(merged.theirs_revision)),
        ("clean", JsonValue::Bool(merged.is_clean())),
        ("changes", views::diff_report(&merged.changes)),
        (
            "conflicts",
            JsonValue::Array(merged.conflicts.iter().map(conflict_json).collect()),
        ),
    ]);
    if merged.is_clean() {
        return Ok(Success {
            result,
            diagnostics: Vec::new(),
        });
    }
    let mut error = CliError::new(
        ExitStatus::Failure,
        "unresolved-conflicts",
        format!(
            "자동 병합할 수 없는 semantic conflict {}건이 있습니다. 앱에서 해결해야 합니다",
            merged.conflicts.len()
        ),
    )
    .with_result(result)
    .with_context(context);
    error.diagnostics = merged
        .conflicts
        .iter()
        .map(|c| Diagnostic {
            severity: Severity::Error,
            code: "semantic-conflict",
            message: format!(
                "{} {} {}{}",
                c.kind,
                c.object,
                c.target,
                c.path.as_ref().map(|p| format!(" {p}")).unwrap_or_default()
            ),
        })
        .collect();
    Err(error)
}

fn status_json(status: &SchemaStatus) -> JsonValue {
    JsonValue::object(vec![
        ("revision", views::revision(status.revision)),
        (
            "schema_version",
            status
                .schema_version
                .map_or(JsonValue::Null, |v| views::uint(u64::from(v))),
        ),
        (
            "target_schema",
            views::uint(u64::from(status.target_schema)),
        ),
        ("state", JsonValue::string(status.state.name())),
    ])
}

fn unsupported(found: Option<u32>, target: u32) -> CliError {
    CliError::new(
        ExitStatus::DataErr,
        "unsupported-schema",
        format!(
            "generation 스키마 {}가 이 도구의 스키마 {target}보다 새 버전입니다",
            found.map_or("?".to_string(), |v| v.to_string())
        ),
    )
}

/// `migrate <경로> [--check]`.
pub fn migrate_command(args: &[String]) -> Result<Success, CliError> {
    let mut check = false;
    let mut rest = Vec::new();
    for arg in args {
        if arg == "--check" && !check {
            check = true;
        } else {
            rest.push(arg.clone());
        }
    }
    let path = paths(&rest, &["<프로젝트 경로>"], "migrate")?[0];
    let context = path.display().to_string();
    if check {
        let status = schema_status(path, CURRENT_SCHEMA_VERSION)
            .map_err(|e| views::open_error(&e).with_context(context.clone()))?;
        let result = status_json(&status);
        return match status.state {
            SchemaState::Unsaved | SchemaState::UpToDate => Ok(Success {
                result,
                diagnostics: Vec::new(),
            }),
            SchemaState::MigrationRequired => Err(CliError::new(
                ExitStatus::Failure,
                "migration-required",
                "migration이 필요합니다",
            )
            .with_result(result)
            .with_context(context)),
            SchemaState::UnsupportedNewer => {
                Err(unsupported(status.schema_version, status.target_schema)
                    .with_result(result)
                    .with_context(context))
            }
        };
    }
    let dir = default_lock_dir().map_err(|e| lock_error(&e))?;
    let outcome = migrate(
        path,
        &product_registry(),
        CURRENT_SCHEMA_VERSION,
        &LockPolicy::Required { dir },
    )
    .map_err(|e| {
        match &e {
            MigrateError::Open(open) => views::open_error(open),
            MigrateError::Lock(lock) => lock_error(lock),
            MigrateError::UnsupportedNewer { found, target } => unsupported(Some(*found), *target),
            MigrateError::Run(_) => {
                CliError::new(ExitStatus::CantCreate, "migration", e.to_string())
            }
        }
        .with_context(context.clone())
    })?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("before", status_json(&outcome.before)),
            ("revision", views::revision(outcome.revision)),
            ("migrated", JsonValue::Bool(outcome.migrated)),
        ]),
        diagnostics: Vec::new(),
    })
}

/// 저장 잠금 실패 분류(XXIV.4.5와 같은 코드).
pub fn lock_error(error: &LockError) -> CliError {
    let message = error.to_string();
    match error {
        LockError::Busy { .. } => CliError::new(ExitStatus::TempFail, "save-lock-busy", message),
        LockError::NoUserDataDir => CliError::new(ExitStatus::Config, "no-user-data-dir", message),
        LockError::Io { kind, .. } => {
            CliError::new(ExitStatus::from_io_kind(*kind), "save-lock-io", message)
        }
    }
}

fn ci_json(report: &CiReport) -> JsonValue {
    JsonValue::object(vec![
        ("revision", views::revision(report.revision)),
        ("passed", JsonValue::Bool(report.passed())),
        (
            "checks",
            JsonValue::Array(
                report
                    .checks
                    .iter()
                    .map(|check| {
                        JsonValue::object(vec![
                            ("name", JsonValue::string(check.name)),
                            ("status", JsonValue::string(check.status.name())),
                            (
                                "findings",
                                JsonValue::Array(
                                    check
                                        .findings
                                        .iter()
                                        .map(|f| {
                                            JsonValue::object(vec![
                                                ("code", JsonValue::string(f.code)),
                                                ("message", JsonValue::string(f.message.clone())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// `ci <경로>`: lockfile·검증·migration 검사를 모두 실행합니다(XXIV.3.1). 하나라도 실패하면 1.
pub fn ci(args: &[String]) -> Result<Success, CliError> {
    let path = paths(args, &["<프로젝트 경로>"], "ci")?[0];
    let context = path.display().to_string();
    let report = run_ci(path).map_err(|e| views::open_error(&e).with_context(context.clone()))?;
    let result = ci_json(&report);
    if report.passed() {
        return Ok(Success {
            result,
            diagnostics: Vec::new(),
        });
    }
    let failed: Vec<&str> = report
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .map(|c| c.name)
        .collect();
    let mut error = CliError::new(
        ExitStatus::Failure,
        "ci-failed",
        format!("CI 검사 실패: {}", failed.join(", ")),
    )
    .with_result(result)
    .with_context(context);
    error.diagnostics = report
        .checks
        .iter()
        .flat_map(|c| c.findings.iter().map(move |f| (c.name, f)))
        .map(|(check, f)| Diagnostic {
            severity: Severity::Error,
            code: f.code,
            message: format!("{check}: {}", f.message),
        })
        .collect();
    Err(error)
}
