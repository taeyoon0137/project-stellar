//! 구조화 오류 타입.
//!
//! 사용자 메시지와 개발자 진단 context를 분리하고, 안정적인 숫자 코드를 제공합니다.

/// 안정적인 숫자 코드를 가진 오류 분류.
///
/// 숫자 값은 FFI·로그·직렬화에서 안정적으로 유지되어야 합니다(golden test로 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ErrorCode {
    /// 내부 오류.
    Internal = 1,
    /// 잘못된 입력.
    InvalidInput = 2,
    /// 대상을 찾을 수 없음.
    NotFound = 3,
    /// 이미 존재함.
    AlreadyExists = 4,
    /// 권한 없음.
    PermissionDenied = 5,
    /// FFI 경계에서 잡은 panic.
    InternalPanic = 6,
}

impl ErrorCode {
    /// 정의된 모든 오류 코드.
    pub const ALL: [ErrorCode; 6] = [
        ErrorCode::Internal,
        ErrorCode::InvalidInput,
        ErrorCode::NotFound,
        ErrorCode::AlreadyExists,
        ErrorCode::PermissionDenied,
        ErrorCode::InternalPanic,
    ];

    /// 안정적인 숫자 코드.
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// 숫자 코드를 분류로 변환합니다. 알 수 없는 값은 `None`.
    pub fn from_u32(value: u32) -> Option<ErrorCode> {
        ErrorCode::ALL
            .into_iter()
            .find(|code| code.as_u32() == value)
    }
}

/// 구조화 오류. 사용자 메시지와 개발자 context를 분리합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StellarError {
    /// 오류 분류.
    pub code: ErrorCode,
    /// 사용자에게 보여줄 메시지.
    pub message: String,
    /// 개발자용 진단 context(사용자에게 노출하지 않습니다).
    pub context: String,
}

impl StellarError {
    /// 코드·사용자 메시지·개발자 context로 오류를 생성합니다.
    pub fn new(code: ErrorCode, message: impl Into<String>, context: impl Into<String>) -> Self {
        StellarError {
            code,
            message: message.into(),
            context: context.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_codes_are_stable() {
        // golden: 숫자 값은 바뀌면 안 됩니다.
        assert_eq!(ErrorCode::Internal.as_u32(), 1);
        assert_eq!(ErrorCode::InvalidInput.as_u32(), 2);
        assert_eq!(ErrorCode::NotFound.as_u32(), 3);
        assert_eq!(ErrorCode::AlreadyExists.as_u32(), 4);
        assert_eq!(ErrorCode::PermissionDenied.as_u32(), 5);
        assert_eq!(ErrorCode::InternalPanic.as_u32(), 6);
    }

    #[test]
    fn code_round_trips() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_u32(code.as_u32()), Some(code));
        }
        assert_eq!(ErrorCode::from_u32(0), None);
        assert_eq!(ErrorCode::from_u32(999), None);
    }

    #[test]
    fn separates_user_message_and_context() {
        let err = StellarError::new(
            ErrorCode::NotFound,
            "찾을 수 없습니다",
            "id=42 lookup failed",
        );
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.message, "찾을 수 없습니다");
        assert_eq!(err.context, "id=42 lookup failed");
    }
}
