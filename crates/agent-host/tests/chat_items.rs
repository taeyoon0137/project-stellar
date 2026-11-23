//! 채팅 보기 모델 (XXIX.2.10): variant별 golden 직렬화.

use stellar_agent_host::chat::{
    AgentChatItem, CliErrorKind, DisplayText, ToolPhase, MAX_ITEM_TEXT_BYTES,
};

fn all_variants() -> Vec<AgentChatItem> {
    vec![
        AgentChatItem::UserPrompt {
            text: DisplayText::new("버튼을 정렬해 줘"),
        },
        AgentChatItem::AssistantText {
            text: DisplayText::new("정렬 제안을 만들었습니다.\n"),
        },
        AgentChatItem::ToolActivity {
            call_id: Some(DisplayText::label("call-1")),
            tool: Some(DisplayText::label("mcp__stellar__propose_edit")),
            phase: ToolPhase::Started,
        },
        AgentChatItem::ToolActivity {
            call_id: None,
            tool: None,
            phase: ToolPhase::Succeeded,
        },
        AgentChatItem::ToolActivity {
            call_id: None,
            tool: None,
            phase: ToolPhase::Failed,
        },
        AgentChatItem::SessionStarted {
            model: Some(DisplayText::label("model-x")),
        },
        AgentChatItem::SessionStarted { model: None },
        AgentChatItem::TurnCompleted { failed: false },
        AgentChatItem::CliError {
            kind: CliErrorKind::UsageLimit,
            text: DisplayText::new("rate_limit"),
        },
        AgentChatItem::CliError {
            kind: CliErrorKind::Authentication,
            text: DisplayText::new("auth"),
        },
        AgentChatItem::CliError {
            kind: CliErrorKind::Other,
            text: DisplayText::new("\u{1b}[31mx"),
        },
        AgentChatItem::DecodeStopped {
            code: "invalid-json",
        },
        AgentChatItem::AssistantText {
            text: DisplayText::new(&"a".repeat(MAX_ITEM_TEXT_BYTES + 1)),
        },
    ]
}

#[test]
fn variant_golden_serialization() {
    let rendered: String = all_variants()
        .iter()
        .map(|i| {
            let line = i.to_golden();
            // 상한 사례는 길이 대신 요약으로 고정합니다.
            if line.len() > 200 {
                format!(
                    "{} truncated={}\n",
                    i.kind(),
                    line.contains("\"truncated\":true")
                )
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    let path = format!(
        "{}/tests/fixtures/chat-items.golden",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("STELLAR_BLESS_DECODER_GOLDEN").is_some() {
        std::fs::write(&path, &rendered).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), rendered);
    // 모든 variant 이름이 golden에 있습니다(closed enum 전체).
    for kind in [
        "user-prompt",
        "assistant-text",
        "tool-activity",
        "session-started",
        "turn-completed",
        "cli-error",
        "decode-stopped",
    ] {
        assert!(rendered.contains(kind), "{kind}");
    }
}
