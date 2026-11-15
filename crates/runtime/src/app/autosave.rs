//! 지연 자동 저장 트리거 (UX.2.11, XIII.1.3·XIII.1.5, ADR-0072).
//!
//! 창마다 [`AutosaveSchedule`]을 두고, document thread가 상태 snapshot을 게시할 때마다(`build_state`, 완료 요청과
//! worker 결과 반영 뒤) 작업 문서 revision·dirty·제스처(history preview) 여부를 관찰합니다. 모든 창 가운데 가장
//! 이른 기한을 시계 thread에 알리면, 시계 thread는 그 시각에 자동 저장 job 하나를 background lane에 같은 병합
//! key로 보냅니다. 그래서 입력·편집 command보다 늦게 처리되고, 밀린 자동 저장 job은 하나로 합쳐집니다.
//!
//! - **저장**: job은 document thread에서 기한이 지난 창만 [`ProjectWindowSession::save_now`]로 저장합니다. Save Now와
//!   같은 경로라 같은 잠금·base generation 검사(stale)·원자적 교체를 씁니다.
//! - **Save Now 합치기**: Save Now(⌘S·창 닫기 flush)가 먼저 저장하면 게시된 상태가 clean이라 대기가 비고, 이미
//!   접수한 자동 저장 job도 쓰지 않습니다. 둘 다 document thread에서 차례로 실행되므로 같은 revision을 두 번 쓰지
//!   않습니다.
//! - **제스처**: history preview가 진행 중인 창은 쓰지 않습니다(연속 drag·scrub 중 디스크 쓰기 0). 지금 셸의
//!   drag는 놓을 때 transaction 하나만 보내므로 drag 중에는 완료 편집 자체가 없습니다.
//! - **flush**: 앱 비활성화는 [`AppRuntime::flush_autosave`]로 debounce 없이 저장합니다. 창 닫기는 셸이 Save Now로
//!   flush하고 실패·stale이면 셸의 닫기 선택(D6)으로 갑니다.
//! - **실패·stale**: 자동 저장은 완료 event를 만들지 않습니다(요청이 아님). 실패하면 그 창의 자동 저장을 멈추고
//!   다음 완료 편집 뒤 다시 시도하며, stale이면 clean이 될 때까지 멈춥니다. 편집은 메모리에 남고 창을 닫을 때의
//!   Save Now가 같은 실패를 셸에 돌려줍니다.
//! - **이미지 가져오기**: 진행 중이면 끝날 때까지 미룹니다. 가져오기는 시작할 때의 loaded generation을 확인하므로
//!   그 사이 저장하면 배치가 실패합니다.
//!
//! 저장은 아직 document thread에서 직렬화와 쓰기를 모두 합니다(worker 쓰기는 XIII.1.4). Release 측정값과 한계는
//! ADR-0072에 있습니다.
//!
//! 자동 저장은 [`AppRuntime::enable_autosave`]로 켭니다(네이티브 앱 runtime). 켜지 않은 runtime(테스트 등)은 Save
//! Now로만 저장합니다.

use super::*;
use std::sync::Condvar;
use std::thread::JoinHandle;
use std::time::Instant;

use stellar_project::autosave::schedule::{AutosaveDecision, AutosaveSchedule, AutosaveSuspension};
use stellar_project::autosave::{AutosaveConfigError, AutosavePolicy};

use crate::command_queue::{CoalesceKey, SubmitOptions};

/// 앱 runtime이 쓰는 성능 예산 파일(빌드 시 포함). 자동 저장 값의 원본입니다.
const BUDGETS: &str = include_str!("../../../../config/performance-budgets.yaml");

/// background lane에서 자동 저장 job을 합치는 key.
const COALESCE_KEY: CoalesceKey = CoalesceKey::Custom(0x4155_544F_5341_5645);

/// 이미지 가져오기가 끝나기를 기다리는 간격.
const IMPORT_DEFER: Duration = Duration::from_millis(100);

/// 시계 thread 이름.
const CLOCK_THREAD_NAME: &str = "stellar-autosave";

/// 시계 thread가 기다리는 기한(기준점에서 지난 시간).
#[derive(Debug, Default)]
struct WakeState {
    deadline: Option<Duration>,
    stopped: bool,
}

/// document thread와 시계 thread가 공유하는 기한.
#[derive(Debug)]
pub(crate) struct AutosaveWake {
    origin: Instant,
    state: Mutex<WakeState>,
    changed: Condvar,
}

impl AutosaveWake {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn lock(&self) -> MutexGuard<'_, WakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 다음 기한을 바꿉니다(`None`이면 기다릴 것 없음).
    fn arm(&self, deadline: Option<Duration>) {
        let mut state = self.lock();
        if state.deadline != deadline {
            state.deadline = deadline;
            self.changed.notify_one();
        }
    }

    fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_one();
    }
}

/// document thread 쪽 자동 저장 설정([`AppSessions`] 안).
#[derive(Debug)]
pub(crate) struct AutosaveSetup {
    policy: AutosavePolicy,
    wake: Arc<AutosaveWake>,
}

/// 창 하나의 자동 저장 상태([`WindowEntry`] 안). 자동 저장을 켠 runtime에서 처음 관찰할 때 만듭니다.
#[derive(Debug, Default)]
pub(crate) struct WindowAutosave(Option<AutosaveSchedule>);

impl WindowAutosave {
    /// 자동 저장이 멈춘 이유(테스트·진단).
    #[cfg(test)]
    pub(crate) fn suspension(&self) -> Option<AutosaveSuspension> {
        self.0.as_ref().and_then(AutosaveSchedule::suspension)
    }
}

/// 시계 thread 소유자([`AppRuntime`] 안).
#[derive(Debug)]
pub(crate) struct AutosaveClock {
    wake: Arc<AutosaveWake>,
    thread: Option<JoinHandle<()>>,
}

impl AutosaveClock {
    /// 시계 thread를 멈추고 끝날 때까지 기다립니다(대기 중인 condvar만 깨우므로 바로 끝남).
    pub(crate) fn stop(&mut self) {
        self.wake.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn gesture_active(window: &ProjectWindowSession) -> bool {
    window.history().history().preview_active()
}

/// 모든 창의 상태를 관찰하고 가장 이른 기한을 시계에 알립니다. `build_state`가 게시 직전에 부릅니다.
pub(super) fn observe(ctx: &mut DocumentContext) {
    let sessions = ctx.app_sessions_mut();
    let Some(setup) = sessions.autosave.as_ref() else {
        return;
    };
    let now = setup.wake.now();
    let mut next: Option<Duration> = None;
    for entry in sessions.windows.values_mut() {
        let schedule = entry
            .autosave
            .0
            .get_or_insert_with(|| AutosaveSchedule::new(setup.policy));
        let window = &entry.session;
        schedule.observe(now, window.revision(), window.has_unsaved_changes());
        let due = match schedule.decide(now, gesture_active(window)) {
            AutosaveDecision::Wait { until } => until,
            AutosaveDecision::Save => now,
            AutosaveDecision::Idle | AutosaveDecision::Gesture | AutosaveDecision::Suspended(_) => {
                continue
            }
        };
        next = Some(next.map_or(due, |current| current.min(due)));
    }
    setup.wake.arm(next);
}

/// 기한이 지난 창(`flush`이면 debounce와 무관하게 대기 중인 모든 창)을 저장합니다. 상태가 바뀌었으면 게시하고,
/// 아니면 다음 기한만 다시 알립니다.
fn run(ctx: &mut DocumentContext, shared: &AppShared, flush: bool) {
    let sessions = ctx.app_sessions_mut();
    let Some(setup) = sessions.autosave.as_ref() else {
        return;
    };
    let now = setup.wake.now();
    // 이미지 가져오기는 시작할 때의 loaded generation에 묶여 있으므로(image_place) 끝날 때까지 쓰지 않습니다.
    if shared.importing_image.load(Ordering::Acquire) {
        setup.wake.arm(Some(now + IMPORT_DEFER));
        return;
    }
    let mut changed = false;
    for entry in sessions.windows.values_mut() {
        let Some(schedule) = entry.autosave.0.as_mut() else {
            continue;
        };
        let window = &mut entry.session;
        let gesture = gesture_active(window);
        let due = if flush {
            schedule.should_flush(gesture)
        } else {
            schedule.decide(now, gesture) == AutosaveDecision::Save
        };
        if !due {
            continue;
        }
        if !window.has_unsaved_changes() {
            schedule.saved();
            continue;
        }
        changed = true;
        match window.save_now() {
            Ok(_) => schedule.saved(),
            Err(WindowSessionSaveError::Stale(_)) => schedule.failed(AutosaveSuspension::Stale),
            Err(error) => {
                eprintln!("Stellar autosave: {error}");
                schedule.failed(AutosaveSuspension::Failed);
            }
        }
    }
    if changed {
        shared.publish_state(build_state(ctx));
    } else {
        observe(ctx);
    }
}

/// 시계 thread: 기한까지 기다렸다가 자동 저장 job을 보냅니다. job이 실행되면 다음 기한을 다시 알립니다.
fn clock_loop(wake: Arc<AutosaveWake>, sender: DocumentSender, shared: Arc<AppShared>) {
    let retry = Duration::from_millis(100);
    loop {
        {
            let mut state = wake.lock();
            loop {
                if state.stopped {
                    return;
                }
                match state.deadline {
                    None => {
                        state = wake
                            .changed
                            .wait(state)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    Some(deadline) => {
                        let now = wake.now();
                        if now >= deadline {
                            break;
                        }
                        state = wake
                            .changed
                            .wait_timeout(state, deadline - now)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                }
            }
            state.deadline = None;
        }
        let job_shared = Arc::clone(&shared);
        let options = SubmitOptions::background().with_coalesce(COALESCE_KEY);
        match sender.try_submit(options, move |ctx: &mut DocumentContext| {
            run(ctx, &job_shared, false)
        }) {
            Ok(_) => {}
            Err(SubmitError::Stopped) => return,
            // background lane은 차면 오래된 job을 밀어내므로 거의 없지만, 거부되면 잠시 뒤 다시 보냅니다.
            Err(SubmitError::QueueFull { .. }) => wake.arm(Some(wake.now() + retry)),
        }
    }
}

impl AppRuntime {
    /// `config/performance-budgets.yaml`의 `autosave` 절로 자동 저장을 켭니다(모듈 문서). 이미 켰으면 그대로입니다.
    pub fn enable_autosave(&mut self) -> Result<(), AutosaveConfigError> {
        let policy = AutosavePolicy::from_budgets_yaml(BUDGETS)?;
        self.enable_autosave_with(policy);
        Ok(())
    }

    /// 주어진 정책으로 자동 저장을 켭니다. 이미 켰으면 그대로입니다.
    pub fn enable_autosave_with(&mut self, policy: AutosavePolicy) {
        if self.autosave.is_some() {
            return;
        }
        let wake = Arc::new(AutosaveWake {
            origin: Instant::now(),
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        // 설정 job은 command lane FIFO라 이 뒤에 접수한 요청보다 먼저 실행됩니다.
        let setup = AutosaveSetup {
            policy,
            wake: Arc::clone(&wake),
        };
        if self
            .sender
            .submit(move |ctx: &mut DocumentContext| {
                ctx.app_sessions_mut().autosave = Some(setup);
                observe(ctx);
            })
            .is_err()
        {
            return;
        }
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let clock_wake = Arc::clone(&wake);
        let thread = std::thread::Builder::new()
            .name(CLOCK_THREAD_NAME.into())
            .spawn(move || clock_loop(clock_wake, sender, shared));
        match thread {
            Ok(thread) => {
                self.autosave = Some(AutosaveClock {
                    wake,
                    thread: Some(thread),
                })
            }
            Err(error) => eprintln!("Stellar autosave clock: {error}"),
        }
    }

    /// 자동 저장 시계를 멈춥니다. document thread보다 먼저 멈춰 종료 중에 새 저장을 접수하지 않습니다.
    pub(super) fn stop_autosave(&mut self) {
        if let Some(clock) = self.autosave.as_mut() {
            clock.stop();
        }
    }

    /// 앱 비활성화 flush: 저장되지 않은 완료 편집을 debounce 없이 저장하도록 접수합니다. 기다리지 않고 완료 event도
    /// 없습니다. 제스처 중이거나 stale인 창은 쓰지 않습니다. 자동 저장을 켜지 않았으면 아무것도 하지 않습니다.
    pub fn flush_autosave(&self) -> Result<(), RequestError> {
        if self.autosave.is_none() {
            return Ok(());
        }
        let shared = Arc::clone(&self.shared);
        self.sender
            .try_submit(
                SubmitOptions::command(),
                move |ctx: &mut DocumentContext| run(ctx, &shared, true),
            )
            .map(|_| ())
            .map_err(|error| match error {
                SubmitError::QueueFull { .. } => RequestError::QueueFull,
                SubmitError::Stopped => RequestError::Stopped,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::window_session::test_fixtures::{rename, saved_project};
    use stellar_project::autosave::FlushPolicy;

    fn policy(debounce_ms: u64, max_delay_ms: u64) -> AutosavePolicy {
        AutosavePolicy {
            debounce: Duration::from_millis(debounce_ms),
            max_delay: Duration::from_millis(max_delay_ms),
            app_deactivate_flush: FlushPolicy {
                timeout: Duration::from_millis(500),
            },
            window_close_flush: FlushPolicy {
                timeout: Duration::from_millis(500),
            },
        }
    }

    fn edit(app: &AppRuntime, session: WindowSessionHandle, command: u64, name: &str) {
        let name = name.to_string();
        let request = app
            .submit(move |ctx, request| {
                let window = window_mut(ctx, session)?;
                let tx = rename(window.document().unwrap(), command, 11, &name);
                window.apply(&tx).unwrap();
                Ok(RuntimeEvent::WindowViewChanged { request, session })
            })
            .unwrap();
        assert!(matches!(
            next_event(app),
            RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
        ));
    }

    fn generation(app: &AppRuntime, session: WindowSessionHandle) -> u64 {
        app.state().window(session).unwrap().generation
    }

    /// `until`이 참이 될 때까지 최대 5초 기다립니다.
    fn wait_for(mut until: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if until() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        until()
    }

    fn suspension(app: &AppRuntime, session: WindowSessionHandle) -> Option<AutosaveSuspension> {
        app.sender
            .submit(move |ctx: &mut DocumentContext| {
                ctx.app_sessions()
                    .windows
                    .get(&session)
                    .and_then(|entry| entry.autosave.suspension())
            })
            .unwrap()
            .wait()
            .unwrap()
    }

    #[test]
    fn repository_budget_enables_autosave() {
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave().unwrap();
        assert!(app.autosave.is_some());
        app.shutdown().unwrap();
    }

    #[test]
    fn edits_are_saved_once_after_the_debounce() {
        let project = saved_project("autosave-debounce");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(200, 5_000));
        let (a, _, _) = open_window(&app, open_project(&app, project.path()));
        assert_eq!(generation(&app, a), 1);
        // 연속 편집 세 번은 마지막 편집의 debounce 뒤 한 번만 저장합니다.
        for (n, name) in [(700, "하나"), (701, "둘"), (702, "셋")] {
            edit(&app, a, n, name);
        }
        assert!(app.state().window(a).unwrap().has_unsaved_changes);
        assert!(wait_for(|| !app
            .state()
            .window(a)
            .unwrap()
            .has_unsaved_changes));
        assert_eq!(generation(&app, a), 2, "debounce 안의 편집은 한 번에 저장");
        // 편집이 없으면 더 쓰지 않습니다.
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(generation(&app, a), 2);
        app.shutdown().unwrap();
    }

    #[test]
    fn save_now_merges_the_pending_autosave() {
        let project = saved_project("autosave-save-now");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(300, 5_000));
        let (a, _, _) = open_window(&app, open_project(&app, project.path()));
        edit(&app, a, 700, "바로 저장");
        let request = app.save_now(a).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowSessionSaved { request: got, generation: 2, written: true, .. }
                if got == request
        ));
        // 대기하던 자동 저장은 같은 편집을 다시 쓰지 않습니다.
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(generation(&app, a), 2, "Save Now 뒤 이중 쓰기");
        app.shutdown().unwrap();
    }

    #[test]
    fn flush_saves_without_waiting_for_the_debounce() {
        let project = saved_project("autosave-flush");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(60_000, 60_000));
        let (a, _, _) = open_window(&app, open_project(&app, project.path()));
        app.flush_autosave().unwrap();
        edit(&app, a, 700, "비활성화");
        app.flush_autosave().unwrap();
        assert!(wait_for(|| generation(&app, a) == 2));
        assert!(!app.state().window(a).unwrap().has_unsaved_changes);
        app.shutdown().unwrap();
    }

    #[test]
    fn stale_window_stops_autosaving_without_writing() {
        let project = saved_project("autosave-stale");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(40, 5_000));
        let owner = open_project(&app, project.path());
        let (a, _, _) = open_window(&app, owner);
        let (b, _, _) = open_window(&app, owner);
        // A가 먼저 저장합니다.
        edit(&app, a, 700, "A");
        assert!(wait_for(|| generation(&app, a) == 2));
        let current = std::fs::read(project.path().join("design").join("current")).unwrap();
        // B는 generation 1 위에서 편집했으므로 자동 저장이 stale로 멈추고 디스크를 바꾸지 않습니다.
        edit(&app, b, 800, "B");
        assert!(wait_for(
            || suspension(&app, b) == Some(AutosaveSuspension::Stale)
        ));
        edit(&app, b, 801, "B 다시");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(suspension(&app, b), Some(AutosaveSuspension::Stale));
        assert!(
            app.state().window(b).unwrap().has_unsaved_changes,
            "편집 보존"
        );
        assert_eq!(generation(&app, b), 1);
        assert_eq!(
            std::fs::read(project.path().join("design").join("current")).unwrap(),
            current
        );
        // 창을 닫을 때의 Save Now는 같은 stale을 셸에 돌려줍니다(셸의 닫기 선택 D6).
        let request = app.save_now(b).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { request: got, failure: RequestFailure::SaveStale, .. }
                if got == request
        ));
        app.shutdown().unwrap();
    }
}
