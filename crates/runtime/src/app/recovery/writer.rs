//! 창마다 최신 미기록 상태 하나만 보관하는 순차 writer. 저장 뒤의 정리도 같은 순서대로 실행합니다.
use super::*;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug)]
enum Job {
    Write(Capture),
    Clear,
}
#[derive(Debug, Default)]
struct Queue {
    pending: Option<Job>,
    closed: bool,
    error: Option<String>,
}
type Shared = Arc<(Mutex<Queue>, Condvar)>;
#[derive(Debug, Default)]
pub(in crate::app) struct WindowCheckpoint {
    root: Option<PathBuf>,
    queue: Option<Shared>,
    seen: Option<(u64, bool)>,
    previous: Option<(WindowSessionId, fs::File)>,
}
impl WindowCheckpoint {
    pub(in crate::app) fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            queue: None,
            seen: None,
            previous: None,
        }
    }
    pub(super) fn retain_previous(&mut self, old: WindowSessionId, lease: fs::File) {
        self.previous = Some((old, lease));
    }
    /// 저수준 close API의 명시적인 폐기는 다음 열기에 다시 제안하지 않습니다.
    pub(in crate::app) fn discard(&mut self) {
        if let Some(queue) = &self.queue {
            let mut pending = queue.0.lock().unwrap_or_else(|e| e.into_inner());
            pending.pending = Some(Job::Clear);
            queue.1.notify_one();
        }
    }
    pub(in crate::app) fn observe(&mut self, window: &ProjectWindowSession) {
        let Some(root) = self.root.as_ref() else {
            return;
        };
        let dirty = window.has_unsaved_changes();
        let key = (window.revision().0, dirty);
        if self.seen == Some(key) {
            let failed = self.queue.as_ref().is_some_and(|queue| {
                queue
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .error
                    .take()
                    .is_some()
            });
            if !failed {
                return;
            }
        }
        if !dirty && self.queue.is_none() {
            self.seen = Some(key);
            return;
        }
        let job = if dirty {
            match Capture::from_window(window) {
                Ok(c) => Job::Write(c),
                Err(e) => {
                    eprintln!("Stellar recovery capture: {e}");
                    return;
                }
            }
        } else {
            Job::Clear
        };
        if self.queue.is_none() {
            let queue: Shared = Arc::default();
            let worker = queue.clone();
            let id = window.identity().clone();
            let store = RecoverySnapshotStore::new(root);
            let owner = window.window();
            let previous = self.previous.take();
            match std::thread::Builder::new()
                .name("stellar-recovery".into())
                .spawn(move || run(worker, store, id, owner, previous))
            {
                Ok(_) => self.queue = Some(queue),
                Err(e) => {
                    eprintln!("Stellar recovery worker: {e}");
                    return;
                }
            }
        }
        let queue = self.queue.as_ref().unwrap();
        let mut pending = queue.0.lock().unwrap_or_else(|e| e.into_inner());
        pending.pending = Some(job);
        queue.1.notify_one();
        self.seen = Some(key);
    }
}
impl Drop for WindowCheckpoint {
    fn drop(&mut self) {
        if let Some(q) = &self.queue {
            q.0.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            q.1.notify_one();
        }
    }
}
fn run(
    queue: Shared,
    store: RecoverySnapshotStore,
    id: ProjectIdentity,
    owner: WindowSessionId,
    mut previous: Option<(WindowSessionId, fs::File)>,
) {
    let mut lease = None;
    loop {
        let job = {
            let state = queue.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut state = queue
                .1
                .wait_while(state, |s| s.pending.is_none() && !s.closed)
                .unwrap_or_else(|e| e.into_inner());
            let Some(job) = state.pending.take() else {
                break;
            };
            job
        };
        let result = (|| -> io::Result<()> {
            if lease.is_none() {
                let dir = store.project_dir(&id);
                fs::create_dir_all(&dir)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                }
                lease = Some(lock_candidate(&store, &id, owner)?);
            }
            match job {
                Job::Write(c) => c.write(&store, &id, owner)?,
                Job::Clear => store
                    .remove_snapshot(&id, owner)
                    .map_err(io::Error::other)?,
            }
            if let Some((old, _)) = &previous {
                store.remove_snapshot(&id, *old).map_err(io::Error::other)?;
                previous = None;
            }
            Ok(())
        })();
        let error = result.err().map(|e| e.to_string());
        if let Some(e) = &error {
            eprintln!("Stellar recovery checkpoint: {e}");
        }
        queue.0.lock().unwrap_or_else(|e| e.into_inner()).error = error;
    }
}
