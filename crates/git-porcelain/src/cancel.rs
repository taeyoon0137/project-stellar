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
//!   지나도 tree가 끝나지 않으면 group 전체에 `SIGKILL`을 보냅니다. group 신호는 **leader(Git)를
//!   아직 회수(wait)하지 않은 동안에만** 보냅니다. 회수 전에는 leader의 pid가 예약되어 있어(이미
//!   끝났어도 zombie로 남음) pgid가 다른 process group에 재사용될 수 없지만, 회수 뒤에는 재사용된
//!   무관한 group에 신호가 갈 수 있기 때문입니다. 실행기는 출력 pipe가 모두 닫히기 전에는 leader를
//!   회수하지 않으므로, Git이 끝난 뒤에도 pipe를 잡고 남은 손자(hook의 백그라운드 프로세스 등)를
//!   같은 group 신호로 종료할 수 있습니다(SEC-2). 유예 시간 동안에는 pipe가 모두 닫히고 Git이 끝나기를
//!   기다리며, 그 안에 둘 다 끝나 회수되면 group에 더 이상 신호를 보내지 않습니다(`SIGTERM`을
//!   무시하고 pipe도 닫은 채 Git보다 오래 사는 자손은 이 경우 강제 종료되지 않습니다). `setsid` 등으로
//!   group을 벗어난 자손은 group 신호로 종료할 수 없습니다.
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
///
/// 호출자는 leader를 아직 회수하지 않았어야 합니다(`try_wait`가 종료 상태를 돌려준 적 없음). leader가
/// 이미 끝나 zombie여도 됩니다. `pipes_closed`는 자식 tree가 stdout·stderr·stdin pipe를 모두 닫았는지
/// 알려 주며, unix에서는 유예 시간 동안 이것이 참이 되고 leader가 끝나기를 기다립니다.
pub(crate) fn terminate_process_tree(child: &mut Child, pipes_closed: &dyn Fn() -> bool) {
    platform::terminate_process_tree(child, pipes_closed);
}

/// leader 상태 조회가 실패했을 때(회수 여부를 알 수 없음) group 신호 없이 직접 자식만 종료·회수합니다.
pub(crate) fn kill_leader_only(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
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

    pub(super) fn terminate_process_tree(child: &mut Child, pipes_closed: &dyn Fn() -> bool) {
        // 자식은 process_group(0)으로 시작했으므로 pgid == 자식 pid. 호출자 계약상 leader는 아직
        // 회수 전(실행 중이거나 zombie)이므로 group 신호가 안전합니다.
        let pgid = i32::try_from(child.id()).unwrap_or(0);
        signal_group(pgid, SIGTERM);
        let grace_deadline = Instant::now() + TERMINATE_GRACE;
        while Instant::now() < grace_deadline {
            // pipe가 모두 닫힌 뒤에만 leader를 조회(회수)합니다. pipe를 잡은 자손이 남아 있으면
            // leader를 회수하지 않아야 아래 SIGKILL을 group에 보낼 수 있습니다.
            if pipes_closed() {
                match child.try_wait() {
                    // tree가 끝나 회수함: 회수 뒤에는 group에 신호를 보내지 않습니다.
                    Ok(Some(_)) => return,
                    Ok(None) => {}
                    // 회수 여부를 알 수 없음: group 신호 없이 직접 자식만 정리합니다.
                    Err(_) => return super::kill_leader_only(child),
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        // 유예 시간이 지나도 leader가 살아 있거나 pipe를 잡은 자손이 남음(아직 회수 전): group 전체를
        // 강제 종료합니다. 이미 끝난 구성원(zombie leader 포함)에는 영향이 없습니다.
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

    #[cfg(test)]
    mod terminate_tests {
        use super::*;
        use std::os::unix::process::CommandExt;

        #[test]
        fn exited_leader_with_closed_pipes_is_reaped_without_waiting_for_grace() {
            // 이미 끝난(회수 전 zombie) leader: pipe가 닫혀 있으면 유예 시간을 기다리지 않고 회수합니다.
            let mut child = std::process::Command::new("/usr/bin/true")
                .process_group(0)
                .spawn()
                .unwrap();
            thread::sleep(Duration::from_millis(100));
            let start = Instant::now();
            terminate_process_tree(&mut child, &|| true);
            assert!(start.elapsed() < TERMINATE_GRACE, "{:?}", start.elapsed());
            assert!(child.try_wait().unwrap().is_some());
        }

        #[test]
        fn sigterm_ignoring_leader_is_killed_after_grace() {
            let mut child = std::process::Command::new("/bin/sh")
                .args(["-c", "trap '' TERM; sleep 30"])
                .process_group(0)
                .spawn()
                .unwrap();
            thread::sleep(Duration::from_millis(100));
            let start = Instant::now();
            terminate_process_tree(&mut child, &|| true);
            let elapsed = start.elapsed();
            assert!(elapsed >= TERMINATE_GRACE, "{elapsed:?}");
            assert!(elapsed < TERMINATE_GRACE * 3, "{elapsed:?}");
            assert!(child.try_wait().unwrap().is_some());
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::process::{Child, Command, Stdio};

    pub(super) fn terminate_process_tree(child: &mut Child, _pipes_closed: &dyn Fn() -> bool) {
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

    pub(super) fn terminate_process_tree(child: &mut Child, _pipes_closed: &dyn Fn() -> bool) {
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
