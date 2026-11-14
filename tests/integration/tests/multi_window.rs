//! 종단 시나리오: 같은 저장 문서를 두 창이 따로 열어 편집합니다(기술 아키텍처 §22.2, VII.5).
//!
//! - 창마다 인메모리 문서와 undo/redo history가 독립입니다.
//! - 먼저 저장한 창이 generation을 올리면, 옛 generation을 base로 계획한 다른 창의 저장은 optimistic check
//!   (V.5.2)에 걸려 아무 chunk 파일도 만들지 않고 거부됩니다.
//! - 두 창의 겹치지 않는 편집을 3-way semantic merge(XIV.10)하면, 한 창의 transaction을 다른 창의 저장
//!   결과 위에 rebase해 순서대로 실행한 문서와 같은 의미 snapshot이 됩니다. 그 문서를 새 base 위에
//!   저장하면 디스크 재열기가 같습니다.

mod common;

use std::path::Path;

use common::builders::{default_modifier, frame, new_document, rectangle, text};
use common::{assert_same_document, key, save_documents, sid, Editor, TempDir};
use stellar_commands::command::Command;
use stellar_commands::modifier_ops::InsertModifier;
use stellar_commands::node_ops::InsertNode;
use stellar_commands::state::DocumentState;
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierKind;
use stellar_history::entry::HistoryPayload;
use stellar_history::window::WindowHistories;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::scaffold::write_default_scaffold;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::MergeInput;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

const SPACE: u64 = 61;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb96a";

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

fn semantic(model: &DocumentModel) -> SemanticSnapshot {
    let mut builder = SnapshotBuilder::new();
    builder
        .add_document(&model.to_records().expect("레코드"))
        .expect("semantic snapshot");
    builder.finish()
}

fn open_saved(root: &Path) -> DocumentModel {
    SavedProjectLoader::new()
        .open(root)
        .expect("열기")
        .load_document(id(1))
        .expect("로드")
        .model
}

fn object_files(root: &Path) -> usize {
    fn walk(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        let path = entry.path();
                        if path.is_dir() {
                            walk(&path)
                        } else {
                            1
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    walk(&root.join("design"))
}

#[test]
fn two_windows_edit_independently_and_stale_save_is_rejected() {
    let sandbox = TempDir::new("windows");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "두 창", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");

    // generation 1: 홈·설정 View를 가진 문서.
    let mut seed = Editor::new(
        new_document(
            id(1),
            "공유 문서",
            &[(
                id(2),
                "페이지",
                vec![(id(3), "홈", id(10)), (id(4), "설정", id(20))],
            )],
        ),
        SPACE,
    );
    seed.apply(
        vec![
            Command::InsertNode(InsertNode::single(frame(
                id(11),
                id(10),
                key(0x10),
                "히어로",
                (0.0, 0.0),
                (1440.0, 400.0),
            ))),
            Command::InsertNode(InsertNode::single(text(
                id(21),
                id(20),
                key(0x10),
                "설정 제목",
                "설정",
            ))),
        ],
        "초기 구성",
    );
    let gen1 = save_documents(&root, project_id, &[seed.model()], &[]);
    let base_model = open_saved(&root);

    // 두 창이 같은 generation을 따로 엽니다.
    let mut a = Editor::new(open_saved(&root), SPACE + 1);
    let mut b = Editor::new(open_saved(&root), SPACE + 2);
    let mut registry = WindowHistories::new();
    registry.open(a.window.window());
    registry.open(b.window.window());
    assert_ne!(a.window.window(), b.window.window());

    // 창 A는 홈 View, 창 B는 설정 View를 편집합니다(겹치지 않음).
    a.apply(
        vec![
            Command::InsertNode(InsertNode::single(rectangle(
                id(12),
                id(11),
                key(0x10),
                "배너",
                600.0,
                200.0,
            ))),
            Command::rename_node(id(11), "메인 히어로"),
        ],
        "창 A",
    );
    assert!(a.select(&[id(12)]));
    b.apply(
        vec![
            Command::InsertNode(InsertNode::single(text(
                id(22),
                id(20),
                key(0x20),
                "설명",
                "알림 설정을 바꿉니다",
            ))),
            Command::InsertModifier(InsertModifier {
                node: id(21),
                entry: default_modifier(id(300), key(0x10), ModifierKind::Opacity),
            }),
        ],
        "창 B",
    );
    b.apply(vec![Command::set_locked(id(21), true)], "창 B 잠금");
    // 창 history는 서로 섞이지 않습니다.
    assert_eq!(a.window.history().len(), 2);
    assert_eq!(b.window.history().len(), 2);
    assert!(!b.model().contains(id(12)));
    assert!(!a.model().contains(id(22)));

    // 창 B가 generation 1을 base로 저장 계획을 세운 사이 창 A가 먼저 저장합니다.
    let stale_base = load_current_generation(&root, project_id).expect("generation 1");
    let b_serialized = serialize_document(b.model()).expect("직렬화");
    let b_states: Vec<LogicalChunkState> = b_serialized.to_dirty_states();
    let b_plan = build_save_plan(
        Some(&stale_base),
        project_id,
        common::GENERATION_SCHEMA_VERSION,
        &b_states,
        vec![id(1)],
    )
    .expect("B 계획");
    let gen2 = save_documents(&root, project_id, &[a.model()], &gen1.documents);
    assert_eq!(gen2.plan.target_generation, 2);
    assert_eq!(
        registry.on_generation_saved(a.window.window()),
        vec![b.window.window()],
        "다른 창은 stale 후보"
    );
    let current = read_current_pointer(&root).expect("current");
    let files = object_files(&root);
    let mut memory = SavedGenerationState::new(Some((
        stale_base.manifest.generation,
        stale_base.manifest_hash,
    )));
    assert!(
        run_save(&root, &b_plan, &mut memory, &CancelToken::new()).is_err(),
        "옛 base 저장은 거부"
    );
    assert_eq!(read_current_pointer(&root).expect("current"), current);
    assert_eq!(
        object_files(&root),
        files,
        "거부된 저장은 파일을 만들지 않음"
    );
    assert_same_document(a.model(), &open_saved(&root), "창 A 저장 유지");

    // 3-way semantic merge(base = generation 1, ours = A, theirs = B)는 충돌 없이 합쳐집니다.
    let result = merge(&MergeInput {
        base: semantic(&base_model),
        ours: semantic(a.model()),
        theirs: semantic(b.model()),
    })
    .expect("merge");
    assert!(result.is_clean(), "{:?}", result.conflicts);

    // B의 transaction을 저장된 A 문서 위에 rebase해 순서대로 실행한 결과와 같습니다.
    let mut rebased = DocumentState::new(open_saved(&root));
    for entry in b.window.history().entries() {
        if let HistoryPayload::Document { forward, .. } = &entry.payload {
            rebased
                .execute_transaction(&forward.rebased(rebased.revision()))
                .expect("rebase 실행");
        }
    }
    assert_eq!(
        result.merged,
        semantic(rebased.model()),
        "merge = rebase 순차 실행"
    );

    // 합친 문서를 새 base(generation 2) 위에 저장 → 재열기.
    let merged_model = rebased.into_model();
    let gen3 = save_documents(&root, project_id, &[&merged_model], &gen2.documents);
    assert_eq!(gen3.plan.target_generation, 3);
    assert_same_document(&merged_model, &open_saved(&root), "합친 문서 재열기");

    // 창 B의 undo는 B 문서만 되돌립니다(창 A 문서·디스크와 무관).
    while b.undo() {}
    assert_same_document(&base_model, b.model(), "창 B 전체 undo");
    assert!(a.model().contains(id(12)));
    assert!(registry.close(b.window.window()));
}
