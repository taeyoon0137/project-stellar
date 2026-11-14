use super::*;
use crate::core_types::property::PropertyKey;
use crate::test_support::{file_digest, large_document, save_documents, scaffold_project, TempDir};
use stellar_project::saved_project::SavedProjectLoader;
use stellar_semantic_diff::value::SemanticValue;

#[test]
fn opaque_semantic_field_and_stale_base_are_explicitly_rejected_without_writes() {
    let dir = TempDir::new("candidate-opaque");
    scaffold_project(&dir.path);
    let model = large_document(4);
    save_documents(&dir.path, &[&model]);
    let index = SavedProjectLoader::new().open(&dir.path).unwrap();
    let base = saved_semantic_snapshot(&index).unwrap();
    let id = model.to_records().unwrap().nodes[0].id;
    let mut target = base.clone();
    let mut object = target.remove(id).unwrap();
    let path = object.path(&[PropertyKey(0xfffe)]);
    object
        .insert_property(path.clone(), SemanticValue::Bytes(vec![0, 255, 7]))
        .unwrap();
    target.insert(object).unwrap();
    let candidate = build_stage_candidate(
        &base,
        &target,
        &StageSelectionRequest::stage([ChangeId::property(path)]),
    )
    .unwrap();
    let before = file_digest(&dir.path);
    assert!(matches!(
        serialize_candidate(&index, &candidate),
        Err(CandidateStorageError::Unsupported(_))
    ));
    // 같은 후보에 다른 실제 index 문서가 주어지면 캐시를 기준으로 저장하지 않습니다.
    let mut changed = model.to_records().unwrap();
    changed.nodes[0].display_name = "different index".into();
    let (changed, _) = stellar_document::model::DocumentModel::from_records(changed).unwrap();
    let other = TempDir::new("candidate-stale");
    scaffold_project(&other.path);
    save_documents(&other.path, &[&changed]);
    let stale = SavedProjectLoader::new().open(&other.path).unwrap();
    assert!(matches!(
        serialize_candidate(&stale, &candidate),
        Err(CandidateStorageError::StaleBase)
    ));
    assert_eq!(file_digest(&dir.path), before);
}
