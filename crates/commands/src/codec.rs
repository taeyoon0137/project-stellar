//! command canonical 직렬화 (VII.1.2).
//!
//! storage에 의존하지 않는 canonical 규칙(V.1: 고정 폭 little-endian, `u32` 길이 프리픽스,
//! trailing bytes 거부)을 따릅니다.
//!
//! command 하나의 형식:
//!
//! | 필드 | 형식 |
//! |---|---|
//! | kind | `u16` LE ([`CommandKind`] numeric) |
//! | payload 길이 | `u32` LE |
//! | payload | 종류별 canonical bytes |
//!
//! 디코딩은 registry에 없는 kind를 [`CommandCodecError::UnknownKind`]로 거부합니다 — payload를
//! 해석하지 않고 거부하므로 알 수 없는 command가 실행 경로에 닿지 않습니다.

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::command::Command;
use crate::kind::CommandKind;

/// command 직렬화 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCodecError {
    /// registry에 없는 command kind.
    UnknownKind {
        /// 발견된 값.
        value: u16,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for CommandCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandCodecError::UnknownKind { value } => {
                write!(f, "알 수 없는 command kind: {value:#06x}")
            }
            CommandCodecError::Malformed { reason } => {
                write!(f, "command binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for CommandCodecError {}

/// canonical byte writer.
// 프로덕션 variant는 VII.2.x가 등록합니다. 그 전까지 일부 helper는 쓰이지 않습니다.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct Writer {
    buf: Vec<u8>,
}

// 프로덕션 variant는 VII.2.x가 등록합니다. 그 전까지 일부 helper는 쓰이지 않습니다.
#[allow(dead_code)]
impl Writer {
    pub(crate) fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn id(&mut self, id: StableId) {
        self.buf.extend_from_slice(&id.to_bytes());
    }

    /// `u32` 길이 프리픽스 + bytes.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.u32(len_u32(bytes.len()));
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn str(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    /// 목록 길이 프리픽스.
    pub(crate) fn len(&mut self, len: usize) {
        self.u32(len_u32(len));
    }
}

/// 길이를 `u32`로 줄입니다. command payload는 메모리 안 값이라 4 GiB를 넘지 않습니다
/// (넘으면 포화시켜 디코딩이 길이 불일치로 거부하게 합니다).
fn len_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// canonical byte reader.
// 프로덕션 variant는 VII.2.x가 등록합니다. 그 전까지 일부 helper는 쓰이지 않습니다.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

// 프로덕션 variant는 VII.2.x가 등록합니다. 그 전까지 일부 helper는 쓰이지 않습니다.
#[allow(dead_code)]
impl<'a> Reader<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CommandCodecError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.input.len())
            .ok_or(CommandCodecError::Malformed {
                reason: "입력 부족",
            })?;
        let slice = &self.input[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CommandCodecError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn bool(&mut self) -> Result<bool, CommandCodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CommandCodecError::Malformed {
                reason: "bool 값은 0 또는 1",
            }),
        }
    }

    pub(crate) fn u16(&mut self) -> Result<u16, CommandCodecError> {
        let mut raw = [0u8; 2];
        raw.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(raw))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CommandCodecError> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(raw))
    }

    pub(crate) fn id(&mut self) -> Result<StableId, CommandCodecError> {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(raw))
    }

    pub(crate) fn bytes(&mut self) -> Result<&'a [u8], CommandCodecError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    pub(crate) fn string(&mut self) -> Result<String, CommandCodecError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CommandCodecError::Malformed {
            reason: "UTF-8이 아닌 문자열",
        })
    }

    /// 목록 길이. 남은 입력보다 큰 값은(원소마다 최소 1바이트) 거부해 거대 할당을 막습니다.
    pub(crate) fn len(&mut self) -> Result<usize, CommandCodecError> {
        let len = self.u32()? as usize;
        if len > self.remaining() {
            return Err(CommandCodecError::Malformed {
                reason: "목록 길이가 남은 입력보다 큽니다",
            });
        }
        Ok(len)
    }

    pub(crate) fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    pub(crate) fn expect_end(&self) -> Result<(), CommandCodecError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(CommandCodecError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

/// command를 canonical bytes로 인코딩합니다.
pub fn encode_command(command: &Command) -> Vec<u8> {
    let mut out = Writer::new();
    encode_command_into(command, &mut out);
    out.into_bytes()
}

/// command를 writer에 덧붙입니다.
// 프로덕션 registry가 비어 있는 동안(VII.2.1 전) 프로덕션 빌드에서 `Command`는 값이 없는
// 타입이라 kind 이후 코드가 도달 불가로 판정됩니다.
#[cfg_attr(not(test), allow(unreachable_code))]
pub(crate) fn encode_command_into(command: &Command, out: &mut Writer) {
    let mut payload = Writer::new();
    encode_payload(command, &mut payload);
    out.u16(command.kind().as_u16());
    out.bytes(&payload.into_bytes());
}

fn encode_payload(command: &Command, out: &mut Writer) {
    let _ = &out;
    match *command {
        #[cfg(test)]
        Command::TestProbe(ref probe) => {
            out.id(probe.node);
            out.str(&probe.name);
            out.bool(probe.fail_after_apply);
        }
    }
}

/// [`encode_command`] bytes에서 command를 복원합니다. 입력 전체를 소비해야 합니다.
pub fn decode_command(bytes: &[u8]) -> Result<Command, CommandCodecError> {
    let mut reader = Reader::new(bytes);
    let command = decode_command_from(&mut reader)?;
    reader.expect_end()?;
    Ok(command)
}

/// reader에서 command 하나를 읽습니다.
pub(crate) fn decode_command_from(reader: &mut Reader<'_>) -> Result<Command, CommandCodecError> {
    let value = reader.u16()?;
    let kind = CommandKind::from_u16(value).ok_or(CommandCodecError::UnknownKind { value })?;
    let payload = reader.bytes()?;
    let mut payload = Reader::new(payload);
    let command = decode_payload(kind, &mut payload)?;
    payload.expect_end()?;
    Ok(command)
}

fn decode_payload(kind: CommandKind, input: &mut Reader<'_>) -> Result<Command, CommandCodecError> {
    let _ = &input;
    match kind {
        #[cfg(test)]
        CommandKind::TestProbe => Ok(Command::TestProbe(crate::command::TestProbe {
            node: input.id()?,
            name: input.string()?,
            fail_after_apply: input.bool()?,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::sid;
    use crate::command::TestProbe;

    fn probe() -> Command {
        Command::TestProbe(TestProbe {
            node: sid(11),
            name: "이름 🎨".into(),
            fail_after_apply: true,
        })
    }

    #[test]
    fn command_round_trips() {
        let command = probe();
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0xFFF0u16.to_le_bytes(), "kind LE");
        assert_eq!(decode_command(&bytes).unwrap(), command);
        assert_eq!(encode_command(&decode_command(&bytes).unwrap()), bytes);
    }

    #[test]
    fn unknown_command_kind_is_rejected() {
        // 검증 기준(VII.1.2): unknown command kind 거부.
        let mut bytes = encode_command(&probe());
        for value in [0x0000u16, 0x00FF, 0x7777, 0xFFFF] {
            bytes[..2].copy_from_slice(&value.to_le_bytes());
            assert_eq!(
                decode_command(&bytes),
                Err(CommandCodecError::UnknownKind { value })
            );
        }
        // payload가 비어 있어도 kind 검사가 먼저입니다.
        assert_eq!(
            decode_command(&[0x34, 0x12, 0, 0, 0, 0]),
            Err(CommandCodecError::UnknownKind { value: 0x1234 })
        );
    }

    #[test]
    fn malformed_framing_is_rejected() {
        let bytes = encode_command(&probe());
        for cut in [0, 1, 2, 5, bytes.len() - 1] {
            assert!(decode_command(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_command(&trailing),
            Err(CommandCodecError::Malformed {
                reason: "trailing bytes"
            })
        );
        // payload 길이가 실제보다 길면 거부.
        let mut long = bytes.clone();
        long[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_command(&long).is_err());
        // bool flag 2는 거부.
        let mut flag = bytes;
        let last = flag.len() - 1;
        flag[last] = 2;
        assert_eq!(
            decode_command(&flag),
            Err(CommandCodecError::Malformed {
                reason: "bool 값은 0 또는 1"
            })
        );
    }
}
