//! 종단 시나리오 공용 helper.
//!
//! - 임시 디렉터리와 결정론적 StableId
//! - command transaction과 창 history로 문서를 고치는 [`Editor`]
//! - 의미 동일성 검사(semantic snapshot·레코드·자식 순서·재직렬화 bytes)와 전체 validator 확인
//! - document_io 직렬화 결과를 storage 저장 transaction으로 디스크에 쓰는 helper
//! - 머신 Git 설정과 무관한 hermetic Git 실행기
//!
//! 모든 helper는 각 crate의 공개 API만 씁니다.
#![allow(dead_code)]

pub mod builders;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use stellar_commands::change::ChangeSet;
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin, WindowSessionId};
use stellar_commands::request::CommandRequest;
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::model::DocumentModel;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::validate::{validate_document, DocumentDiagnostics};
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::runner::{argv, GitOutput, GitRunner};
use stellar_git_porcelain::version::{probe_git_version, GitVersion};
use stellar_history::entry::HistoryError;
use stellar_history::window::WindowHistory;
use stellar_project::document_io::serialize::{serialize_document, SerializedDocument};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::{build_save_plan, SavePlan};

/// 문서 chunk payload 스키마 version과 같은 generation 스키마 version(첫 저장 인자).
pub const GENERATION_SCHEMA_VERSION: u32 = 1;

/// 임시 디렉터리(drop 시 삭제).
pub struct TempDir {
    /// 디렉터리 경로.
    pub path: PathBuf,
}

impl TempDir {
    /// `prefix`로 시작하는 새 빈 디렉터리를 만듭니다.
    pub fn new(prefix: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-it-{prefix}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 결정론적 StableId. `space`마다 timestamp 대역을 나눠 시나리오 사이 id가 겹치지 않습니다.
pub fn sid(space: u64, n: u64) -> StableId {
    StableId::new_v7(
        5_000_000_000_000 + space * 10_000_000 + n,
        0x0a11,
        0x0005_eed0_0000 ^ n,
    )
}

/// 한 byte OrderKey.
pub fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).expect("0이 아닌 byte")
}

/// 문서 semantic snapshot JSON(VI.10.4).
pub fn snapshot(model: &DocumentModel) -> String {
    model_snapshot_json(model).expect("semantic snapshot")
}

/// 전체 validator(VI.10.1)를 통과해야 합니다. 미해결 참조 진단을 돌려줍니다.
pub fn assert_valid(model: &DocumentModel, label: &str) -> DocumentDiagnostics {
    let records = model
        .to_records()
        .unwrap_or_else(|e| panic!("{label}: 레코드 생성 실패: {e}"));
    validate_document(&records).unwrap_or_else(|e| panic!("{label}: 문서 불변식 위반: {e}"))
}

/// 두 문서가 의미상 같아야 합니다: semantic snapshot, 영속 레코드, 모든 node의 자식 순서,
/// ID registry 왕복, 재직렬화 chunk bytes.
pub fn assert_same_document(expected: &DocumentModel, actual: &DocumentModel, label: &str) {
    assert_eq!(
        snapshot(actual),
        snapshot(expected),
        "{label}: semantic snapshot"
    );
    let records = expected.to_records().expect("레코드");
    assert_eq!(
        actual.to_records().expect("레코드"),
        records,
        "{label}: 레코드"
    );
    assert_eq!(
        actual.node_count(),
        expected.node_count(),
        "{label}: node 수"
    );
    for node in &records.nodes {
        let handle = actual
            .runtime_id(node.id)
            .unwrap_or_else(|| panic!("{label}: registry에 {} 없음", node.id));
        assert_eq!(
            actual.stable_id(handle),
            Some(node.id),
            "{label}: registry 왕복"
        );
        assert_eq!(
            actual.children_in_order(node.id),
            expected.children_in_order(node.id),
            "{label}: {} 자식 순서",
            node.id
        );
    }
    assert_eq!(
        serialize_document(actual).expect("직렬화"),
        serialize_document(expected).expect("직렬화"),
        "{label}: 재직렬화 bytes"
    );
}

/// 두 문서의 영속 레코드와 재직렬화 chunk bytes가 같아야 합니다(semantic snapshot보다 가벼운 비교로,
/// 많은 seed를 도는 루프에서 씁니다).
pub fn assert_same_records(expected: &DocumentModel, actual: &DocumentModel, label: &str) {
    assert_eq!(
        actual.to_records().expect("레코드"),
        expected.to_records().expect("레코드"),
        "{label}: 레코드"
    );
    assert_eq!(
        serialize_document(actual).expect("직렬화"),
        serialize_document(expected).expect("직렬화"),
        "{label}: 재직렬화 bytes"
    );
}

/// command transaction과 창 history로 문서를 고치는 편집기(한 창).
pub struct Editor {
    /// 창의 인메모리 문서 상태.
    pub state: DocumentState,
    /// 창 history와 선택.
    pub window: WindowHistory,
    space: u64,
    next_tx: u64,
}

impl Editor {
    /// 모델을 revision 0 상태로 엽니다. `space`는 transaction·창 id 대역입니다.
    pub fn new(model: DocumentModel, space: u64) -> Editor {
        Editor::with_state(DocumentState::new(model), space)
    }

    /// 이미 만든 상태로 엽니다(재열기 후 이어지는 revision 등).
    pub fn with_state(state: DocumentState, space: u64) -> Editor {
        let window = WindowHistory::new(WindowSessionId::from_stable_id(sid(space, 9_999_999)));
        Editor {
            state,
            window,
            space,
            next_tx: 0,
        }
    }

    /// 현재 모델.
    pub fn model(&self) -> &DocumentModel {
        self.state.model()
    }

    /// 현재 revision을 base로 하는 transaction(요청 하나당 command 하나).
    pub fn transaction(&mut self, commands: Vec<Command>) -> CommandTransaction {
        self.next_tx += 1;
        let id = CommandId::from_stable_id(sid(self.space, 8_000_000 + self.next_tx));
        commands.into_iter().fold(
            CommandTransaction::new(id, CommandOrigin::User),
            |tx, command| tx.with_request(CommandRequest::at(&self.state, command)),
        )
    }

    /// transaction을 실행·기록합니다. 실패하면 문서·history는 실행 전 상태입니다.
    pub fn try_apply(&mut self, commands: Vec<Command>) -> Result<ChangeSet, HistoryError> {
        let transaction = self.transaction(commands);
        self.window
            .record_document(&mut self.state, &transaction, None)
    }

    /// transaction을 실행·기록하고 성공과 validator 통과를 강제합니다.
    pub fn apply(&mut self, commands: Vec<Command>, label: &str) -> ChangeSet {
        let revision = self.state.revision();
        let changes = self
            .try_apply(commands)
            .unwrap_or_else(|e| panic!("{label}: transaction 실패: {e}"));
        assert_eq!(
            self.state.revision().0,
            revision.0 + 1,
            "{label}: revision은 transaction당 1 증가"
        );
        assert_valid(self.model(), label);
        changes
    }

    /// 선택을 바꾸고 history에 기록합니다. 기록했으면 `true`.
    pub fn select(&mut self, ids: &[StableId]) -> bool {
        let event = SetSelection::new(Selection::new(ids.iter().copied(), ids.first().copied()));
        self.window
            .record_selection(&self.state, &event)
            .expect("선택 기록")
            .is_some()
    }

    /// 현재 선택.
    pub fn selection(&self) -> Selection {
        self.window.view().selection().clone()
    }

    /// 마지막 항목을 되돌립니다. 되돌릴 항목이 있었으면 `true`.
    pub fn undo(&mut self) -> bool {
        self.window
            .undo(&mut self.state)
            .expect("undo 실행")
            .is_some()
    }

    /// 되돌린 항목을 다시 적용합니다. 다시 적용할 항목이 있었으면 `true`.
    pub fn redo(&mut self) -> bool {
        self.window
            .redo(&mut self.state)
            .expect("redo 실행")
            .is_some()
    }

    /// 현재 semantic snapshot.
    pub fn snapshot(&self) -> String {
        snapshot(self.model())
    }
}

/// 저장 결과.
pub struct SaveOutcome {
    /// 실행한 저장 계획.
    pub plan: SavePlan,
    /// 저장한 문서 직렬화(다음 저장의 clean 판정 기준).
    pub documents: Vec<SerializedDocument>,
}

/// logical chunk 상태 목록을 current generation 위에 저장합니다(V.5 저장 transaction).
pub fn save_states(
    root: &Path,
    project_id: StableId,
    states: &[LogicalChunkState],
    roots: Vec<StableId>,
) -> SavePlan {
    let base = load_current_generation(root, project_id).ok();
    let plan = build_save_plan(
        base.as_ref(),
        project_id,
        GENERATION_SCHEMA_VERSION,
        states,
        roots,
    )
    .expect("저장 계획");
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).expect("저장 실행");
    plan
}

/// 문서들을 직렬화해 저장합니다. `previous`(마지막 저장의 직렬화)와 payload가 같은 chunk는 clean으로
/// 넘겨 base pointer를 재사용하게 합니다(편집기가 아는 dirty 정보, V.3.3).
pub fn save_documents(
    root: &Path,
    project_id: StableId,
    documents: &[&DocumentModel],
    previous: &[SerializedDocument],
) -> SaveOutcome {
    let saved: BTreeMap<_, _> = previous
        .iter()
        .flat_map(|d| d.chunks())
        .map(|c| (c.chunk_id, c.payload.clone()))
        .collect();
    let serialized: Vec<SerializedDocument> = documents
        .iter()
        .map(|m| serialize_document(m).expect("직렬화"))
        .collect();
    let states: Vec<LogicalChunkState> = serialized
        .iter()
        .flat_map(|d| d.chunks())
        .map(|chunk| {
            if saved.get(&chunk.chunk_id) == Some(&chunk.payload) {
                LogicalChunkState::Clean {
                    chunk_id: chunk.chunk_id,
                }
            } else {
                LogicalChunkState::Dirty {
                    chunk_id: chunk.chunk_id,
                    kind: chunk.kind,
                    schema_version: chunk.schema_version,
                    payload: chunk.payload.clone(),
                }
            }
        })
        .collect();
    let roots = serialized.iter().map(|d| d.document_id).collect();
    let plan = save_states(root, project_id, &states, roots);
    SaveOutcome {
        plan,
        documents: serialized,
    }
}

/// 머신 global/system Git 설정과 무관한 실행기. identity는 `config_dir`에 쓴 global 설정 파일로 줍니다.
/// Git이 없거나 너무 오래되면 `None`(호출 테스트는 건너뜀).
pub fn hermetic_git(config_dir: &Path) -> Option<(GitRunner, GitVersion)> {
    let git = discover_git(&GitDiscoveryConfig::default())?;
    let version = probe_git_version(&git).ok()?;
    if (version.major, version.minor) < (2, 28) {
        return None;
    }
    let config = config_dir.join("gitconfig");
    std::fs::write(
        &config,
        "[user]\n\tname = Stellar Test\n\temail = test@example.com\n[core]\n\tautocrlf = false\n",
    )
    .expect("gitconfig 기록");
    let runner = GitRunner::new(git, vec![OsString::from("PATH")], Duration::from_secs(60))
        .with_env_overrides(vec![
            (OsString::from("GIT_CONFIG_GLOBAL"), config.into_os_string()),
            (
                OsString::from("GIT_CONFIG_SYSTEM"),
                OsString::from("/dev/null"),
            ),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
        ]);
    Some((runner, version))
}

/// Git을 실행하고 성공을 강제합니다.
pub fn git_ok<I, S>(runner: &GitRunner, dir: &Path, args: I) -> GitOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = argv(args);
    let out = runner.run(&args, dir).expect("git 실행");
    assert!(
        out.success(),
        "git 실패 {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
