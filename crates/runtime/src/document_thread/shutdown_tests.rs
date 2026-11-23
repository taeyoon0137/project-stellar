//! thread별 종료 순서 시험 (XII.1.6): UI stop → command drain/cancel → render stop → worker stop → runtime destroy.

use super::*;
use crate::render_thread::{FrameFreshness, RenderFrame};
use crate::render_thread::{FrameOutcome, RenderThread};
use crate::worker_pool::{WorkerClass, WorkerJobSpec, WorkerPool, WorkerPoolConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use stellar_core_types::revision::SceneRevision;

/// 단계가 실제로 실행된 순서를 기록합니다.
#[derive(Default)]
struct Trace(Mutex<Vec<&'static str>>);

impl Trace {
    fn push(&self, what: &'static str) {
        self.0.lock().unwrap().push(what);
    }

    fn take(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// 묶음 하나를 만들고, drain 중인 document job이 render에 frame을 넘기고 worker job이 실행 중인 상태에서 종료합니다.
fn lifecycle(trace: &Arc<Trace>, live: &Arc<()>) -> ShutdownReport {
    let document = DocumentThread::spawn().unwrap();
    let render_live = Arc::clone(live);
    let render_trace = Arc::clone(trace);
    let (render, publisher) = RenderThread::<u64>::spawn(
        move |_frame: &RenderFrame<u64>, _fresh: &FrameFreshness<'_>| {
            let _keep = &render_live;
            render_trace.push("render");
            FrameOutcome::Presented
        },
    )
    .unwrap();
    let workers = WorkerPool::new(WorkerPoolConfig::new(2).unwrap()).unwrap();
    let worker_live = Arc::clone(live);
    let _ = workers.try_submit(WorkerJobSpec::new(WorkerClass::TextShaping), move |_ctx| {
        let _keep = worker_live;
        std::thread::sleep(Duration::from_micros(50));
    });
    // UI stop이 열 때까지 document thread를 붙잡아, 아래 command가 종료의 drain 단계에서 실행되게 합니다.
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let ui_trace = Arc::clone(trace);
    let ui_gate = Arc::clone(&gate);
    let threads = RuntimeThreads::new(document, Some(render), Some(workers))
        .with_ui_stop(move || {
            ui_trace.push("ui-stop");
            *ui_gate.0.lock().unwrap() = true;
            ui_gate.1.notify_all();
        })
        .with_shared(Arc::clone(live));
    let sender = threads.sender().unwrap();
    let job_gate = Arc::clone(&gate);
    sender
        .submit(move |_ctx: &mut DocumentContext| {
            let mut open = job_gate.0.lock().unwrap();
            while !*open {
                open = job_gate.1.wait(open).unwrap();
            }
        })
        .unwrap();
    let publisher = Arc::new(Mutex::new(publisher));
    for n in 1..=3u64 {
        let publisher = Arc::clone(&publisher);
        let job_trace = Arc::clone(trace);
        let job_live = Arc::clone(live);
        sender
            .submit(move |_ctx: &mut DocumentContext| {
                let _keep = job_live;
                // drain 중에도 render가 살아 있어 frame 전달이 막히거나 실패하지 않습니다.
                let delivered = publisher.lock().unwrap().publish(SceneRevision(n), n);
                job_trace.push(if delivered.is_ok() {
                    "drain-publish"
                } else {
                    "drain-publish-failed"
                });
            })
            .unwrap();
    }
    drop(sender);
    let report = threads.shutdown(Some(Duration::from_secs(5)));
    drop(publisher);
    report
}

#[test]
fn shutdown_runs_the_fixed_stage_order() {
    let trace = Arc::new(Trace::default());
    let live = Arc::new(());
    let report = lifecycle(&trace, &live);
    assert_eq!(report.stages, ShutdownStage::ORDER.to_vec());
    let codes: Vec<_> = report.stages.iter().map(|stage| stage.code()).collect();
    assert_eq!(
        codes,
        [
            "ui-stop",
            "command-drain",
            "render-stop",
            "worker-stop",
            "runtime-destroy"
        ]
    );
    assert_eq!(
        report.document.unwrap().jobs_run,
        4,
        "남은 command를 모두 실행"
    );
    assert!(report.render.unwrap().is_ok());
    assert_eq!(report.workers.unwrap().panicked_workers, 0);
    let events = trace.take();
    assert_eq!(events.first(), Some(&"ui-stop"), "UI가 먼저 멈춤");
    assert_eq!(
        events.iter().filter(|e| **e == "drain-publish").count(),
        3,
        "drain 중 render 전달 성공: {events:?}"
    );
    assert_eq!(
        Arc::strong_count(&live),
        1,
        "모든 thread가 공유 상태를 놓음"
    );
}

#[test]
fn command_drain_cancels_unstarted_jobs_after_the_timeout() {
    let document = DocumentThread::spawn().unwrap();
    let sender = document.sender();
    let threads: RuntimeThreads<u64> = RuntimeThreads::new(document, None, None);
    let _slow = sender
        .submit(|_ctx: &mut DocumentContext| std::thread::sleep(Duration::from_millis(200)))
        .unwrap();
    let mut queued = sender.submit(|_ctx: &mut DocumentContext| 1u8).unwrap();
    let report = threads.shutdown(Some(Duration::from_millis(20)));
    assert!(matches!(
        report.document,
        Err(DocumentThreadError::ShutdownTimedOut { .. })
    ));
    assert_eq!(report.stages, ShutdownStage::ORDER.to_vec());
    assert!(matches!(
        queued.wait_timeout(Duration::from_secs(5)),
        Err(TicketError::Job(JobError::Lost))
    ));
}

#[test]
fn thousand_lifecycles_have_no_deadlock_or_leak() {
    // 검증 기준(XII.1.6): 1,000 lifecycle stress에서 deadlock·누수 없음.
    let trace = Arc::new(Trace::default());
    let live = Arc::new(());
    let started = Instant::now();
    let finished = AtomicUsize::new(0);
    for _ in 0..1_000 {
        let report = lifecycle(&trace, &live);
        assert_eq!(report.stages, ShutdownStage::ORDER.to_vec());
        assert!(report.document.is_ok());
        assert!(report.render.unwrap().is_ok());
        assert_eq!(report.workers.unwrap().panicked_workers, 0);
        assert_eq!(Arc::strong_count(&live), 1, "lifecycle마다 누수 없음");
        trace.take();
        finished.fetch_add(1, Ordering::Relaxed);
    }
    assert_eq!(finished.load(Ordering::Relaxed), 1_000);
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "1,000 lifecycle이 {:?} 걸림",
        started.elapsed()
    );
}
