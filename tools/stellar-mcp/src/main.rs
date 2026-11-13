//! stellar-mcp 실행 파일 진입점 (XIX.1.1).
//!
//! 인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC를 처리합니다. stdout에는 protocol 메시지만
//! 쓰므로 도움말과 사용법 오류도 stderr에 씁니다.
//!
//! 종료(XIX.1.9): stdin EOF(MCP stdio의 정상 종료 신호)나 부모 프로세스 종료를 감지하면 스스로
//! 끝납니다. 부모 종료는 [`stellar_mcp::parent`]의 `kqueue` 감시 thread가 감지합니다.
//!
//! 종료 코드(sysexits): 0 정상 종료(입력 끝·부모 종료), 64 사용법 오류, 70 부모 감시를 시작할 수
//! 없음, 74 입출력 오류, 78 내장 protocol config 오류.

use std::io::{self, Write};
use std::process::ExitCode;

use stellar_mcp::log::Logger;
use stellar_mcp::protocol::ProtocolConfig;
use stellar_mcp::server;

const USAGE: &str = "사용법: stellar-mcp\n\n\
    인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC 2.0 메시지(한 줄에 하나)를 처리합니다.\n\
    stdout에는 protocol 메시지만 쓰고, 로그와 이 도움말은 stderr에 씁니다.\n\
    stdin이 닫히거나 실행한 부모 프로세스가 끝나면 스스로 종료합니다.\n";

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

fn main() -> ExitCode {
    let mut log = Logger::new(io::stderr());
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [flag] if flag == "--help" || flag == "-h" => {
            let _ = io::stderr().write_all(USAGE.as_bytes());
            return ExitCode::SUCCESS;
        }
        _ => {
            log.error("인자를 받지 않습니다. `stellar-mcp --help`로 사용법을 확인하세요.");
            return ExitCode::from(64);
        }
    }

    if let Err(code) = watch_parent(&mut log) {
        return code;
    }

    // 내장 protocol version config를 먼저 검사합니다(XIX.1.5).
    match ProtocolConfig::builtin() {
        Ok(config) => log.info(&format!(
            "지원 MCP protocol version: {}",
            config.supported().join(", ")
        )),
        Err(e) => {
            log.error(&format!("protocol version config 오류: {e}"));
            return ExitCode::from(78);
        }
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    match server::serve(&mut input, &mut output, &mut log) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log.error(&format!("입출력 오류로 종료합니다: {e}"));
            ExitCode::from(74)
        }
    }
}
