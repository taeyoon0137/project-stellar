//! GC scheduler interface (V.7.4).
//!
//! GC(mark+sweep)는 프로젝트 open 직후가 아니라 **idle 상태이고 저장이 진행
//! 중이지 않을 때만** 시작할 수 있습니다:
//!
//! - 저장 파이프라인은 [`GcScheduler::begin_save`]로 RAII guard를 잡습니다.
//!   guard가 살아 있는 동안 [`GcScheduler::try_begin_gc`]는 항상 `None`입니다.
//! - 사용자 활동은 [`GcScheduler::note_activity`]로 기록하고, 마지막 활동 이후
//!   idle threshold가 지나야 GC를 시작할 수 있습니다.
//! - GC 자체도 한 번에 하나만 실행됩니다(`GcGuard`).
//!
//! 시각은 인자로 주입해(단조 `Instant`) 테스트를 결정론적으로 만듭니다.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// scheduler 내부 상태.
#[derive(Debug)]
struct State {
    active_saves: usize,
    gc_running: bool,
    last_activity: Instant,
}

/// GC 시작 조건 판정자.
#[derive(Debug)]
pub struct GcScheduler {
    idle_threshold: Duration,
    state: Mutex<State>,
}

/// 저장 진행 중 표시 RAII guard. drop되면 저장 종료로 간주합니다.
#[derive(Debug)]
pub struct SaveGuard<'a> {
    scheduler: &'a GcScheduler,
}

impl Drop for SaveGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.scheduler.state.lock().expect("poisoned");
        state.active_saves -= 1;
    }
}

/// GC 실행 중 표시 RAII guard. drop되면 GC 종료로 간주합니다.
#[derive(Debug)]
pub struct GcGuard<'a> {
    scheduler: &'a GcScheduler,
}

impl Drop for GcGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.scheduler.state.lock().expect("poisoned");
        state.gc_running = false;
    }
}

impl GcScheduler {
    /// scheduler를 만듭니다. `now`는 시작 시각(직후는 idle이 아님 — open 직후
    /// GC 시작을 막습니다).
    pub fn new(idle_threshold: Duration, now: Instant) -> GcScheduler {
        GcScheduler {
            idle_threshold,
            state: Mutex::new(State {
                active_saves: 0,
                gc_running: false,
                last_activity: now,
            }),
        }
    }

    /// 사용자 활동을 기록합니다(idle 타이머 리셋).
    pub fn note_activity(&self, now: Instant) {
        let mut state = self.state.lock().expect("poisoned");
        if now > state.last_activity {
            state.last_activity = now;
        }
    }

    /// 저장 시작을 표시합니다. 반환된 guard가 살아 있는 동안 GC는 시작될 수
    /// 없습니다. 저장 자체도 활동으로 기록합니다.
    pub fn begin_save(&self, now: Instant) -> SaveGuard<'_> {
        let mut state = self.state.lock().expect("poisoned");
        state.active_saves += 1;
        if now > state.last_activity {
            state.last_activity = now;
        }
        SaveGuard { scheduler: self }
    }

    /// GC 시작을 시도합니다. idle threshold 경과 + 저장 없음 + GC 미실행일 때만
    /// guard를 반환합니다.
    pub fn try_begin_gc(&self, now: Instant) -> Option<GcGuard<'_>> {
        let mut state = self.state.lock().expect("poisoned");
        if state.active_saves > 0 || state.gc_running {
            return None;
        }
        if now.duration_since(state.last_activity) < self.idle_threshold {
            return None;
        }
        state.gc_running = true;
        Some(GcGuard { scheduler: self })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const IDLE: Duration = Duration::from_secs(30);

    #[test]
    fn gc_does_not_start_right_after_open() {
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(IDLE, t0);
        // open 직후: idle threshold 미경과.
        assert!(scheduler.try_begin_gc(t0).is_none());
        assert!(scheduler.try_begin_gc(t0 + IDLE / 2).is_none());
        // idle threshold 경과 후에는 시작 가능.
        assert!(scheduler.try_begin_gc(t0 + IDLE).is_some());
    }

    #[test]
    fn activity_resets_idle_timer() {
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(IDLE, t0);
        scheduler.note_activity(t0 + Duration::from_secs(20));
        // 마지막 활동 기준으로 다시 idle을 채워야 합니다.
        assert!(scheduler.try_begin_gc(t0 + IDLE).is_none());
        assert!(scheduler
            .try_begin_gc(t0 + Duration::from_secs(20) + IDLE)
            .is_some());
    }

    #[test]
    fn gc_never_starts_while_save_in_progress() {
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(IDLE, t0);
        let idle_time = t0 + IDLE * 2;
        {
            let _save = scheduler.begin_save(t0);
            assert!(
                scheduler.try_begin_gc(idle_time).is_none(),
                "저장 중 GC 금지"
            );
        }
        // 저장 종료 후에는 시작 가능(활동 시각은 저장 시작 t0 기준).
        assert!(scheduler.try_begin_gc(idle_time).is_some());
    }

    #[test]
    fn only_one_gc_at_a_time() {
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(IDLE, t0);
        let idle_time = t0 + IDLE;
        let first = scheduler.try_begin_gc(idle_time).unwrap();
        assert!(scheduler.try_begin_gc(idle_time).is_none());
        drop(first);
        assert!(scheduler.try_begin_gc(idle_time).is_some());
    }

    #[test]
    fn concurrent_saves_block_gc() {
        // 검증 기준(V.7.4): save 중 GC 시작하지 않는 동시성 테스트.
        let t0 = Instant::now();
        let scheduler = GcScheduler::new(Duration::ZERO, t0);
        let saves_active = AtomicUsize::new(0);
        let violation = AtomicBool::new(false);

        std::thread::scope(|scope| {
            // 저장 스레드 4개: 반복해서 저장 guard를 잡았다 놓습니다.
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        let _guard = scheduler.begin_save(Instant::now());
                        saves_active.fetch_add(1, Ordering::SeqCst);
                        std::hint::spin_loop();
                        saves_active.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
            // GC 스레드: 계속 시작을 시도하고, 성공 순간 저장이 진행 중이면 위반.
            scope.spawn(|| {
                for _ in 0..2_000 {
                    if let Some(_gc) = scheduler.try_begin_gc(Instant::now()) {
                        if saves_active.load(Ordering::SeqCst) > 0 {
                            violation.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
        });

        assert!(
            !violation.load(Ordering::SeqCst),
            "저장 진행 중에 GC가 시작되었습니다"
        );
    }
}
