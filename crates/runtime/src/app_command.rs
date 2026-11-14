//! 창 session 명령 (XII.6.4).
//!
//! 셸 메뉴·단축키가 활성 창 session에 보내는 명령입니다. 값은 C ABI(`STELLAR_COMMAND_*`)에 노출되므로 바꾸지 않고
//! 끝에 덧붙입니다. 명령은 [`AppRuntime::perform`](crate::app::AppRuntime::perform)으로 창 session 하나에 보내고,
//! 결과는 명령별 완료 event 하나로 돌아옵니다.

/// 창 session 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppCommand {
    /// 이 창 history의 마지막 항목 되돌리기(§23.2 `Cmd/Ctrl+Z`).
    Undo,
    /// 이 창에서 마지막으로 되돌린 항목 다시 적용(§23.2 `Cmd/Ctrl+Shift+Z`).
    Redo,
    /// 작업 문서를 지금 저장(§22.1 `Save Now`).
    SaveNow,
}

impl AppCommand {
    /// 전체 명령(코드 순).
    pub const ALL: [AppCommand; 3] = [AppCommand::Undo, AppCommand::Redo, AppCommand::SaveNow];

    /// C ABI 코드 값.
    pub const fn code(self) -> u32 {
        match self {
            AppCommand::Undo => 1,
            AppCommand::Redo => 2,
            AppCommand::SaveNow => 3,
        }
    }

    /// 코드 값에서 복원합니다.
    pub fn from_code(code: u32) -> Option<AppCommand> {
        AppCommand::ALL
            .into_iter()
            .find(|command| command.code() == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_unique() {
        let codes: Vec<u32> = AppCommand::ALL.iter().map(|c| c.code()).collect();
        assert_eq!(codes, [1, 2, 3]);
        for command in AppCommand::ALL {
            assert_eq!(AppCommand::from_code(command.code()), Some(command));
        }
        assert_eq!(AppCommand::from_code(0), None);
        assert_eq!(AppCommand::from_code(4), None);
    }
}
