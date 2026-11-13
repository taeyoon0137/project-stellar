//! VI.10.2 document → chunk model serializer 통합 테스트.

mod common;

use common::{
    contains_bytes, id, insert_churned, rich_model, rich_nodes, rich_stacks, runtime_parts, TempDir,
};
use std::collections::BTreeSet;
use stellar_core_types::ids::StableId;
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::record_codec::SubtreeRecords;
use stellar_document::records::NodePayload;
use stellar_project::document_io::manifest::DocumentManifestChunk;
use stellar_project::document_io::serialize::{
    manifest_chunk_id, serialize_document, subtree_chunk_id, view_tree_chunk_id, SerializeError,
};
use stellar_project::document_io::DOCUMENT_CHUNK_SCHEMA_VERSION;

use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::chunk::store::{ChunkStore, WriteOutcome};
use stellar_storage::chunk::view_tree::ViewTreeChunk;

/// 눈에 띄는 generation 값(0x1F2E). 모든 node 핸들이 이 generation을 갖도록 churn합니다.
const CHURN: u32 = 0x1F2E;

/// 핸들 (slot, generation)이 저장될 수 있는 byte 표현들.
fn runtime_patterns(slot: u32, generation: u32) -> Vec<Vec<u8>> {
    let cat = |a: [u8; 4], b: [u8; 4]| [a, b].concat();
    vec![
        cat(slot.to_le_bytes(), generation.to_le_bytes()),
        cat(generation.to_le_bytes(), slot.to_le_bytes()),
        cat(slot.to_be_bytes(), generation.to_be_bytes()),
        cat(generation.to_be_bytes(), slot.to_be_bytes()),
        generation.to_le_bytes().to_vec(),
        generation.to_be_bytes().to_vec(),
    ]
}

#[test]
fn save_bytes_contain_no_runtime_entity_id_pattern() {
    // 검증 기준(VI.10.2): save bytes에 RuntimeEntityId pattern 없음.
    let model = rich_model(1, CHURN);
    let mut patterns = Vec::new();
    for record in rich_nodes(1) {
        let handle = model.runtime_id(record.id).unwrap();
        let (slot, generation) = runtime_parts(handle);
        assert_eq!(generation, CHURN, "churn이 generation을 올림");
        patterns.extend(runtime_patterns(slot, generation));
    }
    assert!(!patterns.is_empty());

    // scanner 자체 검증(positive control): 핸들 bytes를 넣은 buffer는 검출됩니다.
    let mut leaked = b"prefix".to_vec();
    leaked.extend_from_slice(&patterns[0]);
    assert!(patterns.iter().any(|p| contains_bytes(&leaked, p)));

    let serialized = serialize_document(&model).unwrap();
    let dir = TempDir::new("stellar-docio-serialize");
    let store = ChunkStore::new(&dir.path);
    for chunk in serialized.chunks() {
        for pattern in &patterns {
            assert!(
                !contains_bytes(&chunk.payload, pattern),
                "chunk {} payload에 runtime id pattern {pattern:02x?}",
                chunk.chunk_id
            );
        }
        // 실제로 기록된 chunk 파일(envelope + payload)도 검사합니다.
        let hash = match store.write_chunk(chunk.kind, &chunk.payload).unwrap() {
            WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. } => hash,
        };
        let file = dir
            .path
            .join(ChunkObjectPath::for_hash(&hash).to_relative_path());
        let bytes = std::fs::read(file).unwrap();
        for pattern in &patterns {
            assert!(
                !contains_bytes(&bytes, pattern),
                "저장 파일에 runtime id pattern"
            );
        }
    }
}

#[test]
fn serialization_is_independent_of_runtime_layout() {
    // 같은 의미 문서를 (1) churn 없이 정방향, (2) churn + 역방향 형제 삽입으로 만듭니다.
    // runtime 핸들은 다르지만 chunk bytes는 같아야 합니다(핸들이 새지 않는다는 독립 증거).
    let plain = rich_model(1, 0);
    let mut shuffled = DocumentModel::new(common::rich_definition(1));
    let mut nodes = rich_nodes(1);
    // 부모가 먼저 오도록 depth 순으로 정렬하되, 같은 depth 안에서는 역순.
    let depth = |n: &stellar_document::records::NodeRecord,
                 all: &[stellar_document::records::NodeRecord]| {
        let mut d = 0;
        let mut current = n.parent;
        while let Some(p) = current {
            d += 1;
            current = all.iter().find(|x| x.id == p).and_then(|x| x.parent);
        }
        d
    };
    let snapshot = nodes.clone();
    nodes.sort_by_key(|n| (depth(n, &snapshot), std::cmp::Reverse(n.id)));
    for (i, record) in nodes.into_iter().enumerate() {
        insert_churned(&mut shuffled, record, 3 + i as u32);
    }
    for stack in rich_stacks(1).into_iter().rev() {
        shuffled.set_modifier_stack(stack).unwrap();
    }
    let a_handle = plain.runtime_id(id(1, 13)).unwrap();
    let b_handle = shuffled.runtime_id(id(1, 13)).unwrap();
    assert_ne!(
        runtime_parts(a_handle),
        runtime_parts(b_handle),
        "핸들은 다름"
    );

    let a = serialize_document(&plain).unwrap();
    let b = serialize_document(&shuffled).unwrap();
    assert_eq!(a, b, "chunk model byte-identical");
    // 같은 모델을 두 번 직렬화해도 같습니다.
    assert_eq!(serialize_document(&plain).unwrap(), a);
}

#[test]
fn chunk_model_has_manifest_view_trees_and_subtrees() {
    let model = rich_model(2, 0);
    let serialized = serialize_document(&model).unwrap();
    assert_eq!(serialized.document_id, id(2, 1));

    let manifest = &serialized.manifest;
    assert_eq!(manifest.kind, KIND_DOCUMENT_MANIFEST);
    assert_eq!(manifest.chunk_id, manifest_chunk_id(id(2, 1)));
    assert_eq!(manifest.schema_version, DOCUMENT_CHUNK_SCHEMA_VERSION);
    let decoded = DocumentManifestChunk::decode(&manifest.payload).unwrap();
    assert_eq!(&decoded.definition, model.definition());
    let views = [id(2, 3), id(2, 4), id(2, 6)];
    let roots = [id(2, 10), id(2, 20), id(2, 30)];
    assert_eq!(
        decoded
            .view_trees
            .iter()
            .map(|r| r.view)
            .collect::<Vec<_>>(),
        views
    );

    assert_eq!(serialized.view_trees.len(), 3);
    assert_eq!(serialized.subtrees.len(), 3);
    let mut total_nodes = 0;
    for (i, (tree_chunk, subtree_chunk)) in serialized
        .view_trees
        .iter()
        .zip(&serialized.subtrees)
        .enumerate()
    {
        assert_eq!(tree_chunk.kind, KIND_VIEW_TREE);
        assert_eq!(tree_chunk.chunk_id, view_tree_chunk_id(id(2, 1), views[i]));
        assert_eq!(decoded.view_trees[i].chunk, tree_chunk.chunk_id);
        let tree = ViewTreeChunk::decode(&tree_chunk.payload).unwrap();
        assert_eq!(tree.view_root, roots[i]);
        assert_eq!(tree.subtrees.len(), 1, "초기 정책: View당 subtree 하나");
        assert_eq!(tree.subtrees[0].root_node, roots[i]);
        assert_eq!(tree.subtrees[0].chunk, subtree_chunk.chunk_id);
        assert_eq!(subtree_chunk.chunk_id, subtree_chunk_id(id(2, 1), roots[i]));
        assert_eq!(subtree_chunk.kind, KIND_VIEW_TREE);
        let subtree = SubtreeRecords::decode(&subtree_chunk.payload).unwrap();
        assert_eq!(subtree.root, roots[i]);
        total_nodes += subtree.nodes.len();
    }
    assert_eq!(total_nodes, model.node_count(), "모든 node가 정확히 한 번");
    // modifier stack은 subtree chunk에 node payload와 분리되어 들어갑니다.
    let first = SubtreeRecords::decode(&serialized.subtrees[0].payload).unwrap();
    assert_eq!(first.modifier_stacks.len(), 2);
    assert_eq!(serialized.to_dirty_states().len(), 7);
}

#[test]
fn invalid_document_produces_no_chunks() {
    let mut model = rich_model(3, 0);
    // 어느 View root도 아닌 부모 없는 Frame(View당 root 둘).
    model
        .insert_node(common::node(
            3,
            40,
            None,
            0x10,
            "고아",
            NodePayload::Frame(FramePayload::default()),
        ))
        .unwrap();
    match serialize_document(&model) {
        Err(SerializeError::Invalid(err)) => {
            assert_eq!(err.violations.len(), 1);
            assert!(err.to_string().contains("검증 실패"));
        }
        other => panic!("검증 실패여야 함: {other:?}"),
    }
    // NIL id 문서 정의도 거부.
    let mut definition = common::rich_definition(3);
    definition.id = StableId::NIL;
    let nil = DocumentModel::new(definition);
    assert!(matches!(
        serialize_document(&nil),
        Err(SerializeError::Invalid(_))
    ));
}

#[test]
fn chunk_ids_are_domain_separated_by_family_and_document() {
    // 같은 StableId에서 유도해도 계열(manifest·ViewTree·subtree)·문서가 다르면 ChunkId가
    // 다르고, 원시 StableId를 그대로 쓰지 않습니다(UUIDv8).
    let x = id(5, 1);
    let other = id(6, 1);
    let derived = [
        manifest_chunk_id(x),
        view_tree_chunk_id(x, x),
        subtree_chunk_id(x, x),
        manifest_chunk_id(other),
        view_tree_chunk_id(other, x),
        subtree_chunk_id(other, x),
    ];
    let unique: BTreeSet<ChunkId> = derived.iter().copied().collect();
    assert_eq!(unique.len(), derived.len(), "계열·문서별로 서로 다름");
    for chunk in derived {
        assert_ne!(chunk, ChunkId::from_stable_id(x), "원시 StableId 미사용");
        assert_eq!(chunk.stable_id().version(), 8, "UUIDv8");
        assert_eq!(chunk.stable_id().variant(), 0b10, "RFC 9562 variant");
    }
    // 유도 규칙 golden — 저장 형식의 일부이므로 의도한 형식 변경일 때만 갱신합니다.
    // 값은 SHA-256(domain ‖ family ‖ 문서 ‖ 객체)를 독립 구현(Python hashlib)으로 계산해
    // 교차 확인했습니다.
    assert_eq!(
        manifest_chunk_id(x).to_string(),
        "7661e0d5-da0d-8fa8-8447-e95858fe5228",
        "manifest"
    );
    assert_eq!(
        view_tree_chunk_id(x, id(5, 3)).to_string(),
        "9af9620e-b26a-81ba-88bc-a674c2d1689a",
        "ViewTree"
    );
    assert_eq!(
        subtree_chunk_id(x, id(5, 10)).to_string(),
        "192a8036-69b2-8e1f-9a05-685f097b6515",
        "subtree"
    );

    // 실제 문서: View·node id가 같은 두 문서(복제 등)의 chunk 집합이 겹치지 않습니다.
    let a = rich_model(1, 0);
    let mut definition = common::rich_definition(1);
    definition.id = id(9, 1);
    let mut b = DocumentModel::new(definition);
    for record in rich_nodes(1) {
        b.insert_node(record).unwrap();
    }
    for stack in rich_stacks(1) {
        b.set_modifier_stack(stack).unwrap();
    }
    let chunk_set = |model: &DocumentModel| -> BTreeSet<ChunkId> {
        serialize_document(model)
            .unwrap()
            .chunks()
            .into_iter()
            .map(|c| c.chunk_id)
            .collect()
    };
    let (set_a, set_b) = (chunk_set(&a), chunk_set(&b));
    assert_eq!(set_a.len(), 7);
    assert!(set_a.is_disjoint(&set_b), "다른 문서의 ChunkId와 충돌 없음");
}
