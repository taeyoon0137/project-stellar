//! 기존 CLI와 같은 저장 문서 validator 결과를 세션과 revision에 묶어 나눠 반환합니다.
use super::{count, id, invalid, opt_id, revision, CachedRead};
use crate::catalog::paging::{self, PageRequest, PageSource};
use crate::json::JsonValue as J;
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

pub(super) fn page(
    cached: &mut CachedRead,
    arguments: &J,
    request: &PageRequest,
) -> Result<J, RpcError> {
    let document = arguments
        .get("document")
        .map(|v| {
            v.as_str()
                .and_then(|s| s.parse::<StableId>().ok())
                .ok_or_else(|| invalid("문서 ID가 필요합니다"))
        })
        .transpose()?;
    paging::validate_request(
        &cursor_key(document),
        &cached.snapshot.revision().token(),
        request,
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

    paging::build_page(cached.validation.as_ref().expect("검증을 마쳤음"), request)
        .map_err(|error| invalid(error.to_string()))
}
