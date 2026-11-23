//! 종단 시나리오: 새 프로젝트 생성 → command transaction과 창 history로 문서 구성(undo/redo 포함) →
//! document_io 직렬화와 storage 저장 transaction으로 디스크 저장 → 디스크에서 lazy 재열기 → 의미
//! 동일성·validator 확인 → 증분 저장 → Git 커밋·clone 뒤 재열기 → 재열기 후 편집 undo가 처음 저장
//! bytes로 돌아오는지.

mod common;

use std::collections::BTreeSet;

use common::builders::{
    color, default_modifier, ellipse, fragment, frame, image, line, modifier, new_document, path,
    rectangle, text,
};
use common::{
    assert_same_document, assert_valid, git_ok, hermetic_git, key, save_documents, sid, Editor,
    TempDir,
};
use stellar_commands::command::{Command, CommandError};
use stellar_commands::convert::{ConvertFragmentToFrame, ConvertShapeToPath};
use stellar_commands::duplicate::DuplicateSubtree;
use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
use stellar_commands::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::selection::Selection;
use stellar_commands::state::DocumentState;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::{StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Size2, Vec2};
use stellar_document::frame::{keys as frame_keys, Clipping};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    BooleanOp, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::modifier_metadata::keys as modifier_keys;
use stellar_document::records::NodePayload;
use stellar_history::entry::HistoryError;
use stellar_project::create::{create_project, ProjectCreateRequest, ProjectCreated};
use stellar_project::document_io::load::load_document_from_generation;
use stellar_project::document_io::serialize::{subtree_chunk_id, SerializedDocument};
use stellar_project::identity::parse_project_id;
use stellar_project::probe::probe_project;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};

const SPACE: u64 = 11;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

fn id(n: u64) -> StableId {
    sid(SPACE, n)
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

/// 문서 A: 페이지 2개, View 3개(홈 10, 설정 20, 빈 화면 30).
fn landing_document() -> DocumentModel {
    new_document(
        id(1),
        "랜딩",
        &[
            (
                id(2),
                "페이지 1",
                vec![(id(3), "홈", id(10)), (id(4), "설정", id(20))],
            ),
            (id(5), "보관", vec![(id(6), "빈 화면", id(30))]),
        ],
    )
}

/// 문서 B: View 하나(root 110).
fn icon_document() -> DocumentModel {
    new_document(
        id(101),
        "아이콘 시트",
        &[(id(102), "아이콘", vec![(id(103), "기본", id(110))])],
    )
}

/// 편집 단계마다의 (semantic snapshot, 선택) 기록.
struct Timeline {
    states: Vec<(String, Selection)>,
}

impl Timeline {
    fn new(editor: &Editor) -> Timeline {
        Timeline {
            states: vec![(editor.snapshot(), editor.selection())],
        }
    }

    fn push(&mut self, editor: &Editor) {
        self.states.push((editor.snapshot(), editor.selection()));
    }

    fn assert_at(&self, index: usize, editor: &Editor, label: &str) {
        let (snapshot, selection) = &self.states[index];
        assert_eq!(&editor.snapshot(), snapshot, "{label}: 문서 #{index}");
        assert_eq!(&editor.selection(), selection, "{label}: 선택 #{index}");
    }
}

/// 문서 A를 command로 구성합니다. 반환: 편집기, 단계별 기록, 복제본 root id.
fn build_landing(editor: &mut Editor) -> (Timeline, StableId) {
    let mut timeline = Timeline::new(editor);
    let mut next_element = 500u64;
    let mut element = move || {
        next_element += 1;
        id(next_element)
    };

    // 1. frame·text·shape·image·fragment subtree, clip용 path, 다른 View의 text를 한 transaction으로.
    let mut bg_stack = ModifierStack::new(id(13));
    bg_stack
        .insert(default_modifier(id(200), key(0x10), ModifierKind::Fill))
        .expect("fill");
    let card = InsertNode {
        nodes: vec![
            frame(
                id(11),
                id(10),
                key(0x40),
                "카드",
                (24.0, 48.0),
                (320.0, 200.0),
            ),
            text(
                id(12),
                id(11),
                key(0x10),
                "제목",
                "안녕하세요\n두 번째 줄 👩‍👩‍👧",
            ),
            rectangle(id(13), id(11), key(0x20), "배경", 100.0, 40.0),
            image(id(14), id(11), key(0x30), "아바타", id(900)),
            fragment(id(15), id(11), key(0x40), "그룹"),
            ellipse(id(16), id(15), key(0x10), "점", (10.0, 10.0), 12.0),
            line(id(17), id(15), key(0x20), "구분선", 80.0),
        ],
        modifier_stacks: vec![bg_stack],
        layout_records: Vec::new(),
    };
    let changes = editor.apply(
        vec![
            Command::InsertNode(card),
            Command::InsertNode(InsertNode::single(path(
                id(18),
                id(10),
                key(0x80),
                "clip 곡선",
                &mut element,
            ))),
            Command::InsertNode(InsertNode::single(text(
                id(21),
                id(20),
                key(0x10),
                "설정 제목",
                "설정",
            ))),
        ],
        "삽입",
    );
    assert_eq!(changes.inserted_nodes().len(), 9);
    timeline.push(editor);

    // 2. modifier 삽입과 modifier 속성 변경.
    editor.apply(
        vec![
            Command::InsertModifier(InsertModifier {
                node: id(13),
                entry: default_modifier(id(201), key(0x20), ModifierKind::Stroke),
            }),
            Command::InsertModifier(InsertModifier {
                node: id(13),
                entry: modifier(
                    id(202),
                    key(0x30),
                    ModifierPayload::Boolean {
                        op: BooleanOp::Subtract,
                        operands: vec![NodeRef::new(id(16))],
                    },
                ),
            }),
            Command::InsertModifier(InsertModifier {
                node: id(14),
                entry: modifier(
                    id(203),
                    key(0x10),
                    ModifierPayload::Mask {
                        source: NodeRef::new(id(18)),
                    },
                ),
            }),
            Command::InsertModifier(InsertModifier {
                node: id(11),
                entry: default_modifier(id(204), key(0x10), ModifierKind::Shadow),
            }),
            Command::SetModifierProperty(SetModifierProperty {
                node: id(13),
                modifier: ModifierId::new(id(200)),
                key: modifier_keys::FILL_COLOR,
                value: PropertyValue::Color(color(0.2, 0.4, 0.9)),
            }),
            Command::SetModifierProperty(SetModifierProperty {
                node: id(11),
                modifier: ModifierId::new(id(204)),
                key: modifier_keys::SHADOW_OFFSET,
                value: PropertyValue::Offset(Vec2::new(0.0, 4.0).expect("오프셋")),
            }),
        ],
        "modifier",
    );
    timeline.push(editor);

    // 3. Frame 속성과 이름.
    editor.apply(
        vec![
            Command::SetProperty(SetProperty {
                node: id(11),
                key: frame_keys::SIZE,
                value: PropertyValue::Size(Size2::new(360.0, 220.0).expect("크기")),
            }),
            Command::SetProperty(SetProperty {
                node: id(11),
                key: frame_keys::CLIPPING,
                value: PropertyValue::Clipping(Clipping::Path(NodeRef::new(id(18)))),
            }),
            Command::SetProperty(SetProperty {
                node: id(11),
                key: frame_keys::BACKGROUND,
                value: PropertyValue::OptionalColor(Some(color(0.98, 0.97, 0.95))),
            }),
            Command::rename_node(id(12), "카드 제목"),
        ],
        "속성",
    );
    timeline.push(editor);

    // 4. 선택(문서 편집과 같은 history stack).
    assert!(editor.select(&[id(11), id(12)]));
    timeline.push(editor);

    // 5. 카드 subtree를 다른 View로 복제(새 id remap).
    let duplicate = DuplicateSubtree::plan(
        editor.model(),
        id(11),
        id(20),
        key(0x20),
        &mut StableIdGenerator::new(0xD0D0),
        &ManualClock::new(5_900_000_000_000 * 1_000_000),
    )
    .expect("복제 계획");
    let copy_root = duplicate.copy_root().expect("복제 root");
    editor.apply(vec![Command::DuplicateSubtree(duplicate)], "복제");
    assert_eq!(
        editor.model().children_in_order(copy_root).len(),
        editor.model().children_in_order(id(11)).len()
    );
    timeline.push(editor);

    // 6. 종류 변환: 타원 → path, fragment → frame.
    let NodePayload::Shape(geometry) = editor.model().payload(id(16)).expect("타원").clone()
    else {
        panic!("타원은 Shape");
    };
    let element_ids = (0..ConvertShapeToPath::required_ids(&geometry).expect("평가 가능한 타원"))
        .map(|_| element())
        .collect();
    editor.apply(
        vec![
            Command::ConvertShapeToPath(ConvertShapeToPath {
                node: id(16),
                element_ids,
            }),
            Command::ConvertFragmentToFrame(ConvertFragmentToFrame { node: id(15) }),
        ],
        "변환",
    );
    timeline.push(editor);

    // 7. 재정렬·표시·잠금.
    editor.apply(
        vec![
            Command::MoveNode(MoveNode {
                node: id(12),
                new_parent: id(11),
                order_key: OrderKey::between(Some(&key(0x30)), Some(&key(0x40))),
            }),
            Command::set_visibility(id(17), false),
            Command::set_locked(id(13), true),
        ],
        "재정렬",
    );
    timeline.push(editor);

    // 8. 외부 참조가 있는 path 삭제는 정책 오류: 문서·history·revision 그대로.
    let before = editor.snapshot();
    let revision = editor.state.revision();
    let entries = editor.window.history().len();
    match editor.try_apply(vec![Command::DeleteSubtree(DeleteSubtree { root: id(18) })]) {
        Err(HistoryError::Transaction(failure)) => assert!(
            matches!(failure.error, CommandError::ExternalReference { .. }),
            "{failure}"
        ),
        other => panic!("외부 참조 삭제는 거부여야 함: {other:?}"),
    }
    assert_eq!(editor.snapshot(), before);
    assert_eq!(editor.state.revision(), revision);
    assert_eq!(editor.window.history().len(), entries);

    // 9. 선택된 node 삭제는 선택을 정리합니다(system event, 항목 없음).
    editor.apply(
        vec![Command::DeleteSubtree(DeleteSubtree { root: id(12) })],
        "삭제",
    );
    assert_eq!(editor.selection().ids(), &[id(11)]);
    timeline.push(editor);
    (timeline, copy_root)
}

#[test]
fn new_project_edit_save_reopen_round_trip() {
    let sandbox = TempDir::new("lifecycle");
    let Some((runner, version)) = hermetic_git(&sandbox.path) else {
        eprintln!("Git 2.28 이상이 없어 건너뜁니다");
        return;
    };

    // 1. New Project 흐름: 검증 → 폴더 → scaffold → git init → 최초 커밋.
    let request = ProjectCreateRequest {
        parent_dir: sandbox.path.clone(),
        folder_name: "landing-project".to_string(),
        project_name: "랜딩 프로젝트".to_string(),
    };
    let root = match create_project(&request, PROJECT_ID, &runner, &version).expect("생성") {
        ProjectCreated::Created { path } => path,
        other => panic!("생성이 끝까지 완료되어야 함: {other:?}"),
    };
    let probe = probe_project(&root).expect("probe");
    assert_eq!(probe.manifest.project.name, "랜딩 프로젝트");
    let project_id = parse_project_id(&probe.manifest.project.id).expect("project id");

    // 2. command·history로 두 문서를 만듭니다.
    let mut landing = Editor::new(landing_document(), SPACE);
    let (timeline, copy_root) = build_landing(&mut landing);
    let last = timeline.states.len() - 1;
    timeline.assert_at(last, &landing, "편집 직후");

    // undo 3번(삭제·재정렬·변환)과 redo 2번은 기록된 단계와 정확히 같아야 합니다.
    for k in 1..=3 {
        assert!(landing.undo());
        timeline.assert_at(last - k, &landing, "undo");
        assert_valid(landing.model(), "undo");
    }
    for k in (1..=2).rev() {
        assert!(landing.redo());
        timeline.assert_at(last - k, &landing, "redo");
    }
    assert_eq!(landing.window.history().redo_len(), 1);
    // 새 편집은 redo 갈래를 폐기합니다.
    landing.apply(vec![Command::rename_node(id(21), "환경 설정")], "새 편집");
    assert_eq!(landing.window.history().redo_len(), 0);
    let diagnostics = assert_valid(landing.model(), "최종");
    assert!(
        diagnostics.unresolved.is_empty(),
        "미해결 참조 없음: {:?}",
        diagnostics.unresolved
    );
    assert!(landing.model().contains(copy_root));
    assert!(landing.model().contains(id(12)), "삭제 undo 뒤 제목 복원");

    let mut icons = Editor::new(icon_document(), SPACE + 1);
    icons.apply(
        vec![
            Command::InsertNode(InsertNode::single(frame(
                id(111),
                id(110),
                key(0x10),
                "아이콘 틀",
                (0.0, 0.0),
                (24.0, 24.0),
            ))),
            Command::InsertNode(InsertNode::single(rectangle(
                id(112),
                id(111),
                key(0x10),
                "모양",
                20.0,
                20.0,
            ))),
        ],
        "아이콘",
    );

    // 3. 저장(generation 1): document_io 직렬화 → 저장 계획 → 저장 transaction.
    let first = save_documents(&root, project_id, &[landing.model(), icons.model()], &[]);
    assert_eq!(first.plan.target_generation, 1);
    let all_chunks: usize = first.documents.iter().map(|d| d.chunks().len()).sum();
    assert_eq!(first.plan.dirty_chunks.len(), all_chunks);

    // 4. 디스크에서 재열기: 열기는 DocumentManifest chunk만, 문서 로드는 그 문서 chunk만 읽습니다.
    let mut open_reads = Recorder::default();
    let lazy = LazyGeneration::open(&root, project_id, &mut open_reads).expect("lazy 열기");
    assert_eq!(open_reads.reads.len(), 2, "열기 때 manifest chunk만");
    let mut reads = Recorder::default();
    let loaded = load_document_from_generation(&lazy, id(1), &mut reads).expect("문서 A 로드");
    let expected: BTreeSet<ChunkId> = own_content_chunks(&first.documents[0]);
    assert_eq!(
        reads.reads.iter().copied().collect::<BTreeSet<_>>(),
        expected,
        "문서 A chunk만 읽음"
    );
    assert_same_document(landing.model(), &loaded.model, "generation 1 재열기");
    assert_eq!(loaded.diagnostics, diagnostics);

    let saved = SavedProjectLoader::new()
        .open(&root)
        .expect("저장된 프로젝트");
    assert_eq!(saved.generation_number(), 1);
    assert_eq!(
        saved
            .document_ids()
            .expect("문서 id")
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([id(1), id(101)])
    );
    let reopened_icons = saved.load_document(id(101)).expect("문서 B 로드");
    assert_same_document(icons.model(), &reopened_icons.model, "문서 B 재열기");

    // 5. 재열기한 문서를 새 창에서 이어 편집: 설정 View만 바꾸면 그 subtree chunk 하나만 다시 씁니다.
    let mut reopened = Editor::with_state(DocumentState::new(loaded.model), SPACE + 2);
    reopened.apply(
        vec![
            Command::rename_node(id(21), "고급 설정"),
            Command::InsertNode(InsertNode::single(rectangle(
                id(22),
                id(20),
                key(0x30),
                "구역",
                200.0,
                2.0,
            ))),
        ],
        "재열기 후 편집",
    );
    let second = save_documents(
        &root,
        project_id,
        &[reopened.model(), icons.model()],
        &first.documents,
    );
    assert_eq!(second.plan.target_generation, 2);
    let rewritten: Vec<ChunkId> = second
        .plan
        .dirty_chunks
        .iter()
        .map(|w| w.chunk_id)
        .collect();
    assert_eq!(
        rewritten,
        vec![subtree_chunk_id(id(1), id(20))],
        "바뀐 chunk만 기록"
    );
    let saved = SavedProjectLoader::new().open(&root).expect("generation 2");
    assert_eq!(saved.generation_number(), 2);
    let gen2 = saved.load_document(id(1)).expect("generation 2 로드");
    assert_same_document(reopened.model(), &gen2.model, "generation 2 재열기");

    // 6. chunk cache는 Git 밖입니다(ADR-0122). 문서 두 개를 담은 이 프로젝트는 텍스트 원본(root 문서 하나)으로 쓸 수 없어
    //    cache에만 저장되므로, 저장은 Git 작업 트리를 바꾸지 않고 clone에는 cache가 오지 않습니다.
    let status = git_ok(
        &runner,
        &root,
        ["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(
        status.stdout.is_empty(),
        "cache 저장 뒤 작업 트리 clean: {}",
        String::from_utf8_lossy(&status.stdout)
    );
    let clone = sandbox.path.join("landing-clone");
    git_ok(
        &runner,
        &sandbox.path,
        [
            std::ffi::OsStr::new("clone"),
            std::ffi::OsStr::new("-q"),
            root.as_os_str(),
            clone.as_os_str(),
        ],
    );
    probe_project(&clone).expect("clone probe");
    assert!(!stellar_storage::generation::current::current_pointer_path(&clone).exists());
    assert!(!clone.join(".stellar/cache/chunks").exists());

    // 7. 재열기 뒤 편집을 undo하고 저장하면 generation 1과 같은 chunk 집합(같은 content hash)이 됩니다.
    assert!(reopened.undo());
    assert!(!reopened.undo(), "재열기 전 history는 이어지지 않음");
    let third = save_documents(
        &root,
        project_id,
        &[reopened.model(), icons.model()],
        &second.documents,
    );
    assert_eq!(third.plan.target_generation, 3);
    assert_eq!(
        third.plan.manifest.chunk_pointers, first.plan.manifest.chunk_pointers,
        "undo 뒤 저장은 처음 저장과 같은 chunk"
    );
}

/// 문서의 ViewTree·subtree chunk id(manifest 제외).
fn own_content_chunks(document: &SerializedDocument) -> BTreeSet<ChunkId> {
    document
        .view_trees
        .iter()
        .chain(&document.subtrees)
        .map(|c| c.chunk_id)
        .collect()
}
