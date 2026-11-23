//! 플러그인 권한 종류 (XXI.1.1 manifest `permissions`, XXI.4.5, ADR-0097 결정 5).
//!
//! 권한은 닫힌 enum입니다. manifest가 요청(선언)하고 사용자가 승인해야 Host API method를 부를 수 있습니다.
//! 선언하지 않은 권한은 승인할 수 없고, 승인하지 않은 권한이 필요한 method는 `permission-denied`입니다.

use std::fmt;

/// 플러그인 권한.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// 문서 snapshot·projection 읽기.
    DocumentRead,
    /// CommandTransaction 적용(문서 쓰기).
    DocumentApply,
    /// Git 상태 읽기.
    GitRead,
    /// 플러그인 namespace 디렉터리 파일 읽기·쓰기.
    File,
    /// network 요청.
    Network,
    /// 내장 terminal.
    Terminal,
    /// clipboard.
    Clipboard,
}

impl Permission {
    /// 모든 권한(이름 표 순서).
    pub const ALL: [Permission; 7] = [
        Permission::DocumentRead,
        Permission::DocumentApply,
        Permission::GitRead,
        Permission::File,
        Permission::Network,
        Permission::Terminal,
        Permission::Clipboard,
    ];

    /// manifest·IDL·저장 파일에 쓰는 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            Permission::DocumentRead => "document.read",
            Permission::DocumentApply => "document.apply",
            Permission::GitRead => "git.read",
            Permission::File => "file",
            Permission::Network => "network",
            Permission::Terminal => "terminal",
            Permission::Clipboard => "clipboard",
        }
    }

    /// 이름으로 찾습니다. 모르는 이름은 `None`.
    pub fn from_name(name: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.name() == name)
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_unique() {
        let mut names: Vec<_> = Permission::ALL.iter().map(|p| p.name()).collect();
        for p in Permission::ALL {
            assert_eq!(Permission::from_name(p.name()), Some(p));
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Permission::ALL.len());
        assert_eq!(Permission::from_name("Document.Read"), None);
    }
}
