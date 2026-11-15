//! 명령 구현 (XXIV.1.2, XXIV.4.3).
//!
//! 각 명령은 [`Success`] 또는 [`CliError`]를 반환하고, 출력 형식은 [`crate::output`]이
//! 결정합니다. 명령은 stdout·stderr에 직접 쓰지 않습니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::query::{
    document_tree_page, node_detail, project_structure, tokens, TreeCursor, DEFAULT_PAGE_LIMIT,
    MAX_PAGE_LIMIT,
};
use stellar_ops::snapshot::{open_snapshot, probe as ops_probe};
use stellar_ops::types::{GitLink, Manifest};

use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

/// `version`: 도구 이름·버전과 출력 스키마 버전.
pub fn version(args: &[String]) -> Result<Success, CliError> {
    if let Some(extra) = args.first() {
        return Err(CliError::usage(format!(
            "version은 인자를 받지 않습니다: {extra}"
        )));
    }
    Ok(Success {
        result: JsonValue::object(vec![
            ("name", JsonValue::string(env!("CARGO_PKG_NAME"))),
            ("version", JsonValue::string(env!("CARGO_PKG_VERSION"))),
            (
                "output_schema_version",
                JsonValue::int(crate::output::SCHEMA_VERSION),
            ),
        ]),
        diagnostics: Vec::new(),
    })
}

/// inspect·query가 받는 옵션 하나(`--name`)인지.
fn is_option(arg: &str) -> bool {
    arg.starts_with('-') && arg != "-"
}

/// `inspect <경로> [--structure]`: 프로젝트 root와 `stellar.yaml`을 읽기 전용으로 검사합니다.
///
/// 옵션이 없으면 XXIV.1.2 결과(문서 청크를 읽지 않는 IV.5.1 probe)와 byte 단위로 같습니다.
/// `--structure`(XXIV.4.3)는 같은 결과 뒤에 `revision`·`documents`·`chunks` key를 덧붙입니다. 이때는
/// current generation을 열어 문서 정의를 읽습니다. 어느 경우든 파일을 만들거나 고치지 않고, 결과
/// JSON에는 경로와 의존성 repository URL을 넣지 않습니다.
pub fn inspect(args: &[String]) -> Result<Success, CliError> {
    let mut path = None;
    let mut structure = false;
    for arg in args {
        if arg == "--structure" {
            if structure {
                return Err(CliError::usage("--structure를 두 번 지정했습니다"));
            }
            structure = true;
        } else if is_option(arg) {
            return Err(CliError::usage(format!("알 수 없는 옵션: {arg}")));
        } else if path.is_some() {
            return Err(CliError::usage(format!(
                "inspect는 경로 하나만 받습니다: {arg}"
            )));
        } else {
            path = Some(Path::new(arg.as_str()));
        }
    }
    let Some(path) = path else {
        return Err(CliError::usage("inspect에는 프로젝트 경로가 필요합니다"));
    };
    let context = path.display().to_string();
    if !structure {
        let probe = ops_probe(path).map_err(|e| views::open_error(&e).with_context(context))?;
        return Ok(Success {
            result: JsonValue::object(probe_fields(probe.git_link, &probe.manifest)),
            diagnostics: Vec::new(),
        });
    }
    let snapshot = open_snapshot(path).map_err(|e| views::open_error(&e).with_context(context))?;
    let mut fields = probe_fields(snapshot.git_link(), snapshot.manifest());
    fields.extend(views::structure_fields(&project_structure(&snapshot)));
    Ok(Success {
        result: JsonValue::object(fields),
        diagnostics: Vec::new(),
    })
}

fn probe_fields(git_link: GitLink, manifest: &Manifest) -> Vec<(&'static str, JsonValue)> {
    let git_link = match git_link {
        GitLink::Directory => "directory",
        GitLink::GitFile => "gitfile",
    };
    vec![
        (
            "manifest_schema_version",
            JsonValue::int(i64::from(manifest.schema_version)),
        ),
        (
            "project",
            JsonValue::object(vec![
                ("id", JsonValue::string(manifest.project.id.clone())),
                ("name", JsonValue::string(manifest.project.name.clone())),
            ]),
        ),
        ("git_link", JsonValue::string(git_link)),
        (
            "target_profiles",
            JsonValue::Array(
                manifest
                    .target_profiles
                    .iter()
                    .map(|p| {
                        JsonValue::object(vec![
                            ("id", JsonValue::string(p.id.clone())),
                            ("platform", JsonValue::string(p.platform.clone())),
                            ("minimum_os", JsonValue::opt_string(p.minimum_os.as_deref())),
                            ("baseline", JsonValue::opt_string(p.baseline.as_deref())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "dependencies",
            JsonValue::Array(
                manifest
                    .dependencies
                    .iter()
                    .map(|d| {
                        JsonValue::object(vec![
                            ("name", JsonValue::string(d.name.as_str())),
                            ("source", JsonValue::string(d.source.as_str())),
                            ("version", JsonValue::string(d.version.value())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]
}

fn parse_id(text: &str, what: &str) -> Result<StableId, CliError> {
    text.parse::<StableId>()
        .map_err(|_| CliError::usage(format!("{what} id 형식이 잘못되었습니다: {text}")))
}

/// 인자 검사를 마친 query 계획. 사용법 오류에서는 프로젝트를 열지 않습니다.
enum QueryPlan {
    Project,
    Tree {
        document: StableId,
        cursor: Option<TreeCursor>,
        limit: usize,
    },
    Node {
        document: StableId,
        node: StableId,
    },
    Tokens,
    Components,
    Component(StableId),
}

fn no_extra(target: &str, rest: &[String]) -> Result<(), CliError> {
    match rest.first() {
        Some(extra) => Err(CliError::usage(format!(
            "query {target}는 추가 인자를 받지 않습니다: {extra}"
        ))),
        None => Ok(()),
    }
}

fn tree_plan(rest: &[String]) -> Result<QueryPlan, CliError> {
    let mut document = None;
    let mut cursor_text: Option<String> = None;
    let mut limit = DEFAULT_PAGE_LIMIT;
    let mut limit_set = false;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--cursor" if cursor_text.is_none() => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::usage("--cursor에 값이 필요합니다"))?;
                cursor_text = Some(value.clone());
            }
            "--limit" if !limit_set => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::usage("--limit에 값이 필요합니다"))?;
                limit = value
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=MAX_PAGE_LIMIT).contains(n))
                    .ok_or_else(|| {
                        CliError::usage(format!(
                            "--limit은 1..={MAX_PAGE_LIMIT} 정수여야 합니다: {value}"
                        ))
                    })?;
                limit_set = true;
            }
            other if is_option(other) => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )))
            }
            other if document.is_none() => document = Some(parse_id(other, "문서")?),
            other => {
                return Err(CliError::usage(format!(
                    "query tree는 문서 id 하나만 받습니다: {other}"
                )))
            }
        }
    }
    let document =
        document.ok_or_else(|| CliError::usage("query tree에는 <문서 id>가 필요합니다"))?;
    let cursor = match cursor_text {
        None => None,
        Some(text) => Some(
            text.parse::<TreeCursor>()
                .map_err(|e| views::query_error(&e))?,
        ),
    };
    Ok(QueryPlan::Tree {
        document,
        cursor,
        limit,
    })
}

fn query_plan(target: &str, rest: &[String]) -> Result<QueryPlan, CliError> {
    match target {
        "project" => no_extra(target, rest).map(|()| QueryPlan::Project),
        "tokens" => no_extra(target, rest).map(|()| QueryPlan::Tokens),
        "components" => no_extra(target, rest).map(|()| QueryPlan::Components),
        "component" => match rest {
            [component] => Ok(QueryPlan::Component(parse_id(component, "컴포넌트")?)),
            _ => Err(CliError::usage(
                "query component에는 <컴포넌트 id>가 필요합니다",
            )),
        },
        "node" => match rest {
            [document, node] if !is_option(document) && !is_option(node) => Ok(QueryPlan::Node {
                document: parse_id(document, "문서")?,
                node: parse_id(node, "node")?,
            }),
            _ => Err(CliError::usage(
                "query node에는 <문서 id> <node id>가 필요합니다",
            )),
        },
        "tree" => tree_plan(rest),
        other => Err(CliError::usage(format!("알 수 없는 query 대상: {other}"))),
    }
}

/// `query <경로> <대상> [인자]`: 저장된 프로젝트를 읽기 전용으로 조회합니다(XXIV.4.3).
///
/// 대상은 [`crate::cli::QUERY_TARGETS`]입니다. 모든 결과에 조회한 revision
/// (`g<generation>:<content hash>`)이 들어갑니다. 파일을 만들거나 고치지 않습니다.
pub fn query(args: &[String]) -> Result<Success, CliError> {
    let (path, target, rest) = match args {
        [] => {
            return Err(CliError::usage(
                "query에는 프로젝트 경로와 대상이 필요합니다",
            ))
        }
        [p, ..] if is_option(p) => return Err(CliError::usage(format!("알 수 없는 옵션: {p}"))),
        [_] => return Err(CliError::usage("query에는 조회 대상이 필요합니다")),
        [p, t, rest @ ..] => (Path::new(p.as_str()), t.as_str(), rest),
    };
    let plan = query_plan(target, rest)?;
    let context = path.display().to_string();
    let snapshot =
        open_snapshot(path).map_err(|e| views::open_error(&e).with_context(context.clone()))?;
    let result = match plan {
        QueryPlan::Project => {
            let manifest = snapshot.manifest();
            let mut fields = vec![(
                "project",
                JsonValue::object(vec![
                    ("id", JsonValue::string(manifest.project.id.clone())),
                    ("name", JsonValue::string(manifest.project.name.clone())),
                ]),
            )];
            fields.extend(views::structure_fields(&project_structure(&snapshot)));
            JsonValue::object(fields)
        }
        QueryPlan::Tree {
            document,
            cursor,
            limit,
        } => {
            let page = document_tree_page(&snapshot, document, cursor.as_ref(), limit)
                .map_err(|e| views::query_error(&e).with_context(context.clone()))?;
            views::tree_page(&page, limit)
        }
        QueryPlan::Node { document, node } => {
            let detail = node_detail(&snapshot, document, node)
                .map_err(|e| views::query_error(&e).with_context(context.clone()))?;
            views::node_detail(&detail)
        }
        QueryPlan::Tokens => views::definition_set(&tokens(&snapshot)),
        QueryPlan::Component(id) => {
            let detail = stellar_ops::component_source::detail(&snapshot, id)
                .map_err(|err| crate::text_source::operation_error(&err))?
                .ok_or_else(|| {
                    CliError::new(
                        crate::exit::ExitStatus::NoInput,
                        "unknown-component",
                        format!("프로젝트에 컴포넌트 {id}가 없습니다"),
                    )
                })?;
            JsonValue::object(vec![
                ("revision", views::revision(detail.revision)),
                ("component", views::id(id)),
                ("root", views::id(detail.root)),
                ("name", JsonValue::string(detail.name)),
                (
                    "source_hash",
                    JsonValue::string(detail.source_hash.to_string()),
                ),
                (
                    "nodes",
                    JsonValue::Array(detail.nodes.iter().map(JsonValue::from).collect()),
                ),
            ])
        }
        QueryPlan::Components => {
            let set = stellar_ops::component_source::query(&snapshot)
                .map_err(|err| crate::text_source::operation_error(&err).with_context(context))?;
            JsonValue::object(vec![
                ("revision", views::revision(set.revision)),
                ("chunk_kind", JsonValue::string("component-library")),
                ("chunk_count", views::count(set.chunk_count)),
                (
                    "entries",
                    JsonValue::Array(
                        set.entries
                            .into_iter()
                            .map(|entry| crate::component_source::component_json(entry, false))
                            .collect(),
                    ),
                ),
            ])
        }
    };
    Ok(Success {
        result,
        diagnostics: Vec::new(),
    })
}
