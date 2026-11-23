//! 창 문서 하나의 3-way reconciliation (XIII.3.4, ADR-0110 결정 1).
//!
//! base·ours·theirs 문서 레코드를 각각 semantic snapshot으로 올리고([`crate::diff::snapshot_of`]), 병합 판정은
//! [`reconcile`] 하나로 냅니다. 쓸 수 있는 결과는 문서 레코드로 되돌립니다(`document_records`).
//!
//! **무손실 조건**: 세 입력 모두 "레코드 → snapshot → 레코드" 왕복이 같아야 합니다. snapshot에 없는 필드가 있는
//! 입력을 병합하면 결과에서 그 필드가 조용히 사라지므로, 왕복이 다르면 병합하지 않고 [`DocumentMergeError::Lossy`]
//! 입니다. 병합 결과도 다시 snapshot으로 올린 값이 병합 snapshot과 같아야 합니다.
//!
//! 이 모듈은 파일을 읽거나 쓰지 않습니다. 결과 레코드를 창 history에 반영하는 일은 runtime이 합니다.

use std::fmt;

use stellar_document::records::DocumentRecords;
use stellar_semantic_diff::records::document_records;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::apply::{reconcile, Reconciliation};
use stellar_semantic_merge::model::MergeInput;

use crate::compare::ConflictEntry;
use crate::core_types::ids::StableId;
use crate::diff::snapshot_of;

/// 창 문서 병합 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentMerge {
    /// 쓸 수 있는 결과 레코드(canonical 정렬).
    Merged {
        /// 판정 이름(`keep-ours`·`take-theirs`·`merged`).
        decision: &'static str,
        /// 결과 문서 레코드.
        records: Box<DocumentRecords>,
    },
    /// 겹치는 변경이 있거나 결과가 쓰기 조건을 통과하지 못했습니다. 자동 적용하지 않습니다.
    Conflicted {
        /// 겹친 변경(엔진 순서). 결과 위반만 있으면 비어 있습니다.
        conflicts: Vec<ConflictEntry>,
        /// 결과가 새로 만든 snapshot 위반 수.
        violations: usize,
    },
}

/// 병합하지 못한 이유. 어느 경우도 입력을 바꾸지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentMergeError {
    /// 세 입력의 문서 id가 다릅니다.
    DocumentMismatch {
        /// base 문서.
        base: StableId,
        /// 다른 입력 이름.
        side: &'static str,
        /// 그 입력의 문서.
        found: StableId,
    },
    /// 입력(`side`) 또는 결과(`result`)를 snapshot으로 무손실 왕복할 수 없습니다.
    Lossy {
        /// `base`·`ours`·`theirs`·`result`.
        side: &'static str,
    },
    /// snapshot 투영·역투영 실패.
    Projection(String),
    /// 병합 엔진 실패.
    Merge(String),
}

impl fmt::Display for DocumentMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentMergeError::DocumentMismatch { base, side, found } => {
                write!(f, "{side} 문서 {found}가 base 문서 {base}와 다릅니다")
            }
            DocumentMergeError::Lossy { side } => write!(
                f,
                "{side} 문서를 semantic snapshot으로 무손실 왕복할 수 없어 병합하지 않습니다"
            ),
            DocumentMergeError::Projection(e) => write!(f, "semantic 투영 실패: {e}"),
            DocumentMergeError::Merge(e) => write!(f, "semantic merge 실패: {e}"),
        }
    }
}

impl std::error::Error for DocumentMergeError {}

/// `records`의 문서 snapshot. 왕복이 다르면 [`DocumentMergeError::Lossy`]입니다.
fn lossless_snapshot(
    records: &DocumentRecords,
    side: &'static str,
) -> Result<SemanticSnapshot, DocumentMergeError> {
    let mut sorted = records.clone();
    sorted.sort_canonical();
    let snapshot =
        snapshot_of(&[&sorted]).map_err(|e| DocumentMergeError::Projection(e.0.clone()))?;
    let back = document_records(&snapshot, sorted.definition.id)
        .map_err(|e| DocumentMergeError::Projection(e.to_string()))?;
    if back.records != sorted {
        return Err(DocumentMergeError::Lossy { side });
    }
    Ok(snapshot)
}

/// 병합 snapshot에서 문서 `id`의 레코드를 되돌립니다. 다시 올린 snapshot이 `snapshot`의 그 문서 부분과 같아야
/// 합니다(호출자가 문서 집합 전체를 비교할 때는 [`project_documents`]).
pub(crate) fn project_document(
    snapshot: &SemanticSnapshot,
    id: StableId,
) -> Result<DocumentRecords, DocumentMergeError> {
    document_records(snapshot, id)
        .map(|projection| projection.records)
        .map_err(|e| DocumentMergeError::Projection(e.to_string()))
}

/// 병합 snapshot의 모든 문서를 레코드로 되돌리고, 다시 올린 snapshot이 `snapshot`과 정확히 같은지 확인합니다.
/// 문서 밖 객체가 있거나 역투영에서 잃는 값이 있으면 [`DocumentMergeError::Lossy`]입니다.
pub(crate) fn project_documents(
    snapshot: &SemanticSnapshot,
) -> Result<Vec<DocumentRecords>, DocumentMergeError> {
    let mut documents = Vec::new();
    for id in stellar_semantic_diff::records::documents(snapshot) {
        documents.push(project_document(snapshot, id)?);
    }
    let actual = snapshot_of(&documents.iter().collect::<Vec<_>>())
        .map_err(|e| DocumentMergeError::Projection(e.0.clone()))?;
    if &actual != snapshot {
        return Err(DocumentMergeError::Lossy { side: "result" });
    }
    Ok(documents)
}

/// base·ours·theirs 문서 하나를 3-way 병합합니다(모듈 문서). 같은 입력이면 같은 결과입니다.
pub fn merge_document(
    base: &DocumentRecords,
    ours: &DocumentRecords,
    theirs: &DocumentRecords,
) -> Result<DocumentMerge, DocumentMergeError> {
    let id = base.definition.id;
    for (side, records) in [("ours", ours), ("theirs", theirs)] {
        if records.definition.id != id {
            return Err(DocumentMergeError::DocumentMismatch {
                base: id,
                side,
                found: records.definition.id,
            });
        }
    }
    let input = MergeInput {
        base: lossless_snapshot(base, "base")?,
        ours: lossless_snapshot(ours, "ours")?,
        theirs: lossless_snapshot(theirs, "theirs")?,
    };
    let decision = reconcile(input).map_err(|e| DocumentMergeError::Merge(e.to_string()))?;
    let records = match &decision {
        Reconciliation::Conflicted(candidate) => {
            return Ok(DocumentMerge::Conflicted {
                conflicts: candidate
                    .conflicts()
                    .iter()
                    .map(ConflictEntry::from)
                    .collect(),
                violations: 0,
            })
        }
        Reconciliation::Blocked { blocked, .. } => {
            return Ok(DocumentMerge::Conflicted {
                conflicts: Vec::new(),
                violations: blocked.violations.len(),
            })
        }
        Reconciliation::KeepOurs(_) => sorted(ours),
        Reconciliation::TakeTheirs(_) => sorted(theirs),
        Reconciliation::Merged(candidate) => {
            let mut documents = project_documents(candidate.snapshot())?;
            match (documents.pop(), documents.is_empty()) {
                (Some(records), true) if records.definition.id == id => records,
                _ => return Err(DocumentMergeError::Lossy { side: "result" }),
            }
        }
    };
    Ok(DocumentMerge::Merged {
        decision: decision.name(),
        records: Box::new(records),
    })
}

fn sorted(records: &DocumentRecords) -> DocumentRecords {
    let mut records = records.clone();
    records.sort_canonical();
    records
}
