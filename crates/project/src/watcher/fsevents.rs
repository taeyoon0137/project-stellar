//! macOS FSEvents 감시 adapter (XIII.2.2).
//!
//! 프로젝트 root와 외부 symlink target allowlist마다 FSEvents stream을 하나씩 만들고(파일 단위 event,
//! `kFSEventStreamCreateFlagFileEvents`), OS 알림을 플랫폼 독립 [`FileWatchEvent`]로 normalize합니다.
//! 새 crate 의존 없이 시스템 framework(CoreServices·CoreFoundation)와 libdispatch의 C 함수만
//! 선언해 씁니다.
//!
//! normalize 규칙([`Normalizer`], 플랫폼 독립 — 모든 플랫폼에서 테스트):
//! 1. 커널·사용자 drop → `Overflow(EventsDropped)`, 하위 재검사 요구·mount 변화 →
//!    `Overflow(RescanRequired)`, 감시 root 변화 → `Overflow(RootChanged)`.
//! 2. 이름 변경은 FSEvents가 원래·새 경로에 대해 연속 event id로 보고하므로, 연속 두 rename event의
//!    앞 경로가 없고 뒤 경로가 있으면 `Renamed { from, to }`로 짝짓습니다. 짝이 없으면 경로가 없으면
//!    `Removed`(감시 밖으로 나감), 있으면 이미 아는 경로는 `Modified`, 모르는 경로는
//!    `Created`(감시 밖에서 들어옴)입니다.
//! 3. 그 밖에는 현재 존재 여부(`lstat`, 링크를 따르지 않음)로 정합니다: 없음 → `Removed`.
//! 4. FSEvents의 항목 flag는 **누적**됩니다(최근에 만든 파일을 고쳐도 생성 flag가 함께 옴). 그래서
//!    생성 flag가 있어도 이번 감시 동안 이미 보고한 경로이거나, 파일의 생성 시각(birth time)이 감시
//!    시작보다 이르면 `Modified`로, 그 밖에만 `Created`로 보고합니다. 그 외 변화는 `Modified`입니다.
//! 5. root 기준 경로로 표현할 수 없는 경로는 버리지 않고 `Overflow(UnrepresentablePath)`입니다(파일
//!    root를 위해 함께 감시하는 형제 항목은 감시 대상이 아니므로 버림).
//! 6. 대기열이 [`MAX_PENDING_EVENTS`]를 넘으면 그 뒤 알림을 버리고 `Overflow(QueueFull)`을 한 번
//!    내보냅니다. event는 FSEvents event id(시스템 전역 단조 증가) 순으로 정렬해 내보냅니다.

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use super::event::{FileWatchEvent, OverflowReason, WatchPath};
use super::roots::WatchRoots;

/// adapter 내부 대기열 상한(원시 알림 수).
pub const MAX_PENDING_EVENTS: usize = 1 << 16;

/// FSEvents event flag(`FSEvents.h`).
pub mod flags {
    /// 하위 디렉터리 재검사 필요.
    pub const MUST_SCAN_SUB_DIRS: u32 = 0x0000_0001;
    /// 사용자 공간 drop.
    pub const USER_DROPPED: u32 = 0x0000_0002;
    /// 커널 drop.
    pub const KERNEL_DROPPED: u32 = 0x0000_0004;
    /// event id 순환.
    pub const EVENT_IDS_WRAPPED: u32 = 0x0000_0008;
    /// 과거 기록 재생 끝.
    pub const HISTORY_DONE: u32 = 0x0000_0010;
    /// 감시 root 변화.
    pub const ROOT_CHANGED: u32 = 0x0000_0020;
    /// volume mount.
    pub const MOUNT: u32 = 0x0000_0040;
    /// volume unmount.
    pub const UNMOUNT: u32 = 0x0000_0080;
    /// 항목 생성.
    pub const ITEM_CREATED: u32 = 0x0000_0100;
    /// 항목 삭제.
    pub const ITEM_REMOVED: u32 = 0x0000_0200;
    /// inode metadata 수정.
    pub const ITEM_INODE_META_MOD: u32 = 0x0000_0400;
    /// 이름 변경.
    pub const ITEM_RENAMED: u32 = 0x0000_0800;
    /// 내용 수정.
    pub const ITEM_MODIFIED: u32 = 0x0000_1000;
    /// Finder 정보 수정.
    pub const ITEM_FINDER_INFO_MOD: u32 = 0x0000_2000;
    /// 소유자 변경.
    pub const ITEM_CHANGE_OWNER: u32 = 0x0000_4000;
    /// 확장 속성 수정.
    pub const ITEM_XATTR_MOD: u32 = 0x0000_8000;
    /// 항목이 파일.
    pub const ITEM_IS_FILE: u32 = 0x0001_0000;
    /// 항목이 디렉터리.
    pub const ITEM_IS_DIR: u32 = 0x0002_0000;
    /// 항목이 symlink.
    pub const ITEM_IS_SYMLINK: u32 = 0x0004_0000;
}

/// FSEvents가 보고한 원시 알림 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFsEvent {
    /// event id(시스템 전역 단조 증가).
    pub id: u64,
    /// event flag.
    pub flags: u32,
    /// 보고된 절대 경로 bytes.
    pub path: Vec<u8>,
}

/// 경로의 현재 상태(링크를 따르지 않는 조회).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathProbe {
    /// 존재 여부.
    pub exists: bool,
    /// 생성 시각(birth time, 알 수 없으면 `None`).
    pub born: Option<SystemTime>,
}

impl PathProbe {
    /// 실제 파일 시스템을 `lstat`으로 조회합니다.
    pub fn of(path: &Path) -> PathProbe {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => PathProbe {
                exists: true,
                born: meta.created().ok(),
            },
            Err(_) => PathProbe {
                exists: false,
                born: None,
            },
        }
    }
}

/// FSEvents 원시 알림을 [`FileWatchEvent`]로 바꾸는 상태 있는 normalizer(모듈 문서의 규칙).
#[derive(Debug, Clone)]
pub struct Normalizer {
    /// 감시 시작 시각.
    started_at: SystemTime,
    /// 이번 감시 동안 존재를 보고한 경로.
    known: HashSet<WatchPath>,
}

impl Normalizer {
    /// 감시 시작 시각으로 만듭니다.
    pub fn new(started_at: SystemTime) -> Normalizer {
        Normalizer {
            started_at,
            known: HashSet::new(),
        }
    }

    /// 원시 알림 목록(event id 순)을 normalize합니다. `probe`는 경로의 현재 상태입니다(테스트 주입).
    pub fn normalize(
        &mut self,
        raw: &[RawFsEvent],
        roots: &WatchRoots,
        probe: impl Fn(&Path) -> PathProbe,
    ) -> Vec<FileWatchEvent> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < raw.len() {
            let event = &raw[index];
            index += 1;
            if let Some(reason) = overflow_reason(event.flags) {
                push_overflow(&mut out, reason);
                continue;
            }
            if event.flags & (flags::HISTORY_DONE | flags::EVENT_IDS_WRAPPED) != 0
                && event.flags & ITEM_FLAGS == 0
            {
                continue;
            }
            let Some(path) = to_path(&event.path) else {
                push_overflow(&mut out, OverflowReason::UnrepresentablePath);
                continue;
            };
            let Some(watch_path) = roots.relativize(path) else {
                // 파일 root의 형제 항목은 감시 대상이 아니므로 버리고, 그 밖은 넘침으로 보고합니다.
                if !roots.is_file_root_neighbor(path) {
                    push_overflow(&mut out, OverflowReason::UnrepresentablePath);
                }
                continue;
            };
            let state = probe(path);
            if !state.exists && event.flags & flags::ITEM_RENAMED != 0 {
                // 연속 id의 rename 짝: 앞은 없고 뒤는 있음.
                if let Some((to, consumed)) =
                    self.rename_target(event, raw.get(index), roots, &probe)
                {
                    index += consumed;
                    self.known.remove(&watch_path);
                    self.known.insert(to.clone());
                    out.push(FileWatchEvent::Renamed {
                        from: watch_path,
                        to,
                    });
                    continue;
                }
            }
            if !state.exists {
                self.known.remove(&watch_path);
                out.push(FileWatchEvent::Removed(watch_path));
                continue;
            }
            let known = !self.known.insert(watch_path.clone());
            let created_flag = event.flags & (flags::ITEM_CREATED | flags::ITEM_RENAMED) != 0;
            let pre_existing = event.flags & flags::ITEM_RENAMED == 0
                && state.born.is_some_and(|born| born < self.started_at);
            out.push(if created_flag && !known && !pre_existing {
                FileWatchEvent::Created(watch_path)
            } else {
                FileWatchEvent::Modified(watch_path)
            });
        }
        out
    }

    /// `event`(없는 경로의 rename)와 짝을 이루는 다음 rename의 새 경로.
    fn rename_target(
        &self,
        event: &RawFsEvent,
        next: Option<&RawFsEvent>,
        roots: &WatchRoots,
        probe: &impl Fn(&Path) -> PathProbe,
    ) -> Option<(WatchPath, usize)> {
        let next = next?;
        let paired = next.flags & flags::ITEM_RENAMED != 0
            && next.id == event.id.wrapping_add(1)
            && overflow_reason(next.flags).is_none();
        if !paired {
            return None;
        }
        let next_path = to_path(&next.path)?;
        let to = roots.relativize(next_path)?;
        probe(next_path).exists.then_some((to, 1))
    }
}

/// 항목 단위 변화 flag.
const ITEM_FLAGS: u32 = flags::ITEM_CREATED
    | flags::ITEM_REMOVED
    | flags::ITEM_INODE_META_MOD
    | flags::ITEM_RENAMED
    | flags::ITEM_MODIFIED
    | flags::ITEM_FINDER_INFO_MOD
    | flags::ITEM_CHANGE_OWNER
    | flags::ITEM_XATTR_MOD;

/// 넘침으로 처리할 flag.
fn overflow_reason(event_flags: u32) -> Option<OverflowReason> {
    if event_flags & (flags::KERNEL_DROPPED | flags::USER_DROPPED) != 0 {
        Some(OverflowReason::EventsDropped)
    } else if event_flags & flags::ROOT_CHANGED != 0 {
        Some(OverflowReason::RootChanged)
    } else if event_flags & (flags::MUST_SCAN_SUB_DIRS | flags::MOUNT | flags::UNMOUNT) != 0 {
        Some(OverflowReason::RescanRequired)
    } else {
        None
    }
}

/// 같은 사유의 넘침이 연달아 나오면 하나로 합칩니다.
fn push_overflow(out: &mut Vec<FileWatchEvent>, reason: OverflowReason) {
    if out.last() != Some(&FileWatchEvent::Overflow(reason)) {
        out.push(FileWatchEvent::Overflow(reason));
    }
}

#[cfg(unix)]
fn to_path(bytes: &[u8]) -> Option<&Path> {
    use std::os::unix::ffi::OsStrExt;
    Some(Path::new(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn to_path(bytes: &[u8]) -> Option<&Path> {
    std::str::from_utf8(bytes).ok().map(Path::new)
}

#[cfg(target_os = "macos")]
pub use stream::{FsEventsWatcher, WatcherError};

#[cfg(target_os = "macos")]
mod stream {
    use std::collections::VecDeque;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};
    use std::time::{Duration, Instant, SystemTime};

    use super::{Normalizer, PathProbe, RawFsEvent, MAX_PENDING_EVENTS};
    use crate::watcher::event::{FileWatchEvent, OverflowReason, WatchRootId};
    use crate::watcher::roots::{WatchRootError, WatchRootKind, WatchRoots};
    use crate::watcher::ProjectFileWatcher;

    type CFAllocatorRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;
    type FSEventStreamRef = *mut c_void;
    type DispatchQueue = *mut c_void;

    #[repr(C)]
    struct FSEventStreamContext {
        version: CFIndex,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
    }

    type FSEventStreamCallback = extern "C" fn(
        stream: *const c_void,
        info: *mut c_void,
        num_events: usize,
        event_paths: *mut c_void,
        event_flags: *const u32,
        event_ids: *const u64,
    );

    /// `kFSEventStreamEventIdSinceNow`.
    const SINCE_NOW: u64 = u64::MAX;
    /// `kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagWatchRoot |
    /// kFSEventStreamCreateFlagFileEvents`.
    const CREATE_FLAGS: u32 = 0x02 | 0x04 | 0x10;
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// stream latency(초). 짧은 시간의 알림을 OS가 묶어 전달합니다.
    const LATENCY_SECONDS: f64 = 0.02;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeArrayCallBacks: c_void;
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFArrayCreate(
            alloc: CFAllocatorRef,
            values: *const *const c_void,
            num_values: CFIndex,
            callbacks: *const c_void,
        ) -> CFArrayRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn FSEventStreamCreate(
            allocator: CFAllocatorRef,
            callback: FSEventStreamCallback,
            context: *mut FSEventStreamContext,
            paths_to_watch: CFArrayRef,
            since_when: u64,
            latency: f64,
            flags: u32,
        ) -> FSEventStreamRef;
        fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueue);
        fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
        fn FSEventStreamFlushSync(stream: FSEventStreamRef);
        fn FSEventStreamStop(stream: FSEventStreamRef);
        fn FSEventStreamInvalidate(stream: FSEventStreamRef);
        fn FSEventStreamRelease(stream: FSEventStreamRef);
    }

    // libdispatch(libSystem에 포함, 별도 link 불필요).
    extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> DispatchQueue;
        fn dispatch_sync_f(
            queue: DispatchQueue,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        fn dispatch_release(object: DispatchQueue);
    }

    /// 감시 adapter 오류.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum WatcherError {
        /// 감시 root 등록 실패.
        Root(WatchRootError),
        /// 경로에 NUL이 있어 C 문자열로 바꿀 수 없음.
        InvalidPath {
            /// 경로.
            path: PathBuf,
        },
        /// FSEvents stream 생성·시작 실패.
        Stream {
            /// 감시 경로.
            path: PathBuf,
        },
    }

    impl std::fmt::Display for WatcherError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                WatcherError::Root(e) => write!(f, "{e}"),
                WatcherError::InvalidPath { path } => {
                    write!(f, "감시할 수 없는 경로(NUL 포함): {path:?}")
                }
                WatcherError::Stream { path } => {
                    write!(f, "FSEvents stream을 시작하지 못했습니다: {path:?}")
                }
            }
        }
    }

    impl std::error::Error for WatcherError {}

    impl From<WatchRootError> for WatcherError {
        fn from(e: WatchRootError) -> WatcherError {
            WatcherError::Root(e)
        }
    }

    /// 모든 stream이 공유하는 원시 알림 대기열.
    #[derive(Default)]
    struct EventQueue {
        state: Mutex<QueueState>,
        ready: Condvar,
    }

    #[derive(Default)]
    struct QueueState {
        raw: VecDeque<RawFsEvent>,
        dropped: bool,
    }

    impl EventQueue {
        fn lock(&self) -> MutexGuard<'_, QueueState> {
            // callback에서 panic하지 않도록 poison을 무시합니다(상태는 단순 대기열).
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    /// stream 하나의 callback 문맥(stream 수명 동안 유지).
    struct StreamContext {
        queue: Arc<EventQueue>,
    }

    /// FSEvents stream 하나(감시 root 하나).
    struct Stream {
        root: WatchRootId,
        stream: FSEventStreamRef,
        queue: DispatchQueue,
        context: *const StreamContext,
    }

    // SAFETY: stream·queue·context 포인터는 이 값이 소유하며, FSEvents·libdispatch 함수는 스레드 안전
    // 합니다. context는 Drop에서 stream 정지와 대기열 비움 뒤에만 해제합니다.
    unsafe impl Send for Stream {}

    extern "C" fn noop(_context: *mut c_void) {}

    extern "C" fn on_events(
        _stream: *const c_void,
        info: *mut c_void,
        num_events: usize,
        event_paths: *mut c_void,
        event_flags: *const u32,
        event_ids: *const u64,
    ) {
        if info.is_null() || event_paths.is_null() || event_flags.is_null() || event_ids.is_null() {
            return;
        }
        // SAFETY: info는 Stream이 살아 있는 동안 유효한 StreamContext 포인터입니다(Drop은 stream을
        // 무효화하고 대기열을 비운 뒤에만 해제). FSEvents는 num_events 길이의 C 문자열·flag·id 배열을
        // callback 동안 유효하게 넘깁니다(UseCFTypes 미사용).
        let context = unsafe { &*(info as *const StreamContext) };
        let paths = event_paths as *const *const c_char;
        let mut state = context.queue.lock();
        for i in 0..num_events {
            // SAFETY: 위 설명대로 i < num_events 범위의 배열 원소입니다.
            let (path, flags, id) = unsafe {
                let path = *paths.add(i);
                if path.is_null() {
                    continue;
                }
                (CStr::from_ptr(path), *event_flags.add(i), *event_ids.add(i))
            };
            if state.raw.len() >= MAX_PENDING_EVENTS {
                state.dropped = true;
                continue;
            }
            state.raw.push_back(RawFsEvent {
                id,
                flags,
                path: path.to_bytes().to_vec(),
            });
        }
        drop(state);
        context.queue.ready.notify_all();
    }

    impl Stream {
        /// `path`를 감시하는 stream을 만들어 시작합니다.
        fn start(
            root: WatchRootId,
            path: &Path,
            shared: &Arc<EventQueue>,
        ) -> Result<Stream, WatcherError> {
            let c_path = path_to_cstring(path)?;
            let label =
                CString::new("stellar.project-watcher").map_err(|_| WatcherError::InvalidPath {
                    path: path.to_path_buf(),
                })?;
            let context = Box::into_raw(Box::new(StreamContext {
                queue: Arc::clone(shared),
            }));
            let fail = |context: *mut StreamContext| {
                // SAFETY: 아직 어떤 stream에도 넘기지 않았거나 이미 해제된 stream의 context입니다.
                drop(unsafe { Box::from_raw(context) });
                WatcherError::Stream {
                    path: path.to_path_buf(),
                }
            };
            // SAFETY: 아래 CF·FSEvents·dispatch 호출은 문서화된 C API이며, 넘기는 포인터는 모두 이
            // 함수에서 만든 유효 객체입니다. 만든 CF 객체는 stream 생성 뒤 해제합니다(stream이 복사·
            // 보유).
            unsafe {
                let cf_path = CFStringCreateWithCString(std::ptr::null(), c_path.as_ptr(), UTF8);
                if cf_path.is_null() {
                    return Err(fail(context));
                }
                let values = [cf_path];
                let paths = CFArrayCreate(
                    std::ptr::null(),
                    values.as_ptr(),
                    1,
                    &kCFTypeArrayCallBacks as *const c_void,
                );
                CFRelease(cf_path);
                if paths.is_null() {
                    return Err(fail(context));
                }
                let mut stream_context = FSEventStreamContext {
                    version: 0,
                    info: context as *mut c_void,
                    retain: std::ptr::null(),
                    release: std::ptr::null(),
                    copy_description: std::ptr::null(),
                };
                let stream = FSEventStreamCreate(
                    std::ptr::null(),
                    on_events,
                    &mut stream_context,
                    paths,
                    SINCE_NOW,
                    LATENCY_SECONDS,
                    CREATE_FLAGS,
                );
                CFRelease(paths);
                if stream.is_null() {
                    return Err(fail(context));
                }
                let queue = dispatch_queue_create(label.as_ptr(), std::ptr::null());
                if queue.is_null() {
                    FSEventStreamRelease(stream);
                    return Err(fail(context));
                }
                FSEventStreamSetDispatchQueue(stream, queue);
                if FSEventStreamStart(stream) == 0 {
                    FSEventStreamInvalidate(stream);
                    FSEventStreamRelease(stream);
                    dispatch_release(queue);
                    return Err(fail(context));
                }
                Ok(Stream {
                    root,
                    stream,
                    queue,
                    context,
                })
            }
        }

        /// OS가 보유한 알림을 지금 callback으로 전달하게 합니다(돌아올 때 전달 완료).
        fn flush(&self) {
            // SAFETY: 시작된 유효 stream입니다.
            unsafe { FSEventStreamFlushSync(self.stream) };
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: 정지 → 무효화 → 해제 순서로 stream을 정리하고, 직렬 dispatch 대기열에 빈 작업을
            // 동기 실행해 이미 들어간 callback이 모두 끝난 뒤 context를 해제합니다.
            unsafe {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
                FSEventStreamRelease(self.stream);
                dispatch_sync_f(self.queue, std::ptr::null_mut(), noop);
                dispatch_release(self.queue);
                drop(Box::from_raw(self.context as *mut StreamContext));
            }
        }
    }

    fn path_to_cstring(path: &Path) -> Result<CString, WatcherError> {
        use std::os::unix::ffi::OsStrExt;
        CString::new(path.as_os_str().as_bytes()).map_err(|_| WatcherError::InvalidPath {
            path: path.to_path_buf(),
        })
    }

    /// 감시할 OS 경로(파일 root는 부모 디렉터리를 감시하고 normalize에서 걸러 냄).
    fn stream_path(roots: &WatchRoots, root: WatchRootId) -> Option<PathBuf> {
        let root = roots.get(root)?;
        match root.kind {
            WatchRootKind::Directory => Some(root.path.clone()),
            WatchRootKind::File => root.path.parent().map(Path::to_path_buf),
        }
    }

    /// macOS FSEvents 기반 [`ProjectFileWatcher`].
    pub struct FsEventsWatcher {
        roots: WatchRoots,
        queue: Arc<EventQueue>,
        normalizer: Mutex<Normalizer>,
        streams: Vec<Stream>,
    }

    impl FsEventsWatcher {
        /// 프로젝트 root와 외부 symlink target allowlist를 감시합니다.
        ///
        /// allowlist의 각 경로는 실제 경로로 해석되며 프로젝트 root 안의 경로는 거부합니다.
        pub fn start(
            project_root: &Path,
            external_targets: &[PathBuf],
        ) -> Result<FsEventsWatcher, WatcherError> {
            let mut roots = WatchRoots::new(project_root)?;
            for target in external_targets {
                roots.add_external(target)?;
            }
            let normalizer = Mutex::new(Normalizer::new(SystemTime::now()));
            let queue = Arc::new(EventQueue::default());
            let mut streams = Vec::new();
            for root in roots.iter() {
                if let Some(path) = stream_path(&roots, root.id) {
                    streams.push(Stream::start(root.id, &path, &queue)?);
                }
            }
            Ok(FsEventsWatcher {
                roots,
                queue,
                normalizer,
                streams,
            })
        }

        /// 감시 root 목록.
        pub fn roots(&self) -> &WatchRoots {
            &self.roots
        }

        /// 감시 중인 root id(stream 순).
        pub fn watched_roots(&self) -> Vec<WatchRootId> {
            self.streams.iter().map(|s| s.root).collect()
        }

        /// OS가 보유한 알림을 대기열로 즉시 전달합니다.
        pub fn flush(&self) {
            for stream in &self.streams {
                stream.flush();
            }
        }

        /// 대기열의 원시 알림을 꺼내 normalize합니다.
        fn take_events(&self) -> Vec<FileWatchEvent> {
            let (mut raw, dropped) = {
                let mut state = self.queue.lock();
                let dropped = std::mem::take(&mut state.dropped);
                (Vec::from(std::mem::take(&mut state.raw)), dropped)
            };
            raw.sort_by_key(|event| event.id);
            let mut events = self
                .normalizer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .normalize(&raw, &self.roots, PathProbe::of);
            if dropped {
                events.push(FileWatchEvent::Overflow(OverflowReason::QueueFull));
            }
            events
        }
    }

    impl ProjectFileWatcher for FsEventsWatcher {
        fn drain(&mut self) -> Vec<FileWatchEvent> {
            self.take_events()
        }

        fn wait(&mut self, timeout: Duration) -> Vec<FileWatchEvent> {
            let deadline = Instant::now() + timeout;
            {
                let mut state = self.queue.lock();
                while state.raw.is_empty() && !state.dropped {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    state = self
                        .queue
                        .ready
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
            self.take_events()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::flags::*;
    use super::*;
    use crate::watcher::event::{WatchPath, WatchRootId};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-fsevents-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn raw(id: u64, flags: u32, path: &Path) -> RawFsEvent {
        use std::os::unix::ffi::OsStrExt;
        RawFsEvent {
            id,
            flags,
            path: path.as_os_str().as_bytes().to_vec(),
        }
    }

    const START: Duration = Duration::from_secs(1_000);

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// (경로, 생성 시각) 목록에 있는 경로만 존재하는 probe.
    fn probe_from(present: Vec<(PathBuf, Option<SystemTime>)>) -> impl Fn(&Path) -> PathProbe {
        move |p: &Path| match present.iter().find(|(q, _)| q == p) {
            Some((_, born)) => PathProbe {
                exists: true,
                born: *born,
            },
            None => PathProbe {
                exists: false,
                born: None,
            },
        }
    }

    fn normalizer() -> Normalizer {
        Normalizer::new(SystemTime::UNIX_EPOCH + START)
    }

    #[test]
    fn normalization_maps_flags_to_model_events() {
        let dir = TempDir::new();
        let roots = WatchRoots::new(&dir.path).unwrap();
        let root = roots.project().path.clone();
        let probe = probe_from(vec![
            (root.join("new.txt"), at(2_000)),
            (root.join("b"), at(500)),
            (root.join("mod.txt"), at(500)),
        ]);
        let events = normalizer().normalize(
            &[
                raw(10, ITEM_CREATED | ITEM_IS_FILE, &root.join("new.txt")),
                raw(11, ITEM_MODIFIED | ITEM_IS_FILE, &root.join("mod.txt")),
                raw(12, ITEM_REMOVED | ITEM_IS_FILE, &root.join("gone.txt")),
                // 연속 id rename 짝(앞 없음, 뒤 있음).
                raw(20, ITEM_RENAMED | ITEM_IS_FILE, &root.join("a")),
                raw(21, ITEM_RENAMED | ITEM_IS_FILE, &root.join("b")),
                // 짝 없는 rename: 없으면 감시 밖으로 나감.
                raw(30, ITEM_RENAMED | ITEM_IS_FILE, &root.join("away")),
                // 생성 후 곧 삭제(현재 없음).
                raw(31, ITEM_CREATED | ITEM_REMOVED, &root.join("blip")),
                raw(
                    32,
                    ITEM_INODE_META_MOD | ITEM_IS_FILE,
                    &root.join("mod.txt"),
                ),
            ],
            &roots,
            probe,
        );
        let p = |s: &str| WatchPath::project(s).unwrap();
        assert_eq!(
            events,
            vec![
                FileWatchEvent::Created(p("new.txt")),
                FileWatchEvent::Modified(p("mod.txt")),
                FileWatchEvent::Removed(p("gone.txt")),
                FileWatchEvent::Renamed {
                    from: p("a"),
                    to: p("b")
                },
                FileWatchEvent::Removed(p("away")),
                FileWatchEvent::Removed(p("blip")),
                FileWatchEvent::Modified(p("mod.txt")),
            ]
        );
    }

    #[test]
    fn accumulated_flags_do_not_turn_modifications_into_creations() {
        let dir = TempDir::new();
        let roots = WatchRoots::new(&dir.path).unwrap();
        let root = roots.project().path.clone();
        let mut normalizer = normalizer();
        let probe = probe_from(vec![
            (root.join("old.bin"), at(500)),
            (root.join("fresh.bin"), at(2_000)),
        ]);
        let p = |s: &str| WatchPath::project(s).unwrap();
        // 감시 시작 전에 만든 파일을 고치면 누적 생성 flag가 와도 Modified.
        let first = normalizer.normalize(
            &[
                raw(1, ITEM_CREATED | ITEM_MODIFIED, &root.join("old.bin")),
                raw(2, ITEM_CREATED | ITEM_IS_FILE, &root.join("fresh.bin")),
            ],
            &roots,
            &probe,
        );
        assert_eq!(
            first,
            vec![
                FileWatchEvent::Modified(p("old.bin")),
                FileWatchEvent::Created(p("fresh.bin")),
            ]
        );
        // 이미 보고한 경로의 이후 알림(누적 생성·rename flag 포함)은 Modified.
        let second = normalizer.normalize(
            &[
                raw(3, ITEM_CREATED | ITEM_MODIFIED, &root.join("fresh.bin")),
                raw(5, ITEM_RENAMED | ITEM_MODIFIED, &root.join("fresh.bin")),
            ],
            &roots,
            &probe,
        );
        assert_eq!(
            second,
            vec![
                FileWatchEvent::Modified(p("fresh.bin")),
                FileWatchEvent::Modified(p("fresh.bin")),
            ]
        );
        // 삭제 뒤 같은 경로에 새로 만들면 다시 Created.
        let gone = probe_from(vec![]);
        normalizer.normalize(
            &[raw(6, ITEM_REMOVED, &root.join("fresh.bin"))],
            &roots,
            &gone,
        );
        let again = probe_from(vec![(root.join("fresh.bin"), at(3_000))]);
        assert_eq!(
            normalizer.normalize(
                &[raw(7, ITEM_CREATED, &root.join("fresh.bin"))],
                &roots,
                &again
            ),
            vec![FileWatchEvent::Created(p("fresh.bin"))]
        );
    }

    #[test]
    fn normalization_reports_overflow_instead_of_dropping() {
        let dir = TempDir::new();
        let roots = WatchRoots::new(&dir.path).unwrap();
        let root = roots.project().path.clone();
        let events = normalizer().normalize(
            &[
                raw(1, KERNEL_DROPPED, &root),
                raw(2, USER_DROPPED, &root),
                raw(3, MUST_SCAN_SUB_DIRS, &root.join("design")),
                raw(4, ROOT_CHANGED, &root),
                raw(5, HISTORY_DONE, Path::new("")),
                raw(6, ITEM_CREATED, Path::new("/elsewhere/x")),
                raw(7, MOUNT, Path::new("/Volumes/x")),
            ],
            &roots,
            |_| PathProbe {
                exists: true,
                born: None,
            },
        );
        assert_eq!(
            events,
            vec![
                // 연속 같은 사유는 하나로.
                FileWatchEvent::Overflow(OverflowReason::EventsDropped),
                FileWatchEvent::Overflow(OverflowReason::RescanRequired),
                FileWatchEvent::Overflow(OverflowReason::RootChanged),
                FileWatchEvent::Overflow(OverflowReason::UnrepresentablePath),
                FileWatchEvent::Overflow(OverflowReason::RescanRequired),
            ]
        );
    }

    #[test]
    fn rename_across_roots_and_unpaired_rename_in() {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let mut roots = WatchRoots::new(&project).unwrap();
        let ext = roots.add_external(&outside).unwrap();
        let project_root = roots.project().path.clone();
        let outside_root = roots.get(ext).unwrap().path.clone();
        let probe = probe_from(vec![
            (project_root.join("x"), at(500)),
            (project_root.join("in"), at(500)),
        ]);
        let events = normalizer().normalize(
            &[
                raw(40, ITEM_RENAMED, &outside_root.join("x")),
                raw(41, ITEM_RENAMED, &project_root.join("x")),
                // 감시 밖에서 들어온 항목(생성 시각은 오래되었어도 rename이면 Created).
                raw(50, ITEM_RENAMED, &project_root.join("in")),
            ],
            &roots,
            probe,
        );
        assert_eq!(
            events,
            vec![
                FileWatchEvent::Renamed {
                    from: WatchPath::new(ext, "x").unwrap(),
                    to: WatchPath::new(WatchRootId::PROJECT, "x").unwrap(),
                },
                FileWatchEvent::Created(WatchPath::project("in").unwrap()),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    mod live {
        use super::*;
        use crate::watcher::ProjectFileWatcher;
        use std::time::{Duration, Instant};

        /// `pred`를 만족하는 event가 올 때까지(최대 10초) 모읍니다.
        fn collect_until(
            watcher: &mut FsEventsWatcher,
            seen: &mut Vec<FileWatchEvent>,
            pred: impl Fn(&[FileWatchEvent]) -> bool,
        ) -> bool {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                watcher.flush();
                seen.extend(watcher.wait(Duration::from_millis(200)));
                if pred(seen) {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
            }
        }

        fn has(events: &[FileWatchEvent], wanted: &FileWatchEvent) -> bool {
            events.contains(wanted)
        }

        /// stream 시작 직후의 OS 지연을 흡수: 표지 파일 event가 보일 때까지 기다린 뒤 비웁니다.
        fn settle(watcher: &mut FsEventsWatcher, dir: &Path) {
            let marker = dir.join(".settle");
            std::fs::write(&marker, b"s").unwrap();
            let mut seen = Vec::new();
            collect_until(watcher, &mut seen, |events| {
                events
                    .iter()
                    .any(|e| e.paths().iter().any(|p| p.relative().ends_with(".settle")))
            });
            std::fs::remove_file(&marker).unwrap();
            let mut seen = Vec::new();
            collect_until(watcher, &mut seen, |events| {
                events.iter().any(|e| {
                    matches!(e, FileWatchEvent::Removed(p) if p.relative().ends_with(".settle"))
                })
            });
        }

        #[test]
        fn project_root_create_modify_delete_rename() {
            let dir = TempDir::new();
            let project = dir.path.join("project");
            std::fs::create_dir_all(project.join("design")).unwrap();
            std::fs::write(project.join("design/existing.bin"), b"v1").unwrap();
            let mut watcher = FsEventsWatcher::start(&project, &[]).unwrap();
            assert_eq!(watcher.watched_roots(), vec![WatchRootId::PROJECT]);
            settle(&mut watcher, &project);
            let p = |s: &str| WatchPath::project(s).unwrap();
            let mut seen = Vec::new();

            // create
            std::fs::write(project.join("design/new.bin"), b"n").unwrap();
            let created = FileWatchEvent::Created(p("design/new.bin"));
            assert!(
                collect_until(&mut watcher, &mut seen, |e| has(e, &created)),
                "create: {seen:?}"
            );

            // modify(감시 시작 전부터 있던 파일)
            std::fs::write(project.join("design/existing.bin"), b"v2").unwrap();
            let modified = FileWatchEvent::Modified(p("design/existing.bin"));
            assert!(
                collect_until(&mut watcher, &mut seen, |e| has(e, &modified)),
                "modify: {seen:?}"
            );

            // rename
            std::fs::rename(
                project.join("design/existing.bin"),
                project.join("design/renamed.bin"),
            )
            .unwrap();
            let renamed = FileWatchEvent::Renamed {
                from: p("design/existing.bin"),
                to: p("design/renamed.bin"),
            };
            assert!(
                collect_until(&mut watcher, &mut seen, |e| has(e, &renamed)),
                "rename: {seen:?}"
            );

            // delete
            std::fs::remove_file(project.join("design/new.bin")).unwrap();
            let removed = FileWatchEvent::Removed(p("design/new.bin"));
            assert!(
                collect_until(&mut watcher, &mut seen, |e| has(e, &removed)),
                "delete: {seen:?}"
            );
            // 모든 경로는 프로젝트 root 기준.
            assert!(seen
                .iter()
                .flat_map(|e| e.paths())
                .all(|p| p.root() == WatchRootId::PROJECT));
        }

        #[test]
        fn external_allowlist_targets_are_watched_and_others_are_not() {
            let dir = TempDir::new();
            let project = dir.path.join("project");
            let allowed = dir.path.join("allowed");
            let other = dir.path.join("other");
            for d in [&project, &allowed, &other] {
                std::fs::create_dir_all(d).unwrap();
            }
            std::fs::write(allowed.join("font.otf"), b"v1").unwrap();
            let mut watcher =
                FsEventsWatcher::start(&project, std::slice::from_ref(&allowed)).unwrap();
            let ext = WatchRootId(1);
            assert_eq!(watcher.watched_roots(), vec![WatchRootId::PROJECT, ext]);
            settle(&mut watcher, &allowed);
            let mut seen = Vec::new();

            std::fs::write(other.join("ignored.txt"), b"x").unwrap();
            std::fs::write(allowed.join("font.otf"), b"v2").unwrap();
            std::fs::write(allowed.join("added.png"), b"p").unwrap();
            std::fs::rename(allowed.join("added.png"), allowed.join("moved.png")).unwrap();
            std::fs::remove_file(allowed.join("moved.png")).unwrap();
            let e = |s: &str| WatchPath::new(ext, s).unwrap();
            let modified = FileWatchEvent::Modified(e("font.otf"));
            let removed = FileWatchEvent::Removed(e("moved.png"));
            assert!(
                collect_until(&mut watcher, &mut seen, |ev| has(ev, &modified)
                    && has(ev, &removed)),
                "external: {seen:?}"
            );
            // allowlist 밖 디렉터리는 감시하지 않음.
            assert!(seen
                .iter()
                .flat_map(|e| e.paths())
                .all(|p| !p.relative().contains("ignored")));
            assert!(!seen
                .iter()
                .any(|e| matches!(e, FileWatchEvent::Overflow(_))));
        }

        #[test]
        fn external_file_target_is_watched_alone() {
            let dir = TempDir::new();
            let project = dir.path.join("project");
            let fonts = dir.path.join("fonts");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::create_dir_all(&fonts).unwrap();
            std::fs::write(fonts.join("a.otf"), b"v1").unwrap();
            std::fs::write(fonts.join("b.otf"), b"v1").unwrap();
            let mut watcher = FsEventsWatcher::start(&project, &[fonts.join("a.otf")]).unwrap();
            let ext = WatchRootId(1);
            std::thread::sleep(Duration::from_millis(300));
            watcher.flush();
            let _ = watcher.drain();
            let mut seen = Vec::new();
            std::fs::write(fonts.join("b.otf"), b"v2").unwrap();
            std::fs::write(fonts.join("a.otf"), b"v2").unwrap();
            let wanted = FileWatchEvent::Modified(WatchPath::new(ext, "").unwrap());
            assert!(
                collect_until(&mut watcher, &mut seen, |e| has(e, &wanted)),
                "file root: {seen:?}"
            );
            // 형제 파일 알림은 버림(경로·넘침 모두 없음).
            assert!(seen
                .iter()
                .flat_map(|e| e.paths())
                .all(|p| p.root() == ext && p.is_root()));
            assert!(!seen
                .iter()
                .any(|e| matches!(e, FileWatchEvent::Overflow(_))));
        }
    }
}
