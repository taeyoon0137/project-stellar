//! clipboard 모델: 복사 문자열, 붙여넣기 입력 변환, 프로그램의 clipboard 요청 정책 (XVIII.4.2).
//!
//! 실제 시스템 clipboard(`NSPasteboard`) 읽기·쓰기는 앱 셸(XVIII.4.3)이 합니다. 이 모듈은 그
//! 경계의 값만 정합니다.
//!
//! - **복사**: 선택 영역 문자열([`CopiedText`]). [`MAX_COPY_BYTES`]에서 자르고 잘렸는지 알립니다.
//! - **붙여넣기**([`encode_paste`]): 개행(`\r\n`·`\n`)을 터미널 Enter인 `\r`로 바꾸고, tab을 뺀 C0
//!   제어 문자·DEL·C1 제어 문자를 지웁니다. 붙여넣은 글이 escape sequence로 터미널이나 프로그램을
//!   조작하지 못하게 하기 위해서입니다(bracketed paste 끝 표시 `ESC [201~` 주입 포함). bracketed
//!   paste mode(?2004)면 `ESC [200~`·`ESC [201~`로 감쌉니다. bracketed가 아닌 여러 줄 붙여넣기는
//!   줄마다 명령이 실행될 수 있으므로 UI가 [`PasteData::line_count`]로 확인을 받을 수 있습니다.
//! - **프로그램 요청**(OSC 52): 기본은 거부([`ClipboardAccess::Deny`])입니다. 쓰기 허용
//!   ([`ClipboardAccess::AllowWrite`])이어도 emulator는
//!   [`TerminalEvent::ClipboardWrite`](super::TerminalEvent::ClipboardWrite)로 알리기만 하고, 실제로
//!   쓸지는 UI가 정합니다.
//!   clipboard **읽기** 요청(`?`)은 정책과 관계없이 항상 거부합니다(clipboard 내용을 child에 보내지
//!   않음).

use std::fmt;

/// 복사 문자열 상한(byte).
pub const MAX_COPY_BYTES: usize = 32 * 1024 * 1024;
/// 붙여넣기 입력 상한(byte).
pub const MAX_PASTE_BYTES: usize = 32 * 1024 * 1024;

const BRACKET_START: &[u8] = b"\x1b[200~";
const BRACKET_END: &[u8] = b"\x1b[201~";

/// 프로그램(OSC 52)의 clipboard 요청 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardAccess {
    /// 모두 거부(기본).
    #[default]
    Deny,
    /// 쓰기 요청을 UI에 알림. 읽기는 항상 거부.
    AllowWrite,
}

/// 선택 영역을 복사한 문자열.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedText {
    /// 문자열.
    pub text: String,
    /// [`MAX_COPY_BYTES`]에서 잘렸는지.
    pub truncated: bool,
}

/// 붙여넣기 입력 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteError {
    /// [`MAX_PASTE_BYTES`]를 넘습니다.
    TooLarge {
        /// 입력 크기(byte).
        len: usize,
    },
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasteError::TooLarge { len } => write!(
                f,
                "붙여넣을 글이 너무 큽니다({len} byte, 상한 {MAX_PASTE_BYTES})"
            ),
        }
    }
}

impl std::error::Error for PasteError {}

/// PTY에 쓸 붙여넣기 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteData {
    bytes: Vec<u8>,
    line_count: usize,
    removed_controls: usize,
    bracketed: bool,
}

impl PasteData {
    /// PTY 입력 bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 줄 수(`\r` 개수 + 1). 빈 입력은 0.
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// 지운 제어 문자 수.
    pub fn removed_controls(&self) -> usize {
        self.removed_controls
    }

    /// bracketed paste로 감쌌는지.
    pub fn is_bracketed(&self) -> bool {
        self.bracketed
    }
}

/// 붙여넣을 글을 PTY 입력으로 바꿉니다(모듈 문서).
pub fn encode_paste(text: &str, bracketed: bool) -> Result<PasteData, PasteError> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(PasteError::TooLarge { len: text.len() });
    }
    let mut body = String::with_capacity(text.len());
    let mut removed_controls = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                body.push('\r');
            }
            '\n' => body.push('\r'),
            '\t' => body.push('\t'),
            c if c.is_control() => removed_controls += 1,
            c => body.push(c),
        }
    }
    let line_count = if body.is_empty() {
        0
    } else {
        body.matches('\r').count() + 1
    };
    let mut bytes = Vec::with_capacity(body.len() + BRACKET_START.len() + BRACKET_END.len());
    if bracketed {
        bytes.extend_from_slice(BRACKET_START);
    }
    bytes.extend_from_slice(body.as_bytes());
    if bracketed {
        bytes.extend_from_slice(BRACKET_END);
    }
    Ok(PasteData {
        bytes,
        line_count,
        removed_controls,
        bracketed,
    })
}

/// 표준 base64(패딩 선택) 해독. 형식이 틀리면 `None`.
pub(crate) fn decode_base64(input: &[u8]) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let trimmed = match input {
        [rest @ .., b'=', b'='] => rest,
        [rest @ .., b'='] => rest,
        _ => input,
    };
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3 + 2);
    for chunk in trimmed.chunks(4) {
        let mut acc = 0u32;
        for &byte in chunk {
            acc = (acc << 6) | value(byte)?;
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
            2 => out.push((acc >> 4) as u8),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_normalizes_newlines_and_strips_controls() {
        let paste = encode_paste("echo a\r\necho b\nlast\tcol", false).unwrap();
        assert_eq!(paste.bytes(), b"echo a\recho b\rlast\tcol");
        assert_eq!(paste.line_count(), 3);
        assert!(!paste.is_bracketed());
        // 끝 표시 주입·escape sequence·BEL·DEL·C1은 지웁니다.
        let hostile = "ok\x1b[201~rm -rf ~\x07\x7f\u{9b}2J";
        let paste = encode_paste(hostile, true).unwrap();
        assert_eq!(paste.bytes(), b"\x1b[200~ok[201~rm -rf ~2J\x1b[201~");
        assert_eq!(paste.removed_controls(), 4);
        assert_eq!(paste.line_count(), 1);
        assert!(paste.is_bracketed());
        // 한글·emoji는 그대로, 빈 입력은 0줄.
        assert_eq!(
            encode_paste("한글😀", false).unwrap().bytes(),
            "한글😀".as_bytes()
        );
        assert_eq!(encode_paste("", true).unwrap().line_count(), 0);
        assert_eq!(
            encode_paste(&"x".repeat(MAX_PASTE_BYTES + 1), false),
            Err(PasteError::TooLarge {
                len: MAX_PASTE_BYTES + 1
            })
        );
    }

    #[test]
    fn base64_decoding() {
        assert_eq!(decode_base64(b"aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64(b"aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64(b"7ZWc6riA").unwrap(), "한글".as_bytes());
        assert_eq!(decode_base64(b"").unwrap(), b"");
        assert_eq!(decode_base64(b"YQ==").unwrap(), b"a");
        assert_eq!(decode_base64(b"a"), None);
        assert_eq!(decode_base64(b"a$bc"), None);
    }
}
