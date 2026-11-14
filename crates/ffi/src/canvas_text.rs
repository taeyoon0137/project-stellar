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
fn with_text(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    run: impl FnOnce(&DocumentText) -> StellarStatus,
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
    scene
        .text
        .get(&q.row)
        .map(AsRef::as_ref)
        .map_or(StellarStatus::NoError, run)
}
fn rect(r: Rect) -> [f32; 4] {
    [
        r.origin().x(),
        r.origin().y(),
        r.size().width(),
        r.size().height(),
    ]
}

/// # Safety
/// count는 유효한 출력, rows는 null(크기 조회) 또는 capacity 원소 출력입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_text_paint(
    handle: u64,
    session: u64,
    q: StellarCanvasTextQuery,
    rows: *mut StellarCanvasTextElement,
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
        with_text(handle, session, q, |text| unsafe {
            write_text_paint(text, rows, capacity, count)
        })
    })
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
        .try_fold(text.lines.len(), |n, (_, path)| {
            n.checked_add(path.commands().len())?.checked_add(1)
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
    for (origin, path) in &text.glyphs {
        let at = |p: Vec2| [origin.x() + p.x(), origin.y() + p.y()];
        for cmd in path.commands() {
            match *cmd {
                CanvasPathCommand::MoveTo(p) => {
                    let [x, y] = at(p);
                    put(1, [x, y, 0., 0., 0., 0.]);
                }
                CanvasPathCommand::LineTo(p) => {
                    let [x, y] = at(p);
                    put(2, [x, y, 0., 0., 0., 0.]);
                }
                CanvasPathCommand::CubicTo { c1, c2, to } => {
                    let [a, b] = at(c1);
                    let [c, d] = at(c2);
                    let [x, y] = at(to);
                    put(3, [a, b, c, d, x, y]);
                }
                CanvasPathCommand::Close => put(4, [0.; 6]),
            }
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

/// # Safety
/// path는 path_len개의 UTF-8 byte입니다. out_request는 null 또는 유효한 u64 출력입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_set_document_font(
    handle: u64,
    session: u64,
    revision: u64,
    path: *const u8,
    path_len: usize,
    face: u32,
    size_subpixels: u32,
    color: crate::canvas::StellarCanvasColor,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe {
                *out_request = 0;
            }
        }
        if path_len == 0
            || path_len > 32768
            || size_subpixels == 0
            || size_subpixels > i32::MAX as u32
        {
            return StellarStatus::InvalidArgument;
        }
        let file = match unsafe { crate::decode_utf8_owned(path, path_len) } {
            Ok(s) if !s.contains('\0') => std::path::PathBuf::from(s),
            Ok(_) => return StellarStatus::InvalidArgument,
            Err(e) => return e,
        };
        if !file.is_absolute() {
            return StellarStatus::InvalidArgument;
        }
        let Ok(Some(color)) = color.decode() else {
            return StellarStatus::InvalidArgument;
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.set_document_font(
                WindowSessionHandle(session),
                stellar_core_types::revision::DocumentRevision(revision),
                file,
                face,
                size_subpixels,
                color,
            ),
            Err(e) => return e,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}
