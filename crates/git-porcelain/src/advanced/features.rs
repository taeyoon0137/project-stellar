//! 고급 Git 쓰기 기능 feature flag — 기본 비활성 (XXVI.4.2).
//!
//! XXVI.4.1의 읽기 기능(tags·notes·bisect·sparse-checkout·submodule)에 대응하는 **쓰기** 기능은 기능마다 별도
//! 작업·승인을 거쳐서만 추가합니다. 시스템 Git이 그 명령을 지원한다는 이유만으로 자동 노출하지 않습니다.
//!
//! - [`AdvancedFeatureFlags::default`]는 모든 쓰기 기능을 끈 상태입니다.
//! - Git capability probe 결과([`GitCapabilities`])로 flag를 만들어도 모두 꺼져 있습니다
//!   ([`AdvancedFeatureFlags::from_capabilities`]).
//! - flag를 켜려면 그 기능의 승인된 구현 작업이 [`approved_task`](AdvancedWriteFeature::approved_task)에 있어야
//!   합니다. 현재 승인된 쓰기 기능은 없으므로 모든 [`AdvancedFeatureFlags::enable`]은
//!   [`FeatureError::NotApproved`]입니다.
//! - 쓰기 경로는 [`require_enabled`]를 먼저 통과해야 하며, 꺼져 있으면 Git을 실행하지 않습니다.

use std::collections::BTreeSet;

use crate::capability::GitCapabilities;

/// 고급 Git 쓰기 기능.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AdvancedWriteFeature {
    /// tag 생성.
    TagCreate,
    /// tag 삭제.
    TagDelete,
    /// notes 추가·수정·삭제.
    NotesEdit,
    /// bisect start/good/bad/reset.
    BisectControl,
    /// sparse-checkout set/add/disable.
    SparseCheckoutEdit,
    /// submodule init/update/sync.
    SubmoduleUpdate,
}

impl AdvancedWriteFeature {
    /// 정의된 모든 쓰기 기능(고정 순서).
    pub const ALL: [AdvancedWriteFeature; 6] = [
        AdvancedWriteFeature::TagCreate,
        AdvancedWriteFeature::TagDelete,
        AdvancedWriteFeature::NotesEdit,
        AdvancedWriteFeature::BisectControl,
        AdvancedWriteFeature::SparseCheckoutEdit,
        AdvancedWriteFeature::SubmoduleUpdate,
    ];

    /// 안정적인 이름(설정·진단용).
    pub fn as_str(self) -> &'static str {
        match self {
            AdvancedWriteFeature::TagCreate => "git.tag.create",
            AdvancedWriteFeature::TagDelete => "git.tag.delete",
            AdvancedWriteFeature::NotesEdit => "git.notes.edit",
            AdvancedWriteFeature::BisectControl => "git.bisect.control",
            AdvancedWriteFeature::SparseCheckoutEdit => "git.sparse-checkout.edit",
            AdvancedWriteFeature::SubmoduleUpdate => "git.submodule.update",
        }
    }

    /// 이 기능의 쓰기 구현을 승인한 WBS 작업(아직 없으면 `None`).
    pub fn approved_task(self) -> Option<&'static str> {
        None
    }
}

/// flag 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureError {
    /// 기능이 꺼져 있음(Git 미실행).
    Disabled(AdvancedWriteFeature),
    /// 승인된 구현 작업이 없어 켤 수 없음.
    NotApproved(AdvancedWriteFeature),
}

impl std::fmt::Display for FeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeatureError::Disabled(feature) => {
                write!(f, "비활성 기능입니다: {}", feature.as_str())
            }
            FeatureError::NotApproved(feature) => write!(
                f,
                "승인된 구현이 없어 켤 수 없는 기능입니다: {}",
                feature.as_str()
            ),
        }
    }
}

impl std::error::Error for FeatureError {}

/// 고급 Git 쓰기 기능 flag(기본 전부 비활성).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvancedFeatureFlags {
    enabled: BTreeSet<AdvancedWriteFeature>,
}

impl AdvancedFeatureFlags {
    /// Git capability와 무관하게 전부 비활성인 flag(Git이 지원해도 자동 노출하지 않음).
    pub fn from_capabilities(_capabilities: &GitCapabilities) -> AdvancedFeatureFlags {
        AdvancedFeatureFlags::default()
    }

    /// 기능이 켜져 있는지.
    pub fn is_enabled(&self, feature: AdvancedWriteFeature) -> bool {
        self.enabled.contains(&feature)
    }

    /// 켜진 기능 목록.
    pub fn enabled(&self) -> Vec<AdvancedWriteFeature> {
        self.enabled.iter().copied().collect()
    }

    /// 기능을 켭니다. 승인된 구현 작업이 없으면 거부합니다.
    pub fn enable(&mut self, feature: AdvancedWriteFeature) -> Result<(), FeatureError> {
        if feature.approved_task().is_none() {
            return Err(FeatureError::NotApproved(feature));
        }
        self.enabled.insert(feature);
        Ok(())
    }
}

/// 쓰기 경로 진입 전 검사. 꺼져 있으면 [`FeatureError::Disabled`]이며 호출자는 Git을 실행하지 않습니다.
pub fn require_enabled(
    flags: &AdvancedFeatureFlags,
    feature: AdvancedWriteFeature,
) -> Result<(), FeatureError> {
    if flags.is_enabled(feature) {
        Ok(())
    } else {
        Err(FeatureError::Disabled(feature))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::GitVersion;

    #[test]
    fn feature_flags_default_off() {
        let flags = AdvancedFeatureFlags::default();
        for feature in AdvancedWriteFeature::ALL {
            assert!(!flags.is_enabled(feature), "{}", feature.as_str());
            assert_eq!(
                require_enabled(&flags, feature),
                Err(FeatureError::Disabled(feature))
            );
        }
        assert!(flags.enabled().is_empty());
    }

    #[test]
    fn git_support_does_not_auto_enable_writes() {
        // 모든 capability를 지원하는 최신 Git이어도 쓰기 기능은 꺼져 있음.
        let caps = GitCapabilities::from_version(
            GitVersion {
                major: 2,
                minor: 54,
                patch: 0,
                raw: "git version 2.54.0".into(),
            },
            true,
        );
        let flags = AdvancedFeatureFlags::from_capabilities(&caps);
        assert_eq!(flags, AdvancedFeatureFlags::default());
        assert!(flags.enabled().is_empty());
    }

    #[test]
    fn unapproved_features_cannot_be_enabled() {
        let mut flags = AdvancedFeatureFlags::default();
        for feature in AdvancedWriteFeature::ALL {
            assert_eq!(feature.approved_task(), None);
            assert_eq!(
                flags.enable(feature),
                Err(FeatureError::NotApproved(feature))
            );
            assert!(!flags.is_enabled(feature));
        }
    }

    #[test]
    fn disabled_write_path_never_runs_git() {
        use crate::runner::{argv, GitRunner};
        use crate::trust::{GatedGitRunner, GitTrust};
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        let flags = AdvancedFeatureFlags::default();
        // 쓰기 경로 모형: flag 검사 통과 시에만 Git 실행.
        let try_write = |feature| -> Result<(), FeatureError> {
            require_enabled(&flags, feature)?;
            let _ = gate.run(&argv(["tag", "x"]), &std::env::temp_dir());
            Ok(())
        };
        for feature in AdvancedWriteFeature::ALL {
            assert!(try_write(feature).is_err());
        }
        assert_eq!(runner.spawn_count(), 0);
    }

    #[test]
    fn feature_names_are_stable_and_unique() {
        let names: Vec<&str> = AdvancedWriteFeature::ALL
            .iter()
            .map(|f| f.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "git.tag.create",
                "git.tag.delete",
                "git.notes.edit",
                "git.bisect.control",
                "git.sparse-checkout.edit",
                "git.submodule.update"
            ]
        );
    }
}
