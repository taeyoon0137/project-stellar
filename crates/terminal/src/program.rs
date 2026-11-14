//! 외부 CLI를 일반 profile 명령으로 실행하기 (XVIII.5.1).
//!
//! 외부 CLI 에이전트도 셸이나 다른 명령과 똑같이 [`ProfileCommand::Argv`](crate::profile::ProfileCommand)
//! profile로 실행합니다. 이 crate에는 특정 CLI를 알아보는 분기, 전용 실행 경로, 출력 parser가 없습니다
//! (기술 아키텍처 31.9 "터미널 경로에는 공급자별 출력 parser를 두지 않음", 32 "터미널은 AI 로직을
//! 재구현하지 않음"). CLI 출력은 다른 프로그램 출력처럼 emulator 화면으로만 해석됩니다.
//!
//! - **실행 파일 찾기**([`find_program`]): 사용자는 보통 명령 이름만 압니다. profile을 만들 때 child가
//!   받을 `PATH` 값에서 이름을 찾아 **절대 경로**로 바꾸고, profile에는 그 절대 경로를 저장합니다.
//!   실행할 때마다 `PATH`를 검색하지 않으므로 나중에 `PATH` 앞쪽에 생긴 같은 이름의 파일이 대신
//!   실행되지 않습니다([`ArgvCommand`](crate::profile::ArgvCommand) 규칙). UI는 찾은 경로를 사용자에게
//!   보여 줍니다.
//! - `PATH`의 빈 항목과 상대 경로 항목(현재 디렉터리 검색)은 건너뜁니다. 이름에 `/`가 있으면 절대
//!   경로만 받습니다.
//! - 인자는 argv 배열 그대로 전달합니다(셸 문자열 해석 없음).

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::profile::MAX_ARG_BYTES;
use crate::shell::{check_shell_path, ShellPathProblem};

/// `PATH`에서 확인하는 항목 수 상한.
pub const MAX_PATH_ENTRIES: usize = 256;

/// 실행 파일 찾기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramError {
    /// 이름이 비었거나 NUL이 있거나 너무 길거나, `/`가 있는 상대 경로입니다.
    InvalidName,
    /// `PATH`에서 찾지 못했습니다.
    NotFound {
        /// 찾은 이름.
        name: PathBuf,
    },
    /// 지정한 절대 경로를 실행할 수 없습니다.
    Unusable {
        /// 경로.
        path: PathBuf,
        /// 이유.
        problem: ShellPathProblem,
    },
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::InvalidName => f.write_str("실행할 명령 이름이 올바르지 않습니다"),
            ProgramError::NotFound { name } => {
                write!(f, "PATH에서 명령을 찾지 못했습니다: {}", name.display())
            }
            ProgramError::Unusable { path, problem } => write!(
                f,
                "명령을 실행할 수 없습니다({}): {}",
                problem.code(),
                path.display()
            ),
        }
    }
}

impl std::error::Error for ProgramError {}

/// 명령 이름(또는 절대 경로)을 실행 파일 절대 경로로 바꿉니다. `path_var`는 child가 받을 `PATH`
/// 값입니다(없으면 이름 검색 불가).
pub fn find_program(name: &OsStr, path_var: Option<&OsStr>) -> Result<PathBuf, ProgramError> {
    let bytes = name.as_encoded_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ARG_BYTES || bytes.contains(&0) {
        return Err(ProgramError::InvalidName);
    }
    let path = Path::new(name);
    if path.is_absolute() {
        check_shell_path(path).map_err(|problem| ProgramError::Unusable {
            path: path.to_path_buf(),
            problem,
        })?;
        return Ok(path.to_path_buf());
    }
    if bytes.contains(&b'/') {
        return Err(ProgramError::InvalidName);
    }
    let not_found = || ProgramError::NotFound {
        name: path.to_path_buf(),
    };
    let path_var = path_var.ok_or_else(not_found)?;
    std::env::split_paths(path_var)
        .take(MAX_PATH_ENTRIES)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(path))
        .find(|candidate| check_shell_path(candidate).is_ok())
        .ok_or_else(not_found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn names_are_resolved_through_path_to_absolute_programs() {
        let dir = TempDir::new();
        let first = dir.mkdir("first");
        let second = dir.mkdir("second");
        let tool = executable(&second, "agent-tool", "#!/bin/sh\n");
        // 실행 권한이 없는 같은 이름은 건너뜁니다.
        std::fs::write(first.join("agent-tool"), "not executable").unwrap();
        let path_var =
            std::env::join_paths([Path::new("relative/bin"), first.as_path(), second.as_path()])
                .unwrap();
        assert_eq!(
            find_program("agent-tool".as_ref(), Some(&path_var)),
            Ok(tool.clone())
        );
        // 절대 경로는 그대로 검사합니다.
        assert_eq!(find_program(tool.as_os_str(), None), Ok(tool));
        assert_eq!(
            find_program(first.join("agent-tool").as_os_str(), None),
            Err(ProgramError::Unusable {
                path: first.join("agent-tool"),
                problem: ShellPathProblem::NotExecutable,
            })
        );
        assert!(matches!(
            find_program("missing-tool".as_ref(), Some(&path_var)),
            Err(ProgramError::NotFound { .. })
        ));
        assert!(matches!(
            find_program("agent-tool".as_ref(), None),
            Err(ProgramError::NotFound { .. })
        ));
        // 빈 이름, 상대 경로, NUL은 받지 않습니다.
        for bad in ["", "./agent-tool", "bin/agent-tool", "a\0b"] {
            assert_eq!(
                find_program(bad.as_ref(), Some(&path_var)),
                Err(ProgramError::InvalidName),
                "{bad:?}"
            );
        }
        assert!(ProgramError::InvalidName.to_string().contains("명령"));
    }

    /// 임의 CLI 명령을 일반 profile로 실행하는 smoke(XVIII.5.1 완료 조건). 외부 CLI 에이전트처럼
    /// 대체 화면·색·raw 입력을 쓰는 가짜 CLI와 시스템 명령 몇 가지를 같은 경로로 실행합니다.
    #[cfg(target_os = "macos")]
    #[test]
    fn arbitrary_cli_commands_run_as_plain_profiles() {
        use std::ffi::OsString;
        use std::time::{Duration, Instant};

        use crate::cwd::CwdContext;
        use crate::emulator::Terminal;
        use crate::profile::{ArgvCommand, ProfileName, TerminalProfile};
        use crate::pty::PtyLauncher;
        use crate::registry::tests::{fake_env, sh, window, NO_SHELL_SOURCE};
        use crate::registry::{LaunchEnvironment, TerminalRegistry};
        use crate::size::TerminalSize;
        use crate::state::{TerminalExitStatus, TerminalProcessState, TerminationReason};
        use crate::test_support::{pty_shared, OutputCollector};

        let _pty = pty_shared();
        let dir = TempDir::new();
        let bin = dir.mkdir("bin");
        executable(
            &bin,
            "agent-cli",
            "#!/bin/sh\n\
             stty raw -echo\n\
             printf '\\033[?1049h\\033[2J\\033[H\\033[1;32mready\\033[0m [%s][%s] %s\\r\\n' \"$1\" \"$2\" \"$TERM\"\n\
             key=$(dd bs=1 count=1 2>/dev/null)\n\
             stty sane\n\
             printf '\\033[?1049lkey=%s cols=%s\\n' \"$key\" \"$(tput cols)\"\n\
             exit 4\n",
        );
        let path_var = format!("{}:/usr/bin:/bin", bin.display());
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            env_lookup: fake_env,
            size: TerminalSize::new(30, 100).unwrap(),
        };
        let mut launcher = PtyLauncher::with_grace(Duration::from_millis(300));
        let mut registry = TerminalRegistry::new(window(40));
        let trusted = |registry: &TerminalRegistry| {
            crate::capability::TerminalCapability::grant(
                registry.window(),
                crate::capability::TerminalTrust::Trusted,
            )
            .unwrap()
        };

        // 명령 이름을 PATH로 찾아 절대 경로 profile을 만들고 실행합니다.
        let program = find_program("agent-cli".as_ref(), Some(path_var.as_ref())).unwrap();
        assert_eq!(program, bin.join("agent-cli"));
        let mut agent = TerminalProfile::command(
            ProfileName::new("agent").unwrap(),
            ArgvCommand::new(&program, ["--model", "two words"]).unwrap(),
        );
        agent.environment.set("PATH", path_var.as_str()).unwrap();
        let id = registry.create(agent).unwrap();
        registry
            .start(id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        let header = registry.header(id, &context, None).unwrap();
        assert_eq!(
            header.command,
            crate::header::HeaderCommand::Program {
                name: "agent-cli".to_owned()
            }
        );
        let session = registry.session_mut(id).unwrap();
        let process = session.process_mut().unwrap();
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        output.wait_for("ready", crate::pty::tests::TIMEOUT);
        // raw mode라 Enter 없이 한 글자를 받습니다.
        process.write_input(b"y").unwrap();
        let deadline = Instant::now() + crate::pty::tests::TIMEOUT;
        while !registry.poll().contains(&id) {
            assert!(Instant::now() < deadline, "CLI가 끝나지 않았습니다");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            registry.session(id).unwrap().state(),
            TerminalProcessState::Finished {
                status: TerminalExitStatus::Exited(4),
                reason: TerminationReason::ChildExited,
                remaining_processes: 0,
            }
        ));
        let raw = output.wait_eof(crate::pty::tests::TIMEOUT);
        assert!(
            raw.contains("[--model][two words] xterm-256color"),
            "{raw:?}"
        );
        // 출력은 일반 emulator 화면으로만 해석됩니다: 대체 화면을 나온 뒤 주 화면 결과.
        let mut terminal = Terminal::new(TerminalSize::new(30, 100).unwrap());
        terminal.feed(raw.as_bytes());
        let screen = terminal.screen();
        assert!(!screen.modes().alt_screen);
        assert_eq!(screen.lines()[0], "key=y cols=100");
        registry.close(id).unwrap();

        // 시스템 명령도 같은 profile 경로로 실행합니다.
        let cases: [(&str, &[&str], &str); 4] = [
            ("echo", &["arbitrary", "cli"], "arbitrary cli"),
            ("env", &[], "COLORTERM=truecolor"),
            ("tput", &["lines"], "30"),
            ("ls", &["-1", "bin"], "agent-cli"),
        ];
        for (name, args, expected) in cases {
            let program = find_program(name.as_ref(), Some(OsStr::new("/usr/bin:/bin"))).unwrap();
            let profile = TerminalProfile::command(
                ProfileName::new(name).unwrap(),
                ArgvCommand::new(&program, args.iter().map(OsString::from)).unwrap(),
            );
            let id = registry.create(profile).unwrap();
            registry
                .start(id, trusted(&registry), &environment, &mut launcher)
                .unwrap();
            let process = registry.session(id).unwrap().process().unwrap();
            let output = OutputCollector::start(process.output_reader().unwrap());
            let deadline = Instant::now() + crate::pty::tests::TIMEOUT;
            while !registry.poll().contains(&id) {
                assert!(Instant::now() < deadline, "{name} 실행이 끝나지 않았습니다");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                matches!(
                    registry.session(id).unwrap().state(),
                    TerminalProcessState::Finished {
                        status: TerminalExitStatus::Exited(0),
                        ..
                    }
                ),
                "{name}"
            );
            let mut terminal = Terminal::new(TerminalSize::new(30, 100).unwrap());
            terminal.feed(output.wait_eof(crate::pty::tests::TIMEOUT).as_bytes());
            let lines = terminal.screen().lines();
            assert!(
                lines.iter().any(|line| line == expected),
                "{name}: {lines:?}"
            );
            registry.close(id).unwrap();
        }
        assert!(registry.is_empty());
    }
}
