//! present queue 깊이 ≤ 1 frame scheduler (XI.5.5).
//!
//! render thread가 surface frame을 언제 그리고 무엇을 버릴지 정합니다(아키텍처 §6.3: Present 대기열
//! 깊이 최대 1 frame, §9.3: 오래된 렌더 결과는 세대 번호로 폐기).
//!
//! - **pending은 최신 하나**: document thread가 넘긴 snapshot은 [`FrameScheduler::offer`]로 들어오고,
//!   아직 encode하지 않은 이전 snapshot은 새 snapshot이 오면 그 자리에서 버립니다. 늦게 도착한 더
//!   오래된 snapshot도 버립니다.
//! - **present queue 깊이 ≤ 1**: present를 예약해 제출했지만 아직 화면에 나가지 않은 frame이
//!   [`MAX_PRESENT_QUEUE_DEPTH`]개면 새 frame을 시작하지 않고 기다립니다(`Throttled`). 그 사이 들어온
//!   snapshot은 pending을 계속 최신으로 갈아 끼우므로, GPU가 느려도 대기열이 쌓이지 않고 다음 frame은
//!   항상 그 시점 최신 snapshot입니다.
//! - **제출 전 폐기**: encode 직전과 제출 직전에 [`LatestSceneRevision`](XI.1.6)으로 세대를 확인합니다.
//!   그 사이 더 새로운 세대가 알려졌으면 encode하지 않거나, encode한 frame을
//!   [`RendererBackend::discard`]로 버려 GPU에 보내지 않습니다.
//! - 오류·skip 시 snapshot을 pending으로 되돌려 다음 tick에 다시 그립니다(그 사이 더 새 snapshot이 오면
//!   그것으로 대체).
//!
//! scheduler는 surface frame 전용이며 제출하는 모든 frame에 present를 예약합니다.

use std::collections::VecDeque;

use stellar_scene::freshness::{LatestSceneRevision, PresentCheck};
use stellar_scene::handle::SnapshotHandle;

use crate::backend::{
    BackendError, EncodeOutcome, FrameId, GpuProgress, RendererBackend, SkipReason, SubmissionId,
};

/// present 예약 후 화면에 나가지 않은 frame의 최대 수.
pub const MAX_PRESENT_QUEUE_DEPTH: usize = 1;

/// tick 한 번의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TickOutcome {
    /// 그릴 snapshot이 없습니다.
    Idle,
    /// present queue가 차 있어 기다립니다(pending 유지).
    Throttled {
        /// 현재 present queue 깊이.
        queue_depth: usize,
    },
    /// backend가 frame을 건너뛰었습니다(pending 유지).
    Skipped(SkipReason),
    /// 더 새로운 세대가 알려져 이 snapshot을 그리지 않거나 encode한 frame을 버렸습니다.
    DroppedStale {
        /// 버린 scene revision.
        stale: u64,
        /// 판정 시점 최신 scene revision.
        latest: u64,
        /// encode 뒤(제출 직전) 버렸으면 참.
        after_encode: bool,
    },
    /// 제출하고 present를 예약했습니다.
    Submitted {
        /// frame 번호.
        frame: FrameId,
        /// submission.
        submission: SubmissionId,
        /// 그린 scene revision.
        scene_revision: u64,
    },
}

/// scheduler 통계(benchmark 결과 보고용, XI.11.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SchedulerStats {
    /// `offer` 호출 수.
    pub offered: u64,
    /// 제출한 frame 수.
    pub submitted: u64,
    /// encode 전에 버린 오래된 snapshot 수(새 snapshot에 밀림·늦은 도착·세대 확인).
    pub dropped_before_encode: u64,
    /// encode했지만 제출 직전 오래되어 버린 frame 수.
    pub dropped_after_encode: u64,
    /// backend skip 수.
    pub skipped: u64,
    /// present queue가 차서 기다린 tick 수.
    pub throttled: u64,
    /// 관측한 최대 present queue 깊이.
    pub max_queue_depth: usize,
}

impl SchedulerStats {
    /// 버린 오래된 frame 총수.
    pub const fn dropped_stale(&self) -> u64 {
        self.dropped_before_encode + self.dropped_after_encode
    }
}

/// present queue 깊이를 1 이하로 유지하는 frame scheduler.
#[derive(Debug)]
pub struct FrameScheduler {
    latest: LatestSceneRevision,
    pending: Option<SnapshotHandle>,
    present_queue: VecDeque<SubmissionId>,
    next_frame: u64,
    last_submitted_revision: Option<u64>,
    stats: SchedulerStats,
}

fn revision_of(handle: &SnapshotHandle) -> u64 {
    handle.scene_revision().0
}

impl FrameScheduler {
    /// document thread와 공유하는 최신 세대 번호로 scheduler를 만듭니다.
    pub fn new(latest: LatestSceneRevision) -> FrameScheduler {
        FrameScheduler {
            latest,
            pending: None,
            present_queue: VecDeque::new(),
            next_frame: 1,
            last_submitted_revision: None,
            stats: SchedulerStats::default(),
        }
    }

    /// 통계.
    pub const fn stats(&self) -> SchedulerStats {
        self.stats
    }

    /// 현재 present queue 깊이.
    pub fn present_queue_depth(&self) -> usize {
        self.present_queue.len()
    }

    /// 그릴 차례를 기다리는 snapshot의 scene revision.
    pub fn pending_revision(&self) -> Option<u64> {
        self.pending.as_ref().map(revision_of)
    }

    /// 마지막으로 제출한 scene revision.
    pub const fn last_submitted_revision(&self) -> Option<u64> {
        self.last_submitted_revision
    }

    /// 새 snapshot을 받습니다. 버린 snapshot(밀려난 pending 또는 늦게 온 오래된 snapshot)을
    /// 돌려줍니다.
    pub fn offer(&mut self, snapshot: SnapshotHandle) -> Option<SnapshotHandle> {
        self.stats.offered += 1;
        let revision = revision_of(&snapshot);
        let behind_pending = self
            .pending
            .as_ref()
            .is_some_and(|pending| revision <= revision_of(pending));
        let behind_submitted = self
            .last_submitted_revision
            .is_some_and(|submitted| revision <= submitted);
        if behind_pending || behind_submitted {
            self.stats.dropped_before_encode += 1;
            return Some(snapshot);
        }
        let replaced = self.pending.replace(snapshot);
        if replaced.is_some() {
            self.stats.dropped_before_encode += 1;
        }
        replaced
    }

    /// GPU 진행을 반영해 화면에 나간 frame을 present queue에서 뺍니다.
    pub fn observe(&mut self, progress: GpuProgress) {
        if let Some(presented) = progress.presented {
            while self
                .present_queue
                .front()
                .is_some_and(|submission| *submission <= presented)
            {
                self.present_queue.pop_front();
            }
        }
    }

    /// device 재생성 뒤 이전 device의 present 대기를 비웁니다.
    pub fn reset_present_queue(&mut self) {
        self.present_queue.clear();
    }

    fn restore(&mut self, snapshot: SnapshotHandle) {
        if self.pending.is_none() {
            self.pending = Some(snapshot);
        }
    }

    fn stale(&self, snapshot: &SnapshotHandle) -> Option<(u64, u64)> {
        match self.latest.check_handle(snapshot) {
            PresentCheck::Present { .. } => None,
            PresentCheck::Discard { stale, latest } => Some((stale.0, latest.0)),
        }
    }

    /// frame 하나를 진행합니다.
    ///
    /// `encode`는 snapshot과 frame 번호로 backend에 graph를 encode합니다(compile·command 생성 포함).
    /// 오류가 나면 snapshot을 pending으로 되돌리고 오류를 돌려줍니다.
    pub fn tick<B, F>(&mut self, backend: &mut B, encode: F) -> Result<TickOutcome, BackendError>
    where
        B: RendererBackend + ?Sized,
        F: FnOnce(&mut B, &SnapshotHandle, FrameId) -> Result<EncodeOutcome, BackendError>,
    {
        let progress = backend.poll()?;
        self.observe(progress);
        let Some(snapshot) = self.pending.take() else {
            return Ok(TickOutcome::Idle);
        };
        let queue_depth = self.present_queue.len();
        if queue_depth >= MAX_PRESENT_QUEUE_DEPTH {
            self.pending = Some(snapshot);
            self.stats.throttled += 1;
            return Ok(TickOutcome::Throttled { queue_depth });
        }
        if let Some((stale, latest)) = self.stale(&snapshot) {
            self.stats.dropped_before_encode += 1;
            return Ok(TickOutcome::DroppedStale {
                stale,
                latest,
                after_encode: false,
            });
        }
        let frame = FrameId(self.next_frame);
        self.next_frame += 1;
        let encoded = match encode(backend, &snapshot, frame) {
            Ok(EncodeOutcome::Encoded(encoded)) => encoded,
            Ok(EncodeOutcome::Skipped(reason)) => {
                self.stats.skipped += 1;
                self.restore(snapshot);
                return Ok(TickOutcome::Skipped(reason));
            }
            Err(error) => {
                self.restore(snapshot);
                return Err(error);
            }
        };
        // 제출 직전 세대 확인(XI.1.6): encode 중 더 새로운 snapshot이 알려졌으면 GPU에 보내지 않습니다.
        if let Some((stale, latest)) = self.stale(&snapshot) {
            backend.discard(encoded)?;
            self.stats.dropped_after_encode += 1;
            return Ok(TickOutcome::DroppedStale {
                stale,
                latest,
                after_encode: true,
            });
        }
        if let Err(error) = backend.schedule_present(&encoded) {
            // present를 걸 수 없으면 frame을 버리고(실패해도 열린 frame은 닫힘) 다시 시도합니다.
            let _ = backend.discard(encoded);
            self.restore(snapshot);
            return Err(error);
        }
        let submission = match backend.submit(encoded) {
            Ok(submission) => submission,
            Err(error) => {
                self.restore(snapshot);
                return Err(error);
            }
        };
        let scene_revision = revision_of(&snapshot);
        self.present_queue.push_back(submission);
        self.last_submitted_revision = Some(scene_revision);
        self.stats.submitted += 1;
        self.stats.max_queue_depth = self.stats.max_queue_depth.max(self.present_queue.len());
        Ok(TickOutcome::Submitted {
            frame,
            submission,
            scene_revision,
        })
    }
}

#[cfg(test)]
mod tests;
