//! 속성 패널 C ABI (UX.2.2, ADR-0070).
//!
//! 셸은 창의 확정 선택에 대한 CSS 속성 행을 [`stellar_window_inspector`]로 읽고(게시된 상태 snapshot, document
//! thread를 기다리지 않음), 편집은 [`stellar_window_edit_property`] 하나로 보냅니다. 편집은 속성 이름(CSS
//! longhand 이름)과 CSS 값 텍스트 또는 수치 delta를 넘기며, 값 해석·정규화·저장 위치는 코어가 정합니다.
//! 값 문법 오류는 문서를 건드리지 않고 이 호출에서 바로 진단 텍스트로 돌려줍니다. 속성마다 FFI를 따로 두지
//! 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_runtime::app::inspector::{
    parse_input, InspectorInput, InspectorProperty, PropertyEditPhase,
};
use stellar_runtime::app::{AppRuntime, SelectionEditTarget, WindowSessionHandle};

use crate::{ffi_guard, registry, StellarStatus};

/// `StellarInspectorInfo` 레이아웃 버전.
pub const INSPECTOR_INFO_VERSION: u32 = 1;

/// 속성 패널 snapshot 머리(고정 56 byte).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct StellarInspectorInfo {
    /// [`INSPECTOR_INFO_VERSION`].
    pub version: u32,
    /// 문서에 있는 확정 선택 node 수(0이면 행 없음).
    pub count: u32,
    /// 선택에 있는 node 분류 bit(`1 << class`, class: 0 Frame, 1 Rectangle, 2 Ellipse, 3 Line, 4 Polygon,
    /// 5 Text, 6 Image, 7 그 밖).
    pub classes: u32,
    /// 행 수.
    pub row_count: u32,
    /// 게시 시점 문서 revision.
    pub revision: u64,
    /// 게시 시점 Undo 스택 끝 sequence.
    pub history_sequence: u64,
    /// active View(없으면 0).
    pub view: [u8; 16],
    /// 텍스트 첫 부분의 이름 byte 수: 단일 선택이면 레이어 이름, 선택이 없으면 현재 Page 요약(`Page · View`),
    /// 다중 선택이면 0.
    pub name_len: u32,
    /// 전체 텍스트 byte 수.
    pub text_len: u32,
}

/// 속성 행 하나(고정 24 byte). 텍스트는 이름, 값, 선택지(공백 구분) 순서로 이어집니다.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct StellarInspectorRow {
    /// 저장 위치 PropertyKey(묶음 행은 0).
    pub key: u32,
    /// 행 flag(`STELLAR_INSPECTOR_*`).
    pub flags: u32,
    /// 섹션(1 Layer, 2 Position, 3 Size, 4 Overflow, 5 Typography, 6 Background, 7 Fill·Stroke, 8 Border,
    /// 9 종류별).
    pub section: u32,
    /// 이름 byte 수.
    pub name_len: u32,
    /// 값 byte 수(혼합이면 0).
    pub value_len: u32,
    /// 선택지 byte 수.
    pub choices_len: u32,
}

/// 확정 선택의 속성 행을 읽습니다. `info`는 늘 씁니다. `rows`·`text` 용량이 모자라면 `BufferTooSmall`을
/// 돌려주고 `info`의 `row_count`·`text_len`으로 필요한 크기를 알립니다. 창이나 선택이 없으면 `count`가 0입니다.
///
/// # Safety
/// `info`는 쓰기 가능해야 합니다. `rows`는 null 또는 `row_capacity`개, `text`는 null 또는 `text_capacity`
/// byte를 쓸 수 있어야 하며 출력끼리 겹치지 않아야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_inspector(
    handle: u64,
    session: u64,
    info: *mut StellarInspectorInfo,
    rows: *mut StellarInspectorRow,
    row_capacity: usize,
    text: *mut u8,
    text_capacity: usize,
) -> StellarStatus {
    ffi_guard(move || {
        if info.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            info.write(StellarInspectorInfo {
                version: INSPECTOR_INFO_VERSION,
                ..Default::default()
            })
        };
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let state = rt.app.as_ref().map(AppRuntime::state);
        let Some(window) = state
            .as_ref()
            .and_then(|s| s.window(WindowSessionHandle(session)))
        else {
            return StellarStatus::Ok;
        };
        let snapshot = &window.inspector;
        let mut bytes = snapshot.display_name.as_bytes().to_vec();
        let mut out_rows = Vec::with_capacity(snapshot.rows.len());
        for row in &snapshot.rows {
            let name = row.property.name();
            let choices = row.property.choices().join(" ");
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(row.value.as_bytes());
            bytes.extend_from_slice(choices.as_bytes());
            out_rows.push(StellarInspectorRow {
                key: row.key.0,
                flags: row.flags,
                section: row.property.section() as u32,
                name_len: name.len() as u32,
                value_len: row.value.len() as u32,
                choices_len: choices.len() as u32,
            });
        }
        let (Ok(text_len), Ok(name_len)) = (
            u32::try_from(bytes.len()),
            u32::try_from(snapshot.display_name.len()),
        ) else {
            return StellarStatus::Internal;
        };
        unsafe {
            info.write(StellarInspectorInfo {
                version: INSPECTOR_INFO_VERSION,
                count: snapshot.count,
                classes: snapshot.classes,
                row_count: out_rows.len() as u32,
                revision: window.revision.0,
                history_sequence: window.history_sequence,
                view: window.active_view.unwrap_or(StableId::NIL).to_bytes(),
                name_len,
                text_len,
            })
        };
        if out_rows.len() > row_capacity
            || bytes.len() > text_capacity
            || (!out_rows.is_empty() && rows.is_null())
            || (!bytes.is_empty() && text.is_null())
        {
            return StellarStatus::BufferTooSmall;
        }
        unsafe {
            if !out_rows.is_empty() {
                std::ptr::copy_nonoverlapping(out_rows.as_ptr(), rows, out_rows.len());
            }
            if !bytes.is_empty() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), text, bytes.len());
            }
        }
        StellarStatus::Ok
    })
}

/// 속성 편집 머리(고정 48 byte).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StellarPropertyEdit {
    /// 셸이 표시한 snapshot의 active View.
    pub view: [u8; 16],
    /// 표시한 snapshot의 revision.
    pub revision: u64,
    /// 표시한 snapshot의 history sequence.
    pub history_sequence: u64,
    /// `mode` 1일 때 더할 값(px·deg·개수).
    pub delta: f64,
    /// 0: 값 텍스트로 설정, 1: 각 node의 현재 값에 `delta`를 더함.
    pub mode: u32,
    /// 0 확정, 1 연속 중간 값, 2 연속 끝, 3 연속 취소(표시 상태와 무관하게 열린 preview를 닫음), 4 반복 입력
    /// (화살표·색: 바로 기록하고 같은 속성의 0.5초 안 반복은 Undo 한 항목으로 합침).
    pub phase: u32,
}

fn write_diagnostic(text: &str, out: *mut u8, capacity: usize, out_len: *mut usize) {
    if !out_len.is_null() {
        unsafe { out_len.write(text.len()) };
    }
    if out.is_null() || capacity == 0 {
        return;
    }
    let mut end = text.len().min(capacity);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), out, end) };
}

/// 창의 확정 선택 전체에 속성 하나를 적용하도록 요청합니다. 기다리지 않고 request id를 씁니다.
///
/// 결과는 `STELLAR_EVENT_WINDOW_VIEW_CHANGED` 또는 `STELLAR_EVENT_REQUEST_FAILED`(잠김, 선택 변경 등)로 옵니다.
/// 값 텍스트가 속성 문법에 맞지 않으면 요청을 만들지 않고 `InvalidArgument`와 `code: 이유` 형식의 진단을
/// `diagnostic`에 씁니다(`out_diagnostic_len`에는 전체 byte 수).
///
/// # Safety
/// `edit`은 읽기 가능, `name`·`value`는 각 길이만큼 읽기 가능(value는 `mode` 1이면 null 가능)해야 합니다.
/// `out_request`, `diagnostic`(`diagnostic_capacity` byte), `out_diagnostic_len`은 null이거나 쓰기 가능해야
/// 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_edit_property(
    handle: u64,
    session: u64,
    edit: *const StellarPropertyEdit,
    name: *const u8,
    name_len: usize,
    value: *const u8,
    value_len: usize,
    out_request: *mut u64,
    diagnostic: *mut u8,
    diagnostic_capacity: usize,
    out_diagnostic_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if !out_diagnostic_len.is_null() {
            unsafe { *out_diagnostic_len = 0 };
        }
        if edit.is_null() || name.is_null() {
            return StellarStatus::NullPointer;
        }
        let edit = unsafe { *edit };
        let name = unsafe { std::slice::from_raw_parts(name, name_len) };
        let Ok(name) = std::str::from_utf8(name) else {
            return StellarStatus::InvalidUtf8;
        };
        let Some(property) = InspectorProperty::from_name(name) else {
            write_diagnostic(
                &format!("unknown-property: {name} 속성이 없습니다"),
                diagnostic,
                diagnostic_capacity,
                out_diagnostic_len,
            );
            return StellarStatus::InvalidArgument;
        };
        let Some(phase) = PropertyEditPhase::from_code(edit.phase) else {
            return StellarStatus::InvalidArgument;
        };
        let input = match edit.mode {
            0 => {
                if value.is_null() && value_len != 0 {
                    return StellarStatus::NullPointer;
                }
                let bytes = if value_len == 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(value, value_len) }
                };
                let Ok(text) = std::str::from_utf8(bytes) else {
                    return StellarStatus::InvalidUtf8;
                };
                match parse_input(property, text) {
                    Ok(value) => InspectorInput::Set(value),
                    Err(error) => {
                        write_diagnostic(
                            &error.to_string(),
                            diagnostic,
                            diagnostic_capacity,
                            out_diagnostic_len,
                        );
                        return StellarStatus::InvalidArgument;
                    }
                }
            }
            1 if edit.delta.is_finite() && property.grammar().is_numeric() => {
                InspectorInput::Adjust(edit.delta)
            }
            _ => return StellarStatus::InvalidArgument,
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let target = SelectionEditTarget {
            session: WindowSessionHandle(session),
            view: StableId::from_bytes(edit.view),
            revision: DocumentRevision(edit.revision),
            history_sequence: edit.history_sequence,
        };
        let result = match rt.app() {
            Ok(app) => app.edit_selection_property(target, property, input, phase),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(h: u64, session: u64) -> (StellarInspectorInfo, Vec<(String, String, u32)>) {
        let mut info = StellarInspectorInfo::default();
        let mut rows = vec![StellarInspectorRow::default(); 64];
        let mut text = vec![0u8; 4096];
        let status = unsafe {
            stellar_window_inspector(
                h,
                session,
                &mut info,
                rows.as_mut_ptr(),
                rows.len(),
                text.as_mut_ptr(),
                text.len(),
            )
        };
        assert_eq!(status, StellarStatus::Ok);
        let mut offset = info.name_len as usize;
        let mut out = Vec::new();
        for row in &rows[..info.row_count as usize] {
            let mut take = |len: u32| {
                let s = String::from_utf8(text[offset..offset + len as usize].to_vec()).unwrap();
                offset += len as usize;
                s
            };
            let name = take(row.name_len);
            let value = take(row.value_len);
            take(row.choices_len);
            out.push((name, value, row.flags));
        }
        (info, out)
    }

    fn value(rows: &[(String, String, u32)], name: &str) -> String {
        rows.iter()
            .find(|r| r.0 == name)
            .map(|r| r.1.clone())
            .unwrap_or_else(|| panic!("{name} 행 없음"))
    }

    fn edit(
        h: u64,
        session: u64,
        info: &StellarInspectorInfo,
        name: &str,
        value: &str,
    ) -> (StellarStatus, String) {
        let edit = StellarPropertyEdit {
            view: info.view,
            revision: info.revision,
            history_sequence: info.history_sequence,
            delta: 0.,
            mode: 0,
            phase: 0,
        };
        let mut request = 0u64;
        let mut diagnostic = [0u8; 256];
        let mut len = 0usize;
        let status = unsafe {
            stellar_window_edit_property(
                h,
                session,
                &edit,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
                &mut request,
                diagnostic.as_mut_ptr(),
                diagnostic.len(),
                &mut len,
            )
        };
        (
            status,
            String::from_utf8_lossy(&diagnostic[..len.min(256)]).into_owned(),
        )
    }

    #[test]
    fn shell_reads_rows_edits_with_css_text_and_undoes_through_the_c_abi() {
        use crate::window_session::stellar_window_session_select;
        use crate::window_session::tests::{
            create, next_event, open_project, open_window, FixtureProject,
        };
        use stellar_runtime::app::RuntimeEvent;
        let project = FixtureProject::copy("inspector-abi");
        let h = create();
        let session = open_window(h, open_project(h, &project.path));
        // 선택이 없으면 행이 없습니다.
        let (info, rows) = read(h, session);
        assert_eq!((info.count, rows.len()), (0, 0));
        let child = StableId::new_v7(11, 0, 11).to_bytes();
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_select(h, session, child.as_ptr(), 1, &mut request) },
            StellarStatus::Ok
        );
        assert!(matches!(
            next_event(h),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let (info, rows) = read(h, session);
        assert_eq!(info.count, 1);
        assert_eq!(info.classes, 1, "Frame");
        let before = value(&rows, "width");
        assert!(rows.iter().any(|r| r.0 == "background-color"));
        // 잘못된 값은 요청 없이 진단, 올바른 값은 정규화되어 저장.
        let (status, diagnostic) = edit(h, session, &info, "width", "2rem");
        assert_eq!(status, StellarStatus::InvalidArgument);
        assert!(diagnostic.starts_with("unsupported-unit"), "{diagnostic}");
        let (status, _) = edit(h, session, &info, "width", "320");
        assert_eq!(status, StellarStatus::Ok);
        assert!(matches!(
            next_event(h),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let (after, rows) = read(h, session);
        assert_eq!(value(&rows, "width"), "320px");
        // 오래된 snapshot의 편집은 거부됩니다.
        let (status, _) = edit(h, session, &info, "width", "10");
        assert_eq!(status, StellarStatus::Ok);
        assert!(matches!(next_event(h), RuntimeEvent::RequestFailed { .. }));
        let (status, _) = edit(h, session, &after, "background-color", "#3366FF");
        assert_eq!(status, StellarStatus::Ok);
        assert!(matches!(
            next_event(h),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let (_, rows) = read(h, session);
        assert_eq!(value(&rows, "background-color"), "#3366ff");
        // Undo 두 번이면 처음 값.
        for _ in 0..2 {
            let mut request = 0u64;
            assert_eq!(
                unsafe {
                    crate::window_session::stellar_window_session_perform(
                        h,
                        session,
                        stellar_runtime::app_command::AppCommand::Undo.code(),
                        &mut request,
                    )
                },
                StellarStatus::Ok
            );
            next_event(h);
        }
        let (_, rows) = read(h, session);
        assert_eq!(value(&rows, "width"), before);
        crate::stellar_runtime_destroy(h);
    }

    #[test]
    fn layouts_are_fixed() {
        assert_eq!(std::mem::size_of::<StellarInspectorInfo>(), 56);
        assert_eq!(std::mem::size_of::<StellarInspectorRow>(), 24);
        assert_eq!(std::mem::size_of::<StellarPropertyEdit>(), 48);
    }

    #[test]
    fn bad_values_are_diagnosed_without_a_request() {
        let edit = StellarPropertyEdit {
            view: [0; 16],
            revision: 0,
            history_sequence: 0,
            delta: 0.,
            mode: 0,
            phase: 0,
        };
        let name = b"width";
        let value = b"2rem";
        let mut buffer = [0u8; 128];
        let mut len = 0usize;
        let mut request = 7u64;
        let status = unsafe {
            stellar_window_edit_property(
                u64::MAX,
                1,
                &edit,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
                &mut request,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut len,
            )
        };
        assert_eq!(status, StellarStatus::InvalidArgument);
        assert_eq!(request, 0);
        let text = std::str::from_utf8(&buffer[..len]).unwrap();
        assert!(text.starts_with("unsupported-unit:"), "{text}");
        // 진단이 용량보다 길면 UTF-8 경계에서 자릅니다.
        let mut small = [0u8; 20];
        let status = unsafe {
            stellar_window_edit_property(
                u64::MAX,
                1,
                &edit,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
                std::ptr::null_mut(),
                small.as_mut_ptr(),
                small.len(),
                &mut len,
            )
        };
        assert_eq!(status, StellarStatus::InvalidArgument);
        assert!(len > small.len());
    }
}
