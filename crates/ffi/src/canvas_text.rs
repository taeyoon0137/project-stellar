//! Revision과 row를 함께 확인하는 공통 text layout 조회. 폰트 파싱/shape은 여기서 실행하지 않습니다.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_core_types::value::{Rect, Vec2};
use stellar_runtime::{
    app::WindowSessionHandle,
    canvas::text::{CanvasPathCommand, DocumentText},
};

/// 현재 표시한 scene에서 특정 Text 행을 가리키는 복사 가능한 조회 값.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StellarCanvasTextQuery {
    view: [u8; 16],
    node: [u8; 16],
    revision: u64,
    row: u32,
}
/// 렌더링용 pen/path 요소. op 1/2/3/4는 각각 move/line/cubic/close, 5는 줄 hit rect, 6은 1 glyph paint 종료.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StellarCanvasTextElement {
    op: u32,
    points: [f32; 6],
}
fn with_scene_row(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    run: impl FnOnce(&stellar_runtime::canvas::CanvasScene) -> StellarStatus,
) -> StellarStatus {
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
    let Some(win) = state.window(WindowSessionHandle(session)) else {
        return StellarStatus::InvalidArgument;
    };
    let Some(scene) = &win.canvas else {
        return StellarStatus::NoError;
    };
    if win.revision.0 != q.revision
        || win.active_view.map(|v| v.to_bytes()) != Some(q.view)
        || scene.frames.get(q.row as usize).map(|r| r.id.to_bytes()) != Some(q.node)
    {
        return StellarStatus::NoError;
    }
    run(scene)
}

fn with_text(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    run: impl FnOnce(&DocumentText) -> StellarStatus,
) -> StellarStatus {
    with_scene_row(handle, session, q, |scene| {
        scene
            .text
            .get(&q.row)
            .map(AsRef::as_ref)
            .map_or(StellarStatus::NoError, run)
    })
}

/// 불변 scene의 shape path를 같은 revision·View·node의 행에서 복사합니다.
/// # Safety
/// count와 flags는 쓰기 가능해야 하며 rows는 null 또는 capacity개의 정렬된 요소입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_canvas_shape_path(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    rows: *mut StellarCanvasTextElement,
    capacity: usize,
    count: *mut usize,
    flags: *mut u32,
) -> StellarStatus {
    ffi_guard(move || {
        if count.is_null() || flags.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            count.write(0);
            flags.write(0);
        }
        with_scene_row(handle, session, q, |scene| {
            let Some(shape) = scene.shapes.get(&q.row) else {
                return StellarStatus::NoError;
            };
            let cmds = shape.geometry.commands();
            let length = cmds.len() + usize::from(shape.point_cap.is_some());
            unsafe {
                count.write(length);
                flags.write(u32::from(shape.cap.tag()));
            }
            if rows.is_null() {
                return StellarStatus::Ok;
            }
            if capacity < length {
                return StellarStatus::BufferTooSmall;
            }
            for (i, &command) in cmds.iter().enumerate() {
                let (op, points) = path_command(command, Vec2::ZERO);
                unsafe {
                    rows.add(i).write(StellarCanvasTextElement { op, points });
                }
            }
            if let Some(bounds) = shape.point_cap {
                let [x, y, w, h] = rect(bounds);
                unsafe {
                    rows.add(cmds.len()).write(StellarCanvasTextElement {
                        op: 5,
                        points: [x, y, w, h, 0., 0.],
                    });
                }
            }
            StellarStatus::Ok
        })
    })
}
fn rect(r: Rect) -> [f32; 4] {
    [
        r.origin().x(),
        r.origin().y(),
        r.size().width(),
        r.size().height(),
    ]
}

fn path_command(cmd: CanvasPathCommand, origin: Vec2) -> (u32, [f32; 6]) {
    let at = |p: Vec2| [origin.x() + p.x(), origin.y() + p.y()];
    match cmd {
        CanvasPathCommand::MoveTo(p) => {
            let [x, y] = at(p);
            (1, [x, y, 0., 0., 0., 0.])
        }
        CanvasPathCommand::LineTo(p) => {
            let [x, y] = at(p);
            (2, [x, y, 0., 0., 0., 0.])
        }
        CanvasPathCommand::CubicTo { c1, c2, to } => {
            let [a, b] = at(c1);
            let [c, d] = at(c2);
            let [x, y] = at(to);
            (3, [a, b, c, d, x, y])
        }
        CanvasPathCommand::Close => (4, [0.; 6]),
    }
}

pub(super) unsafe fn write_text_paint(
    text: &DocumentText,
    rows: *mut StellarCanvasTextElement,
    capacity: usize,
    count: *mut usize,
) -> StellarStatus {
    let len = text
        .glyphs
        .iter()
        .try_fold(text.lines.len(), |n, (_, index)| {
            n.checked_add(text.outlines[*index as usize].commands().len())?
                .checked_add(1)
        });
    let Some(len) = len else {
        return StellarStatus::Internal;
    };
    unsafe {
        *count = len;
    }
    if rows.is_null() {
        return StellarStatus::Ok;
    }
    if capacity < len {
        return StellarStatus::BufferTooSmall;
    }
    let mut i = 0;
    let mut put = |op, points| {
        unsafe { rows.add(i).write(StellarCanvasTextElement { op, points }) };
        i += 1;
    };
    for r in &text.lines {
        let [x, y, w, h] = rect(*r);
        put(5, [x, y, w, h, 0., 0.]);
    }
    for (origin, index) in &text.glyphs {
        for cmd in text.outlines[*index as usize].commands() {
            let (op, points) = path_command(*cmd, *origin);
            put(op, points);
        }
        put(6, [0.; 6]);
    }
    StellarStatus::Ok
}

/// # Safety
/// offset/upstream은 각각 유효한 출력입니다. offset은 문서 전체 Unicode scalar, upstream은 0/1입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_text_hit(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    x: f32,
    y: f32,
    offset: *mut u32,
    upstream: *mut u32,
) -> StellarStatus {
    ffi_guard(move || {
        if offset.is_null() || upstream.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *offset = 0;
            *upstream = 0;
        }
        with_text(handle, session, q, |text| match text.hit(x, y) {
            Some((scalar, affinity)) => {
                unsafe {
                    *offset = scalar;
                    *upstream = u32::from(affinity);
                }
                StellarStatus::Ok
            }
            None => StellarStatus::InvalidArgument,
        })
    })
}

/// # Safety
/// rects는 null(count 조회) 또는 capacity개의 xywh 출력입니다. start=end이면 caret, 나머지는 선택 영역.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_text_rects(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    start: u32,
    end: u32,
    upstream: u32,
    rects: *mut [f32; 4],
    capacity: usize,
    count: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if count.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *count = 0;
        }
        if upstream > 1 {
            return StellarStatus::InvalidArgument;
        }
        with_text(handle, session, q, |text| {
            let result = if start == end {
                text.caret_rect(start, upstream != 0).map(|r| vec![r])
            } else {
                text.selection_rects(start, end)
            };
            let Some(result) = result else {
                return StellarStatus::InvalidArgument;
            };
            unsafe {
                *count = result.len();
            }
            if !rects.is_null() {
                if capacity < result.len() {
                    return StellarStatus::BufferTooSmall;
                }
                for (i, r) in result.into_iter().enumerate() {
                    unsafe { rects.add(i).write(rect(r)) };
                }
            }
            StellarStatus::Ok
        })
    })
}

/// 저장 장면의 중복 없는 outline 정의 또는 그 outline의 배치입니다.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StellarCanvasTextPart {
    op: u32,
    index: u32,
    points: [f32; 6],
}

/// # Safety
/// count/token은 유효한 출력, rows는 null(크기 조회) 또는 capacity 원소의 buffer입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_text_parts(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    rows: *mut StellarCanvasTextPart,
    capacity: usize,
    count: *mut usize,
    paint_token: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if count.is_null() || paint_token.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *count = 0;
            *paint_token = 0;
        }
        with_text(handle, session, q, |text| {
            let len = text
                .outlines
                .iter()
                .try_fold(text.lines.len(), |n, p| {
                    n.checked_add(p.commands().len())?.checked_add(1)
                })
                .and_then(|n| n.checked_add(text.glyphs.len()));
            let Some(len) = len else {
                return StellarStatus::Internal;
            };
            unsafe {
                *count = len;
                *paint_token = text.paint_token;
            }
            if rows.is_null() {
                return StellarStatus::Ok;
            }
            if capacity < len {
                return StellarStatus::BufferTooSmall;
            }
            let mut i = 0;
            let mut put = |op, index, points| {
                unsafe {
                    rows.add(i)
                        .write(StellarCanvasTextPart { op, index, points })
                };
                i += 1;
            };
            for r in &text.lines {
                let [x, y, w, h] = rect(*r);
                put(5, 0, [x, y, w, h, 0., 0.]);
            }
            for p in &text.outlines {
                for &cmd in p.commands() {
                    let (op, points) = path_command(cmd, Vec2::ZERO);
                    put(op, 0, points);
                }
                put(6, 0, [0.; 6]);
            }
            for (origin, index) in &text.glyphs {
                put(7, *index, [origin.x(), origin.y(), 0., 0., 0., 0.]);
            }
            StellarStatus::Ok
        })
    })
}
