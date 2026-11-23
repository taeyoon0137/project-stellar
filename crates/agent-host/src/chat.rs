//! 채팅 보기 모델 `AgentChatItem` (XXIX.2.10, ADR-0005 §4.4, ADR-0092 결정 7).
//!
//! closed enum이며 variant는 2026-11-14 결정(채팅 보기 기본)과 Stellar 승인·출처 모델(§31.3~§31.5)이
//! 요구하는 범위를 넘지 않습니다. 이 타입은 표시용 텍스트와 CLI가 보고한 짧은 이름만 담습니다.
//!
//! - CommandTransaction이나 document·commands·history·storage·ops 타입을 담지 않고, 그 타입이나 승인 응답으로
//!   바꾸는 함수가 없습니다(`tests/inventory.rs`, crate graph 전이 도달 금지).
//! - 문서 효과(변경 노드·속성·승인 결과)를 나타내는 variant가 없습니다. 문서 효과 표시는 앱의 승인·출처
//!   기록(XXIX.3.6)이 만듭니다. tool 호출 표시는 CLI가 보고한 tool 이름과 단계뿐입니다.
//! - item당 표시 텍스트는 [`MAX_ITEM_TEXT_BYTES`], 세션당 item 수는 [`MAX_SESSION_ITEMS`]가 상한입니다.

use std::collections::VecDeque;

use crate::json::{self, JsonValue};

/// item 하나의 표시 텍스트 최대 byte 수(UTF-8 문자 경계에서 자름).
pub const MAX_ITEM_TEXT_BYTES: usize = 16 * 1024;

/// tool 이름·call id·model 이름 같은 짧은 label 최대 byte 수.
pub const MAX_LABEL_BYTES: usize = 256;

/// 세션당 item 최대 수. 넘으면 오래된 item부터 버리고 버린 수를 셉니다.
pub const MAX_SESSION_ITEMS: usize = 10_000;

/// 상한이 적용된 표시 텍스트. 원문 그대로이며(제어 문자 포함) 화면 표시 단계(XXIX.4.3)가 무력화합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayText {
    text: String,
    truncated: bool,
}

impl DisplayText {
    /// [`MAX_ITEM_TEXT_BYTES`] 상한으로 만듭니다.
    pub fn new(text: &str) -> DisplayText {
        DisplayText::bounded(text, MAX_ITEM_TEXT_BYTES)
    }

    /// [`MAX_LABEL_BYTES`] 상한 label.
    pub fn label(text: &str) -> DisplayText {
        DisplayText::bounded(text, MAX_LABEL_BYTES)
    }

    fn bounded(text: &str, max: usize) -> DisplayText {
        if text.len() <= max {
            return DisplayText {
                text: text.to_string(),
                truncated: false,
            };
        }
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        DisplayText {
            text: text[..end].to_string(),
            truncated: true,
        }
    }

    /// 텍스트.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 상한 때문에 잘렸는지.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn golden(&self) -> JsonValue {
        JsonValue::object(vec![
            ("text", JsonValue::string(&self.text)),
            ("truncated", JsonValue::Bool(self.truncated)),
        ])
    }
}

/// tool 호출 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhase {
    /// CLI가 호출을 시작했다고 보고함.
    Started,
    /// CLI가 성공으로 끝났다고 보고함.
    Succeeded,
    /// CLI가 실패로 끝났다고 보고함.
    Failed,
}

impl ToolPhase {
    fn name(self) -> &'static str {
        match self {
            ToolPhase::Started => "started",
            ToolPhase::Succeeded => "succeeded",
            ToolPhase::Failed => "failed",
        }
    }
}

/// CLI가 보고한 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliErrorKind {
    /// 사용 한도(요금제·rate limit). 세션을 멈춥니다(XXIX.2.8).
    UsageLimit,
    /// 인증 오류. 세션을 멈춥니다(XXIX.2.8).
    Authentication,
    /// 그 밖의 오류.
    Other,
}

impl CliErrorKind {
    fn name(self) -> &'static str {
        match self {
            CliErrorKind::UsageLimit => "usage-limit",
            CliErrorKind::Authentication => "authentication",
            CliErrorKind::Other => "other",
        }
    }
}

/// 채팅 보기 item(closed enum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentChatItem {
    /// 사용자가 보낸 문장(그대로 CLI에 전달한 것, ADR-0005 §4.1).
    UserPrompt {
        /// 문장.
        text: DisplayText,
    },
    /// CLI가 보고한 에이전트 응답 텍스트.
    AssistantText {
        /// 텍스트.
        text: DisplayText,
    },
    /// CLI가 보고한 tool 호출 단계(이름만, 입력·결과 내용 없음).
    ToolActivity {
        /// CLI가 붙인 호출 id(없으면 None).
        call_id: Option<DisplayText>,
        /// tool 이름(시작 단계에서만 알 수 있으면 None).
        tool: Option<DisplayText>,
        /// 단계.
        phase: ToolPhase,
    },
    /// CLI가 세션 시작을 보고함. 모델 이름은 CLI가 보고한 값입니다(§4.1).
    SessionStarted {
        /// 모델 이름.
        model: Option<DisplayText>,
    },
    /// CLI가 한 turn의 끝을 보고함.
    TurnCompleted {
        /// CLI가 오류로 끝났다고 보고했는지.
        failed: bool,
    },
    /// CLI가 보고한 오류(원문 텍스트 그대로).
    CliError {
        /// 종류.
        kind: CliErrorKind,
        /// 텍스트.
        text: DisplayText,
    },
    /// 디코드가 실패해 채팅 보기 디코드를 멈춤(XXIX.2.13). 원문 보기는 계속됩니다.
    DecodeStopped {
        /// 실패 판정 코드([`crate::decoder::DecodeFailure::code`]).
        code: &'static str,
    },
}

impl AgentChatItem {
    /// variant 이름(golden·inventory용 고정 낱말).
    pub fn kind(&self) -> &'static str {
        match self {
            AgentChatItem::UserPrompt { .. } => "user-prompt",
            AgentChatItem::AssistantText { .. } => "assistant-text",
            AgentChatItem::ToolActivity { .. } => "tool-activity",
            AgentChatItem::SessionStarted { .. } => "session-started",
            AgentChatItem::TurnCompleted { .. } => "turn-completed",
            AgentChatItem::CliError { .. } => "cli-error",
            AgentChatItem::DecodeStopped { .. } => "decode-stopped",
        }
    }

    /// 결정론적 golden 직렬화(한 줄 JSON). 화면·저장 형식이 아니라 테스트 기준입니다.
    pub fn to_golden(&self) -> String {
        let opt = |v: &Option<DisplayText>| v.as_ref().map_or(JsonValue::Null, DisplayText::golden);
        let body = match self {
            AgentChatItem::UserPrompt { text } | AgentChatItem::AssistantText { text } => {
                vec![("text", text.golden())]
            }
            AgentChatItem::ToolActivity {
                call_id,
                tool,
                phase,
            } => vec![
                ("call_id", opt(call_id)),
                ("tool", opt(tool)),
                ("phase", JsonValue::string(phase.name())),
            ],
            AgentChatItem::SessionStarted { model } => vec![("model", opt(model))],
            AgentChatItem::TurnCompleted { failed } => vec![("failed", JsonValue::Bool(*failed))],
            AgentChatItem::CliError { kind, text } => vec![
                ("error", JsonValue::string(kind.name())),
                ("text", text.golden()),
            ],
            AgentChatItem::DecodeStopped { code } => vec![("code", JsonValue::string(*code))],
        };
        let mut pairs = vec![("kind", JsonValue::string(self.kind()))];
        pairs.extend(body);
        json::write(&JsonValue::object(pairs))
    }
}

/// 세션 하나의 채팅 item 목록(상한 적용, 세션 메모리 전용).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ChatLog {
    items: VecDeque<AgentChatItem>,
    dropped: u64,
    appended: u64,
}

impl std::fmt::Debug for ChatLog {
    /// 내용은 쓰지 않고 개수만 씁니다(진단·로그에 내용 0).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatLog")
            .field("items", &self.items.len())
            .field("dropped", &self.dropped)
            .finish()
    }
}

impl ChatLog {
    /// 빈 목록.
    pub fn new() -> ChatLog {
        ChatLog::default()
    }

    /// item을 더합니다. 상한을 넘으면 가장 오래된 item을 버립니다.
    pub fn push(&mut self, item: AgentChatItem) {
        if self.items.len() == MAX_SESSION_ITEMS {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(item);
        self.appended += 1;
    }

    /// 현재 item.
    pub fn items(&self) -> impl Iterator<Item = &AgentChatItem> {
        self.items.iter()
    }

    /// 현재 item 수.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 상한 때문에 버린 item 수.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 지금까지 더한 item 수(증분 조회 기준).
    pub fn appended(&self) -> u64 {
        self.appended
    }

    /// `since`(이전 [`ChatLog::appended`]) 뒤에 더해져 아직 남아 있는 item.
    pub fn since(&self, since: u64) -> Vec<AgentChatItem> {
        let first_kept = self.appended - self.items.len() as u64;
        let skip = since.saturating_sub(first_kept) as usize;
        self.items.iter().skip(skip).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_capped_on_char_boundary() {
        let long = "가".repeat(MAX_ITEM_TEXT_BYTES); // 3 byte 문자
        let t = DisplayText::new(&long);
        assert!(t.truncated());
        assert!(t.as_str().len() <= MAX_ITEM_TEXT_BYTES);
        assert!(t.as_str().chars().all(|c| c == '가'));
        assert!(!DisplayText::new("짧음").truncated());
    }

    #[test]
    fn log_is_capped_and_incremental() {
        let mut log = ChatLog::new();
        for _ in 0..MAX_SESSION_ITEMS + 5 {
            log.push(AgentChatItem::TurnCompleted { failed: false });
        }
        assert_eq!(log.len(), MAX_SESSION_ITEMS);
        assert_eq!(log.dropped(), 5);
        let mark = log.appended();
        log.push(AgentChatItem::DecodeStopped { code: "x" });
        assert_eq!(log.since(mark).len(), 1);
        assert_eq!(log.since(0).len(), MAX_SESSION_ITEMS);
        assert!(!format!("{log:?}").contains("TurnCompleted"));
    }
}
