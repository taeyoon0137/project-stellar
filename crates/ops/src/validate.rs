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
    let definitions = std::sync::OnceLock::new();
    ValidationReport {
        revision: snapshot.revision(),
        documents: snapshot
            .documents()
            .iter()
            .map(|document| validate_with_definitions(snapshot, document.id, &definitions))
            .collect(),
    }
}

/// 지정한 문서 하나의 chunk와 참조만 검증합니다. 읽기 실패는 같은 문서의 진단이 됩니다.
pub fn validate_snapshot_document(
    snapshot: &ProjectSnapshot,
    document: StableId,
) -> DocumentValidation {
    validate_with_definitions(snapshot, document, &std::sync::OnceLock::new())
}

type DefinitionIds = std::collections::BTreeSet<StableId>;

fn definition_ids(snapshot: &ProjectSnapshot) -> Result<DefinitionIds, String> {
    // 정의의 원본은 design/components의 텍스트 원본 하나입니다(ADR-0059·ADR-0061). 기존
    // component-library chunk는 정의 출처로 인정하지 않습니다.
    let (library, _) =
        crate::component_source::project_library(snapshot).map_err(|e| e.to_string())?;
    Ok(library.iter().map(|e| e.definition().id).collect())
}

fn validate_with_definitions(
    snapshot: &ProjectSnapshot,
    document: StableId,
    definitions: &std::sync::OnceLock<Result<DefinitionIds, String>>,
) -> DocumentValidation {
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
            return DocumentValidation {
                document,
                node_count: None,
                findings,
            };
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
    let instances = records.nodes.iter().filter_map(|n| match &n.payload {
        stellar_document::records::NodePayload::ComponentInstance(i) => {
            Some((n.id, i.component().id()))
        }
        _ => None,
    });
    for (instance, definition) in instances {
        match definitions.get_or_init(|| definition_ids(snapshot)) {
            Ok(ids) if ids.contains(&definition) => (),
            Ok(_) => findings.push(Finding {
                severity: FindingSeverity::Warning,
                code: "missing-component-definition",
                document,
                message: format!(
                    "인스턴스 {instance}가 참조하는 컴포넌트 정의 {definition}가 없습니다"
                ),
            }),
            Err(e) => {
                findings.push(Finding {
                    severity: FindingSeverity::Error,
                    code: "component-library-load",
                    document,
                    message: e.clone(),
                });
                break;
            }
        }
    }
    DocumentValidation {
        document,
        node_count: Some(records.nodes.len()),
        findings,
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
    fn instances_survive_storage_and_diagnose_disappearing_definition_with_no_writes() {
        use crate::core_types::{reference::ComponentRef, typed_value::TypedValue};
        use stellar_document::{
            component::{
                definition::ComponentPropertyId,
                instance::{ComponentInstancePayload, OverrideKey},
            },
            records::{NodePayload, NodeRecord},
        };
        let dir = TempDir::new("validate-instance");
        scaffold_project(&dir.path);
        let components = dir.path.join("design/components");
        std::fs::create_dir(&components).unwrap();
        let source = crate::component_source::create(
            &components,
            "Card".into(),
            StableId::new_v7(900_010, 0, 1),
            StableId::new_v7(900_011, 0, 1),
        )
        .unwrap();
        let mut model = random_valid_document(311);
        let root = model.definition().pages[0].views[0].root_frame;
        let mut original = Vec::new();
        for n in [900_001, 900_002] {
            let id = StableId::new_v7(n, 0, n);
            let mut payload = ComponentInstancePayload::new(ComponentRef::new(source.id));
            payload.set_property_override(
                OverrideKey::direct(ComponentPropertyId::new(StableId::new_v7(123, 0, 123))),
                TypedValue::String("저장된 값".into()),
            );
            let record = NodeRecord {
                id,
                parent: Some(root),
                order_key: model.next_child_order_key(root),
                visible: true,
                locked: false,
                display_name: "Placed".into(),
                payload: NodePayload::ComponentInstance(payload),
            };
            original.push(record.clone());
            model.insert_node(record).unwrap();
        }
        save_documents(&dir.path, &[&model]);
        let open = open_snapshot(&dir.path).unwrap();
        let reopened = open.document_model(model.document_id()).unwrap();
        // 인스턴스를 담은 subtree도 다른 문서 chunk와 같은 payload schema 1입니다.
        assert!(open
            .generation()
            .unwrap()
            .manifest()
            .chunk_pointers
            .iter()
            .all(|p| p.schema_version == 1));
        for node in &original {
            assert_eq!(reopened.record(node.id).as_ref(), Some(node));
        }
        assert_eq!(
            reopened.references().inbound(source.id),
            original.iter().map(|n| n.id).collect::<Vec<_>>()
        );
        assert!(validate_snapshot(&open).documents[0]
            .findings
            .iter()
            .all(|f| f.code != "missing-component-definition"));
        std::fs::remove_file(source.file).unwrap();
        let before = file_digest(&dir.path);
        let report = validate_project(&dir.path).unwrap();
        assert_eq!(
            report.documents[0]
                .findings
                .iter()
                .filter(|f| f.code == "missing-component-definition")
                .count(),
            2
        );
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
