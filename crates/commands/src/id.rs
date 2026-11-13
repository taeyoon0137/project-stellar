//! command 식별자와 출처 (VII.1.1).
//!
//! - [`CommandId`]: command(또는 transaction) 실행 단위의 안정 식별자. [`StableId`] 기반
//!   newtype이라 node id·[`WindowSessionId`]와 타입 수준에서 섞이지 않습니다.
//! - [`WindowSessionId`]: 창 편집 세션 식별자. core-types(V.9.1)의 정의를 그대로 씁니다.
//! - [`CommandOrigin`]: 편집 출처. `User`, `System`, `Migration`, `FutureAgent`를 가집니다.
//!   `FutureAgent`는 agent 편집도 일반 command 경로를 지난다는 원칙(기술 아키텍처 §31.4)에
//!   맞춰 **출처 표식과 provenance만** 정의합니다 — lease·승인·rollback 동작은 MCP 로드맵
//!   (Phase XIX)의 몫이며 여기서는 구현하지 않습니다. provenance([`AgentProvenance`])는
//!   history 항목에 남아, 나중에 사용자가 특정 agent 편집 세션의 변경만 골라 되돌릴 때
//!   기준으로 씁니다.
//!
//! 직렬화는 canonical binary(V.1: 고정 폭, LE)입니다. `CommandId`는 16바이트 StableId,
//! `CommandOrigin`은 tag `u8` + (FutureAgent만) 16바이트 edit session id입니다. tag 값은
//! 저장·journal 포맷(XXV.1)의 일부이므로 바꾸거나 재사용하지 않습니다.

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::{ParseStableIdError, StableId};

pub use stellar_core_types::ids::WindowSessionId;

/// command(또는 transaction) 실행 단위의 안정 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandId(StableId);

impl CommandId {
    /// `StableId`로부터 만듭니다.
    pub const fn from_stable_id(id: StableId) -> CommandId {
        CommandId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }

    /// 16바이트(big-endian StableId) 표현.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0.to_bytes()
    }

    /// 16바이트 표현에서 복원합니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> CommandId {
        CommandId(StableId::from_bytes(bytes))
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for CommandId {
    type Err = ParseStableIdError;

    /// 표준 hyphen UUID 문자열을 파싱합니다(StableId와 같은 규칙).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<StableId>().map(CommandId)
    }
}

/// agent 편집 provenance(기술 아키텍처 §31.4·§31.5).
///
/// 한 agent edit 세션(승인된 lease 단위)을 식별합니다. history 항목이 이 값을 보존하므로
/// 나중에 사용자가 그 세션의 변경만 골라 되돌릴 수 있습니다. 세션 수명·승인 동작은
/// Phase XIX에서 정의합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentProvenance {
    edit_session: StableId,
}

impl AgentProvenance {
    /// agent edit 세션 id로 만듭니다.
    pub const fn new(edit_session: StableId) -> AgentProvenance {
        AgentProvenance { edit_session }
    }

    /// agent edit 세션 id.
    pub const fn edit_session(&self) -> StableId {
        self.edit_session
    }
}

/// 편집 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandOrigin {
    /// 사용자가 직접 수행한 편집.
    User,
    /// 앱이 만든 편집(외부 변경 반영용 synthetic command, selection 정리 등).
    System,
    /// 문서 schema migration이 만든 편집.
    Migration,
    /// 향후 agent 편집(Phase XIX). 출처 표식과 provenance만 정의하며 동작은 구현하지
    /// 않습니다.
    FutureAgent(AgentProvenance),
}

/// [`CommandOrigin`] 디코딩 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginDecodeError {
    /// 입력이 부족하거나 남습니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 알 수 없는 origin tag.
    UnknownTag {
        /// 발견된 tag.
        tag: u8,
    },
}

impl fmt::Display for OriginDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OriginDecodeError::Malformed { reason } => {
                write!(f, "CommandOrigin binary 형식 위반: {reason}")
            }
            OriginDecodeError::UnknownTag { tag } => {
                write!(f, "알 수 없는 CommandOrigin tag: {tag:#04x}")
            }
        }
    }
}

impl std::error::Error for OriginDecodeError {}

impl CommandOrigin {
    const TAG_USER: u8 = 0;
    const TAG_SYSTEM: u8 = 1;
    const TAG_MIGRATION: u8 = 2;
    const TAG_FUTURE_AGENT: u8 = 3;

    /// 안정 tag. 저장·journal 포맷 계약이므로 값을 바꾸지 않습니다.
    pub const fn tag(&self) -> u8 {
        match self {
            CommandOrigin::User => CommandOrigin::TAG_USER,
            CommandOrigin::System => CommandOrigin::TAG_SYSTEM,
            CommandOrigin::Migration => CommandOrigin::TAG_MIGRATION,
            CommandOrigin::FutureAgent(_) => CommandOrigin::TAG_FUTURE_AGENT,
        }
    }

    /// agent 편집이면 그 provenance.
    pub const fn agent_provenance(&self) -> Option<AgentProvenance> {
        match self {
            CommandOrigin::FutureAgent(provenance) => Some(*provenance),
            _ => None,
        }
    }

    /// canonical bytes를 `out`에 덧붙입니다: tag `u8` + (FutureAgent) 16바이트 세션 id.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        if let CommandOrigin::FutureAgent(provenance) = self {
            out.extend_from_slice(&provenance.edit_session.to_bytes());
        }
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// [`CommandOrigin::encode`]의 역. 입력 전체를 정확히 소비해야 합니다.
    pub fn decode(bytes: &[u8]) -> Result<CommandOrigin, OriginDecodeError> {
        let (origin, used) = CommandOrigin::decode_prefix(bytes)?;
        if used != bytes.len() {
            return Err(OriginDecodeError::Malformed {
                reason: "trailing bytes",
            });
        }
        Ok(origin)
    }

    /// 입력 앞부분에서 origin 하나를 읽고 소비한 바이트 수를 함께 반환합니다.
    pub fn decode_prefix(bytes: &[u8]) -> Result<(CommandOrigin, usize), OriginDecodeError> {
        let Some((&tag, rest)) = bytes.split_first() else {
            return Err(OriginDecodeError::Malformed {
                reason: "입력 부족",
            });
        };
        match tag {
            CommandOrigin::TAG_USER => Ok((CommandOrigin::User, 1)),
            CommandOrigin::TAG_SYSTEM => Ok((CommandOrigin::System, 1)),
            CommandOrigin::TAG_MIGRATION => Ok((CommandOrigin::Migration, 1)),
            CommandOrigin::TAG_FUTURE_AGENT => {
                let Some(id) = rest.get(..16) else {
                    return Err(OriginDecodeError::Malformed {
                        reason: "agent edit session id 부족",
                    });
                };
                let mut raw = [0u8; 16];
                raw.copy_from_slice(id);
                Ok((
                    CommandOrigin::FutureAgent(AgentProvenance::new(StableId::from_bytes(raw))),
                    17,
                ))
            }
            other => Err(OriginDecodeError::UnknownTag { tag: other }),
        }
    }
}

/// `CommandId`와 `WindowSessionId`는 비교할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_commands::id::{CommandId, WindowSessionId};
/// use stellar_core_types::ids::StableId;
/// let id = StableId::new_v7(1, 0, 1);
/// let c = CommandId::from_stable_id(id);
/// let w = WindowSessionId::from_stable_id(id);
/// let _ = c == w;
/// ```
///
/// `CommandId`를 `WindowSessionId` 자리에 대입할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_commands::id::{CommandId, WindowSessionId};
/// use stellar_core_types::ids::StableId;
/// let c = CommandId::from_stable_id(StableId::new_v7(1, 0, 1));
/// let _w: WindowSessionId = c;
/// ```
///
/// node `StableId`를 `CommandId` 자리에 그대로 쓸 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_commands::id::CommandId;
/// use stellar_core_types::ids::StableId;
/// let _c: CommandId = StableId::new_v7(1, 0, 1);
/// ```
#[cfg(doctest)]
struct CommandIdCompileFailDocs;

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0abc, n.wrapping_mul(0x9E37_79B9))
    }

    #[test]
    fn command_id_binary_and_string_round_trip() {
        let id = CommandId::from_stable_id(sid(0x0192_3c4d_5e6f));
        assert_eq!(CommandId::from_bytes(id.to_bytes()), id);
        let text = id.to_string();
        assert_eq!(text, sid(0x0192_3c4d_5e6f).to_string());
        assert_eq!(text.parse::<CommandId>().unwrap(), id);
        assert!("not-a-command-id".parse::<CommandId>().is_err());
    }

    #[test]
    fn origin_round_trips_every_variant() {
        let origins = [
            CommandOrigin::User,
            CommandOrigin::System,
            CommandOrigin::Migration,
            CommandOrigin::FutureAgent(AgentProvenance::new(sid(7))),
        ];
        for origin in origins {
            let bytes = origin.encode();
            assert_eq!(CommandOrigin::decode(&bytes).unwrap(), origin);
            assert_eq!(bytes[0], origin.tag());
        }
        // tag 값은 저장 계약입니다.
        assert_eq!(origins.map(|o| o.tag()), [0, 1, 2, 3], "origin tag golden");
        assert_eq!(
            CommandOrigin::FutureAgent(AgentProvenance::new(sid(7))).agent_provenance(),
            Some(AgentProvenance::new(sid(7)))
        );
        assert_eq!(CommandOrigin::User.agent_provenance(), None);
    }

    #[test]
    fn origin_decode_rejects_malformed_input() {
        assert_eq!(
            CommandOrigin::decode(&[]),
            Err(OriginDecodeError::Malformed {
                reason: "입력 부족"
            })
        );
        assert_eq!(
            CommandOrigin::decode(&[9]),
            Err(OriginDecodeError::UnknownTag { tag: 9 })
        );
        assert_eq!(
            CommandOrigin::decode(&[3, 1, 2]),
            Err(OriginDecodeError::Malformed {
                reason: "agent edit session id 부족"
            })
        );
        assert_eq!(
            CommandOrigin::decode(&[0, 0]),
            Err(OriginDecodeError::Malformed {
                reason: "trailing bytes"
            })
        );
        let mut agent = CommandOrigin::FutureAgent(AgentProvenance::new(sid(3))).encode();
        let (origin, used) = CommandOrigin::decode_prefix(&agent).unwrap();
        assert_eq!(used, 17);
        assert_eq!(origin.tag(), 3);
        agent.push(0xFF);
        assert!(CommandOrigin::decode(&agent).is_err());
    }

    #[test]
    fn window_session_id_is_the_core_type() {
        // 창 세션 id는 core-types 정의를 재사용합니다(별도 정의 없음).
        let w: stellar_core_types::ids::WindowSessionId = WindowSessionId::from_stable_id(sid(5));
        assert_eq!(w.stable_id(), sid(5));
    }
}
