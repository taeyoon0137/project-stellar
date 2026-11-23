//! MCP 경로의 권한 어휘 (XIX.3.1, 아키텍처 §31.3, ADR-0085 결정 5).
//!
//! 권한은 서로 독립입니다(§31.3). 한 권한의 승인이 다른 권한을 함께 허용하지 않습니다(XIX.3.4). 이 모듈은
//! 어휘와 고정 이름만 정의합니다. 저장(ProjectIdentity+AgentSessionId별, XIX.3.2)과 승인 판정(XIX.3.3·XIX.3.4)은
//! 앱이 소유하며, MCP 경로에는 권한 상태를 쓰는 수단이 없습니다(ADR-0005 §3 변경 불가 경로).

/// 권한 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// 프로젝트·문서 읽기.
    Read,
    /// 적용하지 않는 변경 제안(dry-run). 문서 상태를 바꾸지 않습니다.
    Propose,
    /// 승인된 변경 적용(앱 lease·승인·DocumentThread 경로).
    Apply,
    /// 내장 터미널 명령 실행.
    Terminal,
    /// 파일 삭제.
    FileDelete,
    /// Git commit.
    GitCommit,
    /// Git push.
    GitPush,
}

impl Permission {
    /// 모든 권한(고정 순서, XIX.3.1 정의 순서).
    pub const ALL: [Permission; 7] = [
        Permission::Read,
        Permission::Propose,
        Permission::Apply,
        Permission::Terminal,
        Permission::FileDelete,
        Permission::GitCommit,
        Permission::GitPush,
    ];

    /// 고정 이름(catalog·결과 JSON에 쓰는 kebab-case).
    pub fn name(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Propose => "propose",
            Permission::Apply => "apply",
            Permission::Terminal => "terminal",
            Permission::FileDelete => "file-delete",
            Permission::GitCommit => "git-commit",
            Permission::GitPush => "git-push",
        }
    }

    /// 이름으로 찾습니다. 모르는 이름은 `None`입니다.
    pub fn from_name(name: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.name() == name)
    }

    /// 문서·프로젝트 상태를 바꾸지 않는 권한인지(Read·Propose). headless 읽기 backend는 이 권한이
    /// 필요한 tool만 제공합니다(ADR-0005 §3).
    pub fn is_non_mutating(self) -> bool {
        matches!(self, Permission::Read | Permission::Propose)
    }

    /// 문서 편집 계열(Read·Propose·Apply)인지. 터미널·파일 삭제·Git 권한은 문서 권한과 결합하지 않습니다
    /// (§31.3, XIX.2.4).
    pub fn is_document(self) -> bool {
        matches!(
            self,
            Permission::Read | Permission::Propose | Permission::Apply
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_enum_golden() {
        // 검증 기준(XIX.3.1): enum golden.
        let golden: Vec<(&str, bool, bool)> = Permission::ALL
            .iter()
            .map(|p| (p.name(), p.is_non_mutating(), p.is_document()))
            .collect();
        assert_eq!(
            golden,
            [
                ("read", true, true),
                ("propose", true, true),
                ("apply", false, true),
                ("terminal", false, false),
                ("file-delete", false, false),
                ("git-commit", false, false),
                ("git-push", false, false),
            ]
        );
        for p in Permission::ALL {
            assert_eq!(Permission::from_name(p.name()), Some(p));
        }
        assert_eq!(Permission::from_name("admin"), None);
        assert_eq!(Permission::from_name("Read"), None, "대소문자 구분");
        // 순서는 정의 순서와 같고 중복이 없습니다.
        let mut sorted = Permission::ALL;
        sorted.sort();
        assert_eq!(sorted, Permission::ALL);
    }
}
