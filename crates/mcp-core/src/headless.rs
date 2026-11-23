//! 고정한 신뢰 프로젝트의 읽기·검증·dry-run L2 backend (XIX.9.3, ADR-0005 §3, ADR-0085 결정 1).
//!
//! - 실행 인자로 고정한 절대 루트 하나만 다룹니다. tool 인자에 경로·프로젝트를 고르는 key가 없고, 선언되지
//!   않은 key는 거부합니다.
//! - 매 요청 진입과 응답 직전에 신뢰·identity·generation을 다시 확인합니다. 신뢰가 철회되면 모든 요청을
//!   거부합니다.
//! - 목록은 agent-protocol 정의에서 headless가 제공하는(`Availability::All`) 항목만 고릅니다. 쓰기 tool은
//!   목록에 없고, 이름으로 불러도 모르는 tool입니다.
//! - 프로젝트 파일을 만들거나 바꾸지 않습니다. ops의 dry-run·읽기 함수만 부릅니다.
mod comparison;
mod components;
mod context;
mod integrity;
mod operations;
mod resources_extra;
mod validation;
use std::cell::RefCell;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::query::{project_structure, DocumentOutline, DocumentTree, ProjectStructure};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::snapshot::ProjectSnapshot;
use stellar_ops::trusted::{TrustedProject, TrustedReadError};

use crate::catalog::paging::{self, PageSource};
use crate::catalog::resources::{self, Backend};
use crate::catalog::scope::{ApprovedScope, ProjectDocuments, ScopeError, ScopedPayload};
use crate::catalog::tools::{self, EnabledNamespaces, SEMANTIC_DIFF, VALIDATE_DOCUMENT};
use crate::failure::{fail, ResultKind as Kind, WithDetail};
use crate::json::{self, JsonNumber, JsonValue};
use crate::jsonrpc::{codes, RpcError};

/// 시작 시 선택한 프로젝트와 이 stdio 연결의 승인 범위.
#[derive(Debug)]
pub struct Headless {
    project: TrustedProject,
    scope: ApprovedScope,
    session: String,
    cache: RefCell<Option<CachedRead>>,
    #[cfg(test)]
    loads: std::cell::Cell<(usize, usize)>,
}

/// 연결별 snapshot, 최근 문서 트리, 최근 검증 진단과 HEAD 비교 결과를 각 하나씩 보관합니다.
#[derive(Debug)]
struct CachedRead {
    snapshot: ProjectSnapshot,
    tree: Option<(StableId, DocumentTree)>,
    validation: Option<validation::FindingsPage>,
    diagnostics: Option<integrity::DiagnosticsPage>,
    comparison: Option<comparison::DiffPage>,
    components: Option<stellar_ops::component_source::ProjectComponents>,
    component: Option<stellar_ops::component_source::ComponentDetail>,
}

impl Headless {
    /// 실행 인자로 받은 절대 프로젝트 루트 하나로 고정합니다(ADR-0005 §3). 절대 경로가 아니면
    /// 사용법 오류, 신뢰 등록으로 확인할 수 없으면 권한 오류입니다. 승인·신뢰 상태를 쓰지 않습니다.
    pub fn select(root: &std::path::Path) -> Result<Self, crate::exit::Exit> {
        if !root.is_absolute() {
            return Err(crate::exit::USAGE);
        }
        let project =
            TrustedProject::select(root).map_err(|_| crate::exit::for_kind(Kind::NotTrusted))?;
        Ok(Self::new(project))
    }

    fn new(project: TrustedProject) -> Self {
        let session = format!("headless-{}", std::process::id());
        Self {
            scope: ApprovedScope::new(project.project_id().to_string(), session.clone()),
            project,
            session,
            cache: RefCell::new(None),
            #[cfg(test)]
            loads: std::cell::Cell::new((0, 0)),
        }
    }

    /// 요청과 응답 시점의 기존 신뢰·identity 확인.
    pub fn check(&self) -> Result<(), RpcError> {
        self.project.check().map_err(|error| {
            self.cache.borrow_mut().take();
            read_error(error)
        })
    }

    /// agent-protocol 정의 중 headless가 제공하는 resource·template·tool을 노출합니다.
    pub fn list(&self, method: &str, params: Option<&JsonValue>) -> Result<JsonValue, RpcError> {
        self.check()?;
        check_fields(params, &[])?;
        match method {
            "tools/list" => Ok(tools::tools_list(
                Backend::Headless,
                EnabledNamespaces::default(),
            )),
            "resources/list" => Ok(resources::resources_list(Backend::Headless)),
            "resources/templates/list" => Ok(resources::templates_list(Backend::Headless)),
            _ => Err(malformed("제공하지 않는 목록입니다")),
        }
    }

    /// 신뢰 snapshot에서 읽기·검증·dry-run tool 하나를 실행합니다. 실패는 결과 종류를 가진 [`RpcError`]이고,
    /// 서버가 실행 실패를 `isError` 결과로, 모르는 tool·params 형식 오류를 JSON-RPC 오류로 보냅니다(XIX.9.9).
    pub fn call(&self, params: Option<&JsonValue>) -> Result<JsonValue, RpcError> {
        self.with_snapshot(|cached| {
            check_fields(params, &["name", "arguments"])?;
            let params = params.ok_or_else(|| malformed("tool 이름이 필요합니다"))?;
            let tool = params
                .get("name")
                .and_then(JsonValue::as_str)
                .and_then(tools::tool)
                .filter(|tool| {
                    tool.availability.includes(Backend::Headless)
                        && EnabledNamespaces::default().shows(tool)
                })
                .ok_or_else(|| fail(Kind::UnknownTool, "제공하지 않는 tool입니다"))?;
            let empty = JsonValue::Object(Vec::new());
            let args = params.get("arguments").unwrap_or(&empty);
            self.scope
                .check_tool_arguments(tool, args, &Documents(&cached.snapshot))
                .map_err(|error| error.failure())?;
            let request = page_request(args)?;
            let body = match tool.name {
                n if n == SEMANTIC_DIFF.name => {
                    comparison::page(&self.project, cached, args, &request)?
                }
                n if n == VALIDATE_DOCUMENT.name => validation::page(cached, args, &request)?,
                n if n == tools::DIAGNOSTICS.name => integrity::page(cached, args, &request)?,
                n if n == tools::PROPOSE_COMMANDS.name => {
                    operations::propose_commands(cached, args)?
                }
                n if n == tools::SOURCE_SPAN.name => operations::node_source_span(cached, args)?,
                n if n == tools::EXPORT_WEB_FRAME.name => {
                    operations::export_web_frame(cached, args)?
                }
                n if n == tools::EXPORT_PNG.name => operations::export_png(cached, args)?,
                n if n == tools::COPY_CSS.name => operations::copy_css(cached, args)?,
                n if n == tools::CONTEXT.name => context::read(cached, args)?,
                n if n == tools::PROPOSE_CSS_PASTE.name => {
                    operations::propose_css_paste(cached, args)?
                }
                n if n == tools::GIT_STATUS.name => operations::git_status(cached, &request)?,
                n if n == tools::SEMANTIC_CONFLICTS.name => {
                    operations::git_semantic_conflicts(cached)?
                }
                _ => return Err(fail(Kind::Internal, "headless 목록과 구현이 다릅니다")),
            };
            Ok(crate::result::tool_success(body))
        })
    }

    /// scope 확인 → ops snapshot → 기존 catalog 결과 → scope release.
    pub fn read(&self, params: Option<&JsonValue>) -> Result<JsonValue, RpcError> {
        self.with_snapshot(|cached| self.read_cached(params, cached))
    }

    /// 조회와 검증은 같은 세션의 snapshot과 진입·응답 시점의 신뢰 확인을 공유합니다.
    fn with_snapshot(
        &self,
        build: impl FnOnce(&mut CachedRead) -> Result<JsonValue, RpcError>,
    ) -> Result<JsonValue, RpcError> {
        let mut cache = self.cache.borrow_mut().take();
        if let Some(cached) = cache.as_ref() {
            if !self
                .project
                .snapshot_is_current(&cached.snapshot)
                .map_err(read_error)?
            {
                cache = None;
            }
        } else {
            self.check()?;
        }
        if cache.is_none() {
            #[cfg(test)]
            self.loads.set((self.loads.get().0 + 1, self.loads.get().1));
            cache = Some(CachedRead {
                snapshot: self.project.snapshot().map_err(read_error)?,
                tree: None,
                validation: None,
                diagnostics: None,
                comparison: None,
                components: None,
                component: None,
            });
        }
        let cached = cache.as_mut().expect("snapshot을 열었음");
        let result = build(cached)?;
        if json::write(&result).len() > crate::server::MAX_MESSAGE_BYTES - 1024 {
            return Err(fail(Kind::TooLarge, "응답 크기 상한을 넘습니다"));
        }
        if !self
            .project
            .snapshot_is_current(&cached.snapshot)
            .map_err(read_error)?
        {
            return Err(read_error(TrustedReadError::Unavailable));
        }
        let released = self
            .scope
            .release(ScopedPayload {
                project: cached.snapshot.project_id().to_string(),
                session: self.session.clone(),
                body: result,
            })
            .map_err(|e| e.failure())?;
        *self.cache.borrow_mut() = cache;
        Ok(released)
    }

    fn read_cached(
        &self,
        params: Option<&JsonValue>,
        cached: &mut CachedRead,
    ) -> Result<JsonValue, RpcError> {
        check_fields(params, &["uri"])?;
        let uri = params
            .and_then(|p| p.get("uri"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("resource URI가 필요합니다"))?;
        let snapshot = &cached.snapshot;
        let target = self
            .scope
            .check_resource_uri(uri, &Documents(snapshot))
            .map_err(|e| e.failure())?;
        let body = match target.name {
            "project-structure" => {
                JsonValue::object(structure_fields(&project_structure(snapshot)))
            }
            "components" => components::read(cached, uri)?,
            "tokens" => resources_extra::read_tokens(cached),
            "target-profiles" => resources_extra::read_target_profiles(cached),
            "git-status" => resources_extra::read_git_summary(cached)?,
            "component" => {
                let component = target
                    .component
                    .as_deref()
                    .and_then(|id| id.parse().ok())
                    .ok_or_else(|| invalid("컴포넌트 ID가 필요합니다"))?;
                components::read_detail(cached, uri, component)?
            }
            "document" => {
                let document = target
                    .document
                    .as_deref()
                    .and_then(|id| id.parse::<StableId>().ok())
                    .ok_or_else(|| invalid("승인된 문서 ID가 필요합니다"))?;
                let (_, request) = paging::parse_page_query(uri).map_err(|e| e.failure())?;
                paging::validate_request(
                    &document.to_string(),
                    &snapshot.revision().token(),
                    &request,
                )
                .map_err(|e| e.failure())?;
                if cached.tree.as_ref().is_none_or(|(id, _)| *id != document) {
                    cached.tree = None;
                    #[cfg(test)]
                    self.loads.set((self.loads.get().0, self.loads.get().1 + 1));
                    cached.tree = Some((
                        document,
                        DocumentTree::load(snapshot, document).map_err(|_| {
                            fail(Kind::Unavailable, "문서 snapshot을 읽을 수 없습니다")
                        })?,
                    ));
                }
                let source = TreeSource {
                    tree: &cached.tree.as_ref().expect("문서 tree를 열었음").1,
                    document,
                    revision: snapshot.revision(),
                };
                paging::build_page(&source, &request).map_err(|e| e.failure())?
            }
            _ => return Err(fail(Kind::InvalidArguments, "제공하지 않는 resource입니다")),
        };
        if json::write(&body).len() > paging::MAX_PAGE_BYTES {
            return Err(fail(Kind::TooLarge, "resource가 응답 크기 상한을 넘습니다"));
        }
        Ok(paging::read_result(uri, &body))
    }
}

fn invalid(message: impl Into<String>) -> RpcError {
    crate::failure::fail_text(Kind::InvalidArguments, message)
}

/// params 형식 오류(MCP protocol 오류, 결과 종류 없음).
fn malformed(message: &'static str) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, message)
}

/// page·scope 오류를 결과 종류로 나눕니다. 두 오류의 문구는 고정 형식이고 프로젝트 문자열이 없습니다.
trait Failure {
    fn failure(&self) -> RpcError;
}

impl Failure for paging::PageError {
    fn failure(&self) -> RpcError {
        let kind = match self {
            paging::PageError::StaleCursor => Kind::Stale,
            paging::PageError::ItemTooLarge { .. } => Kind::TooLarge,
            _ => Kind::InvalidArguments,
        };
        crate::failure::fail_text(kind, self.to_string())
    }
}

impl Failure for ScopeError {
    fn failure(&self) -> RpcError {
        let kind = match self {
            ScopeError::ForeignDocument
            | ScopeError::ForeignProject
            | ScopeError::ForeignSession => Kind::OutOfScope,
            _ => Kind::InvalidArguments,
        };
        crate::failure::fail_text(kind, self.to_string())
    }
}

/// `cursor`·`limit` 인자.
fn page_request(args: &JsonValue) -> Result<paging::PageRequest, RpcError> {
    Ok(paging::PageRequest {
        cursor: args
            .get("cursor")
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("cursor는 문자열이어야 합니다"))
            })
            .transpose()?,
        limit: args
            .get("limit")
            .map(|v| {
                v.as_i64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| invalid("limit은 양의 정수여야 합니다"))
            })
            .transpose()?,
    })
}

fn read_error(error: TrustedReadError) -> RpcError {
    match error {
        TrustedReadError::NotTrusted => {
            fail(Kind::NotTrusted, "프로젝트 읽기 신뢰를 확인할 수 없습니다")
        }
        TrustedReadError::Unavailable => {
            fail(Kind::Unavailable, "프로젝트 snapshot을 읽을 수 없습니다")
        }
    }
}

fn check_fields(params: Option<&JsonValue>, allowed: &[&str]) -> Result<(), RpcError> {
    if let Some(params) = params {
        let fields = params
            .as_object()
            .ok_or_else(|| malformed("params는 object여야 합니다"))?;
        if fields
            .iter()
            .any(|(key, _)| !allowed.contains(&key.as_str()) && key != "_meta")
        {
            return Err(malformed("선언되지 않은 요청 인자입니다"));
        }
    }
    Ok(())
}

struct Documents<'a>(&'a ProjectSnapshot);
impl ProjectDocuments for Documents<'_> {
    fn owns_document(&self, document: &str) -> bool {
        document
            .parse::<StableId>()
            .ok()
            .is_some_and(|id| self.0.documents().iter().any(|d| d.id == id))
    }
}

struct TreeSource<'a> {
    tree: &'a DocumentTree,
    document: StableId,
    revision: ProjectRevision,
}
impl PageSource for TreeSource<'_> {
    fn key(&self) -> String {
        self.document.to_string()
    }
    fn revision_token(&self) -> String {
        self.revision.token()
    }
    fn revision_json(&self) -> JsonValue {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.tree.total()
    }
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        vec![("document", id(self.document))]
    }
    fn item(&self, index: usize) -> JsonValue {
        let node = self
            .tree
            .node(index)
            .expect("PageSource index는 같은 tree의 total 미만");
        JsonValue::object(vec![
            ("id", id(node.id)),
            ("parent", opt_id(node.parent)),
            ("view", id(node.view)),
            ("depth", uint(u64::from(node.depth))),
            ("kind", JsonValue::string(node.kind)),
            ("name", JsonValue::string(&node.display_name)),
            ("visible", JsonValue::Bool(node.visible)),
            ("locked", JsonValue::Bool(node.locked)),
            ("child_count", count(node.child_count)),
        ])
    }
}

/// StableId 문자열.
fn id(value: StableId) -> JsonValue {
    JsonValue::string(value.to_string())
}

/// 선택 StableId(`None`은 `null`).
fn opt_id(value: Option<StableId>) -> JsonValue {
    value.map_or(JsonValue::Null, id)
}

/// 부호 없는 정수.
fn uint(value: u64) -> JsonValue {
    JsonValue::Number(JsonNumber::from_u64(value))
}

/// `usize` 개수.
fn count(value: usize) -> JsonValue {
    uint(u64::try_from(value).unwrap_or(u64::MAX))
}

/// revision object: `{"token", "generation", "manifest_hash"}`.
fn revision(value: ProjectRevision) -> JsonValue {
    JsonValue::object(vec![
        ("token", JsonValue::string(value.token())),
        ("generation", uint(value.generation())),
        (
            "manifest_hash",
            value
                .manifest_hash()
                .map_or(JsonValue::Null, |h| JsonValue::string(h.to_string())),
        ),
    ])
}

/// 문서 개요.
fn document_outline(outline: &DocumentOutline) -> JsonValue {
    JsonValue::object(vec![
        ("id", id(outline.id)),
        ("name", JsonValue::string(outline.display_name.clone())),
        (
            "pages",
            JsonValue::Array(
                outline
                    .pages
                    .iter()
                    .map(|page| {
                        JsonValue::object(vec![
                            ("id", id(page.id)),
                            ("name", JsonValue::string(page.display_name.clone())),
                            (
                                "views",
                                JsonValue::Array(
                                    page.views
                                        .iter()
                                        .map(|view| {
                                            JsonValue::object(vec![
                                                ("id", id(view.id)),
                                                (
                                                    "name",
                                                    JsonValue::string(view.display_name.clone()),
                                                ),
                                                ("root_frame", id(view.root_frame)),
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

/// 프로젝트 구조의 `documents`·`chunks` 항목.
fn structure_fields(structure: &ProjectStructure) -> Vec<(&'static str, JsonValue)> {
    vec![
        ("revision", revision(structure.revision)),
        (
            "documents",
            JsonValue::Array(structure.documents.iter().map(document_outline).collect()),
        ),
        (
            "chunks",
            JsonValue::Array(
                structure
                    .chunk_kinds
                    .iter()
                    .map(|(name, code, n)| {
                        JsonValue::object(vec![
                            ("kind", JsonValue::opt_string(*name)),
                            ("code", uint(u64::from(*code))),
                            ("count", count(*n)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_ops::test_support::{
        file_digest, large_document, project_id, random_valid_document, save_documents,
        scaffold_project, select_trusted_project, FileTrustStore, ProjectIdentity, TempDir,
        PROJECT_ID_TEXT,
    };

    struct Fixture {
        temp: TempDir,
        project: std::path::PathBuf,
        data: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new("read-cache");
            let project = temp.path.join("project");
            let data = temp.path.join("data");
            scaffold_project(&project);
            let fixture = Self {
                temp,
                project,
                data,
            };
            fixture.grant();
            fixture
        }

        fn identity(&self) -> ProjectIdentity {
            ProjectIdentity::new(project_id(), &self.project).unwrap()
        }

        fn grant(&self) {
            FileTrustStore::open(&self.data)
                .unwrap()
                .grant(&self.identity())
                .unwrap();
        }

        fn backend(&self) -> Headless {
            Headless::new(select_trusted_project(&self.project, self.data.clone()).unwrap())
        }
    }

    fn read(backend: &Headless, uri: &str) -> Result<JsonValue, RpcError> {
        let result = backend.read(Some(&JsonValue::object(vec![(
            "uri",
            JsonValue::string(uri),
        )])))?;
        let text = result.get("contents").unwrap().as_array().unwrap()[0]
            .get("text")
            .unwrap()
            .as_str()
            .unwrap();
        Ok(json::parse(text, json::JsonLimits::default()).unwrap())
    }

    #[test]
    fn one_snapshot_and_latest_tree_survive_only_successful_current_reads() {
        let fx = Fixture::new();
        let first = large_document(2101);
        let second = random_valid_document(91);
        assert!(second.node_count() > 2);
        save_documents(&fx.project, &[&first, &second]);
        let backend = fx.backend();
        let a = format!("stellar://documents/{}?limit=2", first.document_id());
        let b = format!("stellar://documents/{}?limit=2", second.document_id());
        let before = file_digest(&fx.temp.path);
        let page = read(&backend, &a).unwrap();
        let cursor = page.get("next_cursor").unwrap().as_str().unwrap();
        let next = format!(
            "stellar://documents/{}?cursor={cursor}&limit=2",
            first.document_id()
        );
        assert_eq!(
            read(&backend, &next).unwrap().get("offset"),
            Some(&JsonValue::int(2))
        );
        assert_eq!(read(&backend, &a).unwrap(), page);
        read(&backend, "stellar://project/structure").unwrap();
        backend
            .call(Some(&JsonValue::object(vec![
                ("name", JsonValue::string(VALIDATE_DOCUMENT.name)),
                (
                    "arguments",
                    JsonValue::object(vec![("document", id(second.document_id()))]),
                ),
            ])))
            .unwrap();
        assert_eq!(read(&backend, &a).unwrap(), page);
        assert_eq!(
            backend.loads.get(),
            (1, 1),
            "검증한 뒤에도 최근 문서 트리를 재사용"
        );

        read(&backend, &b).unwrap();
        read(&backend, &a).unwrap();
        assert_eq!(backend.loads.get(), (1, 3), "이전 문서는 보관하지 않음");
        assert_eq!(file_digest(&fx.temp.path), before);

        // generation이 그대로여도 같은 길이의 정상 프로젝트 설정 변경을 숨기지 않습니다.
        let manifest = fx.project.join("stellar.yaml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, original.replace("Fixture", "Changed")).unwrap();
        let after_settings = file_digest(&fx.temp.path);
        assert_eq!(read(&backend, &a).unwrap(), page);
        assert_eq!(backend.loads.get(), (2, 4));
        assert_eq!(
            backend
                .cache
                .borrow()
                .as_ref()
                .unwrap()
                .snapshot
                .manifest()
                .project
                .name,
            "Changed"
        );
        assert_eq!(file_digest(&fx.temp.path), after_settings);

        // 다른 generation cursor는 새 snapshot만 열고 큰 tree를 읽기 전에 거부합니다.
        let changed = large_document(2102);
        save_documents(&fx.project, &[&changed, &second]);
        let after_save = file_digest(&fx.temp.path);
        let error = read(&backend, &next).unwrap_err();
        assert_eq!(error.message, paging::PageError::StaleCursor.to_string());
        assert_eq!(backend.loads.get(), (3, 4));
        assert!(
            backend.cache.borrow().is_none(),
            "실패한 요청은 cache를 남기지 않음"
        );
        assert_eq!(
            read(&backend, &a).unwrap().get("total"),
            Some(&JsonValue::int(2102))
        );
        assert_eq!(backend.loads.get(), (4, 5));
        assert_eq!(file_digest(&fx.temp.path), after_save);

        // 문서 전환 중 실제 chunk 읽기가 실패해도 이전 tree로 대체하지 않습니다.
        let objects = fx.project.join(".stellar/cache/chunks/objects");
        let moved_objects = fx.temp.path.join("saved-objects");
        std::fs::rename(&objects, &moved_objects).unwrap();
        let unavailable = file_digest(&fx.temp.path);
        assert_eq!(read(&backend, &b).unwrap_err().code, -32000);
        assert_eq!(backend.loads.get(), (4, 6));
        assert!(backend.cache.borrow().is_none());
        assert_eq!(file_digest(&fx.temp.path), unavailable);
        std::fs::rename(moved_objects, objects).unwrap();
        read(&backend, &a).unwrap();
        assert_eq!(backend.loads.get(), (5, 7));
        assert_eq!(file_digest(&fx.temp.path), after_save);

        let fresh_session = fx.backend();
        read(&fresh_session, &a).unwrap();
        assert_eq!(
            fresh_session.loads.get(),
            (1, 1),
            "연결 사이 cache 공유 없음"
        );
        assert_eq!(file_digest(&fx.temp.path), after_save);
    }

    /// 서버 한 개를 initialize하고 `tools/call` 응답 하나를 돌려줍니다.
    fn served_call(server: crate::server::Server) -> JsonValue {
        let mut server = server;
        let mut log = crate::log::Logger::new(Vec::new());
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        server.handle_frame(init.as_bytes(), &mut log).unwrap();
        server.handle_frame(
            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut log,
        );
        let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"document.validate","arguments":{}}}"#;
        server.handle_frame(call.as_bytes(), &mut log).unwrap()
    }

    fn structured_kind(reply: &JsonValue) -> Option<&str> {
        reply
            .get("result")
            .and_then(|r| r.get("structuredContent"))
            .and_then(|s| s.get("kind"))
            .and_then(JsonValue::as_str)
    }

    #[test]
    fn call_deadline_and_response_limit_turn_results_into_retryable_failures() {
        // 검증 기준(XIX.9.10): 호출 대기 상한·응답 크기 상한 초과 응답.
        let fx = Fixture::new();
        save_documents(&fx.project, &[&large_document(50)]);
        let ok = served_call(crate::server::Server::headless(fx.backend()));
        assert_eq!(structured_kind(&ok), Some("ok"));
        let late = served_call(
            crate::server::Server::headless(fx.backend())
                .with_call_deadline(std::time::Duration::ZERO),
        );
        assert_eq!(structured_kind(&late), Some("timeout"));
        assert_eq!(
            late.get("result").and_then(|r| r.get("isError")),
            Some(&JsonValue::Bool(true))
        );
        let big =
            served_call(crate::server::Server::headless(fx.backend()).with_response_limit(200));
        assert_eq!(structured_kind(&big), Some("too-large"));
        assert!(json::write(&big).len() < 1024, "대체 응답은 작음");
    }

    #[test]
    fn revoked_identity_unavailable_source_and_missing_pointer_clear_cache() {
        let fx = Fixture::new();
        save_documents(&fx.project, &[&large_document(5)]);
        let backend = fx.backend();
        let uri = format!(
            "stellar://documents/{}?limit=2",
            large_document(5).document_id()
        );
        read(&backend, &uri).unwrap();
        FileTrustStore::open(&fx.data)
            .unwrap()
            .revoke(&fx.identity())
            .unwrap();
        let revoked = file_digest(&fx.temp.path);
        assert!(backend.list("resources/list", None).is_err());
        assert!(backend.cache.borrow().is_none());
        assert!(read(&backend, &uri).is_err());
        assert_eq!(file_digest(&fx.temp.path), revoked);
        fx.grant();
        read(&backend, &uri).unwrap();
        assert_eq!(backend.loads.get(), (2, 2));

        let manifest = fx.project.join("stellar.yaml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            original.replace(PROJECT_ID_TEXT, "0195c530-3acd-7bc1-8414-c213f61cb999"),
        )
        .unwrap();
        assert!(read(&backend, &uri).is_err());
        assert!(backend.cache.borrow().is_none());
        std::fs::write(&manifest, original).unwrap();
        read(&backend, &uri).unwrap();
        assert_eq!(backend.loads.get(), (3, 3));

        // 정상 파일을 잠시 이동한 동안 실패하고 복구 후 새로 엽니다.
        let moved = fx.temp.path.join("moved-project");
        std::fs::rename(&fx.project, &moved).unwrap();
        assert!(read(&backend, &uri).is_err());
        assert!(backend.cache.borrow().is_none());
        std::fs::rename(moved, &fx.project).unwrap();
        read(&backend, &uri).unwrap();
        assert_eq!(backend.loads.get(), (4, 4));

        let pointer = fx.project.join(".stellar/cache/chunks/current");
        let moved_pointer = fx.temp.path.join("saved-current");
        std::fs::rename(&pointer, &moved_pointer).unwrap();
        let unsaved = read(&backend, "stellar://project/structure").unwrap();
        assert_eq!(
            unsaved
                .get("revision")
                .unwrap()
                .get("token")
                .unwrap()
                .as_str(),
            Some("g0")
        );
        assert!(read(&backend, &uri).is_err());
        assert!(backend.cache.borrow().is_none());
        std::fs::rename(moved_pointer, pointer).unwrap();
        let restored = file_digest(&fx.temp.path);
        read(&backend, &uri).unwrap();
        assert_eq!(backend.loads.get(), (6, 5));
        assert_eq!(file_digest(&fx.temp.path), restored);
    }
}
