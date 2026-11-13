//! RenderThread와 최신 scene snapshot의 lock-free handoff (XII.1.3).
//!
//! 아키텍처 §9.3: render thread는 불변 Scene Snapshot을 잠금 없이 읽고, document thread는 렌더
//! 완료를 기다리지 않으며, 오래된 렌더 결과는 세대 번호로 폐기합니다. 이 모듈은 그 전달 경로를
//! 구현합니다.
//!
//! # handoff
//!
//! document thread(생산자)와 render thread(소비자) 사이에는 **한 칸짜리 mailbox**가 있습니다.
//!
//! - [`RenderPublisher::publish`]는 새 frame을 mailbox에 atomic swap 한 번으로 넣고 render thread를
//!   깨웁니다(`Thread::unpark`). 아직 렌더되지 않은 이전 frame이 있으면 swap으로 돌려받아 렌더 없이
//!   버립니다(**latest wins**). 기다리는 연산(lock, condvar, 용량 대기, 렌더 완료 대기)이 없으므로
//!   render thread가 GPU 제출 중이거나 멈춰 있거나 panic으로 끝났어도 document thread는 바로
//!   돌아옵니다.
//! - render thread는 mailbox를 atomic swap 한 번으로 비워 frame을 꺼냅니다. 꺼낸 frame은 render
//!   thread만 소유하므로 snapshot을 읽을 때 lock이 필요 없습니다(snapshot 자체가 불변, XI.1.5).
//! - mailbox에는 최대 한 frame만 대기하므로 render thread가 느려도 대기 snapshot 메모리는 늘지
//!   않습니다.
//!
//! # 세대 검사
//!
//! publish는 frame 세대([`SceneRevision`])를 공유 atomic 최신값에 먼저 알립니다(단조, 0과 역행은
//! 거부). render thread는 렌더 전에, renderer는 GPU 제출 직전에 [`FrameFreshness`]로 그 사이 더 새
//! 세대가 알려졌는지 확인하고, 그렇다면 제출하지 않고 폐기합니다(XI.1.6과 같은 규칙).
//!
//! # snapshot 타입
//!
//! crate 의존 정책상 runtime은 `stellar-scene`에 직접 의존하지 않으므로 frame 내용 타입 `S`는
//! generic입니다. 앱에서는 `stellar_scene::handle::SnapshotHandle`(`Arc<SceneSnapshot>`,
//! `Send + Sync`)을 넣습니다. 세대 번호는 core-types의 [`SceneRevision`]을 씁니다.
//!
//! # 종료
//!
//! [`RenderThread::shutdown`]은 정지 표시 → 깨움 → join 순서입니다. 대기 중이던 frame은 렌더하지
//! 않고 버립니다. 종료 뒤 publish는 [`PublishError::Stopped`]입니다. thread별 전체 종료 순서는
//! XII.1.6 소관입니다.

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle, Thread, ThreadId};

use stellar_core_types::revision::SceneRevision;
use stellar_renderer_core::thread_role::{
    bind_thread_role, current_thread_role, ThreadRole, ThreadRoleError,
};

/// render thread 이름(디버거·trace 표시용).
pub const RENDER_THREAD_NAME: &str = "stellar-render";

/// 현재 thread가 render thread인지 여부.
pub fn on_render_thread() -> bool {
    current_thread_role() == Some(ThreadRole::Render)
}

// ---------------------------------------------------------------------------
// LatestSlot: 한 칸짜리 lock-free mailbox
// ---------------------------------------------------------------------------

/// 값 하나를 담는 lock-free mailbox. 넣기·꺼내기가 각각 atomic swap 한 번입니다(wait-free).
///
/// 저장된 pointer는 slot이 소유합니다. swap으로 pointer를 빼낸 thread가 그 값의 유일한 소유자가
/// 되므로 같은 값을 두 thread가 동시에 읽거나 두 번 해제하는 일이 없습니다. compare-and-swap을 쓰지
/// 않으므로 주소 재사용(ABA)도 문제가 되지 않습니다.
struct LatestSlot<T> {
    ptr: AtomicPtr<T>,
    /// 저장된 `T`를 소유함을 drop check에 알립니다.
    _owns: PhantomData<T>,
}

// SAFETY: slot을 통해서는 `T`를 한 thread에서 넣고 다른 thread에서 꺼낼 수만 있고 `&T`를 여러
// thread가 공유하는 경로가 없습니다(channel과 같은 의미). 따라서 `T: Send`면 slot을 thread 간에
// 공유(`Sync`)해도 안전합니다. `Send`는 `PhantomData<T>`로 `T: Send`일 때만 자동 구현됩니다.
unsafe impl<T: Send> Sync for LatestSlot<T> {}

impl<T> LatestSlot<T> {
    fn new() -> LatestSlot<T> {
        LatestSlot {
            ptr: AtomicPtr::new(ptr::null_mut()),
            _owns: PhantomData,
        }
    }

    /// 새 값을 넣고, 아직 꺼내지 않은 이전 값이 있으면 돌려줍니다.
    fn replace(&self, value: T) -> Option<T> {
        let new = Box::into_raw(Box::new(value));
        // AcqRel: Release로 새 값의 초기화를 꺼내는 쪽에 보이게 하고, Acquire로 이전 값의
        // 초기화를 이 thread에 보이게 합니다.
        let old = self.ptr.swap(new, Ordering::AcqRel);
        // SAFETY: null이 아닌 저장 pointer는 모두 `Box::into_raw`로 만든 것이고, swap으로 빼낸
        // 이 thread만 소유합니다.
        (!old.is_null()).then(|| *unsafe { Box::from_raw(old) })
    }

    /// 값을 꺼냅니다. 비어 있으면 `None`입니다.
    fn take(&self) -> Option<T> {
        let old = self.ptr.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: `replace`와 같습니다.
        (!old.is_null()).then(|| *unsafe { Box::from_raw(old) })
    }

    /// 대기 중인 값이 있는지(진단용, 결과는 바로 바뀔 수 있음).
    fn is_occupied(&self) -> bool {
        !self.ptr.load(Ordering::Acquire).is_null()
    }
}

impl<T> Drop for LatestSlot<T> {
    fn drop(&mut self) {
        drop(self.take());
    }
}

// ---------------------------------------------------------------------------
// frame·freshness
// ---------------------------------------------------------------------------

/// render thread에 넘기는 frame: scene 세대 번호와 불변 snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame<S> {
    revision: SceneRevision,
    snapshot: S,
}

impl<S> RenderFrame<S> {
    /// scene 세대 번호.
    pub fn revision(&self) -> SceneRevision {
        self.revision
    }

    /// 읽기 전용 snapshot. 가변 참조는 제공하지 않습니다.
    pub fn snapshot(&self) -> &S {
        &self.snapshot
    }
}

/// 렌더 중인 frame이 아직 최신인지 확인하는 판정기. GPU 제출 직전에 호출합니다.
#[derive(Debug, Clone, Copy)]
pub struct FrameFreshness<'a> {
    latest: &'a AtomicU64,
    revision: SceneRevision,
}

impl FrameFreshness<'_> {
    /// 판정 대상 frame 세대.
    pub fn revision(&self) -> SceneRevision {
        self.revision
    }

    /// 지금까지 알려진 최신 세대.
    pub fn latest(&self) -> SceneRevision {
        SceneRevision(self.latest.load(Ordering::Acquire))
    }

    /// 더 새 세대가 알려졌으면 참입니다. 참이면 제출하지 않고 폐기합니다.
    pub fn is_stale(&self) -> bool {
        self.revision.0 < self.latest.load(Ordering::Acquire)
    }
}

/// renderer가 frame 하나를 처리한 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// GPU에 제출했습니다(present 후보).
    Presented,
    /// 제출 직전 더 새 세대를 확인해 제출하지 않았습니다.
    Discarded,
}

/// render thread에서 frame을 렌더·제출하는 쪽. render thread 안에서만 호출됩니다.
pub trait FrameRenderer<S>: Send + 'static {
    /// frame 하나를 렌더하고 제출합니다. 제출 직전에 `freshness.is_stale()`로 폐기를 판정할 수
    /// 있습니다.
    fn render(&mut self, frame: &RenderFrame<S>, freshness: &FrameFreshness<'_>) -> FrameOutcome;
}

impl<S, F> FrameRenderer<S> for F
where
    F: FnMut(&RenderFrame<S>, &FrameFreshness<'_>) -> FrameOutcome + Send + 'static,
{
    fn render(&mut self, frame: &RenderFrame<S>, freshness: &FrameFreshness<'_>) -> FrameOutcome {
        self(frame, freshness)
    }
}

// ---------------------------------------------------------------------------
// publisher
// ---------------------------------------------------------------------------

/// document thread와 render thread가 공유하는 상태.
struct Shared<S> {
    mailbox: LatestSlot<RenderFrame<S>>,
    /// 알려진 최신 scene 세대(0 = 아직 없음).
    latest: AtomicU64,
    /// 정지 요청.
    stop: AtomicBool,
}

/// publish 성공 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// 빈 mailbox에 넣었습니다.
    Delivered,
    /// 아직 렌더되지 않은 이전 frame을 대체했습니다. 이전 frame은 렌더 없이 버렸습니다.
    ReplacedPending {
        /// 버린 frame의 세대.
        stale: SceneRevision,
    },
}

/// publish 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishError {
    /// render thread가 종료 중이거나 종료됐습니다.
    Stopped,
    /// 세대 0은 "아직 없음"이라 frame에 쓸 수 없습니다(scene revision은 1부터, XI.1.3).
    ZeroRevision,
    /// 이미 알린 최신 세대보다 오래된 세대입니다.
    StaleRevision {
        /// 거부한 세대.
        revision: SceneRevision,
        /// 알려진 최신 세대.
        latest: SceneRevision,
    },
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublishError::Stopped => write!(f, "render thread가 종료됐습니다"),
            PublishError::ZeroRevision => write!(f, "scene 세대 0은 frame에 쓸 수 없습니다"),
            PublishError::StaleRevision { revision, latest } => write!(
                f,
                "scene 세대 {}는 이미 알린 최신 세대 {}보다 오래됐습니다",
                revision.0, latest.0
            ),
        }
    }
}

impl std::error::Error for PublishError {}

/// render thread에 frame을 넘기는 유일한 생산자 handle(document thread가 소유).
///
/// 복제할 수 없고 `publish`가 `&mut self`라 생산자는 언제나 하나입니다. 그래서 세대 단조 검사와
/// mailbox 교체 사이에 다른 생산자가 끼어들 수 없습니다. 모든 연산은 기다리지 않습니다.
///
/// # 사용
///
/// ```
/// use stellar_core_types::revision::SceneRevision;
/// use stellar_runtime::render_thread::{
///     FrameFreshness, FrameOutcome, PublishOutcome, RenderFrame, RenderThread,
/// };
///
/// let (render, mut publisher) = RenderThread::spawn(
///     |frame: &RenderFrame<u64>, freshness: &FrameFreshness<'_>| {
///         // GPU 명령을 만들고, 제출 직전에 세대를 확인합니다.
///         if freshness.is_stale() {
///             return FrameOutcome::Discarded;
///         }
///         let _ = frame.snapshot();
///         FrameOutcome::Presented
///     },
/// )
/// .unwrap();
/// assert_eq!(publisher.publish(SceneRevision(1), 42), Ok(PublishOutcome::Delivered));
/// render.shutdown().unwrap();
/// ```
///
/// # 생산자는 하나
///
/// 복제할 수 없습니다.
///
/// ```compile_fail
/// use stellar_runtime::render_thread::RenderPublisher;
///
/// fn duplicate(publisher: &RenderPublisher<u64>) -> RenderPublisher<u64> {
///     publisher.clone()
/// }
/// ```
///
/// 공유 참조로는 publish할 수 없습니다(`&mut self`).
///
/// ```compile_fail
/// use stellar_core_types::revision::SceneRevision;
/// use stellar_runtime::render_thread::RenderPublisher;
///
/// fn shared(publisher: &RenderPublisher<u64>) {
///     let _ = publisher.publish(SceneRevision(1), 1);
/// }
/// ```
///
/// frame snapshot의 가변 참조는 얻을 수 없습니다.
///
/// ```compile_fail
/// use stellar_runtime::render_thread::RenderFrame;
///
/// fn mutate(frame: &RenderFrame<u64>) {
///     *frame.snapshot() = 1;
/// }
/// ```
pub struct RenderPublisher<S> {
    shared: Arc<Shared<S>>,
    render: Thread,
}

impl<S> fmt::Debug for RenderPublisher<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderPublisher")
            .field("latest", &self.latest_revision())
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

impl<S: Send + 'static> RenderPublisher<S> {
    /// 새 frame을 render thread에 넘깁니다. 기다리지 않고 바로 돌아옵니다.
    ///
    /// 아직 렌더되지 않은 이전 frame은 렌더 없이 버리고([`PublishOutcome::ReplacedPending`]),
    /// 거부된 snapshot은 이 호출 안에서 버립니다.
    pub fn publish(
        &mut self,
        revision: SceneRevision,
        snapshot: S,
    ) -> Result<PublishOutcome, PublishError> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(PublishError::Stopped);
        }
        if revision.0 == 0 {
            return Err(PublishError::ZeroRevision);
        }
        let latest = self.shared.latest.load(Ordering::Acquire);
        if revision.0 < latest {
            return Err(PublishError::StaleRevision {
                revision,
                latest: SceneRevision(latest),
            });
        }
        // 세대를 먼저 알린 뒤 frame을 넣습니다. render thread가 이전 frame을 렌더하는 중이면
        // 제출 직전 판정에서 더 새 세대를 봅니다.
        self.shared.latest.store(revision.0, Ordering::Release);
        let replaced = self
            .shared
            .mailbox
            .replace(RenderFrame { revision, snapshot });
        self.render.unpark();
        Ok(match replaced {
            None => PublishOutcome::Delivered,
            // 이전 frame(snapshot 참조 포함)은 여기서 버립니다.
            Some(stale) => PublishOutcome::ReplacedPending {
                stale: stale.revision,
            },
        })
    }
}

impl<S> RenderPublisher<S> {
    /// 지금까지 알린 최신 세대(없으면 `None`).
    pub fn latest_revision(&self) -> Option<SceneRevision> {
        match self.shared.latest.load(Ordering::Acquire) {
            0 => None,
            value => Some(SceneRevision(value)),
        }
    }

    /// render thread가 아직 꺼내지 않은 frame이 있는지(진단용).
    pub fn has_pending(&self) -> bool {
        self.shared.mailbox.is_occupied()
    }

    /// render thread 정지가 요청됐는지.
    pub fn is_stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }
}

// ---------------------------------------------------------------------------
// thread
// ---------------------------------------------------------------------------

/// render loop 종료 통계.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderLoopStats {
    /// 제출한 frame 수.
    pub frames_presented: u64,
    /// 렌더 전에 더 새 세대를 확인해 버린 frame 수.
    pub frames_discarded_stale: u64,
    /// renderer가 제출 직전에 버린 frame 수.
    pub frames_discarded_by_renderer: u64,
}

/// render thread 생성·종료 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderThreadError {
    /// 운영체제 thread를 만들지 못했습니다.
    Spawn {
        /// 원인 종류.
        kind: io::ErrorKind,
    },
    /// thread 역할을 bind하지 못했습니다.
    RoleBind(ThreadRoleError),
    /// render thread 자신은 자기를 종료·join할 수 없습니다(deadlock).
    ShutdownFromRenderThread,
    /// render thread가 panic으로 끝났습니다.
    Panicked,
}

impl fmt::Display for RenderThreadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderThreadError::Spawn { kind } => {
                write!(f, "render thread를 만들지 못했습니다: {kind}")
            }
            RenderThreadError::RoleBind(error) => {
                write!(f, "render thread 역할 bind 실패: {error}")
            }
            RenderThreadError::ShutdownFromRenderThread => {
                write!(f, "render thread에서 자기 자신을 종료할 수 없습니다")
            }
            RenderThreadError::Panicked => write!(f, "render thread가 panic으로 끝났습니다"),
        }
    }
}

impl std::error::Error for RenderThreadError {}

/// 불변 scene snapshot을 읽어 GPU 명령을 만들고 제출하는 thread.
///
/// drop하면 [`RenderThread::shutdown`]과 같이 정지시키고 join합니다.
pub struct RenderThread<S> {
    shared: Arc<Shared<S>>,
    thread_id: ThreadId,
    join: Option<JoinHandle<Result<RenderLoopStats, RenderThreadError>>>,
}

impl<S> fmt::Debug for RenderThread<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderThread")
            .field("thread_id", &self.thread_id)
            .field("running", &self.join.is_some())
            .finish()
    }
}

impl<S: Send + 'static> RenderThread<S> {
    /// render thread를 시작하고, 그 thread에 frame을 넘기는 유일한 생산자 handle을 함께
    /// 돌려줍니다.
    pub fn spawn<R: FrameRenderer<S>>(
        renderer: R,
    ) -> Result<(RenderThread<S>, RenderPublisher<S>), RenderThreadError> {
        let shared = Arc::new(Shared {
            mailbox: LatestSlot::new(),
            latest: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let loop_shared = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name(RENDER_THREAD_NAME.to_string())
            .spawn(move || run_loop(&loop_shared, renderer))
            .map_err(|error| RenderThreadError::Spawn { kind: error.kind() })?;
        let render = join.thread().clone();
        let thread = RenderThread {
            shared: Arc::clone(&shared),
            thread_id: render.id(),
            join: Some(join),
        };
        Ok((thread, RenderPublisher { shared, render }))
    }
}

impl<S> RenderThread<S> {
    /// render thread의 id.
    pub fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    /// 정지시키고 join합니다. 대기 중이던 frame은 렌더하지 않습니다.
    ///
    /// render thread 자신(renderer 안)에서 호출하면 정지만 요청하고
    /// [`RenderThreadError::ShutdownFromRenderThread`]를 반환합니다.
    pub fn shutdown(mut self) -> Result<RenderLoopStats, RenderThreadError> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> Result<RenderLoopStats, RenderThreadError> {
        self.shared.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else {
            return Ok(RenderLoopStats::default());
        };
        join.thread().unpark();
        if thread::current().id() == self.thread_id {
            // 자기 자신을 join하면 deadlock입니다. handle을 버려 detach하고, loop는 정지 표시를
            // 보고 스스로 끝납니다.
            drop(join);
            return Err(RenderThreadError::ShutdownFromRenderThread);
        }
        // 대기 frame은 render thread가 끝난 뒤 공유 상태와 함께 버려집니다.
        match join.join() {
            Ok(result) => result,
            Err(_) => Err(RenderThreadError::Panicked),
        }
    }
}

impl<S> Drop for RenderThread<S> {
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.stop_and_join();
        }
    }
}

/// render loop가 어떤 경로로 끝나든(renderer panic 포함) 정지를 표시해, 이후 publish가
/// [`PublishError::Stopped`]를 받고 frame을 쌓아 두지 않게 합니다.
struct StopOnExit<'a> {
    stop: &'a AtomicBool,
}

impl Drop for StopOnExit<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// render loop: 새 frame이 오면 꺼내 세대를 확인하고 렌더합니다. 없으면 park합니다.
fn run_loop<S, R: FrameRenderer<S>>(
    shared: &Shared<S>,
    mut renderer: R,
) -> Result<RenderLoopStats, RenderThreadError> {
    let _stop_on_exit = StopOnExit { stop: &shared.stop };
    let _role = bind_thread_role(ThreadRole::Render).map_err(RenderThreadError::RoleBind)?;
    let mut stats = RenderLoopStats::default();
    // 마지막으로 제출한 frame. 다음 frame을 제출할 때까지 snapshot을 붙잡습니다.
    let mut presented: Option<RenderFrame<S>> = None;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let Some(frame) = shared.mailbox.take() else {
            // unpark가 먼저 와도 token이 남아 park가 바로 돌아오므로 깨움을 놓치지 않습니다.
            thread::park();
            continue;
        };
        let freshness = FrameFreshness {
            latest: &shared.latest,
            revision: frame.revision,
        };
        if freshness.is_stale() {
            stats.frames_discarded_stale = stats.frames_discarded_stale.saturating_add(1);
            continue;
        }
        match renderer.render(&frame, &freshness) {
            FrameOutcome::Presented => {
                stats.frames_presented = stats.frames_presented.saturating_add(1);
                presented = Some(frame);
            }
            FrameOutcome::Discarded => {
                stats.frames_discarded_by_renderer =
                    stats.frames_discarded_by_renderer.saturating_add(1);
            }
        }
    }
    drop(presented);
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use crate::document_thread::DocumentThread;

    /// drop 수를 세는 snapshot 대역(회수 확인용).
    #[derive(Debug)]
    struct Tracked {
        revision: u64,
        payload: Vec<u64>,
        drops: Arc<AtomicUsize>,
    }

    impl Tracked {
        fn new(revision: u64, drops: &Arc<AtomicUsize>) -> Arc<Tracked> {
            Arc::new(Tracked {
                revision,
                payload: (0..16).map(|i| revision.wrapping_mul(31) ^ i).collect(),
                drops: Arc::clone(drops),
            })
        }

        fn is_consistent(&self) -> bool {
            self.payload
                .iter()
                .enumerate()
                .all(|(i, value)| *value == self.revision.wrapping_mul(31) ^ i as u64)
        }
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// 조건이 참이 될 때까지 최대 5초 기다립니다.
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::yield_now();
        }
        condition()
    }

    fn rev(value: u64) -> SceneRevision {
        SceneRevision(value)
    }

    /// 제출한 세대를 기록하는 renderer.
    fn recording(log: &Arc<Mutex<Vec<u64>>>) -> impl FrameRenderer<Arc<Tracked>> {
        let log = Arc::clone(log);
        move |frame: &RenderFrame<Arc<Tracked>>, _: &FrameFreshness<'_>| {
            assert_eq!(frame.snapshot().revision, frame.revision().0);
            log.lock().unwrap().push(frame.revision().0);
            FrameOutcome::Presented
        }
    }

    #[test]
    fn handles_cross_threads_but_the_publisher_is_unique() {
        fn assert_send<T: Send>() {}
        assert_send::<RenderPublisher<Arc<Tracked>>>();
        assert_send::<RenderThread<Arc<Tracked>>>();
    }

    #[test]
    fn slot_keeps_only_the_latest_value_and_frees_the_rest() {
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = LatestSlot::new();
        assert!(slot.take().is_none());
        assert!(slot.replace(Tracked::new(1, &drops)).is_none());
        let replaced = slot.replace(Tracked::new(2, &drops)).expect("이전 값");
        assert_eq!(replaced.revision, 1);
        drop(replaced);
        assert_eq!(drops.load(Ordering::Acquire), 1);
        assert!(slot.is_occupied());
        assert_eq!(slot.take().map(|value| value.revision), Some(2));
        assert!(!slot.is_occupied());
        assert_eq!(drops.load(Ordering::Acquire), 2);
        // drop 시 남은 값을 해제합니다.
        assert!(slot.replace(Tracked::new(3, &drops)).is_none());
        drop(slot);
        assert_eq!(drops.load(Ordering::Acquire), 3);
    }

    #[test]
    fn frames_run_on_the_render_thread() {
        let (role_tx, role_rx) = mpsc::channel();
        let (render, mut publisher) =
            RenderThread::spawn(move |_frame: &RenderFrame<u32>, _: &FrameFreshness<'_>| {
                let current = thread::current();
                let _ = role_tx.send((
                    current_thread_role(),
                    current.name().map(str::to_owned),
                    on_render_thread(),
                    current.id(),
                ));
                FrameOutcome::Presented
            })
            .unwrap();
        assert!(!on_render_thread());
        assert_eq!(publisher.publish(rev(1), 7), Ok(PublishOutcome::Delivered));
        let (role, name, on_render, id) = role_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(role, Some(ThreadRole::Render));
        assert_eq!(name.as_deref(), Some(RENDER_THREAD_NAME));
        assert!(on_render);
        assert_eq!(id, render.thread_id());
        assert_eq!(render.shutdown().unwrap().frames_presented, 1);
    }

    #[test]
    fn document_thread_does_not_wait_for_render_submit() {
        // 검증 기준(XII.1.3): render thread가 GPU 제출 중(gate로 멈춤)이어도 document thread의
        // publish는 기다리지 않고, render thread는 풀려난 뒤 최신 frame만 받습니다.
        const PUBLISHED: u64 = 1_000;
        let drops = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel::<u64>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let log = Arc::new(Mutex::new(Vec::new()));
        let render_log = Arc::clone(&log);
        let (render, mut publisher) = RenderThread::spawn(
            move |frame: &RenderFrame<Arc<Tracked>>, _: &FrameFreshness<'_>| {
                let revision = frame.revision().0;
                if revision == 1 {
                    // 첫 frame의 "GPU 제출"이 오래 걸리는 상황.
                    entered_tx.send(revision).unwrap();
                    release_rx.recv().unwrap();
                }
                render_log.lock().unwrap().push(revision);
                FrameOutcome::Presented
            },
        )
        .unwrap();
        assert_eq!(
            publisher.publish(rev(1), Tracked::new(1, &drops)),
            Ok(PublishOutcome::Delivered)
        );
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)), Ok(1));

        // document thread job이 render thread가 멈춘 동안 나머지 세대를 모두 publish합니다.
        let doc = DocumentThread::spawn().unwrap();
        let job_drops = Arc::clone(&drops);
        let mut ticket = doc
            .sender()
            .submit(move |_ctx| {
                let mut outcomes = Vec::new();
                for revision in 2..=PUBLISHED {
                    outcomes.push(
                        publisher
                            .publish(rev(revision), Tracked::new(revision, &job_drops))
                            .unwrap(),
                    );
                }
                (publisher, outcomes)
            })
            .unwrap();
        // gate를 열기 전에 job이 끝나야 합니다(렌더 대기 없음).
        let (publisher, outcomes) = {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(result) = ticket.try_take().unwrap() {
                    break result;
                }
                assert!(
                    Instant::now() < deadline,
                    "document thread가 render 제출을 기다렸습니다"
                );
                thread::yield_now();
            }
        };
        assert_eq!(outcomes[0], PublishOutcome::Delivered);
        for (index, outcome) in outcomes[1..].iter().enumerate() {
            let stale = rev(index as u64 + 2);
            assert_eq!(*outcome, PublishOutcome::ReplacedPending { stale });
        }
        // 대체된 frame은 렌더 없이 즉시 회수됐습니다(대기 1개 + 렌더 중 1개만 생존).
        assert_eq!(
            drops.load(Ordering::Acquire),
            (PUBLISHED - 2) as usize,
            "대체된 snapshot이 회수되지 않았습니다"
        );
        assert!(publisher.has_pending());
        assert_eq!(publisher.latest_revision(), Some(rev(PUBLISHED)));

        release_tx.send(()).unwrap();
        assert!(wait_until(|| log.lock().unwrap().len() == 2));
        assert_eq!(*log.lock().unwrap(), [1, PUBLISHED]);
        let stats = render.shutdown().unwrap();
        assert_eq!(stats.frames_presented, 2);
        assert_eq!(stats.frames_discarded_stale, 0);
        doc.shutdown().unwrap();
        drop(publisher);
        assert_eq!(drops.load(Ordering::Acquire), PUBLISHED as usize);
    }

    #[test]
    fn renderer_discards_a_frame_that_became_stale_before_submit() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let log = Arc::new(Mutex::new(Vec::new()));
        let render_log = Arc::clone(&log);
        let (render, mut publisher) = RenderThread::spawn(
            move |frame: &RenderFrame<Arc<Tracked>>, freshness: &FrameFreshness<'_>| {
                if frame.revision().0 == 1 {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                // 제출 직전 판정.
                if freshness.is_stale() {
                    assert!(freshness.latest() > freshness.revision());
                    return FrameOutcome::Discarded;
                }
                render_log.lock().unwrap().push(frame.revision().0);
                FrameOutcome::Presented
            },
        )
        .unwrap();
        publisher.publish(rev(1), Tracked::new(1, &drops)).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        publisher.publish(rev(2), Tracked::new(2, &drops)).unwrap();
        release_tx.send(()).unwrap();
        assert!(wait_until(|| log.lock().unwrap().len() == 1));
        assert_eq!(*log.lock().unwrap(), [2]);
        let stats = render.shutdown().unwrap();
        assert_eq!(
            stats,
            RenderLoopStats {
                frames_presented: 1,
                frames_discarded_stale: 0,
                frames_discarded_by_renderer: 1,
            }
        );
    }

    #[test]
    fn publish_rejects_zero_and_regressed_revisions() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let (render, mut publisher) = RenderThread::spawn(recording(&log)).unwrap();
        assert_eq!(publisher.latest_revision(), None);
        assert_eq!(
            publisher.publish(rev(0), Tracked::new(0, &drops)),
            Err(PublishError::ZeroRevision)
        );
        publisher.publish(rev(5), Tracked::new(5, &drops)).unwrap();
        assert_eq!(
            publisher.publish(rev(4), Tracked::new(4, &drops)),
            Err(PublishError::StaleRevision {
                revision: rev(4),
                latest: rev(5),
            })
        );
        // 거부된 snapshot은 호출 안에서 버려졌습니다.
        assert_eq!(drops.load(Ordering::Acquire), 2);
        // 같은 세대 재전달은 허용합니다(XI.1.6: 같거나 크면 제출 대상).
        assert!(wait_until(|| !publisher.has_pending()));
        publisher.publish(rev(5), Tracked::new(5, &drops)).unwrap();
        assert!(wait_until(|| log.lock().unwrap().len() == 2));
        assert_eq!(*log.lock().unwrap(), [5, 5]);
        render.shutdown().unwrap();
    }

    #[test]
    fn shutdown_drops_the_pending_frame_and_rejects_later_publishes() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (render, mut publisher) = RenderThread::spawn(
            move |_frame: &RenderFrame<Arc<Tracked>>, _: &FrameFreshness<'_>| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                FrameOutcome::Presented
            },
        )
        .unwrap();
        publisher.publish(rev(1), Tracked::new(1, &drops)).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        publisher.publish(rev(2), Tracked::new(2, &drops)).unwrap();
        let stopper = thread::spawn(move || render.shutdown());
        assert!(wait_until(|| publisher.is_stopped()));
        assert_eq!(
            publisher.publish(rev(3), Tracked::new(3, &drops)),
            Err(PublishError::Stopped)
        );
        release_tx.send(()).unwrap();
        let stats = stopper.join().unwrap().unwrap();
        // 렌더 중이던 1만 제출, 대기 중이던 2는 렌더하지 않고 버립니다.
        assert_eq!(stats.frames_presented, 1);
        drop(publisher);
        assert_eq!(drops.load(Ordering::Acquire), 3, "모든 snapshot 회수");
    }

    #[test]
    fn renderer_panic_never_blocks_the_publisher() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (render, mut publisher) = RenderThread::spawn(
            |_frame: &RenderFrame<Arc<Tracked>>, _: &FrameFreshness<'_>| -> FrameOutcome {
                panic!("의도된 renderer panic(테스트)")
            },
        )
        .unwrap();
        publisher.publish(rev(1), Tracked::new(1, &drops)).unwrap();
        // panic으로 끝난 render thread는 정지를 표시합니다. publish는 기다리지 않고 바로
        // `Stopped`를 받으며 snapshot을 쌓아 두지 않습니다.
        assert!(wait_until(|| publisher.is_stopped()));
        for revision in 2..=100 {
            assert_eq!(
                publisher.publish(rev(revision), Tracked::new(revision, &drops)),
                Err(PublishError::Stopped)
            );
        }
        assert!(!publisher.has_pending());
        assert_eq!(drops.load(Ordering::Acquire), 100);
        assert_eq!(render.shutdown(), Err(RenderThreadError::Panicked));
        drop(publisher);
        assert_eq!(drops.load(Ordering::Acquire), 100);
    }

    #[test]
    fn concurrent_handoff_delivers_consistent_monotonic_frames() {
        // 동시성 검사: 생산자가 10,000 frame을 쉬지 않고 넘기는 동안 render thread가 lock 없이
        // 꺼내 읽습니다. 받은 snapshot은 항상 온전하고(내용 = 세대), 제출 세대는 엄격 증가하며,
        // 마지막 세대는 반드시 제출되고, 끝나면 모든 snapshot이 회수됩니다.
        const FRAMES: u64 = 10_000;
        let drops = Arc::new(AtomicUsize::new(0));
        let presented = Arc::new(Mutex::new(Vec::new()));
        let render_presented = Arc::clone(&presented);
        let (render, mut publisher) = RenderThread::spawn(
            move |frame: &RenderFrame<Arc<Tracked>>, _: &FrameFreshness<'_>| {
                let snapshot = frame.snapshot();
                assert!(snapshot.is_consistent(), "깨진 snapshot을 읽음");
                assert_eq!(snapshot.revision, frame.revision().0);
                render_presented.lock().unwrap().push(frame.revision().0);
                FrameOutcome::Presented
            },
        )
        .unwrap();
        let producer_drops = Arc::clone(&drops);
        let producer = thread::spawn(move || {
            for revision in 1..=FRAMES {
                publisher
                    .publish(rev(revision), Tracked::new(revision, &producer_drops))
                    .unwrap();
            }
            publisher
        });
        let publisher = producer.join().unwrap();
        assert!(wait_until(|| {
            presented.lock().unwrap().last().copied() == Some(FRAMES)
        }));
        let stats = render.shutdown().unwrap();
        let presented = presented.lock().unwrap().clone();
        assert!(presented.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(stats.frames_presented, presented.len() as u64);
        drop(publisher);
        assert_eq!(drops.load(Ordering::Acquire), FRAMES as usize);
    }

    #[test]
    fn errors_display_in_korean() {
        assert!(PublishError::Stopped.to_string().contains("종료"));
        assert!(PublishError::ZeroRevision.to_string().contains("0"));
        assert!(PublishError::StaleRevision {
            revision: rev(1),
            latest: rev(2),
        }
        .to_string()
        .contains("오래"));
        assert!(RenderThreadError::Panicked.to_string().contains("panic"));
        assert!(RenderThreadError::ShutdownFromRenderThread
            .to_string()
            .contains("자기 자신"));
    }
}
