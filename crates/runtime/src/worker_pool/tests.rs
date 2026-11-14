//! worker pool 단위 테스트(설정·이름·닫힌 pool·내부 집계).

use super::*;
use std::sync::mpsc;

#[test]
fn class_and_priority_names_round_trip_and_defaults_follow_the_table() {
    for class in WorkerClass::ALL {
        assert_eq!(WorkerClass::from_name(class.name()), Some(class));
        assert_eq!(WorkerJobSpec::new(class).priority, class.default_priority());
        assert_eq!(WorkerJobSpec::from(class), WorkerJobSpec::new(class));
    }
    for priority in WorkerPriority::ALL {
        assert_eq!(WorkerPriority::from_name(priority.name()), Some(priority));
    }
    assert_eq!(WorkerClass::from_name("unknown"), None);
    assert_eq!(WorkerPriority::from_name("urgent"), None);
    // 모듈 문서 표.
    let interactive = [
        WorkerClass::TextShaping,
        WorkerClass::Tessellation,
        WorkerClass::ImageDecode,
    ];
    for class in interactive {
        assert_eq!(class.default_priority(), WorkerPriority::Interactive);
    }
    assert_eq!(
        WorkerClass::SaveSerialization.default_priority(),
        WorkerPriority::Normal
    );
    assert_eq!(
        WorkerClass::CacheGeneration.default_priority(),
        WorkerPriority::Background
    );
    assert_eq!(
        WorkerClass::Gc.default_priority(),
        WorkerPriority::Background
    );
    // 선언 순서가 실행 순서입니다.
    assert!(WorkerPriority::Interactive < WorkerPriority::Normal);
    assert!(WorkerPriority::Normal < WorkerPriority::Background);
    let spec =
        WorkerJobSpec::new(WorkerClass::ImageDecode).with_priority(WorkerPriority::Background);
    assert_eq!(spec.class, WorkerClass::ImageDecode);
    assert_eq!(spec.priority, WorkerPriority::Background);
}

#[test]
fn config_validation() {
    assert_eq!(
        WorkerPoolConfig::new(0).unwrap_err(),
        WorkerPoolConfigError::InvalidWorkerCount { requested: 0 }
    );
    assert_eq!(
        WorkerPoolConfig::new(MAX_WORKERS + 1).unwrap_err(),
        WorkerPoolConfigError::InvalidWorkerCount {
            requested: MAX_WORKERS + 1
        }
    );
    let one = WorkerPoolConfig::new(1).unwrap();
    assert_eq!(
        one.background_limit(),
        1,
        "worker 하나면 background도 실행해야 합니다"
    );
    assert_eq!(
        one.non_interactive_limit(),
        1,
        "worker 하나면 normal·background도 실행해야 합니다"
    );
    let four = WorkerPoolConfig::new(4).unwrap();
    assert_eq!(four.background_limit(), 3, "worker 하나는 interactive 몫");
    assert_eq!(
        four.non_interactive_limit(),
        3,
        "normal·background 합계도 worker 하나를 남깁니다"
    );
    assert_eq!(
        four.with_non_interactive_limit(4)
            .unwrap()
            .non_interactive_limit(),
        4
    );
    for requested in [0, 5] {
        assert_eq!(
            four.with_non_interactive_limit(requested).unwrap_err(),
            WorkerPoolConfigError::InvalidNonInteractiveLimit {
                requested,
                workers: 4
            }
        );
    }
    assert_eq!(four.with_background_limit(4).unwrap().background_limit(), 4);
    assert_eq!(
        four.with_background_limit(0).unwrap_err(),
        WorkerPoolConfigError::InvalidBackgroundLimit {
            requested: 0,
            workers: 4
        }
    );
    assert_eq!(
        four.with_background_limit(5).unwrap_err(),
        WorkerPoolConfigError::InvalidBackgroundLimit {
            requested: 5,
            workers: 4
        }
    );
    assert_eq!(
        four.with_capacity(WorkerPriority::Normal, 0).unwrap_err(),
        WorkerPoolConfigError::ZeroCapacity {
            priority: WorkerPriority::Normal
        }
    );
    let sized = four.with_capacity(WorkerPriority::Normal, 7).unwrap();
    assert_eq!(sized.capacity(WorkerPriority::Normal), 7);
    assert_eq!(
        sized.capacity(WorkerPriority::Interactive),
        WorkerPoolConfig::DEFAULT_INTERACTIVE_CAPACITY
    );
    let machine = WorkerPoolConfig::for_available_parallelism();
    assert!((2..=MAX_WORKERS).contains(&machine.workers()));
    assert_eq!(machine.background_limit(), machine.workers() - 1);
    assert_eq!(machine.non_interactive_limit(), machine.workers() - 1);
}

fn empty_state() -> PoolState {
    PoolState {
        lanes: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
        running: [0; 3],
        closed: false,
        next_id: 1,
        stats: WorkerPoolStats::default(),
    }
}

fn queued(id: u64, class: WorkerClass) -> QueuedJob {
    QueuedJob {
        id: WorkerJobId(id),
        spec: WorkerJobSpec::new(class),
        run: Box::new(|_: &WorkerContext<'_>| {}),
    }
}

#[test]
fn pop_runnable_skips_background_at_the_limit() {
    let config = WorkerPoolConfig::new(4)
        .unwrap()
        .with_background_limit(1)
        .unwrap();
    let mut state = empty_state();
    state.lanes[2].push_back(queued(1, WorkerClass::Gc));
    state.lanes[1].push_back(queued(2, WorkerClass::SaveSerialization));
    state.lanes[0].push_back(queued(3, WorkerClass::Tessellation));
    // background 상한 1에서 이미 background 하나가 실행 중이면 background는 꺼내지 않습니다.
    state.running[2] = 1;
    let order: Vec<u64> = std::iter::from_fn(|| state.pop_runnable(&config))
        .map(|job| job.id.get())
        .collect();
    assert_eq!(order, vec![3, 2], "interactive → normal, background는 상한");
    assert_eq!(state.lanes[2].len(), 1);
    state.running[2] = 0;
    assert_eq!(state.pop_runnable(&config).map(|job| job.id.get()), Some(1));
}

#[test]
fn pop_runnable_keeps_a_worker_for_interactive_jobs() {
    // 최소 구성(worker 2개, non-interactive 상한 1): 저장 직렬화 하나가 실행 중이면 다른 저장·GC는
    // 꺼내지 않고 interactive만 꺼냅니다.
    let config = WorkerPoolConfig::new(2).unwrap();
    assert_eq!(config.non_interactive_limit(), 1);
    let mut state = empty_state();
    state.lanes[1].push_back(queued(1, WorkerClass::SaveSerialization));
    state.lanes[2].push_back(queued(2, WorkerClass::Gc));
    state.lanes[0].push_back(queued(3, WorkerClass::TextShaping));
    state.running[1] = 1;
    let order: Vec<u64> = std::iter::from_fn(|| state.pop_runnable(&config))
        .map(|job| job.id.get())
        .collect();
    assert_eq!(order, vec![3], "normal·background는 합계 상한");
    // background가 실행 중이어도 normal은 같은 상한에 걸립니다.
    state.running = [0, 0, 1];
    assert!(state.pop_runnable(&config).is_none());
    state.running = [0; 3];
    assert_eq!(state.pop_runnable(&config).map(|job| job.id.get()), Some(1));
    // 상한을 worker 수로 올리면 interactive 전용 worker가 없습니다.
    let all = config.with_non_interactive_limit(2).unwrap();
    state.running = [0, 1, 0];
    assert_eq!(state.pop_runnable(&all).map(|job| job.id.get()), Some(2));
}

#[test]
fn stopped_pool_rejects_with_closed_and_returns_the_payload() {
    let mut pool = WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap();
    let report = pool.stop();
    assert_eq!(report.discarded, 0);
    let rejected = pool
        .try_submit_with(WorkerClass::Gc, 41_u32, |_, _| {})
        .unwrap_err();
    assert_eq!(rejected.reason, WorkerRejectReason::Closed);
    assert_eq!(rejected.payload, 41);
    assert_eq!(pool.stats().rejected, 1);
}

#[test]
fn wait_idle_is_refused_on_role_bound_threads() {
    let pool = Arc::new(WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap());
    for role in [
        ThreadRole::Ui,
        ThreadRole::Document,
        ThreadRole::Render,
        ThreadRole::Worker,
    ] {
        let pool = Arc::clone(&pool);
        let refused = thread::spawn(move || {
            let _guard = bind_thread_role(role).unwrap();
            pool.wait_idle(Duration::from_millis(10))
        })
        .join()
        .unwrap();
        assert_eq!(refused, Err(WaitIdleError::BlockingNotAllowed { role }));
    }
    assert_eq!(pool.wait_idle(Duration::from_secs(1)), Ok(true));
}

#[test]
fn shutdown_discards_unstarted_jobs_and_cancels_running_ones() {
    let pool = WorkerPool::new(WorkerPoolConfig::new(1).unwrap()).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (seen_tx, seen_rx) = mpsc::channel();
    // 실행 중 job은 취소 표시를 볼 때까지 돕니다.
    pool.try_submit(WorkerClass::Gc, move |context: &WorkerContext<'_>| {
        started_tx.send(()).unwrap();
        while !context.is_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        seen_tx.send(context.job_id()).unwrap();
    })
    .unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (ran_tx, ran_rx) = mpsc::channel::<u32>();
    for value in 0..5 {
        let ran_tx = ran_tx.clone();
        pool.try_submit_with(WorkerClass::CacheGeneration, value, move |value, _| {
            ran_tx.send(value).unwrap();
        })
        .unwrap();
    }
    drop(ran_tx);
    let report = pool.shutdown();
    assert_eq!(report.discarded, 5);
    assert_eq!(report.panicked_workers, 0);
    assert_eq!(seen_rx.recv().unwrap(), WorkerJobId(1));
    // 버린 job은 실행되지 않고, closure가 버려져 sender도 모두 닫혔습니다.
    assert!(ran_rx.recv().is_err());
}

#[test]
fn last_pool_owner_can_be_dropped_inside_its_worker() {
    let pool = Arc::new(WorkerPool::new(WorkerPoolConfig::new(2).unwrap()).unwrap());
    let shared = Arc::downgrade(&pool.shared);
    let owner = Arc::clone(&pool);
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    pool.try_submit(WorkerClass::TextShaping, move |_: &WorkerContext<'_>| {
        let _ = release_rx.recv();
        drop(owner);
        done_tx.send(()).unwrap();
    })
    .unwrap();
    // job이 마지막 owner를 가지므로 job 안에서 pool이 drop됩니다.
    drop(pool);
    release_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.strong_count() != 0 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(
        shared.strong_count(),
        0,
        "worker가 끝나고 공유 상태도 회수됨"
    );
}
