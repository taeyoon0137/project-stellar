//! 종료 코드 (XIX.1.7, XIX.9.9).
//!
//! 값과 이름은 XXIV.1.2 표(`tools/stellar-cli/src/exit.rs`의 `ExitStatus`, sysexits 기반)를
//! 그대로 따릅니다. 브리지는 L1 도구에 의존하지 않으므로 같은 표를 여기 두고,
//! `tools/stellar-mcp/tests/exit_table_sync.rs`가 두 표의 일치를 검사합니다. tool·resource 결과 종류와의 대응은
//! `stellar_agent_protocol::outcome`(ADR-0085 결정 7)이고 [`for_kind`]가 그 대응으로 이 표의 항목을 고릅니다.

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
/// 1: 도메인 실패.
pub const FAILURE: Exit = Exit {
    code: 1,
    name: "failure",
};
/// 64 `EX_USAGE`: 사용법 오류(여러 앱 중 instance 미지정 포함).
pub const USAGE: Exit = Exit {
    code: 64,
    name: "usage",
};
/// 65 `EX_DATAERR`: 입력 데이터 형식 오류.
pub const DATAERR: Exit = Exit {
    code: 65,
    name: "dataerr",
};
/// 66 `EX_NOINPUT`: 입력 없음·읽을 수 없음.
pub const NOINPUT: Exit = Exit {
    code: 66,
    name: "noinput",
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
/// 73 `EX_CANTCREAT`: 출력 생성 불가.
pub const CANTCREAT: Exit = Exit {
    code: 73,
    name: "cantcreat",
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

/// 전체 종료 상태(코드 오름차순, XXIV.1.2 표와 같음).
pub const ALL: [Exit; 13] = [
    OK,
    FAILURE,
    USAGE,
    DATAERR,
    NOINPUT,
    UNAVAILABLE,
    SOFTWARE,
    CANTCREAT,
    IOERR,
    TEMPFAIL,
    PROTOCOL,
    NOPERM,
    CONFIG,
];

/// 결과 종류에 대응하는 종료 상태(ADR-0085 결정 7).
pub fn for_kind(kind: stellar_agent_protocol::outcome::ResultKind) -> Exit {
    let code = kind.exit().code;
    ALL.into_iter()
        .find(|exit| exit.code == code)
        .expect("결과 종류의 종료 코드는 XXIV.1.2 표에 있음")
}

impl Exit {
    /// [`std::process::ExitCode`]로 바꿉니다.
    pub fn exit_code(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_agent_protocol::outcome::ResultKind;

    #[test]
    fn every_result_kind_maps_into_this_table_with_the_same_name() {
        for kind in ResultKind::ALL {
            let exit = for_kind(kind);
            assert_eq!((exit.code, exit.name), (kind.exit().code, kind.exit().name));
        }
        let mut codes: Vec<u8> = ALL.iter().map(|e| e.code).collect();
        codes.dedup();
        assert_eq!(codes.len(), ALL.len(), "코드 오름차순·중복 없음");
        assert!(codes.windows(2).all(|w| w[0] < w[1]));
    }
}
