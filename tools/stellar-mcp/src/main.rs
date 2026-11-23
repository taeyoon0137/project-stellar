//! stellar-mcp 실행 파일 진입점 (XIX.1.1, XIX.9.1).
//!
//! 표준 입출력 handle을 열어 `stellar-mcp-core`의 [`stellar_mcp::entry::run`]에 넘기기만 합니다. 인자 해석,
//! backend 선택, protocol 처리는 모두 library에 있습니다(ADR-0005 §3·§8). stdout에는 protocol 메시지만
//! 씁니다.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let stdin = io::stdin();
    let stdout = io::stdout();
    stellar_mcp::entry::run(&args, &mut stdin.lock(), &mut stdout.lock()).exit_code()
}
