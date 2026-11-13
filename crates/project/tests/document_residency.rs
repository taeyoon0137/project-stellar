//! XVI.5.5 비활성 문서 chunk unload·재로드 통합 테스트.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::golden::golden_document;
use common::{rich_model, TempDir};
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::open_set::OpenDocumentSet;
use stellar_document::records::NodePayload;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::document_io::residency::{DirtyState, ResidentDocuments};
use stellar_project::document_io::serialize::{serialize_document, SerializedDocument};
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

#[derive(Default)]
struct Recorder {
    reads: Vec<ChunkId>,
}

impl ChunkReadRecorder for Recorder {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.reads.push(pointer.chunk_id);
    }
}

fn project_id() -> StableId {
    StableId::new_v7(77, 0x0abc, 77)
}

/// 문서들을 실제 저장 파이프라인(V.5)으로 새 generation에 저장합니다.
fn save(root: &Path, documents: &[SerializedDocument]) {
    let base = load_current_generation(root, project_id()).ok();
    let states: Vec<_> = documents.iter().flat_map(|d| d.to_dirty_states()).collect();
    let roots = documents.iter().map(|d| d.document_id).collect();
    let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).unwrap();
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).unwrap();
}

/// 문서 셋(rich 2개 + golden 1개)을 저장한 프로젝트.
fn project() -> (TempDir, Vec<DocumentModel>) {
    let dir = TempDir::new("stellar-docio-residency");
    let models = vec![rich_model(1, 0), rich_model(2, 3), golden_document(24)];
    let serialized: Vec<SerializedDocument> = models
        .iter()
        .map(|m| serialize_document(m).unwrap())
        .collect();
    save(&dir.path, &serialized);
    (dir, models)
}

fn document_chunks(model: &DocumentModel) -> BTreeSet<ChunkId> {
    let serialized = serialize_document(model).unwrap();
    serialized
        .view_trees
        .iter()
        .chain(&serialized.subtrees)
        .map(|c| c.chunk_id)
        .collect()
}

#[test]
fn inactive_document_reload_has_identical_semantic_snapshot() {
    // 검증 기준(XVI.5.5): inactive reload semantic snapshot 동일.
    let (dir, models) = project();
    let ids: Vec<StableId> = models.iter().map(|m| m.document_id()).collect();
    let mut open_recorder = Recorder::default();
    let lazy = LazyGeneration::open(&dir.path, project_id(), &mut open_recorder).unwrap();
    let mut residency = ResidentDocuments::open(&lazy).unwrap();
    assert_eq!(
        residency.resident_count(),
        0,
        "열기는 node chunk를 올리지 않음"
    );

    let mut recorder = Recorder::default();
    for id in &ids {
        residency.ensure_loaded(&lazy, *id, &mut recorder).unwrap();
    }
    let before: Vec<String> = ids
        .iter()
        .map(|id| model_snapshot_json(residency.document(*id).unwrap()).unwrap())
        .collect();
    for (model, snapshot) in models.iter().zip(&before) {
        assert_eq!(
            &model_snapshot_json(model).unwrap(),
            snapshot,
            "로드 = 원본"
        );
    }

    // 문서 0만 열려 있음(활성). 1·2는 비활성.
    let mut open = OpenDocumentSet::new();
    open.open(ids[0]);
    let reads_before_unload = recorder.reads.len();
    let report = residency.unload_inactive(&open);
    assert_eq!(report.unloaded, vec![ids[1], ids[2]]);
    assert_eq!(report.kept_active, vec![ids[0]]);
    assert!(report.kept_dirty.is_empty());
    assert!(residency.is_resident(ids[0]));
    assert!(!residency.is_resident(ids[1]) && !residency.is_resident(ids[2]));

    // 문서 index는 unload되지 않고 chunk 읽기 없이 조회됩니다.
    let index = residency.document_index();
    assert_eq!(index.documents.len(), 3);
    for model in &models {
        assert_eq!(
            index.document(model.document_id()),
            Some(model.definition())
        );
    }
    assert_eq!(
        recorder.reads.len(),
        reads_before_unload,
        "unload·index 조회 중 읽기 0"
    );

    // 재로드: 그 문서의 ViewTree·subtree chunk만 lazy loader로 읽습니다.
    for (i, id) in ids.iter().enumerate().skip(1) {
        let mut reload = Recorder::default();
        residency.ensure_loaded(&lazy, *id, &mut reload).unwrap();
        let read: BTreeSet<ChunkId> = reload.reads.iter().copied().collect();
        assert_eq!(
            read,
            document_chunks(&models[i]),
            "문서 {i}: 그 문서 chunk만"
        );
        assert_eq!(reload.reads.len(), read.len(), "같은 chunk 중복 읽기 없음");
        let after = model_snapshot_json(residency.document(*id).unwrap()).unwrap();
        assert_eq!(after, before[i], "문서 {i}: reload snapshot 동일");
    }
    // 이미 올라온 문서는 다시 읽지 않습니다.
    let mut again = Recorder::default();
    residency.ensure_loaded(&lazy, ids[1], &mut again).unwrap();
    assert!(again.reads.is_empty());
}

#[test]
fn unsaved_dirty_document_is_never_unloaded() {
    let (dir, models) = project();
    let ids: Vec<StableId> = models.iter().map(|m| m.document_id()).collect();
    let mut recorder = Recorder::default();
    let lazy = LazyGeneration::open(&dir.path, project_id(), &mut recorder).unwrap();
    let mut residency = ResidentDocuments::open(&lazy).unwrap();
    for id in &ids {
        residency.ensure_loaded(&lazy, *id, &mut recorder).unwrap();
        assert_eq!(residency.dirty_state(*id), Some(DirtyState::Clean));
    }

    // 비활성 문서 1을 편집(저장 안 함): View A subtree chunk 하나만 dirty.
    let edited_node = common::id(2, 15);
    let model = residency.document_mut(ids[1]).unwrap();
    model
        .set_display_name(edited_node, "미저장 편집".to_string())
        .unwrap();
    let edited_snapshot = model_snapshot_json(model).unwrap();
    let view_a_root = common::id(2, 10);
    let expected_dirty = vec![stellar_project::document_io::serialize::subtree_chunk_id(
        view_a_root,
    )];
    assert_eq!(
        residency.dirty_state(ids[1]),
        Some(DirtyState::Dirty(expected_dirty.clone()))
    );

    let open = OpenDocumentSet::new(); // 아무 문서도 열려 있지 않음.
    let report = residency.unload_inactive(&open);
    assert_eq!(report.unloaded, vec![ids[0], ids[2]]);
    assert_eq!(
        report.kept_dirty,
        vec![(ids[1], DirtyState::Dirty(expected_dirty))]
    );
    assert!(residency.is_resident(ids[1]), "dirty 문서 유지");
    assert_eq!(
        model_snapshot_json(residency.document(ids[1]).unwrap()).unwrap(),
        edited_snapshot,
        "미저장 편집 보존"
    );

    // 직렬화할 수 없는 상태(편집 중 불변식 위반)도 내려놓지 않습니다.
    let model = residency.document_mut(ids[1]).unwrap();
    model
        .insert_node(common::node(
            2,
            77,
            None,
            0x10,
            "고아",
            NodePayload::Fragment(stellar_document::fragment::FragmentPayload),
        ))
        .unwrap();
    assert_eq!(
        residency.dirty_state(ids[1]),
        Some(DirtyState::Unserializable)
    );
    let report = residency.unload_inactive(&open);
    assert_eq!(
        report.kept_dirty,
        vec![(ids[1], DirtyState::Unserializable)]
    );
    residency
        .document_mut(ids[1])
        .unwrap()
        .remove_leaf(common::id(2, 77))
        .unwrap();

    // 저장 후 새 generation을 기준으로 삼으면 clean → unload → 재로드 시 편집 내용 그대로.
    let current: Vec<SerializedDocument> = vec![
        serialize_document(&models[0]).unwrap(),
        serialize_document(residency.document(ids[1]).unwrap()).unwrap(),
        serialize_document(&models[2]).unwrap(),
    ];
    save(&dir.path, &current);
    let mut reopen = Recorder::default();
    let lazy2 = LazyGeneration::open(&dir.path, project_id(), &mut reopen).unwrap();
    residency.adopt_generation(&lazy2).unwrap();
    assert_eq!(residency.dirty_state(ids[1]), Some(DirtyState::Clean));
    let report = residency.unload_inactive(&open);
    assert_eq!(report.unloaded, vec![ids[1]]);
    let mut reload = Recorder::default();
    residency
        .ensure_loaded(&lazy2, ids[1], &mut reload)
        .unwrap();
    assert_eq!(
        model_snapshot_json(residency.document(ids[1]).unwrap()).unwrap(),
        edited_snapshot,
        "저장된 편집이 재로드됨"
    );
}

#[test]
fn unknown_document_is_rejected() {
    let (dir, _) = project();
    let mut recorder = Recorder::default();
    let lazy = LazyGeneration::open(&dir.path, project_id(), &mut recorder).unwrap();
    let mut residency = ResidentDocuments::open(&lazy).unwrap();
    let missing = StableId::new_v7(5, 5, 5);
    assert!(!residency.contains(missing));
    assert!(residency
        .ensure_loaded(&lazy, missing, &mut recorder)
        .is_err());
    assert_eq!(residency.dirty_state(missing), None);
}
