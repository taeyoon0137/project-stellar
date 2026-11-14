//! 기존 CLI와 같은 저장 문서 validator 결과를 세션과 revision에 묶어 나눠 반환합니다.
use super::{check_fields, count, id, invalid, opt_id, revision, Documents, Headless};
use crate::catalog::paging::{self, PageRequest, PageSource};
use crate::catalog::tools::VALIDATE_DOCUMENT;
use crate::json::{self, JsonValue as J};
use crate::jsonrpc::RpcError;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::revision::ProjectRevision;
use stellar_ops::snapshot::ProjectSnapshot;
use stellar_ops::validate::{validate_snapshot_document, Finding, FindingSeverity};

#[derive(Debug)]
pub(super) struct FindingsPage {
    document: Option<StableId>,
    revision: ProjectRevision,
    documents_checked: usize,
    errors: usize,
    findings: Vec<Finding>,
}

fn cursor_key(document: Option<StableId>) -> String {
    document.map_or_else(|| "validate-all".into(), |id| format!("validate-{id}"))
}

impl FindingsPage {
    fn load(snapshot: &ProjectSnapshot, document: Option<StableId>) -> Self {
        let mut documents_checked = 0;
        let mut findings = Vec::new();
        for definition in snapshot
            .documents()
            .iter()
            .filter(|d| document.is_none_or(|id| id == d.id))
        {
            let checked = validate_snapshot_document(snapshot, definition.id);
            documents_checked += 1;
            findings.extend(checked.findings);
        }
        let errors = findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Error)
            .count();
        Self {
            document,
            revision: snapshot.revision(),
            documents_checked,
            errors,
            findings,
        }
    }
}

impl PageSource for FindingsPage {
    fn key(&self) -> String {
        cursor_key(self.document)
    }
    fn revision_token(&self) -> String {
        self.revision.token()
    }
    fn revision_json(&self) -> J {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.findings.len()
    }
    fn header(&self) -> Vec<(&'static str, J)> {
        vec![
            ("document", opt_id(self.document)),
            ("documents_checked", count(self.documents_checked)),
            ("valid", J::Bool(self.errors == 0)),
            ("errors", count(self.errors)),
            ("warnings", count(self.findings.len() - self.errors)),
        ]
    }
    fn item(&self, index: usize) -> J {
        let f = &self.findings[index];
        let severity = match f.severity {
            FindingSeverity::Error => "error",
            FindingSeverity::Warning => "warning",
        };
        J::object(vec![
            ("document", id(f.document)),
            ("severity", J::string(severity)),
            ("code", J::string(f.code)),
            ("message", J::string(&f.message)),
        ])
    }
}

impl Headless {
    /// 저장 문서 검증 요청. 결과의 오류·경고는 검사가 정상 완료된 경우에도 반환됩니다.
    pub fn call(&self, params: Option<&J>) -> Result<J, RpcError> {
        self.with_snapshot(|cached| {
            check_fields(params, &["name", "arguments"])?;
            let params = params.ok_or_else(|| invalid("tool 이름이 필요합니다"))?;
            if params.get("name").and_then(J::as_str) != Some(VALIDATE_DOCUMENT.name) {
                return Err(invalid("제공하지 않는 tool입니다"));
            }
            let empty = J::Object(Vec::new());
            let arguments = params.get("arguments").unwrap_or(&empty);
            self.scope
                .check_tool_arguments(&VALIDATE_DOCUMENT, arguments, &Documents(&cached.snapshot))
                .map_err(|error| invalid(error.to_string()))?;
            let document = arguments
                .get("document")
                .map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<StableId>().ok())
                        .ok_or_else(|| invalid("문서 ID가 필요합니다"))
                })
                .transpose()?;
            let cursor = arguments
                .get("cursor")
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("cursor는 문자열이어야 합니다"))
                })
                .transpose()?;
            let limit = arguments
                .get("limit")
                .map(|v| {
                    v.as_i64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| invalid("limit은 양의 정수여야 합니다"))
                })
                .transpose()?;
            let request = PageRequest { cursor, limit };
            paging::validate_request(
                &cursor_key(document),
                &cached.snapshot.revision().token(),
                &request,
            )
            .map_err(|error| invalid(error.to_string()))?;
            if cached
                .validation
                .as_ref()
                .is_none_or(|page| page.document != document)
            {
                cached.validation = None;
                cached.validation = Some(FindingsPage::load(&cached.snapshot, document));
            }
            let body =
                paging::build_page(cached.validation.as_ref().expect("검증을 마쳤음"), &request)
                    .map_err(|error| invalid(error.to_string()))?;
            Ok(J::object(vec![(
                "content",
                J::Array(vec![J::object(vec![
                    ("type", J::string("text")),
                    ("text", J::string(json::write(&body))),
                ])]),
            )]))
        })
    }
}
