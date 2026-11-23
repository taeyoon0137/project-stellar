//! handshake 뒤 bridge↔app 메시지 frame (XXIX.3.4 앞단, ADR-0124 결정 3).
//!
//! handshake(XIX.1.3)를 마친 연결에서 오가는 메시지를 길이 접두 binary frame으로 정합니다. 이 crate는 core-types만
//! 의존하므로 frame은 StableId·문자열·byte열만 실어 나릅니다. 값의 해석(revision 표기, 전역 resource 종류, command
//! canonical bytes)은 앱 runtime이 기존 공개 함수로 합니다.
//!
//! ```text
//! frame   := "SF" version:u8 kind:u8 request:u64le length:u32le payload[length]
//! id      := 16 byte(StableId)
//! text    := length:u32le utf8[length]
//! bytes   := length:u32le byte[length]
//! list<T> := count:u32le T[count]
//! ```
//!
//! | 종류 | 방향 | payload |
//! |---|---|---|
//! | 1 `BeginEdit` | bridge→app | document:id, targets:list<id>, externals:list<(kind:text, id)>, description:text |
//! | 2 `Propose` | bridge→app | base_revision:text, document:id, commands:list<(kind:text, canonical:bytes)> |
//! | 3 `Complete` | bridge→app | 없음 |
//! | 0x81 `Reply` | app→bridge | result:text(결과 종류 이름), message:text |
//!
//! 해석은 fail closed입니다. 모르는 magic·version·종류, [`MAX_FRAME_BYTES`]를 넘는 길이, payload 끝 뒤에 남는 byte, UTF-8이
//! 아닌 문자열은 모두 [`FrameError`]이고 연결을 닫는 사유입니다. 이 module은 OS 기능을 쓰지 않습니다.

use std::fmt;
use std::io::{self, Read, Write};

use stellar_core_types::ids::StableId;

/// frame magic.
pub const MAGIC: [u8; 2] = *b"SF";

/// frame 형식 version.
pub const VERSION: u8 = 1;

/// frame 하나(머리 포함)의 최대 byte 수. MCP 메시지 상한(agent-protocol `MAX_MESSAGE_BYTES`, 1 MiB)과 같습니다.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// frame 머리 byte 수.
pub const HEADER_BYTES: usize = 2 + 1 + 1 + 8 + 4;

/// bridge가 보내는 요청 frame과 app 응답 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// `document.begin_agent_edit` 대상 선언(브리지가 형식을 검사한 값).
    BeginEdit {
        /// 요청 id(응답에 그대로 실음).
        request: u64,
        /// 대상 문서.
        document: StableId,
        /// target root(제출 순서).
        targets: Vec<StableId>,
        /// 전역 resource(종류 이름, id).
        externals: Vec<(String, StableId)>,
        /// 사용자에게 보일 작업 설명(데이터로만 다룸).
        description: String,
    },
    /// command proposal(사용자 승인 대기).
    Propose {
        /// 요청 id.
        request: u64,
        /// 요청이 기준으로 삼은 프로젝트 revision 표기.
        base_revision: String,
        /// 대상 문서.
        document: StableId,
        /// command(종류 이름, canonical bytes).
        commands: Vec<(String, Vec<u8>)>,
    },
    /// 정상 완료(CompleteAgentEdit).
    Complete {
        /// 요청 id.
        request: u64,
    },
    /// 앱 응답.
    Reply {
        /// 응답하는 요청 id.
        request: u64,
        /// 결과 종류 이름(ADR-0085 `ResultKind::name`).
        result: String,
        /// 문장.
        message: String,
    },
}

impl Frame {
    /// 요청 id.
    pub fn request(&self) -> u64 {
        match self {
            Frame::BeginEdit { request, .. }
            | Frame::Propose { request, .. }
            | Frame::Complete { request }
            | Frame::Reply { request, .. } => *request,
        }
    }

    fn kind(&self) -> u8 {
        match self {
            Frame::BeginEdit { .. } => 1,
            Frame::Propose { .. } => 2,
            Frame::Complete { .. } => 3,
            Frame::Reply { .. } => 0x81,
        }
    }
}

/// frame 해석·쓰기 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// 연결이 frame 경계에서 끝났습니다(정상 종료).
    Closed,
    /// frame 중간에서 끝났습니다.
    Truncated,
    /// magic이 다릅니다.
    BadMagic,
    /// 모르는 version.
    UnsupportedVersion(u8),
    /// 모르는 종류.
    UnknownKind(u8),
    /// 상한을 넘는 frame.
    TooLarge(usize),
    /// payload 형식 오류(남는 byte, UTF-8 아님, 개수 초과).
    Malformed(&'static str),
    /// 입출력 오류.
    Io(io::ErrorKind),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Closed => f.write_str("연결이 닫혔습니다"),
            FrameError::Truncated => f.write_str("frame이 중간에서 끝났습니다"),
            FrameError::BadMagic => f.write_str("frame magic이 다릅니다"),
            FrameError::UnsupportedVersion(v) => write!(f, "지원하지 않는 frame version {v}"),
            FrameError::UnknownKind(k) => write!(f, "모르는 frame 종류 {k}"),
            FrameError::TooLarge(n) => {
                write!(f, "frame {n} byte는 상한 {MAX_FRAME_BYTES}를 넘습니다")
            }
            FrameError::Malformed(why) => write!(f, "frame 형식 오류: {why}"),
            FrameError::Io(kind) => write!(f, "frame 입출력 오류: {kind:?}"),
        }
    }
}

impl std::error::Error for FrameError {}

struct Writer(Vec<u8>);

impl Writer {
    fn id(&mut self, id: StableId) {
        self.0.extend_from_slice(&id.to_bytes());
    }
    fn len(&mut self, n: usize) {
        self.0
            .extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }
    fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], FrameError> {
        if self.0.len() < n {
            return Err(FrameError::Malformed("payload가 짧습니다"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn id(&mut self) -> Result<StableId, FrameError> {
        let mut raw = [0; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(raw))
    }
    fn len(&mut self) -> Result<usize, FrameError> {
        let mut raw = [0; 4];
        raw.copy_from_slice(self.take(4)?);
        let n = u32::from_le_bytes(raw) as usize;
        // 남은 payload보다 큰 개수·길이는 할당 전에 거부합니다.
        if n > self.0.len() {
            return Err(FrameError::Malformed("길이가 payload를 넘습니다"));
        }
        Ok(n)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, FrameError> {
        let n = self.len()?;
        Ok(self.take(n)?.to_vec())
    }
    fn text(&mut self) -> Result<String, FrameError> {
        String::from_utf8(self.bytes()?).map_err(|_| FrameError::Malformed("UTF-8이 아닙니다"))
    }
    fn list<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, FrameError>,
    ) -> Result<Vec<T>, FrameError> {
        let n = self.len()?;
        (0..n).map(|_| item(self)).collect()
    }
    fn end(&self) -> Result<(), FrameError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Malformed("payload 뒤에 byte가 남았습니다"))
        }
    }
}

/// frame을 bytes로 씁니다. 상한을 넘으면 [`FrameError::TooLarge`]입니다.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let mut payload = Writer(Vec::new());
    match frame {
        Frame::BeginEdit {
            document,
            targets,
            externals,
            description,
            ..
        } => {
            payload.id(*document);
            payload.len(targets.len());
            for target in targets {
                payload.id(*target);
            }
            payload.len(externals.len());
            for (kind, id) in externals {
                payload.text(kind);
                payload.id(*id);
            }
            payload.text(description);
        }
        Frame::Propose {
            base_revision,
            document,
            commands,
            ..
        } => {
            payload.text(base_revision);
            payload.id(*document);
            payload.len(commands.len());
            for (kind, bytes) in commands {
                payload.text(kind);
                payload.bytes(bytes);
            }
        }
        Frame::Complete { .. } => {}
        Frame::Reply {
            result, message, ..
        } => {
            payload.text(result);
            payload.text(message);
        }
    }
    let total = HEADER_BYTES + payload.0.len();
    if total > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(total));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(frame.kind());
    out.extend_from_slice(&frame.request().to_le_bytes());
    out.extend_from_slice(&(payload.0.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload.0);
    Ok(out)
}

fn decode_payload(kind: u8, request: u64, payload: &[u8]) -> Result<Frame, FrameError> {
    let mut r = Reader(payload);
    let frame = match kind {
        1 => Frame::BeginEdit {
            request,
            document: r.id()?,
            targets: r.list(Reader::id)?,
            externals: r.list(|r| Ok((r.text()?, r.id()?)))?,
            description: r.text()?,
        },
        2 => Frame::Propose {
            request,
            base_revision: r.text()?,
            document: r.id()?,
            commands: r.list(|r| Ok((r.text()?, r.bytes()?)))?,
        },
        3 => Frame::Complete { request },
        0x81 => Frame::Reply {
            request,
            result: r.text()?,
            message: r.text()?,
        },
        other => return Err(FrameError::UnknownKind(other)),
    };
    r.end()?;
    Ok(frame)
}

/// frame bytes 하나를 해석합니다(머리 포함, 정확히 한 frame).
pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
    let mut cursor = bytes;
    let frame = read_frame(&mut cursor)?;
    if !cursor.is_empty() {
        return Err(FrameError::Malformed("frame 뒤에 byte가 남았습니다"));
    }
    Ok(frame)
}

fn read_exact_or(reader: &mut impl Read, buf: &mut [u8], at_start: bool) -> Result<(), FrameError> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) if at_start && filled == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e.kind())),
        }
    }
    Ok(())
}

/// 연결에서 frame 하나를 읽습니다. frame 경계에서 연결이 끝나면 [`FrameError::Closed`]입니다. 길이를 확인한 뒤에만
/// payload를 할당합니다.
pub fn read_frame(reader: &mut impl Read) -> Result<Frame, FrameError> {
    let mut header = [0u8; HEADER_BYTES];
    read_exact_or(reader, &mut header, true)?;
    if header[..2] != MAGIC {
        return Err(FrameError::BadMagic);
    }
    if header[2] != VERSION {
        return Err(FrameError::UnsupportedVersion(header[2]));
    }
    let kind = header[3];
    if !matches!(kind, 1 | 2 | 3 | 0x81) {
        return Err(FrameError::UnknownKind(kind));
    }
    let mut request = [0u8; 8];
    request.copy_from_slice(&header[4..12]);
    let mut length = [0u8; 4];
    length.copy_from_slice(&header[12..16]);
    let length = u32::from_le_bytes(length) as usize;
    if HEADER_BYTES + length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(HEADER_BYTES + length));
    }
    let mut payload = vec![0u8; length];
    read_exact_or(reader, &mut payload, false)?;
    decode_payload(kind, u64::from_le_bytes(request), &payload)
}

/// frame 하나를 씁니다.
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> Result<(), FrameError> {
    let bytes = encode(frame)?;
    writer
        .write_all(&bytes)
        .and_then(|()| writer.flush())
        .map_err(|e| FrameError::Io(e.kind()))
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
