//! 앱이 시작한 bridge의 세션 결합 (XIX.1.8).
//!
//! 앱이 내장 에이전트 세션을 시작하면(ADR-0005 §4) 그 세션 전용 **결합 token**을 만들고, 공급자
//! CLI가 띄우는 `stellar-mcp`에 환경 변수 [`BINDING_ENV`]로 넘깁니다. bridge는 handshake HELLO에
//! token을 담아 보내고(`STELLAR-IPC/1 HELLO <client-nonce> BIND <token>`), 앱은 그 연결을 token을 만든
//! 세션 하나에만 묶습니다.
//!
//! ## 결합 수단과 근거
//!
//! - **token**: OS CSPRNG(`/dev/urandom`)의 256 bit 값, 소문자 16진수 64자. 세션마다 새로 만들고
//!   세션이 끝나면([`SessionBindings::end`]) 즉시 무효입니다. 끝난 세션의 token은 다시 받지 않습니다.
//! - **전달 경로 = 환경 변수**: 공급자 CLI들은 MCP 서버 설정에 서버 프로세스 환경 변수(`env`)를
//!   지정하는 방법을 공식 문서로 제공합니다(예: Claude Code `--mcp-config`의 `mcpServers.<name>.env`).
//!   명령줄 인자는 macOS `ps`로 다른 사용자도 볼 수 있지만 다른 프로세스의 환경 변수는 같은 사용자나
//!   root만 읽을 수 있으므로(macOS `ps(1)` `-E`, `sysctl` `KERN_PROCARGS2` 접근 제한) 인자 대신 환경
//!   변수로 넘깁니다. 같은 사용자의 프로세스는 이미 같은 권한 경계 안이며(아키텍처 §37), 그 경계는
//!   XIX.1.6·XIX.1.7의 peer 인증이 담당합니다.
//! - **검증**: 앱은 peer 인증(XIX.1.6)과 nonce 검사(XIX.1.3)를 통과한 연결에서만 token을 봅니다.
//!   비교는 상수 시간이고, 모르는 token·끝난 세션의 token은 `REJECT binding-rejected`로 거부합니다.
//!
//! ## 결합은 권한이 아닙니다
//!
//! 결합 결과([`BindingSession`])는 "이 연결이 어느 세션의 것인지"뿐입니다. 권한·승인 상태를 담지
//! 않으며 XIX.3 승인을 대체하지 않습니다. 결합된 연결도 권한 0에서 시작합니다.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::{self, Read};
use std::sync::Mutex;

/// bridge가 결합 token을 받는 환경 변수 이름.
pub const BINDING_ENV: &str = "STELLAR_MCP_SESSION_BINDING";
/// token byte 수.
pub const TOKEN_BYTES: usize = 32;
/// 16진수 표기 길이.
pub const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;
/// 끝난 세션 token을 기억하는 개수(진단용 구분, 넘치면 오래된 것부터 잊고 `Unknown`으로 거부).
pub const ENDED_MEMORY: usize = 1024;

/// 세션 결합 token. `Debug`·로그에 값을 쓰지 않습니다.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingToken([u8; TOKEN_BYTES]);

impl fmt::Debug for BindingToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BindingToken(<redacted>)")
    }
}

impl BindingToken {
    /// OS CSPRNG에서 새 token을 만듭니다.
    pub fn generate() -> io::Result<BindingToken> {
        let mut bytes = [0u8; TOKEN_BYTES];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(BindingToken(bytes))
    }

    /// byte 값에서 만듭니다(테스트·재현용).
    pub fn from_bytes(bytes: [u8; TOKEN_BYTES]) -> BindingToken {
        BindingToken(bytes)
    }

    /// 소문자 16진수 64자를 읽습니다. 대문자·길이 불일치·기타 문자는 거부합니다.
    pub fn parse_hex(text: &str) -> Option<BindingToken> {
        let b = text.as_bytes();
        if b.len() != TOKEN_HEX_LEN {
            return None;
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; TOKEN_BYTES];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (digit(b[2 * i])? << 4) | digit(b[2 * i + 1])?;
        }
        Some(BindingToken(out))
    }

    /// 소문자 16진수 64자.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 상수 시간 비교.
    pub fn constant_time_eq(&self, other: &BindingToken) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

/// 환경 변수 값을 읽습니다. 없으면 `Ok(None)`, 형식이 틀리면 `Err(())`(값은 돌려주지 않음).
pub fn token_from_env_value(value: Option<&str>) -> Result<Option<BindingToken>, MalformedToken> {
    match value {
        None => Ok(None),
        Some(text) => BindingToken::parse_hex(text)
            .map(Some)
            .ok_or(MalformedToken),
    }
}

/// 환경 변수의 token 형식 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedToken;

impl fmt::Display for MalformedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{BINDING_ENV} 값이 소문자 16진수 {TOKEN_HEX_LEN}자가 아닙니다"
        )
    }
}

impl std::error::Error for MalformedToken {}

/// 앱이 정한 세션 식별(앱 runtime의 AgentSession id를 그대로 담습니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingSession(pub u64);

/// 결합 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingError {
    /// 이 앱이 만든 적 없는 token(다른 앱·위조 포함).
    Unknown,
    /// 이미 끝난 세션의 token.
    Ended,
    /// 이 listener는 결합을 받지 않습니다(외부 client 전용).
    Unsupported,
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::Unknown => f.write_str("모르는 세션 결합 token입니다"),
            BindingError::Ended => f.write_str("끝난 세션의 결합 token입니다"),
            BindingError::Unsupported => f.write_str("이 endpoint는 세션 결합을 받지 않습니다"),
        }
    }
}

impl std::error::Error for BindingError {}

#[derive(Debug, Default)]
struct Table {
    active: HashMap<BindingSession, BindingToken>,
    ended: HashSet<BindingToken>,
    ended_order: VecDeque<BindingToken>,
}

/// 앱의 세션 결합 표. IPC I/O 스레드와 세션 관리 쪽이 함께 씁니다(내부 잠금).
#[derive(Debug, Default)]
pub struct SessionBindings {
    table: Mutex<Table>,
}

impl SessionBindings {
    /// 빈 표.
    pub fn new() -> SessionBindings {
        SessionBindings::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        // 잠금 중 panic이 나도 표 내용은 일관된 상태로만 바뀌므로(삽입·삭제 단위) 계속 씁니다.
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 세션의 새 결합 token을 만듭니다. 이미 있으면 이전 token을 끝난 것으로 바꿉니다.
    pub fn issue(&self, session: BindingSession) -> io::Result<BindingToken> {
        let token = BindingToken::generate()?;
        let mut table = self.lock();
        if let Some(previous) = table.active.insert(session, token) {
            remember_ended(&mut table, previous);
        }
        Ok(token)
    }

    /// 세션을 끝냅니다. 그 token은 즉시 무효입니다. 세션이 있었으면 `true`.
    pub fn end(&self, session: BindingSession) -> bool {
        let mut table = self.lock();
        match table.active.remove(&session) {
            Some(token) => {
                remember_ended(&mut table, token);
                true
            }
            None => false,
        }
    }

    /// token을 만든 세션을 찾습니다. 모든 활성 token과 상수 시간으로 비교합니다.
    pub fn verify(&self, token: &BindingToken) -> Result<BindingSession, BindingError> {
        let table = self.lock();
        let mut found = None;
        for (session, candidate) in &table.active {
            if candidate.constant_time_eq(token) {
                found = Some(*session);
            }
        }
        match found {
            Some(session) => Ok(session),
            None if table.ended.contains(token) => Err(BindingError::Ended),
            None => Err(BindingError::Unknown),
        }
    }

    /// 활성 세션 수.
    pub fn active_count(&self) -> usize {
        self.lock().active.len()
    }
}

fn remember_ended(table: &mut Table, token: BindingToken) {
    if table.ended.insert(token) {
        table.ended_order.push_back(token);
        while table.ended_order.len() > ENDED_MEMORY {
            if let Some(old) = table.ended_order.pop_front() {
                table.ended.remove(&old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hex_round_trip_and_debug_is_redacted() {
        let token = BindingToken::generate().unwrap();
        assert_eq!(BindingToken::parse_hex(&token.to_hex()), Some(token));
        assert_eq!(token.to_hex().len(), TOKEN_HEX_LEN);
        assert!(!format!("{token:?}").contains(&token.to_hex()));
        for bad in ["", "AB", &"A".repeat(64), &"g".repeat(64), &"a".repeat(63)] {
            assert_eq!(BindingToken::parse_hex(bad), None, "{bad}");
        }
        assert_eq!(token_from_env_value(None), Ok(None));
        assert_eq!(token_from_env_value(Some("bad")), Err(MalformedToken));
        assert_eq!(token_from_env_value(Some(&token.to_hex())), Ok(Some(token)));
    }

    #[test]
    fn token_binds_only_its_own_session_and_dies_with_it() {
        // 검증 기준(XIX.1.8): 다른 세션·종료된 세션 결합 거부.
        let bindings = SessionBindings::new();
        let a = bindings.issue(BindingSession(1)).unwrap();
        let b = bindings.issue(BindingSession(2)).unwrap();
        assert_ne!(a, b);
        assert_eq!(bindings.verify(&a), Ok(BindingSession(1)));
        assert_eq!(bindings.verify(&b), Ok(BindingSession(2)));
        // 다른 앱·위조 token.
        let forged = BindingToken::from_bytes([7; TOKEN_BYTES]);
        assert_eq!(bindings.verify(&forged), Err(BindingError::Unknown));
        // 세션이 끝나면 그 token은 무효이고 다른 세션에는 영향이 없습니다.
        assert!(bindings.end(BindingSession(1)));
        assert_eq!(bindings.verify(&a), Err(BindingError::Ended));
        assert_eq!(bindings.verify(&b), Ok(BindingSession(2)));
        assert!(!bindings.end(BindingSession(1)));
        // 다시 시작한 세션은 새 token이며 이전 token은 계속 무효입니다.
        let a2 = bindings.issue(BindingSession(1)).unwrap();
        assert_ne!(a2, a);
        assert_eq!(bindings.verify(&a), Err(BindingError::Ended));
        assert_eq!(bindings.verify(&a2), Ok(BindingSession(1)));
        // 같은 세션 재발급도 이전 token을 끝냅니다.
        let b2 = bindings.issue(BindingSession(2)).unwrap();
        assert_eq!(bindings.verify(&b), Err(BindingError::Ended));
        assert_eq!(bindings.verify(&b2), Ok(BindingSession(2)));
        assert_eq!(bindings.active_count(), 2);
    }

    #[test]
    fn ended_memory_is_bounded() {
        let bindings = SessionBindings::new();
        let first = bindings.issue(BindingSession(0)).unwrap();
        bindings.end(BindingSession(0));
        for n in 1..=(ENDED_MEMORY as u64) {
            bindings.issue(BindingSession(n)).unwrap();
            bindings.end(BindingSession(n));
        }
        // 잊은 token도 여전히 거부합니다(Unknown).
        assert_eq!(bindings.verify(&first), Err(BindingError::Unknown));
        assert_eq!(bindings.active_count(), 0);
    }
}
