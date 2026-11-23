use super::*;
use crate::core_types::property::PropertyKey;
use crate::test_support::{file_digest, large_document, save_documents, scaffold_project, TempDir};
use stellar_project::saved_project::SavedProjectLoader;
use stellar_semantic_diff::value::SemanticValue;

/// `model`에 key 하나를 더한 사본.
fn with_key(
    model: &stellar_document::model::DocumentModel,
    n: u64,
) -> stellar_document::model::DocumentModel {
    use stellar_document::context::fixtures::{number, number_type};
    use stellar_document::context::key::{ContextKey, ContextScope};
    let mut copy = crate::diff::DocumentSource::of_model(model)
        .unwrap()
        .into_model()
        .unwrap();
    let mut set = copy.context().clone();
    set.insert_key(
        ContextKey::new(
            StableId::new_v7(95_000 + n, 0x0c, n),
            format!("k{n}"),
            ContextScope::Document(copy.document_id()),
            number_type(),
            number(96_000 + n, 3.0),
        )
        .unwrap(),
    )
    .unwrap();
    copy.replace_context(set);
    copy
}

#[test]
fn staging_a_context_change_writes_the_context_chunk() {
    // 검증 기준(ADR-0114 결정 2): Context가 있는 문서도 stage 왕복을 증명하고, 고른 Context 변경을 index에 씁니다.
    let dir = TempDir::new("candidate-context");
    scaffold_project(&dir.path);
    let model = with_key(&large_document(4), 1);
    save_documents(&dir.path, &[&model]);
    let index = SavedProjectLoader::new().open(&dir.path).unwrap();
    let base = saved_semantic_snapshot(&index).unwrap();
    let work = TempDir::new("candidate-context-work");
    scaffold_project(&work.path);
    save_documents(&work.path, &[&with_key(&model, 2)]);
    let working = SavedProjectLoader::new().open(&work.path).unwrap();
    let target = saved_semantic_snapshot(&working).unwrap();
    let key = StableId::new_v7(95_002, 0x0c, 2);
    let candidate = build_stage_candidate(
        &base,
        &target,
        &StageSelectionRequest::stage([ChangeId::add(key)]),
    )
    .unwrap();
    let files = serialize_candidate(&index, &candidate).unwrap();
    assert!(!files.is_empty());
    for (path, bytes) in &files {
        let full = dir.path.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }
    let staged = SavedProjectLoader::new().open(&dir.path).unwrap();
    let loaded = staged.load_document(model.document_id()).unwrap().model;
    assert!(loaded.context().key(key).is_some(), "stage한 key");
    assert!(loaded
        .context()
        .key(StableId::new_v7(95_001, 0x0c, 1))
        .is_some());
    assert_eq!(
        saved_semantic_snapshot(&staged).unwrap(),
        *candidate.snapshot()
    );
}

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

/// `model`의 Context에 `edit`을 적용한 사본.
fn with_context(
    model: &stellar_document::model::DocumentModel,
    edit: impl FnOnce(&mut stellar_document::context::set::ContextSet),
) -> stellar_document::model::DocumentModel {
    let mut copy = crate::diff::DocumentSource::of_model(model)
        .unwrap()
        .into_model()
        .unwrap();
    let mut set = copy.context().clone();
    edit(&mut set);
    copy.replace_context(set);
    copy
}

#[test]
fn staging_one_requirement_list_keeps_the_other_context_lists_as_they_were() {
    // 검증 기준(ADR-0123 결정 1, ADR-0138 결정 3): requirement·Slot 공개 목록이 있는 문서도 semantic stage 왕복을
    // 증명하고, 목록은 대상(컴포넌트·Slot)마다 고를 수 있는 변경입니다. 고른 목록만 index에 쓰고 나머지 목록은
    // index 값 그대로 둡니다.
    use stellar_semantic_diff::build::context::{
        COMPONENT_CONTEXT_REQUIREMENTS, SLOT_CONTEXT_KEYS,
    };
    let card = StableId::new_v7(97_001, 0x0c, 1);
    let badge = StableId::new_v7(97_002, 0x0c, 2);
    let slot = StableId::new_v7(97_003, 0x0c, 3);
    let other_slot = StableId::new_v7(97_004, 0x0c, 4);
    let key = StableId::new_v7(95_001, 0x0c, 1);
    let model = with_context(&with_key(&large_document(4), 1), |set| {
        set.add_requirement(card, key);
        set.add_slot_key(slot, key);
    });
    let dir = TempDir::new("candidate-context-lists");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&model]);
    let index = SavedProjectLoader::new().open(&dir.path).unwrap();
    let base = saved_semantic_snapshot(&index).unwrap();

    // 작업 트리: 새 컴포넌트 requirement, 새 Slot 공개 목록, 기존 Slot 목록 삭제.
    let working_model = with_context(&model, |set| {
        set.add_requirement(badge, key);
        set.add_slot_key(other_slot, key);
        set.remove_slot_key(slot, key);
    });
    let work = TempDir::new("candidate-context-lists-work");
    scaffold_project(&work.path);
    save_documents(&work.path, &[&working_model]);
    let working = SavedProjectLoader::new().open(&work.path).unwrap();
    let target = saved_semantic_snapshot(&working).unwrap();

    let document = target.get(model.document_id()).unwrap();
    let list = |key, owner: StableId| document.path(&[key]).with_key_segment(&owner.to_string());
    let changed: Vec<_> = diff(&base, &target)
        .changes()
        .iter()
        .filter_map(|change| change.property_path().cloned())
        .collect();
    for path in [
        list(COMPONENT_CONTEXT_REQUIREMENTS, badge),
        list(SLOT_CONTEXT_KEYS, other_slot),
        list(SLOT_CONTEXT_KEYS, slot),
    ] {
        assert!(changed.contains(&path), "목록마다 변경 하나: {path:?}");
    }

    let candidate = build_stage_candidate(
        &base,
        &target,
        &StageSelectionRequest::stage([ChangeId::property(list(
            COMPONENT_CONTEXT_REQUIREMENTS,
            badge,
        ))]),
    )
    .unwrap();
    let files = serialize_candidate(&index, &candidate).unwrap();
    assert!(!files.is_empty());
    for (path, bytes) in &files {
        let full = dir.path.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }
    let staged = SavedProjectLoader::new().open(&dir.path).unwrap();
    let context = staged
        .load_document(model.document_id())
        .unwrap()
        .model
        .context()
        .clone();
    assert!(
        context.requirements(badge).contains(&key),
        "고른 requirement"
    );
    assert!(
        context.requirements(card).contains(&key),
        "index의 requirement"
    );
    assert!(
        context.slot_keys(slot).contains(&key),
        "고르지 않은 삭제는 index 그대로"
    );
    assert!(context.slot_keys(other_slot).is_empty(), "고르지 않은 추가");
    assert_eq!(
        saved_semantic_snapshot(&staged).unwrap(),
        *candidate.snapshot()
    );
}
