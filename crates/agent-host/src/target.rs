//! 구독 CLI 실행 대상 지정과 신원 확인 (XXIX.2.3, ADR-0005 §4.3 실행 대상 확인, ADR-0092 결정 3).
//!
//! 사용자가 확인한, 수정하지 않은 공식 CLI만 실행 대상입니다.
//!
//! 1. **사용자 확인 기록**([`ConfirmedTarget`]): 사용자가 고른 실행 파일의 정규 경로(symlink 해석 뒤),
//!    CLI 종류, 확인 시점 파일 지문(장치·inode·크기·수정 시각·mode). 앱이 OS 사용자 데이터에 둡니다(XXIX.3.5).
//! 2. **확인 시점과 같은 파일인지**: 실행 직전 경로를 다시 해석해 같은 정규 경로인지, 지문이 같은지 봅니다.
//!    사용자가 CLI를 업데이트해 지문이 바뀌면 다시 확인받아야 합니다.
//! 3. **파일 안전 조건**: 일반 파일, 실행 bit, group·other 쓰기 불가, 소유자가 root나 현재 사용자.
//! 4. **코드 서명**(Apple Code Signing Services): `SecStaticCodeCreateWithPath`로 파일을 열고
//!    `SecStaticCodeCheckValidity`를 `kSecCSStrictValidate | kSecCSCheckAllArchitectures |
//!    kSecCSCheckNestedCode | kSecCSNoNetworkAccess`로 실행합니다. 요구 사항은 launch profile의 공급자
//!    Team ID에 대한 `anchor apple generic and certificate leaf[subject.OU] = "<team>"`(TN3127)입니다.
//!    서명 검증이 실행 파일 내용을 서명된 code directory hash와 비교하므로 수정된 파일은 실패합니다.
//!    `kSecCSNoNetworkAccess`로 확인 중 네트워크를 쓰지 않습니다.
//!
//! 하나라도 확인할 수 없으면 실행하지 않습니다(fail closed). Stellar는 CLI 실행 파일을 쓰기로 열지
//! 않고, 설치·업데이트·수정·래핑하지 않습니다. 서명이 없는 배포 형태(예: 해석기가 실행하는 script)는
//! 신원을 확인할 수 없으므로 실행 대상이 아닙니다.

use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use stellar_local_ipc::code_identity::{CodeRule, TeamId};

use crate::profile::LaunchProfile;

/// 파일 지문(확인 시점과 실행 시점 비교용, 내용 hash 대신 서명 검증이 내용을 확인).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFingerprint {
    /// 장치 번호.
    pub dev: u64,
    /// inode.
    pub ino: u64,
    /// 크기.
    pub size: u64,
    /// 수정 시각(초).
    pub mtime: i64,
    /// 수정 시각(나노초 부분).
    pub mtime_nsec: i64,
    /// mode.
    pub mode: u32,
}

impl FileFingerprint {
    fn of(meta: &std::fs::Metadata) -> FileFingerprint {
        FileFingerprint {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
            mode: meta.mode(),
        }
    }
}

/// 사용자가 확인한 실행 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedTarget {
    /// 정규 경로.
    pub path: PathBuf,
    /// CLI 종류 id(launch profile `cli`).
    pub cli: String,
    /// 확인 시점 지문.
    pub fingerprint: FileFingerprint,
}

/// 신원을 확인한 실행 대상. [`verify_target`]만 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTarget {
    path: PathBuf,
    fingerprint: FileFingerprint,
}

impl VerifiedTarget {
    /// 실행할 정규 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 실행 직전에 파일이 확인 뒤 바뀌지 않았는지 다시 봅니다.
    pub fn still_unchanged(&self) -> bool {
        std::fs::metadata(&self.path)
            .map(|m| FileFingerprint::of(&m) == self.fingerprint)
            .unwrap_or(false)
    }
}

/// 실행 대상 확인 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    /// 경로를 해석하거나 읽을 수 없습니다.
    Unreadable,
    /// 일반 파일이 아니거나 실행 bit가 없습니다.
    NotExecutableFile,
    /// group·other가 쓸 수 있거나 소유자가 root·현재 사용자가 아닙니다.
    UnsafePermissions,
    /// 확인 뒤 경로나 파일이 바뀌었습니다(다시 확인 필요).
    ChangedSinceConfirmation,
    /// profile과 CLI 종류가 다릅니다.
    WrongCli,
    /// profile에 서명 Team ID가 없어 신원을 확인할 수 없습니다.
    NoIdentityRule,
    /// 코드 서명 확인 실패.
    Signature(SignatureError),
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            TargetError::Unreadable => "실행 파일을 읽을 수 없습니다",
            TargetError::NotExecutableFile => "실행 가능한 일반 파일이 아닙니다",
            TargetError::UnsafePermissions => "실행 파일 권한이 안전하지 않습니다",
            TargetError::ChangedSinceConfirmation => "확인 뒤 실행 파일이 바뀌었습니다",
            TargetError::WrongCli => "launch profile과 CLI 종류가 다릅니다",
            TargetError::NoIdentityRule => "launch profile에 서명 신원 규칙이 없습니다",
            TargetError::Signature(_) => "실행 파일 서명을 확인하지 못했습니다",
        };
        f.write_str(text)
    }
}

impl std::error::Error for TargetError {}

/// 코드 서명 확인 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureError {
    /// 서명이 없습니다(`errSecCSUnsigned`).
    Unsigned,
    /// 요구 사항(공급자 Team ID)을 만족하지 않습니다(`errSecCSReqFailed`).
    RequirementFailed,
    /// 서명이 유효하지 않거나 확인할 수 없습니다(OSStatus).
    Invalid {
        /// Security framework 결과 코드.
        status: i32,
    },
}

/// 정적 코드 서명 확인 수단. 앱은 [`AppleStaticCodeVerifier`]를 씁니다. 테스트 대역 세션은 대역 실행
/// 파일이 공급자 서명을 가질 수 없으므로 테스트가 자기 구현을 넣습니다.
pub trait CodeVerifier {
    /// `path`의 코드가 `rule`을 만족하는지 정적으로 확인합니다. 네트워크를 쓰지 않습니다.
    fn verify(&self, path: &Path, rule: &CodeRule) -> Result<(), SignatureError>;
}

/// 사용자가 고른 경로를 실행 대상 확인 기록으로 만듭니다(사용자 확인 동작에서 호출).
pub fn confirm_target(path: &Path, cli: &str) -> Result<ConfirmedTarget, TargetError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| TargetError::Unreadable)?;
    let meta = std::fs::metadata(&canonical).map_err(|_| TargetError::Unreadable)?;
    check_file(&meta)?;
    Ok(ConfirmedTarget {
        path: canonical,
        cli: cli.to_string(),
        fingerprint: FileFingerprint::of(&meta),
    })
}

fn check_file(meta: &std::fs::Metadata) -> Result<(), TargetError> {
    if !meta.is_file() || meta.mode() & 0o111 == 0 {
        return Err(TargetError::NotExecutableFile);
    }
    let uid = stellar_local_ipc::os::current_uid();
    if meta.mode() & 0o022 != 0 || (meta.uid() != 0 && meta.uid() != uid) {
        return Err(TargetError::UnsafePermissions);
    }
    Ok(())
}

/// 확인 기록, launch profile, 서명 확인 수단으로 실행 대상 신원을 확인합니다. CLI를 실행하지 않습니다.
pub fn verify_target(
    target: &ConfirmedTarget,
    profile: &LaunchProfile,
    verifier: &dyn CodeVerifier,
) -> Result<VerifiedTarget, TargetError> {
    if target.cli != profile.cli {
        return Err(TargetError::WrongCli);
    }
    let canonical = std::fs::canonicalize(&target.path).map_err(|_| TargetError::Unreadable)?;
    if canonical != target.path {
        return Err(TargetError::ChangedSinceConfirmation);
    }
    let meta = std::fs::metadata(&canonical).map_err(|_| TargetError::Unreadable)?;
    check_file(&meta)?;
    if FileFingerprint::of(&meta) != target.fingerprint {
        return Err(TargetError::ChangedSinceConfirmation);
    }
    let team = profile
        .team_id
        .as_deref()
        .and_then(TeamId::parse)
        .ok_or(TargetError::NoIdentityRule)?;
    verifier
        .verify(&canonical, &CodeRule::SameTeam(team))
        .map_err(TargetError::Signature)?;
    // 서명 확인 동안 파일이 바뀌지 않았는지 다시 봅니다.
    let after = std::fs::metadata(&canonical).map_err(|_| TargetError::Unreadable)?;
    if FileFingerprint::of(&after) != target.fingerprint {
        return Err(TargetError::ChangedSinceConfirmation);
    }
    Ok(VerifiedTarget {
        path: canonical,
        fingerprint: target.fingerprint,
    })
}

/// Apple Code Signing Services 정적 코드 확인.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppleStaticCodeVerifier;

#[allow(non_upper_case_globals, non_snake_case)]
mod sys {
    use std::ffi::c_void;

    pub type CFTypeRef = *const c_void;
    pub type CFIndex = isize;
    pub type OSStatus = i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFURLCreateFromFileSystemRepresentation(
            alloc: CFTypeRef,
            buffer: *const u8,
            length: CFIndex,
            is_directory: u8,
        ) -> CFTypeRef;
        pub fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            count: CFIndex,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        pub fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        pub fn SecStaticCodeCreateWithPath(
            path: CFTypeRef,
            flags: u32,
            code: *mut CFTypeRef,
        ) -> OSStatus;
        pub fn SecStaticCodeCheckValidity(
            code: CFTypeRef,
            flags: u32,
            requirement: CFTypeRef,
        ) -> OSStatus;
        pub fn SecRequirementCreateWithString(
            text: CFTypeRef,
            flags: u32,
            requirement: *mut CFTypeRef,
        ) -> OSStatus;
    }

    /// `kCFStringEncodingUTF8`.
    pub const UTF8: u32 = 0x0800_0100;
    /// `kSecCSCheckAllArchitectures`.
    pub const CHECK_ALL_ARCHITECTURES: u32 = 1 << 0;
    /// `kSecCSCheckNestedCode`.
    pub const CHECK_NESTED_CODE: u32 = 1 << 3;
    /// `kSecCSStrictValidate`.
    pub const STRICT_VALIDATE: u32 = 1 << 4;
    /// `kSecCSNoNetworkAccess`.
    pub const NO_NETWORK_ACCESS: u32 = 1 << 29;
}

/// `errSecCSUnsigned`.
const ERR_UNSIGNED: i32 = -67062;
/// `errSecCSReqFailed`.
const ERR_REQ_FAILED: i32 = -67050;

/// 정적 확인 flag.
pub const STATIC_VALIDITY_FLAGS: u32 = sys::STRICT_VALIDATE
    | sys::CHECK_ALL_ARCHITECTURES
    | sys::CHECK_NESTED_CODE
    | sys::NO_NETWORK_ACCESS;

struct CfOwned(sys::CFTypeRef);

impl CfOwned {
    fn new(raw: sys::CFTypeRef) -> Option<CfOwned> {
        (!raw.is_null()).then_some(CfOwned(raw))
    }
}

impl Drop for CfOwned {
    fn drop(&mut self) {
        // SAFETY: 생성 시 null이 아닌, 이 값이 소유한(Create 규칙) CF 객체입니다.
        unsafe { sys::CFRelease(self.0) }
    }
}

impl CodeVerifier for AppleStaticCodeVerifier {
    fn verify(&self, path: &Path, rule: &CodeRule) -> Result<(), SignatureError> {
        use std::os::unix::ffi::OsStrExt;
        let invalid = SignatureError::Invalid { status: 0 };
        let bytes = path.as_os_str().as_bytes();
        let len = sys::CFIndex::try_from(bytes.len()).map_err(|_| invalid)?;
        // SAFETY: bytes는 len byte 읽기 가능한 경로이며 결과는 Create 규칙으로 소유합니다.
        let url = CfOwned::new(unsafe {
            sys::CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), len, 0)
        })
        .ok_or(invalid)?;
        let text = rule.requirement_text();
        let text_len = sys::CFIndex::try_from(text.len()).map_err(|_| invalid)?;
        // SAFETY: text는 text_len byte UTF-8이며 결과는 Create 규칙으로 소유합니다.
        let cf_text = CfOwned::new(unsafe {
            sys::CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), text_len, sys::UTF8, 0)
        })
        .ok_or(invalid)?;
        let mut requirement: sys::CFTypeRef = std::ptr::null();
        // SAFETY: cf_text는 유효한 CFString, requirement는 쓰기 가능한 지역 포인터입니다.
        let status = unsafe { sys::SecRequirementCreateWithString(cf_text.0, 0, &mut requirement) };
        if status != 0 {
            return Err(SignatureError::Invalid { status });
        }
        let requirement = CfOwned::new(requirement).ok_or(invalid)?;
        let mut code: sys::CFTypeRef = std::ptr::null();
        // SAFETY: url은 유효한 CFURL, code는 쓰기 가능한 지역 포인터입니다.
        let status = unsafe { sys::SecStaticCodeCreateWithPath(url.0, 0, &mut code) };
        if status != 0 {
            return Err(map_status(status));
        }
        let code = CfOwned::new(code).ok_or(invalid)?;
        // SAFETY: code는 유효한 SecStaticCode, requirement는 유효한 SecRequirement입니다.
        let status = unsafe {
            sys::SecStaticCodeCheckValidity(code.0, STATIC_VALIDITY_FLAGS, requirement.0)
        };
        match status {
            0 => Ok(()),
            status => Err(map_status(status)),
        }
    }
}

fn map_status(status: i32) -> SignatureError {
    match status {
        ERR_UNSIGNED => SignatureError::Unsigned,
        ERR_REQ_FAILED => SignatureError::RequirementFailed,
        status => SignatureError::Invalid { status },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> CodeRule {
        CodeRule::SameTeam(TeamId::parse("ABCDE12345").unwrap())
    }

    #[test]
    fn apple_platform_binary_fails_foreign_team_rule() {
        // Apple 서명 /bin/ls는 서명은 유효하지만 공급자 Team ID 요구 사항을 만족하지 않습니다.
        let err = AppleStaticCodeVerifier
            .verify(Path::new("/bin/ls"), &rule())
            .unwrap_err();
        assert_eq!(err, SignatureError::RequirementFailed);
    }

    #[test]
    fn unsigned_file_fails_closed() {
        let dir = std::env::temp_dir().join(format!("stellar-agent-sig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("cli.sh");
        std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        let err = AppleStaticCodeVerifier
            .verify(&script, &rule())
            .unwrap_err();
        assert!(matches!(
            err,
            SignatureError::Unsigned | SignatureError::Invalid { .. }
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_path_fails_closed() {
        assert!(AppleStaticCodeVerifier
            .verify(Path::new("/nonexistent/stellar/cli"), &rule())
            .is_err());
    }
}
