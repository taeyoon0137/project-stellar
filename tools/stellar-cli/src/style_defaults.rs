//! `style-defaults`: 기본값 세트 목록·내용·선택과 node 값 출처 (SD.5·SD.6, ADR-0082, ADR-0086).
//!
//! - `list <경로>`: 내장·사용자 세트와 활성 세트.
//! - `show <경로> [<세트>]`: 세트 사슬과 구획별 값 표(기본은 활성 세트).
//! - `show`는 layout longhand 표(`values`)와 payload longhand 표(`payload`, registry 항목)를 함께 냅니다.
//! - `set <경로> <세트> [--base <revision>] [--dry-run]`: 활성 세트를 바꿉니다(`SetStyleDefaults`의 영속 결과).
//!   ops 쓰기 경로의 순서(교차 프로세스 저장 잠금 → 다시 열기 → base revision 검사)를 지키고, 결과는 두 단계 쓰기
//!   (`prepare_selection` → `commit`)의 `stellar.yaml` 한 줄입니다. 문서 레코드는 바뀌지 않아 generation을 쓰지
//!   않습니다(ADR-0087 결정 2, ADR-0115 결정 4). `--base`가 없으면 지금 revision을 base로 씁니다.
//! - `resolve <경로> <문서 id> <node id>`: node의 layout longhand 값과 출처(`explicit`·`default(<세트>)`).
//!   `stellar query … node`의 `layout`과 같은 `stellar_ops::query::node_layout` 결과입니다.
//!
//! 해석은 앱·MCP와 같은 core 함수(`stellar_ops::style_defaults`, `stellar_ops::query`)입니다.

use std::path::Path;

use stellar_ops::apply::ApplyError;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::lock::{default_lock_dir, SaveLock};
use stellar_ops::query::{node_layout, LayoutUnavailable};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::style_defaults::layout_record::{format_value, LayoutLonghand};
use stellar_ops::style_defaults::{
    self as sd, active_name, flatten_named, load_user_sets, payload_longhands, prepare_selection,
    scope_class, FlatSet, NodeClass, StyleDefaultsFileError,
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
            "style-defaults에는 list <경로> | show <경로> [<세트>] | set <경로> <세트> [--base <revision>] [--dry-run] | resolve <경로> <문서 id> <node id>가 필요합니다",
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

/// payload longhand 표(registry 순서): `{scope, key, class, longhand, value}`.
fn payload_table(set: &FlatSet) -> JsonValue {
    JsonValue::Array(
        payload_longhands()
            .iter()
            .map(|entry| {
                let class = scope_class(entry.scope).expect("payload 범위");
                JsonValue::object(vec![
                    ("scope", JsonValue::string(entry.scope.name())),
                    ("key", JsonValue::string(format!("0x{:04x}", entry.key.0))),
                    ("class", JsonValue::string(class.name())),
                    ("longhand", JsonValue::string(entry.name)),
                    (
                        "value",
                        JsonValue::opt_string(set.payload_value(entry.scope, entry.key)),
                    ),
                ])
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
        ("payload", payload_table(&set)),
    ])))
}

fn run_set(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut base: Option<ProjectRevision> = None;
    let mut dry_run = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dry-run" if !dry_run => dry_run = true,
            "--base" if base.is_none() => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::usage("--base에는 revision이 필요합니다"))?;
                base = Some(value.parse().map_err(|e| {
                    CliError::usage(format!("--base revision 형식이 아닙니다: {e}"))
                })?);
            }
            other if other.starts_with('-') => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )))
            }
            _ => positional.push(arg),
        }
    }
    let [path, name] = positional[..] else {
        return Err(CliError::usage(
            "style-defaults set에는 <프로젝트 경로> <세트 이름>이 필요합니다",
        ));
    };
    let root = Path::new(path);
    let opened = open_snapshot(root).map_err(|e| views::open_error(&e))?;
    // 쓰기는 교차 프로세스 저장 잠금(XXIV.4.5)을 기다리지 않고 잡은 뒤에 다시 열어 base revision을 봅니다(ops 쓰기
    // 경로와 같은 순서). dry-run은 쓰지 않으므로 잠금을 잡지 않습니다.
    let _lock = if dry_run {
        None
    } else {
        let lock_error =
            |e: stellar_ops::lock::LockError| crate::apply::apply_error(&ApplyError::Lock(e));
        let dir = default_lock_dir().map_err(lock_error)?;
        Some(SaveLock::try_acquire(&dir, opened.project_id(), opened.root()).map_err(lock_error)?)
    };
    let snapshot = if dry_run {
        opened
    } else {
        open_snapshot(root).map_err(|e| views::open_error(&e))?
    };
    let revision = snapshot.revision();
    let base = base.unwrap_or(revision);
    if base != revision {
        return Err(crate::apply::apply_error(
            &ApplyError::BaseRevisionMismatch {
                expected: base,
                found: revision,
            },
        ));
    }
    // `SetStyleDefaults`의 영속 결과는 `stellar.yaml` 한 줄입니다(문서 레코드는 그대로). 앱·`stellar apply`가
    // generation 저장 뒤에 쓰는 것과 같은 두 단계 쓰기입니다(ADR-0087 결정 2). 준비 단계가 세트 평탄화와 연 시점
    // 선택을 확인하고, commit은 읽은 bytes hash가 그대로일 때만 씁니다.
    let prepared = prepare_selection(snapshot.root(), Some(snapshot.style_defaults_name()), name)
        .map_err(file_error)?;
    let previous = prepared.previous().to_string();
    let changed = previous != *name;
    if !dry_run {
        prepared.commit(snapshot.root()).map_err(file_error)?;
    }
    Ok(success(JsonValue::object(vec![
        (
            "mode",
            JsonValue::string(if dry_run { "dry-run" } else { "write" }),
        ),
        ("previous", JsonValue::string(previous)),
        ("active", JsonValue::string(name.clone())),
        ("changed", JsonValue::Bool(changed)),
        ("base_revision", views::revision(base)),
        ("revision", views::revision(revision)),
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
    let layout = node_layout(&snapshot, document, node).map_err(|e| views::query_error(&e))?;
    let layout_json = match &layout {
        Ok(_) => views::node_layout(&layout),
        Err(LayoutUnavailable::NoBox) => {
            return Err(CliError::new(
                ExitStatus::DataErr,
                "no-box",
                "Fragment는 상자가 없어 layout longhand가 없습니다",
            ))
        }
        Err(error @ LayoutUnavailable::StyleDefaults { code }) => {
            return Err(CliError::new(
                ExitStatus::DataErr,
                code,
                format!(
                    "{}. 자세한 진단은 stellar style-defaults show가 냅니다",
                    error.message()
                ),
            ))
        }
    };
    let JsonValue::Object(fields) = layout_json else {
        unreachable!("node_layout 결과는 object");
    };
    let mut out = vec![
        ("revision".to_string(), views::revision(snapshot.revision())),
        ("document".to_string(), views::id(document)),
        ("node".to_string(), views::id(node)),
    ];
    out.extend(fields);
    Ok(success(JsonValue::Object(out)))
}
