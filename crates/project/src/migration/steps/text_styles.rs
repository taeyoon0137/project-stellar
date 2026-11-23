//! 옛 Text 문서의 기본 style 표 (IX.11.11, ADR-0075 개정 결정 11).
//!
//! Text payload는 내용과 문단·run의 style index만 저장하고, style 정의는 문서 Text style 표
//! (`DocumentTextStyles`, 문서 manifest)에 있습니다. 표 저장 형식이 생기기 전 schema 1 문서는 표가 비어 있어 모든
//! Text가 미해결 style을 가리킵니다. 이 step은 그런 문서에 **기본 style 표**를 만듭니다.
//!
//! - 문서 기본 typography 하나(CSS initial value의 Typography record)를 둡니다. resource id는 문서 id에서 결정론적으로
//!   만듭니다([`default_typography_id`]): 같은 입력은 같은 결과입니다.
//! - Text node가 쓰는 문단 style index마다 기본 `ParagraphStyle`(typography 없음 → 문서 기본 상속, `line-height: normal`),
//!   run style index마다 기본 `TextStyle`(typography·색·letter-spacing 없음 → 상속)을 둡니다.
//! - 표가 이미 있는 문서는 바꾸지 않습니다. 표에 없는 index는 다른 style로 대체하지 않는다는 규칙(`resolve`)을 그대로
//!   두고 보고합니다.
//! - Text payload bytes는 바꾸지 않습니다(index는 그대로).
//!
//! 1→2 묶음 step으로 [`super::super::PRODUCT_STEPS`]에 등록합니다(IX.11.9·IX.11.10과 같은 시점).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::text::{ParagraphStyle, TextStyle};
use stellar_document::text_styles::DocumentTextStyles;

/// 변환 보고서.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextStylesReport {
    /// 기본 표를 만들었으면 그 문서 기본 typography id.
    pub created: Option<StableId>,
    /// 기본 문단 style을 둔 index.
    pub paragraphs: Vec<u32>,
    /// 기본 문자 style을 둔 index.
    pub runs: Vec<u32>,
    /// 표가 이미 있는데 표에 없는 index를 가리키는 Text(node, 문단 index 또는 run index 표기).
    pub unresolved: Vec<(StableId, String)>,
}

/// 문서 id에서 만든 기본 typography resource id(UUIDv7 형식, 결정론적).
pub fn default_typography_id(document: StableId) -> StableId {
    let bytes = document.to_bytes();
    let tail = u64::from_be_bytes(bytes[8..16].try_into().expect("8 bytes"));
    // 문서 id와 같은 timestamp, 고정 rand_a, 문서 id 꼬리와 상수의 xor. 같은 문서 안 다른 id와 겹치지 않게 상수를 섞습니다.
    StableId::new_v7(document.timestamp_ms(), 0x0575, tail ^ 0x5459_504f_4752_4150)
}

/// 표가 비어 있는 문서에 기본 style 표를 만듭니다.
pub fn default_text_styles(mut records: DocumentRecords) -> (DocumentRecords, TextStylesReport) {
    let mut report = TextStylesReport::default();
    let mut paragraphs = BTreeSet::new();
    let mut runs = BTreeSet::new();
    let mut users: Vec<(StableId, BTreeSet<u32>, BTreeSet<u32>)> = Vec::new();
    for node in &records.nodes {
        if let NodePayload::Text(text) = &node.payload {
            let p: BTreeSet<u32> = text.paragraphs().iter().map(|p| p.style).collect();
            let r: BTreeSet<u32> = text.style_runs().iter().map(|r| r.style).collect();
            paragraphs.extend(&p);
            runs.extend(&r);
            users.push((node.id, p, r));
        }
    }
    let styles = &mut records.definition.text_styles;
    if !styles.is_empty() {
        for (node, p, r) in users {
            for index in p.iter().filter(|i| !styles.paragraphs.contains_key(i)) {
                report.unresolved.push((node, format!("paragraph {index}")));
            }
            for index in r.iter().filter(|i| !styles.runs.contains_key(i)) {
                report.unresolved.push((node, format!("run {index}")));
            }
        }
        return (records, report);
    }
    if paragraphs.is_empty() && runs.is_empty() {
        // Text가 없는 문서는 표가 필요 없습니다.
        return (records, report);
    }
    let resource = default_typography_id(records.definition.id);
    let mut table = DocumentTextStyles::initial(resource);
    table.paragraphs.clear();
    table.runs.clear();
    for index in &paragraphs {
        table.paragraphs.insert(*index, ParagraphStyle::default());
    }
    for index in &runs {
        table
            .runs
            .insert(*index, TextStyle::new(None, None, vec![], vec![]));
    }
    *styles = table;
    report.created = Some(resource);
    report.paragraphs = paragraphs.into_iter().collect();
    report.runs = runs.into_iter().collect();
    (records, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::text::UserText;
    use stellar_core_types::typography::Typography;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::frame::FramePayload;
    use stellar_document::records::NodeRecord;
    use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};
    use stellar_document::validate::validate_document;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(1_700_000_000_000 + n, 0x0abc, n)
    }

    fn node(id: u64, parent: Option<u64>, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![id as u8]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload,
        }
    }

    fn text(content: &str, paragraphs: &[u32], runs: &[(u32, u32, u32)]) -> NodePayload {
        NodePayload::Text(
            TextPayload::new(
                UserText::new(content),
                paragraphs.iter().map(|s| Paragraph { style: *s }).collect(),
                runs.iter()
                    .map(|(a, b, s)| StyleRun {
                        range: TextRange::new(*a, *b).unwrap(),
                        style: *s,
                    })
                    .collect(),
            )
            .unwrap(),
        )
    }

    fn records() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut page = PageDefinition::new(sid(101), String::new());
        page.add_view(ViewDefinition::new(sid(102), String::new(), sid(1)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        records.nodes = vec![
            node(1, None, NodePayload::Frame(FramePayload::default())),
            node(2, Some(1), text("ab\ncd", &[0, 3], &[(0, 2, 5), (3, 5, 1)])),
            node(3, Some(1), text("x", &[3], &[])),
        ];
        records
    }

    /// 기본 표 golden(STTS v1 canonical bytes, hex). 형식이나 기본값이 바뀌면 이 golden이 바뀝니다.
    const GOLDEN: &str = include_str!("text_styles.golden");

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn old_text_documents_get_the_default_style_table_golden() {
        // 검증 기준(IX.11.11): 옛 Text payload migration golden.
        let before = records();
        assert!(before.definition.text_styles.is_empty());
        let (after, report) = default_text_styles(before.clone());
        validate_document(&after).unwrap();
        let resource = default_typography_id(sid(100));
        assert_eq!(report.created, Some(resource));
        assert_eq!(report.paragraphs, [0, 3]);
        assert_eq!(report.runs, [1, 5]);
        let styles = &after.definition.text_styles;
        // 모든 문단·run이 CSS initial typography로 풀립니다.
        for (p, r) in [(0, Some(5)), (3, Some(1)), (3, None)] {
            let resolved = styles.resolve(p, r).unwrap();
            assert_eq!(resolved.typography.record().unwrap(), Typography::default());
        }
        assert!(styles.diagnostics().is_empty());
        // Text payload는 그대로입니다.
        assert_eq!(before.nodes, after.nodes);
        let actual = hex(&styles.encode());
        if std::env::var_os("STELLAR_UPDATE_GOLDEN").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/migration/steps/text_styles.golden"
                ),
                format!("{actual}\n"),
            )
            .unwrap();
        }
        assert_eq!(actual, GOLDEN.trim());
        // 같은 입력은 같은 결과입니다.
        assert_eq!(default_text_styles(before).0, after);
    }

    #[test]
    fn existing_tables_are_kept_and_missing_indexes_reported() {
        let mut records = records();
        records.definition.text_styles = DocumentTextStyles::initial(sid(200));
        let (after, report) = default_text_styles(records.clone());
        assert_eq!(after, records);
        assert_eq!(report.created, None);
        assert_eq!(
            report.unresolved,
            vec![
                (sid(2), "paragraph 3".to_string()),
                (sid(2), "run 1".to_string()),
                (sid(2), "run 5".to_string()),
                (sid(3), "paragraph 3".to_string()),
            ]
        );
        // Text가 없는 문서는 표를 만들지 않습니다.
        let mut empty = records;
        empty.nodes.truncate(1);
        empty.definition.text_styles = DocumentTextStyles::default();
        assert_eq!(default_text_styles(empty).1, TextStylesReport::default());
    }
}
