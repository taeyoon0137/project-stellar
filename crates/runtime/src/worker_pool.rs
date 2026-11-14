//! worker pool: 작업 class와 priority tag (XII.1.4).
//!
//! 아키텍처 §9.2의 Worker Pool(text shaping, tessellation, 이미지 디코딩, 저장 직렬화, 캐시 생성)을
//! 위한 공통 pool을 제공합니다. job마다 **class**(무슨 작업인지)와 **priority**(얼마나 급한지)를 붙이고,
//! pool은 priority로 실행 순서를 정합니다.
//! 실제 작업 경로의 연결은 후속 작업입니다. XI.6.3의 tessellation은 아직 별도 pool을 사용합니다.
//!
//! # class와 기본 priority
//!
//! | class | 대상 | 기본 priority |
//! |---|---|---|
//! | [`WorkerClass::TextShaping`] | 편집·표시 중인 text run shaping | [`WorkerPriority::Interactive`] |
//! | [`WorkerClass::Tessellation`] | 보이는 path의 fill·stroke mesh | [`WorkerPriority::Interactive`] |
//! | [`WorkerClass::ImageDecode`] | 보이는 이미지 decode | [`WorkerPriority::Interactive`] |
//! | [`WorkerClass::SaveSerialization`] | 자동 저장 snapshot 직렬화·chunk 인코딩 | [`WorkerPriority::Normal`] |
//! | [`WorkerClass::CacheGeneration`] | thumbnail 등 미리 계산하는 캐시 | [`WorkerPriority::Background`] |
//! | [`WorkerClass::Gc`] | generation·chunk·asset GC(V.7, XIII.5.6) | [`WorkerPriority::Background`] |
//!
//! priority는 class 기본값([`WorkerJobSpec::new`])을 쓰거나 job마다 바꿀 수 있습니다
//! ([`WorkerJobSpec::with_priority`], 예: 화면 밖 이미지를 미리 decode하면 `Background`).
//!
//! # 실행 순서
//!
//! 1. 쉬는 worker는 `Interactive` → `Normal` → `Background` lane 순으로 가장 오래된 job을 꺼냅니다.
//!    같은 lane 안은 접수 순서(FIFO)이고 lane 사이의 접수 순서는 보존하지 않습니다.
//! 2. `Interactive`가 아닌 job(`Normal`·`Background`)은 합쳐서 동시에 최대
//!    [`WorkerPoolConfig::non_interactive_limit`]개 worker에서만 실행합니다. worker가 둘 이상이면 기본값은
//!    worker 수 - 1이라 worker 하나는 항상 `Interactive` job 몫으로 남습니다. 오래 걸리는 저장 직렬화나
//!    GC가 몰려도(예: 최소 구성 worker 2개에서 창 두 개의 저장) interactive shaping·tessellation이 기다리지
//!    않습니다. worker가 하나면 남길 수 없어 상한은 1입니다.
//! 3. `Background` job은 그 안에서 다시 동시에 최대 [`WorkerPoolConfig::background_limit`]개 worker에서만
//!    실행합니다(기본값도 worker 수 - 1). 상한을 낮추면 GC가 저장 직렬화 자리까지 차지하지 않습니다.
//! 4. 실행 중인 job은 선점하지 않습니다. 오래 걸리는 job은 [`WorkerContext::is_cancelled`]를 확인해
//!    pool 종료 때 일찍 끝낼 수 있습니다.
//!
//! # 접수
//!
//! [`WorkerPool::try_submit`]·[`WorkerPool::try_submit_with`]는 기다리지 않습니다. lane마다 용량이
//! 있고([`WorkerPoolConfig::with_capacity`]), 용량이 차면 job을 돌려줍니다
//! ([`WorkerRejectReason::QueueFull`]). 어느 lane도 이미 접수된 job을 밀어내지 않습니다(저장 직렬화를
//! 조용히 잃지 않게 함). 호출자는 돌려받은 job을 나중에 다시 제출합니다.
//!
//! # 실행 환경
//!
//! worker thread 이름은 `stellar-worker-<n>`이고 시작 시 [`ThreadRole::Worker`]를 bind합니다. 따라서
//! worker 전용 작업(이미지 decode, XI.7.1)을 그대로 실행할 수 있습니다. job이 panic하면(dev·test
//! profile) worker가 잡아 집계하고 다음 job을 계속 실행합니다. release profile은 `panic = "abort"`라
//! process가 끝납니다(`docs/architecture/panic-policy.md`).
//!
//! # 종료
//!
//! [`WorkerPool::shutdown`]과 drop은 접수 닫기 → 아직 시작하지 않은 job 폐기(실행하지 않고 drop) →
//! 취소 표시 → 실행 중인 job 완료 대기 → join 순서입니다. 앱 전체의 thread 종료 순서는 XII.1.6
//! 소관입니다. 창 닫기 flush(XIII.1.6)처럼 끝까지 실행해야 하는 job은 종료 전에 결과를 기다립니다.
//! job 안에서 pool의 마지막 소유권을 놓으면 현재 worker는 자기 자신을 join하지 않고 job 반환 후
//! 종료합니다. 다른 worker는 같은 종료 순서를 따릅니다.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stellar_renderer_core::thread_role::{bind_thread_role, current_thread_role, ThreadRole};

/// worker thread 이름 접두어(`stellar-worker-<n>`).
pub const WORKER_THREAD_NAME_PREFIX: &str = "stellar-worker-";

// ---------------------------------------------------------------------------
// priority·class
// ---------------------------------------------------------------------------

/// worker job priority. 선언 순서가 실행 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkerPriority {
    /// 현재 frame이나 입력 응답이 기다리는 작업. 가장 먼저 실행합니다.
    Interactive,
    /// 곧 끝나야 하지만 frame이 기다리지 않는 작업(저장 직렬화).
    Normal,
    /// 쉬는 시간에 하는 작업(캐시 생성, GC). 동시 실행 수에 상한이 있습니다.
    Background,
}

impl WorkerPriority {
    /// 실행 순서대로 나열한 전체 priority.
    pub const ALL: [WorkerPriority; 3] = [
        WorkerPriority::Interactive,
        WorkerPriority::Normal,
        WorkerPriority::Background,
    ];

    /// 고정 이름(진단·fixture용).
    pub const fn name(self) -> &'static str {
        match self {
            WorkerPriority::Interactive => "interactive",
            WorkerPriority::Normal => "normal",
            WorkerPriority::Background => "background",
        }
    }

    /// 이름에서 복원합니다.
    pub fn from_name(name: &str) -> Option<WorkerPriority> {
        WorkerPriority::ALL
            .into_iter()
            .find(|priority| priority.name() == name)
    }

    const fn lane(self) -> usize {
        match self {
            WorkerPriority::Interactive => 0,
            WorkerPriority::Normal => 1,
            WorkerPriority::Background => 2,
        }
    }
}

/// worker job class(아키텍처 §9.2 Worker Pool 항목 + GC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkerClass {
    /// text shaping.
    TextShaping,
    /// path fill·stroke tessellation.
    Tessellation,
    /// 이미지 decode.
    ImageDecode,
    /// 저장 직렬화(snapshot → chunk payload).
    SaveSerialization,
    /// 캐시 생성(thumbnail 등).
    CacheGeneration,
    /// generation·chunk·asset GC.
    Gc,
}

impl WorkerClass {
    /// 전체 class.
    pub const ALL: [WorkerClass; 6] = [
        WorkerClass::TextShaping,
        WorkerClass::Tessellation,
        WorkerClass::ImageDecode,
        WorkerClass::SaveSerialization,
        WorkerClass::CacheGeneration,
        WorkerClass::Gc,
    ];

    /// 고정 이름(진단·fixture용).
    pub const fn name(self) -> &'static str {
        match self {
            WorkerClass::TextShaping => "text-shaping",
            WorkerClass::Tessellation => "tessellation",
            WorkerClass::ImageDecode => "image-decode",
            WorkerClass::SaveSerialization => "save-serialization",
            WorkerClass::CacheGeneration => "cache-generation",
            WorkerClass::Gc => "gc",
        }
    }

    /// 이름에서 복원합니다.
    pub fn from_name(name: &str) -> Option<WorkerClass> {
        WorkerClass::ALL
            .into_iter()
            .find(|class| class.name() == name)
    }

    /// class 기본 priority(모듈 문서 표).
    pub const fn default_priority(self) -> WorkerPriority {
        match self {
            WorkerClass::TextShaping | WorkerClass::Tessellation | WorkerClass::ImageDecode => {
                WorkerPriority::Interactive
            }
            WorkerClass::SaveSerialization => WorkerPriority::Normal,
            WorkerClass::CacheGeneration | WorkerClass::Gc => WorkerPriority::Background,
        }
    }

    const fn index(self) -> usize {
        match self {
            WorkerClass::TextShaping => 0,
            WorkerClass::Tessellation => 1,
            WorkerClass::ImageDecode => 2,
            WorkerClass::SaveSerialization => 3,
            WorkerClass::CacheGeneration => 4,
            WorkerClass::Gc => 5,
        }
    }
}

/// job tag: class와 priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkerJobSpec {
    /// 작업 class.
    pub class: WorkerClass,
    /// 실행 priority.
    pub priority: WorkerPriority,
}

impl WorkerJobSpec {
    /// class 기본 priority의 tag.
    pub const fn new(class: WorkerClass) -> WorkerJobSpec {
        WorkerJobSpec {
            class,
            priority: class.default_priority(),
        }
    }

    /// priority를 바꾼 tag.
    pub const fn with_priority(self, priority: WorkerPriority) -> WorkerJobSpec {
        WorkerJobSpec {
            class: self.class,
            priority,
        }
    }
}

impl From<WorkerClass> for WorkerJobSpec {
    fn from(class: WorkerClass) -> WorkerJobSpec {
        WorkerJobSpec::new(class)
    }
}

// ---------------------------------------------------------------------------
// 설정
// ---------------------------------------------------------------------------

/// worker 수 상한.
pub const MAX_WORKERS: usize = 64;

/// worker 수에 대한 기본 background 동시 실행 상한: worker가 둘 이상이면 worker 수 - 1, 하나면 1.
pub const fn default_background_limit(workers: usize) -> usize {
    if workers > 1 {
        workers - 1
    } else {
        1
    }
}

/// worker 수에 대한 기본 non-interactive(`Normal`·`Background` 합계) 동시 실행 상한: worker가 둘 이상이면
/// worker 수 - 1(worker 하나는 `Interactive` 전용), 하나면 1.
pub const fn default_non_interactive_limit(workers: usize) -> usize {
    default_background_limit(workers)
}

/// pool 설정 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPoolConfigError {
    /// worker 수가 1..=[`MAX_WORKERS`]가 아닙니다.
    InvalidWorkerCount {
        /// 요청 값.
        requested: usize,
    },
    /// background 상한이 1..=worker 수가 아닙니다.
    InvalidBackgroundLimit {
        /// 요청 값.
        requested: usize,
        /// worker 수.
        workers: usize,
    },
    /// non-interactive 상한이 1..=worker 수가 아닙니다.
    InvalidNonInteractiveLimit {
        /// 요청 값.
        requested: usize,
        /// worker 수.
        workers: usize,
    },
    /// lane 용량은 1 이상이어야 합니다.
    ZeroCapacity {
        /// 해당 lane.
        priority: WorkerPriority,
    },
}

impl fmt::Display for WorkerPoolConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerPoolConfigError::InvalidWorkerCount { requested } => {
                write!(f, "worker 수는 1..={MAX_WORKERS}이어야 합니다: {requested}")
            }
            WorkerPoolConfigError::InvalidBackgroundLimit { requested, workers } => write!(
                f,
                "background 동시 실행 상한은 1..={workers}이어야 합니다: {requested}"
            ),
            WorkerPoolConfigError::InvalidNonInteractiveLimit { requested, workers } => write!(
                f,
                "non-interactive 동시 실행 상한은 1..={workers}이어야 합니다: {requested}"
            ),
            WorkerPoolConfigError::ZeroCapacity { priority } => {
                write!(f, "{} lane 용량은 1 이상이어야 합니다", priority.name())
            }
        }
    }
}

impl std::error::Error for WorkerPoolConfigError {}

/// pool 설정: worker 수, non-interactive·background 동시 실행 상한, lane별 용량.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkerPoolConfig {
    workers: usize,
    non_interactive_limit: usize,
    background_limit: usize,
    capacity: [usize; 3],
}

impl WorkerPoolConfig {
    /// 기본 interactive lane 용량(초기값).
    pub const DEFAULT_INTERACTIVE_CAPACITY: usize = 1024;
    /// 기본 normal lane 용량(초기값).
    pub const DEFAULT_NORMAL_CAPACITY: usize = 256;
    /// 기본 background lane 용량(초기값).
    pub const DEFAULT_BACKGROUND_CAPACITY: usize = 256;

    /// `workers`개 worker와 기본 상한([`default_non_interactive_limit`], [`default_background_limit`])·기본
    /// 용량.
    pub fn new(workers: usize) -> Result<WorkerPoolConfig, WorkerPoolConfigError> {
        if workers == 0 || workers > MAX_WORKERS {
            return Err(WorkerPoolConfigError::InvalidWorkerCount { requested: workers });
        }
        Ok(WorkerPoolConfig {
            workers,
            non_interactive_limit: default_non_interactive_limit(workers),
            background_limit: default_background_limit(workers),
            capacity: [
                WorkerPoolConfig::DEFAULT_INTERACTIVE_CAPACITY,
                WorkerPoolConfig::DEFAULT_NORMAL_CAPACITY,
                WorkerPoolConfig::DEFAULT_BACKGROUND_CAPACITY,
            ],
        })
    }

    /// 이 기기의 병렬성으로 정한 설정: UI·document·render thread 몫 3을 뺀 수를 2..=[`MAX_WORKERS`]로
    /// 자릅니다(최소 2라 interactive 전용 worker 하나가 남음).
    pub fn for_available_parallelism() -> WorkerPoolConfig {
        let available = thread::available_parallelism().map_or(1, usize::from);
        let workers = available.saturating_sub(3).clamp(2, MAX_WORKERS);
        WorkerPoolConfig {
            workers,
            non_interactive_limit: default_non_interactive_limit(workers),
            background_limit: default_background_limit(workers),
            capacity: [
                WorkerPoolConfig::DEFAULT_INTERACTIVE_CAPACITY,
                WorkerPoolConfig::DEFAULT_NORMAL_CAPACITY,
                WorkerPoolConfig::DEFAULT_BACKGROUND_CAPACITY,
            ],
        }
    }

    /// non-interactive(`Normal`·`Background` 합계) 동시 실행 상한을 바꿉니다(1..=worker 수). worker 수와
    /// 같으면 interactive 전용 worker가 없어 긴 저장·GC 동안 interactive job이 기다릴 수 있습니다.
    pub fn with_non_interactive_limit(
        self,
        limit: usize,
    ) -> Result<WorkerPoolConfig, WorkerPoolConfigError> {
        if limit == 0 || limit > self.workers {
            return Err(WorkerPoolConfigError::InvalidNonInteractiveLimit {
                requested: limit,
                workers: self.workers,
            });
        }
        Ok(WorkerPoolConfig {
            non_interactive_limit: limit,
            ..self
        })
    }

    /// background 동시 실행 상한을 바꿉니다(1..=worker 수). 실제 동시 실행 수는 non-interactive 상한도
    /// 넘지 않습니다.
    pub fn with_background_limit(
        self,
        limit: usize,
    ) -> Result<WorkerPoolConfig, WorkerPoolConfigError> {
        if limit == 0 || limit > self.workers {
            return Err(WorkerPoolConfigError::InvalidBackgroundLimit {
                requested: limit,
                workers: self.workers,
            });
        }
        Ok(WorkerPoolConfig {
            background_limit: limit,
            ..self
        })
    }

    /// lane 용량을 바꿉니다(1 이상).
    pub fn with_capacity(
        self,
        priority: WorkerPriority,
        capacity: usize,
    ) -> Result<WorkerPoolConfig, WorkerPoolConfigError> {
        if capacity == 0 {
            return Err(WorkerPoolConfigError::ZeroCapacity { priority });
        }
        let mut lanes = self.capacity;
        lanes[priority.lane()] = capacity;
        Ok(WorkerPoolConfig {
            capacity: lanes,
            ..self
        })
    }

    /// worker 수.
    pub const fn workers(&self) -> usize {
        self.workers
    }

    /// non-interactive(`Normal`·`Background` 합계) job 동시 실행 상한.
    pub const fn non_interactive_limit(&self) -> usize {
        self.non_interactive_limit
    }

    /// background job 동시 실행 상한.
    pub const fn background_limit(&self) -> usize {
        self.background_limit
    }

    /// lane 용량.
    pub const fn capacity(&self, priority: WorkerPriority) -> usize {
        self.capacity[priority.lane()]
    }
}

// ---------------------------------------------------------------------------
// job 접수·실행 타입
// ---------------------------------------------------------------------------

/// pool 안 job 번호(접수 순서, 1부터).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerJobId(u64);

impl WorkerJobId {
    /// 번호.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// 실행 중인 job이 받는 context.
#[derive(Debug)]
pub struct WorkerContext<'a> {
    id: WorkerJobId,
    spec: WorkerJobSpec,
    cancelled: &'a AtomicBool,
}

impl WorkerContext<'_> {
    /// job 번호.
    pub fn job_id(&self) -> WorkerJobId {
        self.id
    }

    /// job tag.
    pub fn spec(&self) -> WorkerJobSpec {
        self.spec
    }

    /// pool이 종료 중인지. 오래 걸리는 job은 이 값을 보고 일찍 끝냅니다.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// 접수 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerRejectReason {
    /// lane 용량이 찼습니다.
    QueueFull {
        /// 해당 lane.
        priority: WorkerPriority,
    },
    /// pool이 종료되어 접수를 닫았습니다.
    Closed,
}

impl fmt::Display for WorkerRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerRejectReason::QueueFull { priority } => {
                write!(f, "worker {} lane 용량이 찼습니다", priority.name())
            }
            WorkerRejectReason::Closed => f.write_str("worker pool 접수가 닫혔습니다"),
        }
    }
}

/// 접수 거부. 제출한 payload를 돌려줍니다.
pub struct WorkerRejected<P> {
    /// 거부 이유.
    pub reason: WorkerRejectReason,
    /// 접수되지 않은 payload.
    pub payload: P,
}

impl<P> fmt::Debug for WorkerRejected<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerRejected")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// pool 집계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerPoolStats {
    /// 접수한 job 수.
    pub submitted: u64,
    /// 거부한 job 수.
    pub rejected: u64,
    /// 끝까지 실행한 job 수.
    pub completed: u64,
    /// 실행 중 panic한 job 수(dev·test profile).
    pub panicked: u64,
    /// 종료 때 실행하지 않고 버린 job 수.
    pub discarded: u64,
    /// class별 완료 수([`WorkerClass::ALL`] 순서).
    pub completed_by_class: [u64; 6],
    /// 동시에 실행한 background job의 최대 수.
    pub peak_background_running: usize,
    /// 동시에 실행한 non-interactive(`Normal`·`Background`) job의 최대 수.
    pub peak_non_interactive_running: usize,
}

impl WorkerPoolStats {
    /// class의 완료 수.
    pub fn completed_for(&self, class: WorkerClass) -> u64 {
        self.completed_by_class[class.index()]
    }
}

/// pool 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPoolError {
    /// worker thread를 만들지 못했습니다.
    Spawn,
}

impl fmt::Display for WorkerPoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerPoolError::Spawn => f.write_str("worker thread를 만들지 못했습니다"),
        }
    }
}

impl std::error::Error for WorkerPoolError {}

/// blocking 대기 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitIdleError {
    /// 현재 thread 역할에서는 blocking 대기가 금지됩니다.
    BlockingNotAllowed {
        /// 현재 thread 역할.
        role: ThreadRole,
    },
}

impl fmt::Display for WaitIdleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WaitIdleError::BlockingNotAllowed { role } => {
                write!(f, "{role:?} thread에서는 worker pool을 기다릴 수 없습니다")
            }
        }
    }
}

impl std::error::Error for WaitIdleError {}

/// 종료 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerPoolShutdown {
    /// 실행하지 않고 버린 job 수.
    pub discarded: u64,
    /// join이 panic을 알린 worker 수(정상이면 0, job panic은 worker 안에서 잡음).
    pub panicked_workers: usize,
}

/// blocking 대기가 허용되는 thread인지 확인합니다. UI·Document·Render는 기다리지 않고, worker가
/// 자기 pool을 기다리면 deadlock이 될 수 있어 역할이 없는 thread(테스트·headless 도구)만 허용합니다.
pub(crate) fn ensure_unbound_thread() -> Result<(), ThreadRole> {
    match current_thread_role() {
        Some(role) => Err(role),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// queue 상태
// ---------------------------------------------------------------------------

type BoxedJob = Box<dyn for<'a> FnOnce(&WorkerContext<'a>) + Send>;

struct QueuedJob {
    id: WorkerJobId,
    spec: WorkerJobSpec,
    run: BoxedJob,
}

struct PoolState {
    lanes: [VecDeque<QueuedJob>; 3],
    /// priority별 실행 중 job 수.
    running: [usize; 3],
    closed: bool,
    next_id: u64,
    stats: WorkerPoolStats,
}

impl PoolState {
    /// 지금 실행할 수 있는 가장 높은 priority의 가장 오래된 job(모듈 문서의 실행 순서).
    fn pop_runnable(&mut self, config: &WorkerPoolConfig) -> Option<QueuedJob> {
        let non_interactive_full = self.non_interactive_running() >= config.non_interactive_limit();
        for priority in WorkerPriority::ALL {
            let blocked = match priority {
                WorkerPriority::Interactive => false,
                WorkerPriority::Normal => non_interactive_full,
                WorkerPriority::Background => {
                    non_interactive_full
                        || self.running[priority.lane()] >= config.background_limit()
                }
            };
            if blocked {
                continue;
            }
            if let Some(job) = self.lanes[priority.lane()].pop_front() {
                return Some(job);
            }
        }
        None
    }

    fn non_interactive_running(&self) -> usize {
        self.running[WorkerPriority::Normal.lane()]
            + self.running[WorkerPriority::Background.lane()]
    }

    fn non_interactive_queued(&self) -> bool {
        !self.lanes[WorkerPriority::Normal.lane()].is_empty()
            || !self.lanes[WorkerPriority::Background.lane()].is_empty()
    }

    fn queued(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    fn running_total(&self) -> usize {
        self.running.iter().sum()
    }

    fn is_idle(&self) -> bool {
        self.queued() == 0 && self.running_total() == 0
    }
}

struct Shared {
    config: WorkerPoolConfig,
    state: Mutex<PoolState>,
    /// worker가 job을 기다리는 condvar.
    ready: Condvar,
    /// [`WorkerPool::wait_idle`]이 기다리는 condvar.
    idle: Condvar,
    cancelled: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        // 임계 구역 안에서 panic하는 코드가 없고 job은 lock 밖에서 실행하므로 poison 상태도 일관됩니다.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn worker_loop(shared: &Shared) {
    // 새 thread라 역할이 없으므로 bind는 실패하지 않습니다. 실패하면 job을 받지 않고 끝냅니다.
    let Ok(_role) = bind_thread_role(ThreadRole::Worker) else {
        return;
    };
    let config = shared.config;
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if let Some(job) = state.pop_runnable(&config) {
                    let lane = job.spec.priority.lane();
                    state.running[lane] += 1;
                    if job.spec.priority == WorkerPriority::Background {
                        state.stats.peak_background_running =
                            state.stats.peak_background_running.max(state.running[lane]);
                    }
                    if job.spec.priority != WorkerPriority::Interactive {
                        state.stats.peak_non_interactive_running = state
                            .stats
                            .peak_non_interactive_running
                            .max(state.non_interactive_running());
                    }
                    break job;
                }
                if state.closed {
                    return;
                }
                state = shared
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let QueuedJob { id, spec, run } = job;
        let context = WorkerContext {
            id,
            spec,
            cancelled: &shared.cancelled,
        };
        // dev·test profile에서 job이 panic해도 실행 수 기록을 정리하고 worker를 계속 씁니다.
        let outcome = catch_unwind(AssertUnwindSafe(|| run(&context)));
        let mut state = shared.lock();
        let lane = spec.priority.lane();
        state.running[lane] -= 1;
        match outcome {
            Ok(()) => {
                state.stats.completed += 1;
                state.stats.completed_by_class[spec.class.index()] += 1;
            }
            Err(_) => state.stats.panicked += 1,
        }
        // non-interactive(또는 background) 자리가 났고 상한 때문에 기다리는 normal·background job이 있으면
        // 쉬는 worker 하나를 깨웁니다.
        let wake_waiting =
            spec.priority != WorkerPriority::Interactive && state.non_interactive_queued();
        let idle = state.is_idle();
        drop(state);
        if wake_waiting {
            shared.ready.notify_one();
        }
        if idle {
            shared.idle.notify_all();
        }
    }
}

// ---------------------------------------------------------------------------
// WorkerPool
// ---------------------------------------------------------------------------

/// class·priority tag로 실행 순서를 정하는 worker pool(모듈 문서).
pub struct WorkerPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.shared.lock();
        f.debug_struct("WorkerPool")
            .field("config", &self.shared.config)
            .field("queued", &state.queued())
            .field("running", &state.running)
            .field("closed", &state.closed)
            .finish()
    }
}

impl WorkerPool {
    /// 설정대로 worker thread를 시작합니다.
    pub fn new(config: WorkerPoolConfig) -> Result<WorkerPool, WorkerPoolError> {
        let shared = Arc::new(Shared {
            config,
            state: Mutex::new(PoolState {
                lanes: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
                running: [0; 3],
                closed: false,
                next_id: 1,
                stats: WorkerPoolStats::default(),
            }),
            ready: Condvar::new(),
            idle: Condvar::new(),
            cancelled: AtomicBool::new(false),
        });
        let mut pool = WorkerPool {
            shared,
            workers: Vec::with_capacity(config.workers()),
        };
        for index in 0..config.workers() {
            let shared = Arc::clone(&pool.shared);
            let handle = thread::Builder::new()
                .name(format!("{WORKER_THREAD_NAME_PREFIX}{index}"))
                .spawn(move || worker_loop(&shared))
                .map_err(|_| WorkerPoolError::Spawn)?;
            pool.workers.push(handle);
        }
        Ok(pool)
    }

    /// 설정.
    pub fn config(&self) -> WorkerPoolConfig {
        self.shared.config
    }

    /// job을 기다리지 않고 접수합니다. 거부되면 job을 돌려줍니다.
    pub fn try_submit<F>(
        &self,
        spec: impl Into<WorkerJobSpec>,
        job: F,
    ) -> Result<WorkerJobId, WorkerRejected<F>>
    where
        F: for<'a> FnOnce(&WorkerContext<'a>) + Send + 'static,
    {
        self.try_submit_with(spec, job, |job, context| job(context))
    }

    /// `payload`를 넘겨 `run`을 실행하는 job을 기다리지 않고 접수합니다. 거부되면 `payload`를
    /// 돌려줍니다(호출자가 나중에 다시 제출).
    pub fn try_submit_with<P, F>(
        &self,
        spec: impl Into<WorkerJobSpec>,
        payload: P,
        run: F,
    ) -> Result<WorkerJobId, WorkerRejected<P>>
    where
        P: Send + 'static,
        F: for<'a> FnOnce(P, &WorkerContext<'a>) + Send + 'static,
    {
        let spec = spec.into();
        let mut state = self.shared.lock();
        if state.closed {
            state.stats.rejected += 1;
            return Err(WorkerRejected {
                reason: WorkerRejectReason::Closed,
                payload,
            });
        }
        let lane = spec.priority.lane();
        if state.lanes[lane].len() >= self.shared.config.capacity(spec.priority) {
            state.stats.rejected += 1;
            return Err(WorkerRejected {
                reason: WorkerRejectReason::QueueFull {
                    priority: spec.priority,
                },
                payload,
            });
        }
        let id = WorkerJobId(state.next_id);
        state.next_id += 1;
        state.stats.submitted += 1;
        state.lanes[lane].push_back(QueuedJob {
            id,
            spec,
            run: Box::new(move |context: &WorkerContext<'_>| run(payload, context)),
        });
        drop(state);
        self.shared.ready.notify_one();
        Ok(id)
    }

    /// 아직 시작하지 않은 job 수.
    pub fn queued(&self) -> usize {
        self.shared.lock().queued()
    }

    /// 실행 중인 job 수.
    pub fn running(&self) -> usize {
        self.shared.lock().running_total()
    }

    /// 집계.
    pub fn stats(&self) -> WorkerPoolStats {
        self.shared.lock().stats
    }

    /// 대기·실행 중인 job이 없어질 때까지 `timeout`까지 기다립니다. 비면 `true`.
    ///
    /// 역할이 없는 thread(테스트·headless 도구)에서만 쓸 수 있습니다. UI·Document·Render thread는
    /// 기다리지 않고, worker thread가 자기 pool을 기다리면 끝나지 않을 수 있기 때문입니다.
    pub fn wait_idle(&self, timeout: Duration) -> Result<bool, WaitIdleError> {
        ensure_unbound_thread().map_err(|role| WaitIdleError::BlockingNotAllowed { role })?;
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        while !state.is_idle() {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            state = self
                .shared
                .idle
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(true)
    }

    /// 접수를 닫고 시작하지 않은 job을 버린 뒤 다른 worker를 join합니다. 호출한 worker 자신은
    /// join하지 않고 현재 job 반환 후 종료합니다.
    pub fn shutdown(mut self) -> WorkerPoolShutdown {
        self.stop()
    }

    fn stop(&mut self) -> WorkerPoolShutdown {
        let discarded: Vec<QueuedJob> = {
            let mut state = self.shared.lock();
            state.closed = true;
            let mut discarded = Vec::new();
            for lane in &mut state.lanes {
                discarded.extend(lane.drain(..));
            }
            state.stats.discarded += discarded.len() as u64;
            discarded
        };
        self.shared.cancelled.store(true, Ordering::Release);
        self.shared.ready.notify_all();
        self.shared.idle.notify_all();
        let count = discarded.len() as u64;
        // closure(캡처 값 포함)는 lock 밖에서 버립니다.
        drop(discarded);
        let mut panicked_workers = 0;
        let current = thread::current().id();
        for worker in self.workers.drain(..) {
            if worker.thread().id() == current {
                // job이 마지막 pool owner를 놓은 경우. 자기 join은 panic하므로 handle만 놓고,
                // 현재 job 반환 후 worker_loop가 closed를 보고 종료하게 합니다.
                continue;
            }
            if worker.join().is_err() {
                panicked_workers += 1;
            }
        }
        WorkerPoolShutdown {
            discarded: count,
            panicked_workers,
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        if !self.workers.is_empty() {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests;
