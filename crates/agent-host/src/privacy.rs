//! 에이전트 관련 금지 필드 corpus (XXIX.6.4, XVI.8.2·XVI.8.6, ADR-0005 §4.2·§4.4, ADR-0101 결정 7).
//!
//! 진단, recovery snapshot, 텔레메트리에 들어가면 안 되는 에이전트 필드와 그 표지 값입니다. XVI.8.2 금지 필드
//! scanner는 이 목록을 corpus로 함께 읽고, XVI.8.6 schema inventory는 [`AGENT_FORBIDDEN_FIELDS`]의 이름이 진단
//! schema key에 없음을 확인합니다. 이 crate의 test(`tests/privacy_fields.rs`)는 표지 값을 세션의 모든 입력에
//! 넣고 진단·`Debug`·고지 기록 출력에서 [`scan`] 결과가 0인지 봅니다.
//!
//! 표지 값은 실제 계정·경로·내용이 아니라 고정 문자열입니다(fixture privacy 원칙, XVI.8).

/// 금지 필드 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForbiddenField {
    /// 필드 이름(schema inventory 기준 낱말).
    pub name: &'static str,
    /// 뜻.
    pub description: &'static str,
    /// test가 입력에 넣는 표지 값. 출력에 이 값이 보이면 누출입니다.
    pub marker: &'static str,
}

/// 에이전트 금지 필드(계획서 XXIX.6.4 목록 순서).
pub const AGENT_FORBIDDEN_FIELDS: [ForbiddenField; 9] = [
    ForbiddenField {
        name: "agent_user_prompt",
        description: "사용자 입력 문장",
        marker: "stellar-privacy-prompt-7f3a",
    },
    ForbiddenField {
        name: "agent_chat_item",
        description: "채팅 item 텍스트",
        marker: "stellar-privacy-chat-7f3a",
    },
    ForbiddenField {
        name: "agent_raw_output",
        description: "원문 버퍼 내용",
        marker: "stellar-privacy-raw-7f3a",
    },
    ForbiddenField {
        name: "agent_cli_stderr",
        description: "CLI 표준 오류",
        marker: "stellar-privacy-stderr-7f3a",
    },
    ForbiddenField {
        name: "agent_cli_executable_path",
        description: "CLI 실행 파일 경로",
        marker: "stellar-privacy-cli-path-7f3a",
    },
    ForbiddenField {
        name: "agent_cli_account_identity",
        description: "CLI가 보고한 계정 식별 정보",
        marker: "stellar-privacy-account-7f3a",
    },
    ForbiddenField {
        name: "agent_session_binding",
        description: "세션 결합 수단(결합 token)",
        marker: "7f3a7f3a7f3a7f3a",
    },
    ForbiddenField {
        name: "agent_ipc_peer",
        description: "IPC 상대 정보",
        marker: "stellar-privacy-peer-7f3a",
    },
    ForbiddenField {
        name: "agent_provenance_index",
        description: "에이전트 변경 출처 색인",
        marker: "stellar-privacy-provenance-7f3a",
    },
];

/// `text`에 보이는 금지 필드의 이름(표지 값 또는 필드 이름이 나온 것). 비어야 합니다.
pub fn scan(text: &str) -> Vec<&'static str> {
    let lower = text.to_ascii_lowercase();
    AGENT_FORBIDDEN_FIELDS
        .iter()
        .filter(|f| lower.contains(f.marker) || lower.contains(f.name))
        .map(|f| f.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_detects_every_field() {
        for f in AGENT_FORBIDDEN_FIELDS {
            assert_eq!(scan(&format!("x {} y", f.marker.to_uppercase())), [f.name]);
            assert_eq!(scan(&format!("{{\"{}\":1}}", f.name)), [f.name]);
        }
        assert!(scan("agent-session=1 duration_ms=5 error=user-stopped").is_empty());
    }
}
