//! MCP client 등록 정보 (XIX.9.8, ADR-0005 §3 "외부 클라이언트 연결 정보").
//!
//! `stellar-mcp --connection-info`는 MCP client에 이 서버를 등록하는 데 필요한 정보를 JSON 한 줄로
//! stdout에 씁니다. Stellar는 다른 도구(client)의 설정 파일을 읽거나 바꾸지 않습니다. 사용자나
//! client가 이 정보를 자기 설정 형식으로 옮깁니다.
//!
//! ## 제공 방식의 근거(MCP 명세 2025-11-25)
//!
//! - Transports "stdio": client가 서버를 subprocess로 실행하고 stdin/stdout으로 메시지를
//!   주고받습니다. 그래서 등록에 필요한 것은 실행 파일(`command`)·인자(`args`)·추가 환경 변수(`env`)
//!   이고, transport는 `"stdio"`입니다. 이 서버는 인자와 환경 변수를 요구하지 않습니다.
//! - Lifecycle "Version Negotiation": 지원 protocol version 목록(`protocolVersions`)을 참고용으로
//!   함께 줍니다. 실제 version은 initialize에서 협상합니다.
//! - Lifecycle "Initialization": `serverInfo`(`name`, `version`)는 initialize 응답의 값과 같습니다.
//! - MCP 명세는 client 설정 파일 형식을 정하지 않습니다. 그래서 특정 client 형식을 만들지 않고 위
//!   값만 담은 Stellar 소유 형식(`schema` = [`SCHEMA_ID`])으로 제공합니다.
//!
//! key 순서는 고정입니다: `schema`, `schemaVersion`, `transport`, `command`, `args`, `env`,
//! `serverInfo`, `protocolVersions`.

use crate::json::JsonValue;
use crate::protocol::ProtocolConfig;
use crate::server::SERVER_NAME;

/// 연결 정보 형식 식별자.
pub const SCHEMA_ID: &str = "stellar-mcp/connection-info";
/// 연결 정보 형식 버전. 호환되지 않는 변경에서만 올립니다.
pub const SCHEMA_VERSION: i64 = 1;
/// 실행 flag.
pub const FLAG: &str = "--connection-info";

/// `command`(이 실행 파일의 절대 경로)로 등록 정보를 만듭니다.
pub fn connection_info(command: &str, protocol: &ProtocolConfig) -> JsonValue {
    connection_info_with_args(command, protocol, Vec::new())
}

/// 프로젝트에 고정한 headless 서버의 등록 정보를 만듭니다.
pub fn connection_info_for_project(
    command: &str,
    protocol: &ProtocolConfig,
    project: &str,
) -> JsonValue {
    connection_info_with_args(
        command,
        protocol,
        vec![
            JsonValue::string("--headless"),
            JsonValue::string("--project"),
            JsonValue::string(project),
        ],
    )
}

fn connection_info_with_args(
    command: &str,
    protocol: &ProtocolConfig,
    args: Vec<JsonValue>,
) -> JsonValue {
    JsonValue::object(vec![
        ("schema", JsonValue::string(SCHEMA_ID)),
        ("schemaVersion", JsonValue::int(SCHEMA_VERSION)),
        ("transport", JsonValue::string("stdio")),
        ("command", JsonValue::string(command)),
        ("args", JsonValue::Array(args)),
        ("env", JsonValue::Object(Vec::new())),
        (
            "serverInfo",
            JsonValue::object(vec![
                ("name", JsonValue::string(SERVER_NAME)),
                ("version", JsonValue::string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "protocolVersions",
            JsonValue::Array(protocol.supported().iter().map(JsonValue::string).collect()),
        ),
    ])
}

/// 실행 파일 절대 경로를 얻습니다(symlink 해석). UTF-8이 아니면 `None`입니다.
pub fn current_command() -> std::io::Result<Option<String>> {
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.to_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::write;

    #[test]
    fn connection_info_has_fixed_keys_and_values() {
        let config = ProtocolConfig::builtin().unwrap();
        let info = connection_info("/Applications/Stellar.app/x/stellar-mcp", &config);
        assert_eq!(
            write(&info),
            "{\"schema\":\"stellar-mcp/connection-info\",\"schemaVersion\":1,\
             \"transport\":\"stdio\",\"command\":\"/Applications/Stellar.app/x/stellar-mcp\",\
             \"args\":[],\"env\":{},\"serverInfo\":{\"name\":\"stellar-mcp\",\"version\":\"0.0.0\"},\
             \"protocolVersions\":[\"2025-11-25\",\"2025-06-18\"]}"
        );
    }

    #[test]
    fn command_path_is_escaped_as_json_string() {
        let config = ProtocolConfig::builtin().unwrap();
        let info = connection_info("/tmp/a \"b\"\n", &config);
        let text = write(&info);
        assert!(text.contains(r#""command":"/tmp/a \"b\"\n""#), "{text}");
        assert!(!text.contains('\n'));
    }

    #[test]
    fn current_command_is_absolute() {
        let command = current_command().unwrap().unwrap();
        assert!(command.starts_with('/'), "{command}");
    }
}
