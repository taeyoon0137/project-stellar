use super::{count, id, invalid, revision, CachedRead};
use crate::catalog::paging::{self, PageSource};
use crate::json::JsonValue;
use crate::jsonrpc::RpcError;
use stellar_ops::component_source::{query, ProjectComponents};

impl PageSource for ProjectComponents {
    fn key(&self) -> String {
        "components".into()
    }
    fn revision_token(&self) -> String {
        format!("{}-{}", self.revision, self.source_hash)
    }
    fn revision_json(&self) -> JsonValue {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.entries.len()
    }
    fn items_field(&self) -> &'static str {
        "entries"
    }
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        vec![("chunk_count", count(self.chunk_count))]
    }
    fn item(&self, index: usize) -> JsonValue {
        let entry = &self.entries[index];
        // 원본 YAML과 같은 한 줄 색 표현입니다. 채우기가 없으면 null입니다.
        let background = entry.background.map_or(JsonValue::Null, |color| {
            JsonValue::string(stellar_ops::component_source::color_source_value(color))
        });
        JsonValue::object(vec![
            ("id", id(entry.id)),
            ("root", id(entry.root)),
            ("node_count", count(entry.node_count)),
            ("name", JsonValue::string(&entry.name)),
            ("width", JsonValue::string(entry.size.width().to_string())),
            ("height", JsonValue::string(entry.size.height().to_string())),
            ("background", background),
            ("source_hash", JsonValue::string(entry.hash.to_string())),
        ])
    }
}

pub(super) fn read(cached: &mut CachedRead, uri: &str) -> Result<JsonValue, RpcError> {
    let (_, request) = paging::parse_page_query(uri).map_err(|e| invalid(e.to_string()))?;
    let unavailable = || RpcError::new(-32000, "프로젝트의 컴포넌트 원본을 읽을 수 없습니다");
    // generation을 바꾸지 않는 외부 편집도 매 요청 확인합니다. 파일명이 바뀌거나
    // bytes가 달라진 경우에만 YAML을 다시 해석합니다.
    if let Some(set) = &cached.components {
        let observed = stellar_ops::compare::working_component_hash(&cached.snapshot)
            .map_err(|_| unavailable())?;
        if set.source_hash == observed {
            return paging::build_page(set, &request).map_err(|e| invalid(e.to_string()));
        }
    }
    let sources = query(&cached.snapshot).map_err(|_| unavailable())?;
    let body = paging::build_page(&sources, &request).map_err(|e| invalid(e.to_string()))?;
    cached.components = Some(sources);
    Ok(body)
}

impl PageSource for stellar_ops::component_source::ComponentDetail {
    fn key(&self) -> String {
        format!("component-{}-{}", self.project, self.component)
    }
    fn revision_token(&self) -> String {
        format!("{}-{}", self.revision, self.source_hash)
    }
    fn revision_json(&self) -> JsonValue {
        revision(self.revision)
    }
    fn total(&self) -> usize {
        self.nodes.len()
    }
    fn items_field(&self) -> &'static str {
        "nodes"
    }
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        vec![
            ("component", id(self.component)),
            ("root", id(self.root)),
            ("name", JsonValue::string(&self.name)),
            (
                "source_hash",
                JsonValue::string(self.source_hash.to_string()),
            ),
        ]
    }
    fn item(&self, index: usize) -> JsonValue {
        JsonValue::from(&self.nodes[index])
    }
}

pub(super) fn read_detail(
    cached: &mut CachedRead,
    uri: &str,
    component: stellar_ops::core_types::ids::StableId,
) -> Result<JsonValue, RpcError> {
    let (_, request) = paging::parse_page_query(uri).map_err(|e| invalid(e.to_string()))?;
    let unavailable = || RpcError::new(-32000, "프로젝트의 컴포넌트 원본을 읽을 수 없습니다");
    if let Some(detail) = cached
        .component
        .as_ref()
        .filter(|detail| detail.component == component)
    {
        let observed = stellar_ops::compare::working_component_hash(&cached.snapshot)
            .map_err(|_| unavailable())?;
        if detail.library_hash == observed {
            return paging::build_page(detail, &request).map_err(|e| invalid(e.to_string()));
        }
    }
    let detail = stellar_ops::component_source::detail(&cached.snapshot, component)
        .map_err(|_| unavailable())?
        .ok_or_else(|| invalid("승인된 프로젝트에 컴포넌트가 없습니다"))?;
    let body = paging::build_page(&detail, &request).map_err(|e| invalid(e.to_string()))?;
    cached.component = Some(detail);
    Ok(body)
}
