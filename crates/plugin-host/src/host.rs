//! Host API dispatcher (XXI.4.2, ADR-0097 결정 5·6·9·10).
//!
//! method 하나를 실행하는 단일 진입점 [`Dispatcher::call`]입니다. 순서는 항상 같습니다.
//!
//! 1. wire 이름을 생성 표([`Method::from_name`])로 찾습니다. 없으면 `unknown-method`.
//! 2. 생성 매개변수 표([`Method::params`])로 매개변수를 검사합니다(모르는 key, 필수 누락, 형식).
//! 3. 필요 권한이 유효 권한(manifest 선언 ∩ 승인)에 있는지 봅니다. 없으면 `permission-denied`.
//! 4. 프로젝트를 다루는 namespace는 신뢰를 다시 확인합니다.
//! 5. namespace별 handler가 기존 ops 공개 함수만 부르고 결과를 IDL record JSON으로 바꿉니다.
//!
//! 오류 문구는 고정 문구이고 프로젝트·플러그인 원문 값을 넣지 않습니다.

use std::collections::BTreeSet;
use std::path::PathBuf;

use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_ops::apply::{apply, ApplyError, ApplyMode, ApplyRequest, MAX_COMMANDS};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::diff::{DiffEntry, DiffPosition};
use stellar_ops::git::{
    change_id_text, system_git_runner, GitError, GitSession, HeadState, StatusArea, StatusRequest,
};
use stellar_ops::lock::LockPolicy;
use stellar_ops::mcp_command_input::{command_input, CommandFields};
use stellar_ops::query::{
    self, property_path_label, DefinitionSet, DocumentOutline, OpsValue, PageOutline, QueryError,
    TreeCursor, ViewOutline, DEFAULT_PAGE_LIMIT,
};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::trusted::{TrustedProject, TrustedReadError};

use crate::admission::AdmittedPlugin;
use crate::generated::host_api::{Method, Namespace};
use crate::idl::{ParamSpec, ParamType};
use crate::json::{JsonNumber, JsonValue as J};
use crate::permission::Permission;
use crate::plugin_data::{PluginData, PluginDataError};
use crate::rpc::RpcError;

/// 한 응답에 싣는 diff 항목 상한(넘으면 `truncated`).
pub const MAX_CHANGE_ENTRIES: usize = 500;

/// network 요청을 실제로 보내는 port. Host(앱)가 제공합니다. 없으면 `network.fetch`는 `unavailable`입니다.
pub trait NetworkPort: Send {
    /// https GET. (HTTP status, UTF-8 본문)을 돌려줍니다.
    fn get(&self, url: &str) -> Result<(u16, String), String>;
}

/// dispatcher 설정.
pub struct HostConfig {
    /// `command.apply` 쓰기 모드의 저장 잠금(`stellar apply`와 같은 정책).
    pub lock: LockPolicy,
    /// network port.
    pub network: Option<Box<dyn NetworkPort>>,
}

impl std::fmt::Debug for HostConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostConfig")
            .field("lock", &self.lock)
            .field("network", &self.network.is_some())
            .finish()
    }
}

/// 한 플러그인·한 프로젝트의 Host API 실행기.
#[derive(Debug)]
pub struct Dispatcher {
    project: TrustedProject,
    plugin: AdmittedPlugin,
    permissions: BTreeSet<Permission>,
    config: HostConfig,
}

fn count(n: usize) -> J {
    J::Number(JsonNumber::from_u64(n as u64))
}

fn id(value: StableId) -> J {
    J::string(value.to_string())
}

fn opt_id(value: Option<StableId>) -> J {
    value.map_or(J::Null, id)
}

fn rev(value: ProjectRevision) -> J {
    J::string(value.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// IDL `PropertyValue`.
fn property_value(value: &OpsValue) -> J {
    let inner = match value {
        OpsValue::Bool(v) => J::Bool(*v),
        OpsValue::Uint(v) => J::Number(JsonNumber::from_u64(*v)),
        OpsValue::Text(v) => J::string(v.clone()),
        OpsValue::Id(v) => id(*v),
        OpsValue::Ids(v) => J::Array(v.iter().map(|x| id(*x)).collect()),
        // f32 표기는 Rust 최단 왕복 표기를 문자열로 둡니다(JSON 숫자 원문 규칙과 무관하게 손실 없음).
        OpsValue::Floats(v) => J::Array(v.iter().map(|x| J::string(x.to_string())).collect()),
        OpsValue::Typed { type_tag, bytes } => J::object(vec![
            ("type_tag", J::int(i64::from(*type_tag))),
            ("bytes", J::string(hex(bytes))),
        ]),
        OpsValue::Bytes(v) => J::string(hex(v)),
    };
    J::object(vec![
        ("type", J::string(value.type_name())),
        ("value", inner),
    ])
}

fn position(value: Option<&DiffPosition>) -> J {
    value.map_or(J::Null, |p| {
        J::object(vec![
            ("parent", opt_id(p.parent)),
            ("order_key_hex", J::opt_string(p.order_key_hex.as_deref())),
        ])
    })
}

fn diff_entry(entry: &DiffEntry) -> J {
    J::object(vec![
        ("kind", J::string(entry.kind)),
        ("document", opt_id(entry.document)),
        ("component", opt_id(entry.component)),
        ("object", id(entry.object)),
        ("object_kind", J::opt_string(entry.object_kind.as_deref())),
        ("path", J::opt_string(entry.path.as_deref())),
        ("key", J::opt_string(entry.key.as_deref())),
        ("old", entry.old.as_ref().map_or(J::Null, property_value)),
        ("new", entry.new.as_ref().map_or(J::Null, property_value)),
        ("from", position(entry.from.as_ref())),
        ("to", position(entry.to.as_ref())),
    ])
}

fn document_outline(document: &DocumentOutline) -> J {
    let view = |v: &ViewOutline| {
        J::object(vec![
            ("id", id(v.id)),
            ("display_name", J::string(v.display_name.clone())),
            ("root_frame", id(v.root_frame)),
        ])
    };
    let page = |p: &PageOutline| {
        J::object(vec![
            ("id", id(p.id)),
            ("display_name", J::string(p.display_name.clone())),
            ("views", J::Array(p.views.iter().map(view).collect())),
        ])
    };
    J::object(vec![
        ("id", id(document.id)),
        ("display_name", J::string(document.display_name.clone())),
        ("pages", J::Array(document.pages.iter().map(page).collect())),
    ])
}

fn definition_set(set: &DefinitionSet) -> J {
    J::object(vec![
        ("revision", rev(set.revision)),
        ("chunk_kind", J::string(set.chunk_kind)),
        ("chunk_count", count(set.chunk_count)),
    ])
}

/// 검사한 매개변수 접근자.
struct Args<'a>(&'a J);

impl Args<'_> {
    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(J::as_str)
    }
    fn id(&self, key: &str) -> StableId {
        self.text(key)
            .and_then(|t| t.parse().ok())
            .expect("매개변수 검사 통과")
    }
    fn u32(&self, key: &str) -> Option<u32> {
        self.0
            .get(key)
            .and_then(J::as_i64)
            .and_then(|v| u32::try_from(v).ok())
    }
    fn bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(J::as_bool)
    }
}

/// command object JSON을 형식 중립 접근자로 씁니다(`ops::mcp_command_input::command_input` 입력).
struct Fields<'a>(&'a J);

impl CommandFields for Fields<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        self.0
            .as_object()
            .map(|pairs| pairs.iter().map(|(k, _)| k.as_str()).collect())
    }
    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(J::as_str)
    }
    fn boolean(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(J::as_bool)
    }
    fn number_text(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(J::Number(number)) => Some(number.as_str()),
            _ => None,
        }
    }
    fn integer(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(J::as_i64)
    }
    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }
}

/// 생성 매개변수 표로 매개변수를 검사합니다. 없는 params는 빈 object로 봅니다.
pub fn check_params(specs: &[ParamSpec], params: &J) -> Result<(), RpcError> {
    let invalid = |reason: &'static str| RpcError::new("invalid-params", reason);
    let pairs = params
        .as_object()
        .ok_or_else(|| invalid("params는 object여야 합니다"))?;
    for (key, _) in pairs {
        if !specs.iter().any(|s| s.name == key) {
            return Err(invalid("정의되지 않은 매개변수가 있습니다"));
        }
    }
    for spec in specs {
        let Some(value) = params.get(spec.name) else {
            if spec.required {
                return Err(invalid("필수 매개변수가 없습니다"));
            }
            continue;
        };
        let ok = match spec.ty {
            ParamType::String => value.as_str().is_some(),
            ParamType::Bool => value.as_bool().is_some(),
            ParamType::U32 => value.as_i64().is_some_and(|v| u32::try_from(v).is_ok()),
            ParamType::Id => value
                .as_str()
                .is_some_and(|t| t.parse::<StableId>().is_ok()),
            ParamType::Revision => value
                .as_str()
                .is_some_and(|t| t.parse::<ProjectRevision>().is_ok()),
            ParamType::JsonList => value
                .as_array()
                .is_some_and(|items| items.iter().all(|i| i.as_object().is_some())),
        };
        if !ok {
            return Err(invalid("매개변수 형식이 틀렸습니다"));
        }
    }
    Ok(())
}

fn trust_failure(error: TrustedReadError) -> RpcError {
    match error {
        TrustedReadError::NotTrusted => RpcError::new("not-trusted", "신뢰한 프로젝트가 아닙니다"),
        TrustedReadError::Unavailable => {
            RpcError::new("unavailable", "프로젝트를 읽을 수 없습니다")
        }
    }
}

fn query_failure(error: &QueryError) -> RpcError {
    match error {
        QueryError::Document(_) | QueryError::UnknownNode { .. } => {
            RpcError::new("not-found", "문서나 node를 찾을 수 없습니다")
        }
        QueryError::InvalidLimit { .. }
        | QueryError::InvalidCursor
        | QueryError::CursorDocumentMismatch { .. }
        | QueryError::CursorOutOfRange { .. } => {
            RpcError::new("invalid-params", "cursor나 limit이 틀렸습니다")
        }
        QueryError::StaleCursor { .. } => RpcError::new(
            "stale",
            "cursor를 만든 뒤 프로젝트가 바뀌었습니다. 처음부터 다시 읽으세요",
        ),
        QueryError::Projection(_) => RpcError::new("internal", "속성을 투영하지 못했습니다"),
    }
}

fn apply_failure(error: &ApplyError) -> RpcError {
    match error {
        ApplyError::BaseRevisionMismatch { .. } | ApplyError::ConcurrentWrite => RpcError::new(
            "stale",
            "base revision이 현재 저장 revision과 다릅니다. 다시 읽으세요",
        ),
        ApplyError::EmptyTransaction
        | ApplyError::TooManyCommands { .. }
        | ApplyError::InvalidCommand { .. }
        | ApplyError::UnknownCommand { .. } => {
            RpcError::new("invalid-params", "command 형식이 틀렸습니다")
        }
        ApplyError::ComponentSourceChanged { .. } => {
            RpcError::new("stale", "컴포넌트 원본이 없거나 source_hash가 다릅니다")
        }
        ApplyError::Rejected { .. } | ApplyError::InvalidResult(_) => RpcError::new(
            "rejected",
            "문서 규칙이 command 묶음을 거부했습니다(아무것도 적용하지 않음)",
        ),
        ApplyError::Document(_) => RpcError::new("not-found", "대상 문서를 읽을 수 없습니다"),
        ApplyError::Lock(_) => RpcError::new(
            "busy",
            "다른 writer가 저장 중입니다. 잠시 뒤 다시 시도하세요",
        ),
        ApplyError::Open(_) | ApplyError::ComponentSource(_) => {
            RpcError::new("unavailable", "프로젝트를 읽을 수 없습니다")
        }
        ApplyError::Projection(_) | ApplyError::Save(_) => {
            RpcError::new("internal", "적용을 완료하지 못했습니다")
        }
    }
}

fn git_failure(error: &GitError) -> RpcError {
    match error {
        GitError::GitUnavailable => RpcError::new("unavailable", "시스템 Git을 찾지 못했습니다"),
        GitError::NotTrusted => RpcError::new("not-trusted", "신뢰한 프로젝트가 아닙니다"),
        GitError::StaleCursor | GitError::InvalidLimit(_) => {
            RpcError::new("invalid-params", "cursor나 limit이 틀렸습니다")
        }
        GitError::NotWorktreeRoot | GitError::Location(_) => {
            RpcError::new("unavailable", "프로젝트 루트의 Git 저장소를 쓸 수 없습니다")
        }
        _ => RpcError::new("internal", "Git 상태를 읽지 못했습니다"),
    }
}

fn data_failure(error: &PluginDataError) -> RpcError {
    match error {
        PluginDataError::Path(_) => {
            RpcError::new("invalid-params", "파일 경로 규칙에 맞지 않습니다")
        }
        PluginDataError::NotFound => RpcError::new("not-found", "파일이 없습니다"),
        PluginDataError::TooLarge => RpcError::new("too-large", "파일 크기 상한을 넘습니다"),
        PluginDataError::NotText => RpcError::new("invalid-params", "UTF-8 텍스트 파일이 아닙니다"),
        PluginDataError::Symlink => {
            RpcError::new("permission-denied", "symlink를 따라가지 않습니다")
        }
        PluginDataError::Io(_) => RpcError::new("internal", "파일을 읽거나 쓰지 못했습니다"),
    }
}

fn head(head: &HeadState) -> J {
    let lossy = |bytes: &[u8]| J::string(String::from_utf8_lossy(bytes).into_owned());
    match head {
        HeadState::Branch { name, commit } => J::object(vec![
            ("state", J::string("branch")),
            ("branch", lossy(name)),
            ("commit", J::string(commit.to_string())),
        ]),
        HeadState::Detached { commit } => J::object(vec![
            ("state", J::string("detached")),
            ("branch", J::Null),
            ("commit", J::string(commit.to_string())),
        ]),
        HeadState::Unborn { branch } => J::object(vec![
            ("state", J::string("unborn")),
            ("branch", lossy(branch)),
            ("commit", J::Null),
        ]),
    }
}

impl Dispatcher {
    /// dispatcher를 만듭니다. 유효 권한은 `granted` 중 manifest가 선언한 것만입니다.
    pub fn new(
        project: TrustedProject,
        plugin: AdmittedPlugin,
        granted: &BTreeSet<Permission>,
        config: HostConfig,
    ) -> Dispatcher {
        let permissions = granted
            .iter()
            .copied()
            .filter(|p| plugin.manifest().declares(*p))
            .collect();
        Dispatcher {
            project,
            plugin,
            permissions,
            config,
        }
    }

    /// 유효 권한(이름 순).
    pub fn permissions(&self) -> &BTreeSet<Permission> {
        &self.permissions
    }

    /// 적재된 플러그인.
    pub fn plugin(&self) -> &AdmittedPlugin {
        &self.plugin
    }

    /// 유효 권한을 바꿉니다(승인·철회 반영). 선언하지 않은 권한은 무시합니다.
    pub fn set_permissions(&mut self, granted: &BTreeSet<Permission>) {
        self.permissions = granted
            .iter()
            .copied()
            .filter(|p| self.plugin.manifest().declares(*p))
            .collect();
    }

    /// method 하나를 실행합니다(모듈 문서 순서).
    pub fn call(&self, method: &str, params: &J) -> Result<J, RpcError> {
        let method = Method::from_name(method)
            .ok_or_else(|| RpcError::new("unknown-method", "정의되지 않은 method입니다"))?;
        check_params(method.params(), params)?;
        if let Some(permission) = method.permission() {
            if !self.permissions.contains(&permission) {
                return Err(RpcError::new(
                    "permission-denied",
                    "이 method에 필요한 권한을 승인받지 않았습니다",
                )
                .with_data(J::object(vec![(
                    "permission",
                    J::string(permission.name()),
                )])));
            }
        }
        if method.namespace() != Namespace::Host {
            self.project.check().map_err(trust_failure)?;
        }
        let args = Args(params);
        match method {
            Method::HostInfo => Ok(self.info()),
            Method::DocumentReadSnapshot => {
                let snapshot = self.project.snapshot().map_err(trust_failure)?;
                let structure = query::project_structure(&snapshot);
                Ok(J::object(vec![
                    ("revision", rev(structure.revision)),
                    (
                        "documents",
                        J::Array(structure.documents.iter().map(document_outline).collect()),
                    ),
                ]))
            }
            Method::QueryTreePage => {
                let snapshot = self.project.snapshot().map_err(trust_failure)?;
                let cursor = match args.text("cursor") {
                    None => None,
                    Some(text) => Some(text.parse::<TreeCursor>().map_err(|e| query_failure(&e))?),
                };
                let limit = args.u32("limit").map_or(DEFAULT_PAGE_LIMIT, |v| v as usize);
                let page = query::document_tree_page(
                    &snapshot,
                    args.id("document"),
                    cursor.as_ref(),
                    limit,
                )
                .map_err(|e| query_failure(&e))?;
                Ok(J::object(vec![
                    ("revision", rev(page.revision)),
                    ("document", id(page.document)),
                    ("total", count(page.total)),
                    ("offset", count(page.offset)),
                    (
                        "nodes",
                        J::Array(
                            page.nodes
                                .iter()
                                .map(|n| {
                                    J::object(vec![
                                        ("id", id(n.id)),
                                        ("parent", opt_id(n.parent)),
                                        ("view", id(n.view)),
                                        ("depth", count(n.depth as usize)),
                                        ("kind", J::string(n.kind)),
                                        ("display_name", J::string(n.display_name.clone())),
                                        ("visible", J::Bool(n.visible)),
                                        ("locked", J::Bool(n.locked)),
                                        ("child_count", count(n.child_count)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "next_cursor",
                        page.next_cursor
                            .map_or(J::Null, |c| J::string(c.to_string())),
                    ),
                ]))
            }
            Method::QueryNode => {
                let snapshot = self.project.snapshot().map_err(trust_failure)?;
                let detail = query::node_detail(&snapshot, args.id("document"), args.id("node"))
                    .map_err(|e| query_failure(&e))?;
                Ok(J::object(vec![
                    ("revision", rev(detail.revision)),
                    ("document", id(detail.document)),
                    ("id", id(detail.id)),
                    ("parent", opt_id(detail.parent)),
                    ("kind", J::string(detail.kind)),
                    ("display_name", J::string(detail.display_name.clone())),
                    ("visible", J::Bool(detail.visible)),
                    ("locked", J::Bool(detail.locked)),
                    (
                        "children",
                        J::Array(detail.children.iter().map(|c| id(*c)).collect()),
                    ),
                    (
                        "properties",
                        J::Array(
                            detail
                                .properties
                                .iter()
                                .map(|p| {
                                    J::object(vec![
                                        ("path", J::string(p.path.clone())),
                                        ("key", J::string(p.key.clone())),
                                        ("value", property_value(&p.value)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    ("modifier_count", count(detail.modifier_count)),
                ]))
            }
            Method::QueryTokens => {
                let snapshot = self.project.snapshot().map_err(trust_failure)?;
                Ok(definition_set(&query::tokens(&snapshot)))
            }
            Method::QueryComponents => {
                let snapshot = self.project.snapshot().map_err(trust_failure)?;
                Ok(definition_set(&query::components(&snapshot)))
            }
            Method::CommandApply => self.apply(&args),
            Method::GitStatus => self.git_status(&args),
            Method::FileRead => {
                let content = self.data()?.read(args.text("path").unwrap_or_default());
                let (path, text) = content.map_err(|e| data_failure(&e))?;
                Ok(J::object(vec![
                    ("path", J::string(path)),
                    ("text", J::string(text)),
                ]))
            }
            Method::FileWrite => {
                let written = self.data()?.write(
                    args.text("path").unwrap_or_default(),
                    args.text("text").unwrap_or_default(),
                );
                let (path, bytes) = written.map_err(|e| data_failure(&e))?;
                Ok(J::object(vec![
                    ("path", J::string(path)),
                    ("bytes", count(bytes)),
                ]))
            }
            Method::FileList => {
                let paths = self.data()?.list().map_err(|e| data_failure(&e))?;
                Ok(J::object(vec![(
                    "paths",
                    J::Array(paths.into_iter().map(J::string).collect()),
                )]))
            }
            Method::NetworkFetch => {
                let url = args.text("url").unwrap_or_default();
                if !url.starts_with("https://") || url.chars().any(char::is_control) {
                    return Err(RpcError::new(
                        "invalid-params",
                        "https URL만 보낼 수 있습니다",
                    ));
                }
                let port = self.config.network.as_ref().ok_or_else(|| {
                    RpcError::new("unavailable", "Host가 network port를 제공하지 않습니다")
                })?;
                let (status, text) = port
                    .get(url)
                    .map_err(|_| RpcError::new("unavailable", "network 요청에 실패했습니다"))?;
                Ok(J::object(vec![
                    ("status", count(usize::from(status))),
                    ("text", J::string(text)),
                ]))
            }
        }
    }

    fn info(&self) -> J {
        let manifest = self.plugin.manifest();
        J::object(vec![
            ("host_api_version", count(crate::HOST_API_VERSION as usize)),
            ("plugin_id", J::string(manifest.id.as_str())),
            ("plugin_version", J::string(manifest.version.to_string())),
            (
                "permissions",
                J::Array(
                    self.permissions
                        .iter()
                        .map(|p| J::string(p.name()))
                        .collect(),
                ),
            ),
            ("limits", crate::rpc::RpcLimits::default().to_json()),
        ])
    }

    fn root(&self) -> Result<PathBuf, RpcError> {
        Ok(self
            .project
            .snapshot()
            .map_err(trust_failure)?
            .root()
            .to_path_buf())
    }

    fn data(&self) -> Result<PluginData, RpcError> {
        Ok(PluginData::new(&self.root()?, &self.plugin.manifest().id))
    }

    fn apply(&self, args: &Args<'_>) -> Result<J, RpcError> {
        let base_revision = args
            .text("base_revision")
            .and_then(|t| t.parse().ok())
            .expect("매개변수 검사 통과");
        let items = args.0.get("commands").and_then(J::as_array).unwrap_or(&[]);
        if items.len() > MAX_COMMANDS {
            return Err(RpcError::new(
                "invalid-params",
                "command 수 상한을 넘습니다",
            ));
        }
        let commands = items
            .iter()
            .enumerate()
            .map(|(index, item)| command_input(&Fields(item), &format!("commands[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                RpcError::new("invalid-params", "command 형식이 틀렸습니다")
                    .with_data(J::object(vec![("code", J::string(e.code))]))
            })?;
        let request = ApplyRequest {
            base_revision,
            document: args.id("document"),
            commands,
        };
        let mode = if args.bool("dry_run").unwrap_or(false) {
            ApplyMode::DryRun
        } else {
            ApplyMode::Write
        };
        let root = self.root()?;
        let outcome =
            apply(&root, &request, mode, &self.config.lock).map_err(|e| apply_failure(&e))?;
        let entries = &outcome.diff.entries;
        let shown = entries.len().min(MAX_CHANGE_ENTRIES);
        Ok(J::object(vec![
            (
                "mode",
                J::string(match outcome.mode {
                    ApplyMode::DryRun => "dry-run",
                    ApplyMode::Write => "write",
                }),
            ),
            ("base_revision", rev(outcome.base_revision)),
            ("revision", rev(outcome.revision)),
            ("document", id(outcome.document)),
            ("command_count", count(outcome.command_count)),
            ("written_chunks", count(outcome.written_chunks)),
            ("change_count", count(entries.len())),
            (
                "changes",
                J::Array(entries[..shown].iter().map(diff_entry).collect()),
            ),
            ("truncated", J::Bool(shown < entries.len())),
        ]))
    }

    fn git_status(&self, args: &Args<'_>) -> Result<J, RpcError> {
        let root = self.root()?;
        let request = StatusRequest {
            cursor: args.text("cursor").map(str::to_owned),
            limit: args
                .u32("limit")
                .map_or(StatusRequest::default().limit, |v| v as usize),
        };
        // 신뢰는 call 앞에서 session의 TrustedProject로 확인했습니다. 같은 판정으로 gate를 엽니다.
        let report = system_git_runner()
            .and_then(|git| GitSession::open(GatedGitRunner::new(git, GitTrust::Trusted), &root))
            .and_then(|session| session.status(&request))
            .map_err(|e| git_failure(&e))?;
        let semantic = match &report.semantic {
            Ok(page) => J::object(vec![
                ("staged_total", count(page.staged_total)),
                ("unstaged_total", count(page.unstaged_total)),
                (
                    "changes",
                    J::Array(
                        page.entries
                            .iter()
                            .map(|entry| {
                                J::object(vec![
                                    ("id", J::string(change_id_text(&entry.id))),
                                    (
                                        "area",
                                        J::string(match entry.area {
                                            StatusArea::Staged => "staged",
                                            StatusArea::Unstaged => "unstaged",
                                        }),
                                    ),
                                    ("kind", J::string(entry.kind.name())),
                                    ("object", id(entry.object)),
                                    ("document", opt_id(entry.document)),
                                    (
                                        "property",
                                        entry
                                            .property
                                            .as_ref()
                                            .map_or(J::Null, |p| J::string(property_path_label(p))),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("next_cursor", J::opt_string(page.next_cursor.as_deref())),
            ]),
            Err(_) => J::Null,
        };
        Ok(J::object(vec![
            ("git_revision", J::string(report.revision.to_string())),
            ("head", head(&report.repository.head)),
            ("semantic", semantic),
        ]))
    }
}
