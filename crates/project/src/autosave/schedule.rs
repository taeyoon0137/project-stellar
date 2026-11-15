//! 자동 저장 시점 (XIII.1.3·XIII.1.5, UX.2.11).
//!
//! [`AutosaveSchedule`]은 창 하나의 자동 저장 시점을 정하는 순수 상태 기계입니다. 시각은 호출자가 정한 단조
//! 기준점에서 지난 시간([`Duration`])으로 받으므로, 테스트는 sleep 없이 시각을 직접 넣습니다. 실제 캡처·쓰기와
//! 시계는 runtime이 맡습니다. 쓰기는 worker에서 하므로(XIII.1.4) 저장 하나가 진행 중인 동안에도 편집을 관찰합니다.
//!
//! | 사건 | 결과 |
//! |---|---|
//! | 완료 편집(관찰한 문서 revision이 바뀜) | 기한을 `이 시각 + debounce`로 미룹니다. 저장되지 않은 첫 편집 시각 + `max_delay`가 상한입니다 |
//! | 같은 revision을 다시 관찰(선택·viewport 변경 등) | 기한을 바꾸지 않습니다 |
//! | 제스처 진행(history preview: drag·scrub) | 기한 없이 기다립니다. 문서 메모리에 미완료 변경이 섞여 있으므로 쓰지 않습니다 |
//! | 캡처([`AutosaveSchedule::begin_save`]) | 그 revision까지는 진행 중인 저장이 맡습니다. 그 뒤 편집은 새 대기로 셉니다 |
//! | 저장 성공 | 진행 중 표시와 실패 표시를 지웁니다. 저장 중에 들어온 편집은 대기로 남습니다 |
//! | 저장 실패 | 실패로 표시하고 실패 시각 + `max_delay`에 다시 시도합니다. 그 사이 완료 편집이 오면 그 편집의 debounce 뒤입니다 |
//! | stale(다른 창·프로세스가 먼저 저장) | clean이 될 때까지 멈춥니다(기술 아키텍처 22.2 "자동 저장을 일시 중단") |
//! | clean 관찰(Save Now 성공, 다시 불러오기) | 대기·실패·stale을 모두 비웁니다. 이미 접수한 자동 저장은 쓰지 않습니다(Save Now와 합쳐짐) |
//!
//! flush(앱 비활성화)는 debounce를 기다리지 않지만 제스처 중이거나, 저장이 진행 중이거나, stale이면 쓰지 않습니다
//! ([`AutosaveSchedule::should_flush`]). 창 닫기 flush는 Save Now가 맡습니다.

use std::time::Duration;

use stellar_core_types::revision::DocumentRevision;

use super::AutosavePolicy;

/// 창 하나의 자동 저장 상태(셸 표시와 상태 관찰 주기 판단용, ADR-0072).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutosaveStatus {
    /// 저장할 완료 편집이 없습니다(clean).
    #[default]
    Idle,
    /// 완료 편집이 저장 기한을 기다립니다(제스처가 끝나기를 기다리는 경우 포함).
    Pending,
    /// worker가 직렬화·쓰기를 하는 중입니다.
    Saving,
    /// 마지막 자동 저장이 실패했습니다. 다시 시도를 기다리거나 다시 쓰는 중이며, 저장에 성공하거나 clean이 되면
    /// 풀립니다.
    Failed,
    /// 이 창이 연 뒤 다른 창·프로세스가 먼저 저장했습니다. clean이 될 때까지 쓰지 않습니다.
    Stale,
}

/// 지금 무엇을 할지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveDecision {
    /// 저장할 완료 편집이 없습니다.
    Idle,
    /// 이 시각(기준점에서 지난 시간)까지 기다립니다.
    Wait {
        /// 기한.
        until: Duration,
    },
    /// 지금 저장합니다.
    Save,
    /// 제스처가 진행 중이라 끝날 때까지 기다립니다(기한 없음).
    Gesture,
    /// 저장이 진행 중입니다. 결과가 오면 다시 판단합니다(기한 없음).
    Saving,
    /// stale로 멈춰 있습니다.
    Stale,
}

/// 진행 중인 저장이 맡지 않은 완료 편집.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    /// 마지막으로 관찰한 작업 문서 revision.
    revision: DocumentRevision,
    /// 최대 지연 기준 시각(저장되지 않은 첫 편집, 실패 뒤에는 실패 시각).
    first: Duration,
    /// debounce 기준 시각(마지막 완료 편집). 실패 뒤 아직 편집이 없으면 `None`(최대 지연에만 다시 시도).
    last: Option<Duration>,
}

/// 창 하나의 자동 저장 시점.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutosaveSchedule {
    policy: AutosavePolicy,
    pending: Option<Pending>,
    /// 진행 중인 저장이 캡처한 revision.
    saving: Option<DocumentRevision>,
    failed: bool,
    stale: bool,
}

impl AutosaveSchedule {
    /// 대기 없이 시작합니다.
    pub fn new(policy: AutosavePolicy) -> AutosaveSchedule {
        AutosaveSchedule {
            policy,
            pending: None,
            saving: None,
            failed: false,
            stale: false,
        }
    }

    /// 정책.
    pub fn policy(&self) -> &AutosavePolicy {
        &self.policy
    }

    /// 셸에 보일 상태. stale → 실패 → 저장 중 → 대기 순으로 앞선 것을 돌려줍니다.
    pub fn status(&self) -> AutosaveStatus {
        if self.stale {
            AutosaveStatus::Stale
        } else if self.failed {
            AutosaveStatus::Failed
        } else if self.saving.is_some() {
            AutosaveStatus::Saving
        } else if self.pending.is_some() {
            AutosaveStatus::Pending
        } else {
            AutosaveStatus::Idle
        }
    }

    /// 상태를 관찰합니다. `dirty`는 저장되지 않은 완료 편집이 있는지, `revision`은 작업 문서 revision입니다.
    pub fn observe(&mut self, now: Duration, revision: DocumentRevision, dirty: bool) {
        if !dirty {
            self.pending = None;
            self.failed = false;
            self.stale = false;
            return;
        }
        match &mut self.pending {
            Some(pending) if pending.revision == revision => {}
            Some(pending) => {
                pending.revision = revision;
                pending.last = Some(now);
            }
            // 진행 중인 저장이 캡처한 상태 그대로면 새 대기가 아닙니다.
            None if self.saving == Some(revision) => {}
            None => {
                self.pending = Some(Pending {
                    revision,
                    first: now,
                    last: Some(now),
                });
            }
        }
    }

    /// `now`에 할 일. `gesture`는 history preview가 진행 중인지입니다.
    pub fn decide(&self, now: Duration, gesture: bool) -> AutosaveDecision {
        if self.stale {
            return AutosaveDecision::Stale;
        }
        if self.saving.is_some() {
            return AutosaveDecision::Saving;
        }
        let Some(pending) = self.pending else {
            return AutosaveDecision::Idle;
        };
        if gesture {
            return AutosaveDecision::Gesture;
        }
        let mut until = pending.first.saturating_add(self.policy.max_delay);
        if let Some(last) = pending.last {
            until = until.min(last.saturating_add(self.policy.debounce));
        }
        if now >= until {
            AutosaveDecision::Save
        } else {
            AutosaveDecision::Wait { until }
        }
    }

    /// debounce를 기다리지 않는 flush(앱 비활성화)에서 지금 쓸지. 실패한 창은 다시 시도합니다.
    pub fn should_flush(&self, gesture: bool) -> bool {
        self.pending.is_some() && !gesture && !self.stale && self.saving.is_none()
    }

    /// `revision`을 캡처해 쓰기를 시작합니다. 그 뒤의 편집은 새 대기로 셉니다.
    pub fn begin_save(&mut self, revision: DocumentRevision) {
        self.saving = Some(revision);
        self.pending = None;
    }

    /// 진행 중인 저장(또는 쓸 것이 없었던 저장)이 성공했습니다.
    pub fn saved(&mut self) {
        self.saving = None;
        self.failed = false;
    }

    /// 진행 중인 저장이 실패했습니다. 저장 중에 들어온 편집이 없으면 `now + max_delay`에 다시 시도합니다.
    pub fn failed(&mut self, now: Duration) {
        self.failed = true;
        self.retry_from(now);
    }

    /// 진행 중인 저장이 stale을 발견했습니다. clean이 될 때까지 쓰지 않습니다.
    pub fn stale(&mut self, now: Duration) {
        self.stale = true;
        self.retry_from(now);
    }

    fn retry_from(&mut self, now: Duration) {
        if let Some(revision) = self.saving.take() {
            if self.pending.is_none() {
                self.pending = Some(Pending {
                    revision,
                    first: now,
                    last: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autosave::FlushPolicy;

    /// 테스트 입력 전용 값(제품 값은 `config/performance-budgets.yaml`).
    fn policy() -> AutosavePolicy {
        AutosavePolicy {
            debounce: ms(100),
            max_delay: ms(1_000),
            app_deactivate_flush: FlushPolicy { timeout: ms(500) },
            window_close_flush: FlushPolicy { timeout: ms(500) },
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn rev(n: u64) -> DocumentRevision {
        DocumentRevision(n)
    }

    #[test]
    fn each_completed_edit_restarts_the_debounce() {
        let mut s = AutosaveSchedule::new(policy());
        assert_eq!(s.decide(ms(0), false), AutosaveDecision::Idle);
        assert_eq!(s.status(), AutosaveStatus::Idle);
        s.observe(ms(0), rev(1), true);
        assert_eq!(s.status(), AutosaveStatus::Pending);
        assert_eq!(
            s.decide(ms(50), false),
            AutosaveDecision::Wait { until: ms(100) }
        );
        s.observe(ms(80), rev(2), true);
        assert_eq!(
            s.decide(ms(100), false),
            AutosaveDecision::Wait { until: ms(180) }
        );
        assert_eq!(s.decide(ms(180), false), AutosaveDecision::Save);
    }

    #[test]
    fn same_revision_does_not_postpone_the_save() {
        // 선택·viewport 변경은 revision을 바꾸지 않으므로 기한을 미루지 않습니다.
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        for t in [20, 40, 60, 80] {
            s.observe(ms(t), rev(1), true);
        }
        assert_eq!(s.decide(ms(100), false), AutosaveDecision::Save);
    }

    #[test]
    fn continuous_edits_are_captured_at_the_max_delay() {
        let mut s = AutosaveSchedule::new(policy());
        for (i, t) in (0..=1_000).step_by(50).enumerate() {
            s.observe(ms(t), rev(i as u64 + 1), true);
            if t < 1_000 {
                assert!(
                    matches!(s.decide(ms(t), false), AutosaveDecision::Wait { .. }),
                    "{t}ms"
                );
            }
        }
        assert_eq!(s.decide(ms(1_000), false), AutosaveDecision::Save);
        s.begin_save(rev(21));
        s.saved();
        // 저장 뒤의 다음 편집은 그 시각부터 다시 셉니다.
        s.observe(ms(1_010), rev(30), true);
        assert_eq!(
            s.decide(ms(1_010), false),
            AutosaveDecision::Wait { until: ms(1_110) }
        );
    }

    #[test]
    fn gesture_in_progress_never_saves() {
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        for t in [100, 1_000, 10_000] {
            assert_eq!(s.decide(ms(t), true), AutosaveDecision::Gesture);
            assert!(!s.should_flush(true));
        }
        // 제스처가 끝나 확정 편집이 기록되면 그 시각부터 debounce, 최대 지연은 첫 편집 기준이라 바로 저장.
        s.observe(ms(10_000), rev(2), true);
        assert_eq!(s.decide(ms(10_000), false), AutosaveDecision::Save);
    }

    #[test]
    fn edits_during_a_save_wait_for_their_own_debounce() {
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        s.begin_save(rev(1));
        assert_eq!(s.status(), AutosaveStatus::Saving);
        // 캡처 직후 게시한 상태(같은 revision, 아직 dirty)는 새 대기가 아닙니다.
        s.observe(ms(1), rev(1), true);
        assert_eq!(s.decide(ms(5_000), false), AutosaveDecision::Saving);
        assert!(!s.should_flush(false), "진행 중 저장과 겹쳐 쓰지 않음");
        // 저장 중 편집은 그 편집부터 debounce를 셉니다.
        s.observe(ms(40), rev(2), true);
        assert_eq!(s.decide(ms(200), false), AutosaveDecision::Saving);
        s.saved();
        assert_eq!(s.status(), AutosaveStatus::Pending);
        assert_eq!(
            s.decide(ms(139), false),
            AutosaveDecision::Wait { until: ms(140) }
        );
        assert_eq!(s.decide(ms(140), false), AutosaveDecision::Save);
    }

    #[test]
    fn save_now_merges_the_pending_autosave() {
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        // Save Now가 저장한 뒤 게시한 상태는 clean입니다.
        s.observe(ms(30), rev(1), false);
        assert_eq!(s.decide(ms(100), false), AutosaveDecision::Idle);
        assert_eq!(s.decide(ms(5_000), false), AutosaveDecision::Idle);
        assert!(!s.should_flush(false));
    }

    #[test]
    fn failure_retries_after_the_max_delay_or_the_next_edit() {
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        s.begin_save(rev(1));
        s.failed(ms(100));
        assert_eq!(s.status(), AutosaveStatus::Failed);
        // 같은 revision 재관찰은 기한을 당기지 않고, 실패 시각 + 최대 지연에 다시 시도합니다.
        s.observe(ms(500), rev(1), true);
        assert_eq!(
            s.decide(ms(500), false),
            AutosaveDecision::Wait { until: ms(1_100) }
        );
        assert_eq!(s.decide(ms(1_100), false), AutosaveDecision::Save);
        // 앱 비활성화 flush는 실패를 바로 다시 시도합니다.
        assert!(s.should_flush(false));
        // 다시 시도 중에도 실패 표시는 남습니다(성공해야 풀림).
        s.begin_save(rev(1));
        assert_eq!(s.status(), AutosaveStatus::Failed);
        s.failed(ms(1_200));
        // 다음 편집은 그 편집의 debounce 뒤입니다.
        s.observe(ms(1_250), rev(2), true);
        assert_eq!(
            s.decide(ms(1_250), false),
            AutosaveDecision::Wait { until: ms(1_350) }
        );
        s.begin_save(rev(2));
        s.saved();
        assert_eq!(s.status(), AutosaveStatus::Idle);
    }

    #[test]
    fn stale_stays_suspended_until_clean() {
        let mut s = AutosaveSchedule::new(policy());
        s.observe(ms(0), rev(1), true);
        s.begin_save(rev(1));
        s.stale(ms(100));
        s.observe(ms(200), rev(2), true);
        assert_eq!(s.decide(ms(5_000), false), AutosaveDecision::Stale);
        assert!(!s.should_flush(false));
        assert_eq!(s.status(), AutosaveStatus::Stale);
        // 다시 불러오기·다른 선택으로 clean이 되면 풀립니다.
        s.observe(ms(6_000), rev(3), false);
        assert_eq!(s.status(), AutosaveStatus::Idle);
        s.observe(ms(6_010), rev(4), true);
        assert_eq!(
            s.decide(ms(6_010), false),
            AutosaveDecision::Wait { until: ms(6_110) }
        );
    }
}
