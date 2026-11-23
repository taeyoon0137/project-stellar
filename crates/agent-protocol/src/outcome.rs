//! tool·resource 결과 종류와 XXIV.1.2 종료 코드 대응 (XIX.9.9, ADR-0085 결정 7).
//!
//! MCP 결과의 `kind`(성공 `structuredContent.kind`, 실패 `structuredContent.kind`·JSON-RPC `error.data.kind`)와
//! `stellar`·`stellar-mcp` 종료 코드(sysexits 기반 XXIV.1.2 표)를 한 표로 고정합니다. 같은 실패가 CLI에서 났을 때의
//! 종료 코드와 같습니다. `stellar-mcp` 프로세스가 시작 단계에서 끝날 때도 이 표를 씁니다.

/// XXIV.1.2 종료 코드 하나(값, 이름).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode {
    /// 프로세스 종료 코드.
    pub code: u8,
    /// XXIV.1.2 `exit_name`.
    pub name: &'static str,
}

const fn exit(code: u8, name: &'static str) -> ExitCode {
    ExitCode { code, name }
}

/// 결과 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResultKind {
    /// 성공.
    Ok,
    /// 도메인 거부(validator가 command를 거부, 결과 문서가 불변식 위반).
    Rejected,
    /// 이 backend에 없는 tool(JSON-RPC protocol 오류).
    UnknownTool,
    /// 선언되지 않은 인자, 형식 오류, 모르는 command 종류.
    InvalidArguments,
    /// base revision·cursor가 현재 revision과 다름.
    Stale,
    /// 응답이 상한을 넘음(범위를 줄여 다시 요청).
    TooLarge,
    /// 문서·node·View·resource가 없음.
    NotFound,
    /// 필요한 구성 요소(Git, 텍스트 원본, 프로젝트 snapshot) 없음.
    Unavailable,
    /// 내부 오류.
    Internal,
    /// 호출량 상한 초과(재시도 가능).
    Busy,
    /// 호출 대기 상한 초과(재시도 가능).
    Timeout,
    /// 프로젝트 신뢰·identity 확인 실패.
    NotTrusted,
    /// 승인 프로젝트·세션 밖 대상.
    OutOfScope,
}

impl ResultKind {
    /// 모든 결과 종류(고정 순서, 종료 코드 오름차순).
    pub const ALL: [ResultKind; 13] = [
        ResultKind::Ok,
        ResultKind::Rejected,
        ResultKind::UnknownTool,
        ResultKind::InvalidArguments,
        ResultKind::Stale,
        ResultKind::TooLarge,
        ResultKind::NotFound,
        ResultKind::Unavailable,
        ResultKind::Internal,
        ResultKind::Busy,
        ResultKind::Timeout,
        ResultKind::NotTrusted,
        ResultKind::OutOfScope,
    ];

    /// 결과 JSON의 고정 이름(kebab-case).
    pub fn name(self) -> &'static str {
        match self {
            ResultKind::Ok => "ok",
            ResultKind::Rejected => "rejected",
            ResultKind::UnknownTool => "unknown-tool",
            ResultKind::InvalidArguments => "invalid-arguments",
            ResultKind::Stale => "stale",
            ResultKind::TooLarge => "too-large",
            ResultKind::NotFound => "not-found",
            ResultKind::Unavailable => "unavailable",
            ResultKind::Internal => "internal",
            ResultKind::Busy => "busy",
            ResultKind::Timeout => "timeout",
            ResultKind::NotTrusted => "not-trusted",
            ResultKind::OutOfScope => "out-of-scope",
        }
    }

    /// 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<ResultKind> {
        ResultKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// XXIV.1.2 종료 코드.
    pub fn exit(self) -> ExitCode {
        match self {
            ResultKind::Ok => exit(0, "ok"),
            ResultKind::Rejected => exit(1, "failure"),
            ResultKind::UnknownTool => exit(64, "usage"),
            ResultKind::InvalidArguments | ResultKind::Stale | ResultKind::TooLarge => {
                exit(65, "dataerr")
            }
            ResultKind::NotFound => exit(66, "noinput"),
            ResultKind::Unavailable => exit(69, "unavailable"),
            ResultKind::Internal => exit(70, "software"),
            ResultKind::Busy | ResultKind::Timeout => exit(75, "tempfail"),
            ResultKind::NotTrusted | ResultKind::OutOfScope => exit(77, "noperm"),
        }
    }

    /// 같은 요청을 그대로 다시 보내면 성공할 수 있는지(일시적 실패).
    pub fn retryable(self) -> bool {
        matches!(self, ResultKind::Busy | ResultKind::Timeout)
    }

    /// MCP protocol 오류(JSON-RPC `error`)로 보내는지. 나머지 실패는 tool 실행 오류(`isError: true` 결과)입니다
    /// (MCP 명세 2025-11-25 "Tools" Error Handling: 모르는 tool은 protocol 오류, 실행·입력 검증 오류는 결과).
    pub fn is_protocol_error(self) -> bool {
        self == ResultKind::UnknownTool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_exit_table_golden() {
        // 검증 기준(XIX.9.9): 대응표 golden.
        let table: Vec<(&str, u8, &str)> = ResultKind::ALL
            .iter()
            .map(|k| (k.name(), k.exit().code, k.exit().name))
            .collect();
        assert_eq!(
            table,
            [
                ("ok", 0, "ok"),
                ("rejected", 1, "failure"),
                ("unknown-tool", 64, "usage"),
                ("invalid-arguments", 65, "dataerr"),
                ("stale", 65, "dataerr"),
                ("too-large", 65, "dataerr"),
                ("not-found", 66, "noinput"),
                ("unavailable", 69, "unavailable"),
                ("internal", 70, "software"),
                ("busy", 75, "tempfail"),
                ("timeout", 75, "tempfail"),
                ("not-trusted", 77, "noperm"),
                ("out-of-scope", 77, "noperm"),
            ]
        );
        for kind in ResultKind::ALL {
            assert_eq!(ResultKind::from_name(kind.name()), Some(kind));
        }
        let retryable: Vec<&str> = ResultKind::ALL
            .iter()
            .filter(|k| k.retryable())
            .map(|k| k.name())
            .collect();
        assert_eq!(retryable, ["busy", "timeout"]);
        assert!(ResultKind::ALL
            .iter()
            .filter(|k| k.is_protocol_error())
            .eq([ResultKind::UnknownTool].iter()));
    }
}
