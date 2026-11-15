//! 캔버스 직접 조작 C ABI(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(두 점, delta, local 상자)와 앱 설정 값만 넘기고, 새 도형의 modifier 구성과
//! 선택·편집 의미는 코어가 정합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{FiniteF32, Vec2};
use stellar_runtime::app::{
    CanvasEdit, CanvasNodeKind, CanvasPick, CanvasSelectionItem, PickPhase, RadiusHandles,
    ShapeKind, ShapePaint, WindowSessionHandle,
};

use crate::canvas::StellarCanvasColor;
use crate::{ffi_guard, registry, StellarStatus};

/// 새 도형의 칠(D7). 셸이 앱 설정(Settings ▸ 캔버스)에서 읽어 넘깁니다. 색공간 0은 없음입니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarShapeStyle {
    fill: StellarCanvasColor,
    stroke: StellarCanvasColor,
    stroke_width: f32,
}

impl StellarShapeStyle {
    fn decode(self) -> Result<ShapePaint, StellarStatus> {
        let fill = self.fill.decode()?;
        let stroke = match self.stroke.decode()? {
            Some(color) => {
                let width = FiniteF32::new(self.stroke_width)
                    .ok()
                    .filter(|w| w.get() > 0.0)
                    .ok_or(StellarStatus::InvalidArgument)?;
                Some((color, width))
            }
            None => None,
        };
        Ok(ShapePaint { fill, stroke })
    }
}

/// 부모 local 공간의 시작·끝 두 점과 셸이 넘긴 칠로 Shape를 만들고 선택까지 한 history 항목에 기록합니다.
///
/// `shape`는 ShapeKind 저장 tag(1 Rectangle, 2 Ellipse, 3 Line, 4 Polygon)입니다.
///
/// # Safety
/// `view`·`parent`는 16 byte, `style`은 `StellarShapeStyle` 하나를 읽을 수 있어야 하고 `out_request`는 null 또는
/// 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_session_insert_shape(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    shape: u8,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    style: *const StellarShapeStyle,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || parent.is_null() || style.is_null() {
            return StellarStatus::NullPointer;
        }
        let paint = match unsafe { *style }.decode() {
            Ok(paint) => paint,
            Err(status) => return status,
        };
        let (Ok(kind), Ok(start), Ok(end)) = (
            ShapeKind::from_tag(shape),
            Vec2::new(x0, y0),
            Vec2::new(x1, y1),
        ) else {
            return StellarStatus::InvalidArgument;
        };
        let view = StableId::from_bytes(unsafe { *view });
        let parent = StableId::from_bytes(unsafe { *parent });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.insert_canvas_shape(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                parent,
                kind,
                [start, end],
                paint,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 화면 중심에서 `(x, y)` point인 점을 고정하고 배율을 `factor`배 합니다(핀치, ⌘ 스크롤).
/// 문서·history는 바꾸지 않습니다. 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다.
///
/// # Safety
/// `out_request`는 null이거나 u64 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_zoom_by(
    handle: u64,
    session: u64,
    x: f64,
    y: f64,
    factor: f64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        if !(x.is_finite() && y.is_finite() && factor.is_finite() && factor > 0.0) {
            return StellarStatus::InvalidArgument;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.zoom_viewport_by(WindowSessionHandle(session), [x, y], factor),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 클릭 선택 flag: Shift(넣고 빼기).
pub const PICK_TOGGLE: u32 = 1;
/// 클릭 선택 flag: ⌘(깊은 선택).
pub const PICK_DEEP: u32 = 2;
/// 클릭 선택 flag: 더블클릭(한 단계 들어가기).
pub const PICK_ENTER: u32 = 4;
/// 클릭 선택 flag: 누름. 고를 node가 이미 선택에 있으면 끌 수 있도록 선택을 그대로 두고 변경을 미룹니다.
pub const PICK_PRESS: u32 = 8;
/// 클릭 선택 flag: 움직이지 않고 놓음. 바로 앞 누름이 미룬 변경을 적용합니다.
pub const PICK_RELEASE: u32 = 16;
/// 영역 선택 flag: Option(완전히 포함된 것만).
pub const AREA_CONTAIN: u32 = 1;
/// 영역 선택 flag: Shift(지금 선택에 더하기).
pub const AREA_ADD: u32 = 2;

/// 클릭 flag를 선택 의도로 바꿉니다. 모르는 bit나 PRESS·RELEASE를 함께 주면 `None`입니다.
fn point_pick(leaf: Option<StableId>, flags: u32) -> Option<CanvasPick> {
    if flags & !(PICK_TOGGLE | PICK_DEEP | PICK_ENTER | PICK_PRESS | PICK_RELEASE) != 0 {
        return None;
    }
    let phase = match (flags & PICK_PRESS != 0, flags & PICK_RELEASE != 0) {
        (false, false) => PickPhase::Click,
        (true, false) => PickPhase::Press,
        (false, true) => PickPhase::Release,
        (true, true) => return None,
    };
    Some(CanvasPick::Point {
        leaf,
        toggle: flags & PICK_TOGGLE != 0,
        deep: flags & PICK_DEEP != 0,
        enter: flags & PICK_ENTER != 0,
        phase,
    })
}

fn submit_pick(
    handle: u64,
    session: u64,
    view: StableId,
    pick: CanvasPick,
    out_request: *mut u64,
) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    let result = match rt.app() {
        Ok(app) => app.canvas_pick(WindowSessionHandle(session), view, pick),
        Err(status) => return status,
    };
    unsafe { rt.finish_request(result, out_request) }
}

/// 포인터 아래 가장 위 행(`leaf`, null이면 모든 Frame 밖)에서 선택을 정해 그 창 history에 기록합니다(UX 명세 5.3).
/// 무엇을 고를지(직계 자식, 넣고 빼기, 깊은 선택, 한 단계 들어가기)는 코어가 정합니다. 누를 때 `PICK_PRESS`,
/// 움직이지 않고 놓을 때 `PICK_RELEASE`를 보내면, 이미 고른 node를 눌러 여러 개를 함께 끌 수 있습니다.
///
/// # Safety
/// `view`는 16 byte, `leaf`는 null 또는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas_pick(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    leaf: *const [u8; 16],
    flags: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() {
            return StellarStatus::NullPointer;
        }
        let Some(pick) = point_pick(
            (!leaf.is_null()).then(|| StableId::from_bytes(unsafe { *leaf })),
            flags,
        ) else {
            return StellarStatus::InvalidArgument;
        };
        submit_pick(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            pick,
            out_request,
        )
    })
}

/// world 영역 `(x0, y0)`–`(x1, y1)`으로 root의 직계 자식을 고릅니다(Option 완전 포함, Shift 더하기).
///
/// # Safety
/// `view`는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_canvas_pick_area(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    flags: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() {
            return StellarStatus::NullPointer;
        }
        let (Ok(from), Ok(to)) = (Vec2::new(x0, y0), Vec2::new(x1, y1)) else {
            return StellarStatus::InvalidArgument;
        };
        if flags & !(AREA_CONTAIN | AREA_ADD) != 0 {
            return StellarStatus::InvalidArgument;
        }
        let pick = CanvasPick::Area {
            from,
            to,
            contain: flags & AREA_CONTAIN != 0,
            add: flags & AREA_ADD != 0,
        };
        submit_pick(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            pick,
            out_request,
        )
    })
}

/// 이동 flag: Option 드래그(복제본을 원본 바로 위에 만들어 옮기고 선택).
pub const MOVE_DUPLICATE: u32 = 1;
/// 이동 flag: delta가 각 node의 부모 공간 단위(화살표 키).
pub const MOVE_PARENT_SPACE: u32 = 2;

fn submit_edit(
    handle: u64,
    session: u64,
    view: StableId,
    edit: CanvasEdit,
    out_request: *mut u64,
) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    let result = match rt.app() {
        Ok(app) => app.canvas_edit(WindowSessionHandle(session), view, edit),
        Err(status) => return status,
    };
    unsafe { rt.finish_request(result, out_request) }
}

/// 선택한 node 전부를 `(dx, dy)`만큼 옮깁니다(UX 명세 5.4). `anchor`는 드래그를 시작한 행(키 이동은 null)이며,
/// 지금 선택이 그 행을 덮지 않으면 옮기지 않습니다. 기본 단위는 world이고 `STELLAR_MOVE_PARENT_SPACE`면 부모 공간,
/// `STELLAR_MOVE_DUPLICATE`면 복제본을 옮기고 고릅니다. 한 호출이 Undo 한 번입니다.
///
/// # Safety
/// `view`는 16 byte, `anchor`는 null 또는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_canvas_move(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    anchor: *const [u8; 16],
    dx: f32,
    dy: f32,
    flags: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(delta) = Vec2::new(dx, dy) else {
            return StellarStatus::InvalidArgument;
        };
        if flags & !(MOVE_DUPLICATE | MOVE_PARENT_SPACE) != 0 {
            return StellarStatus::InvalidArgument;
        }
        let edit = CanvasEdit::Move {
            anchor: (!anchor.is_null()).then(|| StableId::from_bytes(unsafe { *anchor })),
            delta,
            parent_space: flags & MOVE_PARENT_SPACE != 0,
            duplicate: flags & MOVE_DUPLICATE != 0,
        };
        submit_edit(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            edit,
            out_request,
        )
    })
}

/// 하나뿐인 선택 node의 상자를 지금 local 좌표 `(x, y, width, height)`로 바꿉니다(크기 핸들, Undo 한 번).
/// 속성 탭과 같은 `width`·`height` longhand(Line은 `x1`·`y1`·`x2`·`y2`)를 쓰고, 원점이 바뀌면 Frame은 `transform`,
/// 그 밖은 위치 Transform modifier를 바꿉니다. 크기는 0보다 커야 합니다.
///
/// # Safety
/// `view`·`node`는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_canvas_resize(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    out_request: *mut u64,
) -> StellarStatus {
    use stellar_core_types::value::{Rect, Size2};
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let (Ok(origin), Ok(size)) = (Vec2::new(x, y), Size2::new(width, height)) else {
            return StellarStatus::InvalidArgument;
        };
        if size.is_empty() {
            return StellarStatus::InvalidArgument;
        }
        let edit = CanvasEdit::Resize {
            node: StableId::from_bytes(unsafe { *node }),
            rect: Rect::new(origin, size),
        };
        submit_edit(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            edit,
            out_request,
        )
    })
}

/// 하나뿐인 선택 Line의 끝점 하나(`end` 0 시작, 1 끝)를 local `(x, y)`로 옮깁니다(Undo 한 번).
///
/// # Safety
/// `view`·`node`는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_canvas_line_end(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    end: u32,
    x: f32,
    y: f32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(point) = Vec2::new(x, y) else {
            return StellarStatus::InvalidArgument;
        };
        if end > 1 {
            return StellarStatus::InvalidArgument;
        }
        let edit = CanvasEdit::LineEnd {
            node: StableId::from_bytes(unsafe { *node }),
            end: end == 1,
            point,
        };
        submit_edit(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            edit,
            out_request,
        )
    })
}

/// 하나뿐인 선택 node의 모서리 반경을 바꿉니다(반경 핸들, Undo 한 번). `corner`는 -1이면 네 모서리, 0–3이면
/// tl·tr·br·bl 하나(Option)입니다. Frame·Rectangle은 `border-*-radius` longhand, Polygon은 반경 key 하나입니다.
///
/// # Safety
/// `view`·`node`는 16 byte를 읽을 수 있어야 하고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas_radius(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    corner: i32,
    radius: f32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(radius) = FiniteF32::new(radius) else {
            return StellarStatus::InvalidArgument;
        };
        let corner = match corner {
            -1 => None,
            0..=3 => Some(corner as usize),
            _ => return StellarStatus::InvalidArgument,
        };
        if radius.get() < 0.0 {
            return StellarStatus::InvalidArgument;
        }
        let edit = CanvasEdit::Radius {
            node: StableId::from_bytes(unsafe { *node }),
            corner,
            radius,
        };
        submit_edit(
            handle,
            session,
            StableId::from_bytes(unsafe { *view }),
            edit,
            out_request,
        )
    })
}

/// 선택 항목 flag.
pub const ITEM_MOVABLE: u32 = 1;
/// 크기 핸들 있음.
pub const ITEM_RESIZABLE: u32 = 2;
/// 반경 핸들 있음.
pub const ITEM_RADIUS: u32 = 4;
/// 반경이 모서리별 longhand(없으면 값 하나).
pub const ITEM_RADIUS_PER_CORNER: u32 = 8;
/// 가로 고정 크기(없으면 자동 폭, CSS `width: max-content`).
pub const ITEM_FIXED_WIDTH: u32 = 16;
/// 세로 고정 크기(없으면 CSS `height: auto`).
pub const ITEM_FIXED_HEIGHT: u32 = 32;

/// 표시한 선택 node 하나의 상자. kind: 1 Frame, 2 Rectangle, 3 Ellipse, 4 Line, 5 Polygon, 6 Text, 7 Image, 8 Instance.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasSelectionItem {
    id: [u8; 16],
    kind: u32,
    flags: u32,
    world: [f32; 6],
    rect: [f32; 4],
    radii: [f32; 4],
    line: [f32; 4],
}

impl From<&CanvasSelectionItem> for StellarCanvasSelectionItem {
    fn from(item: &CanvasSelectionItem) -> Self {
        let kind = match item.kind {
            CanvasNodeKind::Frame => 1,
            CanvasNodeKind::Rectangle => 2,
            CanvasNodeKind::Ellipse => 3,
            CanvasNodeKind::Line => 4,
            CanvasNodeKind::Polygon => 5,
            CanvasNodeKind::Text => 6,
            CanvasNodeKind::Image => 7,
            CanvasNodeKind::Instance => 8,
        };
        let bit = |on: bool, flag: u32| if on { flag } else { 0 };
        let flags = bit(item.movable, ITEM_MOVABLE)
            | bit(item.resizable, ITEM_RESIZABLE)
            | bit(item.radius.is_some(), ITEM_RADIUS)
            | bit(
                item.radius == Some(RadiusHandles::PerCorner),
                ITEM_RADIUS_PER_CORNER,
            )
            | bit(item.fixed[0], ITEM_FIXED_WIDTH)
            | bit(item.fixed[1], ITEM_FIXED_HEIGHT);
        let (a, b, c, d, x, y) = item.world.components();
        let origin = item.rect.origin();
        let size = item.rect.size();
        let (tl, tr, br, bl) = item.radii.components();
        let line = item
            .line
            .map_or([0.0; 4], |[s, e]| [s.x(), s.y(), e.x(), e.y()]);
        Self {
            id: item.id.to_bytes(),
            kind,
            flags,
            world: [a, b, c, d, x, y],
            rect: [origin.x(), origin.y(), size.width(), size.height()],
            radii: [tl, tr, br, bl],
            line,
        }
    }
}

/// 최신 게시 snapshot의 선택 상자를 씁니다. `items`가 null이면 수만 씁니다(`out_count`).
/// 용량이 모자라면 `STELLAR_BUFFER_TOO_SMALL`과 필요한 수를 씁니다. `out_revision`은 항목을 계산한 문서 revision입니다.
///
/// # Safety
/// `out_count`·`out_revision`은 쓰기 가능하고, `items`는 null이거나 `capacity`개를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas_selection(
    handle: u64,
    session: u64,
    items: *mut StellarCanvasSelectionItem,
    capacity: usize,
    out_count: *mut usize,
    out_revision: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_count.is_null() || out_revision.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            out_count.write(0);
            out_revision.write(0);
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
        let selection = &window.canvas_selection;
        unsafe {
            out_count.write(selection.len());
            out_revision.write(window.revision.0);
        }
        if items.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < selection.len() {
            return StellarStatus::BufferTooSmall;
        }
        for (index, item) in selection.iter().enumerate() {
            unsafe { items.add(index).write(item.into()) };
        }
        StellarStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};

    #[test]
    fn shape_style_layout_is_fixed() {
        assert_eq!(std::mem::size_of::<StellarShapeStyle>(), 44);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke), 20);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke_width), 40);
    }

    #[test]
    fn pick_flags_map_to_phases_and_reject_unknown_or_conflicting_bits() {
        let leaf = Some(StableId::from_bytes([7; 16]));
        let phase = |flags| match point_pick(leaf, flags) {
            Some(CanvasPick::Point { phase, .. }) => Some(phase),
            _ => None,
        };
        assert_eq!(phase(0), Some(PickPhase::Click));
        assert_eq!(phase(PICK_PRESS | PICK_TOGGLE), Some(PickPhase::Press));
        assert_eq!(phase(PICK_RELEASE | PICK_DEEP), Some(PickPhase::Release));
        assert_eq!(phase(PICK_PRESS | PICK_RELEASE), None);
        assert_eq!(phase(32), None);
        assert!(matches!(
            point_pick(leaf, PICK_TOGGLE | PICK_ENTER),
            Some(CanvasPick::Point {
                toggle: true,
                deep: false,
                enter: true,
                ..
            })
        ));
    }

    #[test]
    fn canvas_calls_check_pointers_and_arguments_before_reaching_a_runtime() {
        let view = [1u8; 16];
        let mut request = 9;
        unsafe {
            assert_eq!(
                stellar_window_canvas_pick(
                    0,
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    &mut request
                ),
                StellarStatus::NullPointer
            );
            assert_eq!(request, 0);
            assert_eq!(
                stellar_window_canvas_pick(
                    0,
                    0,
                    &view,
                    std::ptr::null(),
                    PICK_PRESS | PICK_RELEASE,
                    &mut request
                ),
                StellarStatus::InvalidArgument
            );
            assert_eq!(
                stellar_window_canvas_move(
                    0,
                    0,
                    &view,
                    std::ptr::null(),
                    f32::NAN,
                    0.,
                    0,
                    &mut request
                ),
                StellarStatus::InvalidArgument
            );
            assert_eq!(
                stellar_window_canvas_resize(0, 0, &view, &view, 0., 0., 0., 4., &mut request),
                StellarStatus::InvalidArgument
            );
            assert_eq!(
                stellar_window_canvas_radius(0, 0, &view, &view, 4, 1., &mut request),
                StellarStatus::InvalidArgument
            );
            assert_eq!(
                stellar_window_canvas_line_end(0, 0, &view, &view, 2, 0., 0., &mut request),
                StellarStatus::InvalidArgument
            );
            // 유효한 인자는 runtime handle 검사까지 갑니다.
            assert_eq!(
                stellar_window_canvas_pick(0, 0, &view, std::ptr::null(), PICK_PRESS, &mut request),
                StellarStatus::InvalidHandle
            );
            let (mut count, mut revision) = (1, 1);
            assert_eq!(
                stellar_window_canvas_selection(
                    0,
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut count,
                    &mut revision
                ),
                StellarStatus::InvalidHandle
            );
            assert_eq!((count, revision), (0, 0));
        }
    }

    #[test]
    fn shape_style_keeps_absent_paint_and_rejects_invalid_stroke() {
        let gray = Color::new(ColorSpace::Srgb, 0.85, 0.85, 0.85, 1.0).unwrap();
        let style = StellarShapeStyle {
            fill: Some(gray).into(),
            stroke: StellarCanvasColor::default(),
            stroke_width: 0.0,
        };
        assert_eq!(
            style.decode().unwrap(),
            ShapePaint {
                fill: Some(gray),
                stroke: None
            }
        );
        for width in [0.0, -1.0, f32::NAN] {
            let line = StellarShapeStyle {
                fill: StellarCanvasColor::default(),
                stroke: Some(gray).into(),
                stroke_width: width,
            };
            assert_eq!(line.decode(), Err(StellarStatus::InvalidArgument));
        }
    }
}
