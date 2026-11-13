//! 연결 상대의 코드 서명 신원 검사 (XIX.1.6).
//!
//! Apple Code Signing Services로 상대 프로세스를 audit token으로 찾고, 코드 요구 사항(code
//! requirement)을 만족하는지 동적으로 검증합니다.
//!
//! - `SecCodeCopyGuestWithAttributes`(host `NULL` = 커널 root of trust)에 `kSecGuestAttributeAudit`
//!   (audit token CFData)를 주어 상대 SecCode를 얻습니다. pid가 아니라 audit token을 쓰므로 pid
//!   재사용 경쟁이 없습니다.
//! - `SecRequirementCreateWithString`으로 요구 사항을 만들고 `SecCodeCheckValidity`로 검증합니다.
//!   이 함수는 서명을 확인하고, 실행 중 코드의 동적 유효성과 host 유효성을 확인한 뒤 요구 사항을
//!   평가합니다.
//! - 자기 Team ID는 `SecCodeCopySelf` + `SecCodeCopySigningInformation`(`kSecCSSigningInformation`)의
//!   `kSecCodeInfoTeamIdentifier`로 얻습니다. ad-hoc·linker 서명(개발 빌드)에는 없습니다.
//! - Team ID 요구 사항은 TN3127 "Inside Code Signing: Requirements"의 `anchor apple generic`과
//!   `certificate leaf[subject.OU]`(Developer ID 인증서의 Team ID 위치) 항을 씁니다.
//!
//! CoreFoundation·Security framework 함수는 최소 `extern "C"`로 선언하고, 소유한 CF 객체는
//! [`CfOwned`]가 해제합니다. 모든 판단은 실패하면 닫힙니다(fail closed).

use std::fmt;

use crate::peer::AuditToken;

#[allow(non_upper_case_globals, non_snake_case)]
mod sys {
    use std::ffi::c_void;

    pub type CFTypeRef = *const c_void;
    pub type CFIndex = isize;
    pub type OSStatus = i32;

    /// `CFDictionaryKeyCallBacks`·`CFDictionaryValueCallBacks`(주소만 사용).
    #[repr(C)]
    pub struct CFDictionaryCallBacks {
        _version: CFIndex,
        _fns: [*const c_void; 5],
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFTypeDictionaryKeyCallBacks: CFDictionaryCallBacks;
        pub static kCFTypeDictionaryValueCallBacks: CFDictionaryCallBacks;
        pub fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, length: CFIndex) -> CFTypeRef;
        pub fn CFDictionaryCreate(
            alloc: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: CFIndex,
            key_callbacks: *const CFDictionaryCallBacks,
            value_callbacks: *const CFDictionaryCallBacks,
        ) -> CFTypeRef;
        pub fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        pub fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            count: CFIndex,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        pub fn CFStringGetTypeID() -> usize;
        pub fn CFGetTypeID(cf: CFTypeRef) -> usize;
        pub fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut u8,
            size: CFIndex,
            encoding: u32,
        ) -> u8;
        pub fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        pub static kSecGuestAttributeAudit: CFTypeRef;
        pub static kSecCodeInfoTeamIdentifier: CFTypeRef;
        pub fn SecCodeCopySelf(flags: u32, code: *mut CFTypeRef) -> OSStatus;
        pub fn SecCodeCopyGuestWithAttributes(
            host: CFTypeRef,
            attributes: CFTypeRef,
            flags: u32,
            guest: *mut CFTypeRef,
        ) -> OSStatus;
        pub fn SecCodeCheckValidity(
            code: CFTypeRef,
            flags: u32,
            requirement: CFTypeRef,
        ) -> OSStatus;
        pub fn SecRequirementCreateWithString(
            text: CFTypeRef,
            flags: u32,
            requirement: *mut CFTypeRef,
        ) -> OSStatus;
        pub fn SecCodeCopySigningInformation(
            code: CFTypeRef,
            flags: u32,
            information: *mut CFTypeRef,
        ) -> OSStatus;
    }

    /// `kCFStringEncodingUTF8`.
    pub const UTF8: u32 = 0x0800_0100;
    /// `kSecCSDefaultFlags`.
    pub const DEFAULT_FLAGS: u32 = 0;
    /// `kSecCSSigningInformation`.
    pub const SIGNING_INFORMATION: u32 = 1 << 1;
}

/// `errSecCSReqFailed`: 코드가 요구 사항을 만족하지 않습니다(`CSCommon.h`).
pub const ERR_REQ_FAILED: i32 = -67050;
/// `errSecCSUnsigned`: 서명이 없습니다.
pub const ERR_UNSIGNED: i32 = -67062;
/// `errSecCSNoSuchCode`: 해당 attribute의 guest가 없습니다(상대 프로세스 종료 등).
pub const ERR_NO_SUCH_CODE: i32 = -67065;

/// Team ID 길이(Apple Developer Team ID는 영문 대문자·숫자 10자).
pub const TEAM_ID_LEN: usize = 10;

/// 소유한 CF 객체. drop할 때 `CFRelease`합니다.
struct CfOwned(sys::CFTypeRef);

impl CfOwned {
    /// null이 아니면 소유권을 가져옵니다.
    fn new(raw: sys::CFTypeRef) -> Option<CfOwned> {
        (!raw.is_null()).then_some(CfOwned(raw))
    }
}

impl Drop for CfOwned {
    fn drop(&mut self) {
        // SAFETY: 생성 시 null이 아닌, 이 값이 소유한(Create·Copy 규칙) CF 객체입니다.
        unsafe { sys::CFRelease(self.0) }
    }
}

/// Apple Developer Team ID(영문 대문자·숫자 10자).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamId(String);

impl TeamId {
    /// 형식에 맞으면 Team ID로 읽습니다. 요구 사항 문자열에 넣으므로 형식을 엄격히 검사합니다.
    pub fn parse(text: &str) -> Option<TeamId> {
        let valid = text.len() == TEAM_ID_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        valid.then(|| TeamId(text.to_string()))
    }

    /// 문자열 표기.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 상대 코드가 만족해야 하는 규칙.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeRule {
    /// 자신과 같은 Team ID로 서명된 코드(Developer ID·Mac App Store 서명 빌드).
    SameTeam(TeamId),
    /// 자기 Team ID가 없는 개발 빌드 규칙: 유효한 서명(ad-hoc 포함)이 있고 Apple platform binary가
    /// 아닌 코드. 시스템 도구로 만든 가짜 endpoint·bridge를 거부하지만, 같은 사용자가 직접 서명한
    /// 실행 파일은 막지 못합니다(신뢰 경계는 OS 사용자).
    DevelopmentSigned,
    /// 임의의 code requirement 문자열(특정 identifier 고정 등).
    Custom(String),
}

impl CodeRule {
    /// code requirement language 표기.
    pub fn requirement_text(&self) -> String {
        match self {
            CodeRule::SameTeam(team) => format!(
                "anchor apple generic and certificate leaf[subject.OU] = \"{}\"",
                team.as_str()
            ),
            CodeRule::DevelopmentSigned => "!anchor apple".to_string(),
            CodeRule::Custom(text) => text.clone(),
        }
    }

    /// 자기 서명에서 규칙을 정합니다: Team ID가 있으면 [`CodeRule::SameTeam`], 없으면
    /// [`CodeRule::DevelopmentSigned`].
    pub fn for_current_process() -> Result<CodeRule, CodeIdentityError> {
        Ok(match own_team_id()? {
            Some(team) => CodeRule::SameTeam(team),
            None => CodeRule::DevelopmentSigned,
        })
    }
}

/// 코드 신원 검사 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeIdentityError {
    /// 상대 코드가 규칙을 만족하지 않습니다(`errSecCSReqFailed`).
    RequirementFailed,
    /// 상대 코드에 서명이 없습니다(`errSecCSUnsigned`).
    Unsigned,
    /// 상대 코드를 찾거나 검증할 수 없습니다(OSStatus).
    Unverifiable {
        /// Security framework 결과 코드.
        status: i32,
    },
    /// 요구 사항 문자열을 만들 수 없습니다(OSStatus, 0이면 CF 객체 생성 실패).
    InvalidRequirement {
        /// Security framework 결과 코드.
        status: i32,
    },
    /// 자기 Team ID가 형식에 맞지 않습니다.
    MalformedTeamId,
}

impl fmt::Display for CodeIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeIdentityError::RequirementFailed => {
                write!(f, "IPC 상대 코드가 서명 요구 사항을 만족하지 않습니다")
            }
            CodeIdentityError::Unsigned => write!(f, "IPC 상대 코드에 서명이 없습니다"),
            CodeIdentityError::Unverifiable { status } => {
                write!(
                    f,
                    "IPC 상대 코드 서명을 확인할 수 없습니다(OSStatus {status})"
                )
            }
            CodeIdentityError::InvalidRequirement { status } => {
                write!(
                    f,
                    "코드 서명 요구 사항을 만들 수 없습니다(OSStatus {status})"
                )
            }
            CodeIdentityError::MalformedTeamId => {
                write!(f, "자기 코드 서명의 Team ID 형식이 잘못되었습니다")
            }
        }
    }
}

impl std::error::Error for CodeIdentityError {}

/// UTF-8 문자열로 CFString을 만듭니다.
fn cf_string(text: &str) -> Option<CfOwned> {
    let len = sys::CFIndex::try_from(text.len()).ok()?;
    // SAFETY: bytes는 len byte 읽기 가능한 UTF-8이며 결과는 Create 규칙으로 소유합니다.
    CfOwned::new(unsafe {
        sys::CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), len, sys::UTF8, 0)
    })
}

/// 요구 사항 객체를 만듭니다.
fn requirement(rule: &CodeRule) -> Result<CfOwned, CodeIdentityError> {
    let text = cf_string(&rule.requirement_text())
        .ok_or(CodeIdentityError::InvalidRequirement { status: 0 })?;
    let mut out: sys::CFTypeRef = std::ptr::null();
    // SAFETY: text는 유효한 CFString이고 out은 쓰기 가능한 지역 포인터입니다.
    let status =
        unsafe { sys::SecRequirementCreateWithString(text.0, sys::DEFAULT_FLAGS, &mut out) };
    if status != 0 {
        return Err(CodeIdentityError::InvalidRequirement { status });
    }
    CfOwned::new(out).ok_or(CodeIdentityError::InvalidRequirement { status: 0 })
}

/// 요구 사항 문자열이 컴파일되는지 확인합니다.
pub fn validate_rule(rule: &CodeRule) -> Result<(), CodeIdentityError> {
    requirement(rule).map(drop)
}

/// audit token이 가리키는 프로세스의 SecCode를 얻습니다.
fn guest_code(token: &AuditToken) -> Result<CfOwned, CodeIdentityError> {
    let bytes = token.to_bytes();
    let len = sys::CFIndex::try_from(bytes.len())
        .map_err(|_| CodeIdentityError::Unverifiable { status: 0 })?;
    // SAFETY: bytes는 len byte 읽기 가능한 지역 배열이며 결과는 Create 규칙으로 소유합니다.
    let data = CfOwned::new(unsafe { sys::CFDataCreate(std::ptr::null(), bytes.as_ptr(), len) })
        .ok_or(CodeIdentityError::Unverifiable { status: 0 })?;
    // SAFETY: kSecGuestAttributeAudit은 Security framework가 정의한 상수 CFString입니다.
    let key = unsafe { sys::kSecGuestAttributeAudit };
    let keys = [key];
    let values = [data.0];
    // SAFETY: keys·values는 길이 1인 유효한 CF 객체 배열이고, callback은 framework 상수입니다.
    let attributes = CfOwned::new(unsafe {
        sys::CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &sys::kCFTypeDictionaryKeyCallBacks,
            &sys::kCFTypeDictionaryValueCallBacks,
        )
    })
    .ok_or(CodeIdentityError::Unverifiable { status: 0 })?;
    let mut guest: sys::CFTypeRef = std::ptr::null();
    // SAFETY: host null은 커널 root of trust, attributes는 유효한 CFDictionary, guest는 쓰기 가능한
    // 지역 포인터입니다.
    let status = unsafe {
        sys::SecCodeCopyGuestWithAttributes(
            std::ptr::null(),
            attributes.0,
            sys::DEFAULT_FLAGS,
            &mut guest,
        )
    };
    if status != 0 {
        return Err(CodeIdentityError::Unverifiable { status });
    }
    CfOwned::new(guest).ok_or(CodeIdentityError::Unverifiable { status: 0 })
}

/// audit token이 가리키는 상대 코드가 `rule`을 만족하는지 동적으로 검증합니다.
pub fn check_peer_code(token: &AuditToken, rule: &CodeRule) -> Result<(), CodeIdentityError> {
    let requirement = requirement(rule)?;
    let code = guest_code(token)?;
    // SAFETY: code는 유효한 SecCode, requirement는 유효한 SecRequirement입니다.
    let status = unsafe { sys::SecCodeCheckValidity(code.0, sys::DEFAULT_FLAGS, requirement.0) };
    match status {
        0 => Ok(()),
        ERR_REQ_FAILED => Err(CodeIdentityError::RequirementFailed),
        ERR_UNSIGNED => Err(CodeIdentityError::Unsigned),
        status => Err(CodeIdentityError::Unverifiable { status }),
    }
}

/// 자기 코드 서명의 Team ID. ad-hoc·linker 서명이면 `None`입니다.
pub fn own_team_id() -> Result<Option<TeamId>, CodeIdentityError> {
    let mut code: sys::CFTypeRef = std::ptr::null();
    // SAFETY: code는 쓰기 가능한 지역 포인터입니다.
    let status = unsafe { sys::SecCodeCopySelf(sys::DEFAULT_FLAGS, &mut code) };
    if status != 0 {
        return Err(CodeIdentityError::Unverifiable { status });
    }
    let code = CfOwned::new(code).ok_or(CodeIdentityError::Unverifiable { status: 0 })?;
    let mut info: sys::CFTypeRef = std::ptr::null();
    // SAFETY: code는 유효한 SecCode(SecStaticCode 자리에 허용), info는 쓰기 가능한 지역 포인터입니다.
    let status =
        unsafe { sys::SecCodeCopySigningInformation(code.0, sys::SIGNING_INFORMATION, &mut info) };
    if status != 0 {
        return Err(CodeIdentityError::Unverifiable { status });
    }
    let info = CfOwned::new(info).ok_or(CodeIdentityError::Unverifiable { status: 0 })?;
    // SAFETY: info는 유효한 CFDictionary이고 key는 framework 상수입니다. 결과는 빌린 값(Get 규칙)이며
    // info보다 오래 쓰지 않습니다.
    let value = unsafe { sys::CFDictionaryGetValue(info.0, sys::kSecCodeInfoTeamIdentifier) };
    if value.is_null() {
        return Ok(None);
    }
    // SAFETY: value는 null이 아닌 CF 객체입니다.
    if unsafe { sys::CFGetTypeID(value) != sys::CFStringGetTypeID() } {
        return Err(CodeIdentityError::MalformedTeamId);
    }
    let mut buffer = [0u8; 64];
    let size =
        sys::CFIndex::try_from(buffer.len()).map_err(|_| CodeIdentityError::MalformedTeamId)?;
    // SAFETY: value는 CFString이고 buffer는 size byte 쓰기 가능한 지역 배열입니다.
    let ok = unsafe { sys::CFStringGetCString(value, buffer.as_mut_ptr(), size, sys::UTF8) };
    if ok == 0 {
        return Err(CodeIdentityError::MalformedTeamId);
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    let text =
        std::str::from_utf8(&buffer[..end]).map_err(|_| CodeIdentityError::MalformedTeamId)?;
    TeamId::parse(text)
        .map(Some)
        .ok_or(CodeIdentityError::MalformedTeamId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::peer_audit_token;
    use std::os::unix::net::UnixStream;

    fn own_token() -> AuditToken {
        let (a, _b) = UnixStream::pair().unwrap();
        peer_audit_token(&a).unwrap()
    }

    #[test]
    fn team_ids_are_strict() {
        assert!(TeamId::parse("ABCDE12345").is_some());
        for bad in [
            "abcde12345",
            "ABCDE1234",
            "ABCDE123456",
            "ABCDE 1234",
            "ABCDE\"1234",
            "",
        ] {
            assert!(TeamId::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn rule_texts_compile() {
        for rule in [
            CodeRule::SameTeam(TeamId::parse("ABCDE12345").unwrap()),
            CodeRule::DevelopmentSigned,
            CodeRule::Custom("anchor apple".into()),
        ] {
            assert_eq!(validate_rule(&rule), Ok(()), "{rule:?}");
        }
        assert!(matches!(
            validate_rule(&CodeRule::Custom("not a requirement ((".into())),
            Err(CodeIdentityError::InvalidRequirement { .. })
        ));
    }

    #[test]
    fn test_binary_is_development_signed_without_team() {
        // cargo가 만든 테스트 실행 파일은 ad-hoc(linker) 서명이라 Team ID가 없습니다.
        assert_eq!(own_team_id(), Ok(None));
        assert_eq!(
            CodeRule::for_current_process(),
            Ok(CodeRule::DevelopmentSigned)
        );
    }

    #[test]
    fn own_process_satisfies_development_rule_and_fails_stricter_rules() {
        let token = own_token();
        assert_eq!(
            check_peer_code(&token, &CodeRule::DevelopmentSigned),
            Ok(())
        );
        // 실제 Security framework 판정: ad-hoc 코드는 Apple 서명·Team ID 규칙을 만족하지 않습니다.
        assert_eq!(
            check_peer_code(&token, &CodeRule::Custom("anchor apple".into())),
            Err(CodeIdentityError::RequirementFailed)
        );
        assert_eq!(
            check_peer_code(
                &token,
                &CodeRule::SameTeam(TeamId::parse("ABCDE12345").unwrap())
            ),
            Err(CodeIdentityError::RequirementFailed)
        );
    }
}
