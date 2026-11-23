//! 수신 출력 처리 (XXIX.2.4·XXIX.2.12·XXIX.2.13, ADR-0005 §4.4·§6, ADR-0092 결정 9·10).
//!
//! [`SessionOutput`]은 실행기의 [`OutputSink`]입니다. reader 스레드(Agent Host I/O)에서 다음을 합니다.
//!
//! 1. 받은 byte를 먼저 원문 버퍼([`RawBuffer`])에 그대로 넣습니다. 디코더 결과와 관계없이 항상 보존합니다.
//! 2. stdout만 디코더에 넣고 새 [`AgentChatItem`]을 채팅 목록에 더합니다.
//! 3. 디코드가 실패하면(XXIX.2.13) 채팅 목록에 [`AgentChatItem::DecodeStopped`] 하나를 더하고 디코더는 닫힌 채
//!    둡니다. 이후 stdout은 원문 버퍼에만 들어갑니다. 세션 처리에는 [`OutputSignal::DecodeFailed`]를
//!    넘기며, 이 신호는 세션 상태·권한·lease를 바꾸지 않습니다([`crate::session`]).
//! 4. CLI가 보고한 사용 한도·인증 오류는 [`OutputSignal::Cli`]로 넘겨 세션을 멈추게 합니다(XXIX.2.8).
//!
//! lock은 입력 한 조각을 처리하는 동안만 잡습니다. UI는 [`SessionOutput::chat_since`]·
//! [`SessionOutput::take_signals`]로 증분만 가져가므로, UI가 느려도 수신은 멈추지 않습니다.
//! 내용은 세션 메모리에만 있고 `Debug`는 개수만 씁니다.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::chat::{AgentChatItem, ChatLog, DisplayText};
use crate::decoder::{CliSignal, DecodeFailure, Decoder, DecoderKind};
use crate::process::OutputSink;
use crate::raw::{RawBuffer, RawStream};

/// 세션 처리에 넘길 출력 신호.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSignal {
    /// CLI가 보고한 사용 한도·인증 오류.
    Cli(CliSignal),
    /// 디코드 실패 판정.
    DecodeFailed(DecodeFailure),
}

#[derive(Debug)]
struct State {
    raw: RawBuffer,
    decoder: Decoder,
    chat: ChatLog,
    signals: VecDeque<OutputSignal>,
    closed: [bool; 2],
}

/// 세션 하나의 수신 출력.
#[derive(Debug)]
pub struct SessionOutput {
    state: Mutex<State>,
}

/// 원문 버퍼 통계(내용 없음).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawStats {
    /// 담은 byte.
    pub len: usize,
    /// 받은 byte.
    pub received: u64,
    /// 버린 byte.
    pub dropped: u64,
    /// 상한.
    pub cap: usize,
}

impl SessionOutput {
    /// 새 출력 상태.
    pub fn new(kind: DecoderKind, raw_cap: usize) -> SessionOutput {
        SessionOutput {
            state: Mutex::new(State {
                raw: RawBuffer::with_cap(raw_cap),
                decoder: Decoder::new(kind),
                chat: ChatLog::new(),
                signals: VecDeque::new(),
                closed: [false; 2],
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 사용자가 보낸 문장을 채팅 목록에 더합니다(CLI 전달과 같은 동작에서 호출).
    pub fn push_user_prompt(&self, text: &str) {
        self.lock().chat.push(AgentChatItem::UserPrompt {
            text: DisplayText::new(text),
        });
    }

    /// `mark`(이전 반환값) 뒤의 채팅 item과 새 mark.
    pub fn chat_since(&self, mark: u64) -> (Vec<AgentChatItem>, u64) {
        let state = self.lock();
        (state.chat.since(mark), state.chat.appended())
    }

    /// 지금 채팅 item 전체.
    pub fn chat_items(&self) -> Vec<AgentChatItem> {
        self.lock().chat.items().cloned().collect()
    }

    /// 쌓인 신호를 가져갑니다.
    pub fn take_signals(&self) -> Vec<OutputSignal> {
        self.lock().signals.drain(..).collect()
    }

    /// 디코드 실패 판정.
    pub fn decode_failure(&self) -> Option<DecodeFailure> {
        self.lock().decoder.failure()
    }

    /// 원문 보기 표시 텍스트.
    pub fn raw_display_text(&self) -> String {
        self.lock().raw.display_text()
    }

    /// 원문 byte(받은 순서 그대로, 상한 안).
    pub fn raw_bytes(&self) -> Vec<u8> {
        self.lock()
            .raw
            .segments()
            .flat_map(|s| s.bytes.iter().copied())
            .collect()
    }

    /// 원문 버퍼 통계.
    pub fn raw_stats(&self) -> RawStats {
        let state = self.lock();
        RawStats {
            len: state.raw.len(),
            received: state.raw.received(),
            dropped: state.raw.dropped(),
            cap: state.raw.cap(),
        }
    }

    /// stdout·stderr가 모두 끝났는지.
    pub fn streams_closed(&self) -> bool {
        self.lock().closed == [true, true]
    }
}

impl State {
    fn apply(&mut self, out: crate::decoder::DecodeOutput) {
        for item in out.items {
            self.chat.push(item);
        }
        for signal in out.signals {
            self.signals.push_back(OutputSignal::Cli(signal));
        }
        if let Some(failure) = out.failure {
            self.chat
                .push(AgentChatItem::DecodeStopped { code: failure.code });
            self.signals.push_back(OutputSignal::DecodeFailed(failure));
        }
    }
}

impl OutputSink for SessionOutput {
    fn on_output(&self, stream: RawStream, bytes: &[u8]) {
        let mut state = self.lock();
        state.raw.push(stream, bytes);
        if stream == RawStream::Stdout {
            let out = state.decoder.feed(bytes);
            state.apply(out);
        }
    }

    fn on_closed(&self, stream: RawStream) {
        let mut state = self.lock();
        match stream {
            RawStream::Stdout => {
                state.closed[0] = true;
                let out = state.decoder.finish();
                state.apply(out);
            }
            RawStream::Stderr => state.closed[1] = true,
        }
    }
}
