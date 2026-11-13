//! 테스트 공용 도구.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::{Duration, Instant};

/// 실제 PTY를 여는 테스트의 공유/배타 lock. fd 누수 검사처럼 열린 tty 수를 세는 테스트만 배타로
/// 잡고, 나머지 PTY 테스트는 공유로 잡아 서로 병렬로 돕니다.
static PTY_TESTS: RwLock<()> = RwLock::new(());

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn pty_shared() -> RwLockReadGuard<'static, ()> {
    PTY_TESTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn pty_exclusive() -> RwLockWriteGuard<'static, ()> {
    PTY_TESTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 테스트가 끝나면 지우는 임시 디렉터리. 경로는 canonical(symlink 해석)입니다.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-terminal-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let path = std::fs::canonicalize(&path).unwrap();
        TempDir { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 하위 디렉터리를 만들고 canonical 경로를 돌려줍니다.
    pub(crate) fn mkdir(&self, name: &str) -> PathBuf {
        let path = self.path.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(path).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 터미널 출력 reader를 별도 thread에서 읽어 모으는 도구(읽기가 막혀도 테스트가 timeout으로 끝남).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct OutputCollector {
    rx: Receiver<Option<Vec<u8>>>,
    buffer: Vec<u8>,
    eof: bool,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl OutputCollector {
    pub(crate) fn start(mut reader: Box<dyn Read + Send>) -> OutputCollector {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(None);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(Some(chunk[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        OutputCollector {
            rx,
            buffer: Vec::new(),
            eof: false,
        }
    }

    /// 지금까지 모은 출력.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer).into_owned()
    }

    fn receive(&mut self, deadline: Instant) -> bool {
        let now = Instant::now();
        if self.eof || now >= deadline {
            return false;
        }
        match self.rx.recv_timeout(deadline - now) {
            Ok(Some(chunk)) => {
                self.buffer.extend_from_slice(&chunk);
                true
            }
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                false
            }
            Err(RecvTimeoutError::Timeout) => false,
        }
    }

    /// 출력에 `needle`이 나올 때까지 기다립니다.
    pub(crate) fn wait_for(&mut self, needle: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        while !self.text().contains(needle) {
            if !self.receive(deadline) {
                panic!("출력에 {needle:?}가 없습니다: {:?}", self.text());
            }
        }
        self.text()
    }

    /// EOF까지 기다려 전체 출력을 돌려줍니다.
    pub(crate) fn wait_eof(mut self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        while !self.eof {
            if !self.receive(deadline) && !self.eof {
                panic!("출력 EOF를 기다리다 시간이 지났습니다: {:?}", self.text());
            }
        }
        self.text()
    }
}
