//! MCP protocol version 고정과 협상 (XIX.1.5).
//!
//! 지원 version 목록은 코드가 아니라 `config/mcp-protocol.yaml`에 고정합니다. 실행 파일은 빌드 시
//! 이 파일을 내장하므로 설치 위치와 무관하게 같은 목록을 씁니다.
//!
//! 협상 규칙은 MCP 명세 2025-11-25 Lifecycle "Version Negotiation"을 따릅니다.
//!
//! - client가 보낸 `protocolVersion`이 목록에 **정확히** 있으면 그 version으로 응답합니다
//!   ([`Negotiation::Accepted`]).
//! - 없으면 오류 대신 서버가 지원하는 version 중 최신(선호, 목록 첫 항목)으로 응답합니다
//!   ([`Negotiation::Offered`]). 명세상 서버는 지원하지 않는 요청에 "자신이 지원하는 다른
//!   version"으로 응답해야 하고(MUST), 그 version을 지원하지 않는 client가 연결을 끊습니다.
//! - `protocolVersion`이 문자열이 아니거나 없으면 version 협상이 아니라 params 형식 오류이므로
//!   서버가 `-32602`로 거부합니다([`crate::server`]).

use std::fmt;

/// 내장 config 원문(`config/mcp-protocol.yaml`).
pub const CONFIG_TEXT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../config/mcp-protocol.yaml"
));

/// 지원 목록 key.
const VERSIONS_KEY: &str = "supported_versions";

/// config의 선호(첫) version. 빌드 시 [`CONFIG_TEXT`]에서 계산합니다.
pub const PREFERRED_VERSION: &str = first_version(CONFIG_TEXT);

/// 로그·오류 echo에 남기는 client 문자열(요청 version, method 이름) 최대 문자 수.
pub const MAX_ECHO_CHARS: usize = 64;

/// config에서 `supported_versions:` 줄의 첫 항목을 찾습니다(빌드 시 평가 전용).
///
/// 형식 검증은 [`ProtocolConfig::from_config_str`]가 하고, 테스트가 두 결과의 일치를 확인합니다.
/// 키가 없으면 const 평가가 실패해 빌드가 중단됩니다.
const fn first_version(text: &str) -> &str {
    let bytes = text.as_bytes();
    let key = VERSIONS_KEY.as_bytes();
    let mut line_start = 0;
    while line_start < bytes.len() {
        let mut matches = line_start + key.len() < bytes.len();
        let mut k = 0;
        while matches && k < key.len() {
            if bytes[line_start + k] != key[k] {
                matches = false;
            }
            k += 1;
        }
        if matches && bytes[line_start + key.len()] == b':' {
            let mut start = line_start + key.len() + 1;
            while start < bytes.len() && bytes[start] == b' ' {
                start += 1;
            }
            let mut end = start;
            while end < bytes.len() && !matches!(bytes[end], b',' | b' ' | b'\r' | b'\n') {
                end += 1;
            }
            let (_, rest) = bytes.split_at(start);
            let (version, _) = rest.split_at(end - start);
            return match core::str::from_utf8(version) {
                Ok(v) => v,
                Err(_) => panic!("mcp-protocol config의 version이 UTF-8이 아닙니다"),
            };
        }
        while line_start < bytes.len() && bytes[line_start] != b'\n' {
            line_start += 1;
        }
        line_start += 1;
    }
    panic!("mcp-protocol config에 supported_versions가 없습니다")
}

/// config 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolConfigError {
    /// `supported_versions` key가 없습니다.
    MissingKey,
    /// 같은 key가 두 번 나옵니다.
    DuplicateKey,
    /// 알 수 없는 key 또는 `key: value` 형식이 아닌 줄.
    UnknownKey {
        /// 해당 key(또는 줄).
        key: String,
    },
    /// 목록이 비었습니다.
    Empty,
    /// `YYYY-MM-DD` 형식이 아닌 version.
    InvalidVersion {
        /// 원본 값.
        value: String,
    },
    /// 같은 version이 두 번 나옵니다.
    DuplicateVersion {
        /// 중복 값.
        value: String,
    },
}

impl fmt::Display for ProtocolConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolConfigError::MissingKey => {
                write!(f, "mcp-protocol config에 {VERSIONS_KEY}가 없습니다")
            }
            ProtocolConfigError::DuplicateKey => {
                write!(f, "mcp-protocol config에 {VERSIONS_KEY}가 두 번 있습니다")
            }
            ProtocolConfigError::UnknownKey { key } => {
                write!(f, "mcp-protocol config에 알 수 없는 key: {key}")
            }
            ProtocolConfigError::Empty => write!(f, "지원 MCP protocol version 목록이 비었습니다"),
            ProtocolConfigError::InvalidVersion { value } => {
                write!(
                    f,
                    "MCP protocol version 형식(YYYY-MM-DD)이 아닙니다: {value:?}"
                )
            }
            ProtocolConfigError::DuplicateVersion { value } => {
                write!(f, "MCP protocol version이 중복됩니다: {value}")
            }
        }
    }
}

impl std::error::Error for ProtocolConfigError {}

/// 지원 version 목록(비어 있지 않음, 중복 없음, 선호 순).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolConfig {
    versions: Vec<String>,
}

impl ProtocolConfig {
    /// 내장 config([`CONFIG_TEXT`])를 읽습니다.
    pub fn builtin() -> Result<ProtocolConfig, ProtocolConfigError> {
        ProtocolConfig::from_config_str(CONFIG_TEXT)
    }

    /// `key: value` 한 줄 형식의 config 텍스트를 읽습니다.
    pub fn from_config_str(text: &str) -> Result<ProtocolConfig, ProtocolConfigError> {
        let mut versions: Option<Vec<String>> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(ProtocolConfigError::UnknownKey {
                    key: line.to_string(),
                });
            };
            if key.trim() != VERSIONS_KEY {
                return Err(ProtocolConfigError::UnknownKey {
                    key: key.trim().to_string(),
                });
            }
            if versions.is_some() {
                return Err(ProtocolConfigError::DuplicateKey);
            }
            let mut list: Vec<String> = Vec::new();
            for item in value.split(',') {
                let item = item.trim();
                if item.is_empty() && value.trim().is_empty() {
                    break;
                }
                if !is_version(item) {
                    return Err(ProtocolConfigError::InvalidVersion {
                        value: item.to_string(),
                    });
                }
                if list.iter().any(|v| v == item) {
                    return Err(ProtocolConfigError::DuplicateVersion {
                        value: item.to_string(),
                    });
                }
                list.push(item.to_string());
            }
            if list.is_empty() {
                return Err(ProtocolConfigError::Empty);
            }
            versions = Some(list);
        }
        versions
            .map(|versions| ProtocolConfig { versions })
            .ok_or(ProtocolConfigError::MissingKey)
    }

    /// 지원 version(선호 순).
    pub fn supported(&self) -> &[String] {
        &self.versions
    }

    /// 선호 version(목록 첫 항목).
    pub fn preferred(&self) -> &str {
        // from_config_str가 비어 있지 않은 목록만 만듭니다.
        self.versions.first().map_or("", String::as_str)
    }

    /// client 요청 version을 협상합니다(MCP Lifecycle "Version Negotiation").
    ///
    /// 지원하면 그 version([`Negotiation::Accepted`]), 아니면 선호 version([`Negotiation::Offered`])
    /// 입니다. 협상 자체는 실패하지 않습니다.
    pub fn negotiate(&self, requested: &str) -> Negotiation<'_> {
        match self.versions.iter().find(|v| *v == requested) {
            Some(version) => Negotiation::Accepted(version),
            None => Negotiation::Offered(self.preferred()),
        }
    }
}

/// version 협상 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiation<'a> {
    /// client 요청 version을 지원해 같은 version으로 응답합니다.
    Accepted(&'a str),
    /// client 요청 version을 지원하지 않아 서버 선호 version으로 응답합니다. 이 version을 지원하지
    /// 않는 client는 연결을 끊습니다.
    Offered(&'a str),
}

impl<'a> Negotiation<'a> {
    /// 응답 `protocolVersion`에 넣을 version.
    pub fn version(&self) -> &'a str {
        match self {
            Negotiation::Accepted(v) | Negotiation::Offered(v) => v,
        }
    }
}

/// 로그·오류 echo용으로 client가 정한 문자열(요청 version, method 이름)을 [`MAX_ECHO_CHARS`]자까지
/// 자릅니다(넘으면 끝에 `…`). 입력 길이와 무관하게 앞 [`MAX_ECHO_CHARS`]+1자만 봅니다.
pub fn truncate_for_log(requested: &str) -> String {
    if requested.chars().nth(MAX_ECHO_CHARS).is_some() {
        requested
            .chars()
            .take(MAX_ECHO_CHARS)
            .chain(['…'])
            .collect()
    } else {
        requested.to_string()
    }
}

/// `YYYY-MM-DD`(월 01~12, 일 01~31) 형식인지 확인합니다.
fn is_version(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month = (b[5] - b'0') * 10 + (b[6] - b'0');
    let day = (b[8] - b'0') * 10 + (b[9] - b'0');
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_config_pins_exact_versions() {
        let config = ProtocolConfig::builtin().unwrap();
        assert_eq!(config.supported(), ["2025-11-25", "2025-06-18"]);
        assert_eq!(config.preferred(), "2025-11-25");
        // 빌드 시 계산한 상수와 런타임 parser가 같은 선호 version을 봅니다.
        assert_eq!(PREFERRED_VERSION, config.preferred());
    }

    #[test]
    fn negotiation_accepts_exact_matches_and_offers_preferred_otherwise() {
        let config = ProtocolConfig::builtin().unwrap();
        assert_eq!(
            config.negotiate("2025-06-18"),
            Negotiation::Accepted("2025-06-18")
        );
        assert_eq!(
            config.negotiate("2025-11-25"),
            Negotiation::Accepted("2025-11-25")
        );
        // 미지원 version에는 오류 대신 선호(최신) version을 제안합니다(MCP Lifecycle).
        for unsupported in [
            "2024-11-05",
            "2025-03-26",
            "2099-01-01",
            "1.0.0",
            "",
            "2025-11-25 ",
            " 2025-11-25",
            "2025-11-25\n",
            "2025-11-2",
            "2025-11",
        ] {
            let negotiation = config.negotiate(unsupported);
            assert_eq!(
                negotiation,
                Negotiation::Offered("2025-11-25"),
                "{unsupported:?}"
            );
            assert_eq!(negotiation.version(), config.preferred());
        }
        // 다른 config에서는 그 config의 선호 version을 제안합니다.
        let other =
            ProtocolConfig::from_config_str("supported_versions: 2030-01-01, 2029-06-30").unwrap();
        assert_eq!(
            other.negotiate("2025-11-25"),
            Negotiation::Offered("2030-01-01")
        );
        assert_eq!(
            other.negotiate("2029-06-30"),
            Negotiation::Accepted("2029-06-30")
        );
    }

    #[test]
    fn long_requested_versions_are_truncated_for_logs() {
        let long = "9".repeat(1000);
        assert_eq!(truncate_for_log(&long).chars().count(), MAX_ECHO_CHARS + 1);
        assert_eq!(truncate_for_log("2024-11-05"), "2024-11-05");
    }

    #[test]
    fn malformed_configs_are_rejected() {
        use ProtocolConfigError::*;
        let cases: &[(&str, ProtocolConfigError)] = &[
            ("", MissingKey),
            ("# 주석만\n", MissingKey),
            ("supported_versions:", Empty),
            ("supported_versions:   ", Empty),
            (
                "supported_versions: 2025-11-25,",
                InvalidVersion { value: "".into() },
            ),
            (
                "supported_versions: 2025-13-01",
                InvalidVersion {
                    value: "2025-13-01".into(),
                },
            ),
            (
                "supported_versions: 2025-1-01",
                InvalidVersion {
                    value: "2025-1-01".into(),
                },
            ),
            (
                "supported_versions: 2025-11-25 2025-06-18",
                InvalidVersion {
                    value: "2025-11-25 2025-06-18".into(),
                },
            ),
            (
                "supported_versions: latest",
                InvalidVersion {
                    value: "latest".into(),
                },
            ),
            (
                "supported_versions: 2025-11-25, 2025-11-25",
                DuplicateVersion {
                    value: "2025-11-25".into(),
                },
            ),
            (
                "supported_versions: 2025-11-25\nsupported_versions: 2025-06-18",
                DuplicateKey,
            ),
            (
                "supported_versions: 2025-11-25\nminimum: 2025-06-18",
                UnknownKey {
                    key: "minimum".into(),
                },
            ),
            (
                "2025-11-25",
                UnknownKey {
                    key: "2025-11-25".into(),
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                ProtocolConfig::from_config_str(text).unwrap_err(),
                *expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn first_version_reads_the_key_line() {
        assert_eq!(
            first_version("# x\nsupported_versions: 2024-01-02, 2024-01-01\n"),
            "2024-01-02"
        );
        assert_eq!(first_version("supported_versions:2024-01-02"), "2024-01-02");
    }
}
