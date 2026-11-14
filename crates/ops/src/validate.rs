//! 프로젝트 검증 (XXIV.1.3).
//!
//! [`validate_project`]는 snapshot(XXIV.4.2)의 모든 문서를 읽고 문서 validator(VI.10.1)를 다시
//! 실행합니다. 읽기 전용이며 파일을 쓰지 않습니다.
//!
//! | 심각도 | 진단 |
//! |---|---|
//! | 오류 | 문서 chunk를 읽거나 해석할 수 없음(`document-load`), validator 위반(`document-invalid`) |
//! | 경고 | 미해결 참조(III.2.4, `unresolved-reference`): 문서는 유효하지만 대상이 없음 |

use stellar_document::validate::validate_document;

use crate::core_types::ids::StableId;
use crate::error::OpenError;
use crate::query::property_path_label;
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};

/// 검증 진단 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingSeverity {
    /// 오류(검증 실패).
    Error,
    /// 경고(검증 통과).
    Warning,
}

/// 검증 진단 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 심각도.
    pub severity: FindingSeverity,
    /// 고정 진단 코드.
    pub code: &'static str,
    /// 대상 문서.
    pub document: StableId,
    /// 사람이 읽는 설명(경로 미포함).
    pub message: String,
}

/// 문서 하나의 검증 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentValidation {
    /// 문서.
    pub document: StableId,
    /// node 수(읽지 못했으면 `None`).
    pub node_count: Option<usize>,
    /// 진단.
    pub findings: Vec<Finding>,
}

/// 프로젝트 검증 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// 검증한 revision.
    pub revision: ProjectRevision,
    /// 문서별 결과(snapshot 문서 순).
    pub documents: Vec<DocumentValidation>,
}

impl ValidationReport {
    /// 오류 수.
    pub fn error_count(&self) -> usize {
        self.count(FindingSeverity::Error)
    }

    /// 경고 수.
    pub fn warning_count(&self) -> usize {
        self.count(FindingSeverity::Warning)
    }

    fn count(&self, severity: FindingSeverity) -> usize {
        self.documents
            .iter()
            .flat_map(|d| d.findings.iter())
            .filter(|f| f.severity == severity)
            .count()
    }

    /// 오류가 없는지.
    pub fn is_valid(&self) -> bool {
        self.error_count() == 0
    }
}

/// 열린 snapshot을 검증합니다.
pub fn validate_snapshot(snapshot: &ProjectSnapshot) -> ValidationReport {
    let mut documents = Vec::new();
    for definition in snapshot.documents() {
        let document = definition.id;
        let mut findings = Vec::new();
        let records = match snapshot.document_records(document) {
            Ok(records) => records,
            Err(e) => {
                findings.push(Finding {
                    severity: FindingSeverity::Error,
                    code: "document-load",
                    document,
                    message: e.to_string(),
                });
                documents.push(DocumentValidation {
                    document,
                    node_count: None,
                    findings,
                });
                continue;
            }
        };
        match validate_document(&records) {
            Ok(diagnostics) => {
                for unresolved in &diagnostics.unresolved {
                    findings.push(Finding {
                        severity: FindingSeverity::Warning,
                        code: "unresolved-reference",
                        document,
                        message: format!(
                            "{:?} 참조 {}의 대상이 없습니다(위치 {} {})",
                            unresolved.expected(),
                            unresolved.original_id(),
                            unresolved.location().root(),
                            property_path_label(unresolved.location()),
                        ),
                    });
                }
            }
            Err(error) => {
                for violation in &error.violations {
                    findings.push(Finding {
                        severity: FindingSeverity::Error,
                        code: "document-invalid",
                        document,
                        message: violation.to_string(),
                    });
                }
            }
        }
        documents.push(DocumentValidation {
            document,
            node_count: Some(records.nodes.len()),
            findings,
        });
    }
    ValidationReport {
        revision: snapshot.revision(),
        documents,
    }
}

/// 프로젝트를 열어 검증합니다. 열기 실패는 검증 결과가 아니라 오류입니다.
pub fn validate_project(path: &std::path::Path) -> Result<ValidationReport, OpenError> {
    Ok(validate_snapshot(&open_snapshot(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        file_digest, random_valid_document, save_documents, scaffold_project, TempDir,
    };

    #[test]
    fn valid_corpus_passes_and_writes_nothing() {
        let dir = TempDir::new("validate");
        scaffold_project(&dir.path);
        let documents: Vec<_> = (0..4).map(|i| random_valid_document(300 + i)).collect();
        let refs: Vec<_> = documents.iter().collect();
        save_documents(&dir.path, &refs);
        let before = file_digest(&dir.path);
        let report = validate_project(&dir.path).unwrap();
        assert!(report.is_valid(), "{report:?}");
        assert_eq!(report.documents.len(), 4);
        for doc in &report.documents {
            assert!(doc.node_count.is_some());
        }
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn corrupt_document_chunk_is_reported_as_error() {
        let dir = TempDir::new("validate-corrupt");
        scaffold_project(&dir.path);
        let model = random_valid_document(310);
        save_documents(&dir.path, &[&model]);
        // ViewTree·subtree chunk 하나를 망가뜨립니다(manifest 구조 검증은 존재만 확인).
        let snapshot = open_snapshot(&dir.path).unwrap();
        let pointer = snapshot
            .generation()
            .unwrap()
            .manifest()
            .chunk_pointers
            .iter()
            .find(|p| p.kind == stellar_storage::chunk::kinds::KIND_VIEW_TREE)
            .copied();
        drop(snapshot);
        let pointer = pointer.expect("seed 310은 View를 가진 문서");
        let path = stellar_storage::chunk::path::ChunkObjectPath::for_hash(&pointer.content_hash)
            .to_relative_path();
        let path = dir.path.join(path);
        std::fs::write(&path, b"broken").unwrap();
        let report = validate_project(&dir.path).unwrap();
        assert!(!report.is_valid());
        assert_eq!(report.documents[0].node_count, None);
        assert_eq!(report.documents[0].findings[0].code, "document-load");
    }
}
