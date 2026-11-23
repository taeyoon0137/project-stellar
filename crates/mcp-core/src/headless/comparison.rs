//! 고정 HEAD·working generation·프로젝트 컴포넌트 YAML 집합의 변경 페이지.
use super::{id, invalid, opt_id, read_error, revision, uint, CachedRead};
use crate::catalog::paging::{self, PageRequest, PageSource};
use crate::json::{self, JsonLimits, JsonValue};
use crate::jsonrpc::RpcError;
use stellar_ops::compare::{working_components, HeadDiffBase};
use stellar_ops::diff::{DiffEntry, DiffPosition, DiffReport};
use stellar_ops::query::OpsValue;
use stellar_ops::revision::ProjectRevision;
use stellar_ops::trusted::{TrustedProject, TrustedReadError};

#[derive(Debug)]
pub(super) struct DiffPage {
    base: HeadDiffBase,
    revision: ProjectRevision,
    component_source_hash: String,
    report: DiffReport,
}

fn cursor_revision(head: &str, revision: ProjectRevision, component_source_hash: &str) -> String {
    format!("{head}-{}-{component_source_hash}", revision.token())
}

impl PageSource for DiffPage {
    fn key(&self) -> String {
        "semantic-diff".into()
    }
    fn revision_token(&self) -> String {
        cursor_revision(
            self.base.commit(),
            self.revision,
            &self.component_source_hash,
        )
    }
    fn revision_json(&self) -> JsonValue {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.report.entries.len()
    }
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        vec![
            ("base_commit", JsonValue::string(self.base.commit())),
            (
                "component_source_hash",
                JsonValue::string(&self.component_source_hash),
            ),
        ]
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
    let current = cached
        .comparison
        .take()
        .filter(|page| page.base.commit() == head.commit());
    let observe = || working_components(&cached.snapshot).map_err(|_| unavailable());
    let mut observed_sources = None;
    if let Some(page) = current.as_ref() {
        // 페이지를 준비한 뒤 마지막에 작업 파일을 확인해 cache hit에 중복 I/O를 하지 않습니다.
        let page_result = paging::build_page(page, request);
        let checked = observe()?;
        if page.component_source_hash == checked.hash().to_string() {
            if !head.is_current().map_err(|_| unavailable())? {
                return Err(unavailable());
            }
            cached.comparison = current;
            return page_result.map_err(|e| invalid(e.to_string()));
        }
        observed_sources = Some(checked);
    }
    let observed = match observed_sources {
        Some(hash) => hash,
        None => observe()?,
    };
    let source_hash = observed.hash();
    let tag = source_hash.to_string();
    paging::validate_request(
        "semantic-diff",
        &cursor_revision(head.commit(), revision, &tag),
        request,
    )
    .map_err(|e| invalid(e.to_string()))?;
    let base = match current {
        Some(page) => page.base,
        None => HeadDiffBase::open(&head, &cached.snapshot).map_err(|_| unavailable())?,
    };
    let (report, actual_hash) = base
        .compare(&cached.snapshot, &observed)
        .map_err(|_| unavailable())?;
    if actual_hash != source_hash {
        return Err(unavailable());
    }
    let page = DiffPage {
        base,
        revision,
        component_source_hash: tag,
        report,
    };
    let body = paging::build_page(&page, request).map_err(|e| invalid(e.to_string()))?;
    if observe()?.hash() != source_hash || !head.is_current().map_err(|_| unavailable())? {
        return Err(unavailable());
    }
    cached.comparison = Some(page);
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
    let mut fields = vec![
        ("kind", JsonValue::string(entry.kind)),
        ("document", opt_id(entry.document)),
        ("object", id(entry.object)),
        (
            "object_kind",
            JsonValue::opt_string(entry.object_kind.as_deref()),
        ),
        ("path", JsonValue::opt_string(entry.path.as_deref())),
        ("key", JsonValue::opt_string(entry.key.as_deref())),
        ("old", opt_value(entry.old.as_ref())),
        ("new", opt_value(entry.new.as_ref())),
        ("from", diff_position(entry.from.as_ref())),
        ("to", diff_position(entry.to.as_ref())),
    ];
    if let Some(component) = entry.component {
        fields.push(("component", id(component)));
    }
    JsonValue::object(fields)
}
