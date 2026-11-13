//! thread 역할 표시 (XI.7.1).
//!
//! 앱 thread 모델(아키텍처 §9.2: Native UI, Document, Render, Worker Pool)에서 현재 thread의
//! 역할을 thread-local로 기록합니다. 이미지 decode 같은 worker 전용 작업은 이 역할을 확인해
//! UI·Document·Render thread에서의 실행을 거부합니다.
//!
//! 역할은 thread 시작 시 한 번 bind하고 [`ThreadRoleGuard`]가 drop될 때 해제됩니다. 이미 역할이
//! 있는 thread에 다시 bind하면 거부하므로, 예컨대 render thread가 스스로 worker로 바꿀 수
//! 없습니다. guard는 `!Send`라 bind한 thread 밖으로 옮겨 drop할 수 없습니다.
//!
//! ## 역할 pin
//!
//! 역할 전용 권한 표식(예: 이미지 decode의 `DecodeWorkerScope`)은 만들 때 현재 역할을
//! **pin**합니다. pin이 하나라도 남아 있으면 guard가 drop되어도 역할 해제를 마지막 pin이 drop될
//! 때까지 미룹니다. 따라서 worker에서 얻은 표식을 들고 guard를 놓은 뒤 같은 thread를 UI·Document·
//! Render로 다시 bind할 수 없습니다(그 동안 bind는 `AlreadyBound`). 표식이 살아 있는 동안 thread
//! 역할은 항상 표식을 만든 역할입니다. pin을 `mem::forget`하면 역할이 영구히 남는 쪽(fail
//! closed)으로 실패합니다.

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
    /// 현재 역할을 pin한 표식 수.
    static ROLE_PINS: Cell<usize> = const { Cell::new(0) };
    /// guard가 drop됐지만 pin 때문에 해제를 미룬 상태.
    static RELEASE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// 역할 bind guard. drop 시 현재 thread의 역할을 해제합니다. 역할을 pin한 표식이 남아 있으면
/// 해제를 마지막 pin이 drop될 때까지 미룹니다.
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
        let pinned = ROLE_PINS.try_with(Cell::get).unwrap_or(0) > 0;
        if pinned {
            // 남은 pin이 모두 drop될 때 해제합니다.
            let _ = RELEASE_PENDING.try_with(|cell| cell.set(true));
        } else {
            let _ = CURRENT_ROLE.try_with(|cell| cell.set(None));
        }
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
///
/// guard가 drop됐어도 역할을 pin한 표식이 남아 있으면 그 역할을 반환합니다.
pub fn current_thread_role() -> Option<ThreadRole> {
    CURRENT_ROLE.try_with(Cell::get).ok().flatten()
}

/// 현재 역할 pin. drop 시 pin을 풀고, 마지막 pin이면 미뤄 둔 역할 해제를 수행합니다.
/// `!Send`라 pin한 thread 밖으로 옮길 수 없습니다.
#[derive(Debug)]
pub(crate) struct RolePin {
    _not_send: PhantomData<*const ()>,
}

impl Drop for RolePin {
    fn drop(&mut self) {
        let remaining = ROLE_PINS
            .try_with(|cell| {
                let remaining = cell.get().saturating_sub(1);
                cell.set(remaining);
                remaining
            })
            .unwrap_or(0);
        if remaining == 0 && RELEASE_PENDING.try_with(Cell::get).unwrap_or(false) {
            let _ = RELEASE_PENDING.try_with(|cell| cell.set(false));
            let _ = CURRENT_ROLE.try_with(|cell| cell.set(None));
        }
    }
}

/// 현재 thread 역할이 `expected`면 pin합니다. 아니면 현재 역할을 `Err`로 돌려줍니다.
pub(crate) fn pin_current_role(expected: ThreadRole) -> Result<RolePin, Option<ThreadRole>> {
    match current_thread_role() {
        Some(role) if role == expected => {
            ROLE_PINS
                .try_with(|cell| cell.set(cell.get().saturating_add(1)))
                .map_err(|_| Some(role))?;
            Ok(RolePin {
                _not_send: PhantomData,
            })
        }
        other => Err(other),
    }
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
    fn pinned_role_outlives_guard_until_last_pin_drops() {
        thread::spawn(|| {
            let guard = bind_thread_role(ThreadRole::Worker).unwrap();
            let first = pin_current_role(ThreadRole::Worker).unwrap();
            let second = pin_current_role(ThreadRole::Worker).unwrap();
            drop(guard);
            // pin이 남아 있으면 역할이 유지되고 다른 역할로 bind할 수 없습니다.
            assert_eq!(current_thread_role(), Some(ThreadRole::Worker));
            assert_eq!(
                bind_thread_role(ThreadRole::Render).unwrap_err(),
                ThreadRoleError::AlreadyBound {
                    current: ThreadRole::Worker
                }
            );
            drop(first);
            assert_eq!(current_thread_role(), Some(ThreadRole::Worker));
            drop(second);
            // 마지막 pin drop에서 미뤄 둔 해제가 일어납니다.
            assert_eq!(current_thread_role(), None);
            let render = bind_thread_role(ThreadRole::Render).unwrap();
            assert_eq!(
                pin_current_role(ThreadRole::Worker).unwrap_err(),
                Some(ThreadRole::Render)
            );
            drop(render);
            assert_eq!(current_thread_role(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn pin_requires_matching_role_and_unpinned_guard_releases_immediately() {
        thread::spawn(|| {
            assert_eq!(pin_current_role(ThreadRole::Worker).unwrap_err(), None);
            let guard = bind_thread_role(ThreadRole::Worker).unwrap();
            {
                let _pin = pin_current_role(ThreadRole::Worker).unwrap();
            }
            // guard가 살아 있는 동안 pin이 풀려도 역할은 유지됩니다.
            assert_eq!(current_thread_role(), Some(ThreadRole::Worker));
            drop(guard);
            // pin이 없으면 guard drop에서 즉시 해제됩니다.
            assert_eq!(current_thread_role(), None);
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
