//! 원문 보기 버퍼 (XXIX.2.12, ADR-0005 §4.4, ADR-0092 결정 9).
//!
//! CLI stdout·stderr를 받은 순서 그대로 담는 세션별 메모리 버퍼입니다.
//!
//! - 상한([`DEFAULT_RAW_CAP_BYTES`], 실측 근거는 XXIX.2.12 보고서)을 넘으면 오래된 byte부터 버리고 버린
//!   byte 수와 잘림 사실([`RawBuffer::truncated`])을 남깁니다. 메모리는 상한 + 조각 bookkeeping으로 제한됩니다.
//! - 제어 문자를 해석하지 않습니다. PTY·terminal emulation을 쓰지 않습니다. 저장은 받은 byte 그대로이고,
//!   읽기 전용 표시 텍스트([`RawBuffer::display_text`])는 C0·C1 제어 문자와 DEL을 보이는 기호로 바꿔
//!   escape sequence가 실행되지 않게 합니다(줄바꿈·탭만 유지).
//! - 디코더 성공·실패와 관계없이 보존합니다([`crate::output`]).
//! - 세션 메모리에만 둡니다. 직렬화 수단이 없고 `Debug`는 크기만 씁니다(진단·recovery snapshot에 내용 0).

use std::collections::VecDeque;
use std::fmt;

/// 세션당 기본 상한(8 MiB).
pub const DEFAULT_RAW_CAP_BYTES: usize = 8 * 1024 * 1024;

/// 출력 stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStream {
    /// 표준 출력.
    Stdout,
    /// 표준 오류.
    Stderr,
}

/// 받은 조각 하나.
#[derive(Clone, PartialEq, Eq)]
pub struct RawSegment {
    /// stream.
    pub stream: RawStream,
    /// 받은 byte(앞부분이 상한으로 잘렸을 수 있음).
    pub bytes: Vec<u8>,
}

/// 세션별 원문 버퍼.
#[derive(Clone)]
pub struct RawBuffer {
    cap: usize,
    len: usize,
    segments: VecDeque<RawSegment>,
    received: u64,
    dropped: u64,
}

impl fmt::Debug for RawBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawBuffer")
            .field("cap", &self.cap)
            .field("len", &self.len)
            .field("received", &self.received)
            .field("dropped", &self.dropped)
            .finish()
    }
}

impl Default for RawBuffer {
    fn default() -> Self {
        RawBuffer::with_cap(DEFAULT_RAW_CAP_BYTES)
    }
}

/// 조각을 합칠 최대 크기. 작은 write가 많아도 조각 수가 늘지 않게 같은 stream의 연속 조각을 합칩니다.
const MERGE_LIMIT: usize = 64 * 1024;

impl RawBuffer {
    /// 상한을 정해 만듭니다(0이면 1 byte).
    pub fn with_cap(cap: usize) -> RawBuffer {
        RawBuffer {
            cap: cap.max(1),
            len: 0,
            segments: VecDeque::new(),
            received: 0,
            dropped: 0,
        }
    }

    /// 받은 byte를 그대로 더합니다.
    pub fn push(&mut self, stream: RawStream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.received += bytes.len() as u64;
        // 상한보다 큰 입력은 뒷부분만 남깁니다.
        let bytes = if bytes.len() > self.cap {
            let skip = bytes.len() - self.cap;
            self.dropped += skip as u64;
            &bytes[skip..]
        } else {
            bytes
        };
        match self.segments.back_mut() {
            Some(last)
                if last.stream == stream && last.bytes.len() + bytes.len() <= MERGE_LIMIT =>
            {
                last.bytes.extend_from_slice(bytes)
            }
            _ => self.segments.push_back(RawSegment {
                stream,
                bytes: bytes.to_vec(),
            }),
        }
        self.len += bytes.len();
        while self.len > self.cap {
            let excess = self.len - self.cap;
            let front = self.segments.front_mut().expect("len > 0이면 조각이 있음");
            if front.bytes.len() <= excess {
                let removed = self.segments.pop_front().expect("front 있음");
                self.len -= removed.bytes.len();
                self.dropped += removed.bytes.len() as u64;
            } else {
                front.bytes.drain(..excess);
                self.len -= excess;
                self.dropped += excess as u64;
            }
        }
    }

    /// 상한.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// 지금 담은 byte 수(≤ 상한).
    pub fn len(&self) -> usize {
        self.len
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 지금까지 받은 byte 수.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// 상한 때문에 버린 byte 수.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 앞부분이 잘렸는지(원문 보기의 잘림 표시).
    pub fn truncated(&self) -> bool {
        self.dropped > 0
    }

    /// 담은 조각(받은 순서).
    pub fn segments(&self) -> impl Iterator<Item = &RawSegment> {
        self.segments.iter()
    }

    /// 읽기 전용 표시 텍스트. 잘렸으면 첫 줄에 잘림 표시를 넣고, 제어 문자를 무력화합니다.
    pub fn display_text(&self) -> String {
        let mut bytes = Vec::with_capacity(self.len);
        for segment in &self.segments {
            bytes.extend_from_slice(&segment.bytes);
        }
        let mut out = String::with_capacity(bytes.len() + 64);
        if self.truncated() {
            out.push_str(&format!("[앞부분 {} byte 잘림]\n", self.dropped));
        }
        out.push_str(&neutralize_controls(&String::from_utf8_lossy(&bytes)));
        out
    }
}

/// C0(줄바꿈·탭 제외)·DEL·C1 제어 문자를 Unicode Control Pictures(U+2400~)나 U+FFFD로 바꿉니다.
pub fn neutralize_controls(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
            '\u{7f}' => '\u{2421}',
            '\u{80}'..='\u{9f}' => '\u{fffd}',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_tail_within_cap_and_marks_truncation() {
        let mut b = RawBuffer::with_cap(10);
        b.push(RawStream::Stdout, b"0123456789");
        assert!(!b.truncated());
        b.push(RawStream::Stderr, b"abc");
        assert_eq!(b.len(), 10);
        assert_eq!(b.dropped(), 3);
        assert!(b.display_text().ends_with("3456789abc"));
        assert!(b.display_text().starts_with("[앞부분 3 byte 잘림]"));
        b.push(RawStream::Stdout, &[b'z'; 25]);
        assert_eq!(b.len(), 10);
        assert_eq!(b.received(), 38);
        assert_eq!(b.dropped(), 28);
    }

    #[test]
    fn controls_are_neutralized_but_stored_raw() {
        let mut b = RawBuffer::with_cap(1024);
        let hostile = b"\x1b]0;title\x07\x1b[2J\rok\n\ttab\x7f\xc2\x9b";
        b.push(RawStream::Stdout, hostile);
        let stored: Vec<u8> = b.segments().flat_map(|s| s.bytes.clone()).collect();
        assert_eq!(stored, hostile);
        let shown = b.display_text();
        assert!(!shown
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'));
        assert!(shown.contains('\u{241b}'));
        assert!(shown.contains("ok\n\ttab"));
        assert!(!format!("{b:?}").contains("title"));
    }
}
