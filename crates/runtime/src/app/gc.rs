//! 앱 runtime의 유휴 chunk cache GC 일정 (V.10.7 후속, V.7.4 scheduler 연결, ADR-0138 결정 2).
//!
//! 열린 프로젝트의 로컬 chunk cache(`.stellar/cache/chunks/`, ADR-0122)를 사용자가 쉬는 동안 정리합니다. 정리 함수는
//! CLI `stellar cache gc`와 같은 [`stellar_project::cache::collect_garbage`]이고, 시작 조건은 저장소의 V.7.4
//! scheduler([`GcScheduler`])입니다.
//!
//! - **언제**: 일정 thread(`stellar-idle-gc`)가 [`IdleGcConfig::poll`]마다 background lane에 판정 job 하나를 같은 병합
//!   key로 보냅니다. job은 document thread에서 프로젝트마다 관찰합니다: 그 프로젝트 창에 저장하지 않은 편집·진행 중인
//!   제스처가 있거나 자동 저장 쓰기가 진행 중이면(`AppShared::autosave_writing`) 활동으로, cache current pointer가
//!   바뀌었으면(이 앱이나 CLI·MCP가 저장함) 활동으로 기록합니다. 이미지 가져오기 중(`AppShared::importing_image`)도
//!   바쁜 것으로 봅니다. 마지막 활동 뒤 [`IdleGcConfig::idle`]이 지나고 지금
//!   바쁘지 않으며 지난 정리 뒤 current가 바뀌었을 때만 정리합니다. 프로젝트를 연 직후는 활동으로 보므로 열자마자
//!   정리하지 않습니다(V.7.4).
//! - **저장 잠금과 겹치지 않음**: 정리는 판정 job 안(document thread)에서 합니다. 이 프로세스의 저장(Save Now, 자동
//!   저장 캡처, stale 해결, 에이전트 편집)은 모두 document thread job에서 시작하고 자동 저장 writer는 캡처 job이
//!   쓰기 표시를 세운 뒤에만 쓰므로, 판정 job이 쓰기 표시가 내려간 것을 본 동안에는 같은 프로세스의 저장이 시작되지
//!   않습니다. 그래서 정리가 교차 프로세스 저장 잠금을 쥐어도 이 앱의 저장이 `Busy`로 실패하지 않습니다. 다른
//!   프로세스(CLI·MCP)가 잠금을 쥐고 있으면 기다리지 않고 건너뛰고 다음 관찰에서 다시 시도합니다.
//! - **무엇을**: 보존 정책(`config/storage-gc.yaml`)으로 current·최근 정상 세대·stamp 짝을 남기고, 남긴 세대가 참조하지
//!   않으며 유예 시간이 지난 cache 파일만 지웁니다. 한 번이라도 정리했으면 사용자 임시 폴더의 snapshot 보관소
//!   ([`SnapshotStore::prune`])도 같은 유예 시간으로 정리합니다.
//!
//! 일정은 [`AppRuntime::start_idle_gc`]로 켜고, 돌려받은 [`IdleGc`]를 drop하면 멈춥니다(네이티브 앱 runtime이 자동
//! 저장과 함께 켭니다). 켜지 않은 runtime(시험, Debug 검사 실행)은 정리하지 않습니다.

use super::*;
use std::sync::Condvar;
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

use stellar_ops::lock::{default_lock_dir, LockError, SaveLock};
use stellar_project::cache::{collect_garbage, default_retention, GcReport, SnapshotStore};
use stellar_storage::gc::retention::RetentionPolicy;
use stellar_storage::gc::scheduler::GcScheduler;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::hash::ContentHash;

use crate::command_queue::{CoalesceKey, SubmitOptions};

/// 마지막 활동 뒤 정리를 시작하기까지의 기본 시간.
pub const IDLE_THRESHOLD: Duration = Duration::from_secs(60);

/// 기본 관찰 간격.
pub const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// 남기는 최근 정리 기록 수.
pub const RUN_LIMIT: usize = 32;

/// background lane에서 판정 job을 합치는 key.
const COALESCE_KEY: CoalesceKey = CoalesceKey::Custom(0x4944_4C45_5F47_4300);

/// 일정 thread 이름.
const THREAD_NAME: &str = "stellar-idle-gc";

/// 잠금 폴더 공급자.
pub type LockDirSource = Arc<dyn Fn() -> Result<PathBuf, LockError> + Send + Sync>;

/// 유휴 GC 설정.
#[derive(Clone)]
pub struct IdleGcConfig {
    /// 마지막 활동 뒤 기다리는 시간.
    pub idle: Duration,
    /// 관찰 간격.
    pub poll: Duration,
    /// 보존 정책.
    pub retention: RetentionPolicy,
    /// 교차 프로세스 저장 잠금 폴더(저장과 같은 폴더).
    pub lock_dir: LockDirSource,
    /// 함께 정리할 snapshot 보관소(`None`이면 정리하지 않음).
    pub store: Option<SnapshotStore>,
}

impl fmt::Debug for IdleGcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdleGcConfig")
            .field("idle", &self.idle)
            .field("poll", &self.poll)
            .field("retention", &self.retention)
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl Default for IdleGcConfig {
    /// 제품 기본값: [`IDLE_THRESHOLD`], [`POLL_INTERVAL`], `config/storage-gc.yaml`, 기본 잠금 폴더, 사용자 snapshot
    /// 보관소.
    fn default() -> IdleGcConfig {
        IdleGcConfig {
            idle: IDLE_THRESHOLD,
            poll: POLL_INTERVAL,
            retention: default_retention(),
            lock_dir: Arc::new(default_lock_dir),
            store: SnapshotStore::user(),
        }
    }
}

/// 정리 시도 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleGcOutcome {
    /// 정리했습니다.
    Collected(GcReport),
    /// 다른 프로세스가 저장 잠금을 쥐고 있어 건너뛰었습니다(다음 관찰에서 다시).
    LockBusy,
    /// 잠금을 준비하거나 정리하지 못했습니다(다음 저장 뒤 다시).
    Failed(String),
}

/// 정리 기록 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleGcRun {
    /// 프로젝트 session.
    pub project: ProjectSessionId,
    /// 결과.
    pub outcome: IdleGcOutcome,
}

/// 프로젝트 하나의 일정.
#[derive(Debug)]
struct Tracker {
    scheduler: GcScheduler,
    /// 마지막으로 본 current manifest.
    current: Option<ContentHash>,
    /// 마지막으로 정리했을 때의 current(`None`이면 아직 정리하지 않음).
    collected_for: Option<Option<ContentHash>>,
}

#[derive(Debug, Default)]
struct Schedule {
    projects: BTreeMap<ProjectSessionId, Tracker>,
    runs: VecDeque<IdleGcRun>,
}

impl Schedule {
    fn record(&mut self, run: IdleGcRun) {
        if self.runs.len() == RUN_LIMIT {
            self.runs.pop_front();
        }
        self.runs.push_back(run);
    }
}

#[derive(Debug, Default)]
struct Stop {
    stopped: Mutex<bool>,
    changed: Condvar,
}

/// 켠 유휴 GC 일정. drop하면 일정 thread를 멈춥니다.
#[derive(Debug)]
pub struct IdleGc {
    stop: Arc<Stop>,
    schedule: Arc<Mutex<Schedule>>,
    thread: Option<JoinHandle<()>>,
}

impl IdleGc {
    /// 최근 정리 기록(오래된 것부터, 최대 [`RUN_LIMIT`]개).
    pub fn runs(&self) -> Vec<IdleGcRun> {
        lock(&self.schedule).runs.iter().cloned().collect()
    }
}

impl Drop for IdleGc {
    fn drop(&mut self) {
        *self
            .stop
            .stopped
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self.stop.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 프로젝트 `project`가 지금 바쁜지(저장하지 않은 편집·제스처·진행 중인 자동 저장 쓰기).
fn busy(sessions: &AppSessions, project: ProjectSessionId, writing: bool) -> bool {
    writing
        || sessions
            .windows
            .values()
            .filter(|entry| entry.project == project)
            .any(|entry| {
                entry.session.has_unsaved_changes()
                    || entry.session.history().history().preview_active()
            })
}

/// 판정 job 본체(document thread, 모듈 문서). `now`는 단조 시각입니다.
fn observe(
    ctx: &DocumentContext,
    shared: &AppShared,
    config: &IdleGcConfig,
    schedule: &mut Schedule,
    now: Instant,
) {
    let sessions = ctx.app_sessions();
    schedule
        .projects
        .retain(|id, _| sessions.projects.contains_key(id));
    // 쓰기 표시는 document thread에서만 세우므로(자동 저장 캡처) 이 job 동안 바뀌지 않습니다. 이미지 가져오기는 worker가
    // 프로젝트 asset을 쓰는 동안이라 cache와 겹치지 않지만, 같은 프로젝트 폴더 쓰기이므로 바쁜 것으로 봅니다.
    let writing = shared.autosave_writing.load(Ordering::SeqCst)
        || shared.importing_image.load(Ordering::SeqCst);
    let mut collected = false;
    let mut runs = Vec::new();
    for (id, project) in &sessions.projects {
        let root = project.identity.root_path();
        let current = read_current_pointer(root).ok();
        let tracker = schedule.projects.entry(*id).or_insert_with(|| Tracker {
            scheduler: GcScheduler::new(config.idle, now),
            current,
            collected_for: None,
        });
        let busy = busy(sessions, *id, writing);
        if busy || tracker.current != current {
            tracker.scheduler.note_activity(now);
            tracker.current = current;
        }
        if busy || tracker.collected_for == Some(current) {
            continue;
        }
        let Some(_guard) = tracker.scheduler.try_begin_gc(now) else {
            continue;
        };
        let outcome = match (config.lock_dir)()
            .and_then(|dir| SaveLock::for_identity(&dir, &project.identity))
        {
            Ok(_lock) => match collect_garbage(root, &config.retention, SystemTime::now()) {
                Ok(report) => {
                    tracker.collected_for = Some(current);
                    collected = true;
                    IdleGcOutcome::Collected(report)
                }
                Err(error) => {
                    tracker.collected_for = Some(current);
                    IdleGcOutcome::Failed(error.to_string())
                }
            },
            Err(LockError::Busy { .. }) => IdleGcOutcome::LockBusy,
            Err(error) => {
                tracker.collected_for = Some(current);
                IdleGcOutcome::Failed(error.to_string())
            }
        };
        runs.push(IdleGcRun {
            project: *id,
            outcome,
        });
    }
    for run in runs {
        schedule.record(run);
    }
    if collected {
        if let Some(store) = &config.store {
            store.prune(SystemTime::now(), config.retention.grace_period_seconds);
        }
    }
}

fn run_loop(
    stop: Arc<Stop>,
    sender: DocumentSender,
    shared: Arc<AppShared>,
    config: Arc<IdleGcConfig>,
    schedule: Arc<Mutex<Schedule>>,
) {
    loop {
        {
            let stopped = lock(&stop.stopped);
            let (stopped, _) = stop
                .changed
                .wait_timeout_while(stopped, config.poll, |stopped| !*stopped)
                .unwrap_or_else(PoisonError::into_inner);
            if *stopped {
                return;
            }
        }
        let job_shared = Arc::clone(&shared);
        let job_config = Arc::clone(&config);
        let job_schedule = Arc::clone(&schedule);
        let options = SubmitOptions::background().with_coalesce(COALESCE_KEY);
        match sender.try_submit(options, move |ctx: &mut DocumentContext| {
            observe(
                ctx,
                &job_shared,
                &job_config,
                &mut lock(&job_schedule),
                Instant::now(),
            )
        }) {
            Ok(_) | Err(SubmitError::QueueFull { .. }) => {}
            Err(SubmitError::Stopped) => return,
        }
    }
}

impl AppRuntime {
    /// 제품 기본값([`IdleGcConfig::default`])으로 유휴 cache GC 일정을 켭니다(모듈 문서).
    pub fn start_idle_gc(&self) -> IdleGc {
        self.start_idle_gc_with(IdleGcConfig::default())
    }

    /// 주어진 설정으로 유휴 cache GC 일정을 켭니다. thread를 만들지 못하면 정리하지 않는 handle을 돌려줍니다.
    pub fn start_idle_gc_with(&self, config: IdleGcConfig) -> IdleGc {
        let stop = Arc::new(Stop::default());
        let schedule = Arc::new(Mutex::new(Schedule::default()));
        let thread = {
            let stop = Arc::clone(&stop);
            let sender = self.sender.clone();
            let shared = Arc::clone(&self.shared);
            let config = Arc::new(config);
            let schedule = Arc::clone(&schedule);
            std::thread::Builder::new()
                .name(THREAD_NAME.into())
                .spawn(move || run_loop(stop, sender, shared, config, schedule))
        };
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                eprintln!("Stellar idle GC: {error}");
                None
            }
        };
        IdleGc {
            stop,
            schedule,
            thread,
        }
    }
}

#[cfg(test)]
mod tests;
