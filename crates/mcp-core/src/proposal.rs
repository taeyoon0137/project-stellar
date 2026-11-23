//! agent command proposal 입력 해석 (XIX.5.1, ADR-0085 결정 2).
//!
//! MCP tool 인자 `{base_revision, document, commands}`를 L1 적용 요청([`ApplyRequest`])으로 바꿉니다. command
//! object 하나는 `stellar_ops::mcp_command_input::command_input`이 `stellar apply`와 같은 규칙으로 해석하고,
//! 적용 단계는 commands crate의 닫힌 enum만 만듭니다. raw memory patch·임의 script 형식은 없으며 모르는 종류는
//! `unknown-command`로 거부합니다. 입력 schema는 xtask가 같은 표에서 생성한
//! `schemas/command-transaction.schema.json`입니다.

use stellar_ops::apply::{ApplyRequest, MAX_COMMANDS};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::mcp_command_input::{command_input, CommandFields};
use stellar_ops::revision::ProjectRevision;

use crate::json::JsonValue;

/// proposal 입력 오류. 문구에는 요청 위치와 key만 들어가고 프로젝트 내용은 없습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalError {
    /// `unknown-command`, `invalid-transaction`, `too-many-commands`.
    pub code: &'static str,
    /// 원인.
    pub message: String,
}

impl ProposalError {
    fn invalid(message: impl Into<String>) -> Self {
        ProposalError {
            code: "invalid-transaction",
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProposalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// JSON object를 형식 중립 command 접근자로 씁니다.
struct Fields<'a>(&'a JsonValue);

impl CommandFields for Fields<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        self.0
            .as_object()
            .map(|pairs| pairs.iter().map(|(k, _)| k.as_str()).collect())
    }
    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(JsonValue::as_str)
    }
    fn boolean(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(JsonValue::as_bool)
    }
    fn number_text(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(JsonValue::Number(number)) => Some(number.as_str()),
            _ => None,
        }
    }
    fn integer(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(JsonValue::as_i64)
    }
    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }
}

/// tool 인자의 transaction 부분(`base_revision`·`document`·`commands`)을 해석합니다. 선언되지 않은 인자
/// key는 tool 인자 범위 검사(`catalog::scope`)가 먼저 거부합니다. `max_commands`는 MCP 경로 상한입니다.
pub fn parse_transaction(
    args: &JsonValue,
    max_commands: usize,
) -> Result<ApplyRequest, ProposalError> {
    let base_revision = args
        .get("base_revision")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ProposalError::invalid("base_revision 문자열이 필요합니다"))?
        .parse::<ProjectRevision>()
        .map_err(|e| ProposalError::invalid(format!("base_revision: {e}")))?;
    let document = args
        .get("document")
        .and_then(JsonValue::as_str)
        .and_then(|text| text.parse::<StableId>().ok())
        .ok_or_else(|| ProposalError::invalid("document는 StableId여야 합니다"))?;
    let items = args
        .get("commands")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| ProposalError::invalid("commands는 배열이어야 합니다"))?;
    if items.is_empty() {
        return Err(ProposalError::invalid("적용할 command가 없습니다"));
    }
    let limit = max_commands.min(MAX_COMMANDS);
    if items.len() > limit {
        return Err(ProposalError {
            code: "too-many-commands",
            message: format!("command {}개는 상한 {limit}개를 넘습니다", items.len()),
        });
    }
    let commands = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            command_input(&Fields(item), &format!("commands[{index}]")).map_err(|e| ProposalError {
                code: e.code,
                message: e.message,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ApplyRequest {
        base_revision,
        document,
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, JsonLimits};
    use stellar_ops::apply::{decode_commands, ApplyError, CommandInput};

    const DOC: &str = "0195c530-3acd-7bc1-8414-c213f61cb002";
    const NODE: &str = "0195c530-3acd-7bc1-8414-c213f61cb001";

    fn args(commands: &str) -> JsonValue {
        parse(
            &format!(r#"{{"base_revision":"g0","document":"{DOC}","commands":{commands}}}"#),
            JsonLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn proposal_uses_only_the_closed_enum() {
        // 검증 기준(XIX.5.1): unknown command 거부.
        for kind in ["raw-patch", "script", "memory"] {
            let error =
                parse_transaction(&args(&format!(r#"[{{"kind":"{kind}"}}]"#)), 100).unwrap_err();
            assert_eq!(error.code, "unknown-command", "{kind}");
        }
        // canonical bytes로 closed enum 밖 종류를 선언해도 적용 전 변환에서 거부됩니다.
        let request =
            parse_transaction(&args(r#"[{"kind":"raw-patch","canonical":"0000"}]"#), 100).unwrap();
        assert!(matches!(
            decode_commands(&request.commands),
            Err(ApplyError::UnknownCommand { index: 0, .. })
        ));
        // 필드 형식은 그대로 CommandInput이 됩니다.
        let request = parse_transaction(
            &args(&format!(
                r#"[{{"kind":"rename","node":"{NODE}","name":"A"}}]"#
            )),
            100,
        )
        .unwrap();
        assert_eq!(request.document, DOC.parse().unwrap());
        assert!(matches!(
            request.commands.as_slice(),
            [CommandInput::Rename { .. }]
        ));
    }

    #[test]
    fn envelope_and_count_are_checked_before_commands() {
        assert_eq!(
            parse_transaction(&args("[]"), 100).unwrap_err().code,
            "invalid-transaction"
        );
        let two = format!(
            r#"[{{"kind":"delete-subtree","root":"{NODE}"}},{{"kind":"delete-subtree","root":"{NODE}"}}]"#
        );
        assert_eq!(
            parse_transaction(&args(&two), 1).unwrap_err().code,
            "too-many-commands"
        );
        let bad = parse(
            r#"{"base_revision":"x","document":"y","commands":[]}"#,
            JsonLimits::default(),
        )
        .unwrap();
        assert_eq!(
            parse_transaction(&bad, 1).unwrap_err().code,
            "invalid-transaction"
        );
    }
}
