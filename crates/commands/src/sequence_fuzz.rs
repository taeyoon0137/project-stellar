#![cfg(any(test, feature = "test-support"))]
//! **테스트 전용** command sequence fuzz target (XVI.6.7).
//!
//! seed 하나가 결정론적으로 정하는 긴 command sequence를 실행합니다. 외부 fuzz 엔진·crate 없이
//! document test_support의 SplitMix64와 commands 생성기(VII.6.1)만 씁니다.
//!
//! seed마다([`run_sequence`]):
//!
//! 1. random valid document(VI.10.5)에서 시작해 `steps`번 무작위 동작을 고릅니다: 요청 1~3개짜리 새
//!    transaction, canonical bytes를 변조(bit flip·절단)해 디코딩된 transaction, undo(역 transaction
//!    재실행), redo(정방향 재실행).
//! 2. 단계마다 불변식을 확인합니다. 하나라도 어기면 **panic**합니다:
//!    - 성공한 실행 뒤 문서는 전체 validator(VI.10.1)를 통과하고 revision은 정확히 1 오릅니다.
//!    - 거부된 transaction은 문서 semantic snapshot과 revision을 바꾸지 않습니다(원자성, VII.1.6).
//!    - undo는 그 transaction 직전 snapshot으로, redo는 직후 snapshot으로 돌아옵니다.
//! 3. 끝에서 전체 undo가 초기 snapshot, 이어진 전체 redo가 최종 snapshot을 만듭니다.
//!
//! 장기 실행([`run_campaign`])은 seed 범위·시간 예산을 받아 seed마다 panic을 잡고, 실패한 seed를
//! **crash artifact 파일**(seed·단계 수·panic 메시지·재현 명령)로 저장합니다. seed만으로 실행이 완전히
//! 재현되므로 artifact는 seed를 담는 것으로 충분합니다. nightly 실행은 `#[ignore]` test
//! `sequence_fuzz_nightly`를 환경 변수([`SequenceFuzzConfig::from_env`])로 돌립니다 — CI job 등록은 이
//! 저장소 정책상 하지 않았습니다(보고서 XVI.6.7).

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stellar_core_types::ids::StableId;
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::validate::validate_document;

use crate::id::{CommandId, CommandOrigin};
use crate::state::DocumentState;
use crate::test_support::{random_valid_request, FreshIds};
use crate::transaction::CommandTransaction;

/// seed 하나의 실행 통계.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequenceStats {
    /// 확정된 새 transaction 수(변조 디코딩 포함).
    pub committed: usize,
    /// 거부된 transaction 수(원자성 확인 포함).
    pub rejected: usize,
    /// 변조 bytes가 transaction으로 디코딩된 수.
    pub decoded_mutations: usize,
    /// 단계 중 undo 수.
    pub undos: usize,
    /// 단계 중 redo 수.
    pub redos: usize,
    /// 끝에서 되돌린(전체 undo) 항목 수.
    pub final_undo_depth: usize,
}

impl SequenceStats {
    fn add(&mut self, other: SequenceStats) {
        self.committed += other.committed;
        self.rejected += other.rejected;
        self.decoded_mutations += other.decoded_mutations;
        self.undos += other.undos;
        self.redos += other.redos;
        self.final_undo_depth += other.final_undo_depth;
    }
}

/// 확정된 transaction 하나: 정방향, 역방향, 직전·직후 snapshot.
struct Applied {
    forward: CommandTransaction,
    inverse: CommandTransaction,
    before: String,
    after: String,
}

fn snapshot(state: &DocumentState) -> String {
    model_snapshot_json(state.model()).unwrap_or_else(|e| panic!("snapshot 실패: {e}"))
}

fn assert_valid(state: &DocumentState, context: &str) {
    let records = state
        .model()
        .to_records()
        .unwrap_or_else(|e| panic!("{context}: 레코드 생성 실패: {e}"));
    if let Err(error) = validate_document(&records) {
        panic!("{context}: 문서 불변식 위반: {error}");
    }
}

fn transaction_id(seed: u64, step: usize) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(
        3_000_000_000_000 + seed % 1_000_000_000,
        (step & 0x0FFF) as u16,
        seed,
    ))
}

/// 요청 1~3개짜리 후보 transaction. 각 요청은 현재 문서 기준 유효 후보라 뒤 요청은 앞 요청과 충돌해
/// 거부될 수 있습니다(원자성 확인 대상).
fn candidate(
    state: &DocumentState,
    seed: u64,
    step: usize,
    rng: &mut SplitMix64,
    ids: &mut FreshIds,
) -> Option<CommandTransaction> {
    let mut transaction = CommandTransaction::new(transaction_id(seed, step), CommandOrigin::User);
    for _ in 0..1 + rng.below(3) {
        if let Some(request) = random_valid_request(state, rng, ids, 16) {
            transaction.requests.push(request);
        }
    }
    (!transaction.requests.is_empty()).then_some(transaction)
}

/// canonical bytes를 변조해 디코딩되는 transaction(대부분은 디코딩 실패).
fn mutated(transaction: &CommandTransaction, rng: &mut SplitMix64) -> Option<CommandTransaction> {
    let mut bytes = transaction.encode();
    if bytes.is_empty() {
        return None;
    }
    let at = rng.below(bytes.len() as u64) as usize;
    if rng.one_in(3) {
        bytes.truncate(at);
    } else {
        bytes[at] ^= 1 << rng.below(8);
    }
    CommandTransaction::decode(&bytes).ok()
}

/// transaction을 실행하고 불변식을 확인합니다. 성공하면 확정 기록을 돌려줍니다.
fn execute_checked(
    state: &mut DocumentState,
    transaction: CommandTransaction,
    context: &str,
) -> Option<Applied> {
    let before = snapshot(state);
    let revision = state.revision();
    match state.execute_transaction(&transaction) {
        Ok(applied) => {
            assert_eq!(
                state.revision().0,
                revision.0 + 1,
                "{context}: revision은 transaction당 1"
            );
            assert_valid(state, context);
            Some(Applied {
                forward: transaction,
                inverse: applied.inverse,
                before,
                after: snapshot(state),
            })
        }
        Err(_) => {
            assert_eq!(
                state.revision(),
                revision,
                "{context}: 거부는 revision 유지"
            );
            assert_eq!(
                snapshot(state),
                before,
                "{context}: 거부는 문서 불변(원자성)"
            );
            None
        }
    }
}

/// 기록된 transaction을 현재 revision으로 rebase해 재실행합니다. 실패하면 panic입니다.
fn replay(state: &mut DocumentState, transaction: &CommandTransaction, context: &str) {
    let rebased = transaction.rebased(state.revision());
    if let Err(error) = state.execute_transaction(&rebased) {
        panic!("{context}: 재실행 실패: {error}");
    }
    assert_valid(state, context);
}

/// seed 하나의 command sequence를 실행합니다. 불변식을 어기면 panic합니다(결정론적).
pub fn run_sequence(seed: u64, steps: usize) -> SequenceStats {
    let mut state = DocumentState::new(random_valid_document(seed % 1_000_000));
    let mut rng = SplitMix64::new(seed ^ 0x16_6700_5EED);
    let mut ids = FreshIds::new(seed ^ 0x16_6701);
    let mut stats = SequenceStats::default();
    let mut undo: Vec<Applied> = Vec::new();
    let mut redo: Vec<Applied> = Vec::new();
    let initial = snapshot(&state);
    assert_valid(&state, &format!("seed {seed} 시작"));
    for step in 0..steps {
        let context = format!("seed {seed} step {step}");
        match rng.below(10) {
            // 새 transaction(요청 1~3개).
            0..=4 => {
                let Some(transaction) = candidate(&state, seed, step, &mut rng, &mut ids) else {
                    continue;
                };
                match execute_checked(&mut state, transaction, &context) {
                    Some(applied) => {
                        stats.committed += 1;
                        undo.push(applied);
                        redo.clear();
                    }
                    None => stats.rejected += 1,
                }
            }
            // 변조 bytes transaction.
            5 | 6 => {
                let Some(transaction) = candidate(&state, seed, step, &mut rng, &mut ids) else {
                    continue;
                };
                let Some(decoded) = mutated(&transaction, &mut rng) else {
                    continue;
                };
                stats.decoded_mutations += 1;
                let decoded = decoded.rebased(state.revision());
                match execute_checked(&mut state, decoded, &context) {
                    Some(applied) => {
                        stats.committed += 1;
                        undo.push(applied);
                        redo.clear();
                    }
                    None => stats.rejected += 1,
                }
            }
            // undo.
            7 | 8 => {
                let Some(applied) = undo.pop() else {
                    continue;
                };
                replay(&mut state, &applied.inverse, &format!("{context} undo"));
                assert_eq!(snapshot(&state), applied.before, "{context}: undo 결과");
                stats.undos += 1;
                redo.push(applied);
            }
            // redo.
            _ => {
                let Some(applied) = redo.pop() else {
                    continue;
                };
                replay(&mut state, &applied.forward, &format!("{context} redo"));
                assert_eq!(snapshot(&state), applied.after, "{context}: redo 결과");
                stats.redos += 1;
                undo.push(applied);
            }
        }
    }
    // 끝: 전체 undo → 초기, 전체 redo → 최종(남은 redo 갈래 포함 전부 다시 적용).
    while let Some(applied) = redo.pop() {
        replay(
            &mut state,
            &applied.forward,
            &format!("seed {seed} 마무리 redo"),
        );
        undo.push(applied);
    }
    let last = snapshot(&state);
    stats.final_undo_depth = undo.len();
    for applied in undo.iter().rev() {
        replay(
            &mut state,
            &applied.inverse,
            &format!("seed {seed} 전체 undo"),
        );
    }
    assert_eq!(snapshot(&state), initial, "seed {seed}: 전체 undo ≠ 초기");
    for applied in &undo {
        replay(
            &mut state,
            &applied.forward,
            &format!("seed {seed} 전체 redo"),
        );
    }
    assert_eq!(snapshot(&state), last, "seed {seed}: 전체 redo ≠ 최종");
    stats
}

/// 장기 실행 설정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceFuzzConfig {
    /// 첫 seed.
    pub first_seed: u64,
    /// 실행할 seed 수(시간 예산이 먼저 끝나면 그 전에 멈춤).
    pub seeds: u64,
    /// seed당 단계 수.
    pub steps: usize,
    /// 새 seed를 시작하지 않는 시간 예산(`None`이면 seed 수만 따름).
    pub time_budget: Option<Duration>,
    /// crash artifact 저장 디렉터리(`None`이면 저장하지 않고 보고만 함).
    pub artifact_dir: Option<PathBuf>,
}

/// 환경 변수 이름.
pub mod env {
    /// 첫 seed.
    pub const FIRST_SEED: &str = "STELLAR_SEQUENCE_FUZZ_FIRST_SEED";
    /// seed 수.
    pub const SEEDS: &str = "STELLAR_SEQUENCE_FUZZ_SEEDS";
    /// seed당 단계 수.
    pub const STEPS: &str = "STELLAR_SEQUENCE_FUZZ_STEPS";
    /// 시간 예산(초).
    pub const SECONDS: &str = "STELLAR_SEQUENCE_FUZZ_SECONDS";
    /// crash artifact 디렉터리.
    pub const ARTIFACT_DIR: &str = "STELLAR_SEQUENCE_FUZZ_ARTIFACT_DIR";
}

impl SequenceFuzzConfig {
    /// 기본 test 실행에 포함하는 smoke 범위(XVI.6.7 로컬 smoke).
    pub const SMOKE_SEEDS: u64 = 48;
    /// smoke·nightly 기본 단계 수.
    pub const DEFAULT_STEPS: usize = 32;
    /// nightly 기본 seed 수.
    pub const NIGHTLY_SEEDS: u64 = 100_000;
    /// nightly 기본 시간 예산(초).
    pub const NIGHTLY_SECONDS: u64 = 3_600;

    /// smoke 설정(artifact 저장 없음).
    pub fn smoke() -> SequenceFuzzConfig {
        SequenceFuzzConfig {
            first_seed: 0,
            seeds: SequenceFuzzConfig::SMOKE_SEEDS,
            steps: SequenceFuzzConfig::DEFAULT_STEPS,
            time_budget: None,
            artifact_dir: None,
        }
    }

    /// nightly 설정: 환경 변수가 있으면 그 값, 없으면 기본값. artifact는 기본으로 workspace
    /// `target/fuzz-artifacts/command-sequence`(Git 제외 경로)에 저장합니다.
    pub fn from_env() -> SequenceFuzzConfig {
        let number = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(default)
        };
        let artifact_dir = std::env::var_os(env::ARTIFACT_DIR)
            .map(PathBuf::from)
            .unwrap_or_else(default_artifact_dir);
        SequenceFuzzConfig {
            first_seed: number(env::FIRST_SEED, 0),
            seeds: number(env::SEEDS, SequenceFuzzConfig::NIGHTLY_SEEDS),
            steps: number(env::STEPS, SequenceFuzzConfig::DEFAULT_STEPS as u64) as usize,
            time_budget: Some(Duration::from_secs(number(
                env::SECONDS,
                SequenceFuzzConfig::NIGHTLY_SECONDS,
            ))),
            artifact_dir: Some(artifact_dir),
        }
    }
}

fn default_artifact_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    target.join("fuzz-artifacts").join("command-sequence")
}

/// 실패한 seed 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashArtifact {
    /// 실패한 seed.
    pub seed: u64,
    /// seed당 단계 수.
    pub steps: usize,
    /// panic 메시지.
    pub message: String,
    /// 저장한 artifact 파일(저장하지 않았으면 `None`).
    pub path: Option<PathBuf>,
}

impl CrashArtifact {
    /// artifact 파일 이름(seed 16진수).
    pub fn file_name(seed: u64) -> String {
        format!("command-sequence-seed-{seed:016x}.txt")
    }

    /// artifact 파일 내용: seed·단계 수·메시지·재현 명령. 결정론적 재현에는 seed와 단계 수면 충분합니다.
    pub fn render(&self) -> String {
        format!(
            "stellar command sequence fuzz crash\nseed: {}\nsteps: {}\nmessage: {}\nreplay: {}={} {}=1 {}={} cargo test -p stellar-commands sequence_fuzz_nightly -- --ignored --nocapture\n",
            self.seed,
            self.steps,
            self.message.replace('\n', " "),
            env::FIRST_SEED,
            self.seed,
            env::SEEDS,
            env::STEPS,
            self.steps,
        )
    }

    /// [`CrashArtifact::render`] 내용에서 seed·단계 수를 읽습니다(재현용).
    pub fn parse_replay(text: &str) -> Option<(u64, usize)> {
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::trim)
        };
        Some((
            field("seed:")?.parse().ok()?,
            field("steps:")?.parse().ok()?,
        ))
    }
}

/// 장기 실행 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceFuzzReport {
    /// 실행한 seed 수.
    pub seeds_run: u64,
    /// 통과한 seed의 통계 합.
    pub stats: SequenceStats,
    /// 실패한 seed(발견 순).
    pub crashes: Vec<CrashArtifact>,
    /// artifact 저장 실패(경로와 오류 메시지).
    pub artifact_errors: Vec<String>,
}

impl fmt::Display for SequenceFuzzReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "seed {}개, crash {}개, {:?}",
            self.seeds_run,
            self.crashes.len(),
            self.stats
        )
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic payload를 문자열로 읽을 수 없음".to_string())
}

fn save_artifact(dir: &Path, crash: &CrashArtifact) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(CrashArtifact::file_name(crash.seed));
    std::fs::write(&path, crash.render())?;
    Ok(path)
}

/// seed 범위를 `runner`로 실행합니다. seed마다 panic을 잡아 crash로 기록하고, artifact 디렉터리가 있으면
/// crash artifact 파일을 저장합니다. 다음 seed로 계속 진행합니다.
pub fn run_campaign<F>(config: &SequenceFuzzConfig, runner: F) -> SequenceFuzzReport
where
    F: Fn(u64, usize) -> SequenceStats,
{
    let started = Instant::now();
    let mut report = SequenceFuzzReport::default();
    for offset in 0..config.seeds {
        if config
            .time_budget
            .is_some_and(|budget| started.elapsed() >= budget)
        {
            break;
        }
        let Some(seed) = config.first_seed.checked_add(offset) else {
            break;
        };
        report.seeds_run += 1;
        match catch_unwind(AssertUnwindSafe(|| runner(seed, config.steps))) {
            Ok(stats) => report.stats.add(stats),
            Err(payload) => {
                let mut crash = CrashArtifact {
                    seed,
                    steps: config.steps,
                    message: panic_message(payload.as_ref()),
                    path: None,
                };
                if let Some(dir) = &config.artifact_dir {
                    match save_artifact(dir, &crash) {
                        Ok(path) => crash.path = Some(path),
                        Err(error) => report
                            .artifact_errors
                            .push(format!("{}: {error}", dir.display())),
                    }
                }
                report.crashes.push(crash);
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn smoke_command_sequence_fuzz_is_crash_free() {
        // 검증(XVI.6.7 로컬 smoke): 정해진 seed 범위에서 crash 0, 경로 분포 하한.
        let config = SequenceFuzzConfig::smoke();
        let report = run_campaign(&config, run_sequence);
        eprintln!("command sequence fuzz smoke: {report}");
        assert!(report.crashes.is_empty(), "{:?}", report.crashes);
        assert_eq!(report.seeds_run, SequenceFuzzConfig::SMOKE_SEEDS);
        let stats = report.stats;
        assert!(stats.committed > 300, "{stats:?}");
        assert!(stats.rejected > 20, "{stats:?}");
        assert!(stats.decoded_mutations > 20, "{stats:?}");
        assert!(stats.undos > 100 && stats.redos > 15, "{stats:?}");
        assert!(stats.final_undo_depth > 100, "{stats:?}");
    }

    #[test]
    fn same_seed_replays_identically() {
        // seed만으로 재현: 같은 seed는 같은 통계, 다른 seed는 대체로 다른 경로.
        for seed in [3u64, 41, 977] {
            assert_eq!(
                run_sequence(seed, 24),
                run_sequence(seed, 24),
                "seed {seed}"
            );
        }
        assert_ne!(run_sequence(1, 24), run_sequence(2, 24));
    }

    #[test]
    fn crash_seed_is_saved_as_artifact_and_replays() {
        // 검증 기준(XVI.6.7): crash seed 자동 artifact 저장. 주입한 실패 runner로 확인합니다.
        let dir = TempDir::new("stellar-sequence-fuzz-artifacts");
        let config = SequenceFuzzConfig {
            first_seed: 10,
            seeds: 6,
            steps: 5,
            time_budget: None,
            artifact_dir: Some(dir.0.join("nested")),
        };
        let faulty = |seed: u64, steps: usize| {
            if seed % 4 == 1 {
                panic!("주입한 불변식 위반 seed {seed}");
            }
            SequenceStats {
                committed: steps,
                ..SequenceStats::default()
            }
        };
        let report = run_campaign(&config, faulty);
        assert_eq!(report.seeds_run, 6);
        assert!(
            report.artifact_errors.is_empty(),
            "{:?}",
            report.artifact_errors
        );
        let seeds: Vec<u64> = report.crashes.iter().map(|c| c.seed).collect();
        assert_eq!(seeds, vec![13], "10..16 중 seed % 4 == 1");
        assert_eq!(report.stats.committed, 5 * 5, "통과 seed 5개는 계속 진행");
        let crash = &report.crashes[0];
        assert!(crash.message.contains("주입한 불변식 위반 seed 13"));
        let path = crash.path.clone().expect("artifact 저장");
        assert_eq!(
            path,
            dir.0.join("nested").join(CrashArtifact::file_name(13))
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, crash.render());
        assert!(text.contains("STELLAR_SEQUENCE_FUZZ_FIRST_SEED=13"));
        // artifact만으로 같은 실패를 재현합니다.
        let (seed, steps) = CrashArtifact::parse_replay(&text).unwrap();
        assert_eq!((seed, steps), (13, 5));
        assert!(catch_unwind(|| faulty(seed, steps)).is_err());
        // 저장 디렉터리가 없으면 보고만 합니다.
        let in_memory = run_campaign(
            &SequenceFuzzConfig {
                artifact_dir: None,
                ..config.clone()
            },
            faulty,
        );
        assert_eq!(in_memory.crashes.len(), 1);
        assert_eq!(in_memory.crashes[0].path, None);
        // 시간 예산 0이면 seed를 시작하지 않습니다.
        let none = run_campaign(
            &SequenceFuzzConfig {
                time_budget: Some(Duration::ZERO),
                ..config
            },
            faulty,
        );
        assert_eq!(none.seeds_run, 0);
    }

    #[test]
    #[ignore = "장기 nightly 실행: 환경 변수로 seed 범위·시간 예산·artifact 경로를 지정합니다"]
    fn sequence_fuzz_nightly() {
        let config = SequenceFuzzConfig::from_env();
        let report = run_campaign(&config, run_sequence);
        eprintln!("command sequence fuzz nightly: {report}");
        for crash in &report.crashes {
            eprintln!(
                "crash seed {} → {}",
                crash.seed,
                crash
                    .path
                    .as_deref()
                    .map_or("(저장 안 됨)".into(), |p| p.display().to_string())
            );
        }
        assert!(
            report.artifact_errors.is_empty(),
            "{:?}",
            report.artifact_errors
        );
        assert!(
            report.crashes.is_empty(),
            "crash {}개",
            report.crashes.len()
        );
    }
}
