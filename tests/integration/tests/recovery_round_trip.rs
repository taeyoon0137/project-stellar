//! 종단 시나리오: 자동 저장 세대 → 미저장 편집 → 복구 스냅샷(V.9) 기록 → 비정상 종료 → 다음 실행의 최신성
//! probe → 저장 세대 + 미저장 chunk overlay로 문서 복원 → 의미 동일성 → 복구 내용 저장 → 스냅샷이
//! 오래됨으로 바뀌고 정상 종료 정리.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::builders::{default_modifier, frame, new_document, rectangle, text};
use common::{assert_same_document, key, save_documents, sid, Editor, TempDir};
use stellar_commands::command::Command;
use stellar_commands::modifier_ops::InsertModifier;
use stellar_commands::node_ops::InsertNode;
use stellar_commands::selection::{Selection, WindowViewState};
use stellar_commands::state::DocumentState;
use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierKind;
use stellar_project::document_io::load::{
    find_document_manifest, load_document, ChunkSource, ChunkSourceError, LazySource,
};
use stellar_project::document_io::serialize::{
    manifest_chunk_id, serialize_document, SerializedDocument,
};
use stellar_project::identity::{parse_project_id, ProjectIdentity};
use stellar_project::recovery::RecoverySnapshotStore;
use stellar_project::recovery_payload::{OpenTab, RecoveryPayload, UnsavedChunk, Viewport};
use stellar_project::recovery_probe::{
    assess_snapshot, probe_recovery_snapshots, SnapshotFreshness, StaleReason,
};
use stellar_project::recovery_shutdown::{
    begin_window_session, end_window_session_clean, was_clean_shutdown,
};
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::scaffold::write_default_scaffold;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::generation::lazy::NoopRecorder;
use stellar_storage::hash::ContentHash;

const SPACE: u64 = 21;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

fn document() -> DocumentModel {
    new_document(
        id(1),
        "복구 문서",
        &[(
            id(2),
            "페이지",
            vec![(id(3), "홈", id(10)), (id(4), "상세", id(20))],
        )],
    )
}

/// 저장된 generation 위에 미저장 chunk를 덮어 읽는 chunk 원천(복구 경로).
struct Overlay<'a> {
    unsaved: BTreeMap<ChunkId, Vec<u8>>,
    base: LazySource<'a>,
}

impl ChunkSource for Overlay<'_> {
    fn read_chunk(&mut self, chunk: ChunkId) -> Result<Vec<u8>, ChunkSourceError> {
        match self.unsaved.get(&chunk) {
            Some(payload) => Ok(payload.clone()),
            None => self.base.read_chunk(chunk),
        }
    }
}

/// 마지막 저장 직렬화와 payload가 다른 chunk만 미저장 chunk로 담은 복구 payload.
fn capture(
    editor: &Editor,
    saved: &[SerializedDocument],
    base: Option<ContentHash>,
) -> RecoveryPayload {
    let saved: BTreeMap<ChunkId, &Vec<u8>> = saved
        .iter()
        .flat_map(|d| d.chunks())
        .map(|c| (c.chunk_id, &c.payload))
        .collect();
    let current = serialize_document(editor.model()).expect("직렬화");
    let unsaved_chunks = current
        .chunks()
        .into_iter()
        .filter(|c| saved.get(&c.chunk_id) != Some(&&c.payload))
        .map(|c| UnsavedChunk {
            chunk_id: c.chunk_id,
            kind: c.kind,
            payload: c.payload.clone(),
        })
        .collect();
    RecoveryPayload {
        base_manifest_hash: base,
        revision: editor.state.revision().0,
        unsaved_chunks,
        open_tabs: vec![
            OpenTab {
                view: id(3),
                viewport: Viewport {
                    center_x: 360.0,
                    center_y: -120.5,
                    zoom: 1.5,
                },
            },
            OpenTab {
                view: id(4),
                viewport: Viewport {
                    center_x: 0.0,
                    center_y: 0.0,
                    zoom: 0.25,
                },
            },
        ],
        active_tab: 1,
        selection: editor.selection().ids().to_vec(),
        target_profile: id(700),
    }
}

#[test]
fn autosave_crash_recovery_snapshot_round_trip() {
    let sandbox = TempDir::new("recovery");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "복구 프로젝트", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");
    let app_data = sandbox.path.join("app-data");
    let store = RecoverySnapshotStore::new(&app_data);
    let identity = ProjectIdentity::new(project_id, &root).expect("identity");

    // 1. 편집 후 자동 저장 flush(generation 1).
    let mut editor = Editor::new(document(), SPACE);
    let window = editor.window.window();
    begin_window_session(&store, &identity, window).expect("세션 시작");
    editor.apply(
        vec![
            Command::InsertNode(InsertNode::single(frame(
                id(11),
                id(10),
                key(0x10),
                "헤더",
                (0.0, 0.0),
                (1440.0, 80.0),
            ))),
            Command::InsertNode(InsertNode::single(text(
                id(21),
                id(20),
                key(0x10),
                "본문",
                "상세 설명",
            ))),
        ],
        "초기 편집",
    );
    let saved = save_documents(&root, project_id, &[editor.model()], &[]);
    let gen1 = read_current_pointer(&root).expect("current pointer");
    assert!(
        probe_recovery_snapshots(&store, &identity, &root)
            .expect("probe")
            .is_empty(),
        "스냅샷 없음"
    );

    // 2. 저장 뒤 미저장 편집과 선택. 홈 View만 바꿉니다.
    editor.apply(
        vec![
            Command::InsertNode(InsertNode::single(rectangle(
                id(12),
                id(11),
                key(0x10),
                "로고",
                48.0,
                48.0,
            ))),
            Command::InsertModifier(InsertModifier {
                node: id(11),
                entry: default_modifier(id(300), key(0x10), ModifierKind::Shadow),
            }),
        ],
        "미저장 편집",
    );
    editor.apply(vec![Command::rename_node(id(11), "상단 바")], "이름");
    assert!(editor.select(&[id(12), id(11)]));

    // 3. 복구 스냅샷 기록. 미저장 chunk는 홈 View subtree 하나뿐입니다.
    let payload = capture(&editor, &saved.documents, Some(gen1));
    let unsaved: BTreeSet<ChunkId> = payload.unsaved_chunks.iter().map(|c| c.chunk_id).collect();
    assert_eq!(unsaved.len(), 1, "바뀐 View subtree만 미저장");
    store
        .write_snapshot(&identity, window, &payload.encode().expect("인코딩"))
        .expect("스냅샷 기록");
    // 다른 두 창의 스냅샷: 미저장 변경 없음(revision 0), 저장 이전 base(오래됨).
    let idle_window = WindowSessionId::from_stable_id(id(9_001));
    let mut idle = payload.clone();
    idle.revision = 0;
    idle.unsaved_chunks.clear();
    store
        .write_snapshot(&identity, idle_window, &idle.encode().expect("인코딩"))
        .expect("기록");
    let old_window = WindowSessionId::from_stable_id(id(9_002));
    let mut old = payload.clone();
    old.base_manifest_hash = None;
    store
        .write_snapshot(&identity, old_window, &old.encode().expect("인코딩"))
        .expect("기록");
    // 손상 스냅샷도 제안 대상이 아닙니다.
    let broken_window = WindowSessionId::from_stable_id(id(9_003));
    store
        .write_snapshot(&identity, broken_window, b"not a payload")
        .expect("기록");

    // 4. 비정상 종료: 정상 종료 marker 없음. 다음 실행은 최신 스냅샷 하나만 제안합니다.
    assert!(!was_clean_shutdown(&store, &identity, window));
    let proposals = probe_recovery_snapshots(&store, &identity, &root).expect("probe");
    assert_eq!(proposals.len(), 1, "{proposals:?}");
    assert_eq!(proposals[0].window_session, window.to_string());
    assert_eq!(proposals[0].revision, editor.state.revision().0);
    assert_eq!(
        assess_snapshot(Some(gen1), &idle),
        SnapshotFreshness::Stale {
            reason: StaleReason::NoUnsavedChanges
        }
    );

    // 5. 복원: 스냅샷 decode → 저장 세대 + 미저장 chunk overlay → 문서 로드.
    let bytes = store
        .read_snapshot(&identity, window)
        .expect("읽기")
        .expect("스냅샷 있음");
    let decoded = RecoveryPayload::decode(&bytes).expect("decode");
    assert_eq!(decoded, payload);
    let project = SavedProjectLoader::new().open(&root).expect("열기");
    let manifest_id = manifest_chunk_id(id(1));
    let unsaved_map: BTreeMap<ChunkId, Vec<u8>> = decoded
        .unsaved_chunks
        .iter()
        .map(|c| (c.chunk_id, c.payload.clone()))
        .collect();
    let manifest_payload = match unsaved_map.get(&manifest_id) {
        Some(payload) => payload.clone(),
        None => find_document_manifest(&project.generation, id(1))
            .expect("저장된 manifest")
            .to_vec(),
    };
    let mut recorder = NoopRecorder;
    let mut source = Overlay {
        unsaved: unsaved_map,
        base: LazySource::new(&project.generation, &mut recorder),
    };
    let recovered = load_document(&manifest_payload, &mut source).expect("복원 로드");
    assert_same_document(editor.model(), &recovered.model, "복구 문서");
    // 탭은 복원 문서의 View를 가리키고, 선택은 문서에 있는 node로 복원됩니다.
    let views: BTreeSet<StableId> = recovered
        .model
        .definition()
        .pages
        .iter()
        .flat_map(|p| p.views.iter().map(|v| v.id))
        .collect();
    assert!(decoded
        .open_tabs
        .iter()
        .all(|tab| views.contains(&tab.view)));
    let mut view = WindowViewState::new(window);
    view.restore_selection(
        &Selection::new(decoded.selection.iter().copied(), None),
        &recovered.model,
    );
    assert_eq!(view.selection().ids(), editor.selection().ids());

    // 6. 복원 문서를 이어 쓰고 저장: 저장이 기록하는 chunk = 스냅샷의 미저장 chunk.
    let state = DocumentState::with_revision(recovered.model, DocumentRevision(decoded.revision));
    let resumed = Editor::with_state(state, SPACE + 1);
    let second = save_documents(&root, project_id, &[resumed.model()], &saved.documents);
    let written: BTreeSet<ChunkId> = second
        .plan
        .dirty_chunks
        .iter()
        .map(|w| w.chunk_id)
        .collect();
    assert_eq!(written, unsaved, "복구 저장 chunk = 미저장 chunk");
    let gen2 = read_current_pointer(&root).expect("current pointer");
    assert_ne!(gen2, gen1);

    // 7. 정상 저장이 진행됐으므로 스냅샷은 오래됨 — 더 제안하지 않습니다. 정상 종료가 정리합니다.
    assert!(matches!(
        assess_snapshot(Some(gen2), &decoded),
        SnapshotFreshness::Stale {
            reason: StaleReason::BaseGenerationMoved { .. }
        }
    ));
    assert!(probe_recovery_snapshots(&store, &identity, &root)
        .expect("probe")
        .is_empty());
    end_window_session_clean(&store, &identity, window).expect("정상 종료");
    assert!(was_clean_shutdown(&store, &identity, window));
    assert!(store
        .read_snapshot(&identity, window)
        .expect("읽기")
        .is_none());
    assert!(!store
        .list_sessions(&identity)
        .expect("세션 목록")
        .contains(&window.to_string()));

    // 8. 디스크 재열기는 복구한 문서와 같습니다.
    let reopened = SavedProjectLoader::new()
        .open(&root)
        .expect("재열기")
        .load_document(id(1))
        .expect("로드");
    assert_same_document(editor.model(), &reopened.model, "복구 저장 재열기");
}
