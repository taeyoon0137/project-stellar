//! 앱 입력 경로 연결 지점 trace probe (XVI.1.1, XVI.1.2, XVI.1.3, XVI.1.5, ADR-0096 결정 1·2).
//!
//! 앱 입력 경로(AppKit 입력 → DocumentThread → render thread)는 UI 재설계 때 연결합니다. 그때 호출할 API를 지금
//! 정해 두고 core replay·테스트가 같은 API로 trace를 만듭니다.
//!
//! - **T0**: [`InteractionProbe::begin`]은 event 하나마다 새 [`InteractionTraceId`]를 발급하고 지금 시각(Stellar가
//!   event를 받은 시각)을 T0로 기록합니다. 운영체제 event timestamp는 쓰지 않습니다(결정 1).
//! - **병합**: 이벤트 병합(T0→T1)으로 합쳐진 event는 [`InteractionProbe::coalesce`]로 살아남은 event에 붙입니다.
//!   붙은 event도 자기 trace ID·T0를 유지하고, T1~T5는 살아남은 event의 시각을 받습니다. 그래서 각 event의 latency는
//!   자기 수신부터 그 event를 반영한 frame까지입니다. 살아남은 event가 T1을 지난 뒤에는 붙일 수 없습니다.
//! - **stage**: [`InteractionProbe::mark`]는 probe 시계로, [`InteractionProbe::mark_at`]은 다른 스레드가
//!   [`Stamper`]로 찍은 [`ProbeStamp`]로 T1~T5를 기록합니다. [`InteractionProbe::finish`]가 validator(XVI.1.2)를
//!   통과한 완료 trace를 ring buffer store(XVI.1.5)에 넣습니다.
//! - **clock domain**(XVI.1.3): CPU stage 시각은 [`ProbeStamp`]로만 받습니다. 이 값은 probe 시계([`Stamper`])만 만들 수
//!   있어 GPU timestamp나 운영체제 event 시각을 stage에 넣을 수 없습니다(타입으로 강제). GPU 실행 시간은
//!   [`InteractionProbe::set_gpu`]로 따로 기록하고 Logical Interaction Latency에 더하지 않습니다.
//! - **memory bound**: 진행 중 trace는 [`MAX_IN_FLIGHT_TRACES`]개까지입니다. 넘으면 가장 오래된 진행 중 trace를
//!   버리고 [`ProbeCounters::abandoned`]에 셉니다. 완료 trace는 고정 용량 [`TraceStore`]에 들어갑니다.
//! - 취소된 event([`InteractionProbe::cancel`], 예: frame 없이 취소된 제스처)는 frame이 없어 latency가 없으므로
//!   sample이 아니고 [`ProbeCounters::cancelled`]에만 셉니다.

use std::collections::VecDeque;
use std::fmt;

use stellar_core_types::clock::MonotonicClock;

use crate::interaction::{
    CompletedTrace, GpuTiming, InteractionTrace, InteractionTraceId, InteractionTraceIds, Stage,
    TraceDimensions, TraceError,
};
use crate::trace_store::TraceStore;

/// 동시에 진행 중일 수 있는 trace 수 상한.
pub const MAX_IN_FLIGHT_TRACES: usize = 256;

/// probe 시계 시각(CPU monotonic domain, ns). [`Stamper`]만 만들 수 있습니다.
///
/// ```compile_fail
/// use stellar_diagnostics::latency_probe::ProbeStamp;
/// // GPU·운영체제 시각을 stage 시각으로 바꿀 수 없습니다.
/// let _ = ProbeStamp(42);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeStamp(u64);

impl ProbeStamp {
    /// ns 값.
    pub const fn nanos(self) -> u64 {
        self.0
    }
}

/// probe 시계로 시각을 찍는 handle. 시계가 `Clone + Send`이면 다른 스레드에 넘길 수 있습니다.
#[derive(Debug, Clone)]
pub struct Stamper<C> {
    clock: C,
}

impl<C: MonotonicClock> Stamper<C> {
    /// 지금 시각.
    pub fn stamp(&self) -> ProbeStamp {
        ProbeStamp(self.clock.now_nanos())
    }
}

/// probe 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// 진행 중인 trace가 아닙니다(완료·취소·버림·미발급).
    UnknownTrace {
        /// ID.
        id: InteractionTraceId,
    },
    /// 이미 T1을 지난 trace에는 event를 병합할 수 없습니다.
    CoalesceAfterInput {
        /// 살아남은 trace.
        into: InteractionTraceId,
    },
    /// 자기 자신 또는 이미 병합된 trace에 병합하려 했습니다.
    InvalidCoalesce {
        /// 병합하려 한 trace.
        merged: InteractionTraceId,
    },
    /// stage 기록 오류.
    Trace(TraceError),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::UnknownTrace { id } => {
                write!(f, "진행 중인 trace {}가 없습니다", id.get())
            }
            ProbeError::CoalesceAfterInput { into } => write!(
                f,
                "trace {}는 이미 T1을 지나 event를 병합할 수 없습니다",
                into.get()
            ),
            ProbeError::InvalidCoalesce { merged } => {
                write!(f, "trace {}를 병합할 수 없습니다", merged.get())
            }
            ProbeError::Trace(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<TraceError> for ProbeError {
    fn from(error: TraceError) -> ProbeError {
        ProbeError::Trace(error)
    }
}

/// probe 집계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProbeCounters {
    /// 발급한 trace 수(event 수).
    pub begun: u64,
    /// store에 넣은 완료 trace 수(병합된 event 포함).
    pub completed: u64,
    /// 다른 event에 병합된 event 수.
    pub coalesced: u64,
    /// frame 없이 취소된 event 수(sample 아님).
    pub cancelled: u64,
    /// 진행 중 상한을 넘어 버린 trace 수.
    pub abandoned: u64,
}

/// 진행 중 trace 하나와 거기에 병합된 event.
#[derive(Debug, Clone)]
struct InFlight {
    trace: InteractionTrace,
    merged: Vec<InteractionTrace>,
}

/// 앱 입력 경로 연결 지점(ADR-0096 결정 2).
#[derive(Debug, Clone)]
pub struct InteractionProbe<C> {
    stamper: Stamper<C>,
    ids: InteractionTraceIds,
    open: VecDeque<InFlight>,
    store: TraceStore,
    counters: ProbeCounters,
}

impl<C: MonotonicClock + Clone> InteractionProbe<C> {
    /// `clock`으로 시각을 찍고 `store`에 완료 trace를 넣는 probe.
    pub fn new(clock: C, store: TraceStore) -> InteractionProbe<C> {
        InteractionProbe {
            stamper: Stamper { clock },
            ids: InteractionTraceIds::new(),
            open: VecDeque::new(),
            store,
            counters: ProbeCounters::default(),
        }
    }

    /// 다른 스레드에서 stage 시각을 찍을 handle.
    pub fn stamper(&self) -> Stamper<C> {
        self.stamper.clone()
    }

    /// 지금 시각.
    pub fn now(&self) -> ProbeStamp {
        self.stamper.stamp()
    }

    /// event 하나를 받았습니다. 새 trace ID를 발급하고 지금을 T0로 기록합니다(XVI.1.1).
    pub fn begin(&mut self, dimensions: TraceDimensions) -> InteractionTraceId {
        let received = self.now();
        self.begin_at(dimensions, received)
    }

    /// [`InteractionProbe::begin`]과 같고, 수신 스레드가 찍어 둔 시각을 T0로 씁니다.
    pub fn begin_at(
        &mut self,
        dimensions: TraceDimensions,
        received: ProbeStamp,
    ) -> InteractionTraceId {
        let id = self.ids.issue();
        if self.open.len() == MAX_IN_FLIGHT_TRACES {
            if let Some(oldest) = self.open.pop_front() {
                self.counters.abandoned = self
                    .counters
                    .abandoned
                    .saturating_add(1 + oldest.merged.len() as u64);
            }
        }
        self.open.push_back(InFlight {
            trace: InteractionTrace::begin(id, dimensions, received.0),
            merged: Vec::new(),
        });
        self.counters.begun = self.counters.begun.saturating_add(1);
        id
    }

    fn position(&self, id: InteractionTraceId) -> Result<usize, ProbeError> {
        self.open
            .iter()
            .position(|entry| entry.trace.id() == id)
            .ok_or(ProbeError::UnknownTrace { id })
    }

    /// 이벤트 병합: `merged` event를 `into` event에 붙입니다. 둘 다 T1 전이어야 합니다.
    pub fn coalesce(
        &mut self,
        merged: InteractionTraceId,
        into: InteractionTraceId,
    ) -> Result<(), ProbeError> {
        if merged == into {
            return Err(ProbeError::InvalidCoalesce { merged });
        }
        let into_at = self.position(into)?;
        if self.open[into_at].trace.stamp(Stage::T1).is_some() {
            return Err(ProbeError::CoalesceAfterInput { into });
        }
        let merged_at = self.position(merged)?;
        if self.open[merged_at].trace.stamp(Stage::T1).is_some() {
            return Err(ProbeError::InvalidCoalesce { merged });
        }
        let entry = self
            .open
            .remove(merged_at)
            .ok_or(ProbeError::UnknownTrace { id: merged })?;
        let into_at = self.position(into)?;
        let target = &mut self.open[into_at];
        target.merged.push(entry.trace);
        target.merged.extend(entry.merged);
        self.counters.coalesced = self.counters.coalesced.saturating_add(1);
        Ok(())
    }

    /// stage를 지금 시각으로 기록합니다(XVI.1.2).
    pub fn mark(&mut self, id: InteractionTraceId, stage: Stage) -> Result<(), ProbeError> {
        let at = self.now();
        self.mark_at(id, stage, at)
    }

    /// stage를 다른 스레드가 찍은 시각으로 기록합니다(XVI.1.2). 한 stage는 한 번, 시각은 감소하지 않아야 합니다.
    pub fn mark_at(
        &mut self,
        id: InteractionTraceId,
        stage: Stage,
        at: ProbeStamp,
    ) -> Result<(), ProbeError> {
        let index = self.position(id)?;
        self.open[index].trace.mark(stage, at.0)?;
        Ok(())
    }

    /// GPU 실행 시간(GPU clock domain)을 기록합니다. Logical Interaction Latency에 더하지 않습니다(XVI.1.3).
    pub fn set_gpu(&mut self, id: InteractionTraceId, gpu: GpuTiming) -> Result<(), ProbeError> {
        let index = self.position(id)?;
        self.open[index].trace.set_gpu(gpu)?;
        Ok(())
    }

    /// present 대기열 깊이(frame)를 기록합니다.
    pub fn set_queue_depth(
        &mut self,
        id: InteractionTraceId,
        depth: u32,
    ) -> Result<(), ProbeError> {
        let index = self.position(id)?;
        self.open[index].trace.set_queue_depth(depth);
        Ok(())
    }

    /// trace를 끝냅니다. validator(XVI.1.2)가 빠진 stage를 오류로 돌려주고, 그때 trace는 진행 중으로 남습니다.
    /// 통과하면 이 trace와 병합된 event의 trace를 store에 넣고 이 trace를 돌려줍니다.
    pub fn finish(&mut self, id: InteractionTraceId) -> Result<CompletedTrace, ProbeError> {
        let index = self.position(id)?;
        let done = self.open[index].trace.validate()?;
        let mut followers = Vec::with_capacity(self.open[index].merged.len());
        for merged in &self.open[index].merged {
            let mut trace = merged.clone();
            for stage in &Stage::ALL[1..] {
                trace.mark(*stage, done.stamp(*stage))?;
            }
            if let Some(gpu) = self.open[index].trace.gpu() {
                trace.set_gpu(gpu)?;
            }
            if let Some(depth) = done.queue_depth() {
                trace.set_queue_depth(depth);
            }
            followers.push(trace.validate()?);
        }
        self.open.remove(index);
        for trace in followers.into_iter().chain(std::iter::once(done)) {
            self.store.push(trace);
            self.counters.completed = self.counters.completed.saturating_add(1);
        }
        Ok(done)
    }

    /// frame 없이 끝난 event를 닫습니다(sample 아님).
    pub fn cancel(&mut self, id: InteractionTraceId) -> Result<(), ProbeError> {
        let index = self.position(id)?;
        if let Some(entry) = self.open.remove(index) {
            self.counters.cancelled = self
                .counters
                .cancelled
                .saturating_add(1 + entry.merged.len() as u64);
        }
        Ok(())
    }

    /// 진행 중 trace 수.
    pub fn in_flight(&self) -> usize {
        self.open.len()
    }

    /// 집계.
    pub const fn counters(&self) -> ProbeCounters {
        self.counters
    }

    /// 완료 trace store.
    pub const fn store(&self) -> &TraceStore {
        &self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{CacheState, DocumentSizeBucket, InteractionKind, VisibleNodeBucket};
    use std::cell::Cell;
    use std::rc::Rc;

    /// 테스트에서 앞으로 돌릴 수 있는 공유 시계.
    #[derive(Debug, Clone, Default)]
    struct TestClock(Rc<Cell<u64>>);

    impl TestClock {
        fn advance(&self, nanos: u64) {
            self.0.set(self.0.get() + nanos);
        }
    }

    impl MonotonicClock for TestClock {
        fn now_nanos(&self) -> u64 {
            self.0.get()
        }
    }

    fn dims() -> TraceDimensions {
        TraceDimensions {
            kind: InteractionKind::Drag,
            document: DocumentSizeBucket::UpTo10k,
            visible: VisibleNodeBucket::UpTo5k,
            cache: CacheState::Warm,
        }
    }

    fn probe(clock: &TestClock) -> InteractionProbe<TestClock> {
        InteractionProbe::new(clock.clone(), TraceStore::new(64))
    }

    #[test]
    fn begin_issues_one_id_per_event_with_receipt_t0() {
        // 검증 기준(XVI.1.1): 한 event당 trace ID 하나, T0 = 수신 시각.
        let clock = TestClock::default();
        let mut probe = probe(&clock);
        let mut ids = Vec::new();
        for _ in 0..4 {
            clock.advance(1_000);
            ids.push(probe.begin(dims()));
        }
        let unique: std::collections::BTreeSet<u64> = ids.iter().map(|id| id.get()).collect();
        assert_eq!(unique.len(), 4);
        assert_eq!(probe.counters().begun, 4);
        assert_eq!(probe.in_flight(), 4);
        // 수신 스레드가 찍은 시각을 T0로 쓸 수 있습니다.
        let stamper = probe.stamper();
        let received = stamper.stamp();
        clock.advance(500);
        let late = probe.begin_at(dims(), received);
        for stage in &Stage::ALL[1..] {
            probe.mark(late, *stage).unwrap();
        }
        let done = probe.finish(late).unwrap();
        assert_eq!(done.stamp(Stage::T0), 4_000);
        assert_eq!(done.logical_latency_nanos(), 500);
    }

    #[test]
    fn coalesced_events_keep_their_own_id_and_t0() {
        // 이벤트 병합: 붙은 event도 자기 trace를 갖고 latency는 자기 수신부터 셉니다.
        let clock = TestClock::default();
        let mut probe = probe(&clock);
        let first = probe.begin(dims());
        clock.advance(2_000);
        let second = probe.begin(dims());
        probe.coalesce(second, first).unwrap();
        assert_eq!(probe.in_flight(), 1);
        assert_eq!(
            probe.mark(second, Stage::T1),
            Err(ProbeError::UnknownTrace { id: second })
        );
        clock.advance(1_000);
        probe.mark(first, Stage::T1).unwrap();
        // T1 뒤에는 병합할 수 없습니다.
        let third = probe.begin(dims());
        assert_eq!(
            probe.coalesce(third, first),
            Err(ProbeError::CoalesceAfterInput { into: first })
        );
        assert_eq!(
            probe.coalesce(third, third),
            Err(ProbeError::InvalidCoalesce { merged: third })
        );
        for stage in [Stage::T2, Stage::T3, Stage::T4, Stage::T5] {
            clock.advance(1_000);
            probe.mark(first, stage).unwrap();
        }
        let done = probe.finish(first).unwrap();
        assert_eq!(done.logical_latency_nanos(), 7_000);
        let stored: Vec<(u64, u64)> = probe
            .store()
            .iter()
            .map(|t| (t.id().get(), t.logical_latency_nanos()))
            .collect();
        assert_eq!(stored, vec![(second.get(), 5_000), (first.get(), 7_000)]);
        assert_eq!(probe.counters().coalesced, 1);
        assert_eq!(probe.counters().completed, 2);
    }

    #[test]
    fn stages_are_recorded_once_in_order_and_validated() {
        // 검증 기준(XVI.1.2): 누락 stage를 오류로 표시하는 validator. 실패한 finish는 trace를 진행 중으로 둡니다.
        let clock = TestClock::default();
        let mut probe = probe(&clock);
        let id = probe.begin(dims());
        clock.advance(10);
        probe.mark(id, Stage::T1).unwrap();
        clock.advance(10);
        probe.mark(id, Stage::T2).unwrap();
        assert_eq!(
            probe.finish(id),
            Err(ProbeError::Trace(TraceError::MissingStage {
                stage: Stage::T3
            }))
        );
        assert_eq!(probe.in_flight(), 1);
        assert!(probe.store().is_empty());
        assert_eq!(
            probe.mark(id, Stage::T4),
            Err(ProbeError::Trace(TraceError::SkippedStage {
                missing: Stage::T3,
                stage: Stage::T4
            }))
        );
        assert_eq!(
            probe.mark(id, Stage::T2),
            Err(ProbeError::Trace(TraceError::Duplicate {
                stage: Stage::T2
            }))
        );
        // 다른 스레드가 찍은 시각은 감소하면 거부합니다.
        let early = ProbeStamp(5);
        assert!(matches!(
            probe.mark_at(id, Stage::T3, early),
            Err(ProbeError::Trace(TraceError::OutOfOrder { .. }))
        ));
        let stamper = probe.stamper();
        for stage in [Stage::T3, Stage::T4, Stage::T5] {
            clock.advance(10);
            probe.mark_at(id, stage, stamper.stamp()).unwrap();
        }
        probe.set_queue_depth(id, 1).unwrap();
        let done = probe.finish(id).unwrap();
        assert_eq!(done.logical_latency_nanos(), 50);
        assert_eq!(done.queue_depth(), Some(1));
        assert_eq!(probe.in_flight(), 0);
        assert_eq!(probe.store().len(), 1);
        // 끝낸 trace는 더 기록할 수 없습니다.
        assert_eq!(
            probe.mark(id, Stage::T1),
            Err(ProbeError::UnknownTrace { id })
        );
        // frame 없이 취소된 event는 sample이 아닙니다.
        let cancelled = probe.begin(dims());
        probe.cancel(cancelled).unwrap();
        assert_eq!(probe.counters().cancelled, 1);
        assert_eq!(probe.store().len(), 1);
    }
}
