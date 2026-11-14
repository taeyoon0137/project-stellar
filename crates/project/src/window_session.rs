//! 창 문서 세션과 clean 창의 외부 generation 자동 reload (XIII.3.1).
//!
//! 기술 아키텍처 22.2·22.5: 창에 로컬 미저장 변경이 없으면 외부(다른 창·Git·외부 도구·CLI)가 만든 새 정상
//! generation으로 자동 갱신합니다. 갱신하면 그 창의 history(undo/redo)와 선택을 폐기합니다. history는 세션
//! 메모리 전용이고(§23.1) 이전 generation 기준 역 transaction은 새 문서에 맞지 않기 때문입니다(창 종료 시
//! 폐기 VII.5.4와 같은 규칙).
//!
//! [`WindowDocumentSession`]은 창 하나의 문서 상태, dirty 판정([`DirtyTrackedHistory`], XIII.1.2), 디스크 동기
//! 상태([`WindowExternalState`], XIII.3.2), 마지막 저장 상태([`SavedGenerationState`], V.5.4), 자기 저장 기록
//! ([`OwnSaveLedger`], XIII.2.4)을 함께 소유해 이 전환을 한 곳에서 합니다. 문서·선택을 바꾸는 경로는 이
//! type의 method뿐이라 dirty 판정과 디스크 동기 상태가 어긋나지 않습니다.
//!
//! [`WindowDocumentSession::on_external_generation`](XIII.2.5 `ExternalChange::Generation` 입력) 처리:
//! 1. preview(drag·scrub) 진행 중 → 반영을 미룹니다([`ExternalReloadOutcome::Deferred`]). 진행 중 조작을 자동
//!    reload로 빼앗지 않습니다. 마지막으로 미룬 hash는 [`WindowDocumentSession::take_deferred_external`]로
//!    꺼내 preview가 끝난 뒤 다시 넘깁니다.
//! 2. 로컬 미저장 변경 있음 → 세 snapshot 대기 상태(XIII.3.2, [`ExternalReloadOutcome::Pending`]). 문서
//!    메모리는 그대로입니다.
//! 3. 로컬 변경 없음 → 프로젝트를 다시 열고([`SavedProjectLoader`], V.4.4) 이 창 문서를 읽은 뒤(chunk 내용
//!    검증 V.2.6) **모두 성공하면** 문서 상태를 바꾸고 history·선택을 폐기합니다
//!    ([`ExternalReloadOutcome::Reloaded`]). 열기·읽기가 실패하면 아무 상태도 바꾸지 않습니다(유효하지 않은
//!    외부 데이터를 부분 적용하지 않음). 결정한 generation과 실제로 연 current generation이 다르면(그 사이
//!    pointer가 또 바뀜) 바꾸지 않고 [`ExternalReloadOutcome::Superseded`]입니다. 다음 coalescer 변화가 최신
//!    generation을 가져옵니다.
//! 4. 갱신하면 새 generation을 마지막 저장 상태와 자기 저장 기록의 최신 값으로 삼습니다
//!    ([`OwnSaveLedger::record`]). pointer 판정(XIII.2.4)은 `latest()`가 창의 현재 saved generation이라고
//!    가정하므로, 이 기록이 없으면 pointer가 이전 자기 저장 generation으로 되돌아갈 때(Git checkout 등)
//!    자기 저장으로 오판해 reload를 놓칩니다.
//!
//! 메모리 snapshot·저장 계획은 이 창 문서 chunk만 새로 만들고, 같은 generation의 **다른 문서 chunk**는 base
//! pointer를 그대로 씁니다(`Clean`). 이 창 문서가 차지한 chunk는 마지막으로 읽거나 저장한 generation 기준으로
//! 기억합니다(읽기 recorder).
//!
//! [`WindowDocumentSession::save_blocking`]은 저장을 호출한 thread에서 끝까지 실행하는 동기 경로입니다.
//! worker 분리와 stale 결과 폐기는 XIII.1.4 범위입니다.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_commands::change::ChangeSet;
use stellar_commands::id::WindowSessionId;
use stellar_commands::selection::SetSelection;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;
use stellar_history::entry::{GroupingKey, HistoryError, HistorySequence};
use stellar_history::history::UndoOutcome;
use stellar_history::window::WindowHistory;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::generation::lazy::ChunkReadRecorder;
use stellar_storage::generation::loader::{LoadError, LoadedGeneration};
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::CancelToken;
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::{build_save_plan, SavePlanError};

use crate::autosave::dirty::DirtyTrackedHistory;
use crate::document_io::load::{load_document_from_generation, LoadDocumentError};
use crate::document_io::serialize::{
    manifest_chunk_id, serialize_document, SerializeError, SerializedDocument,
};
use crate::external_change::{
    DiskSnapshot, ExternalChangeDecision, ExternalChangeError, GuardedSaveError, MemorySnapshot,
    SnapshotError, SnapshotHashes, WindowDiskState, WindowExternalState,
};
use crate::saved_project::{SavedProject, SavedProjectError, SavedProjectLoader};
use crate::watcher::own_save::OwnSaveLedger;

/// 외부 generation 처리 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalReloadOutcome {
    /// 창의 base와 같은 generation(변화 없음).
    Unchanged,
    /// 로컬 변경이 없어 새 generation으로 갱신하고 history·선택을 폐기함.
    Reloaded {
        /// 새 generation number.
        generation: u64,
        /// 새 manifest hash.
        manifest_hash: ContentHash,
        /// 폐기한 history 항목 수(undo + redo).
        discarded_history_entries: usize,
    },
    /// 로컬 미저장 변경이 있어 세 snapshot을 보존한 대기 상태(XIII.3.2).
    Pending(SnapshotHashes),
    /// preview 진행 중이라 반영을 미룸([`WindowDocumentSession::take_deferred_external`]).
    Deferred,
    /// 결정한 generation보다 새 generation이 이미 current라 바꾸지 않음.
    Superseded,
}

/// [`WindowDocumentSession::save_blocking`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSave {
    /// 저장할 완료 편집이 없음.
    Clean,
    /// preview 진행 중이라 저장하지 않음.
    PreviewInProgress,
    /// 새 generation으로 저장함(manifest hash는 [`WindowDocumentSession::saved`]).
    Saved {
        /// 저장한 문서 revision.
        revision: DocumentRevision,
        /// 새 generation number.
        generation: u64,
    },
}

/// 창 세션 실패. 실패한 동작은 세션 상태를 바꾸지 않습니다.
#[derive(Debug)]
pub enum WindowSessionError {
    /// 프로젝트 열기(매니페스트·저장소 검사·generation) 실패.
    Open(SavedProjectError),
    /// current pointer를 읽지 못함.
    Pointer(CurrentPointerError),
    /// generation 검증 실패.
    Disk(LoadError),
    /// 연 generation이 검사한 current generation과 다름(열기 중 저장소 변경).
    Changed,
    /// 이 창 문서를 읽지 못함(새 generation에 없음·chunk 손상 등).
    Document(LoadDocumentError),
    /// 다시 연 프로젝트의 project id가 창과 다름.
    ProjectChanged {
        /// 창의 project id.
        expected: StableId,
        /// 다시 연 project id.
        found: StableId,
    },
    /// 외부 변경 처리 실패(외부 generation 검증·메모리 snapshot).
    External(ExternalChangeError),
    /// 문서 직렬화 실패.
    Serialize(SerializeError),
    /// 저장 계획 실패.
    Plan(SavePlanError),
    /// 저장 거부·실패(외부 변경 대기 포함).
    Save(GuardedSaveError),
}

impl fmt::Display for WindowSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionError::Open(e) => write!(f, "프로젝트를 열 수 없습니다: {e}"),
            WindowSessionError::Pointer(e) => write!(f, "current pointer를 읽을 수 없습니다: {e}"),
            WindowSessionError::Disk(e) => write!(f, "generation을 검증할 수 없습니다: {e}"),
            WindowSessionError::Changed => {
                f.write_str("열기 중 저장소가 바뀌어 검사한 generation과 다릅니다")
            }
            WindowSessionError::Document(e) => write!(f, "창 문서를 읽을 수 없습니다: {e}"),
            WindowSessionError::ProjectChanged { expected, found } => {
                write!(
                    f,
                    "project id가 바뀌었습니다: 창 {expected}, 디스크 {found}"
                )
            }
            WindowSessionError::External(e) => write!(f, "{e}"),
            WindowSessionError::Serialize(e) => write!(f, "문서를 직렬화할 수 없습니다: {e}"),
            WindowSessionError::Plan(e) => write!(f, "저장 계획을 만들 수 없습니다: {e}"),
            WindowSessionError::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WindowSessionError {}

/// 창 하나의 문서 편집 세션.
#[derive(Debug)]
pub struct WindowDocumentSession {
    root: PathBuf,
    project_id: StableId,
    document_id: StableId,
    window: WindowSessionId,
    document: DocumentState,
    history: DirtyTrackedHistory,
    disk: WindowExternalState,
    saved: SavedGenerationState,
    ledger: OwnSaveLedger,
    /// base generation에서 이 창 문서가 차지한 chunk.
    own_chunks: BTreeSet<ChunkId>,
    /// preview 중이라 미룬 마지막 외부 generation.
    deferred: Option<ContentHash>,
}

impl WindowDocumentSession {
    /// `root` 프로젝트의 current generation에서 `document`를 읽어 창 `window`의 clean 세션을 엽니다.
    pub fn open(
        root: &Path,
        document: StableId,
        window: WindowSessionId,
    ) -> Result<WindowDocumentSession, WindowSessionError> {
        let project = SavedProjectLoader::new()
            .open(root)
            .map_err(WindowSessionError::Open)?;
        let manifest_hash = read_current_pointer(root).map_err(WindowSessionError::Pointer)?;
        let base = DiskSnapshot::load(root, &manifest_hash, project.project_id)
            .map_err(WindowSessionError::Disk)?;
        if project.generation.manifest() != &base.manifest {
            return Err(WindowSessionError::Changed);
        }
        let (model, own_chunks) = load_own_document(&project, document)?;
        let state = DocumentState::new(model);
        let history = DirtyTrackedHistory::new(WindowHistory::new(window), state.revision());
        let mut ledger = OwnSaveLedger::new();
        ledger.record(base.generation(), base.manifest_hash);
        Ok(WindowDocumentSession {
            root: root.to_path_buf(),
            project_id: project.project_id,
            document_id: document,
            window,
            document: state,
            history,
            saved: SavedGenerationState::new(Some((base.generation(), base.manifest_hash))),
            disk: WindowExternalState::open(root, base),
            ledger,
            own_chunks,
            deferred: None,
        })
    }

    /// 창 문서 상태(읽기 전용).
    pub fn document(&self) -> &DocumentState {
        &self.document
    }

    /// 창 문서 id.
    pub fn document_id(&self) -> StableId {
        self.document_id
    }

    /// project id.
    pub fn project_id(&self) -> StableId {
        self.project_id
    }

    /// dirty 판정과 창 history(읽기 전용).
    pub fn history(&self) -> &DirtyTrackedHistory {
        &self.history
    }

    /// 디스크 동기 상태(읽기 전용).
    pub fn disk(&self) -> &WindowExternalState {
        &self.disk
    }

    /// 마지막 저장 상태(다음 저장의 optimistic check 기준).
    pub fn saved(&self) -> &SavedGenerationState {
        &self.saved
    }

    /// 자기 저장 기록(coalescer·pointer 판정 입력).
    pub fn ledger(&self) -> &OwnSaveLedger {
        &self.ledger
    }

    /// 로컬 미저장 변경이 없는지(dirty 판정과 디스크 동기 상태 모두 clean).
    pub fn is_clean(&self) -> bool {
        !self.history.is_dirty() && matches!(self.disk.state(), WindowDiskState::Clean { .. })
    }

    /// preview 중이라 미룬 마지막 외부 generation을 꺼냅니다.
    pub fn take_deferred_external(&mut self) -> Option<ContentHash> {
        self.deferred.take()
    }

    /// 편집 뒤 dirty 판정을 디스크 동기 상태에 반영합니다(clean → dirty).
    fn sync_dirty(&mut self) {
        if self.history.is_dirty() {
            self.disk.mark_dirty();
        }
    }

    /// 완료된 document transaction을 실행·기록합니다.
    pub fn record_document(
        &mut self,
        transaction: &CommandTransaction,
        grouping: Option<GroupingKey>,
    ) -> Result<ChangeSet, HistoryError> {
        let result = self
            .history
            .record_document(&mut self.document, transaction, grouping);
        self.sync_dirty();
        result
    }

    /// 연속 입력 grouping으로 완료 transaction을 기록합니다.
    pub fn record_grouped<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<ChangeSet, HistoryError> {
        let result = self
            .history
            .record_grouped(&mut self.document, transaction, key, clock);
        self.sync_dirty();
        result
    }

    /// 선택 변경을 기록합니다(dirty 없음).
    pub fn record_selection(
        &mut self,
        event: &SetSelection,
    ) -> Result<Option<HistorySequence>, HistoryError> {
        self.history.record_selection(&self.document, event)
    }

    /// pointer gesture preview를 시작합니다.
    pub fn begin_pointer_gesture(&mut self, gesture: u64) -> Result<(), HistoryError> {
        self.history.begin_pointer_gesture(gesture)
    }

    /// slider·scrub preview를 시작합니다.
    pub fn begin_scrub(&mut self, control: u64) -> Result<(), HistoryError> {
        self.history.begin_scrub(control)
    }

    /// preview를 교체합니다(dirty 없음).
    pub fn update_preview(
        &mut self,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        self.history.update_preview(&mut self.document, transaction)
    }

    /// preview를 확정합니다(항목이 기록되면 dirty).
    pub fn end_preview(&mut self) -> Result<Option<HistorySequence>, HistoryError> {
        let result = self.history.end_preview(&self.document);
        self.sync_dirty();
        result
    }

    /// preview를 취소합니다.
    pub fn cancel_preview(&mut self) -> Result<(), HistoryError> {
        self.history.cancel_preview(&mut self.document)
    }

    /// 마지막 항목을 되돌립니다.
    pub fn undo(&mut self) -> Result<Option<UndoOutcome>, HistoryError> {
        let result = self.history.undo(&mut self.document);
        self.sync_dirty();
        result
    }

    /// 되돌린 항목을 다시 적용합니다.
    pub fn redo(&mut self) -> Result<Option<UndoOutcome>, HistoryError> {
        let result = self.history.redo(&mut self.document);
        self.sync_dirty();
        result
    }

    /// 외부의 새 generation(XIII.2.5 `ExternalChange::Generation`)을 반영합니다(모듈 문서 처리 순서).
    pub fn on_external_generation(
        &mut self,
        manifest_hash: &ContentHash,
    ) -> Result<ExternalReloadOutcome, WindowSessionError> {
        if self.history.preview_active() {
            self.deferred = Some(*manifest_hash);
            return Ok(ExternalReloadOutcome::Deferred);
        }
        self.deferred = None;
        self.sync_dirty();
        let document = &self.document;
        let own_chunks = &self.own_chunks;
        let decision = self
            .disk
            .on_external_generation(manifest_hash, |base| {
                let serialized =
                    serialize_document(document.model()).map_err(SnapshotError::Serialize)?;
                let (states, roots) = memory_states(base, &serialized, own_chunks);
                MemorySnapshot::capture(base, states, roots)
            })
            .map_err(WindowSessionError::External)?;
        match decision {
            ExternalChangeDecision::Unchanged => Ok(ExternalReloadOutcome::Unchanged),
            ExternalChangeDecision::Pending(hashes) => Ok(ExternalReloadOutcome::Pending(hashes)),
            ExternalChangeDecision::CleanWindow { external } => self.reload_clean(external),
        }
    }

    /// clean 창을 `external` generation으로 갱신합니다. 모두 읽은 뒤에만 상태를 바꿉니다.
    fn reload_clean(
        &mut self,
        external: DiskSnapshot,
    ) -> Result<ExternalReloadOutcome, WindowSessionError> {
        let project = SavedProjectLoader::new()
            .open(&self.root)
            .map_err(WindowSessionError::Open)?;
        if project.project_id != self.project_id {
            return Err(WindowSessionError::ProjectChanged {
                expected: self.project_id,
                found: project.project_id,
            });
        }
        if project.generation.manifest() != &external.manifest {
            return Ok(ExternalReloadOutcome::Superseded);
        }
        let (model, own_chunks) = load_own_document(&project, self.document_id)?;

        // 여기부터 실패 없음: 문서 교체, history·선택 폐기, 기준 generation 갱신.
        let discarded =
            self.history.history().history().len() + self.history.history().history().redo_len();
        let revision = DocumentRevision(self.document.revision().0.saturating_add(1));
        self.document = DocumentState::with_revision(model, revision);
        self.history = DirtyTrackedHistory::new(WindowHistory::new(self.window), revision);
        let generation = external.generation();
        let manifest_hash = external.manifest_hash;
        self.disk.adopt_reloaded(external);
        self.saved = SavedGenerationState::new(Some((generation, manifest_hash)));
        self.ledger.record(generation, manifest_hash);
        self.own_chunks = own_chunks;
        Ok(ExternalReloadOutcome::Reloaded {
            generation,
            manifest_hash,
            discarded_history_entries: discarded,
        })
    }

    /// 완료 편집을 호출한 thread에서 끝까지 저장합니다(V.5 파이프라인, 외부 변경 대기 중이면 거부).
    ///
    /// 성공하면 새 generation이 base·마지막 저장 상태·자기 저장 기록의 최신 값이 되고, 저장한 revision까지
    /// clean입니다.
    pub fn save_blocking(
        &mut self,
        cancel: &CancelToken,
    ) -> Result<SessionSave, WindowSessionError> {
        if !self.history.is_dirty() {
            return Ok(SessionSave::Clean);
        }
        if self.history.preview_active() {
            return Ok(SessionSave::PreviewInProgress);
        }
        let revision = self.document.revision();
        let serialized =
            serialize_document(self.document.model()).map_err(WindowSessionError::Serialize)?;
        let base = self.disk.base().clone();
        let (states, roots) = memory_states(&base, &serialized, &self.own_chunks);
        let loaded = LoadedGeneration {
            manifest_hash: base.manifest_hash,
            manifest: base.manifest.clone(),
        };
        let plan = build_save_plan(
            Some(&loaded),
            self.project_id,
            base.manifest.schema_version,
            &states,
            roots,
        )
        .map_err(WindowSessionError::Plan)?;
        let mut saved = self.saved.clone();
        self.disk
            .run_guarded_save(&plan, &mut saved, cancel)
            .map_err(WindowSessionError::Save)?;
        self.saved = saved;
        self.ledger.record_saved(&self.saved);
        self.history.mark_saved(revision);
        self.own_chunks = serialized_chunk_ids(&serialized);
        Ok(SessionSave::Saved {
            revision,
            generation: plan.target_generation,
        })
    }
}

/// 읽은 chunk id를 모으는 recorder.
#[derive(Default)]
struct ChunkIdRecorder {
    chunks: BTreeSet<ChunkId>,
}

impl ChunkReadRecorder for ChunkIdRecorder {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.chunks.insert(pointer.chunk_id);
    }
}

/// 연 프로젝트에서 `document`를 읽고, 그 문서가 차지한 chunk(DocumentManifest 포함)를 돌려줍니다.
fn load_own_document(
    project: &SavedProject,
    document: StableId,
) -> Result<(DocumentModel, BTreeSet<ChunkId>), WindowSessionError> {
    let mut recorder = ChunkIdRecorder::default();
    let loaded = load_document_from_generation(&project.generation, document, &mut recorder)
        .map_err(WindowSessionError::Document)?;
    let mut chunks = recorder.chunks;
    chunks.insert(manifest_chunk_id(document));
    Ok((loaded.model, chunks))
}

fn serialized_chunk_ids(serialized: &SerializedDocument) -> BTreeSet<ChunkId> {
    serialized
        .chunks()
        .into_iter()
        .map(|chunk| chunk.chunk_id)
        .collect()
}

/// base 위 메모리 상태: 이 창 문서 chunk는 새로 직렬화한 값, 다른 문서 chunk는 base pointer 그대로.
fn memory_states(
    base: &DiskSnapshot,
    serialized: &SerializedDocument,
    own_chunks: &BTreeSet<ChunkId>,
) -> (Vec<LogicalChunkState>, Vec<StableId>) {
    let written = serialized_chunk_ids(serialized);
    let mut states = serialized.to_dirty_states();
    for pointer in &base.manifest.chunk_pointers {
        if !own_chunks.contains(&pointer.chunk_id) && !written.contains(&pointer.chunk_id) {
            states.push(LogicalChunkState::Clean {
                chunk_id: pointer.chunk_id,
            });
        }
    }
    let mut roots = base.manifest.root_document_ids.clone();
    if !roots.contains(&serialized.document_id) {
        roots.push(serialized.document_id);
    }
    (states, roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autosave::dirty::tests::{document, select, sid, tx};
    use crate::saved_project::tests::{save_documents, write_manifest, TempDir, PROJECT_ID_TEXT};
    use crate::watcher::coalesce::{ChangeCoalescer, ExternalChange};
    use crate::watcher::{FileWatchEvent, WatchPath};
    use stellar_commands::command::Command;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_storage::generation::current::write_current_pointer;
    use stellar_storage::generation::loader::load_current_generation;

    const QUIET: u64 = 10;

    fn window() -> WindowSessionId {
        WindowSessionId::from_stable_id(sid(900))
    }

    /// 문서 `base`의 node `base + 11` 이름만 바꾼 문서.
    fn renamed(base: u64, name: &str) -> DocumentModel {
        let mut model = document(base);
        model
            .set_display_name(sid(base + 11), name.to_string())
            .unwrap();
        model
    }

    fn json(model: &DocumentModel) -> String {
        model_snapshot_json(model).unwrap()
    }

    fn pointer(root: &Path) -> ContentHash {
        read_current_pointer(root).unwrap()
    }

    /// 저장된 프로젝트 하나(문서 0)와 그 위에 연 clean 세션.
    fn opened() -> (TempDir, WindowDocumentSession, Watch) {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        save_documents(&dir.path, &[&document(0)]);
        let session = WindowDocumentSession::open(&dir.path, sid(1), window()).unwrap();
        let watch = Watch::new(&dir.path, session.project_id());
        (dir, session, watch)
    }

    /// 파일 감시 → coalescer(XIII.2.5) 경로.
    struct Watch {
        coalescer: ChangeCoalescer,
        now: u64,
    }

    impl Watch {
        fn new(root: &Path, project: StableId) -> Watch {
            Watch {
                coalescer: ChangeCoalescer::new(root, project, Some(pointer(root)), QUIET),
                now: 0,
            }
        }

        /// current pointer 변경 event 뒤 조용한 구간이 지난 결과.
        fn poll(&mut self, ledger: &OwnSaveLedger) -> Vec<ExternalChange> {
            self.coalescer.push(
                &[FileWatchEvent::Modified(
                    WatchPath::project("design/current").unwrap(),
                )],
                self.now,
            );
            self.now += QUIET;
            self.coalescer.poll(self.now, ledger)
        }

        /// 외부 generation 하나(hash, number)를 기대합니다.
        fn generation(&mut self, ledger: &OwnSaveLedger) -> (ContentHash, u64) {
            let changes = self.poll(ledger);
            let [ExternalChange::Generation {
                manifest_hash,
                generation,
                ..
            }] = changes.as_slice()
            else {
                panic!("외부 generation 하나여야 함: {changes:?}");
            };
            (*manifest_hash, *generation)
        }
    }

    fn selection_is_empty(session: &WindowDocumentSession) -> bool {
        session.history().history().view().selection().is_empty()
    }

    #[test]
    fn clean_window_reloads_the_new_generation_and_discards_history() {
        let (dir, mut session, mut watch) = opened();
        let g1 = pointer(&dir.path);
        assert_eq!(json(session.document().model()), json(&document(0)));
        for ids in [[11u64], [12], [11]] {
            session.record_selection(&select(&ids)).unwrap();
        }
        assert_eq!(session.history().history().history().len(), 3);
        assert!(!selection_is_empty(&session));
        assert!(session.is_clean(), "선택만 바뀐 창은 clean");

        // 외부 writer(다른 창·Git·CLI)가 새 정상 generation을 저장.
        let external = renamed(0, "external edit");
        save_documents(&dir.path, &[&external]);
        let (g2, generation) = watch.generation(session.ledger());
        assert_eq!(generation, 2);
        assert_ne!(g1, g2);
        let before = session.document().revision();

        let outcome = session.on_external_generation(&g2).unwrap();
        assert_eq!(
            outcome,
            ExternalReloadOutcome::Reloaded {
                generation: 2,
                manifest_hash: g2,
                discarded_history_entries: 3,
            }
        );
        // 문서는 새 generation, history·선택은 폐기, 기준 generation 갱신.
        assert_eq!(json(session.document().model()), json(&external));
        assert!(session.history().history().history().is_empty());
        assert_eq!(session.history().history().history().redo_len(), 0);
        assert!(selection_is_empty(&session));
        assert!(
            session.document().revision() > before,
            "revision은 이어서 증가"
        );
        assert!(session.is_clean());
        assert_eq!(session.disk().base().manifest_hash, g2);
        assert_eq!(session.saved().saved(), Some((2, g2)));
        assert_eq!(
            session.ledger().latest().map(|save| save.manifest_hash),
            Some(g2)
        );
        assert_eq!(
            session.on_external_generation(&g2).unwrap(),
            ExternalReloadOutcome::Unchanged
        );

        // reload 뒤 편집은 새 문서 기준으로 기록되고 undo는 reload 이전으로 가지 않음.
        let t = tx(
            session.document(),
            7_000,
            Command::rename_node(sid(12), "after reload"),
        );
        session.record_document(&t, None).unwrap();
        assert!(!session.is_clean());
        assert!(session.undo().unwrap().is_some());
        assert!(session.undo().unwrap().is_none());
        assert_eq!(json(session.document().model()), json(&external));
    }

    #[test]
    fn window_with_local_changes_is_not_reloaded() {
        let (dir, mut session, mut watch) = opened();
        let g1 = pointer(&dir.path);
        let t = tx(
            session.document(),
            7_100,
            Command::rename_node(sid(11), "local edit"),
        );
        session.record_document(&t, None).unwrap();
        let local = json(session.document().model());
        save_documents(&dir.path, &[&renamed(0, "external edit")]);
        let (g2, _) = watch.generation(session.ledger());

        let ExternalReloadOutcome::Pending(hashes) = session.on_external_generation(&g2).unwrap()
        else {
            panic!("로컬 변경이 있으면 대기 상태여야 함");
        };
        assert_eq!(hashes.loaded_base, g1);
        assert_eq!(hashes.external_disk, g2);
        // 메모리 문서·history는 그대로이고 저장은 거부(자동 overwrite 없음).
        assert_eq!(json(session.document().model()), local);
        assert_eq!(session.history().history().history().len(), 1);
        assert!(matches!(
            session.save_blocking(&CancelToken::new()),
            Err(WindowSessionError::Save(
                GuardedSaveError::ExternalChangePending(_)
            ))
        ));
        assert_eq!(pointer(&dir.path), g2);
    }

    #[test]
    fn reload_waits_for_an_in_progress_preview() {
        let (dir, mut session, mut watch) = opened();
        session.begin_pointer_gesture(1).unwrap();
        let t = tx(
            session.document(),
            7_200,
            Command::rename_node(sid(11), "dragging"),
        );
        session.update_preview(&t).unwrap();
        let dragging = json(session.document().model());
        let external = renamed(0, "external edit");
        save_documents(&dir.path, &[&external]);
        let (g2, _) = watch.generation(session.ledger());

        assert_eq!(
            session.on_external_generation(&g2).unwrap(),
            ExternalReloadOutcome::Deferred
        );
        assert_eq!(
            json(session.document().model()),
            dragging,
            "진행 중 조작 유지"
        );
        session.cancel_preview().unwrap();
        let deferred = session.take_deferred_external().unwrap();
        assert_eq!(deferred, g2);
        assert!(matches!(
            session.on_external_generation(&deferred).unwrap(),
            ExternalReloadOutcome::Reloaded { generation: 2, .. }
        ));
        assert_eq!(json(session.document().model()), json(&external));
        assert_eq!(session.take_deferred_external(), None);
    }

    #[test]
    fn superseded_generation_is_not_applied() {
        let (dir, mut session, mut watch) = opened();
        let g1 = pointer(&dir.path);
        save_documents(&dir.path, &[&renamed(0, "second")]);
        let (g2, _) = watch.generation(session.ledger());
        let third = renamed(0, "third");
        save_documents(&dir.path, &[&third]);
        let (g3, _) = watch.generation(session.ledger());

        // g2를 처리하기 전에 pointer가 g3로 바뀜: 바꾸지 않고 다음 변화(g3)로 갱신.
        assert_eq!(
            session.on_external_generation(&g2).unwrap(),
            ExternalReloadOutcome::Superseded
        );
        assert_eq!(json(session.document().model()), json(&document(0)));
        assert_eq!(session.disk().base().manifest_hash, g1);
        assert!(matches!(
            session.on_external_generation(&g3).unwrap(),
            ExternalReloadOutcome::Reloaded { generation: 3, .. }
        ));
        assert_eq!(json(session.document().model()), json(&third));
    }

    #[test]
    fn invalid_external_generation_is_not_partially_applied() {
        let (dir, mut session, mut watch) = opened();
        let g1 = pointer(&dir.path);
        let before = load_current_generation(&dir.path, session.project_id()).unwrap();
        save_documents(&dir.path, &[&renamed(0, "external edit")]);
        let (g2, _) = watch.generation(session.ledger());
        // 새 generation에만 있는 chunk 하나의 내용을 손상(파일은 그대로 존재).
        let after = load_current_generation(&dir.path, session.project_id()).unwrap();
        let fresh = after
            .manifest
            .chunk_pointers
            .iter()
            .find(|p| !before.manifest.chunk_pointers.contains(p))
            .unwrap();
        let path = dir.path.join(
            stellar_storage::chunk::path::ChunkObjectPath::for_hash(&fresh.content_hash)
                .to_relative_path(),
        );
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, bytes).unwrap();

        assert!(session.on_external_generation(&g2).is_err());
        // 아무 상태도 바뀌지 않음.
        assert_eq!(json(session.document().model()), json(&document(0)));
        assert_eq!(session.disk().base().manifest_hash, g1);
        assert_eq!(session.saved().saved(), Some((1, g1)));
        assert_eq!(
            session.ledger().latest().map(|save| save.manifest_hash),
            Some(g1)
        );
        assert!(session.is_clean());
    }

    #[test]
    fn pointer_reverted_to_an_earlier_own_save_reloads_after_an_external_reload() {
        let (dir, mut session, mut watch) = opened();
        // 자기 저장 B(g2): 감시 event는 자기 저장이라 reload하지 않음.
        let t = tx(
            session.document(),
            7_300,
            Command::rename_node(sid(11), "own save"),
        );
        session.record_document(&t, None).unwrap();
        let own = json(session.document().model());
        assert!(matches!(
            session.save_blocking(&CancelToken::new()).unwrap(),
            SessionSave::Saved { generation: 2, .. }
        ));
        let g2 = pointer(&dir.path);
        assert!(session.is_clean());
        assert!(watch.poll(session.ledger()).is_empty(), "자기 저장");

        // 외부 generation X(g3)를 clean 창이 reload.
        let external = renamed(0, "external edit");
        save_documents(&dir.path, &[&external]);
        let (g3, _) = watch.generation(session.ledger());
        assert!(matches!(
            session.on_external_generation(&g3).unwrap(),
            ExternalReloadOutcome::Reloaded { generation: 3, .. }
        ));

        // Git checkout 등으로 pointer가 예전 자기 저장 B로 되돌아감: 창은 X를 보여 주므로 외부 변경.
        write_current_pointer(&dir.path, &g2).unwrap();
        let (reverted, generation) = watch.generation(session.ledger());
        assert_eq!((reverted, generation), (g2, 2));
        assert!(matches!(
            session.on_external_generation(&g2).unwrap(),
            ExternalReloadOutcome::Reloaded { generation: 2, .. }
        ));
        assert_eq!(json(session.document().model()), own);
    }

    #[test]
    fn other_documents_keep_their_chunks_through_session_saves() {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        let other = document(100);
        save_documents(&dir.path, &[&document(0), &other]);
        let mut session = WindowDocumentSession::open(&dir.path, sid(1), window()).unwrap();
        let t = tx(
            session.document(),
            7_400,
            Command::rename_node(sid(11), "edited in window"),
        );
        session.record_document(&t, None).unwrap();
        session.save_blocking(&CancelToken::new()).unwrap();

        let project = SavedProjectLoader::new().open(&dir.path).unwrap();
        assert_eq!(project.generation_number(), 2);
        let mut ids = project.document_ids().unwrap();
        ids.sort();
        assert_eq!(ids, vec![sid(1), sid(101)]);
        assert_eq!(
            json(&project.load_document(sid(101)).unwrap().model),
            json(&other)
        );
        assert_eq!(
            json(&project.load_document(sid(1)).unwrap().model),
            json(session.document().model())
        );
        // 저장 뒤 다른 문서만 바뀐 외부 generation도 이 창 문서는 그대로 다시 읽음.
        let mut watch = Watch::new(&dir.path, session.project_id());
        let edited = project.load_document(sid(1)).unwrap().model;
        save_documents(&dir.path, &[&edited, &renamed(100, "other doc edit")]);
        let (g3, _) = watch.generation(session.ledger());
        assert!(matches!(
            session.on_external_generation(&g3).unwrap(),
            ExternalReloadOutcome::Reloaded { generation: 3, .. }
        ));
        assert_eq!(json(session.document().model()), json(&edited));
    }
}
