//! stellar-mcp 실행 파일 진입점 (XIX.1.1).
//!
//! 인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC를 처리합니다. stdout에는 protocol 메시지만
//! 쓰므로 도움말과 사용법 오류도 stderr에 씁니다.
//!
//! `--connection-info`(XIX.9.8)는 MCP 서버를 실행하지 않고 client 등록 정보 JSON 한 줄을 stdout에
//! 쓰고 끝납니다([`stellar_mcp::connection_info`]). 파일을 만들거나 바꾸지 않습니다.
//!
//! 종료(XIX.1.9): stdin EOF(MCP stdio의 정상 종료 신호)나 부모 프로세스 종료를 감지하면 스스로
//! 끝납니다. 부모 종료는 [`stellar_mcp::parent`]의 `kqueue` 감시 thread가 감지합니다.
//!
//! 종료 코드(sysexits): 0 정상 종료(입력 끝·부모 종료·연결 정보 출력), 64 사용법 오류, 70 부모
//! 감시를 시작할 수 없음, 74 입출력 오류(실행 파일 경로 확인·출력 실패 포함), 78 내장 protocol
//! config 오류.

use std::io::{self, Write};
use std::process::ExitCode;

use stellar_mcp::connection_info;
use stellar_mcp::json;
use stellar_mcp::log::Logger;
use stellar_mcp::protocol::ProtocolConfig;
use stellar_mcp::server;

const USAGE: &str = "사용법: stellar-mcp [--connection-info]\n\n\
    인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC 2.0 메시지(한 줄에 하나)를 처리합니다.\n\
    stdout에는 protocol 메시지만 쓰고, 로그와 이 도움말은 stderr에 씁니다.\n\
    stdin이 닫히거나 실행한 부모 프로세스가 끝나면 스스로 종료합니다.\n\n\
    --connection-info  MCP client 등록 정보(transport, command, args, env)를 JSON 한 줄로\n\
    \x20                  stdout에 씁니다. 다른 도구의 설정은 바꾸지 않습니다.\n";

/// 실행 방식.
enum Mode {
    /// MCP stdio 서버.
    Serve,
    /// client 등록 정보 출력(XIX.9.8).
    ConnectionInfo,
}

/// 부모 프로세스 종료를 감시합니다(XIX.1.9). 감시를 시작할 수 없으면 종료 코드를 반환합니다.
#[cfg(target_os = "macos")]
fn watch_parent(log: &mut Logger<io::Stderr>) -> Result<(), ExitCode> {
    use stellar_mcp::parent::{self, Watch};

    let on_exit = || {
        Logger::new(io::stderr()).info("부모 프로세스 종료를 감지해 종료합니다");
        std::process::exit(0);
    };
    match parent::watch_parent(on_exit) {
        Ok(Watch::Watching(pid)) => {
            log.info(&format!("부모 프로세스 감시 시작(pid {pid})"));
            Ok(())
        }
        Ok(Watch::NoParent) => {
            log.info("부모가 launchd라 stdin 종료로만 끝납니다");
            Ok(())
        }
        Ok(Watch::ParentGone) => {
            log.info("부모 프로세스가 이미 종료되어 끝냅니다");
            Err(ExitCode::SUCCESS)
        }
        Err(e) => {
            log.error(&format!("부모 프로세스 감시를 시작할 수 없습니다: {e}"));
            Err(ExitCode::from(70))
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn watch_parent(_log: &mut Logger<io::Stderr>) -> Result<(), ExitCode> {
    // Windows·기타 OS 지원은 ADR-0006에 따라 보류합니다.
    Ok(())
}

/// client 등록 정보를 한 줄로 씁니다(XIX.9.8).
fn print_connection_info(
    config: &ProtocolConfig,
    out: &mut dyn Write,
    log: &mut Logger<io::Stderr>,
) -> ExitCode {
    let command = match connection_info::current_command() {
        Ok(Some(command)) => command,
        Ok(None) => {
            log.error("실행 파일 경로가 UTF-8이 아니라 연결 정보를 만들 수 없습니다");
            return ExitCode::from(74);
        }
        Err(e) => {
            log.error(&format!("실행 파일 경로를 확인할 수 없습니다: {e}"));
            return ExitCode::from(74);
        }
    };
    let mut line = json::write(&connection_info::connection_info(&command, config));
    line.push('\n');
    match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log.error(&format!("연결 정보 출력 실패: {e}"));
            ExitCode::from(74)
        }
    }
}

fn main() -> ExitCode {
    let mut log = Logger::new(io::stderr());
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mode = match args.as_slice() {
        [] => Mode::Serve,
        [flag] if flag == "--help" || flag == "-h" => {
            let _ = io::stderr().write_all(USAGE.as_bytes());
            return ExitCode::SUCCESS;
        }
        [flag] if flag == connection_info::FLAG => Mode::ConnectionInfo,
        _ => {
            log.error("알 수 없는 인자입니다. `stellar-mcp --help`로 사용법을 확인하세요.");
            return ExitCode::from(64);
        }
    };

    // 내장 protocol version config를 먼저 검사합니다(XIX.1.5).
    let config = match ProtocolConfig::builtin() {
        Ok(config) => config,
        Err(e) => {
            log.error(&format!("protocol version config 오류: {e}"));
            return ExitCode::from(78);
        }
    };

    let stdout = io::stdout();
    let mut output = stdout.lock();
    if let Mode::ConnectionInfo = mode {
        return print_connection_info(&config, &mut output, &mut log);
    }

    if let Err(code) = watch_parent(&mut log) {
        return code;
    }
    log.info(&format!(
        "지원 MCP protocol version: {}",
        config.supported().join(", ")
    ));
    let stdin = io::stdin();
    let mut input = stdin.lock();
    match server::serve(&mut input, &mut output, &mut log) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log.error(&format!("입출력 오류로 종료합니다: {e}"));
            ExitCode::from(74)
        }
    }
}
