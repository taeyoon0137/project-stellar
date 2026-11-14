//! 창 session C ABI (XII.2.2, ADR-0028).
//!
//! 셸의 창 controller 하나가 창 session handle 하나를 소유합니다. 요청은 다른 프로젝트·문서 요청과 같이
//! 기다리지 않고 request id를 돌려주며, 완료는 `stellar_runtime_poll_event`의 event
//! (`STELLAR_EVENT_WINDOW_SESSION_*`)와 [`stellar_window_session_info`]의 상태 snapshot으로만 돌아옵니다.

use stellar_runtime::app::{AppRuntime, ProjectSessionId, WindowSessionHandle};

use crate::{ffi_guard, registry, StellarStatus};

/// `StellarWindowSessionInfo` 레이아웃 버전.
pub const WINDOW_SESSION_INFO_VERSION: u32 = 1;

/// 열린 창 session 정보(고정 88 byte, C ABI).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct StellarWindowSessionInfo {
    /// [`WINDOW_SESSION_INFO_VERSION`](코어가 씀).
    pub version: u32,
    /// 작업 문서가 있으면 1, 없으면 0(저장한 적 없는 프로젝트).
    pub has_document: u32,
    /// 창 session handle.
    pub session: u64,
    /// 소속 프로젝트 session handle.
    pub project: u64,
    /// snapshot 게시 시점의 작업 문서 revision.
    pub revision: u64,
    /// loaded generation 번호(저장한 적 없으면 0).
    pub generation: u64,
    /// undo stack 깊이.
    pub undo_depth: u32,
    /// redo stack 깊이.
    pub redo_depth: u32,
    /// 선택된 node 수.
    pub selection_count: u32,
    /// 0.
    pub reserved: u32,
    /// 창 편집 session id 16 byte(UUID 바이트 순서).
    pub window_id: [u8; 16],
    /// 작업 문서 StableId 16 byte(없으면 0).
    pub document_id: [u8; 16],
}

/// 프로젝트 session 아래에서 창 session 열기를 요청합니다. 기다리지 않고 request id를 씁니다.
///
/// 결과는 `STELLAR_EVENT_WINDOW_SESSION_OPENED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_open(
    handle: u64,
    project: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.open_window_session(ProjectSessionId(project)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 창 session 닫기를 요청합니다. 그 창의 문서 state·history·선택을 저장하지 않고 폐기합니다.
///
/// 결과는 `STELLAR_EVENT_WINDOW_SESSION_CLOSED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_close(
    handle: u64,
    session: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.close_window_session(WindowSessionHandle(session)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 열린 창 session의 최신 snapshot 정보를 씁니다. document thread를 기다리지 않습니다.
///
/// 열려 있지 않은 창 session은 `InvalidArgument`(last error 설정)입니다.
///
/// # Safety
/// `out_info`는 `StellarWindowSessionInfo` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_info(
    handle: u64,
    session: u64,
    out_info: *mut StellarWindowSessionInfo,
) -> StellarStatus {
    ffi_guard(move || {
        if out_info.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let snapshot = rt.app.as_ref().map(AppRuntime::state);
        let Some(summary) = snapshot
            .as_ref()
            .and_then(|state| state.window(WindowSessionHandle(session)))
        else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                format!("열려 있지 않은 창 session입니다: {session}"),
            ));
            return StellarStatus::InvalidArgument;
        };
        let info = StellarWindowSessionInfo {
            version: WINDOW_SESSION_INFO_VERSION,
            has_document: u32::from(summary.document_id.is_some()),
            session: summary.session.0,
            project: summary.project.0,
            revision: summary.revision.0,
            generation: summary.generation,
            undo_depth: summary.undo_depth,
            redo_depth: summary.redo_depth,
            selection_count: summary.selection_len,
            reserved: 0,
            window_id: summary.window_id.stable_id().to_bytes(),
            document_id: summary.document_id.map_or([0; 16], |id| id.to_bytes()),
        };
        unsafe { *out_info = info };
        StellarStatus::Ok
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use stellar_runtime::app::{RequestFailure, RequestId, RuntimeEvent};

    use crate::{
        stellar_project_close, stellar_project_open, stellar_runtime_create,
        stellar_runtime_destroy, stellar_runtime_poll_event, stellar_runtime_state,
        StellarRuntimeState,
    };

    /// 저장 프로젝트 fixture(`crates/runtime/tests/fixtures/window-session-project`)를 복사한 임시 프로젝트.
    pub(crate) struct FixtureProject {
        pub(crate) path: PathBuf,
    }

    impl FixtureProject {
        pub(crate) fn copy(tag: &str) -> FixtureProject {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-ffi-window-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../runtime/tests/fixtures/window-session-project");
            copy_tree(&source, &path);
            std::fs::create_dir(path.join(".git")).unwrap();
            FixtureProject { path }
        }
    }

    impl Drop for FixtureProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// 다음 event를 C ABI로 읽어 해석합니다(최대 5초).
    pub(crate) fn next_event(h: u64) -> RuntimeEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut kind = 0u32;
            let mut need = 0usize;
            let status = unsafe {
                stellar_runtime_poll_event(h, &mut kind, std::ptr::null_mut(), 0, &mut need)
            };
            if status == StellarStatus::Ok {
                let mut buf = vec![0u8; need];
                let mut written = 0usize;
                let status = unsafe {
                    stellar_runtime_poll_event(
                        h,
                        &mut kind,
                        buf.as_mut_ptr(),
                        buf.len(),
                        &mut written,
                    )
                };
                assert_eq!(status, StellarStatus::Ok);
                return RuntimeEvent::decode(kind, &buf[..written]).expect("event payload");
            }
            assert_eq!(status, StellarStatus::NoError);
            assert!(Instant::now() < deadline, "event가 오지 않았습니다");
            std::thread::yield_now();
        }
    }

    pub(crate) fn create() -> u64 {
        let mut h = 0u64;
        assert_eq!(unsafe { stellar_runtime_create(&mut h) }, StellarStatus::Ok);
        h
    }

    pub(crate) fn open_project(h: u64, root: &Path) -> u64 {
        let bytes = root.to_str().unwrap().as_bytes();
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_project_open(h, bytes.as_ptr(), bytes.len(), &mut request) },
            StellarStatus::Ok
        );
        match next_event(h) {
            RuntimeEvent::ProjectOpened { project, .. } => project.0,
            other => panic!("프로젝트 열기 실패: {other:?}"),
        }
    }

    pub(crate) fn open_window(h: u64, project: u64) -> u64 {
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_open(h, project, &mut request) },
            StellarStatus::Ok
        );
        match next_event(h) {
            RuntimeEvent::WindowSessionOpened {
                request: got,
                session,
                ..
            } => {
                assert_eq!(got, RequestId(request));
                session.0
            }
            other => panic!("창 session 열기 실패: {other:?}"),
        }
    }

    pub(crate) fn info(h: u64, session: u64) -> Result<StellarWindowSessionInfo, StellarStatus> {
        let mut info = StellarWindowSessionInfo::default();
        match unsafe { stellar_window_session_info(h, session, &mut info) } {
            StellarStatus::Ok => Ok(info),
            status => Err(status),
        }
    }

    fn state(h: u64) -> StellarRuntimeState {
        let mut state = StellarRuntimeState::default();
        assert_eq!(
            unsafe { stellar_runtime_state(h, &mut state) },
            StellarStatus::Ok
        );
        state
    }

    #[test]
    fn info_layout_is_fixed() {
        assert_eq!(std::mem::size_of::<StellarWindowSessionInfo>(), 88);
        assert_eq!(std::mem::align_of::<StellarWindowSessionInfo>(), 8);
    }

    #[test]
    fn two_window_sessions_open_and_close_through_the_c_abi() {
        let first = FixtureProject::copy("a");
        let second = FixtureProject::copy("b");
        let h = create();
        let project_a = open_project(h, &first.path);
        let project_b = open_project(h, &second.path);
        let a = open_window(h, project_a);
        let b = open_window(h, project_b);
        assert_ne!(a, b);
        assert_eq!(state(h).window_session_count, 2);

        let info_a = info(h, a).unwrap();
        let info_b = info(h, b).unwrap();
        assert_eq!(info_a.version, WINDOW_SESSION_INFO_VERSION);
        assert_eq!((info_a.project, info_b.project), (project_a, project_b));
        assert_eq!((info_a.has_document, info_a.generation), (1, 1));
        assert_eq!(info_a.document_id, info_b.document_id, "같은 fixture 문서");
        assert_ne!(info_a.window_id, info_b.window_id, "창마다 다른 id");
        assert_eq!((info_a.undo_depth, info_a.redo_depth), (0, 0));

        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_close(h, a, &mut request) },
            StellarStatus::Ok
        );
        assert_eq!(
            next_event(h),
            RuntimeEvent::WindowSessionClosed {
                request: RequestId(request),
                session: WindowSessionHandle(a),
            }
        );
        assert_eq!(info(h, a), Err(StellarStatus::InvalidArgument));
        assert!(info(h, b).is_ok());
        assert_eq!(state(h).window_session_count, 1);

        // 프로젝트를 닫으면 남은 창 session도 닫힙니다.
        assert_eq!(
            unsafe { stellar_project_close(h, project_b, &mut request) },
            StellarStatus::Ok
        );
        assert!(matches!(next_event(h), RuntimeEvent::ProjectClosed { .. }));
        assert_eq!(info(h, b), Err(StellarStatus::InvalidArgument));
        assert_eq!(state(h).window_session_count, 0);

        // 닫은 창 session을 다시 닫으면 실패 event입니다.
        assert_eq!(
            unsafe { stellar_window_session_close(h, a, &mut request) },
            StellarStatus::Ok
        );
        let RuntimeEvent::RequestFailed { failure, .. } = next_event(h) else {
            panic!("실패 event가 아님");
        };
        assert_eq!(failure, RequestFailure::UnknownWindowSession);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn null_info_pointer_and_unknown_handle_are_rejected() {
        let h = create();
        assert_eq!(
            unsafe { stellar_window_session_info(h, 1, std::ptr::null_mut()) },
            StellarStatus::NullPointer
        );
        assert_eq!(info(h, 1), Err(StellarStatus::InvalidArgument));
        let mut request = 7u64;
        assert_eq!(
            unsafe { stellar_window_session_open(0, 1, &mut request) },
            StellarStatus::InvalidHandle
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }
}
