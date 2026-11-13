//! VI.10.3 chunk model → document loader 통합 테스트.

mod common;

use std::collections::BTreeSet;

use common::golden::golden_document;
use common::{id, TempDir};
use stellar_core_types::ids::StableId;
use stellar_document::model::{DocumentModel, LoadModelError};
use stellar_document::record_codec::SubtreeRecords;
use stellar_document::validate::validate_document;
use stellar_project::document_io::load::{
    load_document, load_document_from_generation, LoadDocumentError, MemoryChunks,
};
use stellar_project::document_io::serialize::{serialize_document, SerializedDocument};
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::chunk::view_tree::{SubtreeRef, ViewTreeChunk};
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

const GOLDEN_COUNT: u64 = 100;

/// 100개 golden 문서 chunk model 전체(ChunkId + payload)의 content hash. 저장 형식이 바뀌면
/// 이 값이 바뀝니다 — 의도한 스키마 변경일 때만 갱신합니다.
const GOLDEN_CHUNKS_HASH: &str =
    "sha256:98dd5e6d897e2b87052533598055314cee1dfb481f02f4a177013850435d62c5";

/// 의미 동일성: 영속 레코드, 모든 node의 자식 순서, ID registry 왕복.
fn assert_semantically_equal(original: &DocumentModel, loaded: &DocumentModel, label: &str) {
    let records = original.to_records().unwrap();
    assert_eq!(loaded.to_records().unwrap(), records, "{label}: 레코드");
    assert_eq!(
        loaded.node_count(),
        original.node_count(),
        "{label}: node 수"
    );
    for node in &records.nodes {
        let handle = loaded
            .runtime_id(node.id)
            .unwrap_or_else(|| panic!("{label}: registry에 {} 없음", node.id));
        assert_eq!(
            loaded.stable_id(handle),
            Some(node.id),
            "{label}: registry 왕복"
        );
        assert_eq!(
            loaded.children_in_order(node.id),
            original.children_in_order(node.id),
            "{label}: {} 자식 순서",
            node.id
        );
    }
}

fn chunk_digest(documents: &[SerializedDocument]) -> String {
    let mut bytes = Vec::new();
    for document in documents {
        for chunk in document.chunks() {
            bytes.extend_from_slice(&chunk.chunk_id.to_bytes());
            bytes.extend_from_slice(&(chunk.payload.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&chunk.payload);
        }
    }
    ContentHash::compute(&bytes).to_string()
}

#[test]
fn hundred_golden_documents_round_trip_semantically_equal() {
    // 검증 기준(VI.10.3): 100개 golden document round-trip 의미 동일.
    let mut serialized_all = Vec::new();
    let mut total_nodes = 0;
    let mut total_stacks = 0;
    let mut total_unresolved = 0;
    let mut deep = 0;
    for i in 0..GOLDEN_COUNT {
        let original = golden_document(i);
        let label = format!("golden #{i}");
        let records = original.to_records().unwrap();
        let expected_diagnostics = validate_document(&records)
            .unwrap_or_else(|e| panic!("{label}: golden 문서가 유효하지 않음: {e}"));
        total_nodes += records.nodes.len();
        total_stacks += records.modifier_stacks.len();
        total_unresolved += expected_diagnostics.unresolved.len();
        if records.nodes.len() > 64 {
            deep += 1;
        }

        let serialized = serialize_document(&original).unwrap();
        let mut chunks = MemoryChunks::new();
        chunks.insert_document(&serialized);
        let loaded = load_document(&serialized.manifest.payload, &mut chunks).unwrap();
        assert_semantically_equal(&original, &loaded.model, &label);
        assert_eq!(loaded.diagnostics, expected_diagnostics, "{label}: 진단");
        // 재직렬화 byte-identical.
        assert_eq!(
            serialize_document(&loaded.model).unwrap(),
            serialized,
            "{label}"
        );
        serialized_all.push(serialized);
    }
    // fixture가 실제로 다양한 내용을 덮는지.
    assert!(total_nodes > 600, "node {total_nodes}");
    assert!(total_stacks > 100, "stack {total_stacks}");
    assert!(total_unresolved > 10, "미해결 {total_unresolved}");
    assert_eq!(deep, 4, "depth 64 chain 문서");
    let digest = chunk_digest(&serialized_all);
    assert_eq!(digest, GOLDEN_CHUNKS_HASH, "golden chunk hash");
}

#[derive(Default)]
struct Recorder {
    reads: Vec<ChunkId>,
}

impl ChunkReadRecorder for Recorder {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.reads.push(pointer.chunk_id);
    }
}

#[test]
fn saved_generation_loads_each_golden_document_lazily() {
    // 실제 저장 파이프라인(V.5)으로 100개 문서를 한 generation에 저장한 뒤 lazy loader로
    // 문서마다 그 문서의 chunk만 읽어 복원합니다.
    let dir = TempDir::new("stellar-docio-roundtrip");
    let project_id = StableId::new_v7(42, 0x0abc, 42);
    let originals: Vec<DocumentModel> = (0..GOLDEN_COUNT).map(golden_document).collect();
    let serialized: Vec<SerializedDocument> = originals
        .iter()
        .map(|m| serialize_document(m).unwrap())
        .collect();
    let states: Vec<_> = serialized
        .iter()
        .flat_map(|s| s.to_dirty_states())
        .collect();
    let roots: Vec<StableId> = serialized.iter().map(|s| s.document_id).collect();
    let plan = build_save_plan(None, project_id, 1, &states, roots).unwrap();
    let mut memory = SavedGenerationState::new(None);
    run_save(&dir.path, &plan, &mut memory, &CancelToken::new()).unwrap();

    let mut open_recorder = Recorder::default();
    let lazy = LazyGeneration::open(&dir.path, project_id, &mut open_recorder).unwrap();
    assert_eq!(lazy.document_manifests().len(), GOLDEN_COUNT as usize);
    for (i, (original, chunks)) in originals.iter().zip(&serialized).enumerate() {
        let mut recorder = Recorder::default();
        let loaded =
            load_document_from_generation(&lazy, original.document_id(), &mut recorder).unwrap();
        assert_semantically_equal(original, &loaded.model, &format!("disk #{i}"));
        // 읽은 chunk = 그 문서의 ViewTree + subtree(manifest는 열기 때 이미 읽음).
        let expected: BTreeSet<ChunkId> = chunks
            .view_trees
            .iter()
            .chain(&chunks.subtrees)
            .map(|c| c.chunk_id)
            .collect();
        let read: BTreeSet<ChunkId> = recorder.reads.iter().copied().collect();
        assert_eq!(read, expected, "disk #{i}: 다른 문서 chunk를 읽지 않음");
    }
}

#[test]
fn view_tree_with_two_subtrees_is_merged() {
    // loader는 ViewTree의 subtree가 여러 개여도 저장된 참조를 따라 합칩니다.
    let original = golden_document(5);
    let mut serialized = serialize_document(&original).unwrap();
    let tree = ViewTreeChunk::decode(&serialized.view_trees[0].payload).unwrap();
    let subtree = SubtreeRecords::decode(&serialized.subtrees[0].payload).unwrap();
    assert!(subtree.nodes.len() >= 3, "분할할 자식이 있어야 함");
    // root의 마지막 자식 subtree를 별도 chunk로 분리합니다.
    let root = subtree.root;
    let split_root = original.children_in_order(root).last().copied().unwrap();
    let mut moved = std::collections::BTreeSet::from([split_root]);
    let mut first = SubtreeRecords {
        root,
        nodes: Vec::new(),
        modifier_stacks: Vec::new(),
    };
    let mut second = SubtreeRecords {
        root: split_root,
        nodes: Vec::new(),
        modifier_stacks: Vec::new(),
    };
    for node in subtree.nodes {
        let in_second = node.id == split_root || node.parent.is_some_and(|p| moved.contains(&p));
        if in_second {
            moved.insert(node.id);
            second.nodes.push(node);
        } else {
            first.nodes.push(node);
        }
    }
    for stack in subtree.modifier_stacks {
        if moved.contains(&stack.node) {
            second.modifier_stacks.push(stack);
        } else {
            first.modifier_stacks.push(stack);
        }
    }
    let second_chunk = ChunkId::from_stable_id(id(9_999, 1));
    let new_tree = ViewTreeChunk {
        view_root: tree.view_root,
        subtrees: vec![
            tree.subtrees[0],
            SubtreeRef {
                root_node: split_root,
                chunk: second_chunk,
            },
        ],
    };
    serialized.view_trees[0].payload = new_tree.encode().unwrap();
    serialized.subtrees[0].payload = first.encode();
    let mut chunks = MemoryChunks::new();
    chunks.insert_document(&serialized);
    chunks.insert(second_chunk, second.encode());
    let loaded = load_document(&serialized.manifest.payload, &mut chunks).unwrap();
    assert_semantically_equal(&original, &loaded.model, "2 subtree");
}

#[test]
fn corrupted_chunk_models_are_rejected_with_typed_errors() {
    let original = golden_document(3);
    let serialized = serialize_document(&original).unwrap();
    let base = || {
        let mut chunks = MemoryChunks::new();
        chunks.insert_document(&serialized);
        chunks
    };

    // 없는 chunk.
    let mut missing = MemoryChunks::new();
    missing.insert(
        serialized.view_trees[0].chunk_id,
        serialized.view_trees[0].payload.clone(),
    );
    assert!(matches!(
        load_document(&serialized.manifest.payload, &mut missing),
        Err(LoadDocumentError::Read(_))
    ));

    // ViewTree root가 정의와 다름.
    let mut chunks = base();
    let mut tree = ViewTreeChunk::decode(&serialized.view_trees[0].payload).unwrap();
    let real_root = tree.view_root;
    tree.view_root = id(9_999, 2);
    chunks.insert(serialized.view_trees[0].chunk_id, tree.encode().unwrap());
    assert_eq!(
        load_document(&serialized.manifest.payload, &mut chunks).unwrap_err(),
        LoadDocumentError::ViewRootMismatch {
            view: original.definition().pages[0].views[0].id,
            expected: real_root,
            found: id(9_999, 2),
        }
    );

    // subtree root가 ViewTree 참조와 다름.
    let mut chunks = base();
    let mut tree = ViewTreeChunk::decode(&serialized.view_trees[0].payload).unwrap();
    tree.subtrees[0].root_node = id(9_999, 3);
    chunks.insert(serialized.view_trees[0].chunk_id, tree.encode().unwrap());
    assert!(matches!(
        load_document(&serialized.manifest.payload, &mut chunks),
        Err(LoadDocumentError::SubtreeRootMismatch { .. })
    ));

    // subtree bytes 손상.
    let mut chunks = base();
    let mut payload = serialized.subtrees[0].payload.clone();
    payload.truncate(payload.len() - 1);
    chunks.insert(serialized.subtrees[0].chunk_id, payload);
    assert!(matches!(
        load_document(&serialized.manifest.payload, &mut chunks),
        Err(LoadDocumentError::Subtree { .. })
    ));

    // 형식은 맞지만 불변식 위반(형제 OrderKey 중복) → validator가 거부.
    let mut chunks = base();
    let mut subtree = SubtreeRecords::decode(&serialized.subtrees[0].payload).unwrap();
    let root = subtree.root;
    let siblings: Vec<usize> = subtree
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent == Some(root))
        .map(|(i, _)| i)
        .collect();
    assert!(siblings.len() >= 2, "형제 둘 이상 필요");
    let key = subtree.nodes[siblings[0]].order_key.clone();
    subtree.nodes[siblings[1]].order_key = key;
    chunks.insert(serialized.subtrees[0].chunk_id, subtree.encode());
    match load_document(&serialized.manifest.payload, &mut chunks) {
        Err(LoadDocumentError::Model(LoadModelError::Invalid(err))) => {
            assert_eq!(err.violations.len(), 1)
        }
        other => panic!("validator 거부여야 함: {other:?}"),
    }

    // manifest 손상.
    assert!(matches!(
        load_document(&serialized.manifest.payload[..10], &mut base()),
        Err(LoadDocumentError::Manifest(_))
    ));
}
