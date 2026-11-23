//! stellar-diagnostics — 구조화 로그·진단 이벤트.
//!
//! 프로젝트 내용·텍스트·경로 같은 민감 데이터는 [`Sensitive`]에만 담을 수 있고,
//! 비민감 [`TelemetryEvent`]의 기본 필드에는 넣을 수 없습니다(타입으로 강제).

/// CPU/GPU 메모리 예산 provider와 정규화 bucket (XVI.5.1).
pub mod memory;

/// 상호작용 지연 trace T0~T5 (XVI.1.1~XVI.1.4).
pub mod interaction;

/// ring buffer 로컬 trace store (XVI.1.5).
pub mod trace_store;

/// 단계 trace record schema (XI.2.6).
pub mod trace_schema;




/// 민감 데이터(프로젝트 내용·텍스트·경로) 래퍼.
///
/// telemetry 기본 필드에 직접 넣을 수 없고, [`SensitiveDiagnostic`]를 통해서만 다룹니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sensitive(String);

impl Sensitive {
    /// 민감 값을 감쌉니다.
    pub fn new(value: impl Into<String>) -> Self {
        Sensitive(value.into())
    }

    /// 의도적으로 민감 값을 열람합니다(로컬 진단 등 명시적 경로에서만).
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

/// 비민감 telemetry 이벤트.
///
/// 기본 필드는 컴파일 타임 상수 이름과 숫자 코드만 허용합니다. 런타임 문자열·민감 데이터는
/// 담을 수 없습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryEvent {
    /// 정적 이벤트 이름(민감하지 않음).
    pub name: &'static str,
    /// 숫자 이벤트 코드.
    pub code: u32,
}

/// 민감 데이터가 필요한 진단. 비민감 이벤트와 민감 detail을 분리해 담습니다.
///
/// ```
/// use stellar_diagnostics::{Sensitive, SensitiveDiagnostic, TelemetryEvent};
/// let d = SensitiveDiagnostic {
///     event: TelemetryEvent { name: "open_project", code: 10 },
///     detail: Sensitive::new("/private/project/path"),
/// };
/// assert_eq!(d.event.name, "open_project");
/// assert_eq!(d.detail.reveal(), "/private/project/path");
/// ```
///
/// 민감 타입을 비민감 telemetry 필드에 넣으면 컴파일에 실패합니다.
///
/// ```compile_fail
/// use stellar_diagnostics::{Sensitive, TelemetryEvent};
/// let s = Sensitive::new("/private/path");
/// // name은 &'static str만 허용하므로 Sensitive를 넣을 수 없습니다.
/// let _ = TelemetryEvent { name: s, code: 0 };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensitiveDiagnostic {
    /// 비민감 이벤트.
    pub event: TelemetryEvent,
    /// 민감 detail.
    pub detail: Sensitive,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_event_is_non_sensitive() {
        let event = TelemetryEvent {
            name: "save",
            code: 1,
        };
        assert_eq!(event.name, "save");
        assert_eq!(event.code, 1);
    }

    #[test]
    fn sensitive_diagnostic_separates_detail() {
        let d = SensitiveDiagnostic {
            event: TelemetryEvent {
                name: "open",
                code: 2,
            },
            detail: Sensitive::new("프로젝트 텍스트"),
        };
        assert_eq!(d.event.code, 2);
        assert_eq!(d.detail.reveal(), "프로젝트 텍스트");
    }
}
