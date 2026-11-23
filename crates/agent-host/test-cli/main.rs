//! 구독 CLI 테스트 대역 실행 파일 `stellar-agent-host-test-cli` (XXIX.2.2, ADR-0092 결정 2).
//!
//! 네트워크와 실제 계정 없이 공급자 CLI 실행 경로를 재현하는 테스트 전용 실행 파일입니다. 앱 번들에 넣지
//! 않습니다. `stellar-agent-host` library를 쓰지 않아 framework(CoreFoundation 등)를 link하지 않으므로,
//! 실행기가 넘긴 환경 외에 대역 자신이 환경 변수를 더하지 않습니다(허용 목록 환경 측정의 전제).
//!
//! ## 방식
//!
//! - 테스트는 대역 실행 파일을 임시 디렉터리에 CLI 이름으로 복사하고, 같은 경로에 `.scenario`를 붙인
//!   scenario 파일을 둡니다(`<exe>.scenario`). 대역은 실행될 때 자기 경로로 scenario를 찾습니다. 환경
//!   변수를 쓰지 않으므로 허용 목록 환경 검사에 영향이 없습니다.
//! - 인자로 단계를 고릅니다: `--version`·`-v` → `[version]`, `auth status`·`login status` → `[status]`,
//!   `--hang` → 끝없이 대기(하위 프로세스 흔적 시험용), 그 밖 → `[session]`.
//! - scenario는 줄 단위 명령입니다. 각 명령은 결정적이며 네트워크 API를 쓰지 않습니다.
//!
//! | 명령 | 동작 |
//! |---|---|
//! | `out <text>` | stdout에 씀(`\n`·`\t`·`\\`·`\xHH` escape) |
//! | `err <text>` | stderr에 씀 |
//! | `outfile <path>` | 파일 내용을 stdout에 씀(현재 chunk 크기로 나눠 씀) |
//! | `chunk <n>` | 이후 쓰기를 n byte 조각으로 나눔(입력 경계 재현) |
//! | `sleep <ms>` | 대기 |
//! | `flood <bytes>` | `x` 줄을 그 크기만큼 빠르게 씀 |
//! | `read-line` | stdin 한 줄을 읽음 |
//! | `read-all` | stdin을 끝까지 읽음 |
//! | `stdin-dump <path>` | 지금까지 읽은 stdin을 파일로 씀 |
//! | `env-dump <path>` | 환경 변수 이름(정렬)을 파일로 씀 |
//! | `cwd-dump <path>` | 현재 디렉터리를 파일로 씀 |
//! | `args-dump <path>` | 인자를 줄마다 파일로 씀 |
//! | `mark <path>` | 세션 실행 표시 파일을 만듦 |
//! | `touch <path>` | 파일을 만듦(흔적 정리 시험용) |
//! | `spawn-child <pidfile>` | 자기 자신을 `--hang`으로 띄우고 pid를 씀 |
//! | `hang` | 끝없이 대기 |
//! | `exit <code>` | 그 코드로 끝냄 |
//!
//! 명령이 끝나면 종료 코드 0입니다. scenario를 읽을 수 없으면 97, 형식 오류는 98입니다.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// scenario 파일 확장자.
const SCENARIO_SUFFIX: &str = ".scenario";

/// 명령 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// stdout 쓰기.
    Out(Vec<u8>),
    /// stderr 쓰기.
    Err(Vec<u8>),
    /// 파일 내용 stdout.
    OutFile(PathBuf),
    /// chunk 크기.
    Chunk(usize),
    /// 대기(ms).
    Sleep(u64),
    /// 대량 출력(byte).
    Flood(usize),
    /// stdin 한 줄.
    ReadLine,
    /// stdin 끝까지.
    ReadAll,
    /// 읽은 stdin 저장.
    StdinDump(PathBuf),
    /// 환경 변수 이름 저장.
    EnvDump(PathBuf),
    /// cwd 저장.
    CwdDump(PathBuf),
    /// 인자 저장.
    ArgsDump(PathBuf),
    /// 실행 표시.
    Mark(PathBuf),
    /// 파일 만들기.
    Touch(PathBuf),
    /// 하위 프로세스 띄우기.
    SpawnChild(PathBuf),
    /// 끝없이 대기.
    Hang,
    /// 종료 코드.
    Exit(i32),
}

/// 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 버전 확인.
    Version,
    /// 연결 상태 확인.
    Status,
    /// 세션.
    Session,
}

/// scenario(단계별 명령).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Scenario {
    /// `[version]`.
    version: Vec<Step>,
    /// `[status]`.
    status: Vec<Step>,
    /// `[session]`.
    session: Vec<Step>,
}

/// escape를 풉니다.
fn unescape(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            match bytes.get(i + 1)? {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'\\' => out.push(b'\\'),
                b'x' => {
                    let hex = text.get(i + 2..i + 4)?;
                    out.push(u8::from_str_radix(hex, 16).ok()?);
                    i += 2;
                }
                _ => return None,
            }
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

impl Scenario {
    /// scenario 원문을 읽습니다.
    fn parse(source: &str) -> Result<Scenario, String> {
        let mut scenario = Scenario::default();
        let mut section: Option<Phase> = None;
        for (n, line) in source.lines().enumerate() {
            let at = || format!("{}행", n + 1);
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            match line.trim() {
                "[version]" => {
                    section = Some(Phase::Version);
                    continue;
                }
                "[status]" => {
                    section = Some(Phase::Status);
                    continue;
                }
                "[session]" => {
                    section = Some(Phase::Session);
                    continue;
                }
                _ => {}
            }
            let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
            let path = || -> Result<PathBuf, String> {
                if arg.is_empty() {
                    Err(format!("{}: 경로 없음", at()))
                } else {
                    Ok(PathBuf::from(arg))
                }
            };
            let number = || -> Result<u64, String> {
                arg.parse::<u64>().map_err(|_| format!("{}: 숫자", at()))
            };
            let step = match cmd {
                "out" => Step::Out(unescape(arg).ok_or_else(|| format!("{}: escape", at()))?),
                "err" => Step::Err(unescape(arg).ok_or_else(|| format!("{}: escape", at()))?),
                "outfile" => Step::OutFile(path()?),
                "chunk" => Step::Chunk((number()? as usize).max(1)),
                "sleep" => Step::Sleep(number()?),
                "flood" => Step::Flood(number()? as usize),
                "read-line" => Step::ReadLine,
                "read-all" => Step::ReadAll,
                "stdin-dump" => Step::StdinDump(path()?),
                "env-dump" => Step::EnvDump(path()?),
                "cwd-dump" => Step::CwdDump(path()?),
                "args-dump" => Step::ArgsDump(path()?),
                "mark" => Step::Mark(path()?),
                "touch" => Step::Touch(path()?),
                "spawn-child" => Step::SpawnChild(path()?),
                "hang" => Step::Hang,
                "exit" => Step::Exit(arg.parse().map_err(|_| format!("{}: 종료 코드", at()))?),
                _ => return Err(format!("{}: 모르는 명령", at())),
            };
            match section {
                Some(Phase::Version) => scenario.version.push(step),
                Some(Phase::Status) => scenario.status.push(step),
                Some(Phase::Session) => scenario.session.push(step),
                None => return Err(format!("{}: 절 밖 명령", at())),
            }
        }
        Ok(scenario)
    }

    /// 단계의 명령.
    fn steps(&self, phase: Phase) -> &[Step] {
        match phase {
            Phase::Version => &self.version,
            Phase::Status => &self.status,
            Phase::Session => &self.session,
        }
    }
}

/// 인자로 단계를 고릅니다. `None`이면 `--hang` 모드입니다.
fn phase_for(args: &[String]) -> Option<Phase> {
    match args {
        [a] if a == "--hang" => None,
        [a] if a == "--version" || a == "-v" => Some(Phase::Version),
        [a, b, ..] if (a == "auth" || a == "login") && b == "status" => Some(Phase::Status),
        _ => Some(Phase::Session),
    }
}

fn write_chunked(target: &mut dyn Write, bytes: &[u8], chunk: usize) {
    for piece in bytes.chunks(chunk) {
        if target
            .write_all(piece)
            .and_then(|_| target.flush())
            .is_err()
        {
            return;
        }
    }
}

fn hang() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// 대역 실행 파일의 본문. 종료 코드를 돌려줍니다.
fn run(exe: &Path, args: &[String]) -> i32 {
    let Some(phase) = phase_for(args) else { hang() };
    let mut scenario_path = exe.as_os_str().to_owned();
    scenario_path.push(SCENARIO_SUFFIX);
    let Ok(source) = std::fs::read_to_string(&scenario_path) else {
        return 97;
    };
    let Ok(scenario) = Scenario::parse(&source) else {
        return 98;
    };
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr();
    let mut stdin_read: Vec<u8> = Vec::new();
    let mut chunk = usize::MAX;
    for step in scenario.steps(phase) {
        match step {
            Step::Out(bytes) => write_chunked(&mut stdout, bytes, chunk),
            Step::Err(bytes) => write_chunked(&mut stderr, bytes, chunk),
            Step::OutFile(path) => match std::fs::read(path) {
                Ok(bytes) => write_chunked(&mut stdout, &bytes, chunk),
                Err(_) => return 96,
            },
            Step::Chunk(n) => chunk = *n,
            Step::Sleep(ms) => std::thread::sleep(Duration::from_millis(*ms)),
            Step::Flood(total) => {
                let line = [b'x'; 1023];
                let mut written = 0;
                while written < *total {
                    if stdout
                        .write_all(&line)
                        .and_then(|_| stdout.write_all(b"\n"))
                        .is_err()
                    {
                        break;
                    }
                    written += 1024;
                }
                let _ = stdout.flush();
            }
            Step::ReadLine => {
                let mut byte = [0u8; 1];
                while let Ok(1) = std::io::stdin().read(&mut byte) {
                    stdin_read.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
            }
            Step::ReadAll => {
                let _ = std::io::stdin().read_to_end(&mut stdin_read);
            }
            Step::StdinDump(path) => {
                let _ = std::fs::write(path, &stdin_read);
            }
            Step::EnvDump(path) => {
                let mut names: Vec<String> = std::env::vars_os()
                    .map(|(k, _)| k.to_string_lossy().into_owned())
                    .collect();
                names.sort();
                let _ = std::fs::write(path, names.join("\n"));
            }
            Step::CwdDump(path) => {
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let _ = std::fs::write(path, cwd);
            }
            Step::ArgsDump(path) => {
                let _ = std::fs::write(path, args.join("\n"));
            }
            Step::Mark(path) | Step::Touch(path) => {
                let _ = std::fs::write(path, b"");
            }
            Step::SpawnChild(pidfile) => {
                match std::process::Command::new(exe)
                    .arg("--hang")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        let _ = std::fs::write(pidfile, child.id().to_string());
                    }
                    Err(_) => return 95,
                }
            }
            Step::Hang => hang(),
            Step::Exit(code) => return *code,
        }
    }
    0
}

fn main() {
    let exe = std::env::current_exe().unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&exe, &args));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_escapes() {
        let s = Scenario::parse(
            "# c\n[version]\nout 2.1.5\\n\n[status]\nexit 1\n[session]\nchunk 3\nout a\\x1bb\\tc\nsleep 5\n",
        )
        .unwrap();
        assert_eq!(s.version, vec![Step::Out(b"2.1.5\n".to_vec())]);
        assert_eq!(s.status, vec![Step::Exit(1)]);
        assert_eq!(s.session[1], Step::Out(b"a\x1bb\tc".to_vec()));
        assert!(Scenario::parse("out x\n").is_err());
        assert!(Scenario::parse("[session]\nbogus\n").is_err());
    }

    #[test]
    fn phase_selection() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(phase_for(&a(&["--version"])), Some(Phase::Version));
        assert_eq!(phase_for(&a(&["auth", "status"])), Some(Phase::Status));
        assert_eq!(phase_for(&a(&["login", "status"])), Some(Phase::Status));
        assert_eq!(phase_for(&a(&["-p", "--verbose"])), Some(Phase::Session));
        assert_eq!(phase_for(&a(&["--hang"])), None);
    }
}
