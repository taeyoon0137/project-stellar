//! 창 캔버스 C ABI(ADR-0028): 저장된 Free Frame scene과 같은 게시 snapshot의 카메라를 macOS 창으로 복사하고,
//! 창 하나를 대상으로 셸 입력을 받아 코어가 해석합니다.

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::Transform2D;
use stellar_runtime::app::WindowSessionHandle;
use stellar_runtime::canvas::CanvasFrame;

use crate::input::{InputEvent, InputEventKind, Modifiers, WheelDeltaUnit};
use crate::input_macos::StellarMacEvent;
use crate::{convert_received, ffi_guard, queue_input, registry, InstantClock, StellarStatus};

/// 일반 wheel 한 줄(notch)을 화면 point로 환산하는 값입니다. precise scrolling(trackpad)은 point 그대로 씁니다.
pub const WHEEL_LINE_POINTS: f64 = 10.0;

/// 수식키 없는(또는 Shift만 누른) wheel·trackpad 스크롤이면 창 팬 양(화면 point, 내용 이동 방향)을 돌려줍니다.
///
/// 공통 wheel delta는 scroll 양(x 오른쪽·y 아래쪽 양수, AppKit 시스템 스크롤 방향 적용 뒤)이고 팬은 내용
/// 이동이라 부호가 반대입니다. Shift+휠은 시스템이 가로 delta로 바꿔 주므로 그대로 가로 팬입니다(UX 명세 5.1,
/// ADR-0071 UX.3.1). Control·Option·Command가 눌린 스크롤은 팬이 아닙니다(⌘ 스크롤 줌은 셸이 `zoom_by`로 보냄,
/// Caps Lock은 상태라 보지 않습니다).
pub(crate) fn wheel_pan_points(input: &InputEvent) -> Option<(f64, f64)> {
    let InputEventKind::Wheel(wheel) = input.kind() else {
        return None;
    };
    let held = [Modifiers::CONTROL, Modifiers::ALT, Modifiers::META];
    if held
        .iter()
        .any(|modifier| wheel.modifiers.contains(*modifier))
    {
        return None;
    }
    let scale = match wheel.unit {
        WheelDeltaUnit::Point => 1.0,
        WheelDeltaUnit::Line => WHEEL_LINE_POINTS,
    };
    Some((
        -f64::from(wheel.delta.x()) * scale,
        -f64::from(wheel.delta.y()) * scale,
    ))
}

/// 창 session 하나를 대상으로 macOS 입력 이벤트를 제출합니다. 해석은 코어가 합니다.
///
/// - 변환·시간 domain 규칙은 `stellar_runtime_submit_mac_event`와 같습니다.
/// - 수식키 없는 wheel·trackpad 스크롤은 그 창의 팬입니다([`wheel_pan_points`]). 새 팬 요청을 접수하면
///   `out_request`에 request id를 쓰고 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다. 아직 반영되지
///   않은 팬에 더했거나 이동량이 0이면 `out_request`는 0입니다. 접수가 거부되면(`STELLAR_QUEUE_FULL`) 이동량은
///   버리지 않고 다음 스크롤과 함께 제출합니다.
/// - 그 밖의 이벤트는 runtime 입력 대기열에 넣고 `out_request`는 0입니다.
///
/// # Safety
/// `event`는 `StellarMacEvent` 읽기가 유효한 포인터, `out_request`는 null이거나 `u64` 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_submit_mac_event(
    handle: u64,
    session: u64,
    event: *const StellarMacEvent,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let received = InstantClock.now_nanos();
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        if event.is_null() {
            return StellarStatus::NullPointer;
        }
        let raw = unsafe { *event };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let input = match convert_received(rt, &raw) {
            Ok(Some(input)) => input,
            Ok(None) => return StellarStatus::Ok,
            Err(status) => return status,
        };
        let Some((dx, dy)) = wheel_pan_points(&input) else {
            return queue_input(rt, input, received);
        };
        if dx == 0.0 && dy == 0.0 {
            return StellarStatus::Ok;
        }
        let result = match rt.app() {
            Ok(app) => app.wheel_pan(WindowSessionHandle(session), dx, dy),
            Err(status) => return status,
        };
        match result {
            Ok(Some(request)) => unsafe { rt.finish_request(Ok(request), out_request) },
            Ok(None) => StellarStatus::Ok,
            Err(error) => unsafe { rt.finish_request(Err(error), out_request) },
        }
    })
}

/// C ABI의 선택적 비연관 RGBA. 색공간 tag 0은 색 없음입니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasColor {
    space: u32,
    rgba: [f32; 4],
}

impl From<Option<Color>> for StellarCanvasColor {
    fn from(color: Option<Color>) -> Self {
        color.map_or_else(Self::default, |c| Self {
            space: u32::from(c.space().tag()),
            rgba: [c.r(), c.g(), c.b(), c.alpha()],
        })
    }
}

impl StellarCanvasColor {
    pub(crate) fn decode(self) -> Result<Option<Color>, StellarStatus> {
        if self.space == 0 {
            return Ok(None);
        }
        let space = u8::try_from(self.space)
            .ok()
            .and_then(ColorSpace::from_tag)
            .ok_or(StellarStatus::InvalidArgument)?;
        let [r, g, b, a] = self.rgba;
        Color::new(space, r, g, b, a)
            .map(Some)
            .map_err(|_| StellarStatus::InvalidArgument)
    }
}

/// 한 Frame의 그리기 정보. C 레이아웃은 stellar_ffi.h와 공유합니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasFrame {
    id: [u8; 16],
    parent: u32,
    flags: u32,
    world: [f32; 6],
    size: [f32; 2],
    radii: [f32; 4],
    fill: StellarCanvasColor,
    border: StellarCanvasColor,
    border_width: f32,
    fit: u8,
}

fn affine(t: Transform2D) -> [f32; 6] {
    let (a, b, c, d, x, y) = t.components();
    [a, b, c, d, x, y]
}

impl From<&CanvasFrame> for StellarCanvasFrame {
    fn from(frame: &CanvasFrame) -> Self {
        let size = frame.geometry.rect.size();
        let (tl, tr, br, bl) = frame.geometry.radii.components();
        Self {
            id: frame.id.to_bytes(),
            parent: frame.parent.unwrap_or(u32::MAX),
            flags: u32::from(frame.clip_children)
                | (u32::from(frame.hit_target) << 1)
                | (u32::from(frame.rectangle_shape) << 2)
                | (u32::from(frame.instance_frame) << 3)
                | (u32::from(frame.text_node) << 4)
                | (u32::from(frame.image_asset.is_some()) << 5)
                | (u32::from(frame.path_shape) << 6),
            world: affine(frame.world),
            size: [size.width(), size.height()],
            radii: [tl, tr, br, bl],
            fill: frame.background.into(),
            border: frame.border.map(|b| b.color).into(),
            border_width: frame.border.map_or(0.0, |b| b.thickness()),
            fit: frame.image_fit as u8,
        }
    }
}

/// 한 조회에서 사용한 View, revision, 카메라 및 목록 길이.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasInfo {
    revision: u64,
    view: [u8; 16],
    primary: [u8; 16],
    camera: [f32; 6],
    count: u32,
    skipped: u32,
}

/// 현재 창의 active view에 대해 같은 snapshot의 metadata와 선택적 painter 목록을 돌려줍니다.
///
/// # Safety
/// `info`는 쓰기 가능한 포인터이고 `rows`는 null이거나 `capacity`개 항목의 정렬된 쓰기 버퍼여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas(
    handle: u64,
    session: u64,
    width: f32,
    height: f32,
    info: *mut StellarCanvasInfo,
    rows: *mut StellarCanvasFrame,
    capacity: usize,
) -> StellarStatus {
    ffi_guard(move || {
        if info.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { info.write(StellarCanvasInfo::default()) };
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return StellarStatus::InvalidArgument;
        }
        let state = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(runtime) = reg.live.get(&handle) else {
                return StellarStatus::InvalidHandle;
            };
            let Some(app) = &runtime.app else {
                return StellarStatus::InvalidArgument;
            };
            app.state()
        };
        let Some(window) = state.window(WindowSessionHandle(session)) else {
            return StellarStatus::InvalidArgument;
        };
        let (Some(scene), Some(view), Some(viewport)) =
            (&window.canvas, window.active_view, window.viewport)
        else {
            return StellarStatus::NoError;
        };
        let Some(camera) = viewport.world_to_view(width, height) else {
            return StellarStatus::InvalidArgument;
        };
        let Ok(count) = u32::try_from(scene.frames.len()) else {
            return StellarStatus::Internal;
        };
        unsafe {
            info.write(StellarCanvasInfo {
                revision: window.revision.0,
                view: view.to_bytes(),
                primary: window.primary_selection.map_or([0; 16], |id| id.to_bytes()),
                camera: affine(camera),
                count,
                skipped: scene.skipped,
            })
        };
        if rows.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < scene.frames.len() {
            return StellarStatus::BufferTooSmall;
        }
        for (index, frame) in scene.frames.iter().enumerate() {
            unsafe {
                rows.add(index).write(frame.into());
            }
        }
        StellarStatus::Ok
    })
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn background_preserves_none_and_wide_gamut_and_rejects_invalid_values() {
        let p3 = Color::new(ColorSpace::DisplayP3, 1.25, -0.1, 0.5, 0.375).unwrap();
        assert_eq!(
            StellarCanvasColor::from(Some(p3)).decode().unwrap(),
            Some(p3)
        );
        assert_eq!(StellarCanvasColor::default().decode().unwrap(), None);
        for color in [
            StellarCanvasColor {
                space: 257,
                rgba: [0.0; 4],
            },
            StellarCanvasColor {
                space: 1,
                rgba: [0.0, 0.0, 0.0, 2.0],
            },
            StellarCanvasColor {
                space: 2,
                rgba: [f32::NAN, 0.0, 0.0, 1.0],
            },
        ] {
            assert_eq!(color.decode(), Err(StellarStatus::InvalidArgument));
        }
    }
}

#[cfg(test)]
mod window_input_tests {
    use super::*;
    use crate::input_macos::{event_type, modifier_flag, MAC_EVENT_VERSION};
    use crate::window_session::tests::{
        create, next_event, open_project, open_window, FixtureProject,
    };
    use crate::window_session::{stellar_window_view_info, StellarWindowViewInfo};
    use crate::{stellar_runtime_destroy, stellar_runtime_poll_input};
    use stellar_runtime::app::RuntimeEvent;

    fn scroll(delta: (f64, f64), precise: bool, flags: u64) -> StellarMacEvent {
        StellarMacEvent {
            version: MAC_EVENT_VERSION,
            event_type: event_type::SCROLL_WHEEL,
            has_precise_deltas: u8::from(precise),
            modifier_flags: flags,
            timestamp: 10.0,
            receive_uptime: 10.0,
            x: 40.0,
            y: 30.0,
            scrolling_delta_x: delta.0,
            scrolling_delta_y: delta.1,
            ..StellarMacEvent::default()
        }
    }

    fn center(h: u64, session: u64) -> (f64, f64) {
        let mut info = StellarWindowViewInfo::default();
        assert_eq!(
            unsafe { stellar_window_view_info(h, session, &mut info) },
            StellarStatus::Ok
        );
        (info.center_x, info.center_y)
    }

    fn submit(h: u64, session: u64, event: StellarMacEvent) -> u64 {
        let mut request = u64::MAX;
        assert_eq!(
            unsafe { stellar_window_session_submit_mac_event(h, session, &event, &mut request) },
            StellarStatus::Ok
        );
        request
    }

    /// 접수한 팬 요청이 모두 끝날 때까지 완료 event를 읽습니다.
    fn finish(h: u64, session: u64, requests: impl IntoIterator<Item = u64>) {
        for request in requests.into_iter().filter(|&request| request != 0) {
            assert!(matches!(
                next_event(h),
                RuntimeEvent::WindowViewChanged { request: got, session: owner }
                    if got.0 == request && owner.0 == session
            ));
        }
    }

    #[test]
    fn core_interprets_scroll_as_a_pan_of_only_the_target_window() {
        let fixture = FixtureProject::copy("wheel");
        let h = create();
        let project = open_project(h, &fixture.path);
        let a = open_window(h, project);
        let b = open_window(h, project);
        let other = center(h, b);
        // 정밀 스크롤: AppKit delta 방향으로 내용이 움직입니다(중심은 반대).
        let precise = submit(h, a, scroll((12.0, -8.0), true, 0));
        assert_ne!(precise, 0);
        finish(h, a, [precise]);
        assert_eq!(center(h, a), (-12.0, 8.0));
        // 일반 휠 한 줄은 코어 상수만큼의 point입니다.
        let line = submit(h, a, scroll((0.0, 3.0), false, 0));
        finish(h, a, [line]);
        assert_eq!(center(h, a), (-12.0, 8.0 - 3.0 * WHEEL_LINE_POINTS));
        // 연속 스크롤은 합쳐져도 이동량을 잃지 않습니다.
        let before = center(h, a);
        let requests: Vec<u64> = (0..32)
            .map(|_| submit(h, a, scroll((2.0, -1.0), true, 0)))
            .collect();
        finish(h, a, requests);
        assert_eq!(center(h, a), (before.0 - 64.0, before.1 + 32.0));
        // Shift 스크롤은 받은 delta 그대로 팬입니다(시스템이 휠을 가로 delta로 바꿈).
        let before_shift = center(h, a);
        let shifted = submit(h, a, scroll((6.0, 0.0), true, modifier_flag::SHIFT));
        assert_ne!(shifted, 0);
        finish(h, a, [shifted]);
        assert_eq!(center(h, a), (before_shift.0 - 6.0, before_shift.1));
        // 그 밖의 수식키 스크롤은 팬이 아니고 runtime 입력 대기열로 갑니다.
        let after = center(h, a);
        for flag in [
            modifier_flag::CONTROL,
            modifier_flag::OPTION,
            modifier_flag::COMMAND,
        ] {
            assert_eq!(submit(h, a, scroll((12.0, -8.0), true, flag)), 0);
            let mut written = 0usize;
            let mut buf = [0u8; 256];
            assert_eq!(
                unsafe { stellar_runtime_poll_input(h, buf.as_mut_ptr(), buf.len(), &mut written) },
                StellarStatus::Ok
            );
            assert!(written > 0);
        }
        assert_eq!(center(h, a), after);
        assert_eq!(center(h, b), other, "다른 창 viewport 그대로");
        let mut request = 0;
        assert_eq!(
            unsafe {
                stellar_window_session_submit_mac_event(h, a, std::ptr::null(), &mut request)
            },
            StellarStatus::NullPointer
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn header_declares_the_window_input_entry_point() {
        let header = include_str!("../include/stellar_ffi.h");
        assert!(header.contains(
            "StellarStatus stellar_window_session_submit_mac_event(uint64_t handle, uint64_t session, const StellarMacEvent *event, uint64_t *out_request);"
        ));
    }
}
