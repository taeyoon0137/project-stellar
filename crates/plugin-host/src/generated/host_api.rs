//! 생성 파일입니다. `schemas/plugin-host-api.idl.json`에서 `stellar-plugin-bindgen`이 만듭니다(ADR-0097).
//! 손으로 고치지 않습니다. IDL을 고친 뒤 `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen`을 실행합니다.

use crate::idl::{ParamSpec, ParamType};
use crate::permission::Permission;

/// Host API major version.
pub const HOST_API_VERSION: u32 = 1;

/// IDL 형식 버전.
pub const IDL_VERSION: u32 = 1;

/// RPC namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    /// Host 정보. 권한이 필요 없습니다.
    Host,
    /// 문서 snapshot 읽기.
    Document,
    /// 문서 projection 조회.
    Query,
    /// CommandTransaction 적용.
    Command,
    /// Git 상태 읽기.
    Git,
    /// 플러그인 namespace 디렉터리(`plugins/<plugin-id>/`) 파일.
    File,
    /// network 요청. Host가 제공하는 network port로만 보냅니다.
    Network,
}

impl Namespace {
    /// 모든 namespace(IDL 순).
    pub const ALL: [Namespace; 7] = [Namespace::Host, Namespace::Document, Namespace::Query, Namespace::Command, Namespace::Git, Namespace::File, Namespace::Network];

    /// IDL 이름.
    pub fn name(self) -> &'static str {
        match self {
            Namespace::Host => "host",
            Namespace::Document => "document",
            Namespace::Query => "query",
            Namespace::Command => "command",
            Namespace::Git => "git",
            Namespace::File => "file",
            Namespace::Network => "network",
        }
    }
}

/// Host API method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Method {
    /// `host.info`: Host API version, 플러그인 id, 유효 권한, RPC 상한.
    HostInfo,
    /// `document.read_snapshot`: 프로젝트 revision과 문서·페이지·View 개요(ops::query::project_structure).
    DocumentReadSnapshot,
    /// `query.tree_page`: 문서 트리 page(ops::query::document_tree_page).
    QueryTreePage,
    /// `query.node`: node 공통 필드와 속성(ops::query::node_detail).
    QueryNode,
    /// `query.tokens`: token 정의 집합(ops::query::tokens).
    QueryTokens,
    /// `query.components`: component 정의 집합(ops::query::components).
    QueryComponents,
    /// `command.apply`: stellar apply와 같은 ops::apply::apply. command object는 schemas/command-transaction.schema.json 형식입니다.
    CommandApply,
    /// `git.status`: stellar git status와 같은 ops::git::GitSession::status. index를 바꾸지 않습니다.
    GitStatus,
    /// `file.read`: UTF-8 텍스트 파일 읽기.
    FileRead,
    /// `file.write`: UTF-8 텍스트 파일 쓰기(임시 파일 뒤 rename).
    FileWrite,
    /// `file.list`: namespace 디렉터리의 파일 목록.
    FileList,
    /// `network.fetch`: https GET.
    NetworkFetch,
}

impl Method {
    /// 모든 method(IDL 순).
    pub const ALL: [Method; 12] = [
        Method::HostInfo,
        Method::DocumentReadSnapshot,
        Method::QueryTreePage,
        Method::QueryNode,
        Method::QueryTokens,
        Method::QueryComponents,
        Method::CommandApply,
        Method::GitStatus,
        Method::FileRead,
        Method::FileWrite,
        Method::FileList,
        Method::NetworkFetch,
    ];

    /// wire 이름(`namespace.method`).
    pub fn name(self) -> &'static str {
        match self {
            Method::HostInfo => "host.info",
            Method::DocumentReadSnapshot => "document.read_snapshot",
            Method::QueryTreePage => "query.tree_page",
            Method::QueryNode => "query.node",
            Method::QueryTokens => "query.tokens",
            Method::QueryComponents => "query.components",
            Method::CommandApply => "command.apply",
            Method::GitStatus => "git.status",
            Method::FileRead => "file.read",
            Method::FileWrite => "file.write",
            Method::FileList => "file.list",
            Method::NetworkFetch => "network.fetch",
        }
    }

    /// namespace.
    pub fn namespace(self) -> Namespace {
        match self {
            Method::HostInfo => Namespace::Host,
            Method::DocumentReadSnapshot => Namespace::Document,
            Method::QueryTreePage => Namespace::Query,
            Method::QueryNode => Namespace::Query,
            Method::QueryTokens => Namespace::Query,
            Method::QueryComponents => Namespace::Query,
            Method::CommandApply => Namespace::Command,
            Method::GitStatus => Namespace::Git,
            Method::FileRead => Namespace::File,
            Method::FileWrite => Namespace::File,
            Method::FileList => Namespace::File,
            Method::NetworkFetch => Namespace::Network,
        }
    }

    /// 필요 권한(`None`이면 권한 없음).
    pub fn permission(self) -> Option<Permission> {
        match self {
            Method::HostInfo => None,
            Method::DocumentReadSnapshot => Some(Permission::DocumentRead),
            Method::QueryTreePage => Some(Permission::DocumentRead),
            Method::QueryNode => Some(Permission::DocumentRead),
            Method::QueryTokens => Some(Permission::DocumentRead),
            Method::QueryComponents => Some(Permission::DocumentRead),
            Method::CommandApply => Some(Permission::DocumentApply),
            Method::GitStatus => Some(Permission::GitRead),
            Method::FileRead => Some(Permission::File),
            Method::FileWrite => Some(Permission::File),
            Method::FileList => Some(Permission::File),
            Method::NetworkFetch => Some(Permission::Network),
        }
    }

    /// 결과 record 이름.
    pub fn result_type(self) -> &'static str {
        match self {
            Method::HostInfo => "HostInfo",
            Method::DocumentReadSnapshot => "ProjectSnapshot",
            Method::QueryTreePage => "TreePage",
            Method::QueryNode => "NodeDetail",
            Method::QueryTokens => "DefinitionSet",
            Method::QueryComponents => "DefinitionSet",
            Method::CommandApply => "ApplyResult",
            Method::GitStatus => "GitStatus",
            Method::FileRead => "FileContent",
            Method::FileWrite => "FileWritten",
            Method::FileList => "FileList",
            Method::NetworkFetch => "NetworkResponse",
        }
    }

    /// 매개변수 표(IDL 순).
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            Method::HostInfo => &[],
            Method::DocumentReadSnapshot => &[],
            Method::QueryTreePage => &[ParamSpec { name: "document", ty: ParamType::Id, required: true }, ParamSpec { name: "cursor", ty: ParamType::String, required: false }, ParamSpec { name: "limit", ty: ParamType::U32, required: false }],
            Method::QueryNode => &[ParamSpec { name: "document", ty: ParamType::Id, required: true }, ParamSpec { name: "node", ty: ParamType::Id, required: true }],
            Method::QueryTokens => &[],
            Method::QueryComponents => &[],
            Method::CommandApply => &[ParamSpec { name: "base_revision", ty: ParamType::Revision, required: true }, ParamSpec { name: "document", ty: ParamType::Id, required: true }, ParamSpec { name: "commands", ty: ParamType::JsonList, required: true }, ParamSpec { name: "dry_run", ty: ParamType::Bool, required: false }],
            Method::GitStatus => &[ParamSpec { name: "cursor", ty: ParamType::String, required: false }, ParamSpec { name: "limit", ty: ParamType::U32, required: false }],
            Method::FileRead => &[ParamSpec { name: "path", ty: ParamType::String, required: true }],
            Method::FileWrite => &[ParamSpec { name: "path", ty: ParamType::String, required: true }, ParamSpec { name: "text", ty: ParamType::String, required: true }],
            Method::FileList => &[],
            Method::NetworkFetch => &[ParamSpec { name: "url", ty: ParamType::String, required: true }],
        }
    }

    /// wire 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<Method> {
        Method::ALL.into_iter().find(|m| m.name() == name)
    }
}
