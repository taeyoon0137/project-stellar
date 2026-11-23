//! 표시 전용 출력 디코더 (XXIX.2.11, ADR-0005 §4.4, ADR-0092 결정 8).
//!
//! 공급자 출력 해석 코드는 이 모듈에만 있습니다(다른 crate·셸·FFI 0, `tests/inventory.rs`).
//!
//! ## 구성과 입출력
//!
//! - [`DecoderKind`]: launch profile이 고른 decoder profile(id + 버전). 버전이 다르면 resolve하지 않습니다.
//! - [`Decoder`]: stdout byte를 순서대로 받아([`Decoder::feed`]) 줄(`\n`) 단위로 자르고, 줄마다 JSON object
//!   하나를 profile 규칙으로 [`AgentChatItem`]과 [`CliSignal`]로 바꿉니다. 입력 경계에 걸친 줄과 UTF-8
//!   문자는 다음 입력과 이어 붙입니다. 끝([`Decoder::finish`])에서 남은 조각을 마지막 줄로 처리합니다.
//! - stderr는 디코드하지 않습니다(원문 버퍼에만 보존, [`crate::output`]).
//!
//! ## 실패 판정(fail closed)
//!
//! 다음 중 하나면 [`DecodeFailure`] 하나를 내고 디코더를 닫습니다. 닫힌 뒤의 입력은 item을 만들지 않습니다.
//!
//! | code | 조건 |
//! |---|---|
//! | `invalid-utf8` | 줄이 UTF-8이 아님 |
//! | `line-too-long` | 한 줄이 [`MAX_LINE_BYTES`]를 넘음 |
//! | `invalid-json` | JSON 문법 오류·자원 상한 초과 |
//! | `not-object` | 최상위 값이 object가 아님 |
//! | `missing-type` | 문자열 `type` field가 없음 |
//! | `shape` | 표시하는 문서화된 type인데 공식 문서의 필수 field 모양과 다름 |
//!
//! 공식 문서에 있지만 채팅 보기에 보이지 않는 type과 이 decoder profile이 모르는 type은 item을 만들지
//! 않고 건너뜁니다(잘못된 내용을 보이지 않으므로 닫힘 판정과 충돌하지 않음). 건너뛴 줄 수는 세어 둡니다.
//!
//! 같은 원문은 입력을 어떻게 나눠 넣어도 같은 item 목록으로 디코드됩니다(결정론, 시각·난수 입력 없음).
//! 디코더는 권한·문서 상태를 다루지 않습니다(XXIX.2.13은 [`crate::session`]).

mod claude;
mod codex;

use crate::chat::AgentChatItem;
use crate::json::{self, JsonLimits, JsonValue};

/// decoder profile 참조.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderRef {
    /// decoder id([`DecoderKind`] 이름).
    pub id: String,
    /// decoder profile 버전.
    pub version: u32,
}

/// 한 줄 최대 byte 수.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// decoder profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderKind {
    /// Claude Code `-p --output-format stream-json --verbose` 출력(SDKMessage JSON Lines), 버전 1.
    ClaudeStreamJsonV1,
    /// Codex CLI `exec --json` 출력(JSONL event), 버전 1.
    CodexExecJsonlV1,
}

impl DecoderKind {
    /// 모든 decoder profile.
    pub const ALL: [DecoderKind; 2] = [
        DecoderKind::ClaudeStreamJsonV1,
        DecoderKind::CodexExecJsonlV1,
    ];

    /// id.
    pub fn id(self) -> &'static str {
        match self {
            DecoderKind::ClaudeStreamJsonV1 => "claude-stream-json",
            DecoderKind::CodexExecJsonlV1 => "codex-exec-jsonl",
        }
    }

    /// 버전.
    pub fn version(self) -> u32 {
        1
    }

    /// profile 참조를 찾습니다. id와 버전이 모두 같아야 합니다.
    pub fn resolve(reference: &DecoderRef) -> Option<DecoderKind> {
        DecoderKind::ALL
            .into_iter()
            .find(|k| k.id() == reference.id && k.version() == reference.version)
    }
}

/// CLI가 보고한, 세션 처리에 넘길 신호(XXIX.2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliSignal {
    /// 사용 한도 오류.
    UsageLimit,
    /// 인증 오류.
    Authentication,
}

/// 디코드 실패 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeFailure {
    /// 실패 코드(모듈 문서 표).
    pub code: &'static str,
    /// 실패한 줄 번호(1부터).
    pub line: u64,
}

/// [`Decoder::feed`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodeOutput {
    /// 새 item.
    pub items: Vec<AgentChatItem>,
    /// 새 신호.
    pub signals: Vec<CliSignal>,
    /// 이번 입력에서 실패했으면 그 판정(디코더는 닫힘).
    pub failure: Option<DecodeFailure>,
}

/// 줄 단위 decoder profile 구현이 돌려주는 결과.
pub(crate) type LineResult = Result<(), &'static str>;

/// 출력 디코더 하나(세션당 하나). `Debug`는 내용 없이 상태만 씁니다.
pub struct Decoder {
    kind: DecoderKind,
    pending: Vec<u8>,
    line: u64,
    skipped: u64,
    failure: Option<DecodeFailure>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("kind", &self.kind)
            .field("line", &self.line)
            .field("pending_bytes", &self.pending.len())
            .field("skipped", &self.skipped)
            .field("failure", &self.failure)
            .finish()
    }
}

impl Decoder {
    /// 새 디코더.
    pub fn new(kind: DecoderKind) -> Decoder {
        Decoder {
            kind,
            pending: Vec::new(),
            line: 0,
            skipped: 0,
            failure: None,
        }
    }

    /// decoder profile.
    pub fn kind(&self) -> DecoderKind {
        self.kind
    }

    /// 실패해 닫혔으면 그 판정.
    pub fn failure(&self) -> Option<DecodeFailure> {
        self.failure
    }

    /// 표시하지 않고 건너뛴 줄 수.
    pub fn skipped_lines(&self) -> u64 {
        self.skipped
    }

    /// stdout byte를 받습니다.
    pub fn feed(&mut self, bytes: &[u8]) -> DecodeOutput {
        let mut out = DecodeOutput::default();
        if self.failure.is_some() {
            return out;
        }
        let mut rest = bytes;
        while let Some(pos) = rest.iter().position(|&b| b == b'\n') {
            let (head, tail) = rest.split_at(pos);
            rest = &tail[1..];
            if self.pending.len() + head.len() > MAX_LINE_BYTES {
                self.line += 1;
                self.fail("line-too-long", &mut out);
                return out;
            }
            let mut line = std::mem::take(&mut self.pending);
            line.extend_from_slice(head);
            self.decode_line(&line, &mut out);
            if self.failure.is_some() {
                return out;
            }
        }
        if self.pending.len() + rest.len() > MAX_LINE_BYTES {
            self.line += 1;
            self.fail("line-too-long", &mut out);
            return out;
        }
        self.pending.extend_from_slice(rest);
        out
    }

    /// 출력 끝. 줄바꿈 없이 남은 조각을 마지막 줄로 처리합니다.
    pub fn finish(&mut self) -> DecodeOutput {
        let mut out = DecodeOutput::default();
        if self.failure.is_some() || self.pending.is_empty() {
            return out;
        }
        let line = std::mem::take(&mut self.pending);
        self.decode_line(&line, &mut out);
        out
    }

    fn fail(&mut self, code: &'static str, out: &mut DecodeOutput) {
        let failure = DecodeFailure {
            code,
            line: self.line,
        };
        self.failure = Some(failure);
        self.pending.clear();
        out.failure = Some(failure);
    }

    fn decode_line(&mut self, raw: &[u8], out: &mut DecodeOutput) {
        self.line += 1;
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Ok(text) = std::str::from_utf8(raw) else {
            return self.fail("invalid-utf8", out);
        };
        if text.trim().is_empty() {
            return;
        }
        let limits = JsonLimits {
            max_bytes: MAX_LINE_BYTES,
            ..JsonLimits::default()
        };
        let value = match json::parse(text, limits) {
            Ok(value) => value,
            Err(_) => return self.fail("invalid-json", out),
        };
        if value.as_object().is_none() {
            return self.fail("not-object", out);
        }
        let Some(kind) = value.get("type").and_then(JsonValue::as_str) else {
            return self.fail("missing-type", out);
        };
        let (items_before, signals_before) = (out.items.len(), out.signals.len());
        let result = match self.kind {
            DecoderKind::ClaudeStreamJsonV1 => claude::decode(kind, &value, out),
            DecoderKind::CodexExecJsonlV1 => codex::decode(kind, &value, out),
        };
        match result {
            Ok(()) => {
                if out.items.len() == items_before && out.signals.len() == signals_before {
                    self.skipped += 1;
                }
            }
            Err(code) => {
                // 실패한 줄이 만든 item·신호는 버립니다(부분 해석 결과를 보이지 않음).
                out.items.truncate(items_before);
                out.signals.truncate(signals_before);
                self.fail(code, out)
            }
        }
    }
}

/// 원문 전체를 한 번에 디코드합니다(golden 테스트·결정론 확인용).
pub fn decode_all(
    kind: DecoderKind,
    raw: &[u8],
) -> (Vec<AgentChatItem>, Vec<CliSignal>, Option<DecodeFailure>) {
    let mut decoder = Decoder::new(kind);
    let mut a = decoder.feed(raw);
    let b = decoder.finish();
    a.items.extend(b.items);
    a.signals.extend(b.signals);
    (a.items, a.signals, a.failure.or(b.failure))
}

/// object field 도움 함수: 문자열.
pub(crate) fn str_field<'a>(value: &'a JsonValue, key: &str) -> Option<&'a str> {
    value.get(key).and_then(JsonValue::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_requires_exact_version() {
        let ok = DecoderRef {
            id: "claude-stream-json".into(),
            version: 1,
        };
        assert_eq!(
            DecoderKind::resolve(&ok),
            Some(DecoderKind::ClaudeStreamJsonV1)
        );
        let newer = DecoderRef {
            version: 2,
            ..ok.clone()
        };
        assert_eq!(DecoderKind::resolve(&newer), None);
    }

    #[test]
    fn closed_decoder_ignores_later_input() {
        let mut d = Decoder::new(DecoderKind::CodexExecJsonlV1);
        let out = d.feed(b"not json\n{\"type\":\"turn.completed\"}\n");
        assert_eq!(
            out.failure.map(|f| (f.code, f.line)),
            Some(("invalid-json", 1))
        );
        assert!(out.items.is_empty());
        assert!(d.feed(b"{\"type\":\"turn.completed\"}\n").items.is_empty());
        assert!(d.finish().items.is_empty());
    }

    #[test]
    fn line_limit_fails_closed_even_without_newline() {
        let mut d = Decoder::new(DecoderKind::CodexExecJsonlV1);
        let big = vec![b'x'; MAX_LINE_BYTES + 1];
        assert_eq!(d.feed(&big).failure.map(|f| f.code), Some("line-too-long"));
    }
}
