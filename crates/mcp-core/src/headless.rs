//! 고정한 신뢰 프로젝트의 읽기 resource만 제공하는 L2 backend (XIX.9.3).
use std::cell::RefCell;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::query::{project_structure, DocumentOutline, DocumentTree, ProjectStructure};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::snapshot::ProjectSnapshot;
use stellar_ops::trusted::{TrustedProject, TrustedReadError};

use crate::catalog::paging::{self, PageSource};
use crate::catalog::resources::{descriptor, resource};
use crate::catalog::scope::{ApprovedScope, ProjectDocuments, ScopedPayload};
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

/// 연결에 고정한 프로젝트의 snapshot 하나와 가장 최근 문서 하나만 보관합니다.
#[derive(Debug)]
struct CachedRead {
    snapshot: ProjectSnapshot,
    tree: Option<(StableId, DocumentTree)>,
}

impl Headless {
    /// 파일 경로가 아닌 기존 신뢰 등록 ID로 선택합니다. 승인·신뢰 상태를 쓰지 않습니다.
    pub fn select(project: &str) -> Result<Self, crate::exit::Exit> {
        let id = project
            .parse::<StableId>()
            .map_err(|_| crate::exit::USAGE)?;
        let project = TrustedProject::select(id).map_err(|_| crate::exit::NOPERM)?;
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

    /// 구현한 resource·template과 빈 tool 목록만 노출합니다.
    pub fn list(&self, method: &str, params: Option<&JsonValue>) -> Result<JsonValue, RpcError> {
        self.check()?;
        check_fields(params, &[])?;
        let (field, name) = match method {
            "resources/list" => ("resources", Some("project-structure")),
            "resources/templates/list" => ("resourceTemplates", Some("document")),
            _ => ("tools", None),
        };
        Ok(JsonValue::object(vec![(
            field,
            JsonValue::Array(
                name.into_iter()
                    .map(|name| descriptor(resource(name).expect("catalog resource")))
                    .collect(),
            ),
        )]))
    }

    /// scope 확인 → ops snapshot → 기존 catalog 결과 → scope release.
    pub fn read(&self, params: Option<&JsonValue>) -> Result<JsonValue, RpcError> {
        // 성공하고 응답 직전에도 같은 원천일 때만 다시 보관합니다. 어느 실패에도 잔여 cache 없음.
        let mut cache = self.cache.borrow_mut().take();
        let result = self.read_cached(params, &mut cache);
        if result.is_ok() {
            *self.cache.borrow_mut() = cache;
        }
        result
    }

    fn read_cached(
        &self,
        params: Option<&JsonValue>,
        cache: &mut Option<CachedRead>,
    ) -> Result<JsonValue, RpcError> {
        if let Some(cached) = cache {
            if !self
                .project
                .snapshot_is_current(&cached.snapshot)
                .map_err(read_error)?
            {
                *cache = None;
            }
        } else {
            self.check()?;
        }
        check_fields(params, &["uri"])?;
        let uri = params
            .and_then(|p| p.get("uri"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("resource URI가 필요합니다"))?;
        if cache.is_none() {
            #[cfg(test)]
            self.loads.set((self.loads.get().0 + 1, self.loads.get().1));
            *cache = Some(CachedRead {
                snapshot: self.project.snapshot().map_err(read_error)?,
                tree: None,
            });
        }
        let cached = cache.as_mut().expect("snapshot을 열었음");
        let snapshot = &cached.snapshot;
        let target = self
            .scope
            .check_resource_uri(uri, &Documents(snapshot))
            .map_err(|e| invalid(e.to_string()))?;
        let body = match target.name {
            "project-structure" => {
                JsonValue::object(structure_fields(&project_structure(snapshot)))
            }
            "document" => {
                let document = target
                    .document
                    .as_deref()
                    .and_then(|id| id.parse::<StableId>().ok())
                    .ok_or_else(|| invalid("승인된 문서 ID가 필요합니다"))?;
                let (_, request) =
                    paging::parse_page_query(uri).map_err(|e| invalid(e.to_string()))?;
                paging::validate_request(
                    &document.to_string(),
                    &snapshot.revision().token(),
                    &request,
                )
                .map_err(|e| invalid(e.to_string()))?;
                if cached.tree.as_ref().is_none_or(|(id, _)| *id != document) {
                    cached.tree = None;
                    #[cfg(test)]
                    self.loads.set((self.loads.get().0, self.loads.get().1 + 1));
                    cached.tree = Some((
                        document,
                        DocumentTree::load(snapshot, document).map_err(|_| {
                            RpcError::new(-32000, "문서 snapshot을 읽을 수 없습니다")
                        })?,
                    ));
                }
                let source = TreeSource {
                    tree: &cached.tree.as_ref().expect("문서 tree를 열었음").1,
                    document,
                    revision: snapshot.revision(),
                };
                paging::build_page(&source, &request).map_err(|e| invalid(e.to_string()))?
            }
            _ => return Err(invalid("제공하지 않는 resource입니다")),
        };
        if json::write(&body).len() > paging::MAX_PAGE_BYTES {
            return Err(invalid("resource가 응답 크기 상한을 넘습니다"));
        }
        let result = paging::read_result(uri, &body);
        if json::write(&result).len() > crate::server::MAX_MESSAGE_BYTES - 1024 {
            return Err(invalid("resource가 응답 크기 상한을 넘습니다"));
        }
        if !self
            .project
            .snapshot_is_current(snapshot)
            .map_err(read_error)?
        {
            return Err(read_error(TrustedReadError::Unavailable));
        }
        self.scope
            .release(ScopedPayload {
                project: snapshot.project_id().to_string(),
                session: self.session.clone(),
                body: result,
            })
            .map_err(|e| invalid(e.to_string()))
    }
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, message)
}

fn read_error(error: TrustedReadError) -> RpcError {
    RpcError::new(
        -32000,
        match error {
            TrustedReadError::NotTrusted => "프로젝트 읽기 신뢰를 확인할 수 없습니다",
            TrustedReadError::Unavailable => "프로젝트 snapshot을 읽을 수 없습니다",
        },
    )
}

fn check_fields(params: Option<&JsonValue>, allowed: &[&str]) -> Result<(), RpcError> {
    if let Some(params) = params {
        let fields = params
            .as_object()
            .ok_or_else(|| invalid("params는 object여야 합니다"))?;
        if fields
            .iter()
            .any(|(key, _)| !allowed.contains(&key.as_str()) && key != "_meta")
        {
            return Err(invalid("선언되지 않은 요청 인자입니다"));
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
            Headless::new(select_trusted_project(project_id(), self.data.clone()).unwrap())
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
        assert_eq!(backend.loads.get(), (1, 1));
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
        let objects = fx.project.join("design/objects");
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

        let pointer = fx.project.join("design/current");
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
