//! `stellar library add|update|fetch|import`: 외부 library 의존성 추가·update·cache 받기·외부 component 참조 저장
//! (XV.4.3·XV.7.4·XV.7.5, ADR-0137).
//!
//! 동작은 모두 `stellar_ops::library_materialize`가 합니다(앱과 같은 resolver·cache·lock 기록·후보 검증). 이 모듈은 인자를
//! 해석하고 결과를 JSON으로 바꿉니다. 공용 cache는 OS 사용자 데이터 위치(`xyz.taeyoon.stellar/library-cache`)입니다.
//!
//! - `add <경로> <이름> <repository> (--tag|--branch|--commit) <값> [--component <id>]...`: 의존성을 더하고 lock·cache를
//!   갱신합니다. library id는 해석한 commit의 `stellar.yaml` `project.id`입니다.
//! - `update <경로> <이름> [(--tag|--branch|--commit) <값>] [--allow-orphans] [--dry-run]`: 선택자를 다시 해석(또는 바꿈)하고
//!   사용 프로젝트 참조를 후보에 대어 본 뒤 적용합니다. 끊기는 참조가 있으면 `--allow-orphans` 없이는 종료 코드 1이고 아무것도
//!   쓰지 않습니다.
//! - `fetch <경로>`: lock의 모든 library를 cache에 받습니다(lock·`stellar.yaml` 불변).
//! - `import <경로> <이름> <component id>... [--remove]`: 외부 component 참조를 `stellar.yaml`에 저장(또는 삭제)합니다.
//!
//! repository URL은 사용자가 준 값 그대로 Git에만 넘기고 결과·오류에 다시 쓰지 않습니다(XV.1.4).

use std::collections::BTreeSet;
use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::reference::ComponentRef;
use stellar_ops::library::LibraryCache;
use stellar_ops::library_materialize::{
    self as library, AddRequest, DependencyName, DiffCounts, LibraryCommandError, LibraryTools,
    LockReport, NodeReport, RefName, Repository, Reuse, Selector, UpdateRequest,
};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

/// 하위 명령 사용법.
pub const USAGE: &str = "add <경로> <이름> <repository> (--tag|--branch|--commit) <값> [--component <id>]... | update <경로> <이름> [(--tag|--branch|--commit) <값>] [--allow-orphans] [--dry-run] | fetch <경로> | import <경로> <이름> <component id>... [--remove]";

/// `library <하위 명령> ...`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(CliError::usage(format!(
            "library에는 하위 명령이 필요합니다: {USAGE}"
        )));
    };
    match sub.as_str() {
        "add" => run_add(rest),
        "update" => run_update(rest),
        "fetch" => run_fetch(rest),
        "import" => run_import(rest),
        other => Err(CliError::usage(format!(
            "알 수 없는 library 하위 명령: {other} ({USAGE})"
        ))),
    }
}

fn success(result: JsonValue) -> Success {
    Success {
        result,
        diagnostics: Vec::new(),
    }
}

fn count(n: usize) -> JsonValue {
    JsonValue::int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn parse_name(text: &str) -> Result<DependencyName, CliError> {
    DependencyName::new(text)
        .map_err(|e| CliError::usage(format!("의존성 이름이 올바르지 않습니다: {e}")))
}

fn parse_component(text: &str) -> Result<ComponentRef, CliError> {
    text.parse::<StableId>()
        .ok()
        .filter(|id| id.to_string() == text)
        .map(ComponentRef::new)
        .ok_or_else(|| {
            CliError::usage(format!(
                "component id는 canonical 소문자 UUID여야 합니다: {text}"
            ))
        })
}

/// `--tag|--branch|--commit <값>` 하나.
fn parse_selector(flag: &str, value: &str) -> Result<Selector, CliError> {
    let invalid =
        |e: &dyn std::fmt::Display| CliError::usage(format!("{flag} 값이 올바르지 않습니다: {e}"));
    match flag {
        "--tag" => RefName::new(value)
            .map(Selector::Tag)
            .map_err(|e| invalid(&e)),
        "--branch" => RefName::new(value)
            .map(Selector::Branch)
            .map_err(|e| invalid(&e)),
        "--commit" => value
            .parse()
            .map(Selector::Commit)
            .map_err(|e: stellar_ops::library_materialize::SelectorValueError| invalid(&e)),
        _ => Err(CliError::usage(format!("알 수 없는 옵션: {flag}"))),
    }
}

/// 옵션 해석 결과.
#[derive(Default)]
struct Options {
    selector: Option<Selector>,
    components: BTreeSet<ComponentRef>,
    allow_orphans: bool,
    dry_run: bool,
    remove: bool,
    positional: Vec<String>,
}

/// `allowed`에 있는 옵션만 받습니다. 선택자는 하나만.
fn parse_options(args: &[String], allowed: &[&str]) -> Result<Options, CliError> {
    let mut options = Options::default();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        if !arg.starts_with('-') {
            options.positional.push(arg.clone());
            continue;
        }
        if !allowed.contains(&arg.as_str()) {
            return Err(CliError::usage(format!("알 수 없는 옵션: {arg}")));
        }
        match arg.as_str() {
            "--tag" | "--branch" | "--commit" | "--component" => {
                let Some((value, tail)) = rest.split_first() else {
                    return Err(CliError::usage(format!("{arg}에는 값이 필요합니다")));
                };
                rest = tail;
                if arg == "--component" {
                    if !options.components.insert(parse_component(value)?) {
                        return Err(CliError::usage(format!(
                            "같은 component를 두 번 줬습니다: {value}"
                        )));
                    }
                } else if options.selector.is_some() {
                    return Err(CliError::usage(
                        "--tag·--branch·--commit은 하나만 줄 수 있습니다",
                    ));
                } else {
                    options.selector = Some(parse_selector(arg, value)?);
                }
            }
            "--allow-orphans" if !options.allow_orphans => options.allow_orphans = true,
            "--dry-run" if !options.dry_run => options.dry_run = true,
            "--remove" if !options.remove => options.remove = true,
            _ => return Err(CliError::usage(format!("중복된 옵션: {arg}"))),
        }
    }
    Ok(options)
}

fn tools() -> Result<LibraryTools, CliError> {
    LibraryTools::os_default().map_err(|e| command_error(&e))
}

fn run_add(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, &["--tag", "--branch", "--commit", "--component"])?;
    let [path, name, repository] = options.positional.as_slice() else {
        return Err(CliError::usage(format!("library add: {USAGE}")));
    };
    let Some(version) = options.selector else {
        return Err(CliError::usage(
            "library add에는 --tag·--branch·--commit 가운데 하나가 필요합니다",
        ));
    };
    let request = AddRequest {
        name: parse_name(name)?,
        repository: Repository::new(repository)
            .map_err(|e| CliError::usage(format!("repository가 올바르지 않습니다: {e}")))?,
        version,
        components: options.components,
    };
    let mut tools = tools()?;
    let report = library::add(
        Path::new(path),
        &mut tools,
        &request,
        stellar_ops::library::now_unix_ms(),
    )
    .map_err(|e| command_error(&e).with_context(path.clone()))?;
    Ok(success(JsonValue::object(vec![
        ("operation", JsonValue::string("add")),
        ("dependency", JsonValue::string(request.name.as_str())),
        ("lock", lock_json(&report)),
    ])))
}

fn run_update(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(
        args,
        &[
            "--tag",
            "--branch",
            "--commit",
            "--allow-orphans",
            "--dry-run",
        ],
    )?;
    let [path, name] = options.positional.as_slice() else {
        return Err(CliError::usage(format!("library update: {USAGE}")));
    };
    let request = UpdateRequest {
        name: parse_name(name)?,
        version: options.selector,
        allow_orphans: options.allow_orphans,
        dry_run: options.dry_run,
    };
    let mut tools = tools()?;
    let report = library::update(
        Path::new(path),
        &mut tools,
        &request,
        stellar_ops::library::now_unix_ms(),
    )
    .map_err(|e| command_error(&e).with_context(path.clone()))?;
    Ok(success(JsonValue::object(vec![
        ("operation", JsonValue::string("update")),
        ("dependency", JsonValue::string(report.name.as_str())),
        (
            "previous",
            JsonValue::opt_string(report.previous.as_ref().map(|c| c.as_str())),
        ),
        ("candidate", JsonValue::string(report.candidate.as_str())),
        ("diff", report.diff.map_or(JsonValue::Null, diff_json)),
        (
            "orphans",
            JsonValue::Array(
                report
                    .orphans
                    .iter()
                    .map(|d| JsonValue::string(d.to_string()))
                    .collect(),
            ),
        ),
        (
            "ambiguous",
            JsonValue::Array(report.ambiguous.iter().map(|id| views::id(*id)).collect()),
        ),
        ("applied", JsonValue::Bool(report.applied)),
        ("lock", lock_json(&report.lock)),
    ])))
}

fn run_fetch(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, &[])?;
    let [path] = options.positional.as_slice() else {
        return Err(CliError::usage(format!("library fetch: {USAGE}")));
    };
    let mut tools = tools()?;
    let report = library::fetch(
        Path::new(path),
        &mut tools,
        stellar_ops::library::now_unix_ms(),
    )
    .map_err(|e| command_error(&e).with_context(path.clone()))?;
    Ok(success(JsonValue::object(vec![
        ("operation", JsonValue::string("fetch")),
        (
            "nodes",
            JsonValue::Array(report.nodes.iter().map(node_json).collect()),
        ),
    ])))
}

fn run_import(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, &["--remove"])?;
    let Some((path, rest)) = options.positional.split_first() else {
        return Err(CliError::usage(format!("library import: {USAGE}")));
    };
    let Some((name, ids)) = rest.split_first() else {
        return Err(CliError::usage(format!("library import: {USAGE}")));
    };
    if ids.is_empty() {
        return Err(CliError::usage(
            "library import에는 component id가 하나 이상 필요합니다",
        ));
    }
    let mut components = BTreeSet::new();
    for id in ids {
        if !components.insert(parse_component(id)?) {
            return Err(CliError::usage(format!(
                "같은 component를 두 번 줬습니다: {id}"
            )));
        }
    }
    let name = parse_name(name)?;
    let cache = if options.remove {
        // 삭제는 library를 읽지 않습니다.
        None
    } else {
        Some(LibraryCache::os_default().map_err(|e| {
            command_error(&LibraryCommandError::Cache(e)).with_context(path.clone())
        })?)
    };
    let report = match &cache {
        Some(cache) => library::import(
            Path::new(path),
            cache,
            &name,
            &components,
            stellar_ops::library::now_unix_ms(),
        ),
        None => library::remove_imports(Path::new(path), &name, &components),
    }
    .map_err(|e| command_error(&e).with_context(path.clone()))?;
    Ok(success(JsonValue::object(vec![
        (
            "operation",
            JsonValue::string(if options.remove { "remove" } else { "import" }),
        ),
        ("dependency", JsonValue::string(report.name.as_str())),
        (
            "components",
            JsonValue::Array(report.components.iter().map(|id| views::id(*id)).collect()),
        ),
        ("manifest_written", JsonValue::Bool(report.manifest_written)),
    ])))
}

fn reuse_name(reuse: Reuse) -> &'static str {
    match reuse {
        Reuse::CommitHit => "commit-hit",
        Reuse::ContentHit => "content-hit",
        Reuse::Created => "created",
    }
}

fn node_json(node: &NodeReport) -> JsonValue {
    JsonValue::object(vec![
        ("path", JsonValue::string(node.path.to_string())),
        (
            "library_id",
            JsonValue::string(node.node.library_id.to_string()),
        ),
        ("commit", JsonValue::string(node.node.commit.as_str())),
        (
            "content_hash",
            JsonValue::string(node.content_hash.as_str()),
        ),
        ("reuse", JsonValue::string(reuse_name(node.reuse))),
    ])
}

fn lock_json(report: &LockReport) -> JsonValue {
    JsonValue::object(vec![
        (
            "nodes",
            JsonValue::Array(report.nodes.iter().map(node_json).collect()),
        ),
        ("pinned", count(report.pinned)),
        ("manifests_from_cache", count(report.manifests_from_cache)),
        ("manifest_written", JsonValue::Bool(report.manifest_written)),
        ("lock_written", JsonValue::Bool(report.lock_written)),
    ])
}

fn diff_json(diff: DiffCounts) -> JsonValue {
    JsonValue::object(vec![
        ("added", count(diff.added)),
        ("removed", count(diff.removed)),
        ("modified", count(diff.modified)),
    ])
}

/// 명령 오류 → CLI 오류. 끊기는 참조는 `result.orphans`에 담습니다.
pub fn command_error(error: &LibraryCommandError) -> CliError {
    let message = error.to_string();
    let (status, code) = match error {
        LibraryCommandError::Cache(_) | LibraryCommandError::CacheInsideProject => {
            (ExitStatus::Config, "library-cache")
        }
        LibraryCommandError::GitUnavailable => (ExitStatus::Unavailable, "git-unavailable"),
        LibraryCommandError::Manifest(_) => (ExitStatus::DataErr, "manifest"),
        LibraryCommandError::Lock(_) | LibraryCommandError::ContentHashMismatch { .. } => {
            (ExitStatus::DataErr, "lock")
        }
        LibraryCommandError::NotLocked => (ExitStatus::NoInput, "not-locked"),
        LibraryCommandError::DuplicateDependency { .. } => {
            (ExitStatus::Failure, "duplicate-dependency")
        }
        LibraryCommandError::UnknownDependency { .. } => {
            (ExitStatus::NoInput, "unknown-dependency")
        }
        LibraryCommandError::Declarations { .. } => (ExitStatus::Failure, "declaration-conflict"),
        LibraryCommandError::Selector(_)
        | LibraryCommandError::LibraryManifest(_)
        | LibraryCommandError::Resolve(_) => (ExitStatus::Failure, "resolve-failed"),
        LibraryCommandError::Materialize { .. } => (ExitStatus::TempFail, "fetch-failed"),
        LibraryCommandError::Rejected { .. } => (ExitStatus::DataErr, "invalid-library"),
        LibraryCommandError::LockBuild(_) => (ExitStatus::Software, "lock-build"),
        LibraryCommandError::LockWrite(_) | LibraryCommandError::RestoreFailed { .. } => {
            (ExitStatus::IoErr, "write-failed")
        }
        LibraryCommandError::UnknownComponents { .. } => (ExitStatus::Failure, "unknown-component"),
        LibraryCommandError::LibraryUnavailable { .. } => {
            (ExitStatus::Unavailable, "library-unavailable")
        }
        LibraryCommandError::OrphansBlocked { .. } => (ExitStatus::Failure, "orphans"),
    };
    let error_value = CliError::new(status, code, message);
    match error {
        LibraryCommandError::OrphansBlocked { diagnostics } => {
            error_value.with_result(JsonValue::object(vec![(
                "orphans",
                JsonValue::Array(
                    diagnostics
                        .iter()
                        .map(|d| JsonValue::string(d.to_string()))
                        .collect(),
                ),
            )]))
        }
        LibraryCommandError::UnknownComponents { components, .. } => {
            error_value.with_result(JsonValue::object(vec![(
                "components",
                JsonValue::Array(components.iter().map(|id| views::id(*id)).collect()),
            )]))
        }
        _ => error_value,
    }
}
