//! tessellation worker pool과 stale scene revision 결과 폐기 (XI.6.3).
//!
//! 아키텍처 §9.2: tessellation은 Worker Pool에서 실행하고, 결과는 입력 hash와 세대가 현재 상태와
//! 맞을 때만 반영합니다. layout worker result guard(X.7.5)와 같은 규칙을 tessellation에 적용합니다.
//!
//! - **worker 실행**: [`TessellationWorkerPool`]은 thread마다 [`ThreadRole::Worker`]를 bind하고 job을
//!   실행합니다. job queue는 상한이 있는 channel이라 render thread는 [`TessellationWorkerPool::try_submit`]
//!   에서 막히지 않고 `Full`을 받습니다.
//! - **입력 hash**: job의 입력 식별은 [`TessellationKey`](path canonical bytes + style)입니다. key가 같으면
//!   mesh가 같으므로(결정론 계약) 같은 key의 진행 중 job은 하나로 합칩니다.
//! - **세대**: render thread는 [`AsyncTessellation::request`]에 그 key를 필요로 하는 scene revision을
//!   넘깁니다. 진행 중인 key를 더 새 revision이 다시 요청하면 job의 "필요 revision"을 올립니다.
//! - **현재 frame revision**: 판정 기준은 render thread가 지금 처리 중인 scene revision입니다
//!   ([`AsyncTessellation::begin_frame`]으로 알리거나, `request`에 넘긴 revision 중 가장 큰 값). document
//!   thread가 publish한 최신 세대([`stellar_scene::freshness::LatestSceneRevision`])는 기준으로 쓰지
//!   않습니다. render thread는 아직 받지 않은 snapshot이 무엇을 요청할지 알 수 없으므로, 그 세대와 비교하면
//!   편집이 이어지는 동안(frame마다 새 revision) 도착하는 결과가 모두 오래된 것으로 판정되어 같은 key를
//!   계속 다시 제출하고 cache에는 영영 들어가지 못합니다.
//! - **stale 폐기**: [`AsyncTessellation::collect`]는 결과의 필요 revision이 현재 frame revision보다
//!   작으면(render thread가 처리 중인 scene이 그 key를 요청하지 않았으면) cache에 넣지 않고 버립니다. 실패
//!   결과도 cache에 넣지 않습니다(다음 요청에서 재시도).
//!
//! ## frame 규약
//!
//! render thread는 frame마다 `begin_frame(revision)` → 그 scene이 필요로 하는 key마다 `request` →
//! `collect` 순서로 호출합니다. 이 순서면 "현재 scene이 요청한 key의 결과만 적용"이 정확히 성립합니다.
//! frame 시작에 `begin_frame` 전에 `collect`를 호출해도 직전 frame 기준으로 판정하므로 starvation은 없습니다
//! (직전 frame이 요청한 key는 적용). `begin_frame` 뒤, 그 frame의 `request` 전에 `collect`를 호출하면 안
//! 됩니다(직전 frame이 요청한 key가 모두 stale로 판정됨).
//!
//! ## runtime worker pool과의 관계
//!
//! 작업 class·priority 분류는 runtime의 worker pool(XII.1.4, `stellar_runtime::worker_pool`)에 있고, 이
//! pool은 아직 그쪽으로 옮기지 않은 **별도 thread pool**입니다. `AsyncTessellation`이 이 pool을 구체
//! 타입으로 소유하고 renderer-core는 runtime에 의존하지 않으므로, 옮기려면 job 제출을 추상화하는
//! renderer-core API 변경이 필요합니다. 옮기는 일과 그 전까지의 thread 수 조정은
//! `docs/task-reports/XII.1.4.md` "남은 위험"에 적은 후속 작업입니다. 두 pool을 함께 띄우면 이 pool의
//! thread는 runtime pool 크기 계산(`WorkerPoolConfig::for_available_parallelism`)에 들어가지 않으므로
//! CPU를 과다 점유할 수 있습니다.
//!
//! ## panic
//!
//! tessellator는 panic하지 않는다는 계약(XI.6.1·XI.6.2)을 전제합니다. release profile은 `panic = "abort"`이므로 panic은 process를
//! 끝냅니다. dev·test profile에서 tessellator가 panic하면 worker가 그 job을 [`TessellationError::Failed`]
//! 결과로 보내고 계속 실행합니다(진행 중 기록이 남아 같은 key가 영구히 `Pending`이 되지 않게 함).

use std::collections::HashMap;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use stellar_scene::geometry::PathGeometry;
use stellar_scene::handle::SnapshotHandle;

use crate::stroke::{StrokeStyle, StrokeTessellator};
use crate::tessellation::{
    FillMesh, FillStyle, FillTessellator, TessellationCache, TessellationError, TessellationKey,
};
use crate::thread_role::{bind_thread_role, ThreadRole};

/// worker thread 최대 수.
pub const MAX_TESSELLATION_WORKERS: usize = 16;

/// tessellation 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TessellationInput {
    /// fill.
    Fill {
        /// path.
        path: PathGeometry,
        /// style.
        style: FillStyle,
    },
    /// stroke.
    Stroke {
        /// path.
        path: PathGeometry,
        /// style.
        style: StrokeStyle,
    },
}

impl TessellationInput {
    /// cache key(입력 hash).
    pub fn key(&self) -> TessellationKey {
        match self {
            TessellationInput::Fill { path, style } => TessellationKey::fill(path, *style),
            TessellationInput::Stroke { path, style } => TessellationKey::stroke(path, style),
        }
    }
}

/// worker가 쓰는 tessellator 묶음.
#[derive(Clone)]
pub struct Tessellators {
    /// fill tessellator.
    pub fill: Arc<dyn FillTessellator>,
    /// stroke tessellator.
    pub stroke: Arc<dyn StrokeTessellator>,
}

impl fmt::Debug for Tessellators {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tessellators")
    }
}

impl Tessellators {
    fn run(&self, input: &TessellationInput) -> Result<FillMesh, TessellationError> {
        match input {
            TessellationInput::Fill { path, style } => self.fill.tessellate_fill(path, style),
            TessellationInput::Stroke { path, style } => {
                self.stroke.tessellate_stroke(path, &style.canonical())
            }
        }
    }
}

/// worker에 보내는 job.
#[derive(Debug, Clone)]
pub struct TessellationJob {
    /// 입력 key.
    pub key: TessellationKey,
    /// 입력.
    pub input: TessellationInput,
    /// job을 만든 scene revision.
    pub revision: u64,
}

/// worker 결과.
#[derive(Debug, Clone)]
pub struct TessellationOutput {
    /// 입력 key.
    pub key: TessellationKey,
    /// job을 만든 scene revision.
    pub revision: u64,
    /// tessellation 결과.
    pub mesh: Result<FillMesh, TessellationError>,
}

/// worker pool 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPoolError {
    /// worker 수가 1..=[`MAX_TESSELLATION_WORKERS`]가 아닙니다.
    InvalidWorkerCount {
        /// 요청 값.
        requested: usize,
    },
    /// queue 용량이 0입니다.
    InvalidCapacity,
    /// thread를 만들지 못했습니다.
    Spawn,
}

impl fmt::Display for WorkerPoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerPoolError::InvalidWorkerCount { requested } => write!(
                f,
                "tessellation worker 수는 1..={MAX_TESSELLATION_WORKERS}이어야 합니다: {requested}"
            ),
            WorkerPoolError::InvalidCapacity => f.write_str("job queue 용량은 0보다 커야 합니다"),
            WorkerPoolError::Spawn => f.write_str("tessellation worker thread를 만들지 못했습니다"),
        }
    }
}

impl std::error::Error for WorkerPoolError {}

/// job 제출 실패. job을 돌려줍니다.
#[derive(Debug)]
pub enum SubmitError {
    /// queue가 찼습니다(다음 frame에 다시 시도).
    Full(Box<TessellationJob>),
    /// worker가 모두 종료되었습니다.
    Disconnected(Box<TessellationJob>),
}

/// tessellation worker pool.
///
/// drop하면 job queue를 닫고 실행 중인 job이 끝난 뒤 모든 worker를 join합니다.
#[derive(Debug)]
pub struct TessellationWorkerPool {
    jobs: Option<SyncSender<TessellationJob>>,
    results: Receiver<TessellationOutput>,
    workers: Vec<JoinHandle<()>>,
}

fn worker_loop(
    jobs: Arc<Mutex<Receiver<TessellationJob>>>,
    results: mpsc::Sender<TessellationOutput>,
    tessellators: Tessellators,
) {
    // 새 thread라 역할이 없으므로 bind는 실패하지 않습니다. 실패하면 job을 받지 않고 끝냅니다.
    let Ok(_role) = bind_thread_role(ThreadRole::Worker) else {
        return;
    };
    loop {
        let job = match jobs.lock() {
            Ok(receiver) => receiver.recv(),
            // 다른 worker가 lock을 쥔 채 panic한 경우: queue를 더 쓰지 않습니다.
            Err(_) => return,
        };
        let Ok(job) = job else {
            // queue가 닫혔습니다(pool drop).
            return;
        };
        // dev·test profile에서 tessellator가 panic해도 결과를 보내야 render thread의 진행 중 기록이
        // 정리됩니다(release는 panic = "abort"라 여기까지 오지 않음).
        let mesh = catch_unwind(AssertUnwindSafe(|| tessellators.run(&job.input))).unwrap_or(Err(
            TessellationError::Failed {
                reason: "tessellator가 panic했습니다",
            },
        ));
        let output = TessellationOutput {
            key: job.key,
            revision: job.revision,
            mesh,
        };
        if results.send(output).is_err() {
            return;
        }
    }
}

impl TessellationWorkerPool {
    /// `workers`개 thread와 용량 `capacity`의 job queue로 pool을 만듭니다.
    pub fn new(
        workers: usize,
        capacity: usize,
        tessellators: Tessellators,
    ) -> Result<TessellationWorkerPool, WorkerPoolError> {
        if workers == 0 || workers > MAX_TESSELLATION_WORKERS {
            return Err(WorkerPoolError::InvalidWorkerCount { requested: workers });
        }
        if capacity == 0 {
            return Err(WorkerPoolError::InvalidCapacity);
        }
        let (job_tx, job_rx) = mpsc::sync_channel::<TessellationJob>(capacity);
        let (result_tx, result_rx) = mpsc::channel::<TessellationOutput>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let mut pool = TessellationWorkerPool {
            jobs: Some(job_tx),
            results: result_rx,
            workers: Vec::with_capacity(workers),
        };
        for index in 0..workers {
            let jobs = Arc::clone(&job_rx);
            let results = result_tx.clone();
            let tessellators = tessellators.clone();
            let handle = thread::Builder::new()
                .name(format!("stellar-tessellation-{index}"))
                .spawn(move || worker_loop(jobs, results, tessellators))
                .map_err(|_| WorkerPoolError::Spawn)?;
            pool.workers.push(handle);
        }
        Ok(pool)
    }

    /// worker 수.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// job을 막힘 없이 제출합니다.
    pub fn try_submit(&self, job: TessellationJob) -> Result<(), SubmitError> {
        let Some(jobs) = self.jobs.as_ref() else {
            return Err(SubmitError::Disconnected(Box::new(job)));
        };
        jobs.try_send(job).map_err(|error| match error {
            TrySendError::Full(job) => SubmitError::Full(Box::new(job)),
            TrySendError::Disconnected(job) => SubmitError::Disconnected(Box::new(job)),
        })
    }

    /// 끝난 결과 하나를 막힘 없이 꺼냅니다.
    pub fn try_recv(&self) -> Option<TessellationOutput> {
        self.results.try_recv().ok()
    }

    /// 결과 하나를 `timeout`까지 기다립니다(headless·종료 처리용, render thread frame 경로에서는
    /// [`TessellationWorkerPool::try_recv`]를 씁니다).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<TessellationOutput> {
        self.results.recv_timeout(timeout).ok()
    }
}

impl Drop for TessellationWorkerPool {
    fn drop(&mut self) {
        // queue를 닫으면 대기 중인 worker의 recv가 끝납니다.
        self.jobs = None;
        for worker in self.workers.drain(..) {
            // panic한 worker의 join 오류는 무시합니다(이미 결과를 보내지 못함).
            let _ = worker.join();
        }
    }
}

/// 요청 결과.
#[derive(Debug, Clone)]
pub enum RequestOutcome {
    /// cache에 있습니다.
    Ready(Arc<FillMesh>),
    /// worker가 계산 중입니다(같은 key의 진행 중 job과 합침 포함).
    Pending,
    /// queue·진행 중 상한이 차서 이번에는 제출하지 않았습니다(다음 frame에 다시 요청).
    Deferred,
}

/// 결과 처리 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectedResult {
    /// cache에 넣었습니다.
    Applied {
        /// key.
        key: TessellationKey,
        /// 결과를 필요로 한 scene revision.
        revision: u64,
    },
    /// render thread가 처리 중인 scene이 요청하지 않은 오래된 결과라 버렸습니다.
    DiscardedStale {
        /// key.
        key: TessellationKey,
        /// 결과를 필요로 한 scene revision.
        revision: u64,
        /// 판정 시점 현재 frame revision([`AsyncTessellation::frame_revision`]).
        current: u64,
    },
    /// tessellation이 실패했습니다(cache에 넣지 않음).
    Failed {
        /// key.
        key: TessellationKey,
        /// 오류.
        error: TessellationError,
    },
}

/// 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AsyncTessellationStats {
    /// 제출한 job 수.
    pub submitted: u64,
    /// 진행 중 job과 합친 요청 수.
    pub coalesced: u64,
    /// 제출하지 못하고 미룬 요청 수.
    pub deferred: u64,
    /// cache에 넣은 결과 수.
    pub applied: u64,
    /// stale로 버린 결과 수.
    pub discarded_stale: u64,
    /// 실패 결과 수.
    pub failed: u64,
}

/// render thread 쪽 비동기 tessellation 관리자: 요청 합치기·제출·stale 폐기(모듈 문서의 frame 규약).
#[derive(Debug)]
pub struct AsyncTessellation {
    pool: TessellationWorkerPool,
    /// render thread가 처리 중인 scene revision(단조 증가, 0은 아직 없음).
    frame_revision: u64,
    /// 진행 중 key → 그 key를 필요로 한 가장 새 scene revision.
    in_flight: HashMap<TessellationKey, u64>,
    max_in_flight: usize,
    stats: AsyncTessellationStats,
}

impl AsyncTessellation {
    /// pool로 만듭니다. 진행 중 job은 최대 `max_in_flight`개입니다(0이면 1로 둡니다).
    pub fn new(pool: TessellationWorkerPool, max_in_flight: usize) -> AsyncTessellation {
        AsyncTessellation {
            pool,
            frame_revision: 0,
            in_flight: HashMap::new(),
            max_in_flight: max_in_flight.max(1),
            stats: AsyncTessellationStats::default(),
        }
    }

    /// render thread가 scene revision `revision`의 frame을 시작합니다(모듈 문서의 frame 규약).
    ///
    /// 현재 frame revision은 줄어들지 않습니다(더 작은 값은 무시). 요청이 하나도 없는 frame도 이것으로
    /// 알려야 이전 frame만 요청한 key의 결과가 stale로 판정됩니다.
    pub fn begin_frame(&mut self, revision: u64) {
        self.frame_revision = self.frame_revision.max(revision);
    }

    /// snapshot의 scene revision으로 [`AsyncTessellation::begin_frame`]을 호출합니다.
    pub fn begin_frame_for(&mut self, snapshot: &SnapshotHandle) {
        self.begin_frame(snapshot.scene_revision().0);
    }

    /// 현재 frame revision(stale 판정 기준, 0은 아직 없음).
    pub fn frame_revision(&self) -> u64 {
        self.frame_revision
    }

    /// 통계.
    pub fn stats(&self) -> AsyncTessellationStats {
        self.stats
    }

    /// 진행 중 job 수.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// `key`가 진행 중이면 그 key를 필요로 한 가장 새 revision.
    pub fn in_flight_revision(&self, key: &TessellationKey) -> Option<u64> {
        self.in_flight.get(key).copied()
    }

    /// scene revision `revision`이 `input` mesh를 요청합니다.
    ///
    /// 현재 frame revision을 `revision`까지 올립니다([`AsyncTessellation::begin_frame`]과 같음). cache에
    /// 있으면 바로 돌려주고, 같은 key가 진행 중이면 필요 revision만 올립니다. 아니면 worker에 제출합니다.
    pub fn request(
        &mut self,
        cache: &mut TessellationCache,
        revision: u64,
        input: TessellationInput,
    ) -> RequestOutcome {
        self.begin_frame(revision);
        let key = input.key();
        if let Some(mesh) = cache.lookup(&key) {
            return RequestOutcome::Ready(mesh);
        }
        if let Some(wanted) = self.in_flight.get_mut(&key) {
            *wanted = (*wanted).max(revision);
            self.stats.coalesced += 1;
            return RequestOutcome::Pending;
        }
        if self.in_flight.len() >= self.max_in_flight {
            self.stats.deferred += 1;
            return RequestOutcome::Deferred;
        }
        let job = TessellationJob {
            key: key.clone(),
            input,
            revision,
        };
        match self.pool.try_submit(job) {
            Ok(()) => {
                self.in_flight.insert(key, revision);
                self.stats.submitted += 1;
                RequestOutcome::Pending
            }
            Err(SubmitError::Full(_) | SubmitError::Disconnected(_)) => {
                self.stats.deferred += 1;
                RequestOutcome::Deferred
            }
        }
    }

    /// snapshot의 scene revision으로 [`AsyncTessellation::request`]를 호출합니다.
    pub fn request_for(
        &mut self,
        cache: &mut TessellationCache,
        snapshot: &SnapshotHandle,
        input: TessellationInput,
    ) -> RequestOutcome {
        self.request(cache, snapshot.scene_revision().0, input)
    }

    fn judge(
        &mut self,
        cache: &mut TessellationCache,
        output: TessellationOutput,
    ) -> CollectedResult {
        // 진행 중 기록이 없는 결과(중복·알 수 없는 key)는 job revision으로 판정합니다.
        let wanted = self
            .in_flight
            .remove(&output.key)
            .map_or(output.revision, |wanted| wanted.max(output.revision));
        // 기준은 render thread가 처리 중인 scene입니다(document thread의 최신 publish 세대가 아님,
        // 모듈 문서). 현재 scene이 다시 요청한 key는 wanted가 현재 frame revision까지 올라와 있습니다.
        let current = self.frame_revision;
        if wanted < current {
            self.stats.discarded_stale += 1;
            return CollectedResult::DiscardedStale {
                key: output.key,
                revision: wanted,
                current,
            };
        }
        match output.mesh {
            Ok(mesh) => {
                cache.insert(output.key.clone(), mesh);
                self.stats.applied += 1;
                CollectedResult::Applied {
                    key: output.key,
                    revision: wanted,
                }
            }
            Err(error) => {
                self.stats.failed += 1;
                CollectedResult::Failed {
                    key: output.key,
                    error,
                }
            }
        }
    }

    /// 끝난 결과를 막힘 없이 모두 처리합니다(frame의 `request`를 마친 뒤 호출, 모듈 문서의 frame 규약).
    pub fn collect(&mut self, cache: &mut TessellationCache) -> Vec<CollectedResult> {
        let mut out = Vec::new();
        while let Some(output) = self.pool.try_recv() {
            out.push(self.judge(cache, output));
        }
        out
    }

    /// 결과 하나를 `timeout`까지 기다려 처리합니다(headless·테스트·종료용).
    pub fn collect_one_blocking(
        &mut self,
        cache: &mut TessellationCache,
        timeout: Duration,
    ) -> Option<CollectedResult> {
        let output = self.pool.recv_timeout(timeout)?;
        Some(self.judge(cache, output))
    }
}

#[cfg(test)]
mod tests;
