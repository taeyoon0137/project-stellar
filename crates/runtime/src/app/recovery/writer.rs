//! 창마다 최신 미저장 상태 하나만 기록하는 순차 복구 worker.
//!
//! document thread는 상태를 게시할 때 revision·dirty 표식만 남깁니다(O(1)). worker는 마지막 표식 뒤
//! [`DEBOUNCE`]를 기다렸다가(편집이 이어져도 첫 표식 뒤 [`MAX_DELAY`] 안에) 문서 캡처 job을 background
//! lane에 한 번 제출합니다. 직렬화와 원자적 파일 교체, 저장 뒤 정리는 같은 worker가 순서대로 합니다.
use super::*;
use crate::command_queue::SubmitOptions;
use crate::document_thread::DocumentSender;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Instant;

/// 마지막 편집 표식 뒤 캡처까지 기다리는 시간. 임시 correctness 값이며 XIII.8.1 벤치마크로 확정합니다.
pub(in crate::app) const DEBOUNCE: Duration = Duration::from_millis(1000);
/// 편집이 계속 이어져도 첫 표식 뒤 이 시간 안에 한 번은 캡처합니다.
pub(in crate::app) const MAX_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// 문서가 dirty입니다. debounce 뒤 캡처합니다.
    Capture,
    /// 저장·폐기로 기록할 것이 없습니다. 바로 지웁니다.
    Clear,
}

#[derive(Debug, Default)]
struct Queue {
    pending: Option<Mark>,
    /// 처리하지 않은 첫 Capture 표식 시각(최대 지연 기준).
    first: Option<Instant>,
    /// 마지막 Capture 표식 시각(debounce 기준).
    last: Option<Instant>,
    closed: bool,
    failed: bool,
}

impl Queue {
    fn mark(&mut self, mark: Mark, now: Instant) {
        self.pending = Some(mark);
        match mark {
            Mark::Capture => {
                self.first.get_or_insert(now);
                self.last = Some(now);
            }
            Mark::Clear => {
                self.first = None;
                self.last = None;
            }
        }
    }

    fn due(&self) -> Option<Instant> {
        Some((self.last? + DEBOUNCE).min(self.first? + MAX_DELAY))
    }
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

fn lock(queue: &Shared) -> std::sync::MutexGuard<'_, Queue> {
    queue.0.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 살아 있는 복구 worker 수. runtime 종료가 남은 정리(폐기·저장 뒤 삭제)를 잠시 기다리는 데 씁니다.
#[derive(Debug, Clone, Default)]
pub(in crate::app) struct RecoveryWorkers(Arc<(Mutex<usize>, Condvar)>);

impl RecoveryWorkers {
    fn enter(&self) -> WorkerGuard {
        *self.0 .0.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        WorkerGuard(self.clone())
    }

    /// 모든 worker가 끝나거나 `timeout`이 지날 때까지 기다립니다.
    pub(in crate::app) fn wait_idle(&self, timeout: Duration) {
        let (count, idle) = &*self.0;
        let count = count.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = idle.wait_timeout_while(count, timeout, |n| *n > 0);
    }
}

struct WorkerGuard(RecoveryWorkers);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let (count, idle) = &*(self.0).0;
        let mut count = count.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_sub(1);
        idle.notify_all();
    }
}

/// 복구 체크포인트를 쓸 위치와 문서 캡처를 제출할 document thread.
#[derive(Debug, Clone)]
pub(in crate::app) struct CheckpointConfig {
    /// 앱 데이터 폴더(`RecoverySnapshotStore` 기준).
    pub(in crate::app) root: PathBuf,
    /// 캡처 job을 제출할 document thread.
    pub(in crate::app) sender: DocumentSender,
    /// runtime 종료가 기다릴 worker 수.
    pub(in crate::app) workers: RecoveryWorkers,
}

#[derive(Debug)]
pub(in crate::app) struct WindowCheckpoint {
    config: Option<CheckpointConfig>,
    session: WindowSessionHandle,
    queue: Option<Shared>,
    seen: Option<(u64, bool)>,
    previous: Option<(WindowSessionId, fs::File)>,
}

impl WindowCheckpoint {
    pub(in crate::app) fn new(
        config: Option<CheckpointConfig>,
        session: WindowSessionHandle,
    ) -> Self {
        Self {
            config,
            session,
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
            lock(queue).mark(Mark::Clear, Instant::now());
            queue.1.notify_one();
        }
    }

    /// 상태를 게시할 때마다 부릅니다. revision·dirty가 바뀐 경우에만 표식을 남기고 문서는 읽지 않습니다.
    pub(in crate::app) fn observe(&mut self, window: &ProjectWindowSession) {
        let Some(config) = &self.config else {
            return;
        };
        let dirty = window.has_unsaved_changes();
        let key = (window.revision().0, dirty);
        let retry = self
            .queue
            .as_ref()
            .is_some_and(|queue| std::mem::take(&mut lock(queue).failed));
        if self.seen == Some(key) && !retry {
            return;
        }
        self.seen = Some(key);
        if !dirty && self.queue.is_none() {
            // 한 번도 기록하지 않은 창은 지울 것도 없습니다.
            return;
        }
        if self.queue.is_none() {
            let queue: Shared = Arc::default();
            let worker = Worker {
                queue: queue.clone(),
                store: RecoverySnapshotStore::new(&config.root),
                identity: window.identity().clone(),
                owner: window.window(),
                session: self.session,
                sender: config.sender.clone(),
                previous: self.previous.take(),
                active: config.workers.enter(),
            };
            match std::thread::Builder::new()
                .name("stellar-recovery".into())
                .spawn(move || worker.run())
            {
                Ok(_) => self.queue = Some(queue),
                Err(e) => {
                    eprintln!("Stellar recovery worker: {e}");
                    return;
                }
            }
        }
        let queue = self.queue.as_ref().expect("worker queue");
        lock(queue).mark(
            if dirty { Mark::Capture } else { Mark::Clear },
            Instant::now(),
        );
        queue.1.notify_one();
    }
}

impl Drop for WindowCheckpoint {
    fn drop(&mut self) {
        if let Some(queue) = &self.queue {
            lock(queue).closed = true;
            queue.1.notify_one();
        }
    }
}

struct Worker {
    queue: Shared,
    store: RecoverySnapshotStore,
    identity: ProjectIdentity,
    owner: WindowSessionId,
    session: WindowSessionHandle,
    sender: DocumentSender,
    previous: Option<(WindowSessionId, fs::File)>,
    active: WorkerGuard,
}

impl Worker {
    /// 처리할 다음 표식. Capture는 debounce가 끝날 때까지 기다립니다. 닫힌 뒤에는 Clear만 처리합니다.
    fn next(&self) -> Option<Mark> {
        let mut q = lock(&self.queue);
        loop {
            match q.pending {
                Some(Mark::Clear) => break,
                // 창 session이 닫혔으면 document thread에서 더 캡처할 수 없습니다.
                None | Some(Mark::Capture) if q.closed => return None,
                Some(Mark::Capture) => {
                    let due = q.due().expect("Capture 표식에는 시각이 있습니다");
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    q = self
                        .queue
                        .1
                        .wait_timeout(q, due - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                None => q = self.queue.1.wait(q).unwrap_or_else(PoisonError::into_inner),
            }
        }
        q.first = None;
        q.last = None;
        q.pending.take()
    }

    /// 아직 새 표식이 없으면 캡처를 다시 예약합니다(background lane에서 밀려난 경우).
    fn retry_capture(&self) {
        let mut q = lock(&self.queue);
        if q.pending.is_none() && !q.closed {
            q.mark(Mark::Capture, Instant::now());
        }
    }

    /// document thread에서 현재 문서를 한 번 캡처합니다. 창이 없거나 이미 clean이면 `None`입니다.
    fn capture(&self) -> Result<Option<Capture>, ()> {
        let session = self.session;
        let ticket = self
            .sender
            .try_submit(SubmitOptions::background(), move |ctx| {
                let entry = ctx.app_sessions().windows.get(&session)?;
                entry
                    .session
                    .has_unsaved_changes()
                    .then(|| Capture::from_window(&entry.session))
            })
            .map_err(|_| ())?
            .ticket;
        match ticket.wait() {
            Ok(Some(Ok(capture))) => Ok(Some(capture)),
            Ok(Some(Err(e))) => {
                eprintln!("Stellar recovery capture: {e}");
                Err(())
            }
            Ok(None) => Ok(None),
            Err(_) => {
                self.retry_capture();
                Ok(None)
            }
        }
    }

    fn run(mut self) {
        let mut lease = None;
        while let Some(mark) = self.next() {
            let capture = match mark {
                Mark::Capture => match self.capture() {
                    Ok(Some(capture)) => Some(capture),
                    Ok(None) => continue,
                    Err(()) => {
                        lock(&self.queue).failed = true;
                        continue;
                    }
                },
                Mark::Clear => None,
            };
            let result = (|| -> io::Result<()> {
                if lease.is_none() {
                    let dir = self.store.project_dir(&self.identity);
                    fs::create_dir_all(&dir)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                    }
                    lease = Some(lock_candidate(&self.store, &self.identity, self.owner)?);
                }
                match capture {
                    Some(c) => c.write(&self.store, &self.identity, self.owner)?,
                    None => self
                        .store
                        .remove_snapshot(&self.identity, self.owner)
                        .map_err(io::Error::other)?,
                }
                if let Some((old, _)) = &self.previous {
                    self.store
                        .remove_snapshot(&self.identity, *old)
                        .map_err(io::Error::other)?;
                    remove_lock(&self.store, &self.identity, *old);
                    self.previous = None;
                }
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("Stellar recovery checkpoint: {e}");
                lock(&self.queue).failed = true;
            }
        }
        // 남긴 복구본이 없으면 이 창의 잠금 파일도 지웁니다. 잠금을 쥔 채 지워 다른 프로세스와 겹치지 않습니다.
        if lease.is_some()
            && matches!(
                self.store.read_snapshot(&self.identity, self.owner),
                Ok(None)
            )
        {
            remove_lock(&self.store, &self.identity, self.owner);
        }
        drop(lease);
        drop(self.active);
    }
}
