//! 종료 코드 (XXIV.1.2).
//!
//! 0은 성공, 1은 도메인 실패(명령은 정상 실행됐지만 결과가 부정적 — 예: 검증 진단 존재),
//! 64 이상은 BSD `sysexits.h` 값을 그대로 사용합니다. 값은 외부 스크립트·CI가 의존하는 공개
//! 계약이므로 바꾸거나 재사용하지 않습니다.

use std::io;

/// 프로세스 종료 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitStatus {
    /// 0: 성공.
    Ok,
    /// 1: 도메인 실패(검증 진단 등).
    Failure,
    /// 64 `EX_USAGE`: 명령줄 사용법 오류.
    Usage,
    /// 65 `EX_DATAERR`: 입력 데이터 형식 오류(손상된 매니페스트 등).
    DataErr,
    /// 66 `EX_NOINPUT`: 입력이 없거나 읽을 수 없음.
    NoInput,
    /// 69 `EX_UNAVAILABLE`: 필요한 서비스·기능을 사용할 수 없음.
    Unavailable,
    /// 70 `EX_SOFTWARE`: 내부 소프트웨어 오류.
    Software,
    /// 73 `EX_CANTCREAT`: 출력 파일을 만들 수 없음.
    CantCreate,
    /// 74 `EX_IOERR`: 입출력 오류.
    IoErr,
    /// 75 `EX_TEMPFAIL`: 일시적 실패(재시도 가능).
    TempFail,
    /// 76 `EX_PROTOCOL`: 원격·프로토콜 오류.
    Protocol,
    /// 77 `EX_NOPERM`: 권한 부족.
    NoPerm,
    /// 78 `EX_CONFIG`: 설정 오류.
    Config,
}

impl ExitStatus {
    /// 정의된 전체 상태(코드 오름차순).
    pub const ALL: [ExitStatus; 13] = [
        ExitStatus::Ok,
        ExitStatus::Failure,
        ExitStatus::Usage,
        ExitStatus::DataErr,
        ExitStatus::NoInput,
        ExitStatus::Unavailable,
        ExitStatus::Software,
        ExitStatus::CantCreate,
        ExitStatus::IoErr,
        ExitStatus::TempFail,
        ExitStatus::Protocol,
        ExitStatus::NoPerm,
        ExitStatus::Config,
    ];

    /// 프로세스 종료 코드.
    pub fn code(self) -> u8 {
        match self {
            ExitStatus::Ok => 0,
            ExitStatus::Failure => 1,
            ExitStatus::Usage => 64,
            ExitStatus::DataErr => 65,
            ExitStatus::NoInput => 66,
            ExitStatus::Unavailable => 69,
            ExitStatus::Software => 70,
            ExitStatus::CantCreate => 73,
            ExitStatus::IoErr => 74,
            ExitStatus::TempFail => 75,
            ExitStatus::Protocol => 76,
            ExitStatus::NoPerm => 77,
            ExitStatus::Config => 78,
        }
    }

    /// JSON `exit_name`에 쓰는 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            ExitStatus::Ok => "ok",
            ExitStatus::Failure => "failure",
            ExitStatus::Usage => "usage",
            ExitStatus::DataErr => "dataerr",
            ExitStatus::NoInput => "noinput",
            ExitStatus::Unavailable => "unavailable",
            ExitStatus::Software => "software",
            ExitStatus::CantCreate => "cantcreat",
            ExitStatus::IoErr => "ioerr",
            ExitStatus::TempFail => "tempfail",
            ExitStatus::Protocol => "protocol",
            ExitStatus::NoPerm => "noperm",
            ExitStatus::Config => "config",
        }
    }

    /// 도움말에 쓰는 한 줄 설명.
    pub fn summary(self) -> &'static str {
        match self {
            ExitStatus::Ok => "성공",
            ExitStatus::Failure => "도메인 실패(검증 진단 등)",
            ExitStatus::Usage => "사용법 오류",
            ExitStatus::DataErr => "입력 데이터 형식 오류",
            ExitStatus::NoInput => "입력 없음·읽을 수 없음",
            ExitStatus::Unavailable => "필요한 기능 사용 불가",
            ExitStatus::Software => "내부 오류",
            ExitStatus::CantCreate => "출력 생성 불가",
            ExitStatus::IoErr => "입출력 오류",
            ExitStatus::TempFail => "일시적 실패(재시도 가능)",
            ExitStatus::Protocol => "프로토콜 오류",
            ExitStatus::NoPerm => "권한 부족",
            ExitStatus::Config => "설정 오류",
        }
    }

    /// io 오류 종류를 종료 상태로 분류합니다.
    pub fn from_io_kind(kind: io::ErrorKind) -> ExitStatus {
        match kind {
            io::ErrorKind::NotFound => ExitStatus::NoInput,
            io::ErrorKind::PermissionDenied => ExitStatus::NoPerm,
            io::ErrorKind::AlreadyExists => ExitStatus::CantCreate,
            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                ExitStatus::TempFail
            }
            io::ErrorKind::InvalidData => ExitStatus::DataErr,
            _ => ExitStatus::IoErr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_golden() {
        // 공개 계약: 값·이름을 바꾸면 이 golden이 실패합니다.
        let golden: [(u8, &str); 13] = [
            (0, "ok"),
            (1, "failure"),
            (64, "usage"),
            (65, "dataerr"),
            (66, "noinput"),
            (69, "unavailable"),
            (70, "software"),
            (73, "cantcreat"),
            (74, "ioerr"),
            (75, "tempfail"),
            (76, "protocol"),
            (77, "noperm"),
            (78, "config"),
        ];
        let actual: Vec<(u8, &str)> = ExitStatus::ALL
            .iter()
            .map(|s| (s.code(), s.name()))
            .collect();
        assert_eq!(actual, golden);
    }

    #[test]
    fn io_kinds_map_to_sysexits() {
        use io::ErrorKind::*;
        let cases = [
            (NotFound, 66),
            (PermissionDenied, 77),
            (AlreadyExists, 73),
            (Interrupted, 75),
            (WouldBlock, 75),
            (TimedOut, 75),
            (InvalidData, 65),
            (UnexpectedEof, 74),
            (BrokenPipe, 74),
        ];
        for (kind, code) in cases {
            assert_eq!(ExitStatus::from_io_kind(kind).code(), code, "{kind:?}");
        }
    }
}
