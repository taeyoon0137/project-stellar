//! resource 응답의 revision과 page cursor (XIX.2.2).
//!
//! 큰 resource(문서 트리)는 한 응답에 전부 보내지 않고 page로 나눕니다. 모든 page 응답 내용에는
//! 그 내용을 만든 revision과 다음 page cursor가 있습니다.
//!
//! - **page 크기**: 기본 [`DEFAULT_PAGE_ITEMS`], 상한 [`MAX_PAGE_ITEMS`]개 item. 그와 별도로 page 내용
//!   JSON이 [`MAX_PAGE_BYTES`]를 넘지 않게 item을 덜 담습니다. 그래서 item이 커도 MCP 메시지 하나가
//!   브리지 메시지 상한([`crate::server::MAX_MESSAGE_BYTES`], 1 MiB)을 넘지 않습니다. 한 item만으로
//!   상한을 넘으면 [`PageError::ItemTooLarge`]입니다.
//! - **cursor**: MCP 명세 2025-11-25 "Pagination"대로 client에게는 불투명한 문자열입니다. 내부 표기는
//!   `p1.<offset>.<resource key>.<revision>`이며, 다른 resource나 다른 revision(그 사이 저장)에서 만든
//!   cursor는 거부합니다. 그 경우 client는 처음 page부터 다시 읽어야 합니다.
//! - **결과 형태**: `resources/read` 결과 `{"contents":[{"uri","mimeType","text"}]}`이고 `text`가 내용
//!   JSON입니다(MCP Resources "Text Content").

use std::fmt;

use crate::json::{self, JsonNumber, JsonValue};

/// page item 기본 개수.
pub const DEFAULT_PAGE_ITEMS: usize = 500;
/// page item 상한.
pub const MAX_PAGE_ITEMS: usize = 2000;
/// page 내용 JSON 최대 byte 수. tool 결과는 같은 내용을 `structuredContent`와 직렬화 text block에 두 번 싣으므로
/// (XIX.9.6) MCP 메시지 상한 1 MiB 안에 escape 여유까지 들어가도록 3/8로 둡니다.
pub const MAX_PAGE_BYTES: usize = 384 * 1024;

/// page로 나눌 수 있는 resource 내용.
pub trait PageSource {
    /// resource key(문서 id 등). cursor가 이 값에 묶입니다.
    fn key(&self) -> String;
    /// 내용을 만든 revision 표기(`g0`, `g<n>:<hash>`).
    fn revision_token(&self) -> String;
    /// 내용의 `revision` object.
    fn revision_json(&self) -> JsonValue;
    /// 전체 item 수.
    fn total(&self) -> usize;
    /// `index` item(0..total).
    fn item(&self, index: usize) -> JsonValue;
    /// 기존 resource의 목록 필드 이름을 유지합니다.
    fn items_field(&self) -> &'static str {
        "items"
    }
    /// `revision` 다음에 넣을 resource별 고정 field(예: `document`).
    fn header(&self) -> Vec<(&'static str, JsonValue)> {
        Vec::new()
    }
}

/// page 요청.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageRequest {
    /// 이전 응답의 `next_cursor`(첫 page면 `None`).
    pub cursor: Option<String>,
    /// 요청 item 수(`None`이면 기본값).
    pub limit: Option<usize>,
}

/// page 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// limit이 1..=[`MAX_PAGE_ITEMS`] 밖입니다.
    InvalidLimit,
    /// cursor 형식이 틀렸습니다.
    InvalidCursor,
    /// 다른 resource의 cursor입니다.
    ForeignCursor,
    /// cursor를 만든 뒤 revision이 바뀌었습니다.
    StaleCursor,
    /// cursor offset이 범위를 넘습니다.
    CursorOutOfRange,
    /// item 하나가 page byte 상한을 넘습니다.
    ItemTooLarge {
        /// item 위치.
        index: usize,
    },
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageError::InvalidLimit => {
                write!(f, "limit은 1..={MAX_PAGE_ITEMS} 범위여야 합니다")
            }
            PageError::InvalidCursor => f.write_str("cursor 형식이 올바르지 않습니다"),
            PageError::ForeignCursor => f.write_str("다른 resource의 cursor입니다"),
            PageError::StaleCursor => {
                f.write_str("cursor를 만든 뒤 revision이 바뀌었습니다. 첫 page부터 다시 읽으세요")
            }
            PageError::CursorOutOfRange => f.write_str("cursor가 범위를 넘습니다"),
            PageError::ItemTooLarge { index } => {
                write!(
                    f,
                    "item {index}이 page 크기 상한({MAX_PAGE_BYTES} bytes)을 넘습니다"
                )
            }
        }
    }
}

impl std::error::Error for PageError {}

/// 해석한 cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cursor {
    offset: usize,
    key: String,
    revision: String,
}

fn encode_cursor(offset: usize, key: &str, revision: &str) -> String {
    format!("p1.{offset}.{key}.{revision}")
}

fn decode_cursor(text: &str) -> Result<Cursor, PageError> {
    let rest = text.strip_prefix("p1.").ok_or(PageError::InvalidCursor)?;
    let mut parts = rest.splitn(3, '.');
    let offset = parts.next().ok_or(PageError::InvalidCursor)?;
    let key = parts.next().ok_or(PageError::InvalidCursor)?;
    let revision = parts.next().ok_or(PageError::InvalidCursor)?;
    let canonical = !offset.is_empty()
        && offset.bytes().all(|b| b.is_ascii_digit())
        && (offset == "0" || !offset.starts_with('0'));
    if !canonical || key.is_empty() || revision.is_empty() {
        return Err(PageError::InvalidCursor);
    }
    Ok(Cursor {
        offset: offset.parse().map_err(|_| PageError::InvalidCursor)?,
        key: key.to_string(),
        revision: revision.to_string(),
    })
}

fn count(n: usize) -> JsonValue {
    JsonValue::Number(JsonNumber::from_u64(u64::try_from(n).unwrap_or(u64::MAX)))
}

/// 내용 로드 전에도 가능한 limit·resource·revision 검사. 반환값은 (offset, limit)입니다.
pub fn validate_request(
    key: &str,
    revision: &str,
    request: &PageRequest,
) -> Result<(usize, usize), PageError> {
    let limit = request.limit.unwrap_or(DEFAULT_PAGE_ITEMS);
    if limit == 0 || limit > MAX_PAGE_ITEMS {
        return Err(PageError::InvalidLimit);
    }
    let offset = match &request.cursor {
        None => 0,
        Some(text) => {
            let cursor = decode_cursor(text)?;
            if cursor.key != key {
                return Err(PageError::ForeignCursor);
            }
            if cursor.revision != revision {
                return Err(PageError::StaleCursor);
            }
            if cursor.offset == 0 {
                return Err(PageError::CursorOutOfRange);
            }
            cursor.offset
        }
    };
    Ok((offset, limit))
}

/// page 하나의 내용 JSON을 만듭니다.
pub fn build_page(source: &dyn PageSource, request: &PageRequest) -> Result<JsonValue, PageError> {
    let key = source.key();
    let revision = source.revision_token();
    let (offset, limit) = validate_request(&key, &revision, request)?;
    let total = source.total();
    if request.cursor.is_some() && offset >= total {
        return Err(PageError::CursorOutOfRange);
    }
    let end_by_count = offset.saturating_add(limit).min(total);
    // header 크기 + item 크기 합이 상한을 넘지 않게 담습니다(쉼표 1 byte 포함 추정, 아래에서 재확인).
    let mut fields = vec![("revision", source.revision_json())];
    fields.extend(source.header());
    let skeleton = {
        let mut f = fields.clone();
        f.extend([
            ("total", count(total)),
            ("offset", count(offset)),
            ("limit", count(limit)),
            (source.items_field(), JsonValue::Array(Vec::new())),
            (
                "next_cursor",
                JsonValue::string(encode_cursor(total, &key, &revision)),
            ),
        ]);
        json::write(&JsonValue::object(f)).len()
    };
    let mut items = Vec::new();
    let mut used = skeleton;
    let mut index = offset;
    while index < end_by_count {
        let item = source.item(index);
        let size = json::write(&item).len() + usize::from(!items.is_empty());
        if used + size > MAX_PAGE_BYTES {
            if items.is_empty() {
                return Err(PageError::ItemTooLarge { index });
            }
            break;
        }
        used += size;
        items.push(item);
        index += 1;
    }
    let next = (index < total).then(|| encode_cursor(index, &key, &revision));
    fields.extend([
        ("total", count(total)),
        ("offset", count(offset)),
        ("limit", count(limit)),
        (source.items_field(), JsonValue::Array(items)),
        (
            "next_cursor",
            next.map_or(JsonValue::Null, JsonValue::string),
        ),
    ]);
    Ok(JsonValue::object(fields))
}

/// `resources/read` 결과: 내용 JSON 한 개를 text content로 감쌉니다.
pub fn read_result(uri: &str, content: &JsonValue) -> JsonValue {
    JsonValue::object(vec![(
        "contents",
        JsonValue::Array(vec![JsonValue::object(vec![
            ("uri", JsonValue::string(uri)),
            ("mimeType", JsonValue::string(super::resources::MIME_JSON)),
            ("text", JsonValue::string(json::write(content))),
        ])]),
    )])
}

/// page resource URI의 query(`?cursor=..&limit=..`)를 해석합니다. 그 밖의 key·중복은 거부합니다.
pub fn parse_page_query(uri: &str) -> Result<(String, PageRequest), PageError> {
    let (base, query) = match uri.split_once('?') {
        None => return Ok((uri.to_string(), PageRequest::default())),
        Some(pair) => pair,
    };
    let mut request = PageRequest::default();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or(PageError::InvalidCursor)?;
        match key {
            "cursor" if request.cursor.is_none() && !value.is_empty() => {
                request.cursor = Some(value.to_string());
            }
            "limit" if request.limit.is_none() => {
                let canonical = !value.is_empty()
                    && value.bytes().all(|b| b.is_ascii_digit())
                    && !value.starts_with('0');
                if !canonical {
                    return Err(PageError::InvalidLimit);
                }
                request.limit = Some(value.parse().map_err(|_| PageError::InvalidLimit)?);
            }
            "limit" => return Err(PageError::InvalidLimit),
            _ => return Err(PageError::InvalidCursor),
        }
    }
    Ok((base.to_string(), request))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, JsonLimits};
    use crate::server::MAX_MESSAGE_BYTES;

    /// 20만 node 문서 트리를 흉내 내는 page source(깊이 2 tree, 이름 길이 다양).
    struct SyntheticTree {
        total: usize,
        revision: String,
    }

    impl PageSource for SyntheticTree {
        fn key(&self) -> String {
            "01a318c7-1fc1-7334-93f0-7e1b813152c2".to_string()
        }
        fn revision_token(&self) -> String {
            self.revision.clone()
        }
        fn revision_json(&self) -> JsonValue {
            JsonValue::object(vec![("token", JsonValue::string(self.revision.clone()))])
        }
        fn total(&self) -> usize {
            self.total
        }
        fn item(&self, index: usize) -> JsonValue {
            let depth = if index == 0 {
                0
            } else if index % 500 == 1 {
                1
            } else {
                2
            };
            JsonValue::object(vec![
                ("id", JsonValue::string(format!("node-{index:06}"))),
                (
                    "parent",
                    if index == 0 {
                        JsonValue::Null
                    } else {
                        JsonValue::string(format!("node-{:06}", index.saturating_sub(1) / 500))
                    },
                ),
                ("depth", count(depth)),
                ("kind", JsonValue::string("fragment")),
                ("name", JsonValue::string("이름".repeat(index % 40))),
            ])
        }
        fn header(&self) -> Vec<(&'static str, JsonValue)> {
            vec![("document", JsonValue::string(self.key()))]
        }
    }

    fn next_cursor(content: &JsonValue) -> Option<String> {
        content
            .get("next_cursor")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
    }

    #[test]
    fn two_hundred_thousand_nodes_are_paged_with_revision_and_bounded_messages() {
        // 검증 기준(XIX.2.2): 20만 node pagination.
        let source = SyntheticTree {
            total: 200_000,
            revision: "g7:sha256:".to_string() + &"a".repeat(64),
        };
        let mut request = PageRequest::default();
        let mut seen = 0usize;
        let mut pages = 0usize;
        let mut largest = 0usize;
        loop {
            let content = build_page(&source, &request).unwrap();
            pages += 1;
            assert_eq!(
                content
                    .get("revision")
                    .and_then(|r| r.get("token"))
                    .and_then(JsonValue::as_str),
                Some(source.revision.as_str())
            );
            assert_eq!(
                content.get("total").and_then(JsonValue::as_i64),
                Some(200_000)
            );
            assert_eq!(
                content.get("offset").and_then(JsonValue::as_i64),
                Some(seen as i64)
            );
            let items = content.get("items").and_then(JsonValue::as_array).unwrap();
            assert!(!items.is_empty() && items.len() <= DEFAULT_PAGE_ITEMS);
            for (i, item) in items.iter().enumerate() {
                assert_eq!(
                    item.get("id").and_then(JsonValue::as_str),
                    Some(format!("node-{:06}", seen + i).as_str()),
                    "순서·중복·누락 없음"
                );
            }
            seen += items.len();
            // MCP 메시지(resources/read 결과 + JSON-RPC envelope 여유)는 1 MiB 안입니다.
            let message = json::write(&read_result("stellar://documents/x", &content));
            assert!(message.len() + 128 < MAX_MESSAGE_BYTES, "{}", message.len());
            assert!(json::write(&content).len() <= MAX_PAGE_BYTES);
            largest = largest.max(message.len());
            match next_cursor(&content) {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }
        assert_eq!(seen, 200_000);
        assert!(pages >= 200_000 / DEFAULT_PAGE_ITEMS, "{pages}");
        assert!(largest > 0);
    }

    #[test]
    fn byte_budget_shrinks_pages_with_large_items() {
        struct Fat;
        impl PageSource for Fat {
            fn key(&self) -> String {
                "k".into()
            }
            fn revision_token(&self) -> String {
                "g1:x".into()
            }
            fn revision_json(&self) -> JsonValue {
                JsonValue::Null
            }
            fn total(&self) -> usize {
                100
            }
            fn item(&self, _: usize) -> JsonValue {
                JsonValue::string("x".repeat(100 * 1024))
            }
        }
        let content = build_page(
            &Fat,
            &PageRequest {
                cursor: None,
                limit: Some(100),
            },
        )
        .unwrap();
        let items = content.get("items").and_then(JsonValue::as_array).unwrap();
        assert!(items.len() < 100 && items.len() >= 3, "{}", items.len());
        assert!(json::write(&content).len() <= MAX_PAGE_BYTES);
        assert!(next_cursor(&content)
            .unwrap()
            .starts_with(&format!("p1.{}.", items.len())));
    }

    #[test]
    fn foreign_stale_malformed_and_out_of_range_cursors_are_rejected() {
        let source = SyntheticTree {
            total: 10,
            revision: "g1:a".into(),
        };
        let first = build_page(
            &source,
            &PageRequest {
                cursor: None,
                limit: Some(3),
            },
        )
        .unwrap();
        let cursor = next_cursor(&first).unwrap();
        let changed = SyntheticTree {
            total: 10,
            revision: "g2:b".into(),
        };
        let with = |c: &str| PageRequest {
            cursor: Some(c.to_string()),
            limit: None,
        };
        assert_eq!(
            build_page(&changed, &with(&cursor)),
            Err(PageError::StaleCursor)
        );
        let foreign = cursor.replace("01a318c7", "ffffffff");
        assert_eq!(
            build_page(&source, &with(&foreign)),
            Err(PageError::ForeignCursor)
        );
        for bad in ["", "p2.3.k.g1", "p1.03.k.g1", "p1.x.k.g1", "p1.3..g1"] {
            assert_eq!(
                build_page(&source, &with(bad)),
                Err(PageError::InvalidCursor),
                "{bad}"
            );
        }
        let beyond = encode_cursor(10, &source.key(), "g1:a");
        assert_eq!(
            build_page(&source, &with(&beyond)),
            Err(PageError::CursorOutOfRange)
        );
        for limit in [0, MAX_PAGE_ITEMS + 1] {
            assert_eq!(
                build_page(
                    &source,
                    &PageRequest {
                        cursor: None,
                        limit: Some(limit)
                    }
                ),
                Err(PageError::InvalidLimit)
            );
        }
    }

    #[test]
    fn page_query_is_parsed_strictly() {
        assert_eq!(
            parse_page_query("stellar://documents/d?cursor=p1.3.d.g1&limit=50").unwrap(),
            (
                "stellar://documents/d".to_string(),
                PageRequest {
                    cursor: Some("p1.3.d.g1".into()),
                    limit: Some(50)
                }
            )
        );
        assert_eq!(
            parse_page_query("stellar://documents/d").unwrap().1,
            PageRequest::default()
        );
        for bad in [
            "s?limit=0",
            "s?limit=01",
            "s?limit=a",
            "s?limit=1&limit=2",
            "s?other=1",
            "s?cursor",
        ] {
            assert!(parse_page_query(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn read_result_wraps_text_content() {
        let content = JsonValue::object(vec![("revision", JsonValue::Null)]);
        let result = read_result("stellar://project/structure", &content);
        let entry = &result
            .get("contents")
            .and_then(JsonValue::as_array)
            .unwrap()[0];
        assert_eq!(
            entry.get("mimeType").and_then(JsonValue::as_str),
            Some("application/json")
        );
        let text = entry.get("text").and_then(JsonValue::as_str).unwrap();
        assert_eq!(parse(text, JsonLimits::default()).unwrap(), content);
    }
}
