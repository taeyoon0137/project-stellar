//! 프로젝트 신뢰 capability matrix (XVI.7.1).
//!
//! 신뢰 상태(IV.5.2·XVI.7.2)에 따라 허용되는 기능을 **표 하나**([`CAPABILITY_MATRIX`])로 정의합니다
//! (기술 아키텍처 24.3 "처음 여는 프로젝트는 제한 모드", 37.1 "신뢰되지 않은 프로젝트"). 실제 차단
//! 지점은 이 표에서 값을 얻습니다.
//!
//! | capability | 비신뢰 | 신뢰 | 차단 지점 |
//! |---|---|---|---|
//! | 문서 읽기 | 제한(프로젝트 내부만) | 허용 | [`PathResolver`](crate::path_resolver::PathResolver) |
//! | 시스템 Git 실행 | 금지 | 허용 | [`GatedGitRunner`](stellar_git_porcelain::trust::GatedGitRunner) |
//! | 터미널 | 금지 | 허용 | 로드맵(내장 터미널 구현 시 이 표를 사용) |
//! | MCP·에이전트 | 금지 | 허용 | 로드맵(MCP 구현 시 이 표를 사용) |
//! | 외부 filter·diff/merge driver·프로젝트 지정 명령 | 금지 | 허용 | Git 실행 gate(시스템 Git만 실행) |
//! | 외부 symlink target 내용 읽기 | 금지 | 허용 | [`PathResolver`](crate::path_resolver::PathResolver) |
//!
//! "허용"은 신뢰 상태가 막지 않는다는 뜻이며, 기능별 권한(예: MCP 세션 승인)은 따로 적용됩니다.

use stellar_git_porcelain::trust::GitTrust;

use crate::trust::TrustStatus;

/// 신뢰 상태로 허용 여부가 정해지는 프로젝트 기능.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProjectCapability {
    /// 프로젝트 문서·메타데이터 읽기.
    ReadDocument,
    /// 저장소 안에서의 시스템 Git 실행(hooks·filters·fsmonitor를 실행할 수 있음).
    RunGit,
    /// 내장 터미널(로드맵).
    Terminal,
    /// MCP와 에이전트(로드맵).
    Mcp,
    /// 외부 filter·diff/merge driver·프로젝트가 지정한 외부 명령.
    ExternalFilter,
    /// 프로젝트 밖을 가리키는 symlink target의 내용 읽기.
    ReadExternalSymlinkTarget,
}

impl ProjectCapability {
    /// 모든 capability(표 순서).
    pub const ALL: [ProjectCapability; 6] = [
        ProjectCapability::ReadDocument,
        ProjectCapability::RunGit,
        ProjectCapability::Terminal,
        ProjectCapability::Mcp,
        ProjectCapability::ExternalFilter,
        ProjectCapability::ReadExternalSymlinkTarget,
    ];

    /// 고정 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            ProjectCapability::ReadDocument => "read-document",
            ProjectCapability::RunGit => "run-git",
            ProjectCapability::Terminal => "terminal",
            ProjectCapability::Mcp => "mcp",
            ProjectCapability::ExternalFilter => "external-filter",
            ProjectCapability::ReadExternalSymlinkTarget => "read-external-symlink-target",
        }
    }
}

/// capability 허용 수준.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityLevel {
    /// 허용.
    Allowed,
    /// 제한 허용(문서 읽기: 프로젝트 내부 내용만, 외부 target은 metadata만).
    Restricted,
    /// 금지.
    Denied,
}

impl CapabilityLevel {
    /// 고정 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            CapabilityLevel::Allowed => "allowed",
            CapabilityLevel::Restricted => "restricted",
            CapabilityLevel::Denied => "denied",
        }
    }
}

/// 표의 한 행.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityRule {
    /// 기능.
    pub capability: ProjectCapability,
    /// 신뢰되지 않은 프로젝트에서의 수준.
    pub untrusted: CapabilityLevel,
    /// 신뢰된 프로젝트에서의 수준.
    pub trusted: CapabilityLevel,
}

/// 신뢰 capability matrix. 각 capability는 정확히 한 행을 가집니다.
pub const CAPABILITY_MATRIX: [CapabilityRule; 6] = [
    CapabilityRule {
        capability: ProjectCapability::ReadDocument,
        untrusted: CapabilityLevel::Restricted,
        trusted: CapabilityLevel::Allowed,
    },
    CapabilityRule {
        capability: ProjectCapability::RunGit,
        untrusted: CapabilityLevel::Denied,
        trusted: CapabilityLevel::Allowed,
    },
    CapabilityRule {
        capability: ProjectCapability::Terminal,
        untrusted: CapabilityLevel::Denied,
        trusted: CapabilityLevel::Allowed,
    },
    CapabilityRule {
        capability: ProjectCapability::Mcp,
        untrusted: CapabilityLevel::Denied,
        trusted: CapabilityLevel::Allowed,
    },
    CapabilityRule {
        capability: ProjectCapability::ExternalFilter,
        untrusted: CapabilityLevel::Denied,
        trusted: CapabilityLevel::Allowed,
    },
    CapabilityRule {
        capability: ProjectCapability::ReadExternalSymlinkTarget,
        untrusted: CapabilityLevel::Denied,
        trusted: CapabilityLevel::Allowed,
    },
];

/// capability의 허용 수준.
pub fn capability_level(capability: ProjectCapability, trust: TrustStatus) -> CapabilityLevel {
    let rule = CAPABILITY_MATRIX
        .iter()
        .find(|rule| rule.capability == capability);
    match (rule, trust) {
        (Some(rule), TrustStatus::Trusted) => rule.trusted,
        (Some(rule), TrustStatus::NotTrusted) => rule.untrusted,
        // 표에 없는 capability는 금지(안전한 기본값).
        (None, _) => CapabilityLevel::Denied,
    }
}

/// capability 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDenied {
    /// 거부된 기능.
    pub capability: ProjectCapability,
}

impl std::fmt::Display for CapabilityDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "신뢰되지 않은 프로젝트에서는 허용되지 않는 기능입니다: {}",
            self.capability.code()
        )
    }
}

impl std::error::Error for CapabilityDenied {}

/// capability를 검사합니다. 금지면 [`CapabilityDenied`], 아니면 허용 수준(`Allowed`/`Restricted`).
pub fn check_capability(
    capability: ProjectCapability,
    trust: TrustStatus,
) -> Result<CapabilityLevel, CapabilityDenied> {
    match capability_level(capability, trust) {
        CapabilityLevel::Denied => Err(CapabilityDenied { capability }),
        level => Ok(level),
    }
}

/// 표의 [`ProjectCapability::RunGit`] 행에서 Git 실행 gate 상태를 얻습니다.
pub fn git_trust(trust: TrustStatus) -> GitTrust {
    match capability_level(ProjectCapability::RunGit, trust) {
        CapabilityLevel::Allowed => GitTrust::Trusted,
        CapabilityLevel::Restricted | CapabilityLevel::Denied => GitTrust::NotTrusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ProjectIdentity;
    use crate::path_resolver::{ExternalAccess, PathResolver, ResolveError};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use stellar_core_types::ids::StableId;
    use stellar_git_porcelain::runner::{argv, GitRunner};
    use stellar_git_porcelain::trust::{GatedGitRunner, GatedRunError};

    const TRUST: [TrustStatus; 2] = [TrustStatus::NotTrusted, TrustStatus::Trusted];

    /// 표를 `capability | 비신뢰 | 신뢰` 텍스트로 렌더링합니다.
    fn render_matrix() -> String {
        CAPABILITY_MATRIX
            .iter()
            .map(|rule| {
                format!(
                    "{} | {} | {}\n",
                    rule.capability.code(),
                    rule.untrusted.code(),
                    rule.trusted.code()
                )
            })
            .collect()
    }

    #[test]
    fn capability_table_matches_golden() {
        assert_eq!(
            render_matrix(),
            "read-document | restricted | allowed\n\
             run-git | denied | allowed\n\
             terminal | denied | allowed\n\
             mcp | denied | allowed\n\
             external-filter | denied | allowed\n\
             read-external-symlink-target | denied | allowed\n"
        );
    }

    #[test]
    fn every_capability_has_exactly_one_rule() {
        for capability in ProjectCapability::ALL {
            let rows = CAPABILITY_MATRIX
                .iter()
                .filter(|rule| rule.capability == capability)
                .count();
            assert_eq!(rows, 1, "{}", capability.code());
        }
        assert_eq!(CAPABILITY_MATRIX.len(), ProjectCapability::ALL.len());
    }

    #[test]
    fn untrusted_allows_only_restricted_document_reads() {
        for capability in ProjectCapability::ALL {
            let untrusted = capability_level(capability, TrustStatus::NotTrusted);
            if capability == ProjectCapability::ReadDocument {
                assert_eq!(untrusted, CapabilityLevel::Restricted);
                assert_eq!(
                    check_capability(capability, TrustStatus::NotTrusted),
                    Ok(CapabilityLevel::Restricted)
                );
            } else {
                assert_eq!(untrusted, CapabilityLevel::Denied, "{}", capability.code());
                let denied = check_capability(capability, TrustStatus::NotTrusted).unwrap_err();
                assert_eq!(denied.capability, capability);
                assert!(denied.to_string().contains(capability.code()));
            }
            // 신뢰 후에는 신뢰 상태가 막는 기능이 없음.
            assert_eq!(
                check_capability(capability, TrustStatus::Trusted),
                Ok(CapabilityLevel::Allowed)
            );
        }
    }

    #[test]
    fn git_gate_follows_the_table() {
        let cwd = std::env::temp_dir();
        for trust in TRUST {
            // 존재하지 않는 executable: spawn을 시도하면 카운터가 증가합니다.
            let runner = GitRunner::new(
                PathBuf::from("/nonexistent/git"),
                Vec::new(),
                Duration::from_secs(5),
            );
            let gate = GatedGitRunner::new(runner, git_trust(trust));
            let result = gate.run(&argv(["status"]), &cwd);
            match capability_level(ProjectCapability::RunGit, trust) {
                CapabilityLevel::Denied => {
                    assert_eq!(result, Err(GatedRunError::ProjectNotTrusted));
                    assert_eq!(gate.spawn_count(), 0, "비신뢰 Git spawn 0");
                }
                _ => {
                    assert!(matches!(result, Err(GatedRunError::Run(_))));
                    assert_eq!(gate.spawn_count(), 1);
                }
            }
        }
    }

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-capability-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn document_and_external_reads_follow_the_table() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(project.join("page.bin"), b"doc").unwrap();
        std::fs::write(outside.join("secret.txt"), b"external").unwrap();
        symlink(outside.join("secret.txt"), project.join("ext.txt")).unwrap();
        let identity = ProjectIdentity::new(StableId::new_v7(1, 0, 1), &project).unwrap();

        for trust in TRUST {
            let resolver = PathResolver::for_project(&identity, trust);
            // 문서 읽기: 비신뢰에서도 내부 문서는 읽힘(제한 허용).
            assert_ne!(
                capability_level(ProjectCapability::ReadDocument, trust),
                CapabilityLevel::Denied
            );
            assert_eq!(resolver.read("page.bin").unwrap(), b"doc");
            // 외부 symlink target: 표가 금지면 resolver도 내용을 읽지 않음.
            let external = resolver.read("ext.txt");
            match capability_level(ProjectCapability::ReadExternalSymlinkTarget, trust) {
                CapabilityLevel::Denied => {
                    assert_eq!(resolver.external_access(), ExternalAccess::MetadataOnly);
                    assert!(matches!(
                        external,
                        Err(ResolveError::ExternalTargetNotTrusted { .. })
                    ));
                }
                _ => {
                    assert_eq!(resolver.external_access(), ExternalAccess::Allowed);
                    assert_eq!(external.unwrap(), b"external");
                }
            }
        }
    }
}
