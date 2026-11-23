//! layout worker result guard (X.7.5).
//!
//! 레이아웃은 worker에서 계산될 수 있고, 그 사이 문서가 바뀌면 늦게 도착한 결과는 현재 문서와 맞지
//! 않습니다. [`LayoutResultGuard`]는 현재 기대 job(= [`DocumentRevision`] + 입력 hash)을 들고 있다가
//! worker 결과의 job이 **둘 다** 일치할 때만 적용합니다. 그 밖의 결과는 이유([`StaleReason`])와 함께
//! 버리며 적용된 결과를 바꾸지 않습니다.
//!
//! - revision이 같아도 입력 hash가 다르면(같은 revision에서 다른 입력으로 계산) 버립니다.
//! - 입력 hash가 같아도 revision이 다르면(이전·미래 revision 결과) 버립니다.
//! - 현재 job의 결과가 이미 적용되었으면 같은 job의 중복 결과도 버립니다.
//! - 기대 revision은 되돌아갈 수 없습니다(역행 갱신은 오류).

use std::fmt;

use stellar_core_types::revision::DocumentRevision;

use crate::cache::{LayoutInputHash, LayoutInputHasher};
use crate::input::LayoutInput;
use crate::output::LayoutResult;
use crate::revision::LayoutRevision;

impl LayoutInput {
    /// canonical bytes의 입력 hash(X.1.4 hasher, X.7.5 job 식별).
    pub fn input_hash(&self) -> LayoutInputHash {
        let mut hasher = LayoutInputHasher::new();
        hasher.write_bytes(&self.encode());
        hasher.finish()
    }
}

/// worker에 보낸 레이아웃 계산 단위 (X.7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutJob {
    /// 계산 기준 문서 revision.
    pub revision: DocumentRevision,
    /// 계산 기준 입력 hash.
    pub input_hash: LayoutInputHash,
}

impl LayoutJob {
    /// 입력으로부터 job을 만듭니다.
    pub fn for_input(revision: DocumentRevision, input: &LayoutInput) -> LayoutJob {
        LayoutJob {
            revision,
            input_hash: input.input_hash(),
        }
    }
}

/// worker가 돌려준 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutWorkerResult {
    /// 계산한 job.
    pub job: LayoutJob,
    /// 레이아웃 결과.
    pub result: LayoutResult,
}

/// 결과를 버린 이유 (X.7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// 문서 revision이 현재 기대와 다릅니다.
    RevisionMismatch {
        /// 기대 revision.
        expected: DocumentRevision,
        /// 결과 revision.
        actual: DocumentRevision,
    },
    /// revision은 같지만 입력 hash가 다릅니다.
    InputHashMismatch {
        /// 기대 hash.
        expected: LayoutInputHash,
        /// 결과 hash.
        actual: LayoutInputHash,
    },
    /// 현재 job의 결과가 이미 적용되었습니다.
    Duplicate,
}

impl fmt::Display for StaleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StaleReason::RevisionMismatch { expected, actual } => write!(
                f,
                "레이아웃 결과 revision {}가 기대 revision {}와 다릅니다",
                actual.0, expected.0
            ),
            StaleReason::InputHashMismatch { expected, actual } => {
                write!(
                    f,
                    "레이아웃 결과 입력 hash {actual}가 기대 {expected}와 다릅니다"
                )
            }
            StaleReason::Duplicate => f.write_str("이미 적용한 job의 중복 레이아웃 결과입니다"),
        }
    }
}

/// 결과 제출 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardOutcome {
    /// 적용했습니다(새 layout revision).
    Applied(LayoutRevision),
    /// 버렸습니다.
    Discarded(StaleReason),
}

/// guard 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardError {
    /// 기대 revision을 이전 값으로 되돌리려 했습니다.
    RevisionRegressed {
        /// 현재 기대 revision.
        current: DocumentRevision,
        /// 시도한 revision.
        attempted: DocumentRevision,
    },
    /// layout revision 카운터가 범위를 넘었습니다.
    LayoutRevisionOverflow,
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::RevisionRegressed { current, attempted } => write!(
                f,
                "기대 문서 revision을 {}에서 {}로 되돌릴 수 없습니다",
                current.0, attempted.0
            ),
            GuardError::LayoutRevisionOverflow => {
                f.write_str("layout revision이 범위를 넘었습니다")
            }
        }
    }
}

impl std::error::Error for GuardError {}

/// 적용·폐기 집계.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuardStats {
    /// 적용한 결과 수.
    pub applied: u64,
    /// 버린 결과 수.
    pub discarded: u64,
}

/// stale 결과를 버리는 worker result guard (X.7.5).
#[derive(Debug, Clone)]
pub struct LayoutResultGuard {
    expected: LayoutJob,
    expected_applied: bool,
    applied: Option<(LayoutJob, LayoutResult)>,
    layout_revision: LayoutRevision,
    stats: GuardStats,
}

impl LayoutResultGuard {
    /// 처음 기대 job으로 guard를 만듭니다.
    pub fn new(expected: LayoutJob) -> LayoutResultGuard {
        LayoutResultGuard {
            expected,
            expected_applied: false,
            applied: None,
            layout_revision: LayoutRevision::INITIAL,
            stats: GuardStats::default(),
        }
    }

    /// 문서가 바뀌어 기대 job을 갱신합니다. revision 역행은 오류입니다(같은 revision은 허용).
    pub fn expect(&mut self, job: LayoutJob) -> Result<(), GuardError> {
        if job.revision < self.expected.revision {
            return Err(GuardError::RevisionRegressed {
                current: self.expected.revision,
                attempted: job.revision,
            });
        }
        if job != self.expected {
            self.expected = job;
            self.expected_applied = false;
        }
        Ok(())
    }

    /// 현재 기대 job.
    pub const fn expected(&self) -> LayoutJob {
        self.expected
    }

    /// worker 결과를 제출합니다. 기대 job과 정확히 같을 때만 적용합니다.
    pub fn submit(&mut self, result: LayoutWorkerResult) -> Result<GuardOutcome, GuardError> {
        let outcome = if result.job.revision != self.expected.revision {
            Some(StaleReason::RevisionMismatch {
                expected: self.expected.revision,
                actual: result.job.revision,
            })
        } else if result.job.input_hash != self.expected.input_hash {
            Some(StaleReason::InputHashMismatch {
                expected: self.expected.input_hash,
                actual: result.job.input_hash,
            })
        } else if self.expected_applied {
            Some(StaleReason::Duplicate)
        } else {
            None
        };
        if let Some(reason) = outcome {
            self.stats.discarded = self.stats.discarded.saturating_add(1);
            return Ok(GuardOutcome::Discarded(reason));
        }
        let next = self
            .layout_revision
            .checked_next()
            .ok_or(GuardError::LayoutRevisionOverflow)?;
        self.layout_revision = next;
        self.expected_applied = true;
        self.applied = Some((result.job, result.result));
        self.stats.applied = self.stats.applied.saturating_add(1);
        Ok(GuardOutcome::Applied(next))
    }

    /// 마지막으로 적용한 (job, 결과).
    pub fn applied(&self) -> Option<(&LayoutJob, &LayoutResult)> {
        self.applied.as_ref().map(|(job, result)| (job, result))
    }

    /// 마지막으로 적용한 결과의 layout revision.
    pub const fn layout_revision(&self) -> LayoutRevision {
        self.layout_revision
    }

    /// 적용·폐기 집계.
    pub const fn stats(&self) -> GuardStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;
    use crate::input::LayoutNode;
    use crate::test_support::{fixed, id, key};

    /// 너비 `width`인 자식 하나를 가진 입력.
    fn input(width: f32) -> LayoutInput {
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_style(fixed(100.0, 50.0))
                    .with_child(id(2), key(1)),
                LayoutNode::new(id(2)).with_style(fixed(width, 10.0)),
            ],
        )
    }

    fn work(revision: u64, input: &LayoutInput) -> LayoutWorkerResult {
        LayoutWorkerResult {
            job: LayoutJob::for_input(DocumentRevision(revision), input),
            result: compute_layout(input).unwrap(),
        }
    }

    #[test]
    fn stale_revision_and_hash_results_are_discarded() {
        let first = input(10.0);
        let second = input(20.0);
        let mut guard = LayoutResultGuard::new(LayoutJob::for_input(DocumentRevision(1), &first));
        let late = work(1, &first);
        // 결과가 오기 전에 문서가 revision 2로 바뀜.
        guard
            .expect(LayoutJob::for_input(DocumentRevision(2), &second))
            .unwrap();
        assert_eq!(
            guard.submit(late).unwrap(),
            GuardOutcome::Discarded(StaleReason::RevisionMismatch {
                expected: DocumentRevision(2),
                actual: DocumentRevision(1)
            })
        );
        assert!(guard.applied().is_none(), "stale 결과 적용 0");
        // 같은 revision 2지만 다른 입력으로 계산된 결과 → hash 불일치.
        let wrong_input = work(2, &first);
        assert!(matches!(
            guard.submit(wrong_input).unwrap(),
            GuardOutcome::Discarded(StaleReason::InputHashMismatch { .. })
        ));
        // 미래 revision 결과도 버림.
        assert!(matches!(
            guard.submit(work(3, &second)).unwrap(),
            GuardOutcome::Discarded(StaleReason::RevisionMismatch { .. })
        ));
        // 맞는 결과만 적용.
        let correct = work(2, &second);
        let expected_result = correct.result.clone();
        assert_eq!(
            guard.submit(correct.clone()).unwrap(),
            GuardOutcome::Applied(LayoutRevision(1))
        );
        assert_eq!(guard.applied().unwrap().1, &expected_result);
        // 중복 결과는 버리고 적용 상태를 바꾸지 않음.
        assert_eq!(
            guard.submit(correct).unwrap(),
            GuardOutcome::Discarded(StaleReason::Duplicate)
        );
        assert_eq!(guard.layout_revision(), LayoutRevision(1));
        assert_eq!(
            guard.stats(),
            GuardStats {
                applied: 1,
                discarded: 4
            }
        );
    }

    #[test]
    fn interleaved_workers_never_apply_stale_results() {
        // 결정적 PRNG로 revision 갱신과 늦은 결과 도착을 섞습니다.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let inputs: Vec<LayoutInput> = (0..8).map(|i| input(5.0 + i as f32)).collect();
        let mut revision = 1u64;
        let mut current = 0usize;
        let mut guard = LayoutResultGuard::new(LayoutJob::for_input(
            DocumentRevision(revision),
            &inputs[current],
        ));
        let mut in_flight: Vec<LayoutWorkerResult> = Vec::new();
        let mut stale_applied = 0;
        for _ in 0..400 {
            match next() % 3 {
                0 => {
                    // 문서 변경.
                    revision += 1;
                    current = (next() % inputs.len() as u64) as usize;
                    guard
                        .expect(LayoutJob::for_input(
                            DocumentRevision(revision),
                            &inputs[current],
                        ))
                        .unwrap();
                }
                1 => {
                    // 임의 revision·입력으로 작업 시작(일부는 현재 job).
                    let r = revision - (next() % 3).min(revision - 1);
                    let i = if next() % 2 == 0 {
                        current
                    } else {
                        (next() % inputs.len() as u64) as usize
                    };
                    in_flight.push(work(r, &inputs[i]));
                }
                _ => {
                    if in_flight.is_empty() {
                        continue;
                    }
                    let pick = (next() % in_flight.len() as u64) as usize;
                    let arrived = in_flight.swap_remove(pick);
                    let job = arrived.job;
                    if let GuardOutcome::Applied(_) = guard.submit(arrived).unwrap() {
                        if job != guard.expected() {
                            stale_applied += 1;
                        }
                        // 적용된 결과는 항상 현재 입력을 계산한 것.
                        assert_eq!(
                            guard.applied().unwrap().1,
                            &compute_layout(&inputs[current]).unwrap()
                        );
                    }
                }
            }
        }
        assert_eq!(stale_applied, 0);
        let stats = guard.stats();
        assert!(stats.applied > 0 && stats.discarded > 0, "{stats:?}");
    }

    #[test]
    fn expected_revision_cannot_regress_and_hash_is_canonical() {
        let a = input(10.0);
        let mut guard = LayoutResultGuard::new(LayoutJob::for_input(DocumentRevision(5), &a));
        assert_eq!(
            guard.expect(LayoutJob::for_input(DocumentRevision(4), &a)),
            Err(GuardError::RevisionRegressed {
                current: DocumentRevision(5),
                attempted: DocumentRevision(4)
            })
        );
        // 같은 입력 → 같은 hash, 입력 값이 다르면 다른 hash(canonical bytes 기준).
        assert_eq!(a.input_hash(), input(10.0).input_hash());
        assert_ne!(a.input_hash(), input(10.5).input_hash());
        assert!(GuardError::LayoutRevisionOverflow
            .to_string()
            .contains("범위"));
        assert!(StaleReason::Duplicate.to_string().contains("중복"));
    }
}
