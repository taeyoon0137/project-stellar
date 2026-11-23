//! Stellar 소유 launch profile fixture (XXIX.2.6, ADR-0005 §4.3, ADR-0023 결정 9, ADR-0092 결정 4).
//!
//! CLI 종류와 지원 버전 범위마다 profile 하나를 JSON fixture(`crates/agent-host/profiles/*.json`)로 둡니다.
//! profile은 각 CLI 공식 문서의 비대화형 인터페이스와 구조화 출력 형식만 씁니다.
//!
//! - 실행 설정: 버전 확인·연결 상태 확인·세션 명령의 인자, 입력 방식, timeout.
//! - decoder profile id와 버전([`crate::decoder`]).
//! - CLI 프로세스에 전달할 환경 변수 **이름** 목록. 값은 넣지 않습니다(`=` 포함 항목과 값 field는 거부).
//! - 공급자 약관·공식 문서 검토 일자와 결과.
//!
//! 약관 검토 결과가 `allowed`가 아닌 profile은 `enabled`일 수 없고(load 거부), 비활성 profile과 지원 범위
//! 밖 버전은 실행하지 않습니다([`crate::preflight`]). 모르는 key는 거부합니다(fail closed).

use std::fmt;

pub use crate::decoder::DecoderRef;
use crate::json::{self, JsonLimits, JsonValue};

/// profile fixture 형식 이름.
pub const PROFILE_FORMAT: &str = "stellar-launch-profile/1";

/// 세션 명령 인자의 MCP 설정 파일 경로 자리 표시자.
pub const PLACEHOLDER_MCP_CONFIG: &str = "{mcp_config}";

/// 앱 번들에 함께 들어가는 공식 CLI profile fixture(파일 이름, 원문).
pub const OFFICIAL_PROFILE_SOURCES: [(&str, &str); 2] = [
    (
        "claude-code.json",
        include_str!("../profiles/claude-code.json"),
    ),
    ("codex.json", include_str!("../profiles/codex.json")),
];

/// CLI 버전(major.minor.patch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CliVersion {
    /// major.
    pub major: u32,
    /// minor.
    pub minor: u32,
    /// patch.
    pub patch: u32,
}

impl CliVersion {
    /// `1.2.3` 형식만 읽습니다.
    pub fn parse(text: &str) -> Option<CliVersion> {
        let mut parts = text.split('.');
        let mut next = || -> Option<u32> {
            let part = parts.next()?;
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = CliVersion {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// CLI 버전 출력에서 처음 나오는 `숫자.숫자.숫자` 낱말을 찾습니다. 없으면 `None`(판정 불가)입니다.
    pub fn find_in_output(output: &str) -> Option<CliVersion> {
        output
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find_map(|word| CliVersion::parse(word.trim_matches('.')))
    }
}

impl fmt::Display for CliVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// 지원 버전 범위 `[min, max_exclusive)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    /// 포함 하한.
    pub min: CliVersion,
    /// 제외 상한.
    pub max_exclusive: CliVersion,
}

impl VersionRange {
    /// 범위 안인지.
    pub fn contains(&self, version: CliVersion) -> bool {
        self.min <= version && version < self.max_exclusive
    }
}

/// 연결 상태 판정 규칙(각 CLI 공식 문서가 정한 방법, XXIX.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRule {
    /// 종료 코드만 씁니다: 0 = 연결됨, 1 = 연결 안 됨, 그 밖 = 판정 불가. 출력 내용은 읽지 않습니다.
    ExitCode,
}

/// 사용자 메시지를 CLI에 넘기는 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// 표준 입력에 stream JSON user message 한 줄씩(여러 turn, stdin을 열어 둠).
    StreamJsonUser,
    /// 표준 입력에 첫 메시지 원문을 쓰고 닫음(한 turn).
    StdinTextOnce,
}

/// 약관 검토 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermsResult {
    /// 2026-11-14 결정 1의 실행 방식을 명확히 허용함(제품 소유자 검토 완료).
    Allowed,
    /// 검토가 끝나지 않음.
    Pending,
    /// 허용하지 않음.
    NotAllowed,
}

impl TermsResult {
    /// 고정 이름(fixture 표기와 같음).
    pub fn name(self) -> &'static str {
        match self {
            TermsResult::Allowed => "allowed",
            TermsResult::Pending => "pending",
            TermsResult::NotAllowed => "not-allowed",
        }
    }

    fn parse(text: &str) -> Option<TermsResult> {
        match text {
            "allowed" => Some(TermsResult::Allowed),
            "pending" => Some(TermsResult::Pending),
            "not-allowed" => Some(TermsResult::NotAllowed),
            _ => None,
        }
    }
}

/// 약관·공식 문서 검토 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// 약관·사용 정책 검토 일자(YYYY-MM-DD).
    pub terms_date: String,
    /// 약관 검토 결과.
    pub terms_result: TermsResult,
    /// 공식 문서 검토 일자(YYYY-MM-DD).
    pub docs_date: String,
    /// 근거 공식 문서 URL.
    pub docs: Vec<String>,
}

/// timeout(ms). 값의 근거는 ADR-0092와 XXIX.2.4 보고서에 있습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// 버전 확인 명령 전체.
    pub version_ms: u64,
    /// 연결 상태 확인 명령 전체.
    pub status_ms: u64,
    /// 세션 시작 뒤 첫 stdout byte까지.
    pub startup_ms: u64,
    /// 중지 요청(SIGTERM) 뒤 SIGKILL까지의 유예.
    pub terminate_grace_ms: u64,
}

/// launch profile 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProfile {
    /// profile id(소문자·숫자·`.`·`-`).
    pub id: String,
    /// CLI 종류 id.
    pub cli: String,
    /// 사용자에게 보여 주는 CLI 이름(일반 텍스트, ADR-0005 §4.1 상표 표시).
    pub display_name: String,
    /// 활성 목록에 있는지(약관 검토 `allowed`일 때만 참일 수 있음).
    pub enabled: bool,
    /// 지원 버전 범위.
    pub versions: VersionRange,
    /// 실행 파일 서명 Team ID(Apple Developer Team ID). 없으면 신원 확인을 할 수 없어 실행하지 않습니다.
    pub team_id: Option<String>,
    /// 버전 확인 인자.
    pub version_command: Vec<String>,
    /// 연결 상태 확인 인자.
    pub status_command: Vec<String>,
    /// 연결 상태 판정 규칙.
    pub status_rule: StatusRule,
    /// 세션 인자(자리 표시자 [`PLACEHOLDER_MCP_CONFIG`] 허용).
    pub session_command: Vec<String>,
    /// 입력 방식.
    pub input: InputMode,
    /// decoder profile.
    pub decoder: DecoderRef,
    /// CLI 프로세스에 전달할 환경 변수 이름(값 없음).
    pub env: Vec<String>,
    /// timeout.
    pub timeouts: Timeouts,
    /// 검토 기록.
    pub review: Review,
}

/// profile load 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError {
    /// 오류 코드(고정 낱말).
    pub code: &'static str,
    /// 설명(profile 내용의 값은 넣지 않음).
    pub detail: String,
}

impl ProfileError {
    fn new(code: &'static str, detail: impl Into<String>) -> ProfileError {
        ProfileError {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "launch profile 오류({}): {}", self.code, self.detail)
    }
}

impl std::error::Error for ProfileError {}

const TOP_KEYS: [&str; 16] = [
    "format",
    "id",
    "cli",
    "display_name",
    "enabled",
    "versions",
    "team_id",
    "version_command",
    "status_command",
    "status_rule",
    "session_command",
    "input",
    "decoder",
    "env",
    "timeouts_ms",
    "review",
];

fn check_keys(value: &JsonValue, allowed: &[&str], at: &str) -> Result<(), ProfileError> {
    let pairs = value
        .as_object()
        .ok_or_else(|| ProfileError::new("not-object", format!("{at}는 object여야 합니다")))?;
    for (key, _) in pairs {
        if !allowed.contains(&key.as_str()) {
            return Err(ProfileError::new(
                "unknown-key",
                format!("{at}에 모르는 key `{key}`가 있습니다"),
            ));
        }
    }
    Ok(())
}

fn field<'a>(value: &'a JsonValue, key: &str, at: &str) -> Result<&'a JsonValue, ProfileError> {
    value
        .get(key)
        .ok_or_else(|| ProfileError::new("missing-key", format!("{at}.{key}가 없습니다")))
}

fn string_field(value: &JsonValue, key: &str, at: &str) -> Result<String, ProfileError> {
    field(value, key, at)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ProfileError::new("type", format!("{at}.{key}는 문자열이어야 합니다")))
}

fn strings(value: &JsonValue, key: &str, at: &str) -> Result<Vec<String>, ProfileError> {
    let items = field(value, key, at)?
        .as_array()
        .ok_or_else(|| ProfileError::new("type", format!("{at}.{key}는 배열이어야 합니다")))?;
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                ProfileError::new("type", format!("{at}.{key} 항목은 문자열이어야 합니다"))
            })
        })
        .collect()
}

fn millis(value: &JsonValue, key: &str) -> Result<u64, ProfileError> {
    let ms = field(value, key, "timeouts_ms")?
        .as_i64()
        .ok_or_else(|| ProfileError::new("type", format!("timeouts_ms.{key}는 정수여야 합니다")))?;
    if !(1..=3_600_000).contains(&ms) {
        return Err(ProfileError::new(
            "range",
            format!("timeouts_ms.{key}는 1..=3600000이어야 합니다"),
        ));
    }
    Ok(ms as u64)
}

fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// 환경 변수 이름 형식(POSIX 이식 이름: 영문 대문자·숫자·`_`, 숫자로 시작하지 않음).
pub fn is_env_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_uppercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && text.len() <= 128
}

impl LaunchProfile {
    /// fixture 원문을 읽고 검증합니다.
    pub fn parse(source: &str) -> Result<LaunchProfile, ProfileError> {
        let value = json::parse(source, JsonLimits::default())
            .map_err(|e| ProfileError::new("json", e.to_string()))?;
        check_keys(&value, &TOP_KEYS, "profile")?;
        if string_field(&value, "format", "profile")? != PROFILE_FORMAT {
            return Err(ProfileError::new("format", "format이 다릅니다"));
        }
        let id = string_field(&value, "id", "profile")?;
        let cli = string_field(&value, "cli", "profile")?;
        if !is_id(&id) || !is_id(&cli) {
            return Err(ProfileError::new("id", "id·cli 형식이 잘못되었습니다"));
        }
        let display_name = string_field(&value, "display_name", "profile")?;
        if display_name.is_empty() || display_name.chars().any(char::is_control) {
            return Err(ProfileError::new("display-name", "display_name 형식"));
        }
        let enabled = field(&value, "enabled", "profile")?
            .as_bool()
            .ok_or_else(|| ProfileError::new("type", "enabled는 bool이어야 합니다"))?;

        let versions_value = field(&value, "versions", "profile")?;
        check_keys(versions_value, &["min", "max_exclusive"], "versions")?;
        let version = |key: &str| -> Result<CliVersion, ProfileError> {
            CliVersion::parse(&string_field(versions_value, key, "versions")?)
                .ok_or_else(|| ProfileError::new("version", format!("versions.{key} 형식")))
        };
        let versions = VersionRange {
            min: version("min")?,
            max_exclusive: version("max_exclusive")?,
        };
        if versions.min >= versions.max_exclusive {
            return Err(ProfileError::new("version", "versions 범위가 비었습니다"));
        }

        let team_id = match field(&value, "team_id", "profile")? {
            JsonValue::Null => None,
            JsonValue::String(team) => {
                if stellar_local_ipc::code_identity::TeamId::parse(team).is_none() {
                    return Err(ProfileError::new("team-id", "team_id 형식"));
                }
                Some(team.clone())
            }
            _ => return Err(ProfileError::new("type", "team_id는 문자열 또는 null")),
        };

        let version_command = strings(&value, "version_command", "profile")?;
        let status_command = strings(&value, "status_command", "profile")?;
        let session_command = strings(&value, "session_command", "profile")?;
        if version_command.is_empty() || status_command.is_empty() || session_command.is_empty() {
            return Err(ProfileError::new("command", "명령 인자가 비었습니다"));
        }
        for arg in version_command
            .iter()
            .chain(&status_command)
            .chain(&session_command)
        {
            if arg.contains('\0') {
                return Err(ProfileError::new("command", "인자에 NUL"));
            }
            if arg.contains('{') && arg != PLACEHOLDER_MCP_CONFIG {
                return Err(ProfileError::new("placeholder", "모르는 자리 표시자"));
            }
        }
        if version_command
            .iter()
            .chain(&status_command)
            .any(|a| a == PLACEHOLDER_MCP_CONFIG)
        {
            return Err(ProfileError::new(
                "placeholder",
                "자리 표시자는 세션 명령에만 씁니다",
            ));
        }

        let status_rule = match string_field(&value, "status_rule", "profile")?.as_str() {
            "exit-code" => StatusRule::ExitCode,
            _ => return Err(ProfileError::new("status-rule", "모르는 status_rule")),
        };
        let input = match string_field(&value, "input", "profile")?.as_str() {
            "stream-json-user" => InputMode::StreamJsonUser,
            "stdin-text-once" => InputMode::StdinTextOnce,
            _ => return Err(ProfileError::new("input", "모르는 input")),
        };

        let decoder_value = field(&value, "decoder", "profile")?;
        check_keys(decoder_value, &["id", "version"], "decoder")?;
        let decoder = DecoderRef {
            id: string_field(decoder_value, "id", "decoder")?,
            version: field(decoder_value, "version", "decoder")?
                .as_i64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| ProfileError::new("type", "decoder.version은 정수"))?,
        };
        if crate::decoder::DecoderKind::resolve(&decoder).is_none() {
            return Err(ProfileError::new(
                "decoder",
                format!("모르는 decoder profile {}@{}", decoder.id, decoder.version),
            ));
        }

        let env_items = field(&value, "env", "profile")?
            .as_array()
            .ok_or_else(|| ProfileError::new("type", "env는 이름 배열이어야 합니다"))?;
        let mut env = Vec::new();
        for item in env_items {
            let name = item.as_str().ok_or_else(|| {
                ProfileError::new("env-value", "env 항목은 이름 문자열만 허용합니다")
            })?;
            if name.contains('=') {
                return Err(ProfileError::new(
                    "env-value",
                    "env에 값을 넣을 수 없습니다",
                ));
            }
            if !is_env_name(name) {
                return Err(ProfileError::new("env-name", "환경 변수 이름 형식"));
            }
            if env.iter().any(|n: &String| n == name) {
                return Err(ProfileError::new("env-name", "중복 환경 변수 이름"));
            }
            env.push(name.to_string());
        }

        let timeouts_value = field(&value, "timeouts_ms", "profile")?;
        check_keys(
            timeouts_value,
            &["version", "status", "startup", "terminate_grace"],
            "timeouts_ms",
        )?;
        let timeouts = Timeouts {
            version_ms: millis(timeouts_value, "version")?,
            status_ms: millis(timeouts_value, "status")?,
            startup_ms: millis(timeouts_value, "startup")?,
            terminate_grace_ms: millis(timeouts_value, "terminate_grace")?,
        };

        let review_value = field(&value, "review", "profile")?;
        check_keys(
            review_value,
            &["terms_date", "terms_result", "docs_date", "docs", "note"],
            "review",
        )?;
        let terms_date = string_field(review_value, "terms_date", "review")?;
        let docs_date = string_field(review_value, "docs_date", "review")?;
        if !is_date(&terms_date) || !is_date(&docs_date) {
            return Err(ProfileError::new("review-date", "검토 일자 형식"));
        }
        let terms_result =
            TermsResult::parse(&string_field(review_value, "terms_result", "review")?)
                .ok_or_else(|| ProfileError::new("review", "모르는 terms_result"))?;
        let docs = strings(review_value, "docs", "review")?;
        if docs.is_empty() || docs.iter().any(|d| !d.starts_with("https://")) {
            return Err(ProfileError::new("review", "근거 공식 문서 URL이 없습니다"));
        }
        if enabled && terms_result != TermsResult::Allowed {
            return Err(ProfileError::new(
                "enabled-without-terms",
                "약관 검토 결과가 allowed가 아닌 profile은 활성일 수 없습니다",
            ));
        }
        if enabled && team_id.is_none() {
            return Err(ProfileError::new(
                "enabled-without-identity",
                "서명 Team ID 없는 profile은 활성일 수 없습니다",
            ));
        }

        Ok(LaunchProfile {
            id,
            cli,
            display_name,
            enabled,
            versions,
            team_id,
            version_command,
            status_command,
            status_rule,
            session_command,
            input,
            decoder,
            env,
            timeouts,
            review: Review {
                terms_date,
                terms_result,
                docs_date,
                docs,
            },
        })
    }

    /// 실행 가능한 활성 profile인지(약관 검토 allowed + enabled).
    pub fn is_active(&self) -> bool {
        self.enabled && self.review.terms_result == TermsResult::Allowed
    }
}

/// 앱에 들어가는 공식 CLI profile 전체(파일 이름 순).
pub fn official_profiles() -> Result<Vec<LaunchProfile>, ProfileError> {
    OFFICIAL_PROFILE_SOURCES
        .iter()
        .map(|(name, source)| {
            LaunchProfile::parse(source)
                .map_err(|e| ProfileError::new(e.code, format!("{name}: {}", e.detail)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_and_find() {
        assert_eq!(
            CliVersion::parse("2.1.300"),
            Some(CliVersion {
                major: 2,
                minor: 1,
                patch: 300
            })
        );
        assert_eq!(CliVersion::parse("2.1"), None);
        assert_eq!(CliVersion::parse("2.1.3.4"), None);
        assert_eq!(CliVersion::parse("2.x.3"), None);
        assert_eq!(
            CliVersion::find_in_output("2.1.318 (Claude Code)\n").map(|v| v.to_string()),
            Some("2.1.318".into())
        );
        assert_eq!(
            CliVersion::find_in_output("codex-cli 0.98.0").map(|v| v.to_string()),
            Some("0.98.0".into())
        );
        assert_eq!(CliVersion::find_in_output("unknown"), None);
    }

    #[test]
    fn official_profiles_load_and_are_inactive_until_terms_review() {
        let profiles = official_profiles().unwrap();
        assert_eq!(profiles.len(), 2);
        for p in &profiles {
            assert!(!p.enabled, "{}: 약관 검토 전 활성 금지", p.id);
            assert!(!p.is_active());
            assert_eq!(p.review.terms_result, TermsResult::Pending);
            assert!(!p.env.is_empty());
        }
    }

    #[test]
    fn env_names_only() {
        assert!(is_env_name("HOME"));
        assert!(is_env_name("LC_ALL"));
        assert!(!is_env_name("home"));
        assert!(!is_env_name("1A"));
        assert!(!is_env_name("A=B"));
    }
}
