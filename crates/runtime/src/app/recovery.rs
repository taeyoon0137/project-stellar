//! 저장 세대와 분리된 활성 문서 복구 체크포인트(아키텍처 §22.3, V.9).
//!
//! 제안은 V.9.3 규칙을 따릅니다: 기준 세대가 디스크의 current와 같고 미저장 revision이 있는 복구본만
//! 제안합니다. 사용자는 Restore(이 창 메모리에 복원), Discard(복구본 삭제), Later(그대로 둠) 중 하나를
//! 고릅니다. 복구 동작은 디스크의 프로젝트 파일을 쓰지 않습니다.
//!
//! 앱 시작 뒤 창을 열면 셸은 [`AppRuntime::recovery_probe`]로 최신 복구본 하나의 probe 결과를 받고, 사용자의
//! 선택을 [`AppRuntime::recover_checkpoint`]로 넘깁니다(XIII.8.3). 두 함수는 runtime이 복구본을 쓰는 같은
//! 저장소([`AppRuntime::enable_recovery`])를 읽습니다. 저장된 복구본 하나의 판정은
//! [`judge_stored_snapshot`]이고(폐기 표식·payload·기준 세대), 순서는 마지막에 기록한 복구본부터입니다.
//! 정상 저장·닫기·Discard의 삭제는 [`discard_snapshot`]이라, 삭제가 실패해 남은 복구본은 다음 시작에서
//! stale로 무시합니다(XIII.8.7, ADR-0120 결정 2).
use super::*;
use std::fs;
use std::io;
use std::path::PathBuf;
use stellar_commands::selection::Selection;
use stellar_commands::state::DocumentState;
use stellar_commands::view_tabs::CanvasViewport;
use stellar_document::model::DocumentModel;
use stellar_history::window::WindowHistory;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::{manifest_chunk_id, serialize_document};
use stellar_project::recovery::RecoverySnapshotStore;
use stellar_project::recovery_payload::{OpenTab, RecoveryPayload, UnsavedChunk, Viewport};
use stellar_project::recovery_probe::{judge_stored_snapshot, StoredSnapshot};
use stellar_project::recovery_shutdown::discard_snapshot;
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::hash::ContentHash;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
mod writer;
pub(super) use writer::{CheckpointConfig, RecoveryWorkers, WindowCheckpoint};

/// 앱이 표시한 한 후보. 동작은 같은 bytes hash인지 다시 검사한 뒤에만 적용합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCandidate {
    /// 복구본을 남긴(비정상 종료한) 창 편집 session.
    pub window: WindowSessionId,
    /// 제안한 복구본 파일 bytes의 hash.
    pub hash: ContentHash,
}

/// 셸에 돌려주는 복구 probe 결과(XIII.8.3). 사용자에게 보일 요약과, 선택을 적용할 때 넘길 후보입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryProbe {
    /// 선택을 적용할 때 [`AppRuntime::recover_checkpoint`]에 넘길 후보.
    pub candidate: RecoveryCandidate,
    /// 복구본의 미저장 revision(1 이상).
    pub revision: u64,
    /// 복구본에 기록한 열린 View 탭 수.
    pub open_tabs: usize,
    /// 복구본 파일을 마지막으로 기록한 시각(읽을 수 없으면 `None`).
    pub written_at: Option<std::time::SystemTime>,
}

/// 복구 제안에 대한 사용자의 결정. Later는 아무것도 하지 않으므로 요청이 없습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// 이 창 session 메모리에 복원합니다. 디스크는 다음 Save Now·정상 닫기의 일반 저장으로 바뀝니다.
    Restore,
    /// 복구본을 지웁니다. 다음 열기에 다시 제안하지 않습니다.
    Discard,
}
/// 복구 체크포인트 캡처. 문서는 자동 저장과 같은 revision의 불변 snapshot을 함께 씁니다(XIII.8.2).
#[derive(Debug)]
struct Capture {
    snapshot: Arc<crate::window_session::SharedSnapshot>,
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
            snapshot: window
                .shared_snapshot()
                .ok_or_else(|| io::Error::other("열린 문서가 없습니다"))?
                .map_err(io::Error::other)?,
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
        let serialized = self.snapshot.serialized().map_err(io::Error::other)?;
        // 현재 checkout에 원래 chunk가 없어도 이 한 문서의 복구 사본을 만들 수 있게 전부 담습니다.
        self.payload.unsaved_chunks = serialized
            .chunks()
            .into_iter()
            .map(|c| UnsavedChunk {
                chunk_id: c.chunk_id,
                kind: c.kind,
                payload: c.payload.clone(),
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
/// 잠금을 쥔 쪽이 복구본을 지운 뒤 잠금 파일도 지웁니다. 다른 프로세스는 try_lock만 하므로 기다리지 않습니다.
fn remove_lock(store: &RecoverySnapshotStore, id: &ProjectIdentity, window: WindowSessionId) {
    let _ = fs::remove_file(store.snapshot_path(id, window).with_extension("lock"));
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
    /// 셸이 probe 결과로 받은 창 ID와 SHA-256 digest에서 후보를 다시 만듭니다.
    pub fn from_digest(window: StableId, digest: &[u8; 32]) -> Self {
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        Self {
            window: WindowSessionId::from_stable_id(window),
            hash: format!("sha256:{hex}")
                .parse()
                .expect("sha256 digest 문자열은 항상 유효합니다"),
        }
    }
}
impl ProjectSummary {
    /// OS 사용자 데이터의 복구본 저장소 root([`AppRuntime::enable_recovery`]와 같은 위치).
    fn recovery_root() -> Result<PathBuf, String> {
        let base = os_user_data_base()
            .filter(|p| p.is_absolute())
            .ok_or("사용자 데이터 경로 없음")?;
        Ok(base.join(stellar_ops::lock::APP_DATA_DIR))
    }
    fn recovery_store_in(
        &self,
        root: &std::path::Path,
    ) -> Result<(ProjectIdentity, RecoverySnapshotStore), String> {
        let id = ProjectIdentity::new(self.project_id, &self.root).map_err(|e| e.to_string())?;
        if id.canonical_target() != self.canonical_root {
            return Err("열린 프로젝트 폴더가 바뀌었습니다".into());
        }
        Ok((id, RecoverySnapshotStore::new(root)))
    }
    /// 제안할 복구본 하나. V.9.3 규칙(기준 세대 = current, 미저장 revision 있음)을 따르고, 다른 살아 있는
    /// 앱·창이 잠근 복구본, 읽을 수 없는 복구본, 폐기했지만 삭제하지 못한 복구본은 제외합니다. 마지막에 기록한
    /// 복구본부터 고릅니다. OS 사용자 데이터의 저장소를 읽습니다(셸 FFI 경로). 새 코드는
    /// [`AppRuntime::recovery_probe`]를 씁니다.
    pub fn recovery_candidate(&self) -> Result<Option<RecoveryCandidate>, String> {
        self.recovery_candidate_in(&Self::recovery_root()?)
    }
    /// [`ProjectSummary::recovery_candidate`]를 복구본 저장소 `root`에서 찾습니다.
    pub(in crate::app) fn recovery_candidate_in(
        &self,
        root: &std::path::Path,
    ) -> Result<Option<RecoveryCandidate>, String> {
        Ok(self.recovery_probe_in(root)?.map(|probe| probe.candidate))
    }
    /// 복구본 저장소 `root`에서 제안할 복구본 하나의 probe 결과를 만듭니다.
    pub(in crate::app) fn recovery_probe_in(
        &self,
        root: &std::path::Path,
    ) -> Result<Option<RecoveryProbe>, String> {
        let (id, store) = self.recovery_store_in(root)?;
        let current = read_current_pointer(id.canonical_target()).ok();
        let sessions = store
            .list_sessions_newest_first(&id)
            .map_err(|e| e.to_string())?;
        for name in sessions {
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
            let (payload, hash) = match judge_stored_snapshot(&store, &id, window, current, &bytes)
            {
                StoredSnapshot::Fresh { payload, hash } => (payload, hash),
                StoredSnapshot::Stale { reason } => {
                    // V.9.3: 해석할 수 없는 복구본은 제안하지 않고 사유만 남깁니다(내용은 기록하지 않음).
                    if reason == stellar_project::recovery_probe::StaleReason::UnreadablePayload {
                        eprintln!("Stellar recovery: 읽을 수 없는 복구본을 건너뜁니다");
                    }
                    continue;
                }
            };
            // 저장한 적 없는 기준의 복구본은 이 창의 세대에 복원할 수 없으므로 제안하지 않습니다.
            if payload.base_manifest_hash.is_none() {
                continue;
            }
            return Ok(Some(RecoveryProbe {
                candidate: RecoveryCandidate { window, hash },
                revision: payload.revision,
                open_tabs: payload.open_tabs.len(),
                written_at: store.snapshot_modified(&id, window),
            }));
        }
        Ok(None)
    }
    fn read_recovery(
        &self,
        root: &std::path::Path,
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
        let (id, store) = self.recovery_store_in(root)?;
        let guard = lock_candidate(&store, &id, candidate.window).map_err(|e| e.to_string())?;
        let bytes = payload_bytes(&store, &id, candidate.window)?;
        if ContentHash::compute(&bytes) != candidate.hash {
            return Err("표시한 복구본이 바뀌었습니다".into());
        }
        let payload = RecoveryPayload::decode(&bytes).map_err(|e| e.to_string())?;
        Ok((id, store, guard, payload))
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
    fn prepare_recovery_in(
        &self,
        root: &std::path::Path,
        candidate: &RecoveryCandidate,
    ) -> Result<PreparedRecovery, String> {
        let (project, _, lease, payload) = self.read_recovery(root, candidate)?;
        let base = payload
            .base_manifest_hash
            .ok_or("저장된 기준 세대가 없습니다")?;
        if read_current_pointer(project.canonical_target()).ok() != Some(base) {
            return Err("디스크 세대가 바뀌어 복구본을 이 창에 복원할 수 없습니다".into());
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

    /// 표시한 것과 같은 bytes인 복구본만 폐기합니다. 문서는 조립하지 않습니다. 삭제가 실패해도 폐기 표식이
    /// 남아 다음 시작에서 제안하지 않습니다.
    fn discard_recovery_in(
        &self,
        root: &std::path::Path,
        candidate: &RecoveryCandidate,
    ) -> Result<(), String> {
        let (id, store, _lease, _) = self.read_recovery(root, candidate)?;
        discard_snapshot(&store, &id, candidate.window).map_err(|e| e.to_string())?;
        remove_lock(&store, &id, candidate.window);
        Ok(())
    }
}
impl AppRuntime {
    /// 복구본 저장소 root. [`AppRuntime::enable_recovery`]로 켰거나 시험이 바꾼 위치이고, 없으면 OS 사용자
    /// 데이터의 기본 위치입니다(셸 FFI의 [`ProjectSummary::recovery_candidate`]와 같은 곳).
    fn recovery_store_root(&self) -> Result<PathBuf, String> {
        match &self.recovery_root {
            Some(root) => Ok(root.clone()),
            None => ProjectSummary::recovery_root(),
        }
    }

    /// 이 창 session의 프로젝트에 제안할 최신 복구본의 probe 결과(XIII.8.3). 없으면 `None`입니다.
    ///
    /// 파일을 읽으므로 UI thread 밖에서 부릅니다. 셸은 결과를 사용자에게 보이고 Restore·Discard 선택을
    /// [`AppRuntime::recover_checkpoint`]로 넘깁니다. Later는 아무것도 부르지 않으며 복구본은 그대로 남습니다.
    pub fn recovery_probe(
        &self,
        session: WindowSessionHandle,
    ) -> Result<Option<RecoveryProbe>, String> {
        let root = self.recovery_store_root()?;
        let state = self.state();
        state
            .window(session)
            .and_then(|w| state.project(w.project))
            .ok_or("열린 창이 없습니다")?
            .recovery_probe_in(&root)
    }

    /// 셸은 request만 즉시 접수합니다. 파일 읽기와 검증은 worker가 수행합니다. 두 동작 모두
    /// [`RuntimeEvent::WindowViewChanged`] 또는 [`RuntimeEvent::RequestFailed`]로 끝납니다.
    pub fn recover_checkpoint(
        &self,
        session: WindowSessionHandle,
        candidate: RecoveryCandidate,
        action: RecoveryAction,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        let state = self.state();
        let shared = self.shared.clone();
        let sender = self.sender.clone();
        let root = self.recovery_store_root();
        let launch = std::thread::Builder::new()
            .name("stellar-recover-document".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> Result<(), String> {
                    let root = root?;
                    let p = state
                        .window(session)
                        .and_then(|w| state.project(w.project))
                        .ok_or("열린 창이 없습니다")?;
                    if action == RecoveryAction::Discard {
                        return p.discard_recovery_in(&root, &candidate);
                    }
                    let ready = p.prepare_recovery_in(&root, &candidate)?;
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
            "창 또는 디스크 세대가 바뀌어 복구본을 이 창에 복원할 수 없습니다",
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
    w.replace_history(history);
    w.document.as_mut().expect("checked document").state =
        DocumentState::with_revision(model, DocumentRevision(p.revision));
    entry
        .checkpoint
        .retain_previous(recovery.candidate.window, recovery.lease);
    Ok(())
}
