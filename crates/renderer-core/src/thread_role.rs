//! thread 역할 표시 (XI.7.1).
//!
//! 앱 thread 모델(아키텍처 §9.2: Native UI, Document, Render, Worker Pool)에서 현재 thread의
//! 역할을 thread-local로 기록합니다. 이미지 decode 같은 worker 전용 작업은 이 역할을 확인해
//! UI·Document·Render thread에서의 실행을 거부합니다.
//!
//! 역할은 thread 시작 시 한 번 bind하고 [`ThreadRoleGuard`]가 drop될 때 해제됩니다. 이미 역할이
//! 있는 thread에 다시 bind하면 거부하므로, 예컨대 render thread가 스스로 worker로 바꿀 수
//! 없습니다. guard는 `!Send`라 bind한 thread 밖으로 옮겨 drop할 수 없습니다.

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;

/// 앱 thread 역할.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadRole {
    /// Native UI thread.
    Ui,
    /// 유일한 문서 작성자 thread.
    Document,
    /// Scene snapshot을 읽어 GPU 명령을 만드는 thread.
    Render,
    /// worker pool thread(text shaping, tessellation, 이미지 decode 등).
    Worker,
}

/// 역할 bind 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRoleError {
    /// 현재 thread에 이미 역할이 있습니다.
    AlreadyBound {
        /// 현재 역할.
        current: ThreadRole,
    },
}

impl fmt::Display for ThreadRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThreadRoleError::AlreadyBound { current } => {
                write!(f, "현재 thread에 이미 역할이 있습니다: {current:?}")
            }
        }
    }
}

impl std::error::Error for ThreadRoleError {}

thread_local! {
    static CURRENT_ROLE: Cell<Option<ThreadRole>> = const { Cell::new(None) };
}

/// 역할 bind guard. drop 시 현재 thread의 역할을 해제합니다.
#[derive(Debug)]
pub struct ThreadRoleGuard {
    role: ThreadRole,
    _not_send: PhantomData<*const ()>,
}

impl ThreadRoleGuard {
    /// bind한 역할.
    pub fn role(&self) -> ThreadRole {
        self.role
    }
}

impl Drop for ThreadRoleGuard {
    fn drop(&mut self) {
        // thread 종료 중 TLS가 이미 해제됐다면 할 일이 없습니다.
        let _ = CURRENT_ROLE.try_with(|cell| cell.set(None));
    }
}

/// 현재 thread에 역할을 bind합니다. 이미 역할이 있으면 거부합니다.
pub fn bind_thread_role(role: ThreadRole) -> Result<ThreadRoleGuard, ThreadRoleError> {
    CURRENT_ROLE.with(|cell| match cell.get() {
        Some(current) => Err(ThreadRoleError::AlreadyBound { current }),
        None => {
            cell.set(Some(role));
            Ok(ThreadRoleGuard {
                role,
                _not_send: PhantomData,
            })
        }
    })
}

/// 현재 thread의 역할. bind되지 않았으면 `None`.
pub fn current_thread_role() -> Option<ThreadRole> {
    CURRENT_ROLE.try_with(Cell::get).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn bind_once_and_release_on_drop() {
        thread::spawn(|| {
            assert_eq!(current_thread_role(), None);
            let guard = bind_thread_role(ThreadRole::Render).unwrap();
            assert_eq!(guard.role(), ThreadRole::Render);
            assert_eq!(current_thread_role(), Some(ThreadRole::Render));
            // render thread가 worker로 위장할 수 없습니다.
            assert_eq!(
                bind_thread_role(ThreadRole::Worker).unwrap_err(),
                ThreadRoleError::AlreadyBound {
                    current: ThreadRole::Render
                }
            );
            drop(guard);
            assert_eq!(current_thread_role(), None);
            let _worker = bind_thread_role(ThreadRole::Worker).unwrap();
            assert_eq!(current_thread_role(), Some(ThreadRole::Worker));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn roles_are_per_thread() {
        thread::spawn(|| {
            let _guard = bind_thread_role(ThreadRole::Ui).unwrap();
            let other = thread::spawn(current_thread_role).join().unwrap();
            assert_eq!(other, None);
            assert_eq!(current_thread_role(), Some(ThreadRole::Ui));
        })
        .join()
        .unwrap();
    }
}
