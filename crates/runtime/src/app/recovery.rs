//! 저장 세대와 분리된 활성 문서 복구 체크포인트.
use super::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use stellar_commands::selection::Selection;
use stellar_commands::state::DocumentState;
use stellar_commands::view_tabs::CanvasViewport;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_history::window::WindowHistory;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::{
    manifest_chunk_id, serialize_document, serialize_records,
};
use stellar_project::recovery::RecoverySnapshotStore;
use stellar_project::recovery_payload::{OpenTab, RecoveryPayload, UnsavedChunk, Viewport};
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::hash::ContentHash;
#[cfg(test)]
mod tests;
mod writer;
pub(super) use writer::WindowCheckpoint;

/// 앱이 표시한 한 후보. action은 동일 bytes hash를 다시 검사합니다.
#[derive(Debug, Clone)]
pub struct RecoveryCandidate {
    /// 비정상 종료한 세션.
    pub window: WindowSessionId,
    /// 제안한 파일 자체의 hash.
    pub hash: ContentHash,
    /// 기준이 디스크의 현재 세대와 같은지 여부.
    pub matches_base: bool,
}
#[derive(Debug)]
struct Capture {
    records: DocumentRecords,
    payload: RecoveryPayload,
}
impl Capture {
    fn from_window(window: &ProjectWindowSession) -> io::Result<Self> {
        let document = window
            .document()
            .ok_or_else(|| io::Error::other("열린 문서가 없습니다"))?;
        let tabs = window.tabs();
        let active_tab = tabs
            .tabs()
            .iter()
            .position(|t| Some(t.view) == tabs.active_view())
            .unwrap_or(0) as u32;
        Ok(Self {
            records: document.model().to_records().map_err(io::Error::other)?,
            payload: RecoveryPayload {
                base_manifest_hash: window.loaded_generation().map(|g| g.manifest_hash),
                revision: document.revision().0,
                unsaved_chunks: Vec::new(),
                open_tabs: tabs
                    .tabs()
                    .iter()
                    .map(|t| OpenTab {
                        view: t.view,
                        viewport: Viewport {
                            center_x: t.viewport.center_x(),
                            center_y: t.viewport.center_y(),
                            zoom: t.viewport.zoom(),
                        },
                    })
                    .collect(),
                active_tab,
                selection: window.view().selection().ids().to_vec(),
                target_profile: StableId::NIL,
            },
        })
    }
    fn write(
        mut self,
        store: &RecoverySnapshotStore,
        id: &ProjectIdentity,
        window: WindowSessionId,
    ) -> io::Result<()> {
        let serialized = serialize_records(&self.records).map_err(io::Error::other)?;
        // 현재 checkout에 원래 chunk가 없어도 이 한 문서의 복구 사본을 만들 수 있게 전부 담습니다.
        self.payload.unsaved_chunks = std::iter::once(serialized.manifest)
            .chain(serialized.view_trees)
            .chain(serialized.subtrees)
            .map(|c| UnsavedChunk {
                chunk_id: c.chunk_id,
                kind: c.kind,
                payload: c.payload,
            })
            .collect();
        store
            .write_snapshot(
                id,
                window,
                &self.payload.encode().map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)
    }
}
fn load_model(payload: &RecoveryPayload) -> Result<DocumentModel, String> {
    let mut manifests = payload
        .unsaved_chunks
        .iter()
        .filter(|c| c.kind == KIND_DOCUMENT_MANIFEST);
    let manifest = manifests
        .next()
        .filter(|_| manifests.next().is_none())
        .ok_or("복구 문서 manifest가 하나여야 합니다")?;
    let mut chunks = MemoryChunks::new();
    let mut ids = BTreeSet::new();
    for c in &payload.unsaved_chunks {
        if !ids.insert(c.chunk_id) {
            return Err("복구 chunk ID가 중복되었습니다".into());
        }
        chunks.insert(c.chunk_id, c.payload.clone());
    }
    let model = load_document(&manifest.payload, &mut chunks)
        .map_err(|e| e.to_string())?
        .model;
    if manifest.chunk_id != manifest_chunk_id(model.document_id()) {
        return Err("복구 문서 ID가 일치하지 않습니다".into());
    }
    // 모르는 chunk·속성이 있다면 일부만 다시 저장하지 않습니다.
    let canonical = serialize_document(&model).map_err(|e| e.to_string())?;
    let original: std::collections::BTreeMap<_, _> = payload
        .unsaved_chunks
        .iter()
        .map(|c| (c.chunk_id, (c.kind, &c.payload)))
        .collect();
    if canonical.chunks().len() != original.len()
        || !canonical
            .chunks()
            .iter()
            .all(|c| original.get(&c.chunk_id) == Some(&(c.kind, &c.payload)))
    {
        return Err("복구 내용을 손실 없이 재구성할 수 없습니다".into());
    }
    Ok(model)
}
fn lock_candidate(
    store: &RecoverySnapshotStore,
    id: &ProjectIdentity,
    window: WindowSessionId,
) -> io::Result<fs::File> {
    let p = store.snapshot_path(id, window).with_extension("lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(p)?;
    file.try_lock().map_err(io::Error::from)?;
    Ok(file)
}
fn payload_bytes(
    store: &RecoverySnapshotStore,
    id: &ProjectIdentity,
    window: WindowSessionId,
) -> Result<Vec<u8>, String> {
    store
        .read_snapshot(id, window)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "복구본이 더 이상 존재하지 않습니다".into())
}
impl RecoveryCandidate {
    /// 셸이 보관한 불투명 ticket에서 원본 ID와 bytes hash를 복원합니다.
    pub fn from_ticket(window: StableId, hash: &str) -> Result<Self, String> {
        Ok(Self {
            window: WindowSessionId::from_stable_id(window),
            hash: hash.parse().map_err(|_| "잘못된 복구 hash")?,
            matches_base: false,
        })
    }
}
impl ProjectSummary {
    fn recovery_store(&self) -> Result<(ProjectIdentity, RecoverySnapshotStore), String> {
        let id = ProjectIdentity::new(self.project_id, &self.root).map_err(|e| e.to_string())?;
        if id.canonical_target() != self.canonical_root {
            return Err("열린 프로젝트 폴더가 바뀌었습니다".into());
        }
        let base = os_user_data_base()
            .filter(|p| p.is_absolute())
            .ok_or("사용자 데이터 경로 없음")?;
        Ok((
            id,
            RecoverySnapshotStore::new(&base.join(stellar_ops::lock::APP_DATA_DIR)),
        ))
    }
    /// 다른 살아 있는 앱·창이 소유한 복구본은 제안에서 제외합니다. 최근 창부터 표시합니다.
    pub fn recovery_candidate(&self) -> Result<Option<RecoveryCandidate>, String> {
        let (id, store) = self.recovery_store()?;
        let base = read_current_pointer(id.canonical_target()).ok();
        let sessions = store.list_sessions(&id).map_err(|e| e.to_string())?;
        let mut unreadable = None;
        for name in sessions.into_iter().rev() {
            let Some(window) = parse_project_id(&name).map(WindowSessionId::from_stable_id) else {
                continue;
            };
            let _guard = match lock_candidate(&store, &id, window) {
                Ok(g) => g,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e.to_string()),
            };
            let Some(bytes) = store
                .read_snapshot(&id, window)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let payload = match RecoveryPayload::decode(&bytes) {
                Ok(payload) => payload,
                Err(error) => {
                    unreadable = Some(error.to_string());
                    continue;
                }
            };
            if payload.base_manifest_hash.is_none() || payload.revision == 0 {
                continue;
            }
            return Ok(Some(RecoveryCandidate {
                window,
                hash: ContentHash::compute(&bytes),
                matches_base: payload.base_manifest_hash == base,
            }));
        }
        unreadable.map_or(Ok(None), Err)
    }
    fn read_recovery(
        &self,
        candidate: &RecoveryCandidate,
    ) -> Result<
        (
            ProjectIdentity,
            RecoverySnapshotStore,
            fs::File,
            RecoveryPayload,
        ),
        String,
    > {
        let (id, store) = self.recovery_store()?;
        let guard = lock_candidate(&store, &id, candidate.window).map_err(|e| e.to_string())?;
        let bytes = payload_bytes(&store, &id, candidate.window)?;
        if ContentHash::compute(&bytes) != candidate.hash {
            return Err("표시한 복구본이 바뀌었습니다".into());
        }
        let payload = RecoveryPayload::decode(&bytes).map_err(|e| e.to_string())?;
        Ok((id, store, guard, payload))
    }
    /// 사용자가 정한 새 폴더에 활성 문서만 복원합니다. 원본 checkout은 읽기 전용입니다.
    fn export_recovery(&self, candidate: &RecoveryCandidate, folder: &Path) -> Result<(), String> {
        use stellar_project::{
            fs_transaction::DirTransaction, manifest::CURRENT_SCHEMA_VERSION,
            scaffold::write_default_scaffold,
        };
        use stellar_storage::save::{
            cancel::{run_save, CancelToken},
            commit::SavedGenerationState,
            plan::build_save_plan,
        };
        let (id, store, _guard, payload) = self.read_recovery(candidate)?;
        let parent = folder
            .parent()
            .ok_or("새 복구 폴더가 필요합니다")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if parent.starts_with(id.canonical_target()) || !folder.is_absolute() {
            return Err("원본 프로젝트 밖에 새 폴더를 선택하세요".into());
        }
        let name = folder
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("새 복구 폴더 이름이 필요합니다")?;
        stellar_project::folder_name::validate_folder_name(name).map_err(|e| e.to_string())?;
        let model = load_model(&payload)?;
        let serialized = serialize_document(&model).map_err(|e| e.to_string())?;
        let [project] = stellar_project::new_ids::new_ids().map_err(|e| e.to_string())?;
        let plan = build_save_plan(
            None,
            project,
            CURRENT_SCHEMA_VERSION,
            &serialized.to_dirty_states(),
            vec![model.document_id()],
        )
        .map_err(|e| e.to_string())?;
        let (runner, version) = super::create::project_git()?;
        let tx = DirTransaction::create(&parent.join(name)).map_err(|e| e.to_string())?;
        write_default_scaffold(tx.path(), name, &project.to_string()).map_err(|e| e.to_string())?;
        run_save(
            tx.path(),
            &plan,
            &mut SavedGenerationState::new(None),
            &CancelToken::new(),
        )
        .map_err(|e| e.to_string())?;
        stellar_git_porcelain::repo::git_init_main(&runner, &version, tx.path())
            .map_err(|e| e.to_string())?;
        stellar_project::saved_project::SavedProjectLoader::new()
            .open(tx.path())
            .map_err(|e| e.to_string())?
            .load_document(model.document_id())
            .map_err(|e| e.to_string())?;
        tx.commit();
        store
            .remove_snapshot(&id, candidate.window)
            .map_err(|e| e.to_string())
    }
}

/// worker에서 검증한 복구본. 새 편집 또는 다른 세대에 뒤늦게 적용하지 않습니다.
#[derive(Debug)]
struct PreparedRecovery {
    project: ProjectIdentity,
    candidate: RecoveryCandidate,
    lease: fs::File,
    base: ContentHash,
    model: DocumentModel,
    payload: RecoveryPayload,
}
impl ProjectSummary {
    /// 파일 읽기와 문서 조립은 셸 worker에서 수행합니다.
    fn prepare_recovery(&self, candidate: &RecoveryCandidate) -> Result<PreparedRecovery, String> {
        let (project, _, lease, payload) = self.read_recovery(candidate)?;
        let base = payload
            .base_manifest_hash
            .ok_or("저장된 기준 세대가 없습니다")?;
        if read_current_pointer(project.canonical_target()).ok() != Some(base) {
            return Err("디스크 세대가 바뀌었습니다. 별도 문서로 복구하세요".into());
        }
        let model = load_model(&payload)?;
        Ok(PreparedRecovery {
            project,
            candidate: candidate.clone(),
            lease,
            base,
            model,
            payload,
        })
    }
}
impl AppRuntime {
    /// 셸은 request만 즉시 접수합니다. 파일 읽기와 검증은 worker가 수행합니다.
    pub fn recover_checkpoint(
        &self,
        session: WindowSessionHandle,
        candidate: RecoveryCandidate,
        destination: Option<PathBuf>,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        let state = self.state();
        let shared = self.shared.clone();
        let sender = self.sender.clone();
        let launch = std::thread::Builder::new()
            .name("stellar-recover-document".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> Result<(), String> {
                    let p = state
                        .window(session)
                        .and_then(|w| state.project(w.project))
                        .ok_or("열린 창이 없습니다")?;
                    if let Some(dest) = destination {
                        return p.export_recovery(&candidate, &dest);
                    }
                    let ready = p.prepare_recovery(&candidate)?;
                    let snapshot = shared.clone();
                    sender
                        .submit(move |ctx| {
                            apply_recovery(ctx, session, ready)?;
                            snapshot.publish_state(build_state(ctx));
                            Ok::<_, Failure>(())
                        })
                        .map_err(|e| e.to_string())?
                        .wait()
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.message)
                })
                .unwrap_or_else(|_| Err("복구 작업이 중단되었습니다".into()));
                shared.outbox().push_back(match result {
                    Ok(()) => RuntimeEvent::WindowViewChanged { request, session },
                    Err(message) => RuntimeEvent::RequestFailed {
                        request,
                        failure: RequestFailure::WindowSessionLoadFailed,
                        message,
                    },
                });
            });
        if launch.is_err() {
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}
fn apply_recovery(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
    recovery: PreparedRecovery,
) -> Result<(), Failure> {
    let entry = ctx
        .app_sessions_mut()
        .windows
        .get_mut(&session)
        .ok_or_else(|| unknown_window(session))?;
    let w = &mut entry.session;
    if w.identity() != &recovery.project
        || w.has_unsaved_changes()
        || w.revision().0 != 0
        || w.document_id() != Some(recovery.model.document_id())
        || w.loaded_generation().map(|g| g.manifest_hash) != Some(recovery.base)
        || read_current_pointer(w.identity().canonical_target()).ok() != Some(recovery.base)
    {
        return Err(Failure::new(
            RequestFailure::SaveStale,
            "창 또는 디스크 세대가 바뀌었습니다. 별도 문서로 복구하세요",
        ));
    }
    let p = recovery.payload;
    let model = recovery.model;
    let mut history = WindowHistory::new(w.window());
    let tabs = history.view_mut().tabs_mut();
    for tab in &p.open_tabs {
        let viewport = CanvasViewport::new(
            tab.viewport.center_x,
            tab.viewport.center_y,
            tab.viewport.zoom,
        )
        .map_err(|e| Failure::new(RequestFailure::WindowSessionLoadFailed, e.to_string()))?;
        tabs.open(&model, tab.view)
            .map_err(|e| Failure::new(RequestFailure::WindowSessionLoadFailed, e.to_string()))?;
        tabs.set_active_viewport(viewport)
            .map_err(|e| Failure::new(RequestFailure::WindowSessionLoadFailed, e.to_string()))?;
    }
    if let Some(tab) = p.open_tabs.get(p.active_tab as usize) {
        tabs.activate(tab.view)
            .map_err(|e| Failure::new(RequestFailure::WindowSessionLoadFailed, e.to_string()))?;
    }
    history.view_mut().restore_selection(
        &Selection::new(p.selection.iter().copied(), p.selection.first().copied()),
        &model,
    );
    w.history = history;
    w.document.as_mut().expect("checked document").state =
        DocumentState::with_revision(model, DocumentRevision(p.revision));
    entry
        .checkpoint
        .retain_previous(recovery.candidate.window, recovery.lease);
    Ok(())
}
