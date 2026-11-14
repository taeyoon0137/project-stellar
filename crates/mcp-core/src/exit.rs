//! 종료 코드 (XIX.1.7).
//!
//! 값과 이름은 XXIV.1.2 표(`tools/stellar-cli/src/exit.rs`의 `ExitStatus`, sysexits 기반)를
//! 그대로 따릅니다. 브리지는 L1 도구에 의존하지 않으므로 필요한 항목만 같은 값으로 두고,
//! `exit_table_sync` 테스트가 두 표의 일치를 검사합니다.

/// 종료 상태(값, XXIV.1.2 이름).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    /// 프로세스 종료 코드.
    pub code: u8,
    /// XXIV.1.2 `exit_name`.
    pub name: &'static str,
}

/// 0: 성공.
pub const OK: Exit = Exit {
    code: 0,
    name: "ok",
};
/// 64 `EX_USAGE`: 사용법 오류(여러 앱 중 instance 미지정 포함).
pub const USAGE: Exit = Exit {
    code: 64,
    name: "usage",
};
/// 69 `EX_UNAVAILABLE`: 필요한 Stellar 구성 요소(실행 중 앱)를 쓸 수 없음.
pub const UNAVAILABLE: Exit = Exit {
    code: 69,
    name: "unavailable",
};
/// 70 `EX_SOFTWARE`: 내부 오류.
pub const SOFTWARE: Exit = Exit {
    code: 70,
    name: "software",
};
/// 74 `EX_IOERR`: 입출력 오류.
pub const IOERR: Exit = Exit {
    code: 74,
    name: "ioerr",
};
/// 75 `EX_TEMPFAIL`: 일시적 실패(응답 시간 초과).
pub const TEMPFAIL: Exit = Exit {
    code: 75,
    name: "tempfail",
};
/// 76 `EX_PROTOCOL`: 내부 protocol 오류.
pub const PROTOCOL: Exit = Exit {
    code: 76,
    name: "protocol",
};
/// 77 `EX_NOPERM`: 권한 없음(IPC 상대 인증 실패, 안전하지 않은 registry).
pub const NOPERM: Exit = Exit {
    code: 77,
    name: "noperm",
};
/// 78 `EX_CONFIG`: 설정 오류.
pub const CONFIG: Exit = Exit {
    code: 78,
    name: "config",
};

/// 브리지가 쓰는 전체 종료 상태(코드 오름차순).
pub const ALL: [Exit; 9] = [
    OK,
    USAGE,
    UNAVAILABLE,
    SOFTWARE,
    IOERR,
    TEMPFAIL,
    PROTOCOL,
    NOPERM,
    CONFIG,
];

impl Exit {
    /// [`std::process::ExitCode`]로 바꿉니다.
    pub fn exit_code(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.code)
    }
}
