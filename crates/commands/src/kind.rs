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
    /// 테스트 전용 probe(fault injection). 프로덕션 빌드에는 없습니다.
    #[cfg(test)]
    TestProbe,
}

impl CommandKind {
    /// 프로덕션 registry에 등록된 모든 종류(번호 오름차순).
    pub const ALL: &'static [CommandKind] = &[CommandKind::InsertNode, CommandKind::DeleteSubtree];

    /// 영구 예약 값(유효한 종류가 아님).
    pub const RESERVED: u16 = 0x0000;

    /// 안정적인 숫자 값.
    pub fn as_u16(self) -> u16 {
        match self {
            CommandKind::InsertNode => 0x0001,
            CommandKind::DeleteSubtree => 0x0002,
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
            #[cfg(test)]
            CommandKind::TestProbe => "test-probe",
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
            vec![(0x0001, "insert-node"), (0x0002, "delete-subtree")]
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
