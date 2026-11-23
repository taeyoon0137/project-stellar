//! 지연 자동 저장 트리거 (UX.2.11, XIII.1.3·XIII.1.4·XIII.1.5, ADR-0072).
//!
//! 창마다 [`AutosaveSchedule`]을 두고, document thread가 상태 snapshot을 게시할 때마다(`build_state`, 완료 요청과
//! worker 결과 반영 뒤) 작업 문서 revision·dirty·제스처(history preview) 여부를 관찰합니다. 모든 창 가운데 가장
//! 이른 기한을 시계 thread에 알리면, 시계 thread는 그 시각에 자동 저장 job 하나를 background lane에 같은 병합
//! key로 보냅니다. 그래서 입력·편집 command보다 늦게 처리되고, 밀린 자동 저장 job은 하나로 합쳐집니다.
//!
//! - **캡처와 쓰기**: job은 document thread에서 기한이 지난 창의 문서 레코드와 기준 generation만 캡처하고
//!   ([`ProjectWindowSession::capture_save`]), 직렬화·chunk 쓰기·current pointer 전환은 writer thread
//!   (`stellar-autosave-write`)가 차례로 합니다([`SaveCapture::write`]). writer는 Save Now와 같은 잠금·base generation
//!   검사(stale이면 current pointer를 바꾸지 않음)·원자적 교체를 씁니다. 결과는 command lane job이 document thread에서
//!   반영하고 상태를 게시합니다. 창마다 진행 중인 쓰기는 하나이고, 그동안의 편집은 다음 대기로 셉니다.
//! - **Save Now 합치기**: Save Now(⌘S·창 닫기 flush)는 먼저 진행 중인 자동 저장을 끝까지 기다려 반영하고
//!   ([`settle`]), 남은 편집만 저장합니다. 그래서 같은 revision을 두 번 쓰지 않고, 같은 프로세스 안에서 저장 잠금을
//!   다투지 않습니다. 기다리는 상한은 `window_close_flush_timeout_ms`이고, 넘으면 Save Now가 저장 실패를 돌려줍니다.
//! - **제스처**: history preview가 진행 중인 창은 쓰지 않습니다(연속 drag·scrub 중 디스크 쓰기 0). 지금 셸의
//!   drag·텍스트 조합은 runtime preview를 쓰지 않고 끝날 때 transaction 하나만 보내므로, 그 사이에 그 전 편집의
//!   자동 저장이 한 번 끼어들 수 있습니다. 끼어든 캡처는 document thread에서 짧고 쓰기는 writer가 합니다.
//! - **flush**: 앱 비활성화는 [`AppRuntime::flush_autosave`]로 debounce 없이 캡처합니다. 저장이 진행 중이거나 이미지
//!   가져오기 때문에 미룬 flush는 그 일이 끝난 뒤 flush로 다시 실행합니다. 창 닫기는 셸이 Save Now로 flush합니다.
//! - **실패·stale**: 자동 저장은 요청이 아니므로 완료 event를 만들지 않고 상태 snapshot의 창 요약
//!   ([`WindowSessionSummary::autosave`])에 남깁니다. 실패하면 실패 시각 + 최대 지연에(그 사이 편집이 오면 그 편집의
//!   debounce 뒤에) 다시 시도하고, 저장에 성공할 때까지 실패로 표시합니다. stale이면 clean이 될 때까지 멈춥니다.
//! - **이미지 가져오기**: 가져오기는 시작할 때의 loaded generation에 묶여 있으므로 둘을 서로 배제합니다. 자동 저장은
//!   쓰기 표시(`AppShared::autosave_writing`)를 먼저 세우고 가져오기 표시를 확인하며, 가져오기는 가져오기 표시를 먼저
//!   세우고 쓰기 표시가 내려갈 때까지 기다립니다(둘 다 SeqCst). 쓰기 표시는 결과를 반영해 상태를 게시한 뒤 내립니다.
//!
//! - **stale 두 선택지(XII.7.7, ADR-0083)**: stale을 발견하면 자동 저장을 멈추고 알림을 stale 구간마다 한 번만
//!   요청합니다([`AppRuntime::take_stale_alerts`]). 선택([`AppRuntime::resolve_stale`]) 전에는 쓰지 않습니다.
//! - **외부 변경 반영(UX.4.1)**: 감시 thread([`external`])가 current pointer 변화를 보면 그 프로젝트 창마다
//!   [`ProjectWindowSession::sync_external`]을 document thread에서 실행합니다. 제스처 중이면 끝난 뒤(다음 상태
//!   게시) 다시 실행합니다.
//! - **창 닫기·Git UI flush(XIII.1.6·XIV.2.8)**: 제한 시간 안의 flush 결과를 창 닫기 결정 상태로 돌려주고
//!   ([`AppRuntime::flush_for_close`]), Git UI 전에는 저장 여부를 호출자에게 묻습니다([`AppRuntime::git_ui_preflight`]).
//!   어느 경우도 편집을 자동으로 버리지 않습니다.
//! - **trace(XIII.1.7)**: 자동 저장마다 revision·캡처·직렬화·쓰기 시간·결과를 창별로 최근 [`TRACE_LIMIT`]개 남깁니다.
//!
//! 자동 저장은 [`AppRuntime::enable_autosave`]로 켭니다(네이티브 앱 runtime). 켜지 않은 runtime(테스트, Debug 검사
//! 실행)은 Save Now로만 저장합니다.

use super::*;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc;
use std::sync::Condvar;
use std::thread::JoinHandle;
use std::time::Instant;

use stellar_ops::lock::default_lock_dir;
use stellar_project::autosave::flush::{FlushBlock, FlushOutcome, GitUiChoice, GitUiPreflight};
use stellar_project::autosave::schedule::{AutosaveDecision, AutosaveSchedule, AutosaveStatus};
use stellar_project::autosave::trace::{AutosaveTrace, AutosaveTraceResult};
use stellar_project::autosave::{AutosaveConfigError, AutosavePolicy};
use stellar_project::window_session::{StaleResolution, StaleSaveChoice};

use crate::command_queue::{CoalesceKey, SubmitOptions};
use crate::document_thread::JobTicket;
use crate::window_session::{
    ExternalSync, SaveCapture, SaveTimings, SaveWritten, StaleResolveError, StaleResolved,
};

mod external;
pub(crate) use external::ExternalWatch;
#[cfg(test)]
mod pipeline_tests;

/// 창별로 남기는 최근 자동 저장 trace 수.
pub const TRACE_LIMIT: usize = 32;

/// 앱 runtime이 쓰는 성능 예산 파일(빌드 시 포함). 자동 저장 값의 원본입니다.
const BUDGETS: &str = include_str!("../../../../config/performance-budgets.yaml");

/// background lane에서 자동 저장 job을 합치는 key.
const COALESCE_KEY: CoalesceKey = CoalesceKey::Custom(0x4155_544F_5341_5645);

/// 이미지 가져오기가 끝나기를 기다리는 간격.
const IMPORT_DEFER: Duration = Duration::from_millis(100);

/// 대기열이 찼을 때 다시 보내는 간격.
const RETRY: Duration = Duration::from_millis(10);

/// 시계 thread 이름.
const CLOCK_THREAD_NAME: &str = "stellar-autosave";

/// writer thread 이름.
const WRITER_THREAD_NAME: &str = "stellar-autosave-write";

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

type SaveResult = Result<SaveWritten, WindowSessionSaveError>;

/// writer가 끝낸 쓰기 하나(결과와 writer 쪽 시간).
type SlotResult = (SaveResult, SaveTimings);

/// 진행 중인 쓰기 하나의 결과 자리. writer가 채우고 document thread가 꺼냅니다.
#[derive(Debug, Default)]
pub(crate) struct SaveSlot {
    result: Mutex<Option<SlotResult>>,
    ready: Condvar,
}

impl SaveSlot {
    fn lock(&self) -> MutexGuard<'_, Option<SlotResult>> {
        self.result.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn put(&self, result: SaveResult, timings: SaveTimings) {
        *self.lock() = Some((result, timings));
        self.ready.notify_all();
    }

    fn take(&self) -> Option<SlotResult> {
        self.lock().take()
    }

    /// 결과가 올 때까지 `deadline`까지 기다립니다.
    fn wait_until(&self, deadline: Instant) -> Option<SlotResult> {
        let mut result = self.lock();
        loop {
            if let Some(result) = result.take() {
                return Some(result);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            result = self
                .ready
                .wait_timeout(result, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// writer에 보내는 쓰기 하나. 처리하지 못하고 버려지면 결과 자리에 [`WindowSessionSaveError::Interrupted`]를 둡니다.
struct WriteJob {
    capture: Option<SaveCapture>,
    slot: Arc<SaveSlot>,
}

impl Drop for WriteJob {
    fn drop(&mut self) {
        if self.capture.take().is_some() {
            self.slot.put(
                Err(WindowSessionSaveError::Interrupted),
                SaveTimings::default(),
            );
        }
    }
}

/// document thread 쪽 자동 저장 설정([`AppSessions`] 안).
#[derive(Debug)]
pub(crate) struct AutosaveSetup {
    policy: AutosavePolicy,
    wake: Arc<AutosaveWake>,
    writer: mpsc::Sender<WriteJob>,
    /// 진행 중인 저장·이미지 가져오기 때문에 다 하지 못한 flush. 그 일이 끝난 뒤 flush로 다시 실행합니다.
    deferred_flush: bool,
}

/// 창 하나의 자동 저장 상태([`WindowEntry`] 안). 자동 저장을 켠 runtime에서 처음 관찰할 때 만듭니다.
#[derive(Debug, Default)]
pub(crate) struct WindowAutosave {
    schedule: Option<AutosaveSchedule>,
    in_flight: Option<Arc<SaveSlot>>,
    /// 진행 중인 쓰기의 캡처 revision과 캡처 시간(trace용).
    in_flight_capture: Option<(DocumentRevision, Duration)>,
    /// 마지막으로 발견한 stale의 다음 할 일(두 선택지 또는 reconciliation 대기). clean이 되면 지웁니다.
    stale: Option<StaleResolution>,
    /// 제스처·진행 중인 쓰기 때문에 미룬 외부 변경 반영.
    external_deferred: bool,
    /// 마지막으로 반영을 거부한 외부 변경의 진단(UX.4.4). 다음 정상 반영에서 지웁니다.
    rejection: Option<String>,
    /// 최근 자동 저장 trace(XIII.1.7).
    traces: std::collections::VecDeque<AutosaveTrace>,
}

impl WindowAutosave {
    /// 상태 snapshot에 게시할 자동 저장 상태(자동 저장을 켜지 않았으면 [`AutosaveStatus::Idle`]).
    pub(crate) fn status(&self) -> AutosaveStatus {
        self.schedule
            .as_ref()
            .map_or(AutosaveStatus::Idle, AutosaveSchedule::status)
    }
}

/// 시계·writer thread 소유자([`AppRuntime`] 안).
#[derive(Debug)]
pub(crate) struct AutosaveClock {
    wake: Arc<AutosaveWake>,
    thread: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    /// 외부 변경 감시 thread(UX.4.1).
    external: Option<ExternalWatch>,
}

impl AutosaveClock {
    /// 외부 변경 감시와 시계 thread를 멈추고 끝날 때까지 기다립니다(대기 중인 condvar·짧은 poll만 깨우므로 바로
    /// 끝남). 감시를 먼저 멈춰 종료 중에 새 반영 job을 접수하지 않습니다.
    pub(crate) fn stop(&mut self) {
        if let Some(external) = self.external.as_mut() {
            external.stop();
        }
        self.wake.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// document thread가 끝난 뒤 진행 중인 쓰기가 끝날 때까지 `timeout`까지 기다립니다. writer는 document thread가
    /// 가진 송신자가 사라지면 남은 쓰기를 마치고 끝납니다. 넘으면 thread를 분리합니다.
    pub(crate) fn finish_writer(&mut self, timeout: Duration) {
        let Some(writer) = self.writer.take() else {
            return;
        };
        let deadline = Instant::now() + timeout;
        while !writer.is_finished() {
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = writer.join();
    }
}

fn gesture_active(window: &ProjectWindowSession) -> bool {
    window.history().history().preview_active()
}

fn any_in_flight(sessions: &AppSessions) -> bool {
    sessions
        .windows
        .values()
        .any(|entry| entry.autosave.in_flight.is_some())
}

/// 모든 창의 상태를 관찰하고 가장 이른 기한을 시계에 알립니다. `build_state`가 창 요약을 만들기 전에 부릅니다.
pub(super) fn observe(ctx: &mut DocumentContext) {
    let sessions = ctx.app_sessions_mut();
    // 제스처·쓰기가 끝난 창의 미룬 외부 변경을 먼저 반영합니다(UX.4.1 "제스처가 끝난 뒤 반영").
    for entry in sessions.windows.values_mut() {
        if entry.autosave.external_deferred
            && !gesture_active(&entry.session)
            && entry.autosave.in_flight.is_none()
        {
            sync_window(entry);
        }
        if entry.autosave.stale.is_some() && !entry.session.has_unsaved_changes() {
            entry.autosave.stale = None;
        }
    }
    let Some(setup) = sessions.autosave.as_ref() else {
        return;
    };
    let now = setup.wake.now();
    let mut next: Option<Duration> = None;
    for entry in sessions.windows.values_mut() {
        let schedule = entry
            .autosave
            .schedule
            .get_or_insert_with(|| AutosaveSchedule::new(setup.policy));
        let window = &entry.session;
        schedule.observe(now, window.revision(), window.has_unsaved_changes());
        let due = match schedule.decide(now, gesture_active(window)) {
            AutosaveDecision::Wait { until } => until,
            AutosaveDecision::Save => now,
            AutosaveDecision::Idle
            | AutosaveDecision::Gesture
            | AutosaveDecision::Saving
            | AutosaveDecision::Stale => continue,
        };
        next = Some(next.map_or(due, |current| current.min(due)));
    }
    if setup.deferred_flush {
        // 미룬 flush(이미지 가져오기·진행 중 쓰기)는 debounce 기한이 아니라 짧은 재시도 기한을 지킵니다. 그 사이의
        // 다른 상태 게시(외부 변경 반영 등)가 기한을 debounce로 밀어내지 않게 합니다.
        let retry = now + IMPORT_DEFER;
        next = Some(next.map_or(retry, |current| current.min(retry)));
    }
    setup.wake.arm(next);
}

/// writer 결과 하나를 창에 반영하고 trace를 남깁니다.
fn finish(entry: &mut WindowEntry, (result, timings): SlotResult, now: Duration) {
    let (revision, capture) = entry
        .autosave
        .in_flight_capture
        .take()
        .unwrap_or((entry.session.revision(), Duration::ZERO));
    let trace_result = match &result {
        Ok(written) if written.wrote() => AutosaveTraceResult::Saved,
        Ok(_) => AutosaveTraceResult::Unchanged,
        Err(WindowSessionSaveError::Stale(_)) => AutosaveTraceResult::Stale,
        Err(WindowSessionSaveError::Interrupted) => AutosaveTraceResult::Interrupted,
        Err(_) => AutosaveTraceResult::Failed,
    };
    record_trace(
        &mut entry.autosave,
        AutosaveTrace {
            dirty_revision: revision,
            capture,
            serialize: timings.serialize,
            write: timings.write,
            result: trace_result,
        },
    );
    let Some(schedule) = entry.autosave.schedule.as_mut() else {
        return;
    };
    match result {
        Ok(written) => {
            entry.session.apply_save(written);
            schedule.saved();
        }
        Err(WindowSessionSaveError::Stale(_)) => {
            schedule.stale(now);
            entry.autosave.stale = Some(stale_resolution(&entry.session));
        }
        Err(error) => {
            eprintln!("Stellar autosave: {error}");
            schedule.failed(now);
        }
    }
}

fn record_trace(autosave: &mut WindowAutosave, trace: AutosaveTrace) {
    if autosave.traces.len() >= TRACE_LIMIT {
        autosave.traces.pop_front();
    }
    autosave.traces.push_back(trace);
}

/// 디스크 current를 만든 writer로 정한 다음 할 일. 판정하지 못하면 두 선택지입니다(쓰기를 이미 멈춘 상태).
fn stale_resolution(window: &ProjectWindowSession) -> StaleResolution {
    window
        .stale_state()
        .ok()
        .flatten()
        .map_or(StaleResolution::Choose, |(_, resolution)| resolution)
}

/// 외부 변경 반영 transaction id: 창 영속 id와 반영 전 revision에서 만듭니다(창마다 revision마다 유일).
fn external_command_id(window: &ProjectWindowSession) -> stellar_commands::id::CommandId {
    let mut key = [0; 32];
    key[..16].copy_from_slice(&window.window().stable_id().to_bytes());
    key[16..24].copy_from_slice(&window.revision().0.to_le_bytes());
    key[24..].copy_from_slice(b"external");
    let digest = stellar_storage::hash::ContentHash::compute(&key);
    let mut raw = [0; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    stellar_commands::id::CommandId::from_bytes(raw)
}

/// 창 하나에 디스크의 외부 변경을 반영합니다(UX.4.1). 반영했으면 `true`입니다.
fn sync_window(entry: &mut WindowEntry) -> bool {
    if entry.autosave.in_flight.is_some() {
        // 진행 중인 자기 저장이 곧 base를 바꿉니다. 결과를 반영한 뒤 다시 판단합니다.
        entry.autosave.external_deferred = true;
        return false;
    }
    let command = external_command_id(&entry.session);
    match entry.session.sync_external(command) {
        ExternalSync::Unchanged => {
            entry.autosave.external_deferred = false;
            false
        }
        ExternalSync::Deferred => {
            entry.autosave.external_deferred = true;
            false
        }
        ExternalSync::Applied { .. } => {
            entry.autosave.external_deferred = false;
            entry.autosave.rejection = None;
            true
        }
        ExternalSync::Stale { resolution, .. } => {
            // 자동 저장을 멈추고 알림은 stale 구간마다 한 번만(ADR-0083 결정 4).
            entry.autosave.external_deferred = false;
            entry.autosave.stale = Some(resolution);
            if let Some(schedule) = entry.autosave.schedule.as_mut() {
                schedule.mark_stale();
            }
            false
        }
        ExternalSync::Rejected(rejection) => {
            entry.autosave.external_deferred = false;
            entry.autosave.rejection = Some(rejection.to_string());
            false
        }
    }
}

/// `project`의 모든 창에 외부 변경을 반영하고, 반영한 창에 요청 없는 표시 변경을 알립니다.
fn sync_project(ctx: &mut DocumentContext, shared: &AppShared, project: ProjectSessionId) {
    let sessions = ctx.app_sessions_mut();
    let mut applied = Vec::new();
    for (handle, entry) in sessions
        .windows
        .iter_mut()
        .filter(|(_, entry)| entry.project == project)
    {
        if sync_window(entry) {
            applied.push(*handle);
        }
    }
    shared.publish_state(build_state(ctx));
    for session in applied {
        shared.push_unsolicited_view_change(session);
    }
}

/// Save Now 전에 `session`과 같은 프로젝트에서 진행 중인 자동 저장을 기다려 반영합니다(XIII.1.5). 같은 프로세스의
/// 쓰기는 writer 하나가 차례로 하므로 이 뒤의 Save Now는 그 프로젝트의 저장 잠금을 다투지 않고 최신 기준 generation
/// 위에서 씁니다. 다른 프로젝트의 쓰기는 잠금·generation이 따로라 기다리지 않습니다(느린 다른 프로젝트 때문에 이 창의
/// Save Now가 늦거나 실패하지 않게). 쓰기 표시는 writer가 보내는 반영 job이 내립니다. 기다리는 상한
/// (`window_close_flush_timeout_ms`)을 넘으면 [`WindowSessionSaveError::Interrupted`]입니다.
pub(super) fn settle(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
) -> Result<(), WindowSessionSaveError> {
    let sessions = ctx.app_sessions_mut();
    let Some(setup) = sessions.autosave.as_ref() else {
        return Ok(());
    };
    let Some(project) = sessions.windows.get(&session).map(|entry| entry.project) else {
        return Ok(());
    };
    let deadline = Instant::now() + setup.policy.window_close_flush.timeout;
    for entry in sessions
        .windows
        .values_mut()
        .filter(|entry| entry.project == project)
    {
        let Some(slot) = entry.autosave.in_flight.clone() else {
            continue;
        };
        let result = slot
            .wait_until(deadline)
            .ok_or(WindowSessionSaveError::Interrupted)?;
        entry.autosave.in_flight = None;
        finish(entry, result, setup.wake.now());
    }
    Ok(())
}

/// writer가 끝낸 결과를 반영하고 상태를 게시한 뒤 쓰기 표시를 고칩니다. 미룬 flush·기한이 지난 창이 있으면 이어서
/// 실행합니다.
fn collect(ctx: &mut DocumentContext, shared: &AppShared) {
    let sessions = ctx.app_sessions_mut();
    let Some(setup) = sessions.autosave.as_ref() else {
        return;
    };
    let now = setup.wake.now();
    let mut changed = false;
    for entry in sessions.windows.values_mut() {
        let Some(result) = entry
            .autosave
            .in_flight
            .as_ref()
            .and_then(|slot| slot.take())
        else {
            continue;
        };
        entry.autosave.in_flight = None;
        finish(entry, result, now);
        changed = true;
    }
    if changed {
        shared.publish_state(build_state(ctx));
    }
    shared
        .autosave_writing
        .store(any_in_flight(ctx.app_sessions()), Ordering::SeqCst);
    run(ctx, shared, false);
}

/// 기한이 지난 창(`flush`이면 debounce와 무관하게 대기 중인 모든 창)을 캡처해 writer에 보냅니다. 보냈으면 상태를
/// 게시하고, 아니면 다음 기한만 다시 알립니다.
fn run(ctx: &mut DocumentContext, shared: &AppShared, flush: bool) {
    let sessions = ctx.app_sessions_mut();
    let Some(setup) = sessions.autosave.as_mut() else {
        return;
    };
    let flush = flush || std::mem::take(&mut setup.deferred_flush);
    let now = setup.wake.now();
    let mut due = Vec::new();
    let mut defer = false;
    for (handle, entry) in &sessions.windows {
        let Some(schedule) = entry.autosave.schedule.as_ref() else {
            continue;
        };
        let gesture = gesture_active(&entry.session);
        let save = if flush {
            defer |= entry.autosave.in_flight.is_some();
            schedule.should_flush(gesture)
        } else {
            schedule.decide(now, gesture) == AutosaveDecision::Save
        };
        if save {
            due.push(*handle);
        }
    }
    setup.deferred_flush |= defer;
    if due.is_empty() {
        observe(ctx);
        return;
    }
    // 이미지 가져오기와 서로 배제합니다(모듈 문서). 쓰기 표시를 먼저 세우고 가져오기 표시를 확인합니다.
    shared.autosave_writing.store(true, Ordering::SeqCst);
    if shared.importing_image.load(Ordering::SeqCst) {
        setup.deferred_flush |= flush;
        setup.wake.arm(Some(now + IMPORT_DEFER));
        shared
            .autosave_writing
            .store(any_in_flight(sessions), Ordering::SeqCst);
        return;
    }
    for handle in due {
        let Some(entry) = sessions.windows.get_mut(&handle) else {
            continue;
        };
        let window = &entry.session;
        let Some(schedule) = entry.autosave.schedule.as_mut() else {
            continue;
        };
        if !window.has_unsaved_changes() {
            schedule.observe(now, window.revision(), false);
            continue;
        }
        let started = Instant::now();
        let captured = window.capture_save();
        let capture_time = started.elapsed();
        match captured {
            Ok(Some(capture)) => {
                schedule.begin_save(capture.revision());
                let slot = Arc::new(SaveSlot::default());
                entry.autosave.in_flight = Some(Arc::clone(&slot));
                entry.autosave.in_flight_capture = Some((capture.revision(), capture_time));
                if setup
                    .writer
                    .send(WriteJob {
                        capture: Some(capture),
                        slot: Arc::clone(&slot),
                    })
                    .is_err()
                {
                    // writer가 없으면 버려진 job이 결과 자리에 중단을 남깁니다.
                    entry.autosave.in_flight = None;
                    let result = slot.take().unwrap_or((
                        Err(WindowSessionSaveError::Interrupted),
                        SaveTimings::default(),
                    ));
                    finish(entry, result, now);
                }
            }
            Ok(None) => schedule.observe(now, window.revision(), false),
            Err(error) => {
                schedule.begin_save(window.revision());
                entry.autosave.in_flight_capture = Some((window.revision(), capture_time));
                finish(entry, (Err(error), SaveTimings::default()), now);
            }
        }
    }
    shared.publish_state(build_state(ctx));
    shared
        .autosave_writing
        .store(any_in_flight(ctx.app_sessions()), Ordering::SeqCst);
}

/// 결과 반영 job을 command lane에 보냅니다(밀려나지 않게 병합 key 없음). 대기열이 차면 잠시 뒤 다시 보냅니다.
fn submit_collect(sender: &DocumentSender, shared: &Arc<AppShared>) {
    loop {
        let job_shared = Arc::clone(shared);
        match sender.try_submit(
            SubmitOptions::command(),
            move |ctx: &mut DocumentContext| collect(ctx, &job_shared),
        ) {
            Ok(_) | Err(SubmitError::Stopped) => return,
            Err(SubmitError::QueueFull { .. }) => std::thread::sleep(RETRY),
        }
    }
}

/// writer thread: 쓰기를 받은 순서대로 하나씩 하고, 결과마다 반영 job을 보냅니다.
fn writer_loop(jobs: mpsc::Receiver<WriteJob>, sender: DocumentSender, shared: Arc<AppShared>) {
    while let Ok(mut job) = jobs.recv() {
        let Some(capture) = job.capture.take() else {
            continue;
        };
        let (result, timings) =
            std::panic::catch_unwind(AssertUnwindSafe(|| capture.write_timed(default_lock_dir)))
                .unwrap_or((
                    Err(WindowSessionSaveError::Interrupted),
                    SaveTimings::default(),
                ));
        job.slot.put(result, timings);
        submit_collect(&sender, &shared);
    }
}

/// 시계 thread: 기한까지 기다렸다가 자동 저장 job을 보냅니다. job이 실행되면 다음 기한을 다시 알립니다.
fn clock_loop(wake: Arc<AutosaveWake>, sender: DocumentSender, shared: Arc<AppShared>) {
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
            Err(SubmitError::QueueFull { .. }) => wake.arm(Some(wake.now() + IMPORT_DEFER)),
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
        let (writer_tx, writer_rx) = mpsc::channel();
        let writer_sender = self.sender.clone();
        let writer_shared = Arc::clone(&self.shared);
        let writer = match std::thread::Builder::new()
            .name(WRITER_THREAD_NAME.into())
            .spawn(move || writer_loop(writer_rx, writer_sender, writer_shared))
        {
            Ok(writer) => writer,
            Err(error) => {
                eprintln!("Stellar autosave writer: {error}");
                return;
            }
        };
        // 설정 job은 command lane FIFO라 이 뒤에 접수한 요청보다 먼저 실행됩니다.
        let setup = AutosaveSetup {
            policy,
            wake: Arc::clone(&wake),
            writer: writer_tx,
            deferred_flush: false,
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
                let external = ExternalWatch::start(self.sender.clone(), Arc::clone(&self.shared));
                self.autosave = Some(AutosaveClock {
                    wake,
                    thread: Some(thread),
                    writer: Some(writer),
                    external,
                })
            }
            Err(error) => eprintln!("Stellar autosave clock: {error}"),
        }
    }

    /// 자동 저장 시계를 멈추고 document thread를 종료한 뒤(`timeout`까지), 진행 중인 자동 저장 쓰기가 끝나기를
    /// 기다립니다. 시계를 먼저 멈춰 종료 중에 새 저장을 접수하지 않습니다. document thread가 제때 끝나지 않았으면
    /// writer도 기다리지 않습니다.
    ///
    /// 순서는 XII.1.6의 고정 순서([`crate::document_thread::ShutdownStage::ORDER`])를 따릅니다: 셸이 입력을 멈춘 뒤
    /// (UI stop) 감시·시계가 새 job을 내지 않게 하고 document thread가 남은 command를 실행하며(command drain/cancel),
    /// 그다음 worker(writer)를 기다립니다(worker stop). 앱 runtime은 render thread를 소유하지 않습니다.
    pub(super) fn stop_with_autosave(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<DocumentLoopStats, DocumentThreadError> {
        if let Some(clock) = self.autosave.as_mut() {
            clock.stop();
        }
        let stopped = self.document.stop_within(timeout);
        if stopped.is_ok() {
            if let Some(clock) = self.autosave.as_mut() {
                clock.finish_writer(timeout.unwrap_or(AppRuntime::SHUTDOWN_TIMEOUT));
            }
        }
        stopped
    }

    /// 앱 비활성화 flush: 저장되지 않은 완료 편집을 debounce 없이 캡처하도록 접수합니다. 기다리지 않고 완료 event도
    /// 없습니다. 제스처 중이거나 stale인 창은 쓰지 않고, 저장이 진행 중인 창은 그 저장이 끝난 뒤 flush합니다. 자동
    /// 저장을 켜지 않았으면 아무것도 하지 않습니다.
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

    /// document thread에서 `job`을 실행하고 결과 ticket을 돌려줍니다(완료 event 없음).
    fn job<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut DocumentContext) -> T + Send + 'static,
    ) -> Result<JobTicket<T>, RequestError> {
        self.sender.submit(job).map_err(|error| match error {
            SubmitError::QueueFull { .. } => RequestError::QueueFull,
            SubmitError::Stopped => RequestError::Stopped,
        })
    }

    /// 아직 요청하지 않은 stale 알림을 꺼냅니다(XII.7.7). 한 stale 구간의 알림은 한 번만 나옵니다. 셸은 각 알림의
    /// [`StaleResolution::choices`]로 두 선택지 시트를 띄우거나, reconciliation 대기를 표시합니다.
    pub fn take_stale_alerts(&self) -> Result<JobTicket<Vec<StaleAlert>>, RequestError> {
        self.job(|ctx: &mut DocumentContext| {
            let mut alerts = Vec::new();
            for (handle, entry) in ctx.app_sessions_mut().windows.iter_mut() {
                let Some(resolution) = entry.autosave.stale else {
                    continue;
                };
                // 자동 저장을 켜지 않은 runtime은 schedule이 없으므로 한 번 꺼내면 지웁니다.
                let alert = match entry.autosave.schedule.as_mut() {
                    Some(schedule) => schedule.take_stale_alert(),
                    None => std::mem::take(&mut entry.autosave.stale).is_some(),
                };
                if alert {
                    alerts.push(StaleAlert {
                        session: *handle,
                        resolution,
                    });
                }
            }
            alerts
        })
    }

    /// stale 창의 사용자 선택을 실행합니다(XII.7.5·XII.7.6). 진행 중인 자동 저장을 먼저 반영한 뒤 선택을 실행하고,
    /// clean이 되면 자동 저장이 다시 동작합니다. 외부 writer와 겹친 stale은
    /// [`StaleResolveError::ReconciliationRequired`]입니다. 실패하면 편집·history는 그대로입니다.
    pub fn resolve_stale(
        &self,
        session: WindowSessionHandle,
        choice: StaleSaveChoice,
    ) -> Result<JobTicket<Result<StaleResolved, StaleResolveError>>, RequestError> {
        let shared = Arc::clone(&self.shared);
        self.job(move |ctx: &mut DocumentContext| {
            settle(ctx, session).map_err(StaleResolveError::Save)?;
            let sessions = ctx.app_sessions_mut();
            let now = sessions.autosave.as_ref().map(|setup| setup.wake.now());
            let entry = sessions
                .windows
                .get_mut(&session)
                .ok_or(StaleResolveError::NotStale)?;
            let resolved = entry.session.resolve_stale(choice);
            if resolved.is_ok() {
                entry.autosave.stale = None;
                entry.autosave.external_deferred = false;
                if let (Some(schedule), Some(now)) = (entry.autosave.schedule.as_mut(), now) {
                    schedule.observe(
                        now,
                        entry.session.revision(),
                        entry.session.has_unsaved_changes(),
                    );
                }
            }
            shared.publish_state(build_state(ctx));
            if resolved.is_ok() {
                shared.push_unsolicited_view_change(session);
            }
            resolved
        })
    }

    /// `project`의 창들에 디스크의 외부 변경을 지금 반영합니다(UX.4.1). 감시 thread가 current pointer 변화마다 같은
    /// 일을 합니다.
    pub fn sync_external_changes(
        &self,
        project: ProjectSessionId,
    ) -> Result<JobTicket<()>, RequestError> {
        let shared = Arc::clone(&self.shared);
        self.job(move |ctx: &mut DocumentContext| sync_project(ctx, &shared, project))
    }

    /// 창 하나의 외부 변경 상태(stale·거부 진단·미룸, UX.4.1·UX.4.4). 창이 없으면 `None`.
    pub fn external_status(
        &self,
        session: WindowSessionHandle,
    ) -> Result<JobTicket<Option<ExternalStatus>>, RequestError> {
        self.job(move |ctx: &mut DocumentContext| {
            let entry = ctx.app_sessions().windows.get(&session)?;
            Some(ExternalStatus {
                stale: entry.autosave.stale,
                rejection: entry.autosave.rejection.clone(),
                deferred: entry.autosave.external_deferred,
            })
        })
    }

    /// 창 하나의 최근 자동 저장 trace(XIII.1.7, 오래된 것부터).
    pub fn autosave_traces(
        &self,
        session: WindowSessionHandle,
    ) -> Result<JobTicket<Vec<AutosaveTrace>>, RequestError> {
        self.job(move |ctx: &mut DocumentContext| {
            ctx.app_sessions()
                .windows
                .get(&session)
                .map(|entry| entry.autosave.traces.iter().copied().collect())
                .unwrap_or_default()
        })
    }

    /// 창 닫기 flush(XIII.1.6): `timeout` 안에 진행 중인 자동 저장을 반영하고 남은 편집을 저장한 뒤, 창 닫기 결정
    /// 상태를 돌려줍니다. 제한 시간을 넘거나 저장할 수 없으면 [`FlushOutcome::Blocked`]이고 편집은 메모리에
    /// 남습니다(넘긴 flush job은 그대로 끝까지 실행되며 취소·폐기하지 않습니다).
    pub fn flush_for_close(&self, session: WindowSessionHandle, timeout: Duration) -> FlushOutcome {
        let shared = Arc::clone(&self.shared);
        let ticket = self.job(move |ctx: &mut DocumentContext| {
            let outcome = flush_window(ctx, session);
            shared.publish_state(build_state(ctx));
            outcome
        });
        wait_flush(ticket, timeout)
    }

    /// 앱 비활성화 flush(XIII.1.6)를 제한 시간 안에 끝까지 기다립니다. 창마다 결정 상태를 돌려주며, 제한 시간을
    /// 넘으면 모든 창이 [`FlushBlock::TimedOut`]입니다. 기다리지 않는 접수만 필요하면
    /// [`AppRuntime::flush_autosave`]를 씁니다.
    pub fn flush_for_deactivate(
        &self,
        timeout: Duration,
    ) -> Vec<(WindowSessionHandle, FlushOutcome)> {
        let state = self.state();
        let handles: Vec<_> = state.windows.iter().map(|window| window.session).collect();
        let shared = Arc::clone(&self.shared);
        let job_handles = handles.clone();
        let ticket = self.job(move |ctx: &mut DocumentContext| {
            let outcomes: Vec<_> = job_handles
                .iter()
                .map(|session| (*session, flush_window(ctx, *session)))
                .collect();
            shared.publish_state(build_state(ctx));
            outcomes
        });
        let timed_out = || {
            handles
                .iter()
                .map(|session| (*session, FlushOutcome::Blocked(FlushBlock::TimedOut)))
                .collect()
        };
        match ticket {
            Ok(mut ticket) => match ticket.wait_timeout(timeout) {
                Ok(Some(outcomes)) => outcomes,
                _ => timed_out(),
            },
            Err(_) => timed_out(),
        }
    }

    /// Git UI를 열기 전 창의 저장 상태(XIV.2.8). 이 호출은 아무것도 저장하지 않습니다. 창이 없으면 `None`.
    pub fn git_ui_preflight(
        &self,
        session: WindowSessionHandle,
    ) -> Result<JobTicket<Option<GitUiPreflight>>, RequestError> {
        self.job(move |ctx: &mut DocumentContext| {
            ctx.app_sessions().windows.get(&session).map(git_ui_state)
        })
    }

    /// Git UI 전 호출자의 선택을 실행합니다(XIV.2.8). [`GitUiChoice::SaveFirst`]는 창 닫기와 같은 flush를 하고,
    /// [`GitUiChoice::ContinueWithDisk`]는 아무것도 쓰지 않고 지금 상태를 돌려줍니다(메모리 편집 유지). 결과가
    /// [`GitUiPreflight::Clean`]이면 디스크가 메모리와 같습니다.
    pub fn prepare_git_ui(
        &self,
        session: WindowSessionHandle,
        choice: GitUiChoice,
        timeout: Duration,
    ) -> GitUiPreflight {
        match choice {
            GitUiChoice::SaveFirst => match self.flush_for_close(session, timeout) {
                FlushOutcome::Clean | FlushOutcome::Saved { .. } => GitUiPreflight::Clean,
                FlushOutcome::Blocked(block) => GitUiPreflight::Blocked(block),
            },
            GitUiChoice::ContinueWithDisk => {
                let ticket = self.git_ui_preflight(session);
                match ticket.map(|mut ticket| ticket.wait_timeout(timeout)) {
                    Ok(Ok(Some(Some(state)))) => state,
                    Ok(Ok(Some(None))) => GitUiPreflight::Clean,
                    _ => GitUiPreflight::Blocked(FlushBlock::TimedOut),
                }
            }
        }
    }
}

/// stale 알림 하나(XII.7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleAlert {
    /// stale 창.
    pub session: WindowSessionHandle,
    /// 두 선택지 또는 reconciliation 대기.
    pub resolution: StaleResolution,
}

/// 창 하나의 외부 변경 상태(UX.4.1·UX.4.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalStatus {
    /// stale이면 다음 할 일.
    pub stale: Option<StaleResolution>,
    /// 마지막으로 반영을 거부한 외부 변경의 진단(경로·줄·원인을 담을 수 있음).
    pub rejection: Option<String>,
    /// 제스처·진행 중인 쓰기 때문에 미룬 반영이 있는지.
    pub deferred: bool,
}

/// 저장 실패의 경로 없는 code(셸 표시·trace용).
fn save_failure_code(error: &WindowSessionSaveError) -> &'static str {
    match error {
        WindowSessionSaveError::ImageMetadata => "image-metadata",
        WindowSessionSaveError::Lock(_) => "lock",
        WindowSessionSaveError::Stale(_) => "stale",
        WindowSessionSaveError::Serialize(_) => "serialize",
        WindowSessionSaveError::Plan(_) => "plan",
        WindowSessionSaveError::Run(_) => "write",
        WindowSessionSaveError::Reload(_) => "reload",
        WindowSessionSaveError::Interrupted => "interrupted",
        WindowSessionSaveError::Rebase(_) => "rebase",
    }
}

fn wait_flush(
    ticket: Result<JobTicket<FlushOutcome>, RequestError>,
    timeout: Duration,
) -> FlushOutcome {
    match ticket {
        Ok(mut ticket) => match ticket.wait_timeout(timeout) {
            Ok(Some(outcome)) => outcome,
            Ok(None) => FlushOutcome::Blocked(FlushBlock::TimedOut),
            Err(_) => FlushOutcome::Blocked(FlushBlock::Failed("interrupted".into())),
        },
        Err(_) => FlushOutcome::Blocked(FlushBlock::Failed("runtime-stopped".into())),
    }
}

/// document thread의 flush 하나(XIII.1.6). 편집을 버리지 않습니다.
fn flush_window(ctx: &mut DocumentContext, session: WindowSessionHandle) -> FlushOutcome {
    if let Err(error) = settle(ctx, session) {
        return FlushOutcome::Blocked(match error {
            WindowSessionSaveError::Interrupted => FlushBlock::TimedOut,
            other => FlushBlock::Failed(save_failure_code(&other).into()),
        });
    }
    let sessions = ctx.app_sessions_mut();
    let now = sessions.autosave.as_ref().map(|setup| setup.wake.now());
    let Some(entry) = sessions.windows.get_mut(&session) else {
        return FlushOutcome::Clean;
    };
    if !entry.session.has_unsaved_changes() {
        return FlushOutcome::Clean;
    }
    if gesture_active(&entry.session) {
        return FlushOutcome::Blocked(FlushBlock::GestureInProgress);
    }
    match entry.session.save_now() {
        Ok(outcome) => {
            if let (Some(schedule), Some(now)) = (entry.autosave.schedule.as_mut(), now) {
                schedule.observe(now, entry.session.revision(), false);
            }
            if outcome.written {
                FlushOutcome::Saved {
                    generation: outcome.generation,
                }
            } else {
                FlushOutcome::Clean
            }
        }
        Err(WindowSessionSaveError::Stale(_)) => {
            let resolution = stale_resolution(&entry.session);
            entry.autosave.stale = Some(resolution);
            if let Some(schedule) = entry.autosave.schedule.as_mut() {
                schedule.mark_stale();
            }
            FlushOutcome::Blocked(FlushBlock::Stale(resolution))
        }
        Err(error) => FlushOutcome::Blocked(FlushBlock::Failed(save_failure_code(&error).into())),
    }
}

/// Git UI 전 상태(XIV.2.8). 쓰지 않습니다.
fn git_ui_state(entry: &WindowEntry) -> GitUiPreflight {
    if !entry.session.has_unsaved_changes() {
        return GitUiPreflight::Clean;
    }
    if let Some(resolution) = entry.autosave.stale {
        return GitUiPreflight::Blocked(FlushBlock::Stale(resolution));
    }
    if gesture_active(&entry.session) {
        return GitUiPreflight::Blocked(FlushBlock::GestureInProgress);
    }
    GitUiPreflight::PendingEdits {
        revision: entry.session.revision(),
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
                timeout: Duration::from_millis(5_000),
            },
        }
    }

    /// 요청 없는 표시 변경 event(외부 변경 반영)를 건너뛴 다음 완료 event.
    pub(super) fn next_solicited(app: &AppRuntime) -> RuntimeEvent {
        loop {
            let event = next_event(app);
            if event.request() != RequestId(0) {
                return event;
            }
        }
    }

    fn edit(app: &AppRuntime, session: WindowSessionHandle, command: u64, name: &str) {
        let name = name.to_string();
        let request = app
            .submit(move |ctx, request| {
                let window = window_mut(ctx, session)?;
                let tx = rename(window.document().unwrap(), command, 11, &name);
                window.apply(&tx, 0).unwrap();
                Ok(RuntimeEvent::WindowViewChanged { request, session })
            })
            .unwrap();
        assert!(matches!(
            next_solicited(app),
            RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
        ));
    }

    fn generation(app: &AppRuntime, session: WindowSessionHandle) -> u64 {
        app.state().window(session).unwrap().generation
    }

    fn status(app: &AppRuntime, session: WindowSessionHandle) -> AutosaveStatus {
        app.state().window(session).unwrap().autosave
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
        assert_eq!(status(&app, a), AutosaveStatus::Idle);
        // 연속 편집 세 번은 마지막 편집의 debounce 뒤 한 번만 저장합니다.
        for (n, name) in [(700, "하나"), (701, "둘"), (702, "셋")] {
            edit(&app, a, n, name);
        }
        let state = app.state();
        let window = state.window(a).unwrap();
        assert!(window.has_unsaved_changes);
        assert_eq!(window.autosave, AutosaveStatus::Pending);
        assert!(wait_for(|| !app
            .state()
            .window(a)
            .unwrap()
            .has_unsaved_changes));
        assert_eq!(generation(&app, a), 2, "debounce 안의 편집은 한 번에 저장");
        assert_eq!(status(&app, a), AutosaveStatus::Idle);
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
    fn save_now_waits_for_the_autosave_in_flight() {
        // flush가 writer에 보낸 쓰기 바로 뒤의 Save Now는 그 결과를 반영한 뒤 판단하므로 stale·잠금 경합이 없고
        // 같은 revision을 다시 쓰지 않습니다.
        for round in 0..5 {
            let project = saved_project("autosave-settle");
            let mut app = AppRuntime::start().unwrap();
            app.enable_autosave_with(policy(60_000, 60_000));
            let (a, _, _) = open_window(&app, open_project(&app, project.path()));
            edit(&app, a, 700 + round, "진행 중 저장");
            app.flush_autosave().unwrap();
            let request = app.save_now(a).unwrap();
            assert!(
                matches!(
                    next_event(&app),
                    RuntimeEvent::WindowSessionSaved { request: got, generation: 2, .. }
                        if got == request
                ),
                "round {round}"
            );
            assert!(!app.state().window(a).unwrap().has_unsaved_changes);
            assert!(wait_for(|| !app
                .shared
                .autosave_writing
                .load(Ordering::SeqCst)));
            assert_eq!(generation(&app, a), 2);
            app.shutdown().unwrap();
        }
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
    fn flush_during_an_image_import_runs_after_the_import() {
        let project = saved_project("autosave-import");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(60_000, 60_000));
        let (a, _, _) = open_window(&app, open_project(&app, project.path()));
        edit(&app, a, 700, "가져오기 중");
        app.shared.importing_image.store(true, Ordering::SeqCst);
        app.flush_autosave().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(generation(&app, a), 1, "가져오기 중 쓰기");
        assert!(!app.shared.autosave_writing.load(Ordering::SeqCst));
        // 가져오기가 끝나면 debounce(60초)를 기다리지 않고 미룬 flush로 씁니다.
        app.shared.importing_image.store(false, Ordering::SeqCst);
        assert!(wait_for(|| generation(&app, a) == 2));
        app.shutdown().unwrap();
    }

    #[test]
    fn failed_autosave_is_reported_and_retried_after_the_max_delay() {
        let project = saved_project("autosave-failed");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(40, 400));
        let (a, _, _) = open_window(&app, open_project(&app, project.path()));
        // design/objects를 파일로 막아 실제 writer가 실패하게 합니다.
        let objects = project.path().join("design").join("objects");
        let backup = project.path().join("objects-backup");
        std::fs::rename(&objects, &backup).unwrap();
        std::fs::write(&objects, b"blocked").unwrap();
        edit(&app, a, 700, "실패");
        assert!(wait_for(|| status(&app, a) == AutosaveStatus::Failed));
        assert!(
            app.state().window(a).unwrap().has_unsaved_changes,
            "편집 보존"
        );
        assert_eq!(generation(&app, a), 1);
        // 편집 없이도 실패 시각 + 최대 지연에 다시 시도해 성공하면 풀립니다.
        std::fs::remove_file(&objects).unwrap();
        std::fs::rename(&backup, &objects).unwrap();
        assert!(wait_for(|| status(&app, a) == AutosaveStatus::Idle));
        assert_eq!(generation(&app, a), 2);
        assert!(!app.state().window(a).unwrap().has_unsaved_changes);
        app.shutdown().unwrap();
    }

    #[test]
    fn stale_window_stops_autosaving_without_writing() {
        let project = saved_project("autosave-stale");
        let mut app = AppRuntime::start().unwrap();
        app.enable_autosave_with(policy(60_000, 60_000));
        let owner = open_project(&app, project.path());
        let (a, _, _) = open_window(&app, owner);
        let (b, _, _) = open_window(&app, owner);
        // 두 창 모두 generation 1 위에서 편집합니다. clean 창은 다른 창의 저장을 외부 변경으로 받아들이므로
        // (UX.4.1) B는 A가 저장하기 전에 편집합니다.
        edit(&app, a, 700, "A");
        edit(&app, b, 800, "B");
        // flush는 창 순서대로 캡처하고 writer는 받은 순서대로 쓰므로 A가 먼저 저장합니다.
        app.flush_autosave().unwrap();
        assert!(wait_for(|| generation(&app, a) == 2));
        let current = std::fs::read(project.path().join("design").join("current")).unwrap();
        // B는 generation 1 위에서 편집했으므로 자동 저장이 stale로 멈추고 디스크를 바꾸지 않습니다.
        assert!(wait_for(|| status(&app, b) == AutosaveStatus::Stale));
        edit(&app, b, 801, "B 다시");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(status(&app, b), AutosaveStatus::Stale);
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
            next_solicited(&app),
            RuntimeEvent::RequestFailed { request: got, failure: RequestFailure::SaveStale, .. }
                if got == request
        ));
        app.shutdown().unwrap();
    }

    /// document thread 점유 측정(ADR-0072 "Performance"). Release로 실행합니다:
    /// `cargo test -p stellar-runtime --release --lib app::autosave::tests::measure_document_thread_occupancy -- --ignored --nocapture`
    #[test]
    #[ignore = "Release 측정용(ADR-0072)"]
    fn measure_document_thread_occupancy() {
        use crate::window_session::test_fixtures::{identity, saved_project_with, sid, window};
        use stellar_commands::document::definitions::{
            DocumentDefinition, PageDefinition, ViewDefinition,
        };
        use stellar_commands::document::frame::FramePayload;
        use stellar_commands::document::model::DocumentModel;
        use stellar_commands::document::records::{NodePayload, NodeRecord};
        use stellar_core_types::order_key::OrderKey;

        fn model(frames: u64) -> DocumentModel {
            let mut definition = DocumentDefinition::new(sid(1), "측정".to_string());
            let mut page = PageDefinition::new(sid(2), "페이지".to_string());
            page.add_view(ViewDefinition::new(sid(3), "화면".to_string(), sid(10)))
                .unwrap();
            definition.add_page(page).unwrap();
            let mut model = DocumentModel::new(definition);
            let mut key = None;
            for n in 0..=frames {
                let order = OrderKey::between(key.as_ref(), None);
                model
                    .insert_node(NodeRecord {
                        id: sid(10 + n),
                        parent: (n > 0).then(|| sid(10)),
                        order_key: order.clone(),
                        visible: true,
                        locked: false,
                        display_name: format!("n{n}"),
                        payload: NodePayload::Frame(FramePayload::default()),
                    })
                    .unwrap();
                if n > 0 {
                    key = Some(order);
                }
            }
            model
        }

        fn ms(duration: Duration) -> f64 {
            duration.as_secs_f64() * 1_000.0
        }

        for frames in [2_000u64, 20_000] {
            let project = saved_project_with("autosave-measure", &[&model(frames)]);
            let mut session =
                ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
            println!("Frame {frames}: capture(document thread) / write(writer) / apply(document thread) ms");
            for round in 0..7u64 {
                let tx = rename(
                    session.document().unwrap(),
                    900 + round,
                    11,
                    &format!("이름 {round}"),
                );
                session.apply(&tx, 0).unwrap();
                let started = Instant::now();
                let capture = session.capture_save().unwrap().unwrap();
                let captured = started.elapsed();
                let started = Instant::now();
                let written = capture.write(default_lock_dir).unwrap();
                let wrote = started.elapsed();
                let started = Instant::now();
                assert!(session.apply_save(written).written);
                let applied = started.elapsed();
                println!(
                    "  {round}: {:.2} / {:.2} / {:.3}",
                    ms(captured),
                    ms(wrote),
                    ms(applied)
                );
            }
        }
    }
}
