//! semantic diff 요약 artifact (XXIV.3.2).
//!
//! CI가 보관·공유하는 artifact용 요약입니다. 기본은 **redaction**입니다. 프로젝트 사용자가 쓴 텍스트,
//! 이름, 경로가 artifact에 남지 않게 다음을 지웁니다.
//!
//! | 값 | 기본(redact) | `Redaction::None` |
//! |---|---|---|
//! | 문자열 값(표시 이름, 텍스트 등 `text`) | 종류만 남김 | 값 포함 |
//! | 타입드 값(`typed`, 문자열·타이포그래피 등을 담을 수 있음) | 종류만 남김 | 값 포함 |
//! | 불투명 bytes(`bytes`, 텍스트 payload·path command 등) | 종류만 남김 | 값 포함 |
//! | bool·정수·float·StableId | 포함 | 포함 |
//! | 속성 경로 표기·변경 종류·객체 종류 | 포함 | 포함 |
//!
//! 프로젝트 이름·경로·repository URL은 어느 모드에서도 요약에 넣지 않습니다. 순서는 semantic diff의
//! canonical 순서(XIV.3.8)이므로 같은 입력은 같은 bytes가 됩니다.

use std::collections::BTreeMap;

use crate::core_types::ids::StableId;
use crate::diff::{DiffEntry, DiffPosition, DiffReport};
use crate::query::OpsValue;
use crate::revision::ProjectRevision;

/// artifact bytes의 content hash 표기(`sha256:<64 hex>`, ADR-0004). artifact 무결성 확인용입니다.
pub fn artifact_hash(bytes: &[u8]) -> String {
    stellar_storage::hash::ContentHash::compute(bytes).to_string()
}

/// redaction 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redaction {
    /// 프로젝트 텍스트·이름이 될 수 있는 값을 지웁니다(기본).
    ProjectContent,
    /// 지우지 않습니다(명시적 선택).
    None,
}

/// 요약 값: 종류와(지우지 않았으면) 값.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryValue {
    /// 값 종류 이름([`OpsValue::type_name`]).
    pub type_name: &'static str,
    /// 값. redaction으로 지웠으면 `None`.
    pub value: Option<OpsValue>,
}

impl SummaryValue {
    fn of(value: &OpsValue, redaction: Redaction) -> SummaryValue {
        let keep = match redaction {
            Redaction::None => true,
            Redaction::ProjectContent => !matches!(
                value,
                OpsValue::Text(_) | OpsValue::Typed { .. } | OpsValue::Bytes(_)
            ),
        };
        SummaryValue {
            type_name: value.type_name(),
            value: keep.then(|| value.clone()),
        }
    }

    /// redaction으로 값이 지워졌는지.
    pub fn is_redacted(&self) -> bool {
        self.value.is_none()
    }
}

/// 요약의 변경 한 건.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryEntry {
    /// 변경 종류 이름.
    pub kind: &'static str,
    /// 소유 문서.
    pub document: Option<StableId>,
    /// 소유 컴포넌트.
    pub component: Option<StableId>,
    /// 대상 객체.
    pub object: StableId,
    /// 객체 종류(추가·삭제).
    pub object_kind: Option<String>,
    /// 속성 경로 표기(속성 변경).
    pub path: Option<String>,
    /// 이전 값.
    pub old: Option<SummaryValue>,
    /// 이후 값.
    pub new: Option<SummaryValue>,
    /// 이전 위치.
    pub from: Option<DiffPosition>,
    /// 이후 위치.
    pub to: Option<DiffPosition>,
}

/// semantic diff 요약.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffSummary {
    /// 적용한 redaction.
    pub redaction: Redaction,
    /// base revision.
    pub base_revision: ProjectRevision,
    /// target revision.
    pub target_revision: ProjectRevision,
    /// 변경 종류별 개수(이름 순).
    pub by_kind: BTreeMap<&'static str, usize>,
    /// 문서별 개수(StableId 순, 프로젝트 범위는 제외).
    pub by_document: BTreeMap<StableId, usize>,
    /// 변경(canonical 순서).
    pub entries: Vec<SummaryEntry>,
    /// 지운 값 수.
    pub redacted_values: usize,
}

fn entry(change: &DiffEntry, redaction: Redaction) -> SummaryEntry {
    SummaryEntry {
        kind: change.kind,
        document: change.document,
        component: change.component,
        object: change.object,
        object_kind: change.object_kind.clone(),
        path: change.path.clone(),
        old: change.old.as_ref().map(|v| SummaryValue::of(v, redaction)),
        new: change.new.as_ref().map(|v| SummaryValue::of(v, redaction)),
        from: change.from.clone(),
        to: change.to.clone(),
    }
}

/// diff를 요약합니다.
pub fn summarize(
    report: &DiffReport,
    base_revision: ProjectRevision,
    target_revision: ProjectRevision,
    redaction: Redaction,
) -> DiffSummary {
    let entries: Vec<SummaryEntry> = report.entries.iter().map(|c| entry(c, redaction)).collect();
    let mut by_document = BTreeMap::new();
    for change in &report.entries {
        if let Some(document) = change.document {
            *by_document.entry(document).or_default() += 1;
        }
    }
    let redacted_values = entries
        .iter()
        .flat_map(|e| [e.old.as_ref(), e.new.as_ref()])
        .flatten()
        .filter(|v| v.is_redacted())
        .count();
    DiffSummary {
        redaction,
        base_revision,
        target_revision,
        by_kind: report.counts(),
        by_document,
        entries,
        redacted_values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff_records;
    use crate::test_support::random_valid_document;

    #[test]
    fn text_values_are_redacted_by_default_and_kept_on_request() {
        let model = random_valid_document(5);
        let records = model.to_records().unwrap();
        let mut changed = records.clone();
        let node = changed.nodes.first_mut().unwrap();
        node.display_name = "SECRET-NAME".into();
        node.visible = !node.visible;
        let report = diff_records(&[&records], &[&changed]).unwrap();
        let revision = ProjectRevision::Unsaved;

        let redacted = summarize(&report, revision, revision, Redaction::ProjectContent);
        assert_eq!(redacted.entries.len(), 2);
        assert_eq!(redacted.redacted_values, 2, "이전·이후 표시 이름");
        let text = format!("{redacted:?}");
        assert!(!text.contains("SECRET-NAME"), "{text}");
        // bool 값은 남습니다.
        assert!(redacted.entries.iter().any(|e| e
            .new
            .as_ref()
            .and_then(|v| v.value.as_ref())
            .is_some()));

        let plain = summarize(&report, revision, revision, Redaction::None);
        assert_eq!(plain.redacted_values, 0);
        assert!(format!("{plain:?}").contains("SECRET-NAME"));
        assert_eq!(plain.by_document.values().sum::<usize>(), 2);
    }
}
