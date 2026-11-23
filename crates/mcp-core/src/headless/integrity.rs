//! `document.diagnostics`: `stellar diagnose`와 같은 ops 수집(`diagnose_snapshot`)·index 필터 결과를 세션과 revision에
//! 묶어 나눠 반환합니다(ADR-0095).
use std::collections::BTreeSet;

use super::{count, invalid, opt_id, revision, CachedRead};
use crate::catalog::paging::{self, PageRequest, PageSource};
use crate::json::JsonValue as J;
use crate::jsonrpc::RpcError;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::integrity::diagnose_snapshot;
use stellar_ops::integrity::diagnostic::{Diagnostic, DiagnosticKind, Severity};
use stellar_ops::integrity::index::{DiagnosticFilter, MAX_PAGE_LIMIT};
use stellar_ops::query::property_path_label;
use stellar_ops::revision::ProjectRevision;
use stellar_ops::snapshot::ProjectSnapshot;

#[derive(Debug)]
pub(super) struct DiagnosticsPage {
    filter: DiagnosticFilter,
    revision: ProjectRevision,
    documents_checked: usize,
    total: usize,
    items: Vec<Diagnostic>,
}

fn cursor_key(filter: &DiagnosticFilter) -> String {
    let kinds: Vec<&str> = filter.kinds.iter().map(|k| k.name()).collect();
    format!(
        "diagnostics-{}-{}-{}-{}",
        match filter.document {
            None => "all".to_string(),
            Some(None) => "project".to_string(),
            Some(Some(document)) => document.to_string(),
        },
        kinds.join(","),
        filter.min_severity.map_or("any", |s| s.name()),
        filter
            .source
            .map_or_else(|| "any".to_string(), |s| s.to_string()),
    )
}

impl DiagnosticsPage {
    fn load(snapshot: &ProjectSnapshot, filter: DiagnosticFilter) -> Self {
        let collected = diagnose_snapshot(snapshot);
        let index = collected.engine.index();
        // index의 필터 page를 끝까지 읽어 이 연결의 page 원본으로 둡니다(필터 판정은 index 한 곳).
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let page = index
                .page(&filter, cursor.as_ref(), MAX_PAGE_LIMIT)
                .expect("같은 index를 읽는 동안 revision이 바뀌지 않음");
            items.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Self {
            filter,
            revision: snapshot.revision(),
            documents_checked: collected.documents_checked,
            total: index.len(),
            items,
        }
    }
}

impl PageSource for DiagnosticsPage {
    fn key(&self) -> String {
        cursor_key(&self.filter)
    }
    fn revision_token(&self) -> String {
        self.revision.token()
    }
    fn revision_json(&self) -> J {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.items.len()
    }
    fn header(&self) -> Vec<(&'static str, J)> {
        vec![
            ("documents_checked", count(self.documents_checked)),
            ("diagnostics_total", count(self.total)),
        ]
    }
    fn item(&self, index: usize) -> J {
        let d = &self.items[index];
        let remediation = d
            .remediation
            .iter()
            .map(|action| {
                J::object(vec![
                    ("action", J::string(action.name())),
                    ("form", J::string(action.descriptor().form.name())),
                ])
            })
            .collect();
        J::object(vec![
            ("id", J::string(d.id().to_string())),
            ("kind", J::string(d.kind().name())),
            ("severity", J::string(d.severity.name())),
            ("code", J::string(d.code())),
            ("origin", J::string(&d.key.origin)),
            ("document", opt_id(d.document())),
            ("source", opt_id(d.source())),
            (
                "property",
                d.property()
                    .map_or(J::Null, |p| J::string(property_path_label(p))),
            ),
            ("message", J::string(&d.message)),
            ("remediation", J::Array(remediation)),
        ])
    }
}

fn parse_filter(arguments: &J) -> Result<DiagnosticFilter, RpcError> {
    let mut filter = DiagnosticFilter::default();
    let parse_id = |value: &J, what: &str| {
        value
            .as_str()
            .and_then(|s| s.parse::<StableId>().ok())
            .ok_or_else(|| invalid(format!("{what} ID가 필요합니다")))
    };
    if let Some(document) = arguments.get("document") {
        filter.document = Some(Some(parse_id(document, "문서")?));
    }
    match arguments.get("project_only") {
        None | Some(J::Bool(false)) => {}
        Some(J::Bool(true)) if filter.document.is_none() => filter.document = Some(None),
        Some(J::Bool(true)) => {
            return Err(invalid("document와 project_only는 함께 쓸 수 없습니다"))
        }
        Some(_) => return Err(invalid("project_only는 boolean이어야 합니다")),
    }
    if let Some(kinds) = arguments.get("kinds") {
        let J::Array(kinds) = kinds else {
            return Err(invalid("kinds는 배열이어야 합니다"));
        };
        let parsed: Option<BTreeSet<DiagnosticKind>> = kinds
            .iter()
            .map(|k| k.as_str().and_then(DiagnosticKind::from_name))
            .collect();
        filter.kinds = parsed.ok_or_else(|| invalid("알 수 없는 진단 종류입니다"))?;
    }
    if let Some(severity) = arguments.get("min_severity") {
        filter.min_severity = Some(
            severity
                .as_str()
                .and_then(Severity::from_name)
                .ok_or_else(|| invalid("min_severity는 info|warning|error입니다"))?,
        );
    }
    if let Some(source) = arguments.get("source") {
        filter.source = Some(parse_id(source, "source")?);
    }
    Ok(filter)
}

pub(super) fn page(
    cached: &mut CachedRead,
    arguments: &J,
    request: &PageRequest,
) -> Result<J, RpcError> {
    let filter = parse_filter(arguments)?;
    paging::validate_request(
        &cursor_key(&filter),
        &cached.snapshot.revision().token(),
        request,
    )
    .map_err(|error| super::Failure::failure(&error))?;
    if cached
        .diagnostics
        .as_ref()
        .is_none_or(|page| page.filter != filter)
    {
        cached.diagnostics = None;
        cached.diagnostics = Some(DiagnosticsPage::load(&cached.snapshot, filter));
    }
    paging::build_page(cached.diagnostics.as_ref().expect("진단을 모았음"), request)
        .map_err(|error| super::Failure::failure(&error))
}
