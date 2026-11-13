//! 보안 Git 실행기 (IV.3.3).
//!
//! 모든 Git 실행은 [`GitRunner`]를 거칩니다. 핵심 보안·결정성 속성:
//! - **argv 배열**: 인자를 배열로 전달하고 **shell을 거치지 않습니다**. metacharacter
//!   (`;`, `&&`, `$(...)`, `|`, `` ` ``)는 명령으로 해석되지 않고 리터럴 인자로 전달됩니다.
//! - **cwd**: 작업 디렉터리를 명시합니다(상속에 의존하지 않음).
//! - **환경 allowlist**: `env_clear` 후 허용 목록의 변수만 전달합니다.
//! - **timeout**: 제한 시간을 넘으면 자식과 자식 프로세스 tree를 종료합니다.
//! - **취소**: [`GitRunner::run_cancellable`]은 [`GitCancelToken`] 취소를 관찰하면 프로세스 tree를
//!   종료하고 [`GitRunError::Cancelled`]를 반환합니다(XIV.1.4). 부분 출력은 성공으로 돌려주지
//!   않습니다.
//! - **byte capture**: stdout/stderr를 원시 바이트로 캡처합니다(문자열 가정 없음).
//! - **stdin 입력**: [`GitRunner::run_with_input`]은 바이트를 자식 stdin으로 전달합니다(XIV.8.2).
//!   commit message처럼 사용자 텍스트를 argv·shell escaping 없이 넘길 때 사용합니다. 그 밖의
//!   실행은 stdin을 비워(null) 둡니다.
//! - **spawn 계측**: 실제 프로세스 생성 시도 횟수를 [`GitRunner::spawn_count`]로 셉니다(XIV.1.1).
//!   trust gate가 프로세스를 하나도 만들지 않았음을 검증하는 데 사용합니다.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::cancel::{terminate_process_tree, GitCancelToken};

/// stderr 조각을 받는 콜백.
type StderrSink<'a> = &'a mut dyn FnMut(&[u8]);

/// Git 실행 결과(원시 바이트 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    /// 종료 코드(시그널 종료면 `None`).
    pub exit_code: Option<i32>,
    /// stdout 원시 바이트.
    pub stdout: Vec<u8>,
    /// stderr 원시 바이트.
    pub stderr: Vec<u8>,
}

impl GitOutput {
    /// 종료 코드가 0이면 성공.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// Git 실행 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRunError {
    /// 프로세스 생성 실패(executable 없음·권한 등).
    Spawn {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// 제한 시간 초과(자식 프로세스 tree를 종료함).
    Timeout,
    /// 종료 대기 중 io 오류.
    Wait {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// 취소 요청으로 중단함(자식 프로세스 tree를 종료함, XIV.1.4). 부분 출력은 버립니다.
    ///
    /// Git이 중단 전에 저장소를 일부 변경했을 수 있으므로, 호출자는 이 결과를 성공으로 취급하지
    /// 말고 필요하면 저장소 상태를 다시 조회해야 합니다.
    Cancelled,
}

impl std::fmt::Display for GitRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitRunError::Spawn { kind } => write!(f, "Git 프로세스 생성 실패: {kind:?}"),
            GitRunError::Timeout => f.write_str("Git 실행 제한 시간 초과"),
            GitRunError::Wait { kind } => write!(f, "Git 종료 대기 오류: {kind:?}"),
            GitRunError::Cancelled => f.write_str("Git 실행이 취소되었습니다"),
        }
    }
}

impl std::error::Error for GitRunError {}

/// argv·cwd·env allowlist·timeout으로 Git을 실행하는 실행기.
///
/// `Clone`한 실행기는 spawn 계측 카운터를 **공유**합니다(같은 실행기의 사본으로 취급).
#[derive(Debug, Clone)]
pub struct GitRunner {
    git_path: PathBuf,
    env_allowlist: Vec<OsString>,
    env_overrides: Vec<(OsString, OsString)>,
    timeout: Duration,
    spawn_count: Arc<AtomicU64>,
}

impl GitRunner {
    /// Git 경로·환경 allowlist·timeout으로 실행기를 만듭니다.
    pub fn new(git_path: PathBuf, env_allowlist: Vec<OsString>, timeout: Duration) -> GitRunner {
        GitRunner {
            git_path,
            env_allowlist,
            env_overrides: Vec::new(),
            timeout,
            spawn_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 지금까지 이 실행기(및 사본)가 시도한 프로세스 생성 횟수(XIV.1.1 계측).
    ///
    /// spawn이 실패한 시도도 셉니다. 실행 전에 거부된 요청(trust gate 등)은 세지 않습니다.
    pub fn spawn_count(&self) -> u64 {
        self.spawn_count.load(Ordering::SeqCst)
    }

    /// 자식에 설정할 명시적 환경 변수(allowlist 이후 적용). Git 동작 제어(`GIT_CONFIG_GLOBAL`,
    /// `GIT_TERMINAL_PROMPT=0` 등)에 사용합니다. `env_clear` 기반 보안은 유지됩니다.
    pub fn with_env_overrides(mut self, overrides: Vec<(OsString, OsString)>) -> GitRunner {
        self.env_overrides = overrides;
        self
    }

    /// 기존 override를 유지한 채 override를 **덧붙입니다**(같은 이름이면 나중 값이 이깁니다).
    pub fn with_added_env(mut self, extra: Vec<(OsString, OsString)>) -> GitRunner {
        self.env_overrides.extend(extra);
        self
    }

    /// 환경 allowlist에 이름을 **덧붙입니다**(이미 있으면 그대로). 값은 실행 시 현재 프로세스에서 읽습니다.
    pub fn with_allowed_env(mut self, names: Vec<OsString>) -> GitRunner {
        for name in names {
            if !self.env_allowlist.contains(&name) {
                self.env_allowlist.push(name);
            }
        }
        self
    }

    /// 자식 프로세스가 실제로 보게 될 환경 변수 값입니다(override 우선, 그다음 allowlist에 있는
    /// 현재 프로세스 값). allowlist에도 override에도 없으면 자식은 그 변수를 보지 못하므로 `None`.
    pub fn child_env_var(&self, name: &str) -> Option<OsString> {
        if let Some((_, value)) = self
            .env_overrides
            .iter()
            .rev()
            .find(|(key, _)| key.as_os_str() == OsStr::new(name))
        {
            return Some(value.clone());
        }
        if self
            .env_allowlist
            .iter()
            .any(|key| key.as_os_str() == OsStr::new(name))
        {
            return std::env::var_os(name);
        }
        None
    }

    /// 저장소를 **발견하지 않도록** 고정한 사본을 만듭니다(spawn 계측 카운터 공유).
    ///
    /// 중립 디렉터리에서 실행하는 환경 probe(버전·capability·LFS)용입니다. `GIT_DIR`을 저장소가
    /// 될 수 없는 null device로 지정해 Git의 상위 디렉터리 탐색 자체를 끄고, 그 위에
    /// `GIT_CEILING_DIRECTORIES`를 `neutral_cwd`로 지정해 상위 탐색 경계를 한 번 더 둡니다. 그래서
    /// 임시 디렉터리가 우연히 저장소 안에 있어도 그 저장소의 config(alias·fsmonitor 등)를 읽지
    /// 않습니다.
    pub fn isolated_from_repositories(&self, neutral_cwd: &Path) -> GitRunner {
        self.clone().with_added_env(vec![
            (OsString::from("GIT_DIR"), OsString::from(NULL_DEVICE)),
            (
                OsString::from("GIT_CEILING_DIRECTORIES"),
                neutral_cwd.as_os_str().to_os_string(),
            ),
        ])
    }

    /// 주어진 argv와 cwd로 Git을 실행합니다(shell 미사용). stdout/stderr를 캡처합니다.
    pub fn run(&self, args: &[OsString], cwd: &Path) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, None, None)
    }

    /// `input` 바이트를 자식 stdin으로 전달하며 실행합니다(XIV.8.2). 전달 후 stdin을 닫습니다.
    ///
    /// 입력은 argv나 shell을 거치지 않으므로 quote·escape가 필요 없습니다.
    pub fn run_with_input(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, Some(input), None, None)
    }

    /// 취소 가능한 실행(XIV.1.4). `cancel`이 취소되면 프로세스 tree를 종료하고
    /// [`GitRunError::Cancelled`]를 반환합니다. 실행 전에 이미 취소되어 있으면 프로세스를 만들지
    /// 않습니다. Git이 취소 관찰 전에 스스로 끝났다면 그 결과를 그대로 반환합니다.
    pub fn run_cancellable(
        &self,
        args: &[OsString],
        cwd: &Path,
        cancel: &GitCancelToken,
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, Some(cancel), None)
    }

    /// 취소 가능하게 실행하며 stderr 조각을 도착하는 대로 `on_stderr`에 전달합니다(XXVI.1.2 진행률).
    ///
    /// 콜백은 호출 스레드에서 호출됩니다. 완료 결과의 `stderr`에도 같은 바이트 전체가 담깁니다.
    /// 취소·timeout이면 [`GitRunner::run_cancellable`]과 같이 tree를 종료하고 오류를 반환합니다.
    pub fn run_streaming_stderr(
        &self,
        args: &[OsString],
        cwd: &Path,
        cancel: &GitCancelToken,
        on_stderr: &mut dyn FnMut(&[u8]),
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, Some(cancel), Some(on_stderr))
    }

    fn run_inner(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: Option<&[u8]>,
        cancel: Option<&GitCancelToken>,
        mut on_stderr: Option<StderrSink<'_>>,
    ) -> Result<GitOutput, GitRunError> {
        let cancelled = || cancel.is_some_and(GitCancelToken::is_cancelled);
        if cancelled() {
            return Err(GitRunError::Cancelled);
        }

        let mut command = Command::new(&self.git_path);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // 환경 allowlist: 전부 지우고 허용 변수만 전달.
        command.env_clear();
        for name in &self.env_allowlist {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        // 명시적 override(allowlist 이후 적용).
        for (key, value) in &self.env_overrides {
            command.env(key, value);
        }
        // 자식을 새 process group의 leader로 시작해 tree 전체를 종료할 수 있게 합니다.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        let mut child = command
            .spawn()
            .map_err(|e| GitRunError::Spawn { kind: e.kind() })?;

        // 파이프 버퍼가 가득 차 자식이 막히지 않도록 별도 스레드로 동시 drain.
        let mut stdout_pipe = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        let out_thread = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        });
        // streaming이면 stderr 조각을 channel로도 보냅니다(호출 스레드가 콜백 실행).
        let (chunk_tx, chunk_rx) = match on_stderr {
            Some(_) => {
                let (tx, rx) = mpsc::channel::<Vec<u8>>();
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };
        let err_thread = thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(tx) = &chunk_tx {
                            let _ = tx.send(chunk[..n].to_vec());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            buf
        });
        let mut forward = |rx: &Option<mpsc::Receiver<Vec<u8>>>| {
            if let (Some(rx), Some(cb)) = (rx, on_stderr.as_mut()) {
                while let Ok(piece) = rx.try_recv() {
                    cb(&piece);
                }
            }
        };
        // stdin 입력은 별도 스레드로 쓰고 닫습니다(자식이 출력하며 읽는 경우의 교착 방지).
        // 자식이 다 읽지 않고 끝나면 쓰기 오류(BrokenPipe)가 나지만 결과는 종료 코드로 판단합니다.
        let in_thread = match (input, child.stdin.take()) {
            (Some(bytes), Some(mut stdin_pipe)) => {
                let bytes = bytes.to_vec();
                Some(thread::spawn(move || {
                    let _ = stdin_pipe.write_all(&bytes);
                }))
            }
            _ => None,
        };

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            let interrupted = match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if cancelled() => GitRunError::Cancelled,
                Ok(None) if Instant::now() >= deadline => GitRunError::Timeout,
                Ok(None) => {
                    forward(&chunk_rx);
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => GitRunError::Wait { kind: e.kind() },
            };
            // 중단: tree 종료 후 파이프가 닫히면 drain 스레드도 끝납니다. 부분 출력은 버립니다.
            terminate_process_tree(&mut child);
            if let Some(handle) = in_thread {
                let _ = handle.join();
            }
            let _ = out_thread.join();
            let _ = err_thread.join();
            return Err(interrupted);
        };

        if let Some(handle) = in_thread {
            let _ = handle.join();
        }

        let stdout = out_thread.join().unwrap_or_default();
        let stderr = err_thread.join().unwrap_or_default();
        // drain 스레드가 끝난 뒤 남은 조각까지 전달합니다.
        forward(&chunk_rx);
        Ok(GitOutput {
            exit_code: status.code(),
            stdout,
            stderr,
        })
    }
}

/// 저장소가 될 수 없는 null device 경로(저장소 탐색 차단용 `GIT_DIR` 값).
#[cfg(unix)]
pub const NULL_DEVICE: &str = "/dev/null";

/// 저장소가 될 수 없는 null device 경로(저장소 탐색 차단용 `GIT_DIR` 값).
#[cfg(not(unix))]
pub const NULL_DEVICE: &str = "NUL";

/// `&[&str]`을 argv(`Vec<OsString>`)로 만드는 헬퍼.
pub fn argv<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|s| s.as_ref().to_os_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-runner-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn runner_for(path: &str, timeout_ms: u64) -> GitRunner {
        GitRunner::new(
            PathBuf::from(path),
            Vec::new(),
            Duration::from_millis(timeout_ms),
        )
    }

    #[cfg(unix)]
    #[test]
    fn runs_and_captures_stdout() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        let out = runner.run(&argv(["hello world"]), &dir.path).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, b"hello world\n");
    }

    #[cfg(unix)]
    #[test]
    fn shell_metacharacters_are_not_interpreted() {
        // 보안: metacharacter 인자가 shell로 해석되면 파일이 생성됨. argv라 생성되지 않아야 함.
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        let injections = [
            "; touch PWNED1",
            "&& touch PWNED2",
            "$(touch PWNED3)",
            "| touch PWNED4",
            "`touch PWNED5`",
            "> PWNED6",
        ];
        let out = runner.run(&argv(injections), &dir.path).unwrap();
        // echo는 인자를 그대로 출력(리터럴).
        assert!(out.success());
        // 어떤 주입 파일도 생성되지 않음.
        for name in ["PWNED1", "PWNED2", "PWNED3", "PWNED4", "PWNED5", "PWNED6"] {
            assert!(
                !dir.path.join(name).exists(),
                "shell 주입으로 {name} 생성됨(보안 결함)"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn env_overrides_are_applied_to_child() {
        let dir = TempDir::new();
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            Vec::new(),
            Duration::from_millis(5_000),
        )
        .with_env_overrides(vec![(
            OsString::from("STELLAR_OVERRIDE"),
            OsString::from("overridden"),
        )]);
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("STELLAR_OVERRIDE=overridden"),
            "override 미적용: {stdout}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn env_allowlist_filters_environment() {
        let dir = TempDir::new();
        let pid = std::process::id();
        let allowed = format!("STELLAR_TEST_ALLOWED_{pid}");
        let blocked = format!("STELLAR_TEST_BLOCKED_{pid}");
        std::env::set_var(&allowed, "yes-allowed");
        std::env::set_var(&blocked, "secret-blocked");

        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            vec![OsString::from(&allowed)],
            Duration::from_millis(5_000),
        );
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);

        assert!(
            stdout.contains(&format!("{allowed}=yes-allowed")),
            "허용 변수 누락: {stdout}"
        );
        assert!(
            !stdout.contains("secret-blocked"),
            "차단 변수 노출(보안 결함): {stdout}"
        );

        std::env::remove_var(&allowed);
        std::env::remove_var(&blocked);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_long_running_child() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/sleep", 100);
        let start = Instant::now();
        let result = runner.run(&argv(["10"]), &dir.path);
        assert_eq!(result, Err(GitRunError::Timeout));
        // 10초 sleep을 기다리지 않고 빠르게 종료.
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_count_tracks_attempts_and_is_shared_by_clones() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        assert_eq!(runner.spawn_count(), 0);
        runner.run(&argv(["a"]), &dir.path).unwrap();
        let copy = runner.clone();
        copy.run(&argv(["b"]), &dir.path).unwrap();
        // 사본과 원본이 같은 카운터를 공유.
        assert_eq!(runner.spawn_count(), 2);
        assert_eq!(copy.spawn_count(), 2);
        // 실패한 spawn 시도도 셉니다.
        let missing = runner_for("/nonexistent/definitely/not/here", 5_000);
        assert!(missing.run(&[], &dir.path).is_err());
        assert_eq!(missing.spawn_count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn input_bytes_reach_stdin_verbatim_without_shell() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/cat", 5_000);
        let input = "줄1 $(touch PWNED) `id`\n; rm -rf x | y && z\n'\"\n".as_bytes();
        let out = runner.run_with_input(&[], &dir.path, input).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, input);
        assert!(!dir.path.join("PWNED").exists());
    }

    #[cfg(unix)]
    #[test]
    fn added_env_keeps_overrides_and_child_env_var_matches_child() {
        let dir = TempDir::new();
        let pid = std::process::id();
        let allowed = format!("STELLAR_CHILD_ENV_ALLOWED_{pid}");
        let hidden = format!("STELLAR_CHILD_ENV_HIDDEN_{pid}");
        std::env::set_var(&allowed, "from-process");
        std::env::set_var(&hidden, "not-passed");
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            vec![OsString::from(&allowed)],
            Duration::from_millis(5_000),
        )
        .with_env_overrides(vec![
            (OsString::from("STELLAR_A"), OsString::from("1")),
            (OsString::from("STELLAR_KEEP"), OsString::from("kept")),
        ])
        .with_added_env(vec![(OsString::from("STELLAR_A"), OsString::from("2"))]);

        assert_eq!(
            runner.child_env_var(&allowed),
            Some(OsString::from("from-process"))
        );
        assert_eq!(runner.child_env_var(&hidden), None);
        assert_eq!(runner.child_env_var("STELLAR_A"), Some(OsString::from("2")));
        assert_eq!(
            runner.child_env_var("STELLAR_KEEP"),
            Some(OsString::from("kept"))
        );

        // 실제 자식 환경도 같은 값을 봅니다.
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&format!("{allowed}=from-process")),
            "{stdout}"
        );
        assert!(stdout.contains("STELLAR_A=2\n"), "{stdout}");
        assert!(!stdout.contains("STELLAR_A=1\n"), "{stdout}");
        assert!(stdout.contains("STELLAR_KEEP=kept"), "{stdout}");
        assert!(!stdout.contains("not-passed"), "{stdout}");
        std::env::remove_var(&allowed);
        std::env::remove_var(&hidden);
    }

    #[cfg(unix)]
    #[test]
    fn isolated_runner_sets_discovery_guards_and_shares_counter() {
        let dir = TempDir::new();
        let runner = runner_for("/usr/bin/env", 5_000);
        let isolated = runner.isolated_from_repositories(&dir.path);
        assert_eq!(
            isolated.child_env_var("GIT_DIR"),
            Some(OsString::from(NULL_DEVICE))
        );
        assert_eq!(
            isolated.child_env_var("GIT_CEILING_DIRECTORIES"),
            Some(dir.path.as_os_str().to_os_string())
        );
        // 원본은 바뀌지 않음.
        assert_eq!(runner.child_env_var("GIT_DIR"), None);
        isolated.run(&[], &dir.path).unwrap();
        assert_eq!(runner.spawn_count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn streaming_stderr_delivers_chunks_before_exit_and_keeps_full_output() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/sh", 5_000);
        let script = "printf 'step 1\\r' >&2; sleep 0.3; printf 'step 2\\n' >&2; printf out";
        let mut seen: Vec<(Instant, Vec<u8>)> = Vec::new();
        let start = Instant::now();
        let out = runner
            .run_streaming_stderr(
                &argv(["-c", script]),
                &dir.path,
                &GitCancelToken::new(),
                &mut |chunk| seen.push((Instant::now(), chunk.to_vec())),
            )
            .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"step 1\rstep 2\n");
        let joined: Vec<u8> = seen.iter().flat_map(|(_, c)| c.clone()).collect();
        assert_eq!(joined, out.stderr);
        // 첫 조각은 프로세스 종료(≥0.3초) 전에 전달됨.
        assert!(seen[0].0.duration_since(start) < Duration::from_millis(300));
    }

    #[test]
    fn missing_executable_is_spawn_error() {
        let dir = TempDir::new();
        let runner = runner_for("/nonexistent/definitely/not/here", 5_000);
        match runner.run(&[], &dir.path) {
            Err(GitRunError::Spawn { .. }) => {}
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn captures_nonzero_exit_and_does_not_fake_success() {
        let dir = TempDir::new();
        // /usr/bin/false는 종료 코드 1.
        let runner = runner_for("/usr/bin/false", 5_000);
        let out = runner.run(&[], &dir.path).unwrap();
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(1));
    }

    #[cfg(unix)]
    #[test]
    fn cwd_is_respected() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/pwd", 5_000);
        let out = runner.run(&[], &dir.path).unwrap();
        let printed = String::from_utf8_lossy(&out.stdout);
        let printed = printed.trim();
        // canonicalize로 symlink(예: /var→/private/var) 차이를 흡수해 비교.
        let expected = std::fs::canonicalize(&dir.path).unwrap();
        let actual = std::fs::canonicalize(printed).unwrap();
        assert_eq!(actual, expected);
    }
}
