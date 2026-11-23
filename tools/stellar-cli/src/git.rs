//! `stellar git status|stage|unstage|commit` (ADR-0080).
//!
//! 앱과 같은 Git 엔진(`stellar_ops::git`)을 씁니다. 신뢰 등록이 있는 프로젝트에서만 시스템 Git을 실행합니다.
//! push·force push 명령은 없습니다(push는 시스템 Git에 맡깁니다). 필수 동반 변경은 자동으로 포함하지 않고
//! 실패 결과의 `required`에 돌려줍니다. 호출자가 고른 항목을 `--with`로 다시 넘깁니다.

use std::path::Path;

use stellar_ops::candidate::SelectionOperation;
use stellar_ops::git::{
    change_id_text, ChangeEntry, CommitOutcome, GitError, GitItem, GitSession, GitStatusReport,
    HeadState, IdentityKey, SelectionOutcome, SourceUnit, StagedFinding, StatusArea, StatusRequest,
    UpstreamState, WorktreeKind, DEFAULT_STATUS_LIMIT, MAX_STATUS_LIMIT,
};
use stellar_ops::git::{SelectionInput, SemanticChangeKind};
use stellar_ops::query::property_path_label;

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;

/// 하위 명령 사용법.
pub const USAGE: &str = "status <경로> [--cursor <c>] [--limit <n>] | stage <경로> <대상>... [--with <대상>]... | unstage <경로> <대상>... [--with <대상>]... | commit <경로> -m <메시지>";

/// `git <하위 명령> ...`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(CliError::usage(format!(
            "git에는 하위 명령이 필요합니다: {USAGE}"
        )));
    };
    match sub.as_str() {
        "status" => status(rest),
        "stage" => select(rest, SelectionOperation::Stage),
        "unstage" => select(rest, SelectionOperation::Unstage),
        "commit" => commit(rest),
        other => Err(CliError::usage(format!(
            "알 수 없는 git 하위 명령: {other} ({USAGE})"
        ))),
    }
}

fn path_arg<'a>(rest: &'a [String], name: &str) -> Result<(&'a String, &'a [String]), CliError> {
    match rest.split_first() {
        Some((path, tail)) if !path.starts_with('-') => Ok((path, tail)),
        _ => Err(CliError::usage(format!(
            "git {name}에는 <경로>가 필요합니다"
        ))),
    }
}

fn open(path: &str) -> Result<GitSession, CliError> {
    GitSession::open_for_os_user(Path::new(path)).map_err(|e| git_error(&e).with_context(path))
}

fn status(rest: &[String]) -> Result<Success, CliError> {
    let (path, mut tail) = path_arg(rest, "status")?;
    let mut request = StatusRequest::default();
    let (mut cursor_seen, mut limit_seen) = (false, false);
    while let Some((flag, more)) = tail.split_first() {
        let Some((value, more)) = more.split_first() else {
            return Err(CliError::usage(format!("{flag}에는 값이 필요합니다")));
        };
        match flag.as_str() {
            "--cursor" if !cursor_seen => {
                cursor_seen = true;
                request.cursor = Some(value.clone());
            }
            "--limit" if !limit_seen => {
                limit_seen = true;
                request.limit = value
                    .parse()
                    .ok()
                    .filter(|n| (1..=MAX_STATUS_LIMIT).contains(n))
                    .ok_or_else(|| {
                        CliError::usage(format!(
                            "--limit은 1..={MAX_STATUS_LIMIT}이어야 합니다(기본 {DEFAULT_STATUS_LIMIT})"
                        ))
                    })?;
            }
            _ => return Err(CliError::usage(format!("알 수 없거나 중복된 옵션: {flag}"))),
        }
        tail = more;
    }
    let session = open(path)?;
    let report = session
        .status(&request)
        .map_err(|e| git_error(&e).with_context(path.as_str()))?;
    let mut diagnostics = Vec::new();
    if let Err(reason) = &report.semantic {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            code: "semantic-status-unavailable",
            message: reason.clone(),
        });
    }
    Ok(Success {
        result: status_json(&report),
        diagnostics,
    })
}

fn parse_item(text: &str) -> Result<GitItem, CliError> {
    text.parse()
        .map_err(|e: stellar_ops::git::ItemParseError| CliError::usage(e.to_string()))
}

fn select(rest: &[String], operation: SelectionOperation) -> Result<Success, CliError> {
    let name = match operation {
        SelectionOperation::Stage => "stage",
        SelectionOperation::Unstage => "unstage",
    };
    let (path, mut tail) = path_arg(rest, name)?;
    let mut items = Vec::new();
    let mut approved = Vec::new();
    while let Some((arg, more)) = tail.split_first() {
        if arg == "--with" {
            let Some((value, more)) = more.split_first() else {
                return Err(CliError::usage("--with에는 대상이 필요합니다"));
            };
            approved.push(parse_item(value)?);
            tail = more;
        } else if arg.starts_with('-') {
            return Err(CliError::usage(format!("알 수 없는 옵션: {arg}")));
        } else {
            items.push(parse_item(arg)?);
            tail = more;
        }
    }
    if items.is_empty() {
        return Err(CliError::usage(format!(
            "git {name}에는 대상이 하나 이상 필요합니다"
        )));
    }
    let session = open(path)?;
    let outcome = session
        .select(
            &SelectionInput {
                operation,
                items,
                approved,
            },
            &stellar_ops::git::CancelToken::new(),
        )
        .map_err(|e| git_error(&e).with_context(path.as_str()))?;
    Ok(Success {
        result: selection_json(&outcome),
        diagnostics: Vec::new(),
    })
}

fn commit(rest: &[String]) -> Result<Success, CliError> {
    let (path, tail) = path_arg(rest, "commit")?;
    let message = match tail {
        [flag, message] if flag == "-m" || flag == "--message" => message,
        _ => {
            return Err(CliError::usage(
                "git commit에는 <경로> -m <메시지>가 필요합니다",
            ))
        }
    };
    let session = open(path)?;
    let outcome = session
        .commit(message)
        .map_err(|e| git_error(&e).with_context(path.as_str()))?;
    let diagnostics = outcome.warnings.iter().map(finding_diagnostic).collect();
    Ok(Success {
        result: commit_json(&outcome),
        diagnostics,
    })
}

fn lossy(bytes: &[u8]) -> JsonValue {
    JsonValue::string(String::from_utf8_lossy(bytes).into_owned())
}

fn kebab(debug: String) -> String {
    let mut out = String::new();
    for (i, c) in debug.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn head_json(head: &HeadState) -> JsonValue {
    match head {
        HeadState::Branch { name, commit } => JsonValue::object(vec![
            ("state", JsonValue::string("branch")),
            ("branch", lossy(name)),
            ("commit", JsonValue::string(commit.to_string())),
        ]),
        HeadState::Detached { commit } => JsonValue::object(vec![
            ("state", JsonValue::string("detached")),
            ("branch", JsonValue::Null),
            ("commit", JsonValue::string(commit.to_string())),
        ]),
        HeadState::Unborn { branch } => JsonValue::object(vec![
            ("state", JsonValue::string("unborn")),
            ("branch", lossy(branch)),
            ("commit", JsonValue::Null),
        ]),
    }
}

fn upstream_json(upstream: &UpstreamState) -> JsonValue {
    match upstream {
        UpstreamState::None => JsonValue::object(vec![("state", JsonValue::string("none"))]),
        UpstreamState::Tracking {
            name,
            ahead,
            behind,
        } => JsonValue::object(vec![
            ("state", JsonValue::string("tracking")),
            ("name", lossy(name)),
            ("ahead", views::uint(*ahead)),
            ("behind", views::uint(*behind)),
        ]),
        UpstreamState::Gone { name } => JsonValue::object(vec![
            ("state", JsonValue::string("gone")),
            ("name", lossy(name)),
        ]),
    }
}

fn repository_fields(
    repository: &stellar_ops::git::GitRepositoryStatus,
) -> Vec<(&'static str, JsonValue)> {
    let counts = &repository.counts;
    vec![
        ("head", head_json(&repository.head)),
        ("upstream", upstream_json(&repository.upstream)),
        (
            "counts",
            JsonValue::object(vec![
                ("staged", views::uint(counts.staged)),
                ("unstaged", views::uint(counts.unstaged)),
                ("untracked", views::uint(counts.untracked)),
                ("conflicted", views::uint(counts.conflicted)),
            ]),
        ),
        (
            "in_progress",
            JsonValue::Array(
                repository
                    .conflict
                    .operations
                    .iter()
                    .map(|op| JsonValue::string(kebab(format!("{op:?}"))))
                    .collect(),
            ),
        ),
    ]
}

fn change_json(entry: &ChangeEntry) -> JsonValue {
    JsonValue::object(vec![
        ("id", JsonValue::string(change_id_text(&entry.id))),
        (
            "area",
            JsonValue::string(match entry.area {
                StatusArea::Staged => "staged",
                StatusArea::Unstaged => "unstaged",
            }),
        ),
        ("kind", JsonValue::string(kind_name(entry.kind))),
        ("object", views::id(entry.object)),
        ("document", views::opt_id(entry.document)),
        (
            "property",
            entry.property.as_ref().map_or(JsonValue::Null, |p| {
                JsonValue::string(property_path_label(p))
            }),
        ),
    ])
}

fn kind_name(kind: SemanticChangeKind) -> String {
    kind.name().to_string()
}

fn missing_keys(keys: Vec<IdentityKey>) -> JsonValue {
    JsonValue::Array(
        keys.into_iter()
            .map(|k| JsonValue::string(k.config_key()))
            .collect(),
    )
}

fn status_json(report: &GitStatusReport) -> JsonValue {
    let mut fields = vec![
        ("revision", JsonValue::string(report.revision.to_string())),
        (
            "worktree",
            JsonValue::string(match report.location.kind {
                WorktreeKind::Main => "main",
                WorktreeKind::Linked => "linked",
            }),
        ),
    ];
    fields.extend(repository_fields(&report.repository));
    fields.push((
        "identity",
        JsonValue::object(vec![
            (
                "configured",
                JsonValue::Bool(report.missing_identity.is_none()),
            ),
            (
                "missing",
                missing_keys(
                    report
                        .missing_identity
                        .map(|m| m.missing_keys())
                        .unwrap_or_default(),
                ),
            ),
        ]),
    ));
    fields.push((
        "remotes",
        JsonValue::Array(
            report
                .remotes
                .iter()
                .map(|r| {
                    JsonValue::object(vec![
                        ("name", JsonValue::string(&r.name)),
                        ("fetch_url", JsonValue::opt_string(r.fetch_url.as_deref())),
                        ("push_urls", views::count(r.push_urls)),
                    ])
                })
                .collect(),
        ),
    ));
    fields.push((
        "units",
        JsonValue::Array(
            report
                .units
                .iter()
                .map(|u| {
                    JsonValue::object(vec![
                        ("id", JsonValue::string(u.unit.to_string())),
                        ("staged", JsonValue::Bool(u.staged)),
                        ("unstaged", JsonValue::Bool(u.unstaged)),
                        ("untracked", JsonValue::Bool(u.untracked)),
                        ("conflicted", JsonValue::Bool(u.conflicted)),
                        ("paths", views::count(u.paths)),
                    ])
                })
                .collect(),
        ),
    ));
    fields.push((
        "raw",
        JsonValue::Array(
            report
                .raw
                .iter()
                .map(|r| {
                    JsonValue::object(vec![
                        ("path", lossy(r.path.as_bytes())),
                        ("xy", JsonValue::string(&r.xy)),
                    ])
                })
                .collect(),
        ),
    ));
    fields.push((
        "semantic",
        match &report.semantic {
            Ok(page) => JsonValue::object(vec![
                ("staged_total", views::count(page.staged_total)),
                ("unstaged_total", views::count(page.unstaged_total)),
                (
                    "changes",
                    JsonValue::Array(page.entries.iter().map(change_json).collect()),
                ),
                (
                    "next_cursor",
                    JsonValue::opt_string(page.next_cursor.as_deref()),
                ),
            ]),
            Err(_) => JsonValue::Null,
        },
    ));
    JsonValue::object(fields)
}

fn units_json<'a>(units: impl IntoIterator<Item = &'a SourceUnit>) -> JsonValue {
    JsonValue::Array(
        units
            .into_iter()
            .map(|u| JsonValue::string(u.to_string()))
            .collect(),
    )
}

fn selection_json(outcome: &SelectionOutcome) -> JsonValue {
    JsonValue::object(vec![
        ("applied_changes", views::count(outcome.applied_changes)),
        ("units", units_json(&outcome.units)),
        ("index_entries", views::count(outcome.index_entries)),
        ("revision", JsonValue::string(outcome.revision.to_string())),
    ])
}

fn commit_json(outcome: &CommitOutcome) -> JsonValue {
    let mut fields = vec![
        ("commit", JsonValue::string(outcome.head.to_string())),
        (
            "previous_head",
            outcome
                .previous_head
                .as_ref()
                .map_or(JsonValue::Null, |h| JsonValue::string(h.to_string())),
        ),
        ("staged_remaining", views::count(outcome.staged_remaining)),
        (
            "tree_matches_validated",
            JsonValue::Bool(outcome.tree_matches_validated),
        ),
        ("revision", JsonValue::string(outcome.revision.to_string())),
    ];
    fields.extend(repository_fields(&outcome.repository));
    JsonValue::object(fields)
}

fn finding_json(finding: &StagedFinding) -> JsonValue {
    JsonValue::object(vec![
        (
            "severity",
            JsonValue::string(if finding.error { "error" } else { "warning" }),
        ),
        ("code", JsonValue::string(finding.code)),
        ("document", views::opt_id(finding.document)),
        ("message", JsonValue::string(&finding.message)),
    ])
}

fn finding_diagnostic(finding: &StagedFinding) -> Diagnostic {
    Diagnostic {
        severity: if finding.error {
            Severity::Error
        } else {
            Severity::Warning
        },
        code: finding.code,
        message: finding.message.clone(),
    }
}

/// 엔진 오류 → CLI 오류. 필수 동반 변경·재검증 결과는 `result`에 담습니다.
pub fn git_error(error: &GitError) -> CliError {
    use stellar_ops::git::GitCommitError as CommitError;
    let message = error.to_string();
    match error {
        GitError::NotTrusted => CliError::new(ExitStatus::NoPerm, "not-trusted", message),
        GitError::GitUnavailable => {
            CliError::new(ExitStatus::Unavailable, "git-unavailable", message)
        }
        GitError::Path(kind) => CliError::new(ExitStatus::from_io_kind(*kind), "path", message),
        GitError::NotWorktreeRoot | GitError::Location(_) => {
            CliError::new(ExitStatus::NoInput, "not-a-worktree-root", message)
        }
        GitError::CompanionRequired {
            changes,
            units,
            reasons,
        } => {
            let required: Vec<JsonValue> = changes
                .iter()
                .map(|c| JsonValue::string(change_id_text(c)))
                .chain(units.iter().map(|u| JsonValue::string(u.to_string())))
                .collect();
            CliError::new(ExitStatus::Failure, "companion-required", message).with_result(
                JsonValue::object(vec![
                    ("required", JsonValue::Array(required)),
                    (
                        "reasons",
                        JsonValue::Array(
                            reasons
                                .iter()
                                .map(|r| {
                                    JsonValue::object(vec![
                                        ("unit", JsonValue::string(r.unit.to_string())),
                                        ("component", views::id(r.component)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]),
            )
        }
        GitError::UnexpectedApproval(_) => {
            CliError::new(ExitStatus::Failure, "unexpected-approval", message)
        }
        GitError::UnknownChange(_) => {
            CliError::new(ExitStatus::Failure, "stale-selection", message)
        }
        GitError::EmptySelection => CliError::usage(message),
        GitError::StaleCursor => CliError::new(ExitStatus::TempFail, "stale-cursor", message),
        GitError::InvalidCursor | GitError::InvalidLimit(_) => CliError::usage(message),
        GitError::InvalidStagedSnapshot(findings) => {
            let mut error = CliError::new(ExitStatus::Failure, "invalid-staged-snapshot", message)
                .with_result(JsonValue::object(vec![(
                    "findings",
                    JsonValue::Array(findings.iter().map(finding_json).collect()),
                )]));
            error.diagnostics = findings.iter().map(finding_diagnostic).collect();
            error
        }
        GitError::IndexChangedBeforeCommit => {
            CliError::new(ExitStatus::TempFail, "index-changed", message)
        }
        GitError::Commit(CommitError::MissingIdentity(missing)) => {
            CliError::new(ExitStatus::Config, "missing-identity", message).with_result(
                JsonValue::object(vec![("missing", missing_keys(missing.missing_keys()))]),
            )
        }
        GitError::Commit(CommitError::Git(failure)) => {
            CliError::new(ExitStatus::Failure, "git-commit-failed", message).with_result(
                JsonValue::object(vec![
                    (
                        "exit_code",
                        failure
                            .exit_code
                            .map_or(JsonValue::Null, |c| JsonValue::int(i64::from(c))),
                    ),
                    ("stderr", lossy(&failure.stderr)),
                ]),
            )
        }
        _ => CliError::new(ExitStatus::Failure, "git-error", message),
    }
}
