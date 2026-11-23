//! 자동 저장 trace (XIII.1.7).
//!
//! 자동 저장 하나마다 dirty revision, 캡처·직렬화·쓰기 시간, 결과를 남깁니다. 필드는 숫자와 정적 결과 code뿐이라
//! 경로·프로젝트 이름·문서 내용을 담을 수 없습니다(II.3.2 telemetry 규칙과 같음). 한 줄 JSON
//! ([`AutosaveTrace::to_json_line`])의 key 집합과 순서가 [`TRACE_SCHEMA`]이고, [`validate_trace_line`]이 그
//! schema를 검사합니다.

use std::fmt;
use std::time::Duration;

use stellar_core_types::revision::DocumentRevision;

/// trace 한 줄의 schema 이름.
pub const TRACE_SCHEMA_NAME: &str = "stellar-autosave-trace";

/// trace 한 줄의 schema version.
pub const TRACE_SCHEMA_VERSION: u32 = 1;

/// 한 줄 JSON의 key(순서 고정). 값은 모두 숫자이고 `result`만 [`AutosaveTraceResult::code`] 문자열입니다.
pub const TRACE_SCHEMA: [&str; 7] = [
    "schema",
    "schema_version",
    "dirty_revision",
    "capture_ns",
    "serialize_ns",
    "write_ns",
    "result",
];

/// 자동 저장 결과 code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutosaveTraceResult {
    /// 새 generation을 썼습니다.
    Saved,
    /// 바뀐 chunk가 없어 쓰지 않았습니다.
    Unchanged,
    /// 다른 writer가 먼저 저장해 쓰지 않았습니다.
    Stale,
    /// 쓰기 실패.
    Failed,
    /// worker가 끝내지 못했습니다(종료·panic).
    Interrupted,
}

impl AutosaveTraceResult {
    /// 모든 값(schema 검사용).
    pub const ALL: [AutosaveTraceResult; 5] = [
        AutosaveTraceResult::Saved,
        AutosaveTraceResult::Unchanged,
        AutosaveTraceResult::Stale,
        AutosaveTraceResult::Failed,
        AutosaveTraceResult::Interrupted,
    ];

    /// 안정 문자열 code.
    pub fn code(self) -> &'static str {
        match self {
            AutosaveTraceResult::Saved => "saved",
            AutosaveTraceResult::Unchanged => "unchanged",
            AutosaveTraceResult::Stale => "stale",
            AutosaveTraceResult::Failed => "failed",
            AutosaveTraceResult::Interrupted => "interrupted",
        }
    }
}

/// 자동 저장 하나의 trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutosaveTrace {
    /// 캡처한 작업 문서 revision.
    pub dirty_revision: DocumentRevision,
    /// document thread 캡처 시간.
    pub capture: Duration,
    /// writer의 직렬화 시간(문서 레코드 → chunk).
    pub serialize: Duration,
    /// writer의 잠금·base 검사·chunk 쓰기·current pointer 교체 시간.
    pub write: Duration,
    /// 결과.
    pub result: AutosaveTraceResult,
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

impl AutosaveTrace {
    /// [`TRACE_SCHEMA`] 순서의 한 줄 JSON(끝 줄바꿈 없음).
    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"schema\":\"{TRACE_SCHEMA_NAME}\",\"schema_version\":{TRACE_SCHEMA_VERSION},\
             \"dirty_revision\":{},\"capture_ns\":{},\"serialize_ns\":{},\"write_ns\":{},\
             \"result\":\"{}\"}}",
            self.dirty_revision.0,
            nanos(self.capture),
            nanos(self.serialize),
            nanos(self.write),
            self.result.code()
        )
    }
}

/// trace 한 줄이 schema를 어긴 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSchemaError(pub String);

impl fmt::Display for TraceSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "자동 저장 trace schema 위반: {}", self.0)
    }
}

impl std::error::Error for TraceSchemaError {}

/// 한 줄 JSON이 [`TRACE_SCHEMA`]를 따르는지 검사합니다: 평평한 object, key 집합·순서 일치, `schema`·`result`만
/// 정해진 문자열이고 나머지는 부호 없는 정수. 그 밖의 문자열 값(경로·이름)이 들어갈 자리는 없습니다.
pub fn validate_trace_line(line: &str) -> Result<(), TraceSchemaError> {
    let error = |message: &str| Err(TraceSchemaError(message.to_string()));
    let Some(body) = line
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return error("object가 아닙니다");
    };
    let fields: Vec<&str> = body.split(',').collect();
    if fields.len() != TRACE_SCHEMA.len() {
        return error("key 수가 다릅니다");
    }
    for (field, key) in fields.iter().zip(TRACE_SCHEMA) {
        let Some((name, value)) = field.split_once(':') else {
            return error("key:value 형식이 아닙니다");
        };
        if name != format!("\"{key}\"") {
            return error("key 이름·순서가 다릅니다");
        }
        let valid = match key {
            "schema" => value == format!("\"{TRACE_SCHEMA_NAME}\""),
            "schema_version" => value == TRACE_SCHEMA_VERSION.to_string(),
            "result" => AutosaveTraceResult::ALL
                .iter()
                .any(|result| value == format!("\"{}\"", result.code())),
            _ => !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        };
        if !valid {
            return error("값이 schema와 다릅니다");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_lines_follow_the_schema_for_every_result() {
        for result in AutosaveTraceResult::ALL {
            let trace = AutosaveTrace {
                dirty_revision: DocumentRevision(42),
                capture: Duration::from_micros(460),
                serialize: Duration::from_millis(12),
                write: Duration::from_millis(30),
                result,
            };
            let line = trace.to_json_line();
            assert_eq!(validate_trace_line(&line), Ok(()), "{line}");
        }
        let golden = AutosaveTrace {
            dirty_revision: DocumentRevision(7),
            capture: Duration::from_nanos(1),
            serialize: Duration::from_nanos(2),
            write: Duration::from_nanos(3),
            result: AutosaveTraceResult::Saved,
        };
        assert_eq!(
            golden.to_json_line(),
            "{\"schema\":\"stellar-autosave-trace\",\"schema_version\":1,\"dirty_revision\":7,\
             \"capture_ns\":1,\"serialize_ns\":2,\"write_ns\":3,\"result\":\"saved\"}"
        );
    }

    #[test]
    fn lines_with_paths_names_or_extra_keys_are_rejected() {
        let good = AutosaveTrace {
            dirty_revision: DocumentRevision(1),
            capture: Duration::ZERO,
            serialize: Duration::ZERO,
            write: Duration::ZERO,
            result: AutosaveTraceResult::Stale,
        }
        .to_json_line();
        for bad in [
            good.replace("\"result\":\"stale\"", "\"result\":\"/Users/me/project\""),
            good.replace("\"dirty_revision\":1", "\"dirty_revision\":\"문서 이름\""),
            good.replace('}', ",\"path\":\"/tmp/x\"}"),
            good.replace("\"capture_ns\"", "\"project\""),
            good.replace("schema_version\":1", "schema_version\":2"),
            "not json".to_string(),
        ] {
            assert!(validate_trace_line(&bad).is_err(), "{bad}");
        }
    }
}
