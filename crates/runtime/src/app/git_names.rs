//! 창에 표시한 저장 노드 이름을 시스템 Git index에 적용합니다.
//! 문서 스레드의 불변 session snapshot에서 대상 경로를 얻고, 사용자 신뢰와 저장소 위치는 Git worker가 확인합니다.
use std::collections::BTreeMap;
use std::fs::DirBuilder;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use stellar_core_types::ids::StableId;
use stellar_git_porcelain::cancel::GitCancelToken;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, SnapshotEntry, SnapshotSource,
};
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_ops::candidate::{
    diff, saved_semantic_snapshot, ObjectKind, SemanticChange, SemanticDiff, SemanticSnapshot,
    SemanticValue, StageSelectionRequest, DISPLAY_NAME,
};
use stellar_project::identity::{parse_project_id, ProjectIdentity};
use stellar_project::probe::safe_probe;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::trust::TrustStatus;
use stellar_project::trust_store::{os_user_data_base, FileTrustStore};

use super::{
    AppRuntime, ProjectSummary, RequestError, RequestFailure, RequestId, RuntimeEvent,
    WindowSessionHandle,
};
use crate::staging::{
    apply_stage_candidate, prepare_stage_candidate_with_preview, prepare_unstage_candidate,
};
pub use stellar_ops::candidate::SelectionOperation;

/// 한 저장 노드에서 HEAD, index, working 이름이 어떻게 다른지 보여 줍니다.
#[derive(Clone, Debug)]
pub struct GitNameRow {
    /// 이 노드의 문서.
    pub document: StableId,
    /// 이름 속성을 소유하는 노드.
    pub node: StableId,
    /// HEAD의 이름.
    pub head: String,
    /// 선택 Stage 전의 index 이름. 전체를 표시할 수 있는 이름만 허용합니다.
    pub index: String,
    /// 저장된 working 이름.
    pub working: String,
    /// HEAD에서 index로 가는 이름 변경이 있는지.
    pub staged: bool,
    staged_preview: Option<SemanticDiff>,
    unstaged_preview: Option<SemanticDiff>,
}

#[derive(Default, Debug)]
pub(super) struct WindowGit {
    snapshots: BTreeMap<WindowSessionHandle, (RequestId, Vec<GitNameRow>)>,
    active: BTreeMap<WindowSessionHandle, (RequestId, GitCancelToken)>,
}

impl WindowGit {
    pub(super) fn close(&mut self, session: WindowSessionHandle) {
        if let Some((_, token)) = self.active.get(&session) {
            token.cancel();
        }
        self.snapshots.remove(&session);
    }

    pub(super) fn close_project_windows(&mut self, project: &super::ProjectSession) {
        for window in &project.windows {
            self.close(*window);
        }
    }

    pub(super) fn cancel_all(&mut self) {
        for (_, token) in self.active.values() {
            token.cancel();
        }
        self.active.clear();
        self.snapshots.clear();
    }
}

fn trust(project: &ProjectSummary) -> Result<(), String> {
    let root = &project.canonical_root;
    if project.root.canonicalize().ok().as_deref() != Some(root) {
        return Err("열린 프로젝트 폴더가 바뀌었습니다".into());
    }
    let identity = ProjectIdentity::new(project.project_id, root).map_err(|e| e.to_string())?;
    let dir = os_user_data_base()
        .filter(|p| p.is_absolute())
        .ok_or("사용자 데이터 경로 없음")?;
    let store = FileTrustStore::open(&dir.join(stellar_ops::lock::APP_DATA_DIR))
        .map_err(|e| e.to_string())?;
    if store.status(&identity) != TrustStatus::Trusted {
        return Err("프로젝트 신뢰가 필요합니다".into());
    }
    let probe = safe_probe(root, TrustStatus::NotTrusted).map_err(|e| e.to_string())?;
    if parse_project_id(&probe.manifest.project.id) != Some(project.project_id) {
        return Err("열린 프로젝트 ID가 바뀌었습니다".into());
    }
    Ok(())
}

struct TempParent(PathBuf);
impl TempParent {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        loop {
            let dir = std::env::temp_dir().join(format!(
                "stellar-git-names-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for TempParent {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn live(cancel: &GitCancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("Git 작업을 취소했습니다".into())
    } else {
        Ok(())
    }
}

fn snapshot(
    git: &GatedGitRunner,
    project: &ProjectSummary,
    entries: &[SnapshotEntry],
    parent: &Path,
) -> Result<SemanticSnapshot, String> {
    let files = materialize_snapshot(git, &project.canonical_root, entries, parent)
        .map_err(|e| e.to_string())?;
    let saved = SavedProjectLoader::for_materialized_snapshot()
        .open(files.root())
        .map_err(|e| e.to_string())?;
    if saved.project_id != project.project_id {
        return Err("Git 상태의 프로젝트 ID가 다릅니다".into());
    }
    saved_semantic_snapshot(&saved).map_err(|e| e.to_string())
}

fn read_names(
    git: &GatedGitRunner,
    project: &ProjectSummary,
    parent: &Path,
    cancel: &GitCancelToken,
) -> Result<(Vec<GitNameRow>, bool), String> {
    let root = &project.canonical_root;
    live(cancel)?;
    let location =
        stellar_git_porcelain::location::locate_repository(git, root).map_err(|e| e.to_string())?;
    if location.worktree_root != *root {
        return Err("Stellar 프로젝트 폴더와 Git 작업 트리 root가 다릅니다".into());
    }
    let head = read_tree_snapshot(git, root, &SnapshotSource::Head).map_err(|e| e.to_string())?;
    let index = read_index_snapshot(git, root).map_err(|e| e.to_string())?;
    if index.has_conflicts() {
        return Err("index 충돌을 먼저 해결해야 합니다".into());
    }
    let indexed = snapshot(git, project, &index.entries, parent)?;
    live(cancel)?;
    let working_project = SavedProjectLoader::new()
        .open(root)
        .map_err(|e| e.to_string())?;
    if working_project.project_id != project.project_id {
        return Err("작업 프로젝트 ID가 다릅니다".into());
    }
    let working = saved_semantic_snapshot(&working_project).map_err(|e| e.to_string())?;
    let staged = if head.entries == index.entries {
        SemanticDiff::default()
    } else {
        let committed = snapshot(git, project, &head.entries, parent)?;
        diff(&committed, &indexed)
    };
    let unstaged = diff(&indexed, &working);
    let mut rows = BTreeMap::new();
    let mut other = false;
    for (in_index, changes) in [(true, staged), (false, unstaged)] {
        for change in changes.into_changes() {
            let SemanticChange::SetProperty(property) = &change else {
                other = true;
                continue;
            };
            let node = property.path.root();
            let name = property.path.keys() == [DISPLAY_NAME]
                && indexed
                    .get(node)
                    .is_some_and(|object| matches!(object.kind(), ObjectKind::Node(_)));
            let (Some(SemanticValue::Text(old)), Some(SemanticValue::Text(new)), Some(document)) =
                (&property.old, &property.new, change.document())
            else {
                other = true;
                continue;
            };
            if !name {
                other = true;
                continue;
            }
            if old.len() > super::EVENT_TEXT_MAX || new.len() > super::EVENT_TEXT_MAX {
                return Err("이름 전체를 표시할 수 없어 Stage 목록을 열지 않았습니다".into());
            }
            if !rows.contains_key(&node) && rows.len() == 64 {
                return Err(
                    "이름 변경이 64개를 넘습니다. Git 패널의 이름 Stage 지원 범위를 초과합니다"
                        .into(),
                );
            }
            let row = rows.entry(node).or_insert_with(|| GitNameRow {
                document,
                node,
                head: old.clone(),
                index: new.clone(),
                working: new.clone(),
                staged: false,
                staged_preview: None,
                unstaged_preview: None,
            });
            if in_index {
                row.staged = true;
                row.staged_preview = Some(SemanticDiff::new(vec![change]));
            } else {
                row.index = old.clone();
                row.working = new.clone();
                row.unstaged_preview = Some(SemanticDiff::new(vec![change]));
            }
        }
    }
    live(cancel)?;
    Ok((rows.into_values().collect(), other))
}

impl AppRuntime {
    /// 표시 중인 행 하나를 가져옵니다. 캐시만 읽으며 Git 실행이나 파일 I/O가 없습니다.
    pub fn git_name_row(
        &self,
        session: WindowSessionHandle,
        generation: RequestId,
        row: usize,
    ) -> Option<GitNameRow> {
        let git = self
            .shared
            .git_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (current, rows) = git.snapshots.get(&session)?;
        (*current == generation)
            .then(|| rows.get(row))
            .flatten()
            .cloned()
    }

    /// 백그라운드 Git 요청을 취소합니다. 다른 창이나 새 작업은 건드리지 않습니다.
    pub fn cancel_git_names(&self, session: WindowSessionHandle, request: RequestId) {
        let git = self
            .shared
            .git_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((active, token)) = git.active.get(&session) {
            if *active == request {
                token.cancel()
            }
        }
    }

    /// 저장된 노드 이름 변경을 조회하거나, 표시 세대의 행 하나를 실제 index에 적용합니다.
    /// 실행 worker는 창마다 최대 하나, runtime 전체에 최대 둘입니다.
    pub fn request_git_names(
        &self,
        session: WindowSessionHandle,
        selection: Option<(RequestId, usize)>,
    ) -> Result<RequestId, RequestError> {
        self.request_git_name_selection(
            session,
            selection.map(|(generation, row)| (generation, row, SelectionOperation::Stage)),
        )
    }

    /// 표시한 이름의 Stage 또는 Unstage를 실제 Git index에 적용합니다.
    pub fn request_git_name_selection(
        &self,
        session: WindowSessionHandle,
        selection: Option<(RequestId, usize, SelectionOperation)>,
    ) -> Result<RequestId, RequestError> {
        let state = self.state();
        let project = state
            .window(session)
            .and_then(|w| state.project(w.project))
            .cloned();
        let mut jobs = self
            .shared
            .git_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if jobs.active.len() >= 2 || jobs.active.contains_key(&session) {
            return Err(RequestError::QueueFull);
        }
        let request = self.reserve_request()?;
        let token = GitCancelToken::new();
        let preview = selection.and_then(|(generation, row, operation)| {
            let (actual, rows) = jobs.snapshots.get(&session)?;
            if *actual != generation {
                return None;
            }
            let entry = rows.get(row)?;
            match operation {
                SelectionOperation::Stage => entry.unstaged_preview.clone(),
                SelectionOperation::Unstage => entry.staged_preview.clone(),
            }
        });
        jobs.active.insert(session, (request, token.clone()));
        drop(jobs);
        let shared = Arc::clone(&self.shared);
        let worker = std::thread::Builder::new()
            .name("stellar-git-names".into())
            .spawn(move || {
                let mut published = false;
                let mut run = || -> Result<_, String> {
                    let project = project.as_ref().ok_or("이미 닫힌 창입니다")?;
                    live(&token)?;
                    if project.trust != TrustStatus::Trusted { return Err("프로젝트 신뢰가 필요합니다".into()); }
                    trust(project)?;
                    let git = discover_git(&GitDiscoveryConfig::default())
                        .ok_or("시스템 Git을 찾지 못했습니다")?;
                    let git = GatedGitRunner::new(
                        GitRunner::new(git, vec![], Duration::from_secs(15)),
                        GitTrust::Trusted,
                    );
                    let parent = TempParent::new()?;
                    if let Some((_, _, operation)) = selection {
                        let preview = preview.as_ref().ok_or("표시 세대나 선택 행이 바뀌었습니다")?;
                        let ids = preview.changes().iter().map(SemanticChange::id);
                        let prepared = match operation {
                            SelectionOperation::Stage => prepare_stage_candidate_with_preview(
                                &git, &project.canonical_root, &parent.0,
                                &StageSelectionRequest::stage(ids), Some(preview),
                            ),
                            SelectionOperation::Unstage => prepare_unstage_candidate(
                                &git, &project.canonical_root, &parent.0,
                                &StageSelectionRequest::unstage(ids), Some(preview),
                            ),
                        }.map_err(|e| e.to_string())?;
                        live(&token)?;
                        trust(project)?;
                        apply_stage_candidate(
                            &git,
                            &project.canonical_root,
                            &parent.0,
                            &prepared,
                            &token,
                        )
                        .map_err(|e| e.to_string())?;
                        published = true;
                    }
                    read_names(&git, project, &parent.0, &token)
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut run))
                    .unwrap_or_else(|_| Err("Git worker가 중단됐습니다".into()));
                let mut jobs = shared
                    .git_names
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let active = jobs
                    .active
                    .get(&session)
                    .is_some_and(|(id, _)| *id == request);
                if active {
                    jobs.active.remove(&session);
                }
                let result = result.and_then(|rows| {
                    if active && !token.is_cancelled() {
                        Ok(rows)
                    } else {
                        Err("창이 닫혔거나 작업이 취소됐습니다".into())
                    }
                });
                let event = match result {
                    Ok((items, other_changes)) => {
                        let rows = items.len() as u32;
                        jobs.snapshots.insert(session, (request, items));
                        RuntimeEvent::GitNames {
                            request,
                            session,
                            rows,
                            other_changes,
                            staged: selection.is_some(),
                        }
                    }
                    Err(message) => {
                        if active {
                            jobs.snapshots.remove(&session);
                        }
                        RuntimeEvent::RequestFailed {
                            request,
                            failure: RequestFailure::GitNamesUnavailable,
                            message: if published {format!("선택한 이름의 index 변경은 완료됐습니다. 목록을 다시 읽지 못했습니다. 새로고침하세요. {message}")} else {message},
                        }
                    }
                };
                drop(jobs);
                shared.outbox().push_back(event);
            });
        if worker.is_err() {
            let mut jobs = self
                .shared
                .git_names
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            jobs.active.remove(&session);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}
