//! 종단 시나리오: 의미 경계 chunk 분할(V.6.4)과 idle 병합(V.6.5)이 문서 의미를 바꾸지 않는지.
//!
//! document_io 기본 직렬화는 View마다 subtree chunk 하나를 씁니다. 여기서는 command로 만든 문서의 View
//! subtree를 storage split planner 입력([`SplitTree`])으로 옮기고(의미 경계 = Frame), 계획한 layout대로
//! ViewTree 하나와 subtree chunk 여러 개를 만들어 저장 transaction으로 디스크에 씁니다. 다시 열어 loader가
//! chunk를 합친 문서가 원본과 의미상 같은지, 분할·sticky 재분할·병합 각 단계에서 확인합니다.
//!
//! 크기 모델: subtree chunk payload는 root id 16 + node 수 4 + stack 수 4 bytes에 node마다 길이 prefix
//! 4 + node 레코드, stack마다 4 + stack bytes를 더한 값입니다. 자식 chunk 참조는 ViewTree가 담으므로
//! 부모 chunk payload에 더해지지 않습니다. 따라서 `chunk_base_bytes = 24`, `child_ref_bytes = 0`이고
//! 계획의 `payload_bytes`는 실제 payload 길이와 같아야 합니다.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use common::builders::{default_modifier, frame, new_document, rectangle, text};
use common::{
    assert_same_document, assert_same_records, assert_valid, key, save_states, sid, Editor, TempDir,
};
use stellar_commands::command::Command;
use stellar_commands::node_ops::{DeleteSubtree, InsertNode};
use stellar_commands::test_support::{random_valid_request, FreshIds};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::layout_record::LayoutRecord;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierKind, ModifierStack};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::{encode_node_record, SubtreeRecords, SubtreeSections};
use stellar_document::records::NodeRecord;
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::{
    serialize_document, subtree_chunk_id, view_tree_chunk_id,
};
use stellar_project::document_io::DOCUMENT_CHUNK_SCHEMA_VERSION;
use stellar_project::identity::parse_project_id;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::scaffold::write_default_scaffold;
use stellar_storage::chunk::budget::ChunkBudget;
use stellar_storage::chunk::envelope::ChunkKindCode;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_VIEW_TREE;
use stellar_storage::chunk::merge::{
    begin_idle_merge, plan_chunk_merges, run_idle_merge, IdleMergeOutcome,
};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::split::{
    plan_chunk_split, ChunkCostModel, ChunkLayout, SplitNode, SplitPolicy, SplitTree,
};
use stellar_storage::chunk::view_tree::{SubtreeRef, ViewTreeChunk};
use stellar_storage::gc::scheduler::GcScheduler;

const SPACE: u64 = 31;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb967";

/// subtree chunk payload 크기 모델(모듈 문서).
const COST: ChunkCostModel = ChunkCostModel {
    chunk_base_bytes: 24,
    child_ref_bytes: 0,
};

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

fn policy(threshold_bytes: u64) -> SplitPolicy {
    SplitPolicy {
        threshold_bytes,
        cost: COST,
    }
}

/// View 하나의 split 입력과 그 subtree 레코드.
struct ViewPartition {
    view: StableId,
    records: SubtreeRecords,
    /// View subtree의 확장 절(가이드·View 변수, ADR-0126). 분할 크기에는 넣지 않고 소유 node가 있는 chunk에 씁니다.
    sections: SubtreeSections,
    tree: SplitTree,
}

/// 문서의 View별 split 입력(정의 순서).
fn partitions(model: &DocumentModel) -> Vec<ViewPartition> {
    let records = model.to_records().expect("레코드");
    records
        .view_subtrees_with_sections()
        .expect("View 분할")
        .into_iter()
        .filter(|(_, subtree, _)| !subtree.nodes.is_empty())
        .map(|(view, subtree, sections)| {
            let stack_bytes: BTreeMap<StableId, u64> = subtree
                .modifier_stacks
                .iter()
                .map(|s| (s.node, 4 + s.encode().len() as u64))
                .collect();
            // 명시 layout record(ADR-0086)도 node 소유 bytes로 셉니다. chunk에 record가 있으면 절 머리 4 byte가 더 붙습니다.
            let layout_bytes: BTreeMap<StableId, u64> = subtree
                .layout_records
                .iter()
                .map(|r| (r.node, 4 + r.encode().len() as u64))
                .collect();
            let nodes = subtree
                .nodes
                .iter()
                .map(|n| SplitNode {
                    id: n.id,
                    parent: n.parent,
                    boundary: n.kind() == NodeKind::Frame,
                    own_bytes: 4
                        + encode_node_record(n).len() as u64
                        + stack_bytes.get(&n.id).copied().unwrap_or(0)
                        + layout_bytes.get(&n.id).copied().unwrap_or(0),
                })
                .collect();
            ViewPartition {
                view,
                tree: SplitTree::new(nodes).expect("split tree"),
                records: subtree,
                sections,
            }
        })
        .collect()
}

/// layout대로 만든 문서 chunk model: chunk id → (kind, payload). manifest는 기본 직렬화를 그대로 씁니다.
fn chunk_model(
    model: &DocumentModel,
    parts: &[ViewPartition],
    layouts: &BTreeMap<StableId, ChunkLayout>,
) -> BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)> {
    let document = model.document_id();
    let serialized = serialize_document(model).expect("직렬화");
    let mut out = BTreeMap::new();
    out.insert(
        serialized.manifest.chunk_id,
        (
            serialized.manifest.kind,
            serialized.manifest.payload.clone(),
        ),
    );
    for part in parts {
        let layout = &layouts[&part.view];
        let by_id: BTreeMap<StableId, &NodeRecord> =
            part.records.nodes.iter().map(|n| (n.id, n)).collect();
        let stacks: BTreeMap<StableId, &ModifierStack> = part
            .records
            .modifier_stacks
            .iter()
            .map(|s| (s.node, s))
            .collect();
        let layout_records: BTreeMap<StableId, &LayoutRecord> = part
            .records
            .layout_records
            .iter()
            .map(|r| (r.node, r))
            .collect();
        let mut refs = Vec::new();
        let mut covered = BTreeSet::new();
        for chunk in layout.chunks() {
            let mut members: Vec<StableId> = chunk.nodes.clone();
            for node in &members {
                assert!(covered.insert(*node), "node {node}가 두 chunk에 속함");
            }
            let subtree = SubtreeRecords {
                root: chunk.root,
                nodes: members.iter().map(|n| by_id[n].clone()).collect(),
                modifier_stacks: {
                    members.sort();
                    members
                        .iter()
                        .filter_map(|n| stacks.get(n).map(|s| (*s).clone()))
                        .collect()
                },
                layout_records: members
                    .iter()
                    .filter_map(|n| layout_records.get(n).map(|r| (*r).clone()))
                    .collect(),
            };
            // 가이드는 소유 node가 있는 chunk, View 변수는 View root chunk의 확장 절입니다(ADR-0126).
            let sections = SubtreeSections {
                node_guides: part
                    .sections
                    .node_guides
                    .iter()
                    .filter(|g| members.contains(&g.owner()))
                    .cloned()
                    .collect(),
                view_variants: if chunk.root == part.records.root {
                    part.sections.view_variants.clone()
                } else {
                    Vec::new()
                },
            };
            let payload = subtree.encode();
            let section = if subtree.layout_records.is_empty() {
                0
            } else {
                4
            };
            assert_eq!(
                payload.len() as u64,
                chunk.payload_bytes + section,
                "크기 모델 = 실제 payload 길이"
            );
            let payload = subtree.encode_with(&sections);
            let chunk_id = subtree_chunk_id(document, chunk.root);
            refs.push(SubtreeRef {
                root_node: chunk.root,
                chunk: chunk_id,
            });
            out.insert(chunk_id, (KIND_VIEW_TREE, payload));
        }
        assert_eq!(
            covered.len(),
            part.records.nodes.len(),
            "모든 node가 chunk에 속함"
        );
        let tree = ViewTreeChunk {
            view_root: part.records.root,
            subtrees: refs,
        };
        out.insert(
            view_tree_chunk_id(document, part.view),
            (KIND_VIEW_TREE, tree.encode().expect("ViewTree")),
        );
    }
    out
}

/// chunk model을 메모리에서 로드합니다.
fn load_from_memory(
    model: &DocumentModel,
    chunks: &BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)>,
) -> DocumentModel {
    let mut memory = MemoryChunks::new();
    for (chunk_id, (_, payload)) in chunks {
        memory.insert(*chunk_id, payload.clone());
    }
    let serialized = serialize_document(model).expect("직렬화");
    // 무작위 command에 Context 편집(XXX.7.8)이 있어 manifest가 context-set chunk를 가리킬 수 있습니다. 이 시험이 나누는
    // 것은 subtree chunk뿐이므로 context-set chunk는 직렬화 결과를 그대로 넣습니다.
    if let Some(context) = &serialized.context {
        memory.insert(context.chunk_id, context.payload.clone());
    }
    let manifest = serialized.manifest;
    load_document(&manifest.payload, &mut memory)
        .expect("chunk model 로드")
        .model
}

/// chunk model을 저장합니다. `previous`와 payload가 같은 chunk는 clean으로 넘깁니다. 기록한 chunk id 반환.
fn save_model(
    root: &Path,
    project_id: StableId,
    document: StableId,
    chunks: &BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)>,
    previous: &BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)>,
) -> BTreeSet<ChunkId> {
    let states: Vec<LogicalChunkState> = chunks
        .iter()
        .map(|(chunk_id, (kind, payload))| {
            if previous.get(chunk_id).is_some_and(|(_, p)| p == payload) {
                LogicalChunkState::Clean {
                    chunk_id: *chunk_id,
                }
            } else {
                LogicalChunkState::Dirty {
                    chunk_id: *chunk_id,
                    kind: *kind,
                    schema_version: DOCUMENT_CHUNK_SCHEMA_VERSION,
                    payload: payload.clone(),
                }
            }
        })
        .collect();
    let plan = save_states(root, project_id, &states, vec![document]);
    let pointers: BTreeSet<ChunkId> = plan
        .manifest
        .chunk_pointers
        .iter()
        .map(|p| p.chunk_id)
        .collect();
    assert_eq!(
        pointers,
        chunks.keys().copied().collect(),
        "generation chunk 집합 = chunk model"
    );
    plan.dirty_chunks.iter().map(|w| w.chunk_id).collect()
}

/// 디스크에서 다시 열어 로드합니다.
fn reopen(root: &Path, document: StableId) -> DocumentModel {
    SavedProjectLoader::new()
        .open(root)
        .expect("재열기")
        .load_document(document)
        .expect("문서 로드")
        .model
}

fn subtree_chunk_count(
    chunks: &BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)>,
    views: usize,
) -> usize {
    // manifest 1 + View마다 ViewTree 1.
    chunks.len() - 1 - views
}

/// section 6개 × card 8개(각 card: 제목 text + Fill modifier가 있는 사각형)를 command로 만든 문서.
fn catalog(editor: &mut Editor) {
    for s in 0..6u64 {
        let section = id(100 + s);
        let mut nodes = vec![frame(
            section,
            id(10),
            key(0x10 + s as u8),
            &format!("섹션 {s}"),
            (0.0, 200.0 * s as f32),
            (1440.0, 180.0),
        )];
        let mut stacks = Vec::new();
        for c in 0..8u64 {
            let card = id(1_000 + s * 100 + c * 10);
            nodes.push(frame(
                card,
                section,
                key(0x10 + c as u8),
                &format!("카드 {s}-{c}"),
                (170.0 * c as f32, 0.0),
                (160.0, 160.0),
            ));
            nodes.push(text(
                id(1_000 + s * 100 + c * 10 + 1),
                card,
                key(0x10),
                "제목",
                &format!("상품 {s}-{c}: 가볍고 튼튼한 디자인 소품\n두 번째 설명 줄 {c}"),
            ));
            let shape = id(1_000 + s * 100 + c * 10 + 2);
            nodes.push(rectangle(shape, card, key(0x20), "썸네일", 140.0, 90.0));
            let mut stack = ModifierStack::new(shape);
            stack
                .insert(default_modifier(
                    id(5_000 + s * 100 + c),
                    key(0x10),
                    ModifierKind::Fill,
                ))
                .expect("fill");
            stacks.push(stack);
        }
        editor.apply(
            vec![Command::InsertNode(InsertNode {
                nodes,
                modifier_stacks: stacks,
                layout_records: Vec::new(),
                node_guides: Vec::new(),
            })],
            "섹션 삽입",
        );
    }
    editor.apply(
        vec![Command::InsertNode(InsertNode::single(text(
            id(21),
            id(20),
            key(0x10),
            "안내",
            "작은 View",
        )))],
        "작은 View",
    );
}

#[test]
fn split_sticky_resplit_and_idle_merge_preserve_semantics_on_disk() {
    let sandbox = TempDir::new("chunks");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "chunk 프로젝트", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");
    let document = id(1);
    let mut editor = Editor::new(
        new_document(
            document,
            "카탈로그",
            &[(
                id(2),
                "페이지",
                vec![(id(3), "목록", id(10)), (id(4), "안내", id(20))],
            )],
        ),
        SPACE,
    );
    catalog(&mut editor);
    let views = 2;

    // 0. 승인 임계치(config)에서는 분할이 없고, 계획 layout의 chunk model이 기본 직렬화와 byte-identical.
    let budget = ChunkBudget::from_budget_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/performance-budgets.yaml"),
    )
    .expect("chunk 예산");
    let approved = SplitPolicy::from_budget(&budget, COST);
    let parts = partitions(editor.model());
    let single: BTreeMap<StableId, ChunkLayout> = parts
        .iter()
        .map(|p| {
            let plan = plan_chunk_split(&p.tree, &approved, &BTreeSet::new()).expect("분할 계획");
            assert_eq!(
                plan.layout.chunks().len(),
                1,
                "승인 임계치 이하 문서는 분할하지 않음"
            );
            (p.view, plan.layout)
        })
        .collect();
    let default_chunks: BTreeMap<ChunkId, (ChunkKindCode, Vec<u8>)> =
        serialize_document(editor.model())
            .expect("직렬화")
            .chunks()
            .into_iter()
            .map(|c| (c.chunk_id, (c.kind, c.payload.clone())))
            .collect();
    assert_eq!(chunk_model(editor.model(), &parts, &single), default_chunks);

    // 1. 작은 임계치로 분할 → 저장 → 재열기.
    let policy = policy(2_048);
    let mut layouts: BTreeMap<StableId, ChunkLayout> = parts
        .iter()
        .map(|p| {
            let plan = plan_chunk_split(&p.tree, &policy, &BTreeSet::new()).expect("분할 계획");
            assert!(plan.released_roots.is_empty());
            for chunk in plan.layout.chunks() {
                assert!(chunk.oversized || chunk.payload_bytes <= policy.threshold_bytes);
            }
            (p.view, plan.layout)
        })
        .collect();
    let list_view = id(3);
    let first_roots = layouts[&list_view].split_roots();
    assert!(
        first_roots.len() >= 6,
        "섹션 단위 이상으로 분할: {}",
        first_roots.len()
    );
    let gen1 = chunk_model(editor.model(), &parts, &layouts);
    assert_same_document(
        editor.model(),
        &load_from_memory(editor.model(), &gen1),
        "분할 메모리",
    );
    let written = save_model(&root, project_id, document, &gen1, &BTreeMap::new());
    assert_eq!(written.len(), gen1.len(), "첫 저장은 전부 기록");
    assert_same_document(editor.model(), &reopen(&root, document), "분할 재열기");

    // 2. chunk root 하나 삭제 + 다른 chunk 안 node 편집 → sticky 재분할 → 증분 저장.
    let victim = layouts[&list_view].chunks()[1].root;
    let edited = id(1_000 + 5 * 100 + 7 * 10 + 1);
    let edited_chunk = layouts[&list_view]
        .chunks()
        .iter()
        .find(|c| c.nodes.contains(&edited))
        .map(|c| c.root)
        .expect("편집 대상 chunk");
    let victim_subtree: BTreeSet<StableId> = layouts[&list_view]
        .chunks()
        .iter()
        .find(|c| c.root == victim)
        .map(|c| c.nodes.iter().copied().collect())
        .expect("삭제 chunk");
    assert!(!victim_subtree.contains(&edited));
    editor.apply(
        vec![
            Command::DeleteSubtree(DeleteSubtree { root: victim }),
            // 크기가 같은 편집이라 재분할이 layout을 바꾸지 않습니다.
            Command::set_visibility(edited, false),
        ],
        "삭제와 편집",
    );
    let parts = partitions(editor.model());
    let previous_roots = first_roots;
    let plan = plan_chunk_split(&parts[0].tree, &policy, &previous_roots).expect("재분할");
    let still_present: BTreeSet<StableId> = previous_roots
        .iter()
        .copied()
        .filter(|r| editor.model().contains(*r))
        .collect();
    let gone: Vec<StableId> = previous_roots
        .iter()
        .copied()
        .filter(|r| !editor.model().contains(*r))
        .collect();
    assert!(gone.contains(&victim));
    assert_eq!(plan.released_roots, gone, "삭제된 chunk root는 released");
    assert!(
        still_present.is_subset(&plan.layout.split_roots()),
        "남은 이전 root는 유지(sticky)"
    );
    layouts.insert(list_view, plan.layout);
    let gen2 = chunk_model(editor.model(), &parts, &layouts);
    let written = save_model(&root, project_id, document, &gen2, &gen1);
    assert_eq!(
        written,
        BTreeSet::from([
            view_tree_chunk_id(document, list_view),
            subtree_chunk_id(document, edited_chunk),
        ]),
        "ViewTree와 편집된 chunk만 기록"
    );
    assert_same_document(editor.model(), &reopen(&root, document), "재분할 재열기");

    // 3. 많은 card를 지워 chunk를 작게 만듭니다. 저장 경로는 병합하지 않습니다(sticky).
    let mut deletions = Vec::new();
    for s in 0..6u64 {
        for c in 1..8u64 {
            let card = id(1_000 + s * 100 + c * 10);
            if editor.model().contains(card) {
                deletions.push(Command::DeleteSubtree(DeleteSubtree { root: card }));
            }
        }
    }
    editor.apply(deletions, "card 정리");
    let parts = partitions(editor.model());
    let before_roots = layouts[&list_view].split_roots();
    let plan = plan_chunk_split(&parts[0].tree, &policy, &before_roots).expect("재분할");
    let shrunk_roots = plan.layout.split_roots();
    assert!(
        shrunk_roots.is_subset(&before_roots),
        "저장 경로 분할은 새 root를 만들지 않음(모두 임계치 이하)"
    );
    assert!(!shrunk_roots.is_empty());
    layouts.insert(list_view, plan.layout);
    let gen3 = chunk_model(editor.model(), &parts, &layouts);
    save_model(&root, project_id, document, &gen3, &gen2);
    assert_same_document(editor.model(), &reopen(&root, document), "축소 재열기");

    // 4. idle 병합: 저장과 배타, 결과 layout으로 저장 → 재열기.
    let t0 = Instant::now();
    let scheduler = GcScheduler::new(Duration::from_secs(5), t0);
    assert_eq!(
        run_idle_merge(&scheduler, t0, &parts[0].tree, &policy, &shrunk_roots).expect("병합"),
        IdleMergeOutcome::NotStarted,
        "idle threshold 전"
    );
    let idle = t0 + Duration::from_secs(10);
    {
        let window = begin_idle_merge(&scheduler, idle).expect("병합 창");
        let plan = window
            .plan(&parts[0].tree, &policy, &shrunk_roots)
            .expect("병합 계획");
        let _save = scheduler.begin_save(idle);
        assert_eq!(
            window.finish(plan),
            IdleMergeOutcome::Superseded,
            "겹친 저장"
        );
    }
    let idle = idle + Duration::from_secs(10);
    let merge = match run_idle_merge(&scheduler, idle, &parts[0].tree, &policy, &shrunk_roots)
        .expect("병합")
    {
        IdleMergeOutcome::Ready(plan) => plan,
        other => panic!("병합이 실행되어야 함: {other:?}"),
    };
    assert!(!merge.is_noop(), "작은 chunk는 병합 대상");
    merge.check_base(&shrunk_roots).expect("기준 layout");
    let remap = merge
        .remap_manifest(|root| subtree_chunk_id(document, root))
        .expect("remap");
    let merged_layout = merge.layout().clone();
    let live: BTreeSet<ChunkId> = merged_layout
        .chunks()
        .iter()
        .map(|c| subtree_chunk_id(document, c.root))
        .collect();
    remap.validate(&live).expect("remap 대상은 살아 있는 chunk");
    for absorption in merge.absorbed() {
        assert_eq!(
            remap.resolve(subtree_chunk_id(document, absorption.chunk_root)),
            Some(subtree_chunk_id(document, absorption.into))
        );
    }
    for chunk in merged_layout.chunks() {
        assert!(chunk.oversized || chunk.payload_bytes <= policy.threshold_bytes);
    }
    // 병합 결과는 다음 저장 분할의 고정점입니다(split/merge 진동 없음).
    let resplit =
        plan_chunk_split(&parts[0].tree, &policy, &merged_layout.split_roots()).expect("재분할");
    assert_eq!(resplit.layout, merged_layout);
    assert!(resplit.released_roots.is_empty());
    layouts.insert(list_view, merged_layout);
    let gen4 = chunk_model(editor.model(), &parts, &layouts);
    assert!(
        subtree_chunk_count(&gen4, views) < subtree_chunk_count(&gen3, views),
        "병합 뒤 subtree chunk 감소"
    );
    for absorption in merge.absorbed() {
        assert!(!gen4.contains_key(&subtree_chunk_id(document, absorption.chunk_root)));
    }
    save_model(&root, project_id, document, &gen4, &gen3);
    assert_same_document(editor.model(), &reopen(&root, document), "병합 재열기");

    // 5. 처음 상태까지 undo해도 각 단계 문서는 유효하고, 분할 chunk model로 왕복합니다.
    while editor.undo() {
        assert_valid(editor.model(), "undo");
    }
    let parts = partitions(editor.model());
    let layouts: BTreeMap<StableId, ChunkLayout> = parts
        .iter()
        .map(|p| {
            (
                p.view,
                plan_chunk_split(&p.tree, &policy, &BTreeSet::new())
                    .expect("분할")
                    .layout,
            )
        })
        .collect();
    let empty = chunk_model(editor.model(), &parts, &layouts);
    assert_same_document(
        editor.model(),
        &load_from_memory(editor.model(), &empty),
        "undo 끝",
    );
}

#[test]
fn random_documents_split_resplit_and_merge_round_trip() {
    // seed마다: random valid document를 작은 임계치로 분할 → 무작위 command 적용 → 이전 layout으로 sticky
    // 재분할 → idle 병합. 각 chunk model은 원본과 의미상 같고, 병합 결과는 다음 분할의 고정점입니다.
    // random 문서 Frame 레코드는 100에서 450 bytes 정도라, 병합 여지가 있도록 node 몇 개 크기로 잡습니다.
    let policy = policy(512);
    let mut split_chunks = 0usize;
    let mut released = 0usize;
    let mut absorbed = 0usize;
    for seed in 0..32u64 {
        let label = format!("seed {seed}");
        let mut editor = Editor::new(random_valid_document(seed), SPACE + 1 + seed);
        let initial: BTreeMap<StableId, BTreeSet<StableId>> = partitions(editor.model())
            .iter()
            .map(|p| {
                let plan = plan_chunk_split(&p.tree, &policy, &BTreeSet::new()).expect("분할");
                (p.view, plan.layout.split_roots())
            })
            .collect();
        split_chunks += initial.values().map(BTreeSet::len).sum::<usize>();

        let mut rng = SplitMix64::new(seed ^ 0xC4_0000);
        let mut ids = FreshIds::new(seed ^ 0xC4_0001);
        // node 편집 12번. 가이드·View 변수 command(ADR-0126)는 node 크기를 바꾸지 않으므로 적용하되 횟수에 넣지 않습니다(분포 유지).
        let mut node_edits = 0;
        for _ in 0..64 {
            if node_edits == 12 {
                break;
            }
            let Some(request) = random_valid_request(&editor.state, &mut rng, &mut ids, 32) else {
                node_edits += 1;
                continue;
            };
            if !matches!(
                request.command,
                Command::SetNodeGuides(_) | Command::EditViewVariants(_)
            ) {
                node_edits += 1;
            }
            // 검증을 통과한 후보도 전체 validator가 결과를 거부할 수 있습니다(원자 롤백).
            let before = editor.model().to_records().expect("레코드");
            if editor.try_apply(vec![request.command]).is_err() {
                assert_eq!(
                    editor.model().to_records().expect("레코드"),
                    before,
                    "{label}: 거부는 무변경"
                );
            }
        }
        // 절반쯤의 leaf를 지워 chunk를 작게 만듭니다(병합 후보). 외부 참조가 있으면 정책 오류로 거부됩니다.
        let leaves: Vec<StableId> = editor
            .model()
            .to_records()
            .expect("레코드")
            .nodes
            .iter()
            .filter(|n| n.parent.is_some() && editor.model().children_in_order(n.id).is_empty())
            .map(|n| n.id)
            .collect();
        for leaf in leaves {
            if rng.one_in(2) {
                let _ =
                    editor.try_apply(vec![Command::DeleteSubtree(DeleteSubtree { root: leaf })]);
            }
        }
        assert_valid(editor.model(), &label);

        let parts = partitions(editor.model());
        let mut resplit_layouts = BTreeMap::new();
        let mut merged_layouts = BTreeMap::new();
        for part in &parts {
            let previous = initial.get(&part.view).cloned().unwrap_or_default();
            let plan = plan_chunk_split(&part.tree, &policy, &previous).expect("재분할");
            let roots = plan.layout.split_roots();
            for root in &previous {
                let boundary = part
                    .tree
                    .nodes()
                    .iter()
                    .any(|n| n.id == *root && n.boundary);
                if boundary {
                    assert!(roots.contains(root), "{label}: 남은 root {root} 유지");
                } else {
                    assert!(
                        plan.released_roots.contains(root),
                        "{label}: {root} released"
                    );
                }
            }
            released += plan.released_roots.len();
            let merge = plan_chunk_merges(&part.tree, &policy, &roots).expect("병합");
            absorbed += merge.absorbed().len();
            let fixpoint = plan_chunk_split(&part.tree, &policy, &merge.layout().split_roots())
                .expect("재분할");
            assert_eq!(&fixpoint.layout, merge.layout(), "{label}: 병합 고정점");
            resplit_layouts.insert(part.view, plan.layout);
            merged_layouts.insert(part.view, merge.layout().clone());
        }
        let resplit = chunk_model(editor.model(), &parts, &resplit_layouts);
        assert_same_records(
            editor.model(),
            &load_from_memory(editor.model(), &resplit),
            &format!("{label}: 재분할"),
        );
        let merged = chunk_model(editor.model(), &parts, &merged_layouts);
        assert_same_document(
            editor.model(),
            &load_from_memory(editor.model(), &merged),
            &format!("{label}: 병합"),
        );
    }
    // 분포 회귀 방지: 분할·release·병합이 실제로 일어났는지.
    assert!(split_chunks > 200, "분할 chunk {split_chunks}");
    assert!(released > 10, "released {released}");
    assert!(absorbed > 10, "병합 흡수 {absorbed}");
}

/// 알려진 결함(연결 누락, `docs/task-reports/V.6.4.md` "남은 위험 / 한계" 첫 항목): document_io
/// serializer(`serialize_document`)는 V.6.4 split planner를 쓰지 않고 View마다 subtree chunk를 하나만
/// 씁니다. 그래서 승인 임계치(ADR-0012,
/// `storage.chunk_split_threshold_bytes`)를 넘는 View도 의미 경계(Frame)로 나뉘지 않은 단일 chunk로
/// 저장됩니다. 위 테스트들은 planner layout을 테스트에서 직접 chunk model로 옮겨 검증했습니다. serializer가
/// planner와 연결되면 `#[ignore]`를 지웁니다.
#[test]
#[ignore = "결함: document_io serializer가 V.6.4 split planner와 연결되지 않아 View당 subtree chunk 1개로 고정"]
fn product_serializer_splits_views_over_approved_threshold() {
    let budget = ChunkBudget::from_budget_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/performance-budgets.yaml"),
    )
    .expect("chunk 예산");
    let mut editor = Editor::new(
        new_document(
            id(1),
            "큰 문서",
            &[(id(2), "페이지", vec![(id(3), "목록", id(10))])],
        ),
        SPACE + 500,
    );
    // section 40개 × 본문 text 25개(각 300 bytes 이상) = 300KiB 이상인 View.
    let body = "가".repeat(100);
    let mut commands = Vec::new();
    for s in 0..40u64 {
        let section = id(100_000 + s * 1_000);
        let order =
            OrderKey::from_bytes(vec![0x10 + (s / 16) as u8, 0x10 + (s % 16) as u8]).expect("key");
        let mut nodes = vec![frame(
            section,
            id(10),
            order,
            "섹션",
            (0.0, 0.0),
            (100.0, 100.0),
        )];
        for c in 0..25u64 {
            nodes.push(text(
                id(100_000 + s * 1_000 + c + 1),
                section,
                key(0x10 + c as u8),
                "본문",
                &body,
            ));
        }
        commands.push(Command::InsertNode(InsertNode {
            nodes,
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
            node_guides: Vec::new(),
        }));
    }
    editor.apply(commands, "큰 View");

    let approved = SplitPolicy::from_budget(&budget, COST);
    let parts = partitions(editor.model());
    let plan = plan_chunk_split(&parts[0].tree, &approved, &BTreeSet::new()).expect("분할 계획");
    assert!(plan.layout.chunks().len() > 1, "planner는 분할을 요구");
    let serialized = serialize_document(editor.model()).expect("직렬화");
    for subtree in &serialized.subtrees {
        assert!(
            subtree.payload.len() as u64 <= budget.split_threshold_bytes,
            "subtree chunk {} bytes > 임계치 {} bytes",
            subtree.payload.len(),
            budget.split_threshold_bytes
        );
    }
}
