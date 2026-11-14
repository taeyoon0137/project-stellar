//! worker result 출처(source revision·input hash)와 stale result 거부 (XII.1.5).
//!
//! 아키텍처 §9.2: "워커 결과는 입력 해시와 문서 세대가 현재 상태와 일치할 때만 반영합니다." 이 모듈은
//! worker 결과마다 출처를 붙이고, 결과를 적용하는 쪽(document thread)이 gate를 거쳐서만 값을 꺼내게 합니다.
//!
//! # 흐름
//!
//! 1. **캡처**: 결과를 적용할 thread가 현재 문서 revision에서 job 입력을 캡처합니다
//!    ([`CapturedInput::capture`]). 출처([`WorkerSource`])는 그 revision과 **worker에 넘기는 그 입력**의
//!    hash([`WorkerInput::input_hash`])입니다. 따라서 결과의 출처와 실제 계산 입력이 어긋나지 않습니다.
//! 2. **기대**: 같은 thread가 [`WorkerResultGate::expect`]로 "key의 현재 입력은 이 출처"라고 알립니다.
//!    입력 hash가 바뀌었으면 [`Expectation::NewInput`]이고 job을 제출합니다. 같으면
//!    [`Expectation::SameInput`]이며 진행 중인 결과가 그대로 유효합니다(다시 제출하지 않음).
//! 3. **제출**: [`WorkerPool::try_submit_sourced`]가 job을 실행해 결과를 [`WorkerResult`]로 보냅니다.
//! 4. **판정**: [`WorkerResultGate::admit`]만 결과 값을 꺼냅니다. [`WorkerResult`]는 값을 공개하지
//!    않으므로 gate를 거치지 않고 결과를 적용할 수 없습니다.
//!
//! # stale 판정
//!
//! gate는 key마다 현재 입력 hash와, 그 입력이 처음 기대된 revision(`since`)·마지막으로 확인된
//! revision(`latest`)을 기록합니다. 결과는 아래를 모두 만족할 때만 적용합니다([`Admission::Apply`]).
//!
//! | 조건 | 어기면 |
//! |---|---|
//! | key를 기대하고 있음 | [`StaleReason::NotExpected`] |
//! | 결과 입력 hash = 현재 입력 hash | [`StaleReason::InputChanged`] |
//! | `since` ≤ 결과 revision | [`StaleReason::BeforeCurrentInput`] |
//! | 결과 revision ≤ `latest` | [`StaleReason::AheadOfExpectation`] |
//! | 현재 기대에 대해 아직 적용하지 않음 | [`StaleReason::AlreadyApplied`] |
//!
//! `since..=latest`는 key 입력이 같은 hash로 이어진 revision 구간입니다. 다른 node 편집으로 문서 revision이
//! 계속 올라도 key 입력이 같으면 같은 hash로 다시 기대하므로(`latest`만 오름) 진행 중인 결과가 버려지지
//! 않습니다(XI.6.3의 starvation과 같은 문제를 피함). 입력이 A → B → A로 돌아오면 `since`가 새로 정해져
//! 처음 A 구간의 결과는 같은 hash여도 보수적으로 버립니다.
//!
//! # 규약
//!
//! - gate는 한 문서 session에 속하고 key는 작업 종류까지 구분해야 합니다. reload로 revision이
//!   초기화되면 이전 결과 channel과 gate를 함께 버리고 새 session을 만듭니다.
//! - 캡처 입력은 계산이 끝날 때까지 불변이어야 합니다. hash는 결과에 영향을 주는 모든 입력을 담고,
//!   계산은 hash에 없는 외부 가변 상태를 읽지 않습니다.
//! - 결과를 적용하는 thread는 key 입력이 바뀔 수 있는 revision마다 `expect`를 부르거나, key가 사라지면
//!   [`WorkerResultGate::forget`]을 부릅니다. 캡처 → `expect` → (`NewInput`이면) 제출 순서를 지킵니다.
//!   `expect` 없이 더 새 revision에서 캡처한 결과는 `AheadOfExpectation`으로 버립니다.
//! - 제출이 거부되면([`WorkerRejected`]) 돌려받은 [`SourcedJob`]을 그대로 다시 제출합니다(출처 유지).
//! - job이 panic하면(dev·test profile) 결과가 오지 않습니다. 다음 입력 변경의 `NewInput`에서 다시
//!   제출합니다.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use stellar_core_types::revision::DocumentRevision;
use stellar_renderer_core::thread_role::ThreadRole;

use crate::worker_pool::{
    ensure_unbound_thread, WorkerClass, WorkerContext, WorkerJobId, WorkerJobSpec, WorkerPool,
    WorkerRejected,
};

// ---------------------------------------------------------------------------
// 출처
// ---------------------------------------------------------------------------

/// worker 입력 hash(128비트).
///
/// [`WorkerInputHash::of_bytes`]는 입력 canonical bytes의 FNV-1a 128 hash입니다. 이미 두 개의 64비트
/// hash를 가진 입력(예: layout의 primary·check hash)은 [`WorkerInputHash::from_parts`]로 옮깁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerInputHash(u128);

impl WorkerInputHash {
    /// FNV-1a 128 offset basis.
    const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    /// FNV-1a 128 prime(2^88 + 2^8 + 0x3b).
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    /// canonical bytes의 FNV-1a 128 hash.
    pub fn of_bytes(bytes: &[u8]) -> WorkerInputHash {
        let mut state = WorkerInputHash::OFFSET_BASIS;
        for byte in bytes {
            state ^= u128::from(*byte);
            state = state.wrapping_mul(WorkerInputHash::PRIME);
        }
        WorkerInputHash(state)
    }

    /// 두 64비트 hash(상위·하위)로 만듭니다.
    pub const fn from_parts(high: u64, low: u64) -> WorkerInputHash {
        WorkerInputHash(((high as u128) << 64) | low as u128)
    }

    /// (상위, 하위) 64비트.
    pub const fn to_parts(self) -> (u64, u64) {
        ((self.0 >> 64) as u64, self.0 as u64)
    }
}

impl fmt::Display for WorkerInputHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// worker job 입력. 입력 hash는 결정적이어야 합니다(같은 입력 → 같은 hash).
pub trait WorkerInput: Send + 'static {
    /// 입력 hash.
    fn input_hash(&self) -> WorkerInputHash;
}

impl WorkerInput for Vec<u8> {
    fn input_hash(&self) -> WorkerInputHash {
        WorkerInputHash::of_bytes(self)
    }
}

impl WorkerInput for String {
    fn input_hash(&self) -> WorkerInputHash {
        WorkerInputHash::of_bytes(self.as_bytes())
    }
}

/// worker 결과의 출처: 입력을 캡처한 문서 revision과 입력 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkerSource {
    /// 입력을 캡처한 문서 revision.
    pub revision: DocumentRevision,
    /// 입력 hash.
    pub input_hash: WorkerInputHash,
}

/// 문서 revision에서 캡처한 job 입력. 출처는 이 입력에서 한 번 계산합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedInput<I> {
    source: WorkerSource,
    input: I,
}

impl<I: WorkerInput> CapturedInput<I> {
    /// `revision`에서 `input`을 캡처합니다.
    pub fn capture(revision: DocumentRevision, input: I) -> CapturedInput<I> {
        let input_hash = input.input_hash();
        CapturedInput {
            source: WorkerSource {
                revision,
                input_hash,
            },
            input,
        }
    }
}

impl<I> CapturedInput<I> {
    /// 출처.
    pub fn source(&self) -> WorkerSource {
        self.source
    }

    /// 입력.
    pub fn input(&self) -> &I {
        &self.input
    }

    /// 입력을 꺼냅니다.
    pub fn into_input(self) -> I {
        self.input
    }
}

// ---------------------------------------------------------------------------
// 결과와 channel
// ---------------------------------------------------------------------------

/// worker 결과. key·출처·class는 볼 수 있지만 값은 [`WorkerResultGate::admit`]을 통과해야 꺼냅니다.
pub struct WorkerResult<K, T> {
    key: K,
    source: WorkerSource,
    class: WorkerClass,
    value: T,
}

impl<K, T> WorkerResult<K, T> {
    pub(crate) fn new(key: K, source: WorkerSource, class: WorkerClass, value: T) -> Self {
        WorkerResult {
            key,
            source,
            class,
            value,
        }
    }

    /// 결과 key.
    pub fn key(&self) -> &K {
        &self.key
    }

    /// 출처.
    pub fn source(&self) -> WorkerSource {
        self.source
    }

    /// job class.
    pub fn class(&self) -> WorkerClass {
        self.class
    }
}

impl<K: fmt::Debug, T> fmt::Debug for WorkerResult<K, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerResult")
            .field("key", &self.key)
            .field("source", &self.source)
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

/// 결과 송신 쪽(worker job이 복제해 가짐).
pub struct WorkerResultSender<K, T> {
    tx: Sender<WorkerResult<K, T>>,
}

impl<K, T> Clone for WorkerResultSender<K, T> {
    fn clone(&self) -> Self {
        WorkerResultSender {
            tx: self.tx.clone(),
        }
    }
}

impl<K, T> fmt::Debug for WorkerResultSender<K, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkerResultSender")
    }
}

/// 결과 수신 쪽(결과를 적용하는 thread가 소유).
pub struct WorkerResultReceiver<K, T> {
    rx: Receiver<WorkerResult<K, T>>,
}

impl<K, T> fmt::Debug for WorkerResultReceiver<K, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkerResultReceiver")
    }
}

/// 결과 channel을 만듭니다.
pub fn worker_result_channel<K, T>() -> (WorkerResultSender<K, T>, WorkerResultReceiver<K, T>) {
    let (tx, rx) = mpsc::channel();
    (WorkerResultSender { tx }, WorkerResultReceiver { rx })
}

/// 결과 대기 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultWaitError {
    /// 현재 thread 역할에서는 blocking 대기가 금지됩니다.
    BlockingNotAllowed {
        /// 현재 thread 역할.
        role: ThreadRole,
    },
}

impl fmt::Display for ResultWaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResultWaitError::BlockingNotAllowed { role } => {
                write!(f, "{role:?} thread에서는 worker 결과를 기다릴 수 없습니다")
            }
        }
    }
}

impl std::error::Error for ResultWaitError {}

impl<K, T> WorkerResultReceiver<K, T> {
    /// 도착한 결과 하나를 기다리지 않고 꺼냅니다.
    pub fn try_recv(&self) -> Option<WorkerResult<K, T>> {
        self.rx.try_recv().ok()
    }

    /// 도착한 결과를 기다리지 않고 모두 꺼냅니다(도착 순서).
    pub fn drain(&self) -> Vec<WorkerResult<K, T>> {
        self.rx.try_iter().collect()
    }

    /// 결과 하나를 `timeout`까지 기다립니다. 시간이 지나거나 모든 송신 쪽이 닫히면 `None`.
    ///
    /// 역할이 없는 thread(테스트·headless 도구)에서만 쓸 수 있습니다. document·UI·render thread는
    /// [`WorkerResultReceiver::try_recv`]로만 확인합니다.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<WorkerResult<K, T>>, ResultWaitError> {
        ensure_unbound_thread().map_err(|role| ResultWaitError::BlockingNotAllowed { role })?;
        match self.rx.recv_timeout(timeout) {
            Ok(result) => Ok(Some(result)),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// 출처를 붙여 제출
// ---------------------------------------------------------------------------

/// 출처를 붙인 job. 제출이 거부되면 그대로 돌려받아 다시 제출합니다.
pub struct SourcedJob<K, I, F> {
    /// 결과 key.
    pub key: K,
    /// 캡처한 입력.
    pub input: CapturedInput<I>,
    /// 계산.
    pub compute: F,
}

impl<K: fmt::Debug, I, F> fmt::Debug for SourcedJob<K, I, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourcedJob")
            .field("key", &self.key)
            .field("source", &self.input.source)
            .finish_non_exhaustive()
    }
}

impl WorkerPool {
    /// 캡처한 입력으로 `compute`를 실행하고 결과를 출처와 함께 `results`로 보내는 job을 기다리지 않고
    /// 접수합니다. 거부되면 job을 그대로 돌려줍니다.
    pub fn try_submit_sourced<K, I, T, F>(
        &self,
        spec: impl Into<WorkerJobSpec>,
        results: &WorkerResultSender<K, T>,
        job: SourcedJob<K, I, F>,
    ) -> Result<WorkerJobId, WorkerRejected<SourcedJob<K, I, F>>>
    where
        K: Send + 'static,
        I: WorkerInput,
        T: Send + 'static,
        F: for<'a> FnOnce(&I, &WorkerContext<'a>) -> T + Send + 'static,
    {
        let tx = results.tx.clone();
        self.try_submit_with(spec, job, move |job, context| {
            let SourcedJob {
                key,
                input,
                compute,
            } = job;
            let value = compute(&input.input, context);
            let result = WorkerResult::new(key, input.source, context.spec().class, value);
            // 결과를 받을 쪽이 사라졌으면 버립니다(정상 종료 경로).
            let _ = tx.send(result);
        })
    }
}

// ---------------------------------------------------------------------------
// gate
// ---------------------------------------------------------------------------

/// [`WorkerResultGate::expect`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expectation {
    /// 입력이 새로 정해졌습니다. 이 출처로 job을 제출합니다.
    NewInput,
    /// 입력 hash가 같습니다. 진행 중이거나 이미 적용한 결과가 그대로 유효합니다.
    SameInput {
        /// 현재 입력의 결과를 이미 적용했는지.
        applied: bool,
    },
}

/// gate 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    /// 기대 revision을 이전 값으로 되돌릴 수 없습니다.
    RevisionRegressed {
        /// 마지막으로 확인한 revision.
        latest: DocumentRevision,
        /// 요청 revision.
        requested: DocumentRevision,
    },
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::RevisionRegressed { latest, requested } => write!(
                f,
                "worker 결과 기대 revision이 역행했습니다: {} → {}",
                latest.0, requested.0
            ),
        }
    }
}

impl std::error::Error for GateError {}

/// 결과를 버린 이유(모듈 문서의 stale 판정 표).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StaleReason {
    /// key를 기대하지 않습니다(기대한 적 없거나 잊음).
    NotExpected,
    /// 결과 입력 hash가 현재 입력 hash와 다릅니다.
    InputChanged {
        /// 현재 입력 hash.
        current: WorkerInputHash,
    },
    /// 현재 입력이 정해지기 전 revision에서 캡처한 결과입니다.
    BeforeCurrentInput {
        /// 현재 입력이 처음 기대된 revision.
        since: DocumentRevision,
    },
    /// 아직 확인하지 않은 더 새 revision에서 캡처한 결과입니다.
    AheadOfExpectation {
        /// 마지막으로 확인한 revision.
        latest: DocumentRevision,
    },
    /// 현재 입력의 결과를 이미 적용했습니다(중복 결과).
    AlreadyApplied,
}

impl StaleReason {
    /// 고정 이름(fixture·진단용).
    pub const fn name(&self) -> &'static str {
        match self {
            StaleReason::NotExpected => "not-expected",
            StaleReason::InputChanged { .. } => "input-changed",
            StaleReason::BeforeCurrentInput { .. } => "before-current-input",
            StaleReason::AheadOfExpectation { .. } => "ahead-of-expectation",
            StaleReason::AlreadyApplied => "already-applied",
        }
    }
}

/// gate 판정.
#[derive(Debug, PartialEq, Eq)]
pub enum Admission<K, T> {
    /// 적용합니다.
    Apply {
        /// key.
        key: K,
        /// 출처.
        source: WorkerSource,
        /// 결과 값.
        value: T,
    },
    /// 오래된 결과라 버렸습니다(값은 이미 drop됨).
    Stale {
        /// key.
        key: K,
        /// 출처.
        source: WorkerSource,
        /// 이유.
        reason: StaleReason,
    },
}

/// gate 집계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateStats {
    /// 적용한 결과 수.
    pub applied: u64,
    /// 버린 결과 수.
    pub stale: u64,
    /// 이유별 버린 수(`not-expected`, `input-changed`, `before-current-input`, `ahead-of-expectation`,
    /// `already-applied` 순).
    pub stale_by_reason: [u64; 5],
}

impl GateStats {
    fn record_stale(&mut self, reason: StaleReason) {
        self.stale += 1;
        let index = match reason {
            StaleReason::NotExpected => 0,
            StaleReason::InputChanged { .. } => 1,
            StaleReason::BeforeCurrentInput { .. } => 2,
            StaleReason::AheadOfExpectation { .. } => 3,
            StaleReason::AlreadyApplied => 4,
        };
        self.stale_by_reason[index] += 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    input_hash: WorkerInputHash,
    since: DocumentRevision,
    latest: DocumentRevision,
    applied: bool,
}

/// key별 현재 입력 기대를 기록하고 worker 결과를 판정하는 gate(모듈 문서).
///
/// 결과를 적용하는 thread(보통 document thread)가 소유합니다.
#[derive(Debug)]
pub struct WorkerResultGate<K> {
    slots: HashMap<K, Slot>,
    stats: GateStats,
}

impl<K> Default for WorkerResultGate<K> {
    fn default() -> Self {
        WorkerResultGate {
            slots: HashMap::new(),
            stats: GateStats::default(),
        }
    }
}

impl<K: Eq + Hash> WorkerResultGate<K> {
    /// 빈 gate.
    pub fn new() -> WorkerResultGate<K> {
        WorkerResultGate::default()
    }

    /// key의 현재 입력이 `source`라고 알립니다(모듈 문서의 규약).
    pub fn expect(&mut self, key: K, source: WorkerSource) -> Result<Expectation, GateError> {
        match self.slots.get_mut(&key) {
            Some(slot) => {
                if source.revision < slot.latest {
                    return Err(GateError::RevisionRegressed {
                        latest: slot.latest,
                        requested: source.revision,
                    });
                }
                if source.input_hash == slot.input_hash {
                    slot.latest = source.revision;
                    return Ok(Expectation::SameInput {
                        applied: slot.applied,
                    });
                }
                *slot = Slot {
                    input_hash: source.input_hash,
                    since: source.revision,
                    latest: source.revision,
                    applied: false,
                };
                Ok(Expectation::NewInput)
            }
            None => {
                self.slots.insert(
                    key,
                    Slot {
                        input_hash: source.input_hash,
                        since: source.revision,
                        latest: source.revision,
                        applied: false,
                    },
                );
                Ok(Expectation::NewInput)
            }
        }
    }

    /// key 기대를 지웁니다(문서에서 사라진 key). 이후 그 key의 결과는 모두 버립니다.
    pub fn forget(&mut self, key: &K) -> bool {
        self.slots.remove(key).is_some()
    }

    /// key의 현재 기대(입력 hash와 마지막으로 확인한 revision).
    pub fn expected(&self, key: &K) -> Option<WorkerSource> {
        self.slots.get(key).map(|slot| WorkerSource {
            revision: slot.latest,
            input_hash: slot.input_hash,
        })
    }

    /// 기대 중인 key 수.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// 기대 중인 key가 없는지.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 집계.
    pub fn stats(&self) -> GateStats {
        self.stats
    }

    /// 결과를 판정합니다. 적용할 결과만 값을 돌려줍니다.
    pub fn admit<T>(&mut self, result: WorkerResult<K, T>) -> Admission<K, T> {
        let WorkerResult {
            key, source, value, ..
        } = result;
        let verdict = match self.slots.get_mut(&key) {
            None => Err(StaleReason::NotExpected),
            Some(slot) => {
                if source.input_hash != slot.input_hash {
                    Err(StaleReason::InputChanged {
                        current: slot.input_hash,
                    })
                } else if source.revision < slot.since {
                    Err(StaleReason::BeforeCurrentInput { since: slot.since })
                } else if source.revision > slot.latest {
                    Err(StaleReason::AheadOfExpectation {
                        latest: slot.latest,
                    })
                } else if slot.applied {
                    Err(StaleReason::AlreadyApplied)
                } else {
                    slot.applied = true;
                    Ok(())
                }
            }
        };
        match verdict {
            Ok(()) => {
                self.stats.applied += 1;
                Admission::Apply { key, source, value }
            }
            Err(reason) => {
                self.stats.record_stale(reason);
                drop(value);
                Admission::Stale {
                    key,
                    source,
                    reason,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
