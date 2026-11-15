//! stellar-mcp 실행 파일 진입점 (XIX.1.1).
//!
//! 인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC를 처리합니다. stdout에는 protocol 메시지만
//! 쓰므로 도움말과 사용법 오류도 stderr에 씁니다.
//!
//! - `--attach [--instance <id>]`(XIX.1.7): MCP 세션을 시작하기 전에 실행 중 앱을 골라 앱 측과 같은
//!   기준으로 인증하고 handshake합니다([`stellar_mcp::attach`]). 실패하면 앱에 아무것도 보내지 않고
//!   XXIV.1.2 표의 종료 코드로 끝납니다.
//! - `--connection-info`(XIX.9.8): MCP 서버를 실행하지 않고 client 등록 정보 JSON 한 줄을 stdout에
//!   쓰고 끝납니다([`stellar_mcp::connection_info`]). 파일을 만들거나 바꾸지 않습니다.
//!
//! 종료(XIX.1.9): stdin EOF(MCP stdio의 정상 종료 신호)나 부모 프로세스 종료를 감지하면 스스로
//! 끝납니다. 부모 종료는 [`stellar_mcp::parent`]의 `kqueue` 감시 thread가 감지합니다.
//!
//! 종료 코드는 XXIV.1.2 표([`stellar_mcp::exit`])를 따릅니다: 0 정상 종료(입력 끝·부모 종료·연결
//! 정보 출력), 64 사용법 오류, 69 앱 없음, 70 내부 오류(부모 감시 시작 실패 포함), 74 입출력 오류,
//! 75 응답 시간 초과, 76 IPC protocol 오류, 77 IPC 상대 인증 실패, 78 설정 오류.

use std::io::{self, Write};
use std::process::ExitCode;

use stellar_mcp::connection_info;
use stellar_mcp::exit::{self, Exit};
use stellar_mcp::json;
use stellar_mcp::log::Logger;
use stellar_mcp::protocol::ProtocolConfig;
use stellar_mcp::server;

const USAGE: &str = "사용법: stellar-mcp [--attach [--instance <id>] | --headless --project <절대 경로> | --connection-info]\n\n\
    인자 없이 실행하면 stdin/stdout으로 MCP JSON-RPC 2.0 메시지(한 줄에 하나)를 처리합니다.\n\
    stdout에는 protocol 메시지만 쓰고, 로그와 이 도움말은 stderr에 씁니다.\n\
    stdin이 닫히거나 실행한 부모 프로세스가 끝나면 스스로 종료합니다.\n\n\
    --attach           실행 중 Stellar 앱을 인증하고 연결한 뒤 MCP 세션을 시작합니다.\n\
    \x20                  앱이 여러 개이면 --instance <id>(소문자 16진수 16자)로 고릅니다.\n\
    --headless --project <절대 경로>  신뢰한 프로젝트 루트 하나로 읽기 세션을 시작합니다.\n\
    --connection-info  MCP client 등록 정보(transport, command, args, env)를 JSON 한 줄로\n\
    \x20                  stdout에 씁니다. 다른 도구의 설정은 바꾸지 않습니다.\n";

/// 실행 방식.
enum Mode {
    /// MCP stdio 서버(앱 없음).
    Serve,
    /// 실행 중 앱에 연결한 MCP stdio 서버(XIX.1.7). 값은 지정 instance id 원문입니다.
    Attach(Option<String>),
    /// 실행 인자로 고정한 신뢰 프로젝트 루트의 읽기 전용 세션(ADR-0005 §3).
    Headless(std::path::PathBuf),
    /// client 등록 정보 출력(XIX.9.8).
    ConnectionInfo,
}

/// 인자를 해석합니다. 도움말이면 `Ok(None)`입니다.
fn parse_args(args: &[std::ffi::OsString]) -> Result<Option<Mode>, ()> {
    let text: Option<Vec<&str>> = args.iter().map(|a| a.to_str()).collect();
    let Some(text) = text else {
        return Err(());
    };
    Ok(Some(match text.as_slice() {
        [] => Mode::Serve,
        ["--help" | "-h"] => return Ok(None),
        [flag] if *flag == connection_info::FLAG => Mode::ConnectionInfo,
        ["--attach"] => Mode::Attach(None),
        ["--headless", "--project", root] => Mode::Headless(std::path::PathBuf::from(root)),
        ["--attach", "--instance", id] => Mode::Attach(Some((*id).to_string())),
        _ => return Err(()),
    }))
}

/// 부모 프로세스 종료를 감시합니다(XIX.1.9). 감시를 시작할 수 없으면 종료 상태를 반환합니다.
#[cfg(target_os = "macos")]
fn watch_parent(log: &mut Logger<io::Stderr>) -> Result<(), Exit> {
    use stellar_mcp::parent::{self, Watch};

    let on_exit = || {
        Logger::new(io::stderr()).info("부모 프로세스 종료를 감지해 종료합니다");
        std::process::exit(i32::from(exit::OK.code));
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
            Err(exit::OK)
        }
        Err(e) => {
            log.error(&format!("부모 프로세스 감시를 시작할 수 없습니다: {e}"));
            Err(exit::SOFTWARE)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn watch_parent(_log: &mut Logger<io::Stderr>) -> Result<(), Exit> {
    // Windows·기타 OS 지원은 ADR-0006에 따라 보류합니다.
    Ok(())
}

/// 실행 중 앱을 인증하고 연결합니다(XIX.1.7). 연결은 MCP 세션이 끝날 때까지 유지하고, 앱이 연결을
/// 닫으면 이 bridge의 하나뿐인 세션도 끝냅니다(XIX.1.4).
#[cfg(target_os = "macos")]
fn attach(
    instance: Option<&str>,
    log: &mut Logger<io::Stderr>,
) -> Result<stellar_local_ipc::handshake::Session, Exit> {
    use stellar_local_ipc::nonce::NonceLedger;
    use stellar_local_ipc::registry::InstanceId;
    use stellar_mcp::attach::watch_app_connection;

    let wanted = match instance.map(InstanceId::parse) {
        None => None,
        Some(Some(id)) => Some(id),
        Some(None) => {
            log.error("--instance는 소문자 16진수 16자여야 합니다");
            return Err(exit::USAGE);
        }
    };
    match stellar_mcp::attach::attach(wanted.as_ref(), &mut NonceLedger::new()) {
        Ok((endpoint, mut session)) => {
            log.info(&format!(
                "Stellar 앱 instance {}에 인증 후 연결했습니다",
                endpoint.instance
            ));
            let on_end = |end: stellar_mcp::attach::AppConnectionEnd| {
                let status = end.exit();
                Logger::new(io::stderr())
                    .warn(&format!("{end}: MCP 세션을 끝냅니다({})", status.name));
                std::process::exit(i32::from(status.code));
            };
            if let Err(e) = watch_app_connection(&mut session, on_end) {
                log.error(&format!("앱 연결 감시를 시작할 수 없습니다: {e}"));
                return Err(exit::SOFTWARE);
            }
            Ok(session)
        }
        Err(e) => {
            let status = e.exit();
            log.error(&format!(
                "Stellar 앱에 연결할 수 없습니다({}): {e}",
                status.name
            ));
            Err(status)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn attach(_instance: Option<&str>, log: &mut Logger<io::Stderr>) -> Result<(), Exit> {
    log.error("--attach는 macOS에서만 지원합니다(ADR-0006)");
    Err(exit::UNAVAILABLE)
}

/// client 등록 정보를 한 줄로 씁니다(XIX.9.8).
fn print_connection_info(
    config: &ProtocolConfig,
    out: &mut dyn Write,
    log: &mut Logger<io::Stderr>,
) -> Exit {
    let command = match connection_info::current_command() {
        Ok(Some(command)) => command,
        Ok(None) => {
            log.error("실행 파일 경로가 UTF-8이 아니라 연결 정보를 만들 수 없습니다");
            return exit::IOERR;
        }
        Err(e) => {
            log.error(&format!("실행 파일 경로를 확인할 수 없습니다: {e}"));
            return exit::IOERR;
        }
    };
    let mut line = json::write(&connection_info::connection_info(&command, config));
    line.push('\n');
    match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => exit::OK,
        Err(e) => {
            log.error(&format!("연결 정보 출력 실패: {e}"));
            exit::IOERR
        }
    }
}

fn main() -> ExitCode {
    run().exit_code()
}

fn run() -> Exit {
    let mut log = Logger::new(io::stderr());
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mode = match parse_args(&args) {
        Ok(Some(mode)) => mode,
        Ok(None) => {
            let _ = io::stderr().write_all(USAGE.as_bytes());
            return exit::OK;
        }
        Err(()) => {
            log.error("알 수 없는 인자입니다. `stellar-mcp --help`로 사용법을 확인하세요.");
            return exit::USAGE;
        }
    };

    // 내장 protocol version config를 먼저 검사합니다(XIX.1.5).
    let config = match ProtocolConfig::builtin() {
        Ok(config) => config,
        Err(e) => {
            log.error(&format!("protocol version config 오류: {e}"));
            return exit::CONFIG;
        }
    };

    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut mcp_server = server::Server::new();
    let _app_session = match mode {
        Mode::ConnectionInfo => return print_connection_info(&config, &mut output, &mut log),
        Mode::Serve => None,
        Mode::Headless(project) => {
            match stellar_mcp::headless::Headless::select(&project) {
                Ok(backend) => mcp_server = server::Server::headless(backend),
                Err(status) => {
                    log.error(
                        "읽기 프로젝트를 선택할 수 없습니다: 절대 경로와 프로젝트 신뢰를 확인하세요",
                    );
                    return status;
                }
            }
            None
        }
        Mode::Attach(instance) => match attach(instance.as_deref(), &mut log) {
            Ok(session) => Some(session),
            Err(status) => return status,
        },
    };

    if let Err(status) = watch_parent(&mut log) {
        return status;
    }
    log.info(&format!(
        "지원 MCP protocol version: {}",
        config.supported().join(", ")
    ));
    let stdin = io::stdin();
    let mut input = stdin.lock();
    match server::serve_with(mcp_server, &mut input, &mut output, &mut log) {
        Ok(()) => exit::OK,
        Err(e) => {
            log.error(&format!("입출력 오류로 종료합니다: {e}"));
            exit::IOERR
        }
    }
}
