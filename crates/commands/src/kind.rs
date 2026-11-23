//! command 종류 numeric registry (VII.1.2).
//!
//! [`CommandKind`]는 commands crate가 소유하는 **닫힌(closed) numeric registry**입니다. 각
//! 종류의 숫자 값은 command 직렬화(VII.1.2)·history journal(XXV.1)의 일부이므로 한 번 정한
//! 값은 바꾸거나 재사용하지 않습니다. 디코딩은 registry에 없는 값을 거부합니다.
//!
//! 각 문서 command 작업(VII.2.x)이 자기 종류를 명시 번호로 등록합니다. `0x0000`은 영구
//! 예약(유효한 종류가 아님)입니다. 테스트 빌드에만 존재하는 `TestProbe`(`0xFFF0`)는 command 계층 자체(transaction guard 등)를 검증하는
//! fault-injection용 test double이며 [`CommandKind::ALL`]에 포함되지 않습니다.

use std::fmt;

/// command 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandKind {
    /// node(subtree) 삽입 (VII.2.1).
    InsertNode,
    /// node subtree 삭제 (VII.2.1·VII.2.2).
    DeleteSubtree,
    /// node 이동·재정렬 (VII.2.3).
    MoveNode,
    /// node 속성 변경 (VII.2.4).
    SetProperty,
    /// modifier 삽입 (VII.2.6).
    InsertModifier,
    /// modifier 제거 (VII.2.6).
    RemoveModifier,
    /// modifier 순서 변경 (VII.2.6).
    MoveModifier,
    /// modifier 속성 변경 (VII.2.6).
    SetModifierProperty,
    /// Fragment → Frame 변환 (VII.2.7).
    ConvertFragmentToFrame,
    /// Shape → Path 변환 (VII.2.7).
    ConvertShapeToPath,
    /// node 레코드 복원(변환의 역, VII.2.7).
    RestoreNodeRecords,
    /// subtree 복제 (VII.2.8).
    DuplicateSubtree,
    /// Unicode scalar 범위의 Text 본문 교체.
    ReplaceText,
    /// 페이지 추가 (UX.12.1, ADR-0079).
    AddPage,
    /// 페이지 삭제(AddPage의 역, ADR-0079).
    RemovePage,
    /// 페이지 이름 변경 (UX.12.1, ADR-0079).
    RenamePage,
    /// View 추가 (UX.12.1, ADR-0079).
    AddView,
    /// View 삭제(AddView의 역, ADR-0079).
    RemoveView,
    /// View 이름 변경 (UX.12.1, ADR-0079).
    RenameView,
    /// 테스트 전용 probe(fault injection). 프로덕션 빌드에는 없습니다.
    #[cfg(test)]
    TestProbe,
}

impl CommandKind {
    /// 프로덕션 registry에 등록된 모든 종류(번호 오름차순).
    pub const ALL: &'static [CommandKind] = &[
        CommandKind::InsertNode,
        CommandKind::DeleteSubtree,
        CommandKind::MoveNode,
        CommandKind::SetProperty,
        CommandKind::InsertModifier,
        CommandKind::RemoveModifier,
        CommandKind::MoveModifier,
        CommandKind::SetModifierProperty,
        CommandKind::ConvertFragmentToFrame,
        CommandKind::ConvertShapeToPath,
        CommandKind::RestoreNodeRecords,
        CommandKind::DuplicateSubtree,
        CommandKind::ReplaceText,
        CommandKind::AddPage,
        CommandKind::RemovePage,
        CommandKind::RenamePage,
        CommandKind::AddView,
        CommandKind::RemoveView,
        CommandKind::RenameView,
    ];

    /// 영구 예약 값(유효한 종류가 아님).
    pub const RESERVED: u16 = 0x0000;

    /// 안정적인 숫자 값.
    pub fn as_u16(self) -> u16 {
        match self {
            CommandKind::InsertNode => 0x0001,
            CommandKind::DeleteSubtree => 0x0002,
            CommandKind::MoveNode => 0x0003,
            CommandKind::SetProperty => 0x0010,
            CommandKind::InsertModifier => 0x0020,
            CommandKind::RemoveModifier => 0x0021,
            CommandKind::MoveModifier => 0x0022,
            CommandKind::SetModifierProperty => 0x0023,
            CommandKind::ConvertFragmentToFrame => 0x0030,
            CommandKind::ConvertShapeToPath => 0x0031,
            CommandKind::RestoreNodeRecords => 0x0032,
            CommandKind::DuplicateSubtree => 0x0040,
            CommandKind::ReplaceText => 0x0050,
            CommandKind::AddPage => 0x0060,
            CommandKind::RemovePage => 0x0061,
            CommandKind::RenamePage => 0x0062,
            CommandKind::AddView => 0x0063,
            CommandKind::RemoveView => 0x0064,
            CommandKind::RenameView => 0x0065,
            #[cfg(test)]
            CommandKind::TestProbe => 0xFFF0,
        }
    }

    /// 숫자 값에서 복원합니다. registry에 없는 값은 `None`입니다.
    pub fn from_u16(value: u16) -> Option<CommandKind> {
        #[cfg(test)]
        if value == CommandKind::TestProbe.as_u16() {
            return Some(CommandKind::TestProbe);
        }
        CommandKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_u16() == value)
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            CommandKind::InsertNode => "insert-node",
            CommandKind::DeleteSubtree => "delete-subtree",
            CommandKind::MoveNode => "move-node",
            CommandKind::SetProperty => "set-property",
            CommandKind::InsertModifier => "insert-modifier",
            CommandKind::RemoveModifier => "remove-modifier",
            CommandKind::MoveModifier => "move-modifier",
            CommandKind::SetModifierProperty => "set-modifier-property",
            CommandKind::ConvertFragmentToFrame => "convert-fragment-to-frame",
            CommandKind::ConvertShapeToPath => "convert-shape-to-path",
            CommandKind::RestoreNodeRecords => "restore-node-records",
            CommandKind::DuplicateSubtree => "duplicate-subtree",
            CommandKind::ReplaceText => "replace-text",
            CommandKind::AddPage => "add-page",
            CommandKind::RemovePage => "remove-page",
            CommandKind::RenamePage => "rename-page",
            CommandKind::AddView => "add-view",
            CommandKind::RemoveView => "remove-view",
            CommandKind::RenameView => "rename-view",
            #[cfg(test)]
            CommandKind::TestProbe => "test-probe",
        }
    }

    /// 이 종류를 처음 담은 command codec 버전([`crate::codec::COMMAND_CODEC_VERSION`], ADR-0079).
    ///
    /// history journal header는 기록한 codec 버전을 담고, reader는 journal 안 command 종류가 그 버전
    /// 안에 있는지 확인합니다. 한 번 정한 값은 바꾸지 않습니다.
    pub fn codec_version(self) -> u32 {
        match self {
            CommandKind::InsertNode
            | CommandKind::DeleteSubtree
            | CommandKind::MoveNode
            | CommandKind::SetProperty
            | CommandKind::InsertModifier
            | CommandKind::RemoveModifier
            | CommandKind::MoveModifier
            | CommandKind::SetModifierProperty
            | CommandKind::ConvertFragmentToFrame
            | CommandKind::ConvertShapeToPath
            | CommandKind::RestoreNodeRecords
            | CommandKind::DuplicateSubtree
            | CommandKind::ReplaceText => 1,
            CommandKind::AddPage
            | CommandKind::RemovePage
            | CommandKind::RenamePage
            | CommandKind::AddView
            | CommandKind::RemoveView
            | CommandKind::RenameView => 2,
            #[cfg(test)]
            CommandKind::TestProbe => 1,
        }
    }

    /// Undo·Redo 메뉴에 쓰는 동작 이름(코어가 정합니다, ADR-0079). 셸은 "실행 취소"·"실행 복귀" 같은
    /// 앞뒤 문구만 붙입니다.
    pub fn action_name(self) -> &'static str {
        match self {
            CommandKind::InsertNode => "삽입",
            CommandKind::DeleteSubtree => "삭제",
            CommandKind::MoveNode => "이동",
            CommandKind::SetProperty => "속성 변경",
            CommandKind::InsertModifier => "Modifier 추가",
            CommandKind::RemoveModifier => "Modifier 삭제",
            CommandKind::MoveModifier => "Modifier 순서 변경",
            CommandKind::SetModifierProperty => "Modifier 속성 변경",
            CommandKind::ConvertFragmentToFrame => "Frame으로 변환",
            CommandKind::ConvertShapeToPath => "Path로 변환",
            CommandKind::RestoreNodeRecords => "변환 되돌리기",
            CommandKind::DuplicateSubtree => "복제",
            CommandKind::ReplaceText => "텍스트 입력",
            CommandKind::AddPage => "페이지 추가",
            CommandKind::RemovePage => "페이지 삭제",
            CommandKind::RenamePage => "페이지 이름 변경",
            CommandKind::AddView => "View 추가",
            CommandKind::RemoveView => "View 삭제",
            CommandKind::RenameView => "View 이름 변경",
            #[cfg(test)]
            CommandKind::TestProbe => "test probe",
        }
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:#06x})", self.name(), self.as_u16())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn registry_numbers_are_unique_sorted_and_not_reserved() {
        let numbers: Vec<u16> = CommandKind::ALL.iter().map(|k| k.as_u16()).collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        assert_eq!(numbers, sorted, "번호 오름차순");
        let unique: BTreeSet<u16> = numbers.iter().copied().collect();
        assert_eq!(unique.len(), numbers.len(), "번호 중복 금지");
        assert!(!unique.contains(&CommandKind::RESERVED));
        assert!(!unique.contains(&CommandKind::TestProbe.as_u16()));
        for kind in CommandKind::ALL {
            assert_eq!(CommandKind::from_u16(kind.as_u16()), Some(*kind));
        }
    }

    #[test]
    fn registry_numbers_are_golden() {
        let golden: Vec<(u16, &str)> = CommandKind::ALL
            .iter()
            .map(|k| (k.as_u16(), k.name()))
            .collect();
        assert_eq!(
            golden,
            vec![
                (0x0001, "insert-node"),
                (0x0002, "delete-subtree"),
                (0x0003, "move-node"),
                (0x0010, "set-property"),
                (0x0020, "insert-modifier"),
                (0x0021, "remove-modifier"),
                (0x0022, "move-modifier"),
                (0x0023, "set-modifier-property"),
                (0x0030, "convert-fragment-to-frame"),
                (0x0031, "convert-shape-to-path"),
                (0x0032, "restore-node-records"),
                (0x0040, "duplicate-subtree"),
                (0x0050, "replace-text"),
                (0x0060, "add-page"),
                (0x0061, "remove-page"),
                (0x0062, "rename-page"),
                (0x0063, "add-view"),
                (0x0064, "remove-view"),
                (0x0065, "rename-view"),
            ]
        );
    }

    #[test]
    fn unknown_numbers_are_rejected() {
        assert_eq!(CommandKind::from_u16(CommandKind::RESERVED), None);
        for value in [0x0FFF, 0x7FFF, 0xFFEF, 0xFFF1, 0xFFFF] {
            assert_eq!(CommandKind::from_u16(value), None, "{value:#06x}");
        }
        assert_eq!(
            CommandKind::from_u16(0xFFF0),
            Some(CommandKind::TestProbe),
            "테스트 빌드의 probe"
        );
        assert_eq!(CommandKind::TestProbe.to_string(), "test-probe(0xfff0)");
    }
}
