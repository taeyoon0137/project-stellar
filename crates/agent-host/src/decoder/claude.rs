//! decoder profile `claude-stream-json@1`.
//!
//! 근거: Claude Code CLI reference(`-p`, `--output-format stream-json`, `--verbose`)와 Agent SDK TypeScript
//! reference의 `SDKMessage` 정의(2026-11-23 확인). 한 줄이 `SDKMessage` 하나입니다.
//!
//! | `type` | 표시 |
//! |---|---|
//! | `system`, `subtype` `init` | `SessionStarted { model }`(`model` 문자열 필수) |
//! | `assistant` | `message.content`(배열 필수)의 `text` 블록 → `AssistantText`, `tool_use` 블록 → `ToolActivity` Started(`name` 필수). `error`(`SDKAssistantMessageError`)가 있으면 `CliError` |
//! | `user` | `message.content` 배열의 `tool_result` 블록 → `ToolActivity` Succeeded/Failed(`is_error`). 문자열 content는 표시하지 않음 |
//! | `result` | `is_error`(bool 필수) → `TurnCompleted`. 오류 subtype의 `errors` 문자열 → `CliError` Other |
//! | 그 밖 | 표시하지 않음 |
//!
//! 사용량·비용 field(`usage`, `total_cost_usd`, `modelUsage`)는 읽지 않습니다(ADR-0005 §4.1).

use super::{str_field, CliSignal, DecodeOutput, LineResult};
use crate::chat::{AgentChatItem, CliErrorKind, DisplayText, ToolPhase};
use crate::json::JsonValue;

/// `SDKAssistantMessageError` 값 분류.
fn error_kind(code: &str) -> (CliErrorKind, Option<CliSignal>) {
    match code {
        "rate_limit" | "billing_error" => (CliErrorKind::UsageLimit, Some(CliSignal::UsageLimit)),
        "authentication_failed"
        | "oauth_org_not_allowed"
        | "account_on_hold"
        | "cloud_credential_error" => (
            CliErrorKind::Authentication,
            Some(CliSignal::Authentication),
        ),
        _ => (CliErrorKind::Other, None),
    }
}

fn content(value: &JsonValue) -> Result<&JsonValue, &'static str> {
    let message = value.get("message").ok_or("shape")?;
    if message.as_object().is_none() {
        return Err("shape");
    }
    message.get("content").ok_or("shape")
}

pub(super) fn decode(kind: &str, value: &JsonValue, out: &mut DecodeOutput) -> LineResult {
    match kind {
        "system" => {
            if str_field(value, "subtype") == Some("init") {
                let model = str_field(value, "model").ok_or("shape")?;
                out.items.push(AgentChatItem::SessionStarted {
                    model: Some(DisplayText::label(model)),
                });
            }
            Ok(())
        }
        "assistant" => {
            let blocks = content(value)?.as_array().ok_or("shape")?;
            for block in blocks {
                match str_field(block, "type") {
                    Some("text") => {
                        let text = str_field(block, "text").ok_or("shape")?;
                        out.items.push(AgentChatItem::AssistantText {
                            text: DisplayText::new(text),
                        });
                    }
                    Some("tool_use") => {
                        let name = str_field(block, "name").ok_or("shape")?;
                        out.items.push(AgentChatItem::ToolActivity {
                            call_id: str_field(block, "id").map(DisplayText::label),
                            tool: Some(DisplayText::label(name)),
                            phase: ToolPhase::Started,
                        });
                    }
                    Some(_) => {}
                    None => return Err("shape"),
                }
            }
            match value.get("error") {
                None | Some(JsonValue::Null) => {}
                Some(JsonValue::String(code)) => {
                    let (kind, signal) = error_kind(code);
                    out.items.push(AgentChatItem::CliError {
                        kind,
                        text: DisplayText::new(code),
                    });
                    out.signals.extend(signal);
                }
                Some(_) => return Err("shape"),
            }
            Ok(())
        }
        "user" => {
            match content(value)? {
                JsonValue::String(_) => {}
                JsonValue::Array(blocks) => {
                    for block in blocks {
                        if str_field(block, "type") == Some("tool_result") {
                            let failed = match block.get("is_error") {
                                None | Some(JsonValue::Null) => false,
                                Some(JsonValue::Bool(b)) => *b,
                                Some(_) => return Err("shape"),
                            };
                            out.items.push(AgentChatItem::ToolActivity {
                                call_id: str_field(block, "tool_use_id").map(DisplayText::label),
                                tool: None,
                                phase: if failed {
                                    ToolPhase::Failed
                                } else {
                                    ToolPhase::Succeeded
                                },
                            });
                        }
                    }
                }
                _ => return Err("shape"),
            }
            Ok(())
        }
        "result" => {
            let failed = value
                .get("is_error")
                .and_then(JsonValue::as_bool)
                .ok_or("shape")?;
            str_field(value, "subtype").ok_or("shape")?;
            if let Some(errors) = value.get("errors") {
                for error in errors.as_array().ok_or("shape")? {
                    let text = error.as_str().ok_or("shape")?;
                    out.items.push(AgentChatItem::CliError {
                        kind: CliErrorKind::Other,
                        text: DisplayText::new(text),
                    });
                }
            }
            out.items.push(AgentChatItem::TurnCompleted { failed });
            Ok(())
        }
        _ => Ok(()),
    }
}
