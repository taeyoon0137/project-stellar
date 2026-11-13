//! Git 명령 취소와 프로세스 tree 종료 (XIV.1.4).
//!
//! [`GitCancelToken`]은 다른 스레드(UI 등)에서 실행 중인 Git 명령의 취소를 요청하는 공유
//! 플래그입니다. [`GitRunner::run_cancellable`](crate::runner::GitRunner::run_cancellable)은
//! 취소를 관찰하면 Git과 **Git이 만든 자식 프로세스 tree 전체**를 종료하고
//! [`GitRunError::Cancelled`](crate::runner::GitRunError::Cancelled)를 반환합니다. 취소된
//! 실행의 부분 stdout/stderr는 성공 결과로 돌려주지 않습니다.
//!
//! 플랫폼별 tree 종료:
//! - **unix**: Git을 새 process group의 leader로 시작하고(`process_group(0)`), 종료 시 group 전체에
//!   먼저 `SIGTERM`을 보내 Git이 lock 파일(`index.lock` 등)을 정리할 기회를 준 뒤, 유예 시간이
//!   지나면 group 전체에 `SIGKILL`을 보냅니다. 유예 시간 안에 Git이 끝나도 group에 남은 자식을
//!   정리하기 위해 `SIGKILL`은 항상 보냅니다.
//! - **Windows**: `taskkill /PID <pid> /T /F`(argv, shell 없음)로 자식 tree까지 강제 종료합니다.
//! - 그 밖의 플랫폼: 직접 자식만 종료합니다.

use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 취소 요청 공유 플래그. `Clone`한 사본은 같은 플래그를 공유합니다.
#[derive(Debug, Clone, Default)]
pub struct GitCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl GitCancelToken {
    /// 취소되지 않은 새 token.
    pub fn new() -> GitCancelToken {
        GitCancelToken::default()
    }

    /// 취소를 요청합니다(여러 번 호출해도 같음).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 취소가 요청되었는지.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// `SIGTERM` 후 `SIGKILL`까지의 유예 시간(unix).
pub const TERMINATE_GRACE: Duration = Duration::from_millis(1_500);

/// 자식 프로세스 tree를 종료하고 직접 자식을 회수(wait)합니다.
pub(crate) fn terminate_process_tree(child: &mut Child) {
    platform::terminate_process_tree(child);
}

#[cfg(unix)]
mod platform {
    use super::TERMINATE_GRACE;
    use std::process::Child;
    use std::thread;
    use std::time::{Duration, Instant};

    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    const SIGKILL: i32 = 9;
    const SIGTERM: i32 = 15;

    /// process group 전체에 신호를 보냅니다. `pgid <= 1`이면 보내지 않습니다.
    fn signal_group(pgid: i32, signal: i32) {
        if pgid <= 1 {
            return;
        }
        // SAFETY: kill(2)는 정수 인자만 받고 메모리를 다루지 않는 시스템 호출입니다. pgid > 1을
        // 보장하므로 음수 pid는 자식이 만든 process group만 가리키며, 모든 프로세스(-1)나 호출자
        // 자신의 group(0)에는 신호를 보내지 않습니다.
        let _ = unsafe { kill(-pgid, signal) };
    }

    pub(super) fn terminate_process_tree(child: &mut Child) {
        // 자식은 process_group(0)으로 시작했으므로 pgid == 자식 pid.
        let pgid = i32::try_from(child.id()).unwrap_or(0);
        signal_group(pgid, SIGTERM);
        let grace_deadline = Instant::now() + TERMINATE_GRACE;
        loop {
            match child.try_wait() {
                Ok(None) if Instant::now() < grace_deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                _ => break,
            }
        }
        // group에 남은 프로세스(Git이 끝난 뒤의 손자 포함)를 강제 종료.
        signal_group(pgid, SIGKILL);
        let _ = child.kill();
        let _ = child.wait();
    }

    /// 테스트용: pid가 아직 존재하는지(`kill(pid, 0)`).
    #[cfg(test)]
    pub(crate) fn process_exists(pid: i32) -> bool {
        if pid <= 1 {
            return false;
        }
        // SAFETY: 신호 0은 존재·권한 검사만 하고 아무 신호도 보내지 않습니다.
        unsafe { kill(pid, 0) == 0 }
    }
}

#[cfg(windows)]
mod platform {
    use std::process::{Child, Command, Stdio};

    pub(super) fn terminate_process_tree(child: &mut Child) {
        let pid = child.id().to_string();
        // /T: 자식 tree 포함, /F: 강제 종료. argv 방식으로 실행합니다(shell 없음).
        let _ = Command::new("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::process::Child;

    pub(super) fn terminate_process_tree(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(all(test, unix))]
pub(crate) use platform::process_exists;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_shared_between_clones() {
        let token = GitCancelToken::new();
        let copy = token.clone();
        assert!(!token.is_cancelled());
        copy.cancel();
        assert!(token.is_cancelled());
        token.cancel();
        assert!(copy.is_cancelled());
    }

    #[test]
    fn already_cancelled_request_spawns_nothing() {
        use crate::runner::{argv, GitRunError, GitRunner};
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let token = GitCancelToken::new();
        token.cancel();
        assert_eq!(
            runner.run_cancellable(&argv(["status"]), &std::env::temp_dir(), &token),
            Err(GitRunError::Cancelled)
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[test]
    fn untrusted_cancellable_request_spawns_nothing() {
        use crate::runner::{argv, GitRunner};
        use crate::trust::{GatedGitRunner, GatedRunError, GitTrust};
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        assert_eq!(
            gate.run_cancellable(
                &argv(["status"]),
                &std::env::temp_dir(),
                &GitCancelToken::new()
            ),
            Err(GatedRunError::ProjectNotTrusted)
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod process_tree {
        use super::*;
        use crate::runner::{argv, GitRunError, GitRunner};
        use crate::test_support::{
            hermetic_runner, hermetic_runner_with_timeout, system_git, TempDir,
        };
        use std::ffi::OsString;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::thread;
        use std::time::Instant;

        /// pid 4개(script, sleep, 중첩 sh, 중첩 sleep)를 기록하고 오래 대기하는 자식 tree script.
        fn write_tree_script(dir: &Path) -> PathBuf {
            let script = dir.join("longrun.sh");
            let body = "#!/bin/sh\n\
                        printf 'partial-output\\n'\n\
                        echo $$ >> \"$1\"\n\
                        sleep 30 &\n\
                        echo $! >> \"$1\"\n\
                        sh -c 'sleep 30 & echo $! >> \"$0\"; wait' \"$1\" &\n\
                        echo $! >> \"$1\"\n\
                        wait\n";
            std::fs::write(&script, body).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }

        /// alias로 자식 tree script를 실행하는 장기 실행 Git 요청 argv.
        fn long_running_git_args(script: &Path, pidfile: &Path) -> Vec<OsString> {
            let alias = format!("alias.longrun=!{} {}", script.display(), pidfile.display());
            argv(["-c", alias.as_str(), "longrun"])
        }

        fn read_pids(pidfile: &Path) -> Vec<i32> {
            std::fs::read_to_string(pidfile)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect()
        }

        fn wait_for_pids(pidfile: &Path, count: usize, timeout: Duration) -> Vec<i32> {
            let deadline = Instant::now() + timeout;
            loop {
                let pids = read_pids(pidfile);
                if pids.len() >= count || Instant::now() >= deadline {
                    return pids;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        /// 제한 시간 안에 사라지지 않은 pid 목록.
        fn survivors_after(pids: &[i32], timeout: Duration) -> Vec<i32> {
            let deadline = Instant::now() + timeout;
            loop {
                let alive: Vec<i32> = pids
                    .iter()
                    .copied()
                    .filter(|pid| process_exists(*pid))
                    .collect();
                if alive.is_empty() || Instant::now() >= deadline {
                    return alive;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }

        #[test]
        fn cancel_terminates_whole_process_tree_and_is_not_success() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("cancel-tree");
            let script = write_tree_script(dir.path());
            let pidfile = dir.path().join("pids");
            let runner = hermetic_runner(git);
            let args = long_running_git_args(&script, &pidfile);

            let token = GitCancelToken::new();
            let canceller = {
                let token = token.clone();
                let pidfile = pidfile.clone();
                thread::spawn(move || {
                    let pids = wait_for_pids(&pidfile, 4, Duration::from_secs(10));
                    token.cancel();
                    pids
                })
            };
            let start = Instant::now();
            let result = runner.run_cancellable(&args, dir.path(), &token);
            let pids = canceller.join().unwrap();

            // 부분 출력(partial-output)이 있어도 성공으로 반환하지 않음.
            assert_eq!(result, Err(GitRunError::Cancelled));
            assert!(start.elapsed() < Duration::from_secs(10), "취소가 늦음");
            assert_eq!(
                pids.len(),
                4,
                "자식 tree가 기대대로 만들어지지 않음: {pids:?}"
            );
            // 핵심: 취소 후 자식 프로세스 tree 잔존 0.
            let survivors = survivors_after(&pids, Duration::from_secs(5));
            assert!(survivors.is_empty(), "잔존 프로세스: {survivors:?}");
        }

        #[test]
        fn timeout_also_terminates_tree_without_waiting_for_grandchildren() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("timeout-tree");
            let script = write_tree_script(dir.path());
            let pidfile = dir.path().join("pids");
            let runner = hermetic_runner_with_timeout(git, Duration::from_millis(1_000));
            let args = long_running_git_args(&script, &pidfile);

            let start = Instant::now();
            let result = runner.run(&args, dir.path());
            assert_eq!(result, Err(GitRunError::Timeout));
            // 손자가 stdout 파이프를 잡고 있어도 30초를 기다리지 않음.
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timeout 처리가 늦음"
            );
            let pids = read_pids(&pidfile);
            assert_eq!(
                pids.len(),
                4,
                "자식 tree가 기대대로 만들어지지 않음: {pids:?}"
            );
            let survivors = survivors_after(&pids, Duration::from_secs(5));
            assert!(survivors.is_empty(), "잔존 프로세스: {survivors:?}");
        }

        #[test]
        fn uncancelled_cancellable_run_returns_normal_result() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("cancel-normal");
            let runner: GitRunner = hermetic_runner(git);
            let out = runner
                .run_cancellable(&argv(["--version"]), dir.path(), &GitCancelToken::new())
                .expect("정상 실행");
            assert!(out.success());
            assert!(out.stdout.starts_with(b"git version"));
        }
    }
}
