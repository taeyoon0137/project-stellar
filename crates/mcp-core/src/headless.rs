//! 고정한 신뢰 프로젝트의 읽기 resource만 제공하는 L2 backend (XIX.9.3).
use stellar_ops::core_types::ids::StableId;
use stellar_ops::query::{
    project_structure, DocumentOutline, DocumentTree, ProjectStructure, TreeCursor,
};
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
}

impl Headless {
    /// 파일 경로가 아닌 기존 신뢰 등록 ID로 선택합니다. 승인·신뢰 상태를 쓰지 않습니다.
    pub fn select(project: &str) -> Result<Self, crate::exit::Exit> {
        let id = project
            .parse::<StableId>()
            .map_err(|_| crate::exit::USAGE)?;
        let project = TrustedProject::select(id).map_err(|_| crate::exit::NOPERM)?;
        let session = format!("headless-{}", std::process::id());
        Ok(Self {
            scope: ApprovedScope::new(id.to_string(), session.clone()),
            project,
            session,
        })
    }

    /// 요청과 응답 시점의 기존 신뢰·identity 확인.
    pub fn check(&self) -> Result<(), RpcError> {
        self.project.check().map_err(read_error)
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
        self.check()?;
        check_fields(params, &["uri"])?;
        let uri = params
            .and_then(|p| p.get("uri"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid("resource URI가 필요합니다"))?;
        let snapshot = self.project.snapshot().map_err(read_error)?;
        let target = self
            .scope
            .check_resource_uri(uri, &Documents(&snapshot))
            .map_err(|e| invalid(e.to_string()))?;
        let body = match target.name {
            "project-structure" => {
                JsonValue::object(structure_fields(&project_structure(&snapshot)))
            }
            "document" => {
                let document = target
                    .document
                    .as_deref()
                    .and_then(|id| id.parse::<StableId>().ok())
                    .ok_or_else(|| invalid("승인된 문서 ID가 필요합니다"))?;
                let (_, request) =
                    paging::parse_page_query(uri).map_err(|e| invalid(e.to_string()))?;
                let tree = DocumentTree::load(&snapshot, document)
                    .map_err(|_| RpcError::new(-32000, "문서 snapshot을 읽을 수 없습니다"))?;
                let source = TreeSource {
                    tree,
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
        self.check()?;
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

struct TreeSource {
    tree: DocumentTree,
    document: StableId,
    revision: ProjectRevision,
}
impl PageSource for TreeSource {
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
        let page = self
            .tree
            .page(
                Some(&TreeCursor {
                    document: self.document,
                    offset: index,
                    revision: self.revision,
                }),
                1,
            )
            .expect("PageSource index는 같은 tree의 total 미만");
        let node = &page.nodes[0];
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
