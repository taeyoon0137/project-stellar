//! worker pool class·priority 실행 순서 (XII.1.4).
//!
//! 검증 기준 "interactive 작업이 background GC보다 먼저 실행"을 실제 worker thread로 확인합니다.
//!
//! - 대기열 순서: worker가 하나뿐이고 GC가 먼저 쌓여 있어도, 나중에 온 interactive job이 먼저 실행됩니다.
//! - 실행 자리: 오래 걸리는 GC가 몰려도 background 동시 실행 상한 때문에 worker 하나가 남아 interactive
//!   job이 GC가 끝나기 전에 실행됩니다. 저장 직렬화(normal)와 GC를 합친 non-interactive 상한도 worker
//!   하나를 남기므로 최소 구성(worker 2개)에서 긴 저장이 몰려도 interactive job이 기다리지 않습니다.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use stellar_renderer_core::image_decode::DecodeWorkerScope;
use stellar_renderer_core::thread_role::{current_thread_role, ThreadRole};
use stellar_runtime::worker_pool::{
    WorkerClass, WorkerContext, WorkerJobSpec, WorkerPool, WorkerPoolConfig, WorkerPriority,
    WorkerRejectReason, WORKER_THREAD_NAME_PREFIX,
};

const TIMEOUT: Duration = Duration::from_secs(10);

type Log = Arc<Mutex<Vec<String>>>;

fn log_job(log: &Log, label: &str) -> impl FnOnce(&WorkerContext<'_>) + Send + 'static {
    let log = Arc::clone(log);
    let label = label.to_string();
    move |_| log.lock().unwrap().push(label)
}

/// worker 하나를 `release`가 올 때까지 붙잡는 job을 제출하고 시작을 기다립니다.
fn occupy(pool: &WorkerPool, spec: WorkerJobSpec) -> Sender<()> {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    pool.try_submit(spec, move |_: &WorkerContext<'_>| {
        started_tx.send(()).unwrap();
        let _ = release_rx.recv();
    })
    .unwrap();
    started_rx.recv_timeout(TIMEOUT).unwrap();
    release_tx
}

#[test]
fn interactive_jobs_run_before_queued_background_gc() {
    // worker 하나: 대기열 순서만으로 실행 순서가 정해집니다.
    let pool = WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap();
    let release = occupy(&pool, WorkerJobSpec::new(WorkerClass::TextShaping));
    let log: Log = Arc::default();
    // 접수 순서: GC·캐시가 먼저, 저장 직렬화, interactive가 가장 나중.
    let submissions = [
        ("gc-1", WorkerJobSpec::new(WorkerClass::Gc)),
        ("gc-2", WorkerJobSpec::new(WorkerClass::Gc)),
        ("cache-1", WorkerJobSpec::new(WorkerClass::CacheGeneration)),
        ("save-1", WorkerJobSpec::new(WorkerClass::SaveSerialization)),
        ("tess-1", WorkerJobSpec::new(WorkerClass::Tessellation)),
        ("decode-1", WorkerJobSpec::new(WorkerClass::ImageDecode)),
        ("gc-3", WorkerJobSpec::new(WorkerClass::Gc)),
        ("shape-1", WorkerJobSpec::new(WorkerClass::TextShaping)),
        // 화면 밖 미리 decode는 background로 낮춥니다.
        (
            "prefetch-1",
            WorkerJobSpec::new(WorkerClass::ImageDecode).with_priority(WorkerPriority::Background),
        ),
        ("save-2", WorkerJobSpec::new(WorkerClass::SaveSerialization)),
        ("tess-2", WorkerJobSpec::new(WorkerClass::Tessellation)),
    ];
    for (label, spec) in submissions {
        pool.try_submit(spec, log_job(&log, label)).unwrap();
    }
    assert_eq!(pool.queued(), submissions.len());
    release.send(()).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    let order = log.lock().unwrap().clone();
    assert_eq!(
        order,
        [
            "tess-1",
            "decode-1",
            "shape-1",
            "tess-2",
            "save-1",
            "save-2",
            "gc-1",
            "gc-2",
            "cache-1",
            "gc-3",
            "prefetch-1",
        ],
        "interactive → normal → background, lane 안은 접수 순서"
    );
    let stats = pool.stats();
    assert_eq!(stats.completed, 12);
    assert_eq!(stats.completed_for(WorkerClass::Gc), 3);
    assert_eq!(stats.completed_for(WorkerClass::ImageDecode), 2);
    assert_eq!(stats.completed_for(WorkerClass::TextShaping), 2);
}

#[test]
fn long_background_gc_never_takes_every_worker() {
    // worker 4개, 기본 background 상한 3: GC가 아무리 많아도 worker 하나는 interactive 몫입니다.
    let config = WorkerPoolConfig::new(4).unwrap();
    assert_eq!(config.background_limit(), 3);
    let pool = WorkerPool::new(config).unwrap();
    let (started_tx, started_rx) = mpsc::channel::<u32>();
    let gates: Vec<Sender<()>> = (0..6)
        .map(|index| {
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let started_tx = started_tx.clone();
            pool.try_submit(WorkerClass::Gc, move |_: &WorkerContext<'_>| {
                started_tx.send(index).unwrap();
                let _ = release_rx.recv();
            })
            .unwrap();
            release_tx
        })
        .collect();
    let mut started: Vec<u32> = (0..3)
        .map(|_| started_rx.recv_timeout(TIMEOUT).unwrap())
        .collect();
    started.sort_unstable();
    assert_eq!(started, vec![0, 1, 2], "GC는 접수 순서대로 상한까지만 시작");
    // 상한에 걸린 GC는 쉬는 worker가 있어도 시작하지 않습니다.
    assert!(started_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(pool.running(), 3);
    assert_eq!(pool.queued(), 3);

    // GC가 하나도 끝나지 않은 상태에서 interactive job이 남은 worker에서 바로 실행됩니다.
    let (done_tx, done_rx) = mpsc::channel::<Option<ThreadRole>>();
    pool.try_submit(WorkerClass::Tessellation, move |_: &WorkerContext<'_>| {
        done_tx.send(current_thread_role()).unwrap();
    })
    .unwrap();
    assert_eq!(
        done_rx.recv_timeout(TIMEOUT).unwrap(),
        Some(ThreadRole::Worker)
    );
    // job closure가 끝난 뒤 worker가 완료를 집계하므로 집계가 반영될 때까지 잠깐 기다립니다.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while pool.stats().completed < 1 && std::time::Instant::now() < deadline {
        thread::yield_now();
    }
    let stats = pool.stats();
    assert_eq!(stats.completed, 1, "interactive job만 끝났습니다");
    assert_eq!(
        stats.completed_for(WorkerClass::Gc),
        0,
        "GC는 하나도 끝나지 않았습니다"
    );
    assert_eq!(pool.running(), 3, "GC 3개는 아직 실행 중");

    for gate in gates {
        gate.send(()).unwrap();
    }
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    let stats = pool.stats();
    assert_eq!(stats.completed, 7);
    assert_eq!(stats.peak_background_running, 3);
}

#[test]
fn long_saves_and_gc_never_take_the_interactive_worker() {
    // 최소 구성 worker 2개(non-interactive 상한 1): 창 두 개의 긴 저장 직렬화와 GC가 몰려도 worker 하나는
    // interactive 몫이라 shaping이 저장이 끝나기 전에 실행됩니다.
    let config = WorkerPoolConfig::new(2).unwrap();
    assert_eq!(config.non_interactive_limit(), 1);
    let pool = WorkerPool::new(config).unwrap();
    let (started_tx, started_rx) = mpsc::channel::<&'static str>();
    let gates: Vec<Sender<()>> = [
        ("save-1", WorkerClass::SaveSerialization),
        ("save-2", WorkerClass::SaveSerialization),
        ("gc-1", WorkerClass::Gc),
    ]
    .into_iter()
    .map(|(label, class)| {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let started_tx = started_tx.clone();
        pool.try_submit(class, move |_: &WorkerContext<'_>| {
            started_tx.send(label).unwrap();
            let _ = release_rx.recv();
        })
        .unwrap();
        release_tx
    })
    .collect();
    assert_eq!(started_rx.recv_timeout(TIMEOUT).unwrap(), "save-1");
    // 쉬는 worker가 있어도 두 번째 저장과 GC는 시작하지 않습니다.
    assert!(started_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(pool.running(), 1);
    assert_eq!(pool.queued(), 2);

    let (done_tx, done_rx) = mpsc::channel::<Option<ThreadRole>>();
    pool.try_submit(WorkerClass::TextShaping, move |_: &WorkerContext<'_>| {
        done_tx.send(current_thread_role()).unwrap();
    })
    .unwrap();
    assert_eq!(
        done_rx.recv_timeout(TIMEOUT).unwrap(),
        Some(ThreadRole::Worker),
        "저장이 끝나기 전에 interactive shaping 실행"
    );
    let deadline = std::time::Instant::now() + TIMEOUT;
    while pool.stats().completed < 1 && std::time::Instant::now() < deadline {
        thread::yield_now();
    }
    let stats = pool.stats();
    assert_eq!(stats.completed, 1, "interactive job만 끝났습니다");
    assert_eq!(stats.completed_for(WorkerClass::SaveSerialization), 0);

    // 저장을 하나씩 풀면 다음 저장, 그다음 GC 순서로 하나씩만 실행됩니다.
    let mut gates = gates.into_iter();
    gates.next().unwrap().send(()).unwrap();
    assert_eq!(started_rx.recv_timeout(TIMEOUT).unwrap(), "save-2");
    gates.next().unwrap().send(()).unwrap();
    assert_eq!(started_rx.recv_timeout(TIMEOUT).unwrap(), "gc-1");
    gates.next().unwrap().send(()).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    let stats = pool.stats();
    assert_eq!(stats.completed, 4);
    assert_eq!(stats.peak_non_interactive_running, 1);
}

#[test]
fn normal_save_serialization_runs_before_background_gc() {
    let pool = WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap();
    let release = occupy(&pool, WorkerJobSpec::new(WorkerClass::Gc));
    let log: Log = Arc::default();
    pool.try_submit(WorkerClass::Gc, log_job(&log, "gc"))
        .unwrap();
    pool.try_submit(WorkerClass::SaveSerialization, log_job(&log, "save"))
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    assert_eq!(*log.lock().unwrap(), ["save", "gc"]);
}

#[test]
fn full_lane_rejects_without_dropping_queued_jobs() {
    let config = WorkerPoolConfig::new(1)
        .unwrap()
        .with_capacity(WorkerPriority::Background, 2)
        .unwrap();
    let pool = WorkerPool::new(config).unwrap();
    let release = occupy(&pool, WorkerJobSpec::new(WorkerClass::TextShaping));
    let (ran_tx, ran_rx): (Sender<u32>, Receiver<u32>) = mpsc::channel();
    let submit = |value: u32, class: WorkerClass| {
        let ran_tx = ran_tx.clone();
        pool.try_submit_with(class, value, move |value, _| ran_tx.send(value).unwrap())
    };
    submit(1, WorkerClass::Gc).unwrap();
    submit(2, WorkerClass::CacheGeneration).unwrap();
    let rejected = submit(3, WorkerClass::Gc).unwrap_err();
    assert_eq!(
        rejected.reason,
        WorkerRejectReason::QueueFull {
            priority: WorkerPriority::Background
        }
    );
    assert_eq!(rejected.payload, 3, "거부된 payload를 돌려받습니다");
    // lane마다 용량이 따로라 interactive는 그대로 받습니다.
    submit(4, WorkerClass::Tessellation).unwrap();
    release.send(()).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    drop(ran_tx);
    let ran: Vec<u32> = ran_rx.iter().collect();
    assert_eq!(ran, vec![4, 1, 2], "이미 접수된 job은 버리지 않습니다");
    let stats = pool.stats();
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.submitted, 4);
}

#[test]
fn jobs_run_on_named_worker_role_threads_and_can_decode_images() {
    let pool = WorkerPool::new(WorkerPoolConfig::new(2).unwrap()).unwrap();
    let (tx, rx) = mpsc::channel();
    pool.try_submit(
        WorkerClass::ImageDecode,
        move |context: &WorkerContext<'_>| {
            let name = thread::current().name().map(str::to_string);
            // worker 전용 decode scope(XI.7.1)를 worker thread에서 얻을 수 있습니다.
            let scope = DecodeWorkerScope::current().is_ok();
            tx.send((name, current_thread_role(), scope, context.spec()))
                .unwrap();
        },
    )
    .unwrap();
    let (name, role, scope, spec) = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(name.unwrap().starts_with(WORKER_THREAD_NAME_PREFIX));
    assert_eq!(role, Some(ThreadRole::Worker));
    assert!(scope);
    assert_eq!(spec, WorkerJobSpec::new(WorkerClass::ImageDecode));
    // 테스트 thread(역할 없음)는 decode scope를 얻지 못합니다.
    assert!(DecodeWorkerScope::current().is_err());
}

#[test]
fn panicking_job_is_counted_and_the_worker_keeps_running() {
    let pool = WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap();
    pool.try_submit(WorkerClass::CacheGeneration, |_: &WorkerContext<'_>| {
        panic!("test panic");
    })
    .unwrap();
    let (tx, rx) = mpsc::channel();
    pool.try_submit(WorkerClass::Tessellation, move |_: &WorkerContext<'_>| {
        tx.send(()).unwrap();
    })
    .unwrap();
    rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    let stats = pool.stats();
    assert_eq!(stats.panicked, 1);
    assert_eq!(stats.completed, 1);
    // background 실행 수가 정리되어 다음 background job도 실행됩니다.
    let (tx, rx) = mpsc::channel();
    pool.try_submit(WorkerClass::Gc, move |_: &WorkerContext<'_>| {
        tx.send(()).unwrap();
    })
    .unwrap();
    rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(pool.shutdown().panicked_workers, 0);
}

#[test]
fn mixed_stress_keeps_background_under_the_limit() {
    // worker 3개, background 상한 1: 여러 생산자가 섞어 제출해도 background는 동시에 하나만 실행합니다.
    let config = WorkerPoolConfig::new(3)
        .unwrap()
        .with_background_limit(1)
        .unwrap();
    let pool = Arc::new(WorkerPool::new(config).unwrap());
    let running_background = Arc::new(Mutex::new((0_usize, 0_usize)));
    let producers: Vec<_> = (0..4)
        .map(|producer| {
            let pool = Arc::clone(&pool);
            let running_background = Arc::clone(&running_background);
            thread::spawn(move || {
                for index in 0..100_usize {
                    let class = WorkerClass::ALL[(producer + index) % WorkerClass::ALL.len()];
                    let running_background = Arc::clone(&running_background);
                    let job = move |context: &WorkerContext<'_>| {
                        let background = context.spec().priority == WorkerPriority::Background;
                        if background {
                            let mut guard = running_background.lock().unwrap();
                            guard.0 += 1;
                            guard.1 = guard.1.max(guard.0);
                        }
                        thread::sleep(Duration::from_micros(200));
                        if background {
                            running_background.lock().unwrap().0 -= 1;
                        }
                    };
                    let mut pending = job;
                    loop {
                        match pool.try_submit(class, pending) {
                            Ok(_) => break,
                            Err(rejected) => {
                                pending = rejected.payload;
                                thread::yield_now();
                            }
                        }
                    }
                }
            })
        })
        .collect();
    for producer in producers {
        producer.join().unwrap();
    }
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    let stats = pool.stats();
    assert_eq!(stats.completed, 400);
    assert_eq!(stats.peak_background_running, 1);
    assert!(
        stats.peak_non_interactive_running <= config.non_interactive_limit(),
        "normal·background 합계도 상한 안"
    );
    assert_eq!(running_background.lock().unwrap().1, 1);
}
