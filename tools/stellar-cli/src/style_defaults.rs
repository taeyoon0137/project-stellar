//! `style-defaults`: 기본값 세트 목록·내용·선택과 node 값 출처 (SD.5·SD.6, ADR-0082, ADR-0086).
//!
//! - `list <경로>`: 내장·사용자 세트와 활성 세트.
//! - `show <경로> [<세트>]`: 세트 사슬과 구획별 값 표(기본은 활성 세트).
//! - `set <경로> <세트>`: `stellar.yaml`의 `style_defaults` 한 줄을 바꿉니다(프로젝트 저장 잠금을 잡음).
//! - `resolve <경로> <문서 id> <node id>`: node의 layout longhand 값과 출처(`explicit`·`default(<세트>)`).
//!
//! 해석은 앱·MCP와 같은 core 함수(`stellar_ops::style_defaults`)입니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::lock::{default_lock_dir, SaveLock};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::style_defaults::layout_record::{format_value, LayoutLonghand};
use stellar_ops::style_defaults::{
    self as sd, active_name, flatten_named, load_user_sets, resolve, set_active, FlatSet,
    NodeClass, StyleDefaultsFileError,
};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

fn file_error(error: StyleDefaultsFileError) -> CliError {
    let status = match &error {
        StyleDefaultsFileError::Io { .. } => ExitStatus::IoErr,
        StyleDefaultsFileError::Set(sd::StyleDefaultsError::UnknownSet { .. }) => {
            ExitStatus::NoInput
        }
        _ => ExitStatus::DataErr,
    };
    CliError::new(status, error.code(), error.to_string())
}

fn parse_id(value: &str, kind: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
}

fn reject_options(args: &[String]) -> Result<(), CliError> {
    match args.iter().find(|a| a.starts_with('-')) {
        Some(option) => Err(CliError::usage(format!("알 수 없는 옵션: {option}"))),
        None => Ok(()),
    }
}

/// `style-defaults list|show|set|resolve …`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    match args.split_first() {
        Some((sub, rest)) if sub == "list" => run_list(rest),
        Some((sub, rest)) if sub == "show" => run_show(rest),
        Some((sub, rest)) if sub == "set" => run_set(rest),
        Some((sub, rest)) if sub == "resolve" => run_resolve(rest),
        _ => Err(CliError::usage(
            "style-defaults에는 list <경로> | show <경로> [<세트>] | set <경로> <세트> | resolve <경로> <문서 id> <node id>가 필요합니다",
        )),
    }
}

fn success(result: JsonValue) -> Success {
    Success {
        result,
        diagnostics: Vec::new(),
    }
}

fn run_list(args: &[String]) -> Result<Success, CliError> {
    reject_options(args)?;
    let [path] = args else {
        return Err(CliError::usage(
            "style-defaults list에는 <프로젝트 경로>가 필요합니다",
        ));
    };
    let snapshot = open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e))?;
    let sets = load_user_sets(snapshot.root()).map_err(file_error)?;
    let active = active_name(snapshot.manifest()).to_string();
    let mut items: Vec<JsonValue> = sd::BUILTIN_SETS
        .iter()
        .map(|name| {
            JsonValue::object(vec![
                ("name", JsonValue::string(*name)),
                ("kind", JsonValue::string("builtin")),
                ("active", JsonValue::Bool(*name == active)),
            ])
        })
        .collect();
    for (name, source) in &sets {
        items.push(JsonValue::object(vec![
            ("name", JsonValue::string(name.clone())),
            ("kind", JsonValue::string("user")),
            ("extends", JsonValue::string(source.extends.clone())),
            ("path", JsonValue::string(sd::set_file_path(name))),
            ("active", JsonValue::Bool(*name == active)),
        ]));
    }
    Ok(success(JsonValue::object(vec![
        ("revision", views::revision(snapshot.revision())),
        ("active", JsonValue::string(active)),
        ("sets", JsonValue::Array(items)),
    ])))
}

fn set_table(set: &FlatSet) -> JsonValue {
    JsonValue::Array(
        LayoutLonghand::ALL
            .iter()
            .map(|longhand| {
                let mut fields = vec![("longhand", JsonValue::string(longhand.name()))];
                for class in NodeClass::ALL {
                    fields.push((
                        class.name(),
                        JsonValue::string(format_value(set.value(class, *longhand))),
                    ));
                }
                JsonValue::object(fields)
            })
            .collect(),
    )
}

fn run_show(args: &[String]) -> Result<Success, CliError> {
    reject_options(args)?;
    let (path, name) = match args {
        [path] => (path, None),
        [path, name] => (path, Some(name.as_str())),
        _ => {
            return Err(CliError::usage(
                "style-defaults show에는 <프로젝트 경로> [<세트 이름>]이 필요합니다",
            ))
        }
    };
    let snapshot = open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e))?;
    let sets = load_user_sets(snapshot.root()).map_err(file_error)?;
    let name = name.unwrap_or_else(|| active_name(snapshot.manifest()));
    let set = flatten_named(name, &sets).map_err(file_error)?;
    Ok(success(JsonValue::object(vec![
        ("revision", views::revision(snapshot.revision())),
        ("name", JsonValue::string(set.name())),
        (
            "chain",
            JsonValue::Array(
                set.chain()
                    .iter()
                    .map(|n| JsonValue::string(n.clone()))
                    .collect(),
            ),
        ),
        ("values", set_table(&set)),
    ])))
}

fn run_set(args: &[String]) -> Result<Success, CliError> {
    reject_options(args)?;
    let [path, name] = args else {
        return Err(CliError::usage(
            "style-defaults set에는 <프로젝트 경로> <세트 이름>이 필요합니다",
        ));
    };
    let snapshot = open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e))?;
    let lock_error = |e: stellar_ops::lock::LockError| {
        CliError::new(ExitStatus::TempFail, "lock", e.to_string())
    };
    let dir = default_lock_dir().map_err(lock_error)?;
    let _lock =
        SaveLock::try_acquire(&dir, snapshot.project_id(), snapshot.root()).map_err(lock_error)?;
    let previous = set_active(snapshot.root(), name).map_err(file_error)?;
    let changed = previous != *name;
    Ok(success(JsonValue::object(vec![
        ("previous", JsonValue::string(previous)),
        ("active", JsonValue::string(name.clone())),
        ("changed", JsonValue::Bool(changed)),
    ])))
}

fn run_resolve(args: &[String]) -> Result<Success, CliError> {
    reject_options(args)?;
    let [path, document, node] = args else {
        return Err(CliError::usage(
            "style-defaults resolve에는 <프로젝트 경로> <문서 id> <node id>가 필요합니다",
        ));
    };
    let document = parse_id(document, "문서 id")?;
    let node = parse_id(node, "node id")?;
    let snapshot = open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e))?;
    let sets = load_user_sets(snapshot.root()).map_err(file_error)?;
    let set = flatten_named(active_name(snapshot.manifest()), &sets).map_err(file_error)?;
    let model = snapshot
        .document_model(document)
        .map_err(|e| views::document_error(&e))?;
    let payload = model.payload(node).ok_or_else(|| {
        CliError::new(
            ExitStatus::NoInput,
            "unknown-node",
            format!("문서 {document}에 node {node}가 없습니다"),
        )
    })?;
    let resolved = resolve(&set, payload, model.layout_record(node)).ok_or_else(|| {
        CliError::new(
            ExitStatus::DataErr,
            "no-box",
            "Fragment는 상자가 없어 layout longhand가 없습니다",
        )
    })?;
    let values = resolved
        .values
        .iter()
        .map(|v| {
            JsonValue::object(vec![
                ("longhand", JsonValue::string(v.longhand.name())),
                ("value", JsonValue::string(format_value(&v.value))),
                ("source", JsonValue::string(v.source.to_string())),
            ])
        })
        .collect();
    Ok(success(JsonValue::object(vec![
        ("revision", views::revision(snapshot.revision())),
        ("document", views::id(document)),
        ("node", views::id(node)),
        ("style_defaults", JsonValue::string(set.name())),
        ("class", JsonValue::string(resolved.class.name())),
        ("values", JsonValue::Array(values)),
    ])))
}
