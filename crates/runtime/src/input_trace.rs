//! 입력 지연 trace: T0 수신 시각과 DocumentThread hit-test request까지의 queue latency (XII.3.5).
//!
//! 아키텍처 §6.1 측정 구간 중 T0(운영체제 입력 이벤트를 Stellar가 수신)와 T1(이벤트 병합·좌표
//! 변환·Hit Test 완료)의 **후보** 시각을 입력 하나마다 남깁니다. 어느 후보를 T0/T1로 확정할지와
//! 입력별 trace ID는 계측 단계(XVI.1)가 정합니다.
//!
//! | 필드 | 찍는 곳 | 의미 | 후보 |
//! |---|---|---|---|
//! | [`InputReceipt::event_time`] | 셸 변환 | 운영체제 이벤트 시각(native timestamp를 코어 단조 domain으로 옮긴 값) | T0 |
//! | [`InputReceipt::received`] | 코어 경계(FFI) | 코어가 이벤트를 받은 시각 | T0 |
//! | [`InputLatencyTrace::enqueued`] | 접수 thread | document thread input lane에 hit-test request를 접수한 시각 | |
//! | [`InputLatencyTrace::started`] | document thread | request를 꺼내 hit test를 시작한 시각 | T1 |
//! | [`InputLatencyTrace::finished`] | document thread | hit test가 끝난 시각 | T1 |
//!
//! 모든 시각은 같은 [`MonotonicClock`] domain의 nanoseconds입니다. 수신 쪽과 document thread가 같은
//! 시계를 읽어야 차이가 의미를 가지므로, 시계는 호출자가 [`TraceClock`]으로 넘깁니다(앱은 FFI의
//! 프로세스 단조 시계, 테스트는 수동 시계).
//!
//! # 병합
//!
//! 셸→코어 입력 대기열(XII.3.4)이 연속 move·wheel을 합치면 receipt는 그 묶음에서 **가장 이른** 값을
//! 유지하고([`InputReceipt::absorb`]), [`InputReceipt::merged`]에 합쳐진 이벤트 수를 셉니다. 아직
//! 반영되지 않은 가장 오래된 입력을 기준으로 지연을 재기 위해서입니다. document thread 수신함
//! (XII.1.2)에서 같은 병합 key의 더 새 request로 대체된 request는 실행되지 않으므로
//! ([`JobError::Superseded`](crate::document_thread::JobError::Superseded)) trace를 남기지 않습니다.
//!
//! # 내용 없음
//!
//! trace에는 시각, 입력 분류([`InputEventClass`]), 병합 수만 담습니다. 위치·key·text 같은 입력 내용은
//! 담지 않습니다.

use std::sync::Arc;

use stellar_core_types::clock::MonotonicClock;

use crate::command_queue::InputEventClass;
use crate::document_thread::{DocumentContext, DocumentSender, SubmitError, Submitted};

/// 수신 쪽과 document thread가 함께 읽는 단조 시계.
pub type TraceClock = Arc<dyn MonotonicClock + Send + Sync>;

/// 입력 하나의 T0 후보(수신 기록). 병합된 항목은 묶음에서 가장 이른 값을 가집니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputReceipt {
    event_time: u64,
    received: u64,
    merged: u32,
}

impl InputReceipt {
    /// 이벤트 시각과 수신 시각으로 receipt를 만듭니다(병합 0).
    pub const fn new(event_time: u64, received: u64) -> InputReceipt {
        InputReceipt {
            event_time,
            received,
            merged: 0,
        }
    }

    /// 지금(`clock`)을 수신 시각으로 receipt를 만듭니다.
    pub fn stamp(clock: &dyn MonotonicClock, event_time: u64) -> InputReceipt {
        InputReceipt::new(event_time, clock.now_nanos())
    }

    /// T0 후보: 운영체제 이벤트 시각(코어 단조 domain).
    pub const fn event_time(&self) -> u64 {
        self.event_time
    }

    /// T0 후보: 코어 경계가 이벤트를 받은 시각.
    pub const fn received(&self) -> u64 {
        self.received
    }

    /// 이 항목에 합쳐진 뒤 이벤트 수.
    pub const fn merged(&self) -> u32 {
        self.merged
    }

    /// 뒤에 온 이벤트 `later`를 이 항목에 합친 receipt: 시각은 더 이른 값, 병합 수는 `later`와 그
    /// 병합 수를 더합니다(포화).
    pub fn absorb(self, later: InputReceipt) -> InputReceipt {
        InputReceipt {
            event_time: self.event_time.min(later.event_time),
            received: self.received.min(later.received),
            merged: self.merged.saturating_add(later.merged).saturating_add(1),
        }
    }
}

/// 입력 하나의 T0 → hit-test request 지연 trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputLatencyTrace {
    /// 입력 분류(접수 옵션과 같음).
    pub class: InputEventClass,
    /// T0 후보.
    pub receipt: InputReceipt,
    /// input lane 접수 시각.
    pub enqueued: u64,
    /// T1 후보: document thread가 hit test를 시작한 시각.
    pub started: u64,
    /// T1 후보: hit test가 끝난 시각.
    pub finished: u64,
}

impl InputLatencyTrace {
    /// T0 후보 `[운영체제 이벤트 시각, 코어 수신 시각]`.
    pub const fn t0_candidates(&self) -> [u64; 2] {
        [self.receipt.event_time, self.receipt.received]
    }

    /// T1 후보 `[hit test 시작, hit test 끝]`.
    pub const fn t1_candidates(&self) -> [u64; 2] {
        [self.started, self.finished]
    }

    /// 코어 수신(T0) → document thread hit-test request 시작까지(queue latency).
    pub const fn queue_latency_nanos(&self) -> u64 {
        self.started.saturating_sub(self.receipt.received)
    }

    /// input lane 대기 시간(접수 → 시작).
    pub const fn queue_wait_nanos(&self) -> u64 {
        self.started.saturating_sub(self.enqueued)
    }

    /// hit test 실행 시간(시작 → 끝).
    pub const fn hit_test_nanos(&self) -> u64 {
        self.finished.saturating_sub(self.started)
    }
}

/// hit-test request 결과: hit test 값과 그 trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitTestResponse<T> {
    /// hit test가 돌려준 값.
    pub value: T,
    /// 지연 trace.
    pub trace: InputLatencyTrace,
}

impl DocumentSender {
    /// 입력 하나의 hit-test request를 input lane에 접수합니다(XII.3.5).
    ///
    /// 접수 직전 `clock`으로 [`InputLatencyTrace::enqueued`]를 찍고, document thread가 request를
    /// 꺼내면 `hit_test` 앞뒤로 [`InputLatencyTrace::started`]·[`InputLatencyTrace::finished`]를
    /// 찍습니다. 우선순위·병합은 `class`의 접수 옵션([`InputEventClass::submit_options`])이며,
    /// 접수 판정은 [`DocumentSender::try_submit`]과 같습니다(기다리지 않음).
    pub fn submit_hit_test<F, T>(
        &self,
        class: InputEventClass,
        receipt: InputReceipt,
        clock: &TraceClock,
        hit_test: F,
    ) -> Result<Submitted<HitTestResponse<T>>, SubmitError>
    where
        F: FnOnce(&mut DocumentContext) -> T + Send + 'static,
        T: Send + 'static,
    {
        let clock = Arc::clone(clock);
        let enqueued = clock.now_nanos();
        self.try_submit(class.submit_options(), move |ctx| {
            let started = clock.now_nanos();
            let value = hit_test(ctx);
            let finished = clock.now_nanos();
            HitTestResponse {
                value,
                trace: InputLatencyTrace {
                    class,
                    receipt,
                    enqueued,
                    started,
                    finished,
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    use crate::command_queue::{QueueCapacity, SubmitOptions};
    use crate::document_thread::{DocumentThread, JobError, SubmitOutcome, TicketError};

    /// thread 사이에서 공유하는 수동 시계. 테스트가 시각을 직접 정합니다.
    #[derive(Debug, Default)]
    struct SharedClock(AtomicU64);

    impl SharedClock {
        fn set(&self, nanos: u64) {
            self.0.store(nanos, Ordering::SeqCst);
        }
    }

    impl MonotonicClock for SharedClock {
        fn now_nanos(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn clocks() -> (Arc<SharedClock>, TraceClock) {
        let clock = Arc::new(SharedClock::default());
        let trace: TraceClock = clock.clone();
        (clock, trace)
    }

    /// document thread를 job 실행 중으로 막아 두고, 실행이 시작된 뒤 해제 handle을 돌려줍니다.
    fn block(sender: &DocumentSender) -> mpsc::Sender<()> {
        let (release, wait) = mpsc::channel::<()>();
        let (running, started) = mpsc::channel::<()>();
        sender
            .submit(move |_ctx| {
                let _ = running.send(());
                let _ = wait.recv();
            })
            .unwrap();
        started.recv().unwrap();
        release
    }

    #[test]
    fn trace_has_t0_and_t1_candidates_in_order() {
        // 검증 기준(XII.3.5): trace에 T0/T1 후보 timestamp가 있고, T0 → hit-test request까지
        // queue latency를 잽니다.
        let (clock, trace_clock) = clocks();
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let release = block(&sender);

        // 운영체제 이벤트 시각 90, 코어 수신 100, input lane 접수 150.
        clock.set(100);
        let receipt = InputReceipt::stamp(&*clock, 90);
        clock.set(150);
        let hit_clock = Arc::clone(&clock);
        let submitted = sender
            .submit_hit_test(
                InputEventClass::Discrete,
                receipt,
                &trace_clock,
                move |ctx| {
                    // hit test 20ns.
                    hit_clock.set(420);
                    ctx.open_count()
                },
            )
            .unwrap();
        assert_eq!(submitted.outcome, SubmitOutcome::Enqueued);
        // document thread가 바쁜 동안 시각이 흐릅니다.
        clock.set(400);
        release.send(()).unwrap();

        let response = submitted.ticket.wait().unwrap();
        assert_eq!(response.value, 0);
        let trace = response.trace;
        assert_eq!(trace.class, InputEventClass::Discrete);
        assert_eq!(trace.t0_candidates(), [90, 100]);
        assert_eq!(trace.enqueued, 150);
        assert_eq!(trace.t1_candidates(), [400, 420]);
        assert_eq!(trace.queue_latency_nanos(), 300);
        assert_eq!(trace.queue_wait_nanos(), 250);
        assert_eq!(trace.hit_test_nanos(), 20);
        doc.shutdown().unwrap();
    }

    #[test]
    fn hit_test_requests_run_before_queued_commands() {
        // hit-test request는 input lane이라 먼저 접수된 command job보다 먼저 시작합니다.
        let (clock, trace_clock) = clocks();
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let release = block(&sender);
        let command_clock = Arc::clone(&clock);
        let command = sender
            .try_submit(SubmitOptions::command(), move |_ctx| {
                command_clock.now_nanos()
            })
            .unwrap();
        let tick = Arc::clone(&clock);
        let hit = sender
            .submit_hit_test(
                InputEventClass::Discrete,
                InputReceipt::new(1, 2),
                &trace_clock,
                move |_ctx| tick.set(10),
            )
            .unwrap();
        release.send(()).unwrap();
        let trace = hit.ticket.wait().unwrap().trace;
        // command job은 hit test가 시계를 10으로 바꾼 뒤에 실행됩니다.
        assert_eq!(trace.t1_candidates(), [0, 10]);
        assert_eq!(command.ticket.wait().unwrap(), 10);
        doc.shutdown().unwrap();
    }

    #[test]
    fn superseded_moves_leave_no_trace_and_the_latest_keeps_its_receipt() {
        // document thread가 바쁜 동안 같은 장치 move request는 최신 1건만 실행됩니다.
        let (clock, trace_clock) = clocks();
        let doc = DocumentThread::spawn_with(QueueCapacity::new(4, 4, 4).unwrap()).unwrap();
        let sender = doc.sender();
        let release = block(&sender);
        let class = InputEventClass::PointerMove { device: 1 };
        let mut tickets = Vec::new();
        for n in 1..=3u64 {
            clock.set(n * 10);
            let submitted = sender
                .submit_hit_test(
                    class,
                    InputReceipt::stamp(&*clock, n),
                    &trace_clock,
                    move |_ctx| n,
                )
                .unwrap();
            let expected = if n == 1 {
                SubmitOutcome::Enqueued
            } else {
                SubmitOutcome::Coalesced
            };
            assert_eq!(submitted.outcome, expected);
            tickets.push(submitted.ticket);
        }
        clock.set(100);
        release.send(()).unwrap();
        let last = tickets.pop().unwrap().wait().unwrap();
        assert_eq!(last.value, 3);
        assert_eq!(last.trace.t0_candidates(), [3, 30]);
        assert_eq!(last.trace.queue_latency_nanos(), 70);
        for ticket in tickets {
            assert_eq!(
                ticket.wait().unwrap_err(),
                TicketError::Job(JobError::Superseded)
            );
        }
        doc.shutdown().unwrap();
    }

    #[test]
    fn rejected_requests_return_the_submit_error() {
        let (_clock, trace_clock) = clocks();
        let doc = DocumentThread::spawn_with(QueueCapacity::new(1, 1, 1).unwrap()).unwrap();
        let sender = doc.sender();
        let release = block(&sender);
        let receipt = InputReceipt::new(0, 0);
        let first = sender
            .submit_hit_test(InputEventClass::Wheel, receipt, &trace_clock, |_ctx| ())
            .unwrap();
        let full = sender
            .submit_hit_test(InputEventClass::Wheel, receipt, &trace_clock, |_ctx| ())
            .unwrap_err();
        assert!(matches!(full, SubmitError::QueueFull { .. }));
        release.send(()).unwrap();
        assert!(first.ticket.wait().is_ok());
        doc.shutdown().unwrap();
        let stopped = sender
            .submit_hit_test(InputEventClass::Wheel, receipt, &trace_clock, |_ctx| ())
            .unwrap_err();
        assert_eq!(stopped, SubmitError::Stopped);
    }

    #[test]
    fn absorb_keeps_the_earliest_times_and_counts_merges() {
        let first = InputReceipt::new(10, 12);
        let merged = first
            .absorb(InputReceipt::new(20, 25))
            .absorb(InputReceipt::new(30, 31));
        assert_eq!(merged.event_time(), 10);
        assert_eq!(merged.received(), 12);
        assert_eq!(merged.merged(), 2);
        // 이미 병합된 항목을 합치면 그 병합 수도 더합니다.
        let combined = InputReceipt::new(5, 50).absorb(merged);
        assert_eq!(
            (
                combined.event_time(),
                combined.received(),
                combined.merged()
            ),
            (5, 12, 3)
        );
        let saturated = InputReceipt {
            merged: u32::MAX,
            ..first
        }
        .absorb(first);
        assert_eq!(saturated.merged(), u32::MAX);
    }

    #[test]
    fn durations_saturate_instead_of_wrapping() {
        // 시계가 다른 domain이라 순서가 어긋나도 음수 대신 0입니다.
        let trace = InputLatencyTrace {
            class: InputEventClass::Resize,
            receipt: InputReceipt::new(500, 500),
            enqueued: 400,
            started: 300,
            finished: 200,
        };
        assert_eq!(trace.queue_latency_nanos(), 0);
        assert_eq!(trace.queue_wait_nanos(), 0);
        assert_eq!(trace.hit_test_nanos(), 0);
    }
}
