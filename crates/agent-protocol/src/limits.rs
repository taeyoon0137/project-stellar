//! MCP 경로의 자원 상한 (XIX.9.10, ADR-0085 결정 4).
//!
//! 값은 아키텍처 §6.2~§6.4(인터랙션 경로의 단일 점유 2 ms, 외부 프로세스·Git의 동기 실행 금지)와 실측으로
//! 정했습니다. 실측 값과 근거는 `docs/task-reports/XIX.9.10.md`에 있습니다. 상한을 넘으면 처리하지 않고 거부하거나
//! (`too-large`, `invalid-arguments`) busy로 응답합니다(`busy`, `timeout`, 재시도 가능).

use std::time::Duration;

/// 수신·송신 JSON-RPC 메시지 하나의 최대 byte 수(1 MiB). 넘는 응답은 `too-large`로 바꿉니다.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// page 기본 항목 수.
pub const DEFAULT_PAGE_ITEMS: usize = 200;

/// page 최대 항목 수. 범위 밖 `limit`은 `invalid-arguments`입니다.
pub const MAX_PAGE_ITEMS: usize = 1000;

/// page 내용 JSON 최대 byte 수. tool 결과는 같은 내용을 `structuredContent`와 직렬화 text block에 두 번 싣으므로
/// (XIX.9.6) 메시지 상한 안에 escape 여유까지 들어가도록 3/8로 둡니다.
pub const MAX_PAGE_BYTES: usize = MAX_MESSAGE_BYTES / 8 * 3;

/// 요청 하나의 호출 대기 상한. 넘으면 결과를 버리고 `timeout`으로 응답합니다.
pub const CALL_DEADLINE: Duration = Duration::from_secs(30);

/// 호출량 창 길이.
pub const RATE_WINDOW: Duration = Duration::from_secs(1);

/// 연결 하나가 [`RATE_WINDOW`] 안에 보낼 수 있는 요청 수(`ping`·`initialize` 제외). 넘으면 `busy`입니다.
pub const MAX_CALLS_PER_WINDOW: usize = 100;

/// stdio 연결 하나에서 동시에 처리하는 요청 수. 요청을 읽은 순서대로 하나씩 처리합니다.
pub const MAX_IN_FLIGHT: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_follow_adr_0085() {
        assert_eq!(MAX_MESSAGE_BYTES, 1_048_576);
        assert_eq!((DEFAULT_PAGE_ITEMS, MAX_PAGE_ITEMS), (200, 1000));
        assert_eq!(MAX_PAGE_BYTES, 393_216);
        assert_eq!(CALL_DEADLINE, Duration::from_secs(30));
        assert_eq!(
            (RATE_WINDOW, MAX_CALLS_PER_WINDOW),
            (Duration::from_secs(1), 100)
        );
        assert_eq!(MAX_IN_FLIGHT, 1);
        // page 한도 안의 결과(두 번 실림)가 메시지 상한 안에 들어갑니다.
        const { assert!(MAX_PAGE_BYTES * 2 < MAX_MESSAGE_BYTES) };
    }

    #[test]
    fn tool_schemas_use_the_page_limit() {
        // page 인자를 받는 tool은 schema 상한이 MAX_PAGE_ITEMS와 같습니다.
        let fragment = format!("\"maximum\":{MAX_PAGE_ITEMS}}}");
        for tool in crate::catalog::tools::all_tools() {
            if tool.input_schema.contains("\"limit\"") {
                assert!(tool.input_schema.contains(&fragment), "{}", tool.name);
            }
        }
    }
}
