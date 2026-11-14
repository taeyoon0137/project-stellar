//! worker result 출처와 stale result 거부 (XII.1.5).
//!
//! 검증 기준 "rapid edit fixture에서 stale apply 0"을 실제 worker pool로 확인합니다.
//!
//! - `tests/fixtures/worker_rapid_edit_v1.txt`: 결과 도착 순서를 고정한 case별 판정(apply/stale 이유).
//! - 무작위 rapid edit: worker 4개가 비결정 순서로 결과를 내는 동안 3,000번 편집하고, 적용한 결과가 항상
//!   적용 시점의 현재 입력과 같은지(stale apply 0)와 편집이 멈춘 뒤 모든 key가 최신 입력의 결과로
//!   수렴하는지(starvation 없음) 확인합니다.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use stellar_core_types::revision::DocumentRevision;
use stellar_runtime::worker_pool::{
    WorkerClass, WorkerContext, WorkerPool, WorkerPoolConfig, WorkerPriority, WorkerRejectReason,
};
use stellar_runtime::worker_result::{
    worker_result_channel, Admission, CapturedInput, Expectation, SourcedJob, StaleReason,
    WorkerResultGate, WorkerResultReceiver, WorkerResultSender,
};

const FIXTURE: &str = include_str!("fixtures/worker_rapid_edit_v1.txt");

/// fixture가 담은 case 수(파서가 case를 건너뛰지 않았는지 확인).
const EXPECTED_CASES: usize = 7;

/// fixture runner worker 수(release 전 동시에 붙잡을 수 있는 job 수 상한).
const FIXTURE_WORKERS: usize = 8;

const TIMEOUT: Duration = Duration::from_secs(10);

struct FixtureRun {
    /// release 전 job: 이름 → (key, release channel).
    ///
    /// field는 선언 순서대로 drop되므로 `pool`보다 먼저 선언합니다. 판정 assert가 panic하면 unwind 중에
    /// release channel이 먼저 닫혀 붙잡힌 job이 끝나고, 그 뒤 `WorkerPool` drop의 join이 끝납니다(반대
    /// 순서면 join이 끝나지 않아 실패 대신 멈춤).
    jobs: HashMap<String, (String, Sender<()>)>,
    pool: WorkerPool,
    tx: WorkerResultSender<String, String>,
    rx: WorkerResultReceiver<String, String>,
    gate: WorkerResultGate<String>,
    revision: u64,
    doc: HashMap<String, String>,
    /// edit 뒤 아직 expect하지 않은 key(fixture 규약 검사).
    unexpected_edits: HashSet<String>,
    applied: u64,
    stale: u64,
}

impl FixtureRun {
    fn new() -> FixtureRun {
        let (tx, rx) = worker_result_channel();
        FixtureRun {
            jobs: HashMap::new(),
            pool: WorkerPool::new(WorkerPoolConfig::new(FIXTURE_WORKERS).unwrap()).unwrap(),
            tx,
            rx,
            gate: WorkerResultGate::new(),
            revision: 0,
            doc: HashMap::new(),
            unexpected_edits: HashSet::new(),
            applied: 0,
            stale: 0,
        }
    }

    fn capture(&self, key: &str, line: usize) -> CapturedInput<String> {
        let input = self
            .doc
            .get(key)
            .unwrap_or_else(|| panic!("{line}행: 입력이 없는 key: {key}"))
            .clone();
        CapturedInput::capture(DocumentRevision(self.revision), input)
    }

    fn submit(&mut self, job: &str, key: &str, line: usize) {
        assert!(
            self.jobs.len() < FIXTURE_WORKERS,
            "{line}행: 동시에 붙잡은 job이 worker 수를 넘습니다"
        );
        let captured = self.capture(key, line);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let sourced = SourcedJob {
            key: key.to_string(),
            input: captured,
            compute: move |input: &String, _: &WorkerContext<'_>| {
                let _ = release_rx.recv();
                input.clone()
            },
        };
        self.pool
            .try_submit_sourced(WorkerClass::TextShaping, &self.tx, sourced)
            .unwrap_or_else(|rejected| panic!("{line}행: 제출 거부: {:?}", rejected.reason));
        let previous = self
            .jobs
            .insert(job.to_string(), (key.to_string(), release_tx));
        assert!(previous.is_none(), "{line}행: 중복 job 이름: {job}");
    }

    fn release(&mut self, job: &str, expected: &[&str], line: usize) {
        assert!(
            self.unexpected_edits.is_empty(),
            "{line}행: fixture 규약 위반 — expect하지 않은 편집: {:?}",
            self.unexpected_edits
        );
        let (key, release_tx) = self
            .jobs
            .remove(job)
            .unwrap_or_else(|| panic!("{line}행: 알 수 없는 job: {job}"));
        release_tx.send(()).unwrap();
        let result = self
            .rx
            .recv_timeout(TIMEOUT)
            .unwrap()
            .unwrap_or_else(|| panic!("{line}행: {job} 결과가 오지 않습니다"));
        assert_eq!(result.key(), &key, "{line}행: 다른 job의 결과");
        match (self.gate.admit(result), expected) {
            (Admission::Apply { key, value, .. }, ["apply"]) => {
                // stale apply 0: 적용한 값은 그 시점 문서의 현재 입력입니다.
                assert_eq!(
                    Some(&value),
                    self.doc.get(&key),
                    "{line}행: stale 결과를 적용했습니다"
                );
                self.applied += 1;
            }
            (Admission::Stale { reason, .. }, ["stale", name]) => {
                assert_eq!(reason.name(), *name, "{line}행: stale 이유");
                self.stale += 1;
            }
            (admission, _) => panic!("{line}행: 기대 {expected:?}, 실제 {admission:?}"),
        }
    }
}

#[test]
fn rapid_edit_fixture_applies_no_stale_result() {
    let mut cases = 0;
    let mut totals = (0_u64, 0_u64);
    let mut run: Option<(String, FixtureRun)> = None;
    for (index, raw) in FIXTURE.lines().enumerate() {
        let line = index + 1;
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let (command, expected) = match text.split_once("=>") {
            Some((command, expected)) => (command.trim(), Some(expected.trim())),
            None => (text, None),
        };
        let words: Vec<&str> = command.split_whitespace().collect();
        let expected: Vec<&str> =
            expected.map_or_else(Vec::new, |e| e.split_whitespace().collect());
        if let ["case", name] = words.as_slice() {
            assert!(run.is_none(), "{line}행: 이전 case가 끝나지 않았습니다");
            run = Some(((*name).to_string(), FixtureRun::new()));
            continue;
        }
        let (name, state) = run
            .as_mut()
            .unwrap_or_else(|| panic!("{line}행: case 밖 명령: {text}"));
        match words.as_slice() {
            ["edit", key, input] => {
                state.revision += 1;
                state.doc.insert((*key).to_string(), (*input).to_string());
                state.unexpected_edits.insert((*key).to_string());
            }
            ["touch"] => state.revision += 1,
            ["expect", key] => {
                let captured = state.capture(key, line);
                let outcome = state.gate.expect((*key).to_string(), captured.source());
                let actual = match outcome {
                    Ok(Expectation::NewInput) => "new",
                    Ok(Expectation::SameInput { applied: false }) => "same",
                    Ok(Expectation::SameInput { applied: true }) => "same-applied",
                    Err(error) => panic!("{line}행: {error}"),
                };
                assert_eq!(expected, [actual], "{line}행: {name} expect");
                state.unexpected_edits.remove(*key);
            }
            ["submit", job, key] => state.submit(job, key, line),
            ["release", job] => state.release(job, &expected, line),
            ["forget", key] => {
                assert!(
                    state.gate.forget(&(*key).to_string()),
                    "{line}행: 기대 없음"
                );
            }
            ["end"] => {
                let (_, state) = run.take().unwrap();
                assert!(state.jobs.is_empty(), "{line}행: release하지 않은 job");
                assert_eq!(state.gate.stats().applied, state.applied);
                assert_eq!(state.gate.stats().stale, state.stale);
                totals.0 += state.applied;
                totals.1 += state.stale;
                cases += 1;
            }
            _ => panic!("{line}행: 알 수 없는 명령: {text}"),
        }
    }
    assert!(run.is_none(), "마지막 case가 끝나지 않았습니다");
    assert_eq!(cases, EXPECTED_CASES);
    assert_eq!(totals, (8, 11), "fixture 전체 apply·stale 수");
}

#[test]
fn fixture_failure_releases_waiting_jobs_before_join() {
    let (done_tx, done_rx) = mpsc::channel();
    let runner = thread::spawn(move || {
        let failed = std::panic::catch_unwind(|| {
            let mut run = FixtureRun::new();
            run.revision = 1;
            run.doc.insert("key".into(), "value".into());
            let source = run.capture("key", 1).source();
            run.gate.expect("key".into(), source).unwrap();
            run.submit("first", "key", 1);
            run.submit("waiting", "key", 2);
            // 적용되어야 하는 결과를 일부러 잘못 판정합니다. waiting의 release sender가
            // pool보다 먼저 drop되지 않으면 unwind의 join이 멈춥니다.
            run.release("first", &["stale", "not-expected"], 3);
        })
        .is_err();
        done_tx.send(failed).unwrap();
    });
    assert!(done_rx.recv_timeout(TIMEOUT).unwrap());
    runner.join().unwrap();
}

#[test]
fn rejected_sourced_job_keeps_its_capture_and_is_stale_after_retry() {
    let config = WorkerPoolConfig::new(1)
        .unwrap()
        .with_capacity(WorkerPriority::Interactive, 1)
        .unwrap();
    let pool = WorkerPool::new(config).unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    pool.try_submit(WorkerClass::TextShaping, move |_: &WorkerContext<'_>| {
        started_tx.send(()).unwrap();
        let _ = release_rx.recv();
    })
    .unwrap();
    started_rx.recv_timeout(TIMEOUT).unwrap();
    pool.try_submit(WorkerClass::TextShaping, |_: &WorkerContext<'_>| {})
        .unwrap();

    let (tx, rx) = worker_result_channel();
    let mut gate = WorkerResultGate::new();
    let captured = CapturedInput::capture(DocumentRevision(5), "old".to_string());
    let source = captured.source();
    gate.expect("draft", source).unwrap();
    let rejected = pool
        .try_submit_sourced(
            WorkerClass::TextShaping,
            &tx,
            SourcedJob {
                key: "draft",
                input: captured,
                compute: |input: &String, _: &WorkerContext<'_>| input.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(
        rejected.reason,
        WorkerRejectReason::QueueFull {
            priority: WorkerPriority::Interactive,
        }
    );
    assert_eq!(rejected.payload.key, "draft");
    assert_eq!(rejected.payload.input.source(), source);
    assert_eq!(rejected.payload.input.input(), "old");
    assert!(rx.try_recv().is_none(), "거부한 job은 실행하지 않습니다");

    let current = CapturedInput::capture(DocumentRevision(6), "new".to_string());
    gate.expect("draft", current.source()).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    pool.try_submit_sourced(WorkerClass::TextShaping, &tx, rejected.payload)
        .unwrap();
    let result = rx.recv_timeout(TIMEOUT).unwrap().unwrap();
    assert_eq!(result.source(), source, "재제출해도 출처는 r5입니다");
    assert_eq!(result.class(), WorkerClass::TextShaping);
    assert!(matches!(
        gate.admit(result),
        Admission::Stale {
            reason: StaleReason::InputChanged { .. },
            ..
        }
    ));

    pool.try_submit_sourced(
        WorkerClass::TextShaping,
        &tx,
        SourcedJob {
            key: "draft",
            input: current,
            compute: |input: &String, _: &WorkerContext<'_>| input.clone(),
        },
    )
    .unwrap();
    let result = rx.recv_timeout(TIMEOUT).unwrap().unwrap();
    assert!(matches!(gate.admit(result), Admission::Apply { value, .. } if value == "new"));
    assert_eq!((gate.stats().applied, gate.stats().stale), (1, 1));
}

/// 결정적 PRNG(xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

struct StressConsumer {
    gate: WorkerResultGate<usize>,
    doc: Vec<String>,
    applied: Vec<Option<String>>,
    submitted: u64,
}

impl StressConsumer {
    fn judge(&mut self, rx: &WorkerResultReceiver<usize, String>) {
        for result in rx.drain() {
            if let Admission::Apply { key, value, .. } = self.gate.admit(result) {
                // stale apply 0: 적용 시점의 현재 입력과 같아야 합니다.
                assert_eq!(
                    value, self.doc[key],
                    "key {key}에 stale 결과를 적용했습니다"
                );
                self.applied[key] = Some(value);
            }
        }
    }

    /// key의 현재 입력을 기대하고, 새 입력이면 제출합니다.
    fn refresh(
        &mut self,
        pool: &WorkerPool,
        tx: &WorkerResultSender<usize, String>,
        rx: &WorkerResultReceiver<usize, String>,
        revision: u64,
        key: usize,
    ) {
        let captured = CapturedInput::capture(DocumentRevision(revision), self.doc[key].clone());
        match self.gate.expect(key, captured.source()).unwrap() {
            Expectation::NewInput => {}
            Expectation::SameInput { .. } => return,
        }
        let mut job = SourcedJob {
            key,
            input: captured,
            compute: |input: &String, _: &WorkerContext<'_>| {
                // 입력마다 다른 짧은 지연으로 결과 도착 순서를 섞습니다.
                let spin: u64 = input.bytes().map(u64::from).sum::<u64>() % 5;
                thread::sleep(Duration::from_micros(20 * spin));
                input.clone()
            },
        };
        loop {
            match pool.try_submit_sourced(WorkerClass::TextShaping, tx, job) {
                Ok(_) => break,
                Err(rejected) => {
                    job = rejected.payload;
                    self.judge(rx);
                    thread::yield_now();
                }
            }
        }
        self.submitted += 1;
    }
}

#[test]
fn rapid_edit_stress_never_applies_stale_results_and_converges() {
    const KEYS: usize = 16;
    const EDITS: u64 = 3_000;
    let pool = WorkerPool::new(WorkerPoolConfig::new(4).unwrap()).unwrap();
    let (tx, rx) = worker_result_channel::<usize, String>();
    let mut consumer = StressConsumer {
        gate: WorkerResultGate::new(),
        doc: (0..KEYS).map(|key| format!("k{key}:init")).collect(),
        applied: vec![None; KEYS],
        submitted: 0,
    };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut revision = 1;
    for key in 0..KEYS {
        consumer.refresh(&pool, &tx, &rx, revision, key);
    }
    for step in 0..EDITS {
        revision += 1;
        let key = (rng.next() % KEYS as u64) as usize;
        // 3/4는 key 입력을 바꾸고, 1/4는 다른 곳만 바꿉니다(입력 그대로, revision만 증가).
        if !rng.next().is_multiple_of(4) {
            consumer.doc[key] = format!("k{key}:{step}");
        }
        consumer.refresh(&pool, &tx, &rx, revision, key);
        // 가끔 모든 key를 같은 revision으로 다시 확인합니다(입력이 같으면 진행 중 결과 유지).
        if step % 97 == 0 {
            for other in 0..KEYS {
                consumer.refresh(&pool, &tx, &rx, revision, other);
            }
        }
        consumer.judge(&rx);
        // 편집 사이에 가끔 쉬어 결과가 제때 도착해 적용되는 경우도 섞습니다.
        if step % 8 == 0 {
            thread::sleep(Duration::from_micros(150));
        }
    }
    assert_eq!(pool.wait_idle(TIMEOUT), Ok(true));
    consumer.judge(&rx);

    // 편집이 멈추면 모든 key가 최신 입력의 결과로 수렴합니다.
    for key in 0..KEYS {
        assert_eq!(
            consumer.applied[key].as_deref(),
            Some(consumer.doc[key].as_str()),
            "key {key}가 최신 입력으로 수렴하지 않았습니다"
        );
    }
    let stats = consumer.gate.stats();
    assert_eq!(stats.applied + stats.stale, consumer.submitted);
    assert!(stats.stale > 0, "rapid edit이 늦은 결과를 만들어야 합니다");
    assert_eq!(pool.stats().completed, consumer.submitted);
}
