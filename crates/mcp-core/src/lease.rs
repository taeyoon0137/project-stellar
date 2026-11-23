//! 에이전트 편집 대상 선언 `document.begin_agent_edit`의 tool 정의와 인자 해석 (XIX.4.1, XIX.4.4, ADR-0093,
//! ADR-0112).
//!
//! 브리지는 tool 인자의 형식(StableId, 빈 대상·중복·설명 길이, 전역 resource 종류)만 검사해 [`BeginAgentEdit`]로
//! 바꿉니다. 문서 기준 대상 확인, 승인 대기·lease 획득·해제 같은 요청 관리는 앱이 소유하는 에이전트 편집 crate
//! (`stellar_agent_edit::lease`)가 맡고, 이 crate는 그 crate에 닿지 않습니다(ADR-0112). 잠금 판정·겹침·파일 공유는
//! `stellar_commands::lock::{lease, coordinator}` 한 곳에 있습니다.

use std::fmt;

use stellar_ops::commands::lock::lease::{ExternalKind, ExternalResource};
use stellar_ops::core_types::ids::StableId;

use crate::agent_protocol::catalog::resources::Availability;
use crate::agent_protocol::catalog::tools::ToolDef;
use crate::agent_protocol::outcome::ResultKind;
use crate::agent_protocol::permission::Permission;
use crate::json::JsonValue;

/// 대상 root·전역 resource 수 상한(tool schema `maxItems`와 같음).
pub const MAX_TARGETS: usize = 1000;
/// 작업 설명 최대 길이(문자 수, schema `maxLength`와 같음).
pub const MAX_DESCRIPTION_CHARS: usize = 2000;

/// `document.begin_agent_edit` tool 정의(attached 전용). 대상 선언과 잠금 요청이며 문서를 바꾸지 않으므로
/// Propose 권한입니다. 승인은 Stellar 사용자 UI가 받습니다.
pub const BEGIN_AGENT_EDIT: ToolDef = ToolDef {
    name: "document.begin_agent_edit",
    title: "Begin agent edit",
    description: "편집할 target root node, 따로 잠글 전역 resource, 작업 설명을 제출해 사용자 승인 뒤 편집 lease를 요청합니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","targets","description"],"properties":{"document":{"type":"string"},"targets":{"type":"array","minItems":1,"maxItems":1000,"items":{"type":"string"}},"externals":{"type":"array","maxItems":1000,"items":{"type":"object","additionalProperties":false,"required":["kind","id"],"properties":{"kind":{"type":"string","enum":["token","component-definition","asset"]},"id":{"type":"string"}}}},"description":{"type":"string","minLength":1,"maxLength":2000}}}"#,
    read_only: false,
    destructive: false,
    idempotent: false,
    availability: Availability::AttachedOnly,
    grant: Permission::Propose,
};

/// 해석한 `BeginAgentEdit` 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginAgentEdit {
    /// 대상 문서.
    pub document: StableId,
    /// 잠글 subtree root(제출 순서).
    pub targets: Vec<StableId>,
    /// 따로 잠글 전역 resource(XIX.4.4).
    pub externals: Vec<ExternalResource>,
    /// 사용자에게 보일 작업 설명(데이터로만 다룸).
    pub description: String,
}

/// lease 요청 오류. 문구에는 요청 위치와 StableId만 들어갑니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRequestError {
    /// 고정 코드.
    pub code: &'static str,
    /// MCP 결과 종류.
    pub kind: ResultKind,
    /// 원인.
    pub message: String,
}

impl LeaseRequestError {
    fn new(code: &'static str, kind: ResultKind, message: impl Into<String>) -> Self {
        LeaseRequestError {
            code,
            kind,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid-arguments", ResultKind::InvalidArguments, message)
    }
}

impl fmt::Display for LeaseRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LeaseRequestError {}

fn stable_id(value: Option<&JsonValue>, at: &str) -> Result<StableId, LeaseRequestError> {
    value
        .and_then(JsonValue::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| LeaseRequestError::invalid(format!("{at}는 StableId여야 합니다")))
}

/// tool 인자를 해석합니다(XIX.4.1). 빈 대상, 중복 대상, 빈 설명은 거부합니다. 선언되지 않은 key는 tool 인자
/// 범위 검사(`catalog::scope`)가 먼저 거부합니다.
pub fn parse_begin_agent_edit(args: &JsonValue) -> Result<BeginAgentEdit, LeaseRequestError> {
    let document = stable_id(args.get("document"), "document")?;
    let items = args
        .get("targets")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| LeaseRequestError::invalid("targets는 배열이어야 합니다"))?;
    if items.is_empty() {
        return Err(LeaseRequestError::new(
            "empty-targets",
            ResultKind::InvalidArguments,
            "편집할 target root가 없습니다",
        ));
    }
    if items.len() > MAX_TARGETS {
        return Err(LeaseRequestError::invalid(format!(
            "target {}개는 상한 {MAX_TARGETS}개를 넘습니다",
            items.len()
        )));
    }
    let mut targets = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let id = stable_id(Some(item), &format!("targets[{index}]"))?;
        if targets.contains(&id) {
            return Err(LeaseRequestError::invalid(format!(
                "targets[{index}] {id}가 중복입니다"
            )));
        }
        targets.push(id);
    }
    let mut externals = Vec::new();
    if let Some(value) = args.get("externals") {
        let items = value
            .as_array()
            .ok_or_else(|| LeaseRequestError::invalid("externals는 배열이어야 합니다"))?;
        if items.len() > MAX_TARGETS {
            return Err(LeaseRequestError::invalid("externals 수가 상한을 넘습니다"));
        }
        for (index, item) in items.iter().enumerate() {
            let at = format!("externals[{index}]");
            let keys_ok = item
                .as_object()
                .is_some_and(|pairs| pairs.iter().all(|(k, _)| k == "kind" || k == "id"));
            if !keys_ok {
                return Err(LeaseRequestError::invalid(format!(
                    "{at}는 kind·id만 가진 object여야 합니다"
                )));
            }
            let kind = item
                .get("kind")
                .and_then(JsonValue::as_str)
                .and_then(ExternalKind::from_name)
                .ok_or_else(|| {
                    LeaseRequestError::invalid(format!("{at}.kind가 올바르지 않습니다"))
                })?;
            let id = stable_id(item.get("id"), &format!("{at}.id"))?;
            let resource = ExternalResource { kind, id };
            if externals.contains(&resource) {
                return Err(LeaseRequestError::invalid(format!("{at}가 중복입니다")));
            }
            externals.push(resource);
        }
    }
    let description = args
        .get("description")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| LeaseRequestError::invalid("description 문자열이 필요합니다"))?;
    if description.trim().is_empty() {
        return Err(LeaseRequestError::invalid("작업 설명이 비었습니다"));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(LeaseRequestError::invalid(format!(
            "작업 설명은 {MAX_DESCRIPTION_CHARS}자를 넘을 수 없습니다"
        )));
    }
    Ok(BeginAgentEdit {
        document,
        targets,
        externals,
        description: description.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    //! BeginAgentEdit 인자 해석 test (XIX.4.1). 문서 기준 대상 확인과 요청 관리 test는
    //! `stellar_agent_edit::lease`에 있습니다.

    use super::*;
    use crate::json::{parse, JsonLimits};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn args(text: &str) -> JsonValue {
        parse(text, JsonLimits::default()).unwrap()
    }

    #[test]
    fn tool_definition_is_attached_only_and_well_formed() {
        assert!(crate::agent_protocol::catalog::tools::valid_tool_name(
            BEGIN_AGENT_EDIT.name
        ));
        assert_eq!(BEGIN_AGENT_EDIT.availability, Availability::AttachedOnly);
        assert_eq!(BEGIN_AGENT_EDIT.grant, Permission::Propose);
        let schema = args(BEGIN_AGENT_EDIT.input_schema);
        assert_eq!(
            schema
                .get("additionalProperties")
                .and_then(JsonValue::as_bool),
            Some(false)
        );
    }

    #[test]
    fn begin_agent_edit_rejects_malformed_arguments() {
        // 검증 기준(XIX.4.1): empty target 거부. 없는 node 거부는 문서를 아는 앱 쪽 test입니다.
        let doc = sid(1);
        let empty = format!(r#"{{"document":"{doc}","targets":[],"description":"x"}}"#);
        assert_eq!(
            parse_begin_agent_edit(&args(&empty)).unwrap_err().code,
            "empty-targets"
        );
        for bad in [
            format!(r#"{{"document":"{doc}","description":"x"}}"#),
            format!(r#"{{"document":"{doc}","targets":["nope"],"description":"x"}}"#),
            format!(
                r#"{{"document":"{doc}","targets":["{0}","{0}"],"description":"x"}}"#,
                sid(13)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"  "}}"#,
                sid(13)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"script","id":"{}"}}]}}"#,
                sid(13),
                sid(5)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"token","id":"{}","path":"/"}}]}}"#,
                sid(13),
                sid(5)
            ),
        ] {
            assert_eq!(
                parse_begin_agent_edit(&args(&bad)).unwrap_err().kind,
                ResultKind::InvalidArguments,
                "{bad}"
            );
        }
        // 정상 요청: 대상 root와 전역 resource.
        let ok = format!(
            r#"{{"document":"{doc}","targets":["{}"],"description":"작업","externals":[{{"kind":"component-definition","id":"{}"}}]}}"#,
            sid(13),
            sid(600)
        );
        let request = parse_begin_agent_edit(&args(&ok)).unwrap();
        assert_eq!(request.document, doc);
        assert_eq!(request.targets, [sid(13)]);
        assert_eq!(
            request.externals,
            [ExternalResource {
                kind: ExternalKind::ComponentDefinition,
                id: sid(600)
            }]
        );
        assert_eq!(request.description, "작업");
    }
}
