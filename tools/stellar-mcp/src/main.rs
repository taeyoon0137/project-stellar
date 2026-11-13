//! stellar-mcp 실행 파일 진입점 (XIX.1.1).
//!
//! 인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC를 처리합니다. stdout에는 protocol 메시지만
//! 쓰므로 도움말과 사용법 오류도 stderr에 씁니다. 종료 코드: 0 정상 종료(입력 끝), 64 사용법
//! 오류, 74 입출력 오류, 78 내장 protocol config 오류(sysexits).

use std::io::{self, Write};
use std::process::ExitCode;

use stellar_mcp::log::Logger;
use stellar_mcp::protocol::ProtocolConfig;
use stellar_mcp::server;

const USAGE: &str = "사용법: stellar-mcp\n\n\
    인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC 2.0 메시지(한 줄에 하나)를 처리합니다.\n\
    stdout에는 protocol 메시지만 쓰고, 로그와 이 도움말은 stderr에 씁니다.\n";

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
