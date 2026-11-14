//! HEAD commit과 working generation에 고정한 문서 변경 페이지.
use super::{id, invalid, opt_id, read_error, revision, uint, CachedRead};
use crate::catalog::paging::{self, PageRequest, PageSource};
use crate::json::{self, JsonLimits, JsonValue};
use crate::jsonrpc::RpcError;
use stellar_ops::compare::semantic_diff_head;
use stellar_ops::diff::{DiffEntry, DiffPosition, DiffReport};
use stellar_ops::query::OpsValue;
use stellar_ops::revision::ProjectRevision;
use stellar_ops::trusted::{TrustedProject, TrustedReadError};

#[derive(Debug)]
pub(super) struct DiffPage {
    head: String,
    revision: ProjectRevision,
    report: DiffReport,
}

fn cursor_revision(head: &str, revision: ProjectRevision) -> String {
    format!("{head}-{}", revision.token())
}

impl PageSource for DiffPage {
    fn key(&self) -> String {
        "semantic-diff".into()
    }
    fn revision_token(&self) -> String {
        cursor_revision(&self.head, self.revision)
    }
    fn revision_json(&self) -> JsonValue {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.report.entries.len()
    }
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        vec![("base_commit", JsonValue::string(&self.head))]
    }
    fn item(&self, index: usize) -> JsonValue {
        diff_entry(&self.report.entries[index])
    }
}

pub(super) fn page(
    project: &TrustedProject,
    cached: &mut CachedRead,
    arguments: &JsonValue,
    request: &PageRequest,
) -> Result<JsonValue, RpcError> {
    if arguments
        .get("base")
        .is_some_and(|base| base.as_str() != Some("HEAD"))
    {
        return Err(invalid("base에는 HEAD만 지정할 수 있습니다"));
    }
    let unavailable =
        || RpcError::new(-32000, "같은 프로젝트의 HEAD와 현재 저장 세대가 필요합니다");
    let head = project.head().map_err(|error| match error {
        TrustedReadError::Unavailable => unavailable(),
        other => read_error(other),
    })?;
    let revision = cached.snapshot.revision();
    paging::validate_request(
        "semantic-diff",
        &cursor_revision(head.commit(), revision),
        request,
    )
    .map_err(|e| invalid(e.to_string()))?;

    if cached
        .comparison
        .as_ref()
        .is_none_or(|page| page.head != head.commit())
    {
        cached.comparison = None;
        let report = semantic_diff_head(&head, &cached.snapshot).map_err(|_| unavailable())?;
        cached.comparison = Some(DiffPage {
            head: head.commit().into(),
            revision,
            report,
        });
    }
    let body = paging::build_page(cached.comparison.as_ref().expect("비교가 끝남"), request)
        .map_err(|e| invalid(e.to_string()))?;
    if !head.is_current().map_err(|_| unavailable())? {
        return Err(read_error(TrustedReadError::Unavailable));
    }
    Ok(body)
}

fn float(value: f32) -> JsonValue {
    json::parse(&value.to_string(), JsonLimits::default()).unwrap_or(JsonValue::Null)
}

/// bytes 소문자 16진수.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn value_inner(value: &OpsValue) -> JsonValue {
    match value {
        OpsValue::Bool(v) => JsonValue::Bool(*v),
        OpsValue::Uint(v) => uint(*v),
        OpsValue::Text(v) => JsonValue::string(v.clone()),
        OpsValue::Id(v) => id(*v),
        OpsValue::Ids(v) => JsonValue::Array(v.iter().copied().map(id).collect()),
        OpsValue::Floats(v) => JsonValue::Array(v.iter().copied().map(float).collect()),
        OpsValue::Typed { type_tag, bytes } => JsonValue::object(vec![
            ("type_tag", uint(u64::from(*type_tag))),
            ("hex", JsonValue::string(hex(bytes))),
        ]),
        OpsValue::Bytes(v) => JsonValue::object(vec![("hex", JsonValue::string(hex(v)))]),
    }
}

fn diff_position(position: Option<&DiffPosition>) -> JsonValue {
    match position {
        None => JsonValue::Null,
        Some(p) => JsonValue::object(vec![
            ("parent", opt_id(p.parent)),
            (
                "order_key",
                JsonValue::opt_string(p.order_key_hex.as_deref()),
            ),
        ]),
    }
}

/// 선택 속성 값(`{"type","value"}` 또는 `null`).
fn opt_value(value: Option<&OpsValue>) -> JsonValue {
    value.map_or(JsonValue::Null, |v| {
        JsonValue::object(vec![
            ("type", JsonValue::string(v.type_name())),
            ("value", value_inner(v)),
        ])
    })
}

/// semantic diff 변경 한 건.
fn diff_entry(entry: &DiffEntry) -> JsonValue {
    JsonValue::object(vec![
        ("kind", JsonValue::string(entry.kind)),
        ("document", opt_id(entry.document)),
        ("object", id(entry.object)),
        (
            "object_kind",
            JsonValue::opt_string(entry.object_kind.as_deref()),
        ),
        ("path", JsonValue::opt_string(entry.path.as_deref())),
        ("old", opt_value(entry.old.as_ref())),
        ("new", opt_value(entry.new.as_ref())),
        ("from", diff_position(entry.from.as_ref())),
        ("to", diff_position(entry.to.as_ref())),
    ])
}
