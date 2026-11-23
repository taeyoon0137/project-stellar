//! decoder profile `codex-exec-jsonl@1`.
//!
//! 근거: Codex CLI non-interactive mode 문서(`codex exec --json`: stdout이 JSON Lines event stream,
//! event type `thread.started`·`turn.started`·`turn.completed`·`turn.failed`·`item.*`·`error`, item type
//! `agent_message`·`command_execution` 등, 2026-11-23 확인).
//!
//! | `type` | 표시 |
//! |---|---|
//! | `thread.started` | `SessionStarted { model: None }` |
//! | `turn.completed` | `TurnCompleted { failed: false }`(`usage`는 읽지 않음) |
//! | `turn.failed` | `error.message`가 있으면 `CliError` Other, 그리고 `TurnCompleted { failed: true }` |
//! | `error` | `message`(문자열 필수) → `CliError` Other |
//! | `item.started`·`item.updated`·`item.completed` | `item`(object, `type` 문자열 필수). `agent_message`는 completed의 `text`(필수) → `AssistantText`. `mcp_tool_call`·`command_execution`·`file_change`는 `ToolActivity`(started → Started, completed의 `status` `failed` → Failed, 그 밖 → Succeeded) |
//! | 그 밖 | 표시하지 않음 |
//!
//! 공식 문서가 사용 한도·인증 오류를 구분하는 field를 정하지 않으므로 이 profile은 [`super::CliSignal`]을
//! 내지 않습니다. `codex exec`는 한 turn 뒤 종료하므로 오류 turn 뒤 세션은 프로세스 종료로 멈춥니다.

use super::{str_field, DecodeOutput, LineResult};
use crate::chat::{AgentChatItem, CliErrorKind, DisplayText, ToolPhase};
use crate::json::JsonValue;

pub(super) fn decode(kind: &str, value: &JsonValue, out: &mut DecodeOutput) -> LineResult {
    match kind {
        "thread.started" => out
            .items
            .push(AgentChatItem::SessionStarted { model: None }),
        "turn.completed" => out
            .items
            .push(AgentChatItem::TurnCompleted { failed: false }),
        "turn.failed" => {
            if let Some(message) = value.get("error").and_then(|e| str_field(e, "message")) {
                out.items.push(AgentChatItem::CliError {
                    kind: CliErrorKind::Other,
                    text: DisplayText::new(message),
                });
            }
            out.items
                .push(AgentChatItem::TurnCompleted { failed: true });
        }
        "error" => {
            let message = str_field(value, "message").ok_or("shape")?;
            out.items.push(AgentChatItem::CliError {
                kind: CliErrorKind::Other,
                text: DisplayText::new(message),
            });
        }
        "item.started" | "item.updated" | "item.completed" => {
            let item = value.get("item").ok_or("shape")?;
            if item.as_object().is_none() {
                return Err("shape");
            }
            let item_type = str_field(item, "type").ok_or("shape")?;
            let completed = kind == "item.completed";
            let label = match item_type {
                "agent_message" => {
                    if completed {
                        let text = str_field(item, "text").ok_or("shape")?;
                        out.items.push(AgentChatItem::AssistantText {
                            text: DisplayText::new(text),
                        });
                    }
                    return Ok(());
                }
                "mcp_tool_call" => str_field(item, "tool").unwrap_or("mcp_tool_call"),
                "command_execution" => "command_execution",
                "file_change" => "file_change",
                _ => return Ok(()),
            };
            let phase = match kind {
                "item.started" => ToolPhase::Started,
                "item.completed" if str_field(item, "status") == Some("failed") => {
                    ToolPhase::Failed
                }
                "item.completed" => ToolPhase::Succeeded,
                _ => return Ok(()),
            };
            out.items.push(AgentChatItem::ToolActivity {
                call_id: str_field(item, "id").map(DisplayText::label),
                tool: Some(DisplayText::label(label)),
                phase,
            });
        }
        _ => {}
    }
    Ok(())
}
