//! MCP payload의 프로젝트·세션 범위 제한 (XIX.2.5).
//!
//! agent에게 가는 프로젝트 내용은 사용자가 승인한 프로젝트 하나와 그 세션으로 제한합니다
//! (아키텍처 §31.3, ADR-0005 §3 "실행 인자로 고정한 프로젝트 루트 하나", attached는 결합 창 세션).
//! 세 지점에서 검사합니다.
//!
//! 1. **요청 resource URI**([`ApprovedScope::check_resource_uri`]): `stellar` scheme의 정의된
//!    resource·template만 받습니다(XIX.2.1). 다른 scheme(`file:` 등), 정의 밖 경로, `..`·percent
//!    encoding·빈 segment, 정의 밖 query(page가 아닌 resource의 query, page resource에서
//!    `cursor`·`limit` 밖의 key·중복·형식 오류, XIX.2.2 [`parse_page_query`])는 거부합니다. URI에는
//!    프로젝트를 고르는 수단이 없습니다.
//!    문서 id는 승인 프로젝트의 문서여야 합니다([`ProjectDocuments`]).
//! 2. **tool 인자**([`ApprovedScope::check_tool_arguments`]): 인자는 object이고 input schema에 선언된
//!    key만 받습니다. 그래서 `project`·`path`·`root` 같은 범위 밖 지정이 들어올 수 없습니다. 문서 id
//!    인자는 1과 같이 검사합니다.
//! 3. **응답 payload**([`ApprovedScope::release`]): backend가 만든 payload는 출처 프로젝트·세션
//!    표지를 달고, 승인 범위와 다르면 agent에게 보내지 않습니다(심층 방어).

use std::fmt;

use super::paging::parse_page_query;
use super::resources::{ResourceDef, RESOURCES};
use super::tools::ToolDef;
use crate::json::{self, JsonLimits, JsonValue};

/// 범위 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// `stellar` scheme이 아닌 URI.
    ForeignScheme,
    /// 정의된 resource가 아닌 URI(프로젝트 선택 시도 포함).
    UnknownResource,
    /// URI 형식이 위험하거나 잘못됐습니다(`..`, `%`, 빈 segment 등).
    MalformedUri,
    /// 승인 프로젝트의 문서가 아닙니다.
    ForeignDocument,
    /// tool 인자가 object가 아니거나 선언되지 않은 key가 있습니다.
    UndeclaredArgument {
        /// key(인자가 object가 아니면 빈 문자열).
        key: String,
    },
    /// payload의 출처 프로젝트가 승인 프로젝트와 다릅니다.
    ForeignProject,
    /// payload의 출처 세션이 승인 세션과 다릅니다.
    ForeignSession,
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::ForeignScheme => f.write_str("stellar scheme이 아닌 URI는 받지 않습니다"),
            ScopeError::UnknownResource => f.write_str("정의되지 않은 resource입니다"),
            ScopeError::MalformedUri => f.write_str("URI 형식이 올바르지 않습니다"),
            ScopeError::ForeignDocument => f.write_str("승인된 프로젝트의 문서가 아닙니다"),
            ScopeError::UndeclaredArgument { key } => {
                write!(f, "선언되지 않은 tool 인자입니다: {key:?}")
            }
            ScopeError::ForeignProject => f.write_str("승인된 프로젝트 밖의 내용입니다"),
            ScopeError::ForeignSession => f.write_str("다른 세션의 내용입니다"),
        }
    }
}

impl std::error::Error for ScopeError {}

/// 승인 프로젝트의 문서 목록 조회(backend가 구현).
pub trait ProjectDocuments {
    /// `document`가 승인 프로젝트의 문서인지.
    fn owns_document(&self, document: &str) -> bool;
}

/// 사용자가 승인한 범위: 프로젝트 하나와 세션 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedScope {
    project: String,
    session: String,
}

/// 출처 표지가 붙은 응답 payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedPayload {
    /// 만든 프로젝트 key.
    pub project: String,
    /// 만든 세션.
    pub session: String,
    /// 내용.
    pub body: JsonValue,
}

/// 해석한 resource 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTarget {
    /// 정의 이름.
    pub name: &'static str,
    /// URI의 문서 id(문서 template만).
    pub document: Option<String>,
    /// URI의 공유 정의 ID(컴포넌트 template만). backend는 승인된 프로젝트에서만 조회합니다.
    pub component: Option<String>,
    /// query(`cursor`·`limit`) 원문.
    pub query: Option<String>,
}

fn safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `def`의 URI template이 `path`(scheme 뒤, query 앞)와 맞으면 변수 값(`{document}`)을 돌려줍니다.
fn match_template(def: &ResourceDef, path: &str) -> Option<Option<String>> {
    let pattern = def.uri.strip_prefix("stellar://")?;
    let pattern = pattern.split('{').next().unwrap_or(pattern);
    let has_var = def.uri.contains("{document}") || def.uri.contains("{component}");
    if has_var {
        let rest = path.strip_prefix(pattern)?;
        safe_segment(rest).then(|| Some(rest.to_string()))
    } else {
        (path == pattern.trim_end_matches('/')).then_some(None)
    }
}

impl ApprovedScope {
    /// 승인 범위를 만듭니다. `project`는 프로젝트 식별 key(IV.2.3), `session`은 세션 id입니다.
    pub fn new(project: impl Into<String>, session: impl Into<String>) -> ApprovedScope {
        ApprovedScope {
            project: project.into(),
            session: session.into(),
        }
    }

    /// 승인 프로젝트 key.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// 요청 resource URI를 검사합니다.
    pub fn check_resource_uri(
        &self,
        uri: &str,
        documents: &dyn ProjectDocuments,
    ) -> Result<ResourceTarget, ScopeError> {
        let rest = uri
            .strip_prefix("stellar://")
            .ok_or(ScopeError::ForeignScheme)?;
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, Some(query.to_string())),
            None => (rest, None),
        };
        if path.contains('%') || path.contains("//") || path.ends_with('/') || path.is_empty() {
            return Err(ScopeError::MalformedUri);
        }
        if path.split('/').any(|s| !safe_segment(s)) {
            return Err(ScopeError::MalformedUri);
        }
        for def in RESOURCES {
            if let Some(value) = match_template(def, path) {
                let (document, component) = if def.uri.contains("{component}") {
                    (None, value)
                } else {
                    (value, None)
                };
                if query.is_some() && !def.paginated {
                    return Err(ScopeError::MalformedUri);
                }
                // page resource의 query는 `cursor`·`limit`만 받습니다(XIX.2.2 형식). 정의 밖 key는
                // 뒤 단계에 맡기지 않고 여기서 거부합니다.
                if query.is_some() && parse_page_query(uri).is_err() {
                    return Err(ScopeError::MalformedUri);
                }
                if let Some(document) = &document {
                    if !documents.owns_document(document) {
                        return Err(ScopeError::ForeignDocument);
                    }
                }
                return Ok(ResourceTarget {
                    name: def.name,
                    document,
                    component,
                    query,
                });
            }
        }
        Err(ScopeError::UnknownResource)
    }

    /// tool 인자를 검사합니다(선언된 key만, 문서 id는 승인 프로젝트 소속).
    pub fn check_tool_arguments(
        &self,
        tool: &ToolDef,
        arguments: &JsonValue,
        documents: &dyn ProjectDocuments,
    ) -> Result<(), ScopeError> {
        let pairs = arguments
            .as_object()
            .ok_or(ScopeError::UndeclaredArgument { key: String::new() })?;
        let schema = json::parse(tool.input_schema, JsonLimits::default())
            .map_err(|_| ScopeError::UndeclaredArgument { key: String::new() })?;
        let properties = schema.get("properties");
        for (key, value) in pairs {
            if properties.and_then(|p| p.get(key)).is_none() {
                return Err(ScopeError::UndeclaredArgument { key: key.clone() });
            }
            if key == "document" {
                let id = value
                    .as_str()
                    .ok_or(ScopeError::UndeclaredArgument { key: key.clone() })?;
                if !documents.owns_document(id) {
                    return Err(ScopeError::ForeignDocument);
                }
            }
        }
        Ok(())
    }

    /// payload를 agent에게 보낼 수 있으면 내용을 돌려줍니다.
    pub fn release(&self, payload: ScopedPayload) -> Result<JsonValue, ScopeError> {
        if payload.project != self.project {
            return Err(ScopeError::ForeignProject);
        }
        if payload.session != self.session {
            return Err(ScopeError::ForeignSession);
        }
        Ok(payload.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tools::{all_tools, DOCUMENT_TOOLS};

    struct Docs;
    impl ProjectDocuments for Docs {
        fn owns_document(&self, document: &str) -> bool {
            document == "01a318c7-1fc1-7334-93f0-7e1b813152c2"
        }
    }

    const MINE: &str = "01a318c7-1fc1-7334-93f0-7e1b813152c2";
    const OTHER: &str = "0195c530-3acd-7bc1-8414-c213f61cb999";

    fn scope() -> ApprovedScope {
        ApprovedScope::new("project-a", "session-1")
    }

    #[test]
    fn defined_resources_of_the_approved_project_are_accepted() {
        let scope = scope();
        for (uri, name) in [
            ("stellar://project/structure", "project-structure"),
            ("stellar://selection", "selection"),
            ("stellar://git/status", "git-status"),
            ("stellar://document/current", "current-document"),
            ("stellar://document/current?limit=10", "current-document"),
        ] {
            assert_eq!(
                scope.check_resource_uri(uri, &Docs).unwrap().name,
                name,
                "{uri}"
            );
        }
        let target = scope
            .check_resource_uri(
                &format!("stellar://documents/{MINE}?cursor=p1.3.x.g1"),
                &Docs,
            )
            .unwrap();
        assert_eq!(target.name, "document");
        assert_eq!(target.document.as_deref(), Some(MINE));
    }

    #[test]
    fn cross_project_access_is_rejected() {
        // 검증 기준(XIX.2.5): cross-project access 거부.
        let scope = scope();
        let cases = [
            (
                "file:///Users/me/other/stellar.yaml",
                ScopeError::ForeignScheme,
            ),
            ("https://example.com/x", ScopeError::ForeignScheme),
            (
                "stellar://projects/other/structure",
                ScopeError::UnknownResource,
            ),
            ("stellar://project/other", ScopeError::UnknownResource),
            (
                &format!("stellar://documents/{OTHER}") as &str,
                ScopeError::ForeignDocument,
            ),
            ("stellar://documents/..", ScopeError::MalformedUri),
            ("stellar://documents/%2e%2e", ScopeError::MalformedUri),
            ("stellar://documents//x", ScopeError::MalformedUri),
            (
                "stellar://project/structure?cursor=x",
                ScopeError::MalformedUri,
            ),
            ("stellar://", ScopeError::MalformedUri),
        ];
        for (uri, expected) in cases {
            assert_eq!(scope.check_resource_uri(uri, &Docs), Err(expected), "{uri}");
        }
        // tool 인자로 다른 프로젝트·경로를 지정할 수 없습니다.
        let validate = DOCUMENT_TOOLS
            .iter()
            .find(|t| t.name == "document.validate")
            .unwrap();
        for (args, expected) in [
            (
                r#"{"project":"/Users/me/other"}"#,
                ScopeError::UndeclaredArgument {
                    key: "project".into(),
                },
            ),
            (
                r#"{"path":"../other"}"#,
                ScopeError::UndeclaredArgument { key: "path".into() },
            ),
            (
                &format!(r#"{{"document":"{OTHER}"}}"#) as &str,
                ScopeError::ForeignDocument,
            ),
            ("[]", ScopeError::UndeclaredArgument { key: String::new() }),
        ] {
            let value = json::parse(args, JsonLimits::default()).unwrap();
            assert_eq!(
                scope.check_tool_arguments(validate, &value, &Docs),
                Err(expected),
                "{args}"
            );
        }
        let ok = json::parse(
            &format!(r#"{{"document":"{MINE}"}}"#),
            JsonLimits::default(),
        )
        .unwrap();
        scope.check_tool_arguments(validate, &ok, &Docs).unwrap();
        // 응답 payload도 다른 프로젝트·세션이면 내보내지 않습니다.
        let body = JsonValue::object(vec![("revision", JsonValue::Null)]);
        let payload = |project: &str, session: &str| ScopedPayload {
            project: project.into(),
            session: session.into(),
            body: body.clone(),
        };
        assert_eq!(
            scope.release(payload("project-b", "session-1")),
            Err(ScopeError::ForeignProject)
        );
        assert_eq!(
            scope.release(payload("project-a", "session-2")),
            Err(ScopeError::ForeignSession)
        );
        assert_eq!(scope.release(payload("project-a", "session-1")), Ok(body));
    }

    #[test]
    fn paginated_query_accepts_only_cursor_and_limit() {
        // page resource의 query도 정의된 key(`cursor`·`limit`)만 받습니다. 정의 밖 key로 범위를
        // 바꾸려는 시도는 뒤 단계(page 해석)에 기대지 않고 범위 검사에서 거부합니다.
        let scope = scope();
        for uri in [
            "stellar://document/current?project=other",
            "stellar://document/current?limit=10&project=other",
            &format!("stellar://documents/{MINE}?root=/Users/me/other") as &str,
            "stellar://document/current?limit=10&limit=20",
            "stellar://document/current?limit=010",
            "stellar://document/current?cursor=",
            "stellar://document/current?cursor",
        ] {
            assert_eq!(
                scope.check_resource_uri(uri, &Docs),
                Err(ScopeError::MalformedUri),
                "{uri}"
            );
        }
        let target = scope
            .check_resource_uri(
                &format!("stellar://documents/{MINE}?limit=10&cursor=p1.3.x.g1"),
                &Docs,
            )
            .unwrap();
        assert_eq!(target.query.as_deref(), Some("limit=10&cursor=p1.3.x.g1"));
    }

    #[test]
    fn no_tool_declares_a_project_or_path_argument() {
        for tool in all_tools() {
            let schema = json::parse(tool.input_schema, JsonLimits::default()).unwrap();
            for key in ["project", "project_root", "root", "path", "uri"] {
                assert!(
                    schema.get("properties").and_then(|p| p.get(key)).is_none(),
                    "{}: {key}",
                    tool.name
                );
            }
        }
    }
}
