//! 창 session 명령과 셸 공유 command ID (XII.6.4, XII.6.5).
//!
//! 셸 메뉴·단축키가 활성 창 session에 보내는 명령입니다. 명령마다 두 식별자가 있고 둘 다 코어가 정합니다.
//!
//! - **코드**([`AppCommand::code`]): C ABI 정수(`STELLAR_COMMAND_*`). 값은 바꾸지 않고 끝에 덧붙입니다.
//! - **공유 이름**([`AppCommand::name`]): 셸의 명령 식별자와 같은 문자열(예: `edit.undo`). macOS·Windows 셸은 각자
//!   운영체제 관례의 단축키(macOS `⌘Z`·`⇧⌘Z`·`⌘S`, Windows `Ctrl+Z`·`Ctrl+Y`·`Ctrl+S`)를 메뉴에 두고, 메뉴 항목의
//!   명령 이름을 이 이름으로 코어에 물어 코드를 얻습니다(`stellar_command_code`). 단축키는 셸이, 명령 ID는 코어가
//!   소유합니다.
//!
//! 명령은 [`AppRuntime::perform`](crate::app::AppRuntime::perform)으로 창 session 하나에 보내고, 결과는 명령별 완료
//! event 하나로 돌아옵니다.

/// 창 session 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppCommand {
    /// 이 창 history의 마지막 항목 되돌리기(§23.2 `Cmd/Ctrl+Z`).
    Undo,
    /// 이 창에서 마지막으로 되돌린 항목 다시 적용(§23.2 `Cmd/Ctrl+Shift+Z`).
    Redo,
    /// 작업 문서를 지금 저장(§22.1 `Save Now`).
    SaveNow,
    /// active 탭 확대 한 단계(×2, 화면 중심 고정, XII.4.3).
    ZoomIn,
    /// active 탭 축소 한 단계(÷2, 화면 중심 고정).
    ZoomOut,
    /// active 탭 배율 100%(화면 중심 고정).
    ZoomToActualSize,
}

impl AppCommand {
    /// 전체 명령(코드 순).
    pub const ALL: [AppCommand; 6] = [
        AppCommand::Undo,
        AppCommand::Redo,
        AppCommand::SaveNow,
        AppCommand::ZoomIn,
        AppCommand::ZoomOut,
        AppCommand::ZoomToActualSize,
    ];

    /// C ABI 코드 값.
    pub const fn code(self) -> u32 {
        match self {
            AppCommand::Undo => 1,
            AppCommand::Redo => 2,
            AppCommand::SaveNow => 3,
            AppCommand::ZoomIn => 4,
            AppCommand::ZoomOut => 5,
            AppCommand::ZoomToActualSize => 6,
        }
    }

    /// 셸과 공유하는 command 이름(셸 메뉴 명령 식별자와 같은 문자열).
    pub const fn name(self) -> &'static str {
        match self {
            AppCommand::Undo => "edit.undo",
            AppCommand::Redo => "edit.redo",
            AppCommand::SaveNow => "file.save_now",
            AppCommand::ZoomIn => "view.zoom_in",
            AppCommand::ZoomOut => "view.zoom_out",
            AppCommand::ZoomToActualSize => "view.zoom_actual_size",
        }
    }

    /// 코드 값에서 복원합니다.
    pub fn from_code(code: u32) -> Option<AppCommand> {
        AppCommand::ALL
            .into_iter()
            .find(|command| command.code() == code)
    }

    /// 공유 이름에서 복원합니다. 코어가 구현하지 않은 이름은 `None`입니다.
    pub fn from_name(name: &str) -> Option<AppCommand> {
        AppCommand::ALL
            .into_iter()
            .find(|command| command.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_unique() {
        let codes: Vec<u32> = AppCommand::ALL.iter().map(|c| c.code()).collect();
        assert_eq!(codes, [1, 2, 3, 4, 5, 6]);
        for command in AppCommand::ALL {
            assert_eq!(AppCommand::from_code(command.code()), Some(command));
        }
        assert_eq!(AppCommand::from_code(0), None);
        assert_eq!(AppCommand::from_code(7), None);
    }

    #[test]
    fn shared_names_are_stable_and_unique() {
        let names: Vec<&str> = AppCommand::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(
            names,
            [
                "edit.undo",
                "edit.redo",
                "file.save_now",
                "view.zoom_in",
                "view.zoom_out",
                "view.zoom_actual_size"
            ]
        );
        for command in AppCommand::ALL {
            assert_eq!(AppCommand::from_name(command.name()), Some(command));
        }
        // 셸 전용 명령과 아직 구현하지 않은 명령은 코어 ID가 없습니다.
        for name in ["app.quit", "edit.copy", "view.zoom_to_fit", "", "EDIT.UNDO"] {
            assert_eq!(AppCommand::from_name(name), None, "{name}");
        }
    }
}
