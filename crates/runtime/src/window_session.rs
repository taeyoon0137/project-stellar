//! 창 하나의 프로젝트 편집 session (XII.2.1).
//!
//! 같은 프로젝트를 여러 창에서 열 수 있고 각 창은 독립된 인메모리 문서 상태와 undo/redo stack을
//! 가집니다(기술 아키텍처 §22.2·§23.2). [`ProjectWindowSession`]은 창 하나가 소유하는 그 묶음입니다.
//!
//! | 필드 | 내용 |
//! |---|---|
//! | [`ProjectIdentity`] | 연 프로젝트(project StableId + root 경로, IV.2.3) |
//! | [`WindowSessionId`] | 창 편집 session id(V.9.1, VII.1.1) |
//! | 문서 state | 이 창만의 [`DocumentState`](모델 + revision). 다른 session과 공유하지 않습니다 |
//! | view state | 이 창의 [`WindowViewState`](선택, VII.3.1). history가 함께 소유합니다 |
//! | history | 이 창의 [`WindowHistory`](VII.5.1) |
//! | loaded generation | 창을 열 때 읽은 current generation([`LoadedGeneration`], 첫 저장 전이면 없음) |
//!
//! # 로드
//!
//! [`ProjectWindowSession::load`]는 디스크의 current generation을 검증해 읽고(V.4.4), 그 generation의 첫
//! root 문서를 이 창의 작업 문서로 로드합니다(VI.10.3). 다른 root 문서는 읽지 않으며 저장할 때 base
//! generation의 chunk를 그대로 이어 씁니다. 창 하나가 문서 하나를 편집하고, 문서 전환은 이 type의 범위
//! 밖입니다. current pointer가 없으면(저장한 적 없는 프로젝트) 작업 문서와 loaded generation이 모두
//! 없습니다.
//!
//! 로드할 때마다 디스크에서 새 모델을 만들므로, 같은 프로젝트를 연 두 session도 모델·revision·history·
//! 선택을 공유하지 않습니다. session은 document thread(유일한 작성자, XII.1.1)의 session 표에서만
//! 살아 있고 `Send`가 아닌 context 밖으로 나가지 않습니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_commands::change::ChangeSet;
use stellar_commands::selection::{Selection, SetSelection, WindowViewState};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::revision::DocumentRevision;
use stellar_history::entry::{HistoryError, HistorySequence};
use stellar_history::window::WindowHistory;
use stellar_project::document_io::load::{load_document_from_generation, LoadDocumentError};
use stellar_project::identity::ProjectIdentity;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::current::CurrentPointerError;
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration, LazyLoadError};
use stellar_storage::generation::loader::{load_current_generation, LoadError, LoadedGeneration};

/// session 로드 실패.
#[derive(Debug)]
pub enum WindowSessionLoadError {
    /// current generation을 열거나 검증하지 못했습니다(누락 manifest·chunk, 다른 project id 등).
    Generation(LazyLoadError),
    /// 두 번 읽은 current generation이 서로 다릅니다(읽는 사이 다른 저장이 끝남). 다시 열면 됩니다.
    ConcurrentSave,
    /// 작업 문서를 chunk에서 로드하지 못했습니다.
    Document(LoadDocumentError),
}

impl fmt::Display for WindowSessionLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionLoadError::Generation(error) => {
                write!(f, "저장된 generation을 열지 못했습니다: {error}")
            }
            WindowSessionLoadError::ConcurrentSave => {
                f.write_str("여는 동안 다른 저장이 끝나 generation이 바뀌었습니다")
            }
            WindowSessionLoadError::Document(error) => {
                write!(f, "문서를 로드하지 못했습니다: {error}")
            }
        }
    }
}

impl std::error::Error for WindowSessionLoadError {}

/// 작업 문서 하나: 이 창의 문서 state와 로드할 때 읽은 그 문서의 chunk.
#[derive(Debug)]
pub(crate) struct WindowDocument {
    pub(crate) state: DocumentState,
    /// 로드할 때 읽은 이 문서의 chunk(DocumentManifest·ViewTree·subtree). 저장할 때 다른 문서의 chunk와
    /// 구분해, 이 집합 밖의 base chunk만 그대로 이어 씁니다.
    pub(crate) owned_chunks: BTreeSet<ChunkId>,
}

/// 창 하나의 프로젝트 편집 session(모듈 문서).
#[derive(Debug)]
pub struct ProjectWindowSession {
    identity: ProjectIdentity,
    window: WindowSessionId,
    pub(crate) document: Option<WindowDocument>,
    pub(crate) history: WindowHistory,
    pub(crate) loaded: Option<LoadedGeneration>,
}

/// 읽은 chunk id를 모읍니다.
#[derive(Default)]
struct ChunkCollector(BTreeSet<ChunkId>);

impl ChunkReadRecorder for ChunkCollector {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.0.insert(pointer.chunk_id);
    }
}

fn is_missing_current(error: &LoadError) -> bool {
    matches!(
        error,
        LoadError::CurrentPointer(CurrentPointerError::Missing { .. })
    )
}

impl ProjectWindowSession {
    /// 프로젝트 `identity`의 current generation과 첫 root 문서를 로드해 창 `window`의 session을 만듭니다.
    ///
    /// 저장한 적 없는 프로젝트(current pointer 없음)는 작업 문서·loaded generation 없이 엽니다.
    pub fn load(
        identity: ProjectIdentity,
        window: WindowSessionId,
    ) -> Result<ProjectWindowSession, WindowSessionLoadError> {
        let root = identity.root_path().to_path_buf();
        let project_id = identity.project_id();
        let loaded = match load_current_generation(&root, project_id) {
            Ok(loaded) => loaded,
            Err(error) if is_missing_current(&error) => {
                return Ok(ProjectWindowSession::empty(identity, window));
            }
            Err(error) => {
                return Err(WindowSessionLoadError::Generation(LazyLoadError::Load(
                    error,
                )))
            }
        };
        let mut collector = ChunkCollector::default();
        let lazy = LazyGeneration::open(&root, project_id, &mut collector)
            .map_err(WindowSessionLoadError::Generation)?;
        if lazy.manifest() != &loaded.manifest {
            return Err(WindowSessionLoadError::ConcurrentSave);
        }
        let document = match loaded.manifest.root_document_ids.first().copied() {
            None => None,
            Some(document_id) => {
                let mut owned = ChunkCollector::default();
                let document = load_document_from_generation(&lazy, document_id, &mut owned)
                    .map_err(WindowSessionLoadError::Document)?;
                // DocumentManifest chunk는 열기 때 읽었으므로 그 문서의 것만 골라 더합니다.
                for (chunk, payload) in lazy.document_manifests() {
                    let manifest =
                        stellar_project::document_io::manifest::DocumentManifestChunk::decode(
                            payload,
                        )
                        .map_err(|error| {
                            WindowSessionLoadError::Document(LoadDocumentError::Manifest(error))
                        })?;
                    if manifest.definition.id == document_id {
                        owned.0.insert(*chunk);
                    }
                }
                Some(WindowDocument {
                    state: DocumentState::new(document.model),
                    owned_chunks: owned.0,
                })
            }
        };
        Ok(ProjectWindowSession {
            identity,
            window,
            document,
            history: WindowHistory::new(window),
            loaded: Some(loaded),
        })
    }

    /// 저장된 generation이 없는 프로젝트의 session.
    fn empty(identity: ProjectIdentity, window: WindowSessionId) -> ProjectWindowSession {
        ProjectWindowSession {
            identity,
            window,
            document: None,
            history: WindowHistory::new(window),
            loaded: None,
        }
    }

    /// 연 프로젝트.
    pub fn identity(&self) -> &ProjectIdentity {
        &self.identity
    }

    /// 창 편집 session id.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 이 창의 작업 문서 state. 저장된 문서가 없으면 `None`.
    pub fn document(&self) -> Option<&DocumentState> {
        self.document.as_ref().map(|document| &document.state)
    }

    /// 작업 문서 StableId.
    pub fn document_id(&self) -> Option<StableId> {
        self.document().map(|state| state.model().document_id())
    }

    /// 작업 문서 revision(문서가 없으면 0).
    pub fn revision(&self) -> DocumentRevision {
        self.document()
            .map_or(DocumentRevision(0), DocumentState::revision)
    }

    /// 작업 문서가 base generation에서 차지한 chunk(DocumentManifest·ViewTree·subtree). 문서가 없으면
    /// `None`. 저장은 이 집합 밖의 base chunk만 그대로 이어 씁니다.
    pub fn document_chunks(&self) -> Option<&BTreeSet<ChunkId>> {
        self.document
            .as_ref()
            .map(|document| &document.owned_chunks)
    }

    /// 이 창의 history.
    pub fn history(&self) -> &WindowHistory {
        &self.history
    }

    /// 이 창의 view state(선택).
    pub fn view(&self) -> &WindowViewState {
        self.history.view()
    }

    /// 창을 열 때(또는 마지막 저장 뒤) 기준이 된 generation. 첫 저장 전이면 `None`.
    pub fn loaded_generation(&self) -> Option<&LoadedGeneration> {
        self.loaded.as_ref()
    }

    /// loaded generation 번호(없으면 0).
    pub fn generation_number(&self) -> u64 {
        self.loaded
            .as_ref()
            .map_or(0, |loaded| loaded.manifest.generation)
    }

    /// 작업 문서에 transaction을 실행하고 이 창 history에 기록합니다. 작업 문서가 없으면
    /// [`WindowSessionEditError::NoDocument`].
    pub fn apply(
        &mut self,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        self.history
            .record_document(&mut document.state, transaction, None)
            .map_err(WindowSessionEditError::History)
    }

    /// 선택을 바꾸고 이 창 history에 기록합니다(같은 선택이면 기록하지 않음).
    pub fn select(
        &mut self,
        selection: Selection,
    ) -> Result<Option<HistorySequence>, WindowSessionEditError> {
        let document = self
            .document
            .as_ref()
            .ok_or(WindowSessionEditError::NoDocument)?;
        self.history
            .record_selection(&document.state, &SetSelection::new(selection))
            .map_err(WindowSessionEditError::History)
    }
}

/// session 편집 실패. 문서·선택은 바뀌지 않았습니다.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowSessionEditError {
    /// 작업 문서가 없습니다(저장된 문서가 없는 프로젝트).
    NoDocument,
    /// history 기록 또는 transaction 실행 실패.
    History(HistoryError),
}

impl fmt::Display for WindowSessionEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionEditError::NoDocument => f.write_str("편집할 문서가 없습니다"),
            WindowSessionEditError::History(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WindowSessionEditError {}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! 저장된 문서가 있는 임시 프로젝트 fixture(테스트 전용).

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use stellar_commands::command::Command;
    use stellar_commands::document::definitions::{
        DocumentDefinition, PageDefinition, ViewDefinition,
    };
    use stellar_commands::document::frame::FramePayload;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::records::{NodePayload, NodeRecord};
    use stellar_commands::id::{CommandId, CommandOrigin};
    use stellar_commands::state::DocumentState;
    use stellar_commands::transaction::CommandTransaction;
    use stellar_core_types::ids::{StableId, WindowSessionId};
    use stellar_core_types::order_key::OrderKey;
    use stellar_project::document_io::serialize::serialize_document;
    use stellar_project::identity::{parse_project_id, ProjectIdentity};
    use stellar_project::scaffold::write_default_scaffold;
    use stellar_storage::save::cancel::{run_save, CancelToken};
    use stellar_storage::save::commit::SavedGenerationState;
    use stellar_storage::save::plan::build_save_plan;

    pub const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    /// drop 시 지우는 임시 디렉터리.
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-window-session-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 결정론적 id.
    pub fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    pub fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(StableId::new_v7(9_000 + n, 0x0123, n))
    }

    fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).expect("key"),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        }
    }

    /// 문서(1): 페이지(2) 아래 View 3(root 10)·View 4(root 20). 10 아래 11·12.
    pub fn sample_document() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
        let mut page = PageDefinition::new(sid(2), "페이지".to_string());
        page.add_view(ViewDefinition::new(sid(3), "첫 화면".to_string(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(
            sid(4),
            "둘째 화면".to_string(),
            sid(20),
        ))
        .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(10, None, 0x10)).unwrap();
        model.insert_node(frame(11, Some(10), 0x10)).unwrap();
        model.insert_node(frame(12, Some(10), 0x20)).unwrap();
        model.insert_node(frame(20, None, 0x20)).unwrap();
        model
    }

    /// `stellar.yaml` + `.git` + [`sample_document`]를 저장한 generation 1을 가진 프로젝트.
    pub fn saved_project(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        write_default_scaffold(dir.path(), "창 session", PROJECT_ID).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let serialized = serialize_document(&sample_document()).unwrap();
        let plan = build_save_plan(
            None,
            project_id(),
            1,
            &serialized.to_dirty_states(),
            vec![serialized.document_id],
        )
        .unwrap();
        let mut memory = SavedGenerationState::new(None);
        run_save(dir.path(), &plan, &mut memory, &CancelToken::new()).unwrap();
        dir
    }

    /// 저장한 generation 없이 scaffold만 있는 프로젝트.
    pub fn unsaved_project(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        write_default_scaffold(dir.path(), "빈 프로젝트", PROJECT_ID).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    pub fn project_id() -> StableId {
        parse_project_id(PROJECT_ID).unwrap()
    }

    pub fn identity(root: &Path) -> ProjectIdentity {
        ProjectIdentity::new(project_id(), root).unwrap()
    }

    /// 현재 revision 기준 이름 변경 transaction.
    pub fn rename(doc: &DocumentState, command: u64, node: u64, name: &str) -> CommandTransaction {
        CommandTransaction::new(
            CommandId::from_stable_id(StableId::new_v7(command, 0x0c0c, command)),
            CommandOrigin::User,
        )
        .with_command(
            doc.revision(),
            Command::rename_node(sid(node), name.to_string()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(session: &ProjectWindowSession) -> String {
        model_snapshot_json(session.document().unwrap().model()).unwrap()
    }

    #[test]
    fn loads_identity_window_document_history_and_generation() {
        let project = saved_project("load");
        let session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        assert_eq!(session.identity().project_id(), project_id());
        assert_eq!(session.window(), window(1));
        assert_eq!(session.document_id(), Some(sid(1)));
        assert_eq!(session.revision(), DocumentRevision(0));
        assert_eq!(session.generation_number(), 1);
        assert!(session.loaded_generation().is_some());
        assert!(session.history().history().is_empty());
        assert_eq!(session.view().window(), window(1));
        assert!(session.view().selection().is_empty());
        assert_eq!(
            snapshot(&session),
            model_snapshot_json(&sample_document()).unwrap()
        );
        // 작업 문서 chunk: manifest + ViewTree 2 + subtree 2(root 10·20).
        let owned = session.document_chunks().unwrap();
        assert_eq!(owned.len(), 5);
        let pointers: BTreeSet<ChunkId> = session
            .loaded_generation()
            .unwrap()
            .manifest
            .chunk_pointers
            .iter()
            .map(|p| p.chunk_id)
            .collect();
        assert_eq!(
            owned, &pointers,
            "문서가 하나뿐이면 generation chunk 전부가 그 문서의 것"
        );
    }

    #[test]
    fn two_sessions_do_not_share_state() {
        // 검증 기준(XII.2.1): 두 session이 서로 state를 공유하지 않습니다.
        let project = saved_project("isolation");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        let original = snapshot(&a);
        assert_eq!(snapshot(&b), original);
        assert_ne!(a.window(), b.window());

        // A 편집: B의 문서·revision·history는 그대로.
        let tx = rename(a.document().unwrap(), 100, 11, "A가 바꾼 이름");
        a.apply(&tx).unwrap();
        assert_eq!(a.revision(), DocumentRevision(1));
        assert_ne!(snapshot(&a), original);
        assert_eq!(snapshot(&b), original, "B 문서는 A 편집을 보지 않습니다");
        assert_eq!(b.revision(), DocumentRevision(0));
        assert_eq!(a.history().history().len(), 1);
        assert!(b.history().history().is_empty());

        // B 선택: A 선택·history는 그대로.
        b.select(Selection::new([sid(12)], None)).unwrap();
        assert_eq!(b.view().selection().ids(), &[sid(12)]);
        assert!(a.view().selection().is_empty());
        assert_eq!(a.history().history().len(), 1);
        assert_eq!(b.history().history().len(), 1);

        // B 편집은 A가 바꾼 node를 보지 않은 B 문서 위에서 실행됩니다.
        let tx = rename(b.document().unwrap(), 200, 11, "B가 바꾼 이름");
        b.apply(&tx).unwrap();
        let name = |session: &ProjectWindowSession| {
            session
                .document()
                .unwrap()
                .model()
                .record(sid(11))
                .unwrap()
                .display_name
        };
        assert_eq!(name(&a), "A가 바꾼 이름");
        assert_eq!(name(&b), "B가 바꾼 이름");

        // loaded generation은 각 session이 따로 들고 있습니다(같은 디스크 세대).
        assert_eq!(a.generation_number(), b.generation_number());
        assert!(!std::ptr::eq(
            a.loaded_generation().unwrap(),
            b.loaded_generation().unwrap()
        ));
        // A를 버려도 B는 그대로 동작합니다.
        drop(a);
        assert_eq!(name(&b), "B가 바꾼 이름");
        assert_eq!(b.revision(), DocumentRevision(1));
    }

    #[test]
    fn unsaved_project_opens_without_document_or_generation() {
        let project = unsaved_project("unsaved");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(3)).unwrap();
        assert!(session.document().is_none());
        assert!(session.loaded_generation().is_none());
        assert_eq!(session.generation_number(), 0);
        assert_eq!(
            session.select(Selection::empty()),
            Err(WindowSessionEditError::NoDocument)
        );
    }

    #[test]
    fn corrupt_generation_is_a_typed_error() {
        let project = saved_project("corrupt");
        // current pointer가 가리키는 manifest를 지우면 열기 검증이 실패합니다.
        let manifest_dir = project.path().join("design").join("generations");
        assert!(manifest_dir.is_dir(), "generation manifest 디렉터리");
        std::fs::remove_dir_all(&manifest_dir).unwrap();
        let error = ProjectWindowSession::load(identity(project.path()), window(4)).unwrap_err();
        assert!(
            matches!(error, WindowSessionLoadError::Generation(_)),
            "{error}"
        );
        assert!(error.to_string().contains("generation"));
    }

    /// `root` 아래 일반 파일(상대 경로 → bytes). 숨김 파일(`.git` 등)은 넣지 않습니다.
    fn tree(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            base: &std::path::Path,
            dir: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let relative = path.strip_prefix(base).unwrap().to_string_lossy();
                    out.insert(relative.replace('\\', "/"), std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn committed_fixture_project_matches_the_sample_project() {
        // 셸 검사(`--window-session-check`)와 FFI 테스트가 복사해 쓰는 저장 프로젝트 fixture입니다. 저장 형식이
        // 바뀌면 `STELLAR_UPDATE_FIXTURES=1 cargo test -p stellar-runtime committed_fixture`로 다시 만듭니다.
        let generated = saved_project("fixture");
        let expected = tree(generated.path());
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("window-session-project");
        if std::env::var("STELLAR_UPDATE_FIXTURES").as_deref() == Ok("1") {
            for (relative, bytes) in &expected {
                let path = fixture.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        assert_eq!(
            tree(&fixture),
            expected,
            "fixture가 sample 프로젝트 저장 결과와 다릅니다"
        );
        // fixture 그대로 열면(.git만 더함) 같은 작업 문서가 로드됩니다.
        let copy = TempDir::new("fixture-copy");
        for (relative, bytes) in &expected {
            let path = copy.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let session = ProjectWindowSession::load(identity(copy.path()), window(5)).unwrap();
        assert_eq!(
            snapshot(&session),
            model_snapshot_json(&sample_document()).unwrap()
        );
    }
}
