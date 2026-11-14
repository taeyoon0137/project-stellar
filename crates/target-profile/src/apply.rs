//! 승인된 profile upgrade 적용과 undo (IX.10.10).
//!
//! 흐름: [`UpgradePlan`](IX.10.9) → 사용자 결정([`UsageResolution`])과 함께 [`approve`] → [`apply_upgrade`] →
//! (필요하면) [`undo_upgrade`].
//!
//! - **원자 단위**: [`UpgradeTransaction`]은 문서 편집([`OverrideEdit`] — profile override 값·node 제외)과 매니페스트 절
//!   교체(pin 변경)를 함께 담습니다. host([`UpgradeHost`])는 문서 편집을 **하나의 `CommandTransaction`**(VII.1.6, 전부
//!   성공 또는 전부 롤백)으로 실행하고 같은 단위에서 `stellar.yaml`을 working tree에 기록합니다. 실패하면 아무것도
//!   바꾸지 않습니다.
//! - **자동 commit 없음**: [`UpgradeHost`]에는 Git commit 수단이 없습니다. 적용 결과는 working tree 변경(`stellar.yaml`
//!   수정, 문서 파일 수정)으로만 남고 commit은 사용자가 Git 화면에서 직접 합니다.
//! - **stale 검사**: 적용은 plan을 계산한 revision·매니페스트 절이 host의 현재 상태와 같을 때만 합니다. undo는 적용 직후
//!   revision·절일 때만 합니다.
//! - **undo**: 적용한 transaction의 역(편집 역순·각 편집의 역, 매니페스트 이전 절)을 같은 host 경로로 실행합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::typed_value::TypedValue;

use crate::manifest::ProjectTargetProfiles;
use crate::overrides::OverrideScope;
use crate::profile::TargetProfileId;
use crate::upgrade::{ProjectSnapshot, UpgradePlan};

/// 영향받는 사용 하나에 대한 사용자 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageResolution {
    /// 그대로 둡니다(새 release의 진단만 받음).
    Keep,
    /// 이 profile에서 사용 node를 제외합니다(구조 override).
    ExcludeNode,
    /// 이 profile의 `Always` override 값을 지웁니다(사용이 profile override에서 온 속성일 때만).
    ClearProfileOverride,
}

/// 승인된 upgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedUpgrade {
    plan: UpgradePlan,
    resolutions: Vec<(usize, UsageResolution)>,
}

impl ApprovedUpgrade {
    /// plan.
    pub fn plan(&self) -> &UpgradePlan {
        &self.plan
    }

    /// (영향 index, 결정)(index 오름차순).
    pub fn resolutions(&self) -> &[(usize, UsageResolution)] {
        &self.resolutions
    }
}

/// 승인 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    /// 영향 목록 밖 index.
    InvalidIndex(usize),
    /// 같은 index에 결정이 두 번.
    DuplicateResolution(usize),
    /// 속성 사용이 아닌 항목에 override 지우기.
    NotAPropertyUsage(usize),
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalError::InvalidIndex(index) => write!(f, "영향 목록에 없는 index {index}"),
            ApprovalError::DuplicateResolution(index) => {
                write!(f, "index {index}에 결정이 두 번 있습니다")
            }
            ApprovalError::NotAPropertyUsage(index) => {
                write!(
                    f,
                    "index {index}는 속성 사용이 아니라 override를 지울 수 없습니다"
                )
            }
        }
    }
}

impl std::error::Error for ApprovalError {}

/// plan을 사용자 결정과 함께 승인합니다. 결정이 없는 영향은 [`UsageResolution::Keep`]입니다.
pub fn approve(
    plan: UpgradePlan,
    mut resolutions: Vec<(usize, UsageResolution)>,
) -> Result<ApprovedUpgrade, ApprovalError> {
    resolutions.sort_by_key(|(index, _)| *index);
    let mut seen = BTreeSet::new();
    for (index, resolution) in &resolutions {
        let affected = plan
            .affected
            .get(*index)
            .ok_or(ApprovalError::InvalidIndex(*index))?;
        if !seen.insert(*index) {
            return Err(ApprovalError::DuplicateResolution(*index));
        }
        if *resolution == UsageResolution::ClearProfileOverride && affected.usage.path.is_none() {
            return Err(ApprovalError::NotAPropertyUsage(*index));
        }
    }
    Ok(ApprovedUpgrade { plan, resolutions })
}

/// 문서 편집 하나(profile override). host가 `CommandRequest`로 바꿉니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OverrideEdit {
    /// override 값 변경(`None` = 없음).
    Value {
        /// View index.
        view: usize,
        /// profile.
        profile: TargetProfileId,
        /// 범위.
        scope: OverrideScope,
        /// path.
        path: PropertyPath,
        /// 이전 값.
        before: Option<TypedValue>,
        /// 이후 값.
        after: Option<TypedValue>,
    },
    /// node 제외 여부 변경.
    Exclusion {
        /// View index.
        view: usize,
        /// profile.
        profile: TargetProfileId,
        /// node.
        node: StableId,
        /// 이후 제외 여부(이전은 반대).
        excluded: bool,
    },
}

impl OverrideEdit {
    /// 역 편집.
    pub fn inverse(&self) -> OverrideEdit {
        match self {
            OverrideEdit::Value {
                view,
                profile,
                scope,
                path,
                before,
                after,
            } => OverrideEdit::Value {
                view: *view,
                profile: profile.clone(),
                scope: *scope,
                path: path.clone(),
                before: after.clone(),
                after: before.clone(),
            },
            OverrideEdit::Exclusion {
                view,
                profile,
                node,
                excluded,
            } => OverrideEdit::Exclusion {
                view: *view,
                profile: profile.clone(),
                node: *node,
                excluded: !excluded,
            },
        }
    }
}

/// 원자 적용 단위(모듈 문서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeTransaction {
    /// 적용 기준 revision.
    pub base_revision: DocumentRevision,
    /// 문서 편집(순서대로).
    pub edits: Vec<OverrideEdit>,
    /// 적용 전 매니페스트 절.
    pub manifest_before: ProjectTargetProfiles,
    /// 적용 후 매니페스트 절.
    pub manifest_after: ProjectTargetProfiles,
}

impl UpgradeTransaction {
    /// `base`(적용 후 revision)를 기준으로 하는 역 transaction.
    pub fn inverse(&self, base: DocumentRevision) -> UpgradeTransaction {
        UpgradeTransaction {
            base_revision: base,
            edits: self.edits.iter().rev().map(OverrideEdit::inverse).collect(),
            manifest_before: self.manifest_after.clone(),
            manifest_after: self.manifest_before.clone(),
        }
    }
}

/// host 실행 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// 기준 revision이 현재와 다름.
    StaleRevision,
    /// 편집을 적용할 수 없음(이전 값 불일치·없는 View 등). 아무것도 바뀌지 않았습니다.
    Rejected {
        /// 편집 index.
        index: usize,
    },
    /// working tree 매니페스트 기록 실패. 문서도 되돌렸습니다.
    ManifestWrite,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::StaleRevision => f.write_str("기준 revision이 현재 문서와 다릅니다"),
            HostError::Rejected { index } => write!(f, "편집 #{index}를 적용할 수 없습니다"),
            HostError::ManifestWrite => f.write_str("stellar.yaml을 기록하지 못했습니다"),
        }
    }
}

impl std::error::Error for HostError {}

/// upgrade를 실행하는 host(문서 command 계층 + 프로젝트 working tree). commit 수단은 없습니다.
pub trait UpgradeHost: ProjectSnapshot {
    /// transaction을 원자 적용하고 새 revision을 돌려줍니다. 문서 편집은 하나의 `CommandTransaction`으로, 매니페스트는
    /// working tree `stellar.yaml` 기록으로 반영합니다. 실패하면 아무것도 바꾸지 않습니다.
    fn apply_transaction(
        &mut self,
        transaction: &UpgradeTransaction,
    ) -> Result<DocumentRevision, HostError>;
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedUpgrade {
    /// 적용한 transaction.
    pub transaction: UpgradeTransaction,
    /// 적용 후 revision.
    pub new_revision: DocumentRevision,
    /// undo transaction.
    pub undo: UpgradeTransaction,
}

/// 적용·undo 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// plan 이후 문서가 바뀜.
    StaleRevision,
    /// plan 이후 매니페스트 절이 바뀜.
    StaleManifest,
    /// override 지우기 대상 값이 profile override에 없음.
    NotAnOverride {
        /// 영향 index.
        index: usize,
    },
    /// 영향의 View index가 프로젝트에 없음.
    UnknownView {
        /// 영향 index.
        index: usize,
    },
    /// host 실행 실패(아무것도 바뀌지 않음).
    Host(HostError),
}

impl From<HostError> for ApplyError {
    fn from(error: HostError) -> ApplyError {
        ApplyError::Host(error)
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::StaleRevision => f.write_str("plan 이후 문서가 바뀌었습니다"),
            ApplyError::StaleManifest => {
                f.write_str("plan 이후 stellar.yaml profile 절이 바뀌었습니다")
            }
            ApplyError::NotAnOverride { index } => {
                write!(f, "영향 #{index}의 값은 profile override가 아닙니다")
            }
            ApplyError::UnknownView { index } => write!(f, "영향 #{index}의 View가 없습니다"),
            ApplyError::Host(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

/// 승인된 upgrade를 원자 적용합니다(자동 commit 없음).
pub fn apply_upgrade(
    host: &mut dyn UpgradeHost,
    approved: &ApprovedUpgrade,
) -> Result<AppliedUpgrade, ApplyError> {
    let plan = &approved.plan;
    if host.revision() != plan.base_revision {
        return Err(ApplyError::StaleRevision);
    }
    if host.target_profiles() != &plan.base_section {
        return Err(ApplyError::StaleManifest);
    }
    let mut edits = Vec::new();
    let mut excluded: BTreeSet<(usize, StableId)> = BTreeSet::new();
    for (index, resolution) in &approved.resolutions {
        let Some(affected) = plan.affected.get(*index) else {
            continue;
        };
        let view = host
            .views()
            .get(affected.view)
            .ok_or(ApplyError::UnknownView { index: *index })?;
        let overrides = view.overrides.profile(&plan.profile);
        match resolution {
            UsageResolution::Keep => {}
            UsageResolution::ExcludeNode => {
                let node = affected.usage.node;
                let already = overrides.is_some_and(|po| po.excluded_nodes().contains(&node));
                if !already && excluded.insert((affected.view, node)) {
                    edits.push(OverrideEdit::Exclusion {
                        view: affected.view,
                        profile: plan.profile.clone(),
                        node,
                        excluded: true,
                    });
                }
            }
            UsageResolution::ClearProfileOverride => {
                let path = affected
                    .usage
                    .path
                    .as_ref()
                    .ok_or(ApplyError::NotAnOverride { index: *index })?;
                let before = overrides
                    .and_then(|po| po.get(OverrideScope::Always, path))
                    .cloned()
                    .ok_or(ApplyError::NotAnOverride { index: *index })?;
                edits.push(OverrideEdit::Value {
                    view: affected.view,
                    profile: plan.profile.clone(),
                    scope: OverrideScope::Always,
                    path: path.clone(),
                    before: Some(before),
                    after: None,
                });
            }
        }
    }
    let transaction = UpgradeTransaction {
        base_revision: plan.base_revision,
        edits,
        manifest_before: plan.base_section.clone(),
        manifest_after: plan.upgraded_section(),
    };
    let new_revision = host.apply_transaction(&transaction)?;
    let undo = transaction.inverse(new_revision);
    Ok(AppliedUpgrade {
        transaction,
        new_revision,
        undo,
    })
}

/// 적용한 upgrade를 되돌립니다(적용 직후 상태일 때만). 새 revision을 돌려줍니다.
pub fn undo_upgrade(
    host: &mut dyn UpgradeHost,
    applied: &AppliedUpgrade,
) -> Result<DocumentRevision, ApplyError> {
    if host.revision() != applied.new_revision {
        return Err(ApplyError::StaleRevision);
    }
    if host.target_profiles() != &applied.transaction.manifest_after {
        return Err(ApplyError::StaleManifest);
    }
    Ok(host.apply_transaction(&applied.undo)?)
}

#[cfg(test)]
mod test_git {
    //! hermetic 시스템 Git fixture(사용자·시스템 설정을 섞지 않음). Git이 없으면 테스트가 실패하고,
    //! `STELLAR_ALLOW_MISSING_GIT=1`일 때만 Git 확인을 건너뜁니다.
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 시스템 Git이 없을 때 Git fixture 확인을 건너뛰도록 명시적으로 허용하는 환경 변수(dependencies crate와 같은 이름).
    pub const ALLOW_MISSING_GIT_ENV: &str = "STELLAR_ALLOW_MISSING_GIT";

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(prefix: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-target-profile-{prefix}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("임시 디렉터리");
            TempDir(std::fs::canonicalize(&path).expect("canonical 임시 경로"))
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub struct FixtureGit {
        git: PathBuf,
        home: PathBuf,
    }

    impl FixtureGit {
        pub fn find(home: &Path) -> Option<FixtureGit> {
            FixtureGit::locate(
                std::env::var_os("PATH"),
                std::env::var_os(ALLOW_MISSING_GIT_ENV).as_deref(),
                home,
            )
        }

        /// `path`(PATH 값)에서 git을 찾습니다. `allow_missing`은 [`ALLOW_MISSING_GIT_ENV`] 값입니다.
        ///
        /// git이 없으면 기본적으로 **panic**(테스트 실패)합니다. 검증 없이 통과하는 Git fixture 테스트를 만들지 않기
        /// 위해서입니다. `allow_missing`이 정확히 `1`일 때만 경고를 남기고 `None`을 돌려줘 Git 확인을 건너뜁니다.
        pub fn locate(
            path: Option<OsString>,
            allow_missing: Option<&OsStr>,
            home: &Path,
        ) -> Option<FixtureGit> {
            let git = path.and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("git"))
                    .find(|candidate| candidate.is_file())
            });
            let Some(git) = git else {
                if allow_missing == Some(OsStr::new("1")) {
                    eprintln!(
                        "시스템 git 없음 — {ALLOW_MISSING_GIT_ENV}=1 이므로 Git working tree 확인을 건너뜁니다"
                    );
                    return None;
                }
                panic!(
                    "Git working tree fixture에는 시스템 git이 필요합니다(PATH에 없음). \
                     건너뛰려면 {ALLOW_MISSING_GIT_ENV}=1을 설정하세요"
                );
            };
            Some(FixtureGit {
                git,
                home: home.to_path_buf(),
            })
        }

        pub fn ok(&self, cwd: &Path, args: &[&str]) -> String {
            let output = Command::new(&self.git)
                .args(args)
                .current_dir(cwd)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", &self.home)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .output()
                .expect("git 실행");
            assert!(output.status.success(), "fixture git 실패: {args:?}");
            String::from_utf8(output.stdout).expect("utf8 stdout")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_git::{FixtureGit, TempDir};
    use super::*;
    use crate::builtin::properties as p;
    use crate::capability::{CapabilityStatus, CapabilitySubject, CapabilityTable, ProfileLevel};
    use crate::capability::{StatusLadder, StatusStep};
    use crate::node::ManifestNode;
    use crate::overrides::VariantLayer;
    use crate::registry::{ProfileRegistry, RegistryCatalog};
    use crate::upgrade::fixture::{project, release_two, v, MemoryProject};
    use crate::upgrade::plan_profile_upgrade;
    use crate::validate::fixture::{id, sid, KeyMap, CLIPPED, KEY_STATUS_BAR, MODAL, ROOT};
    use crate::validate::{validate_view, CapabilityDiagnosticCode, ViewModel};
    use std::path::{Path, PathBuf};

    /// 테스트 전용 결정론 텍스트 기록기(제한 YAML writer 대용, 파일 bytes 비교용).
    fn emit(node: &ManifestNode, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        match node {
            ManifestNode::Scalar(text) => {
                out.push_str(&format!("{pad}{text:?}\n"));
            }
            ManifestNode::Sequence(items) => {
                for item in items {
                    out.push_str(&format!("{pad}-\n"));
                    emit(item, indent + 2, out);
                }
            }
            ManifestNode::Mapping(pairs) => {
                for (key, value) in pairs {
                    out.push_str(&format!("{pad}{key}:\n"));
                    emit(value, indent + 2, out);
                }
            }
        }
    }

    fn manifest_text(section: &ProjectTargetProfiles) -> String {
        let mut pairs = vec![("schema_version".to_string(), ManifestNode::scalar("1"))];
        pairs.extend(section.to_entries());
        pairs.push((
            "dependencies".to_string(),
            ManifestNode::Mapping(Vec::new()),
        ));
        let mut out = String::new();
        emit(&ManifestNode::Mapping(pairs), 0, &mut out);
        out
    }

    /// 메모리 문서 + working tree `stellar.yaml`. commit 수단이 없습니다.
    struct WorkingTreeHost {
        project: MemoryProject,
        manifest_path: PathBuf,
        fail_manifest_write: bool,
    }

    impl ProjectSnapshot for WorkingTreeHost {
        fn revision(&self) -> DocumentRevision {
            self.project.revision
        }
        fn target_profiles(&self) -> &ProjectTargetProfiles {
            &self.project.section
        }
        fn views(&self) -> &[ViewModel] {
            &self.project.views
        }
    }

    impl UpgradeHost for WorkingTreeHost {
        fn apply_transaction(
            &mut self,
            transaction: &UpgradeTransaction,
        ) -> Result<DocumentRevision, HostError> {
            if transaction.base_revision != self.project.revision {
                return Err(HostError::StaleRevision);
            }
            // 사본에 전부 적용한 뒤 교체(CommandTransaction의 전부 성공 또는 전부 롤백).
            let mut views = self.project.views.clone();
            for (index, edit) in transaction.edits.iter().enumerate() {
                let rejected = HostError::Rejected { index };
                match edit {
                    OverrideEdit::Value {
                        view,
                        profile,
                        scope,
                        path,
                        before,
                        after,
                    } => {
                        let po = views
                            .get_mut(*view)
                            .ok_or(rejected.clone())?
                            .overrides
                            .profile_mut(profile);
                        if po.get(*scope, path) != before.as_ref() {
                            return Err(rejected);
                        }
                        match after {
                            Some(value) => po.set(*scope, path.clone(), value.clone()),
                            None => po.clear(*scope, path),
                        };
                    }
                    OverrideEdit::Exclusion {
                        view,
                        profile,
                        node,
                        excluded,
                    } => {
                        let po = views
                            .get_mut(*view)
                            .ok_or(rejected.clone())?
                            .overrides
                            .profile_mut(profile);
                        let changed = if *excluded {
                            po.exclude_node(*node)
                        } else {
                            po.include_node(*node)
                        };
                        if !changed {
                            return Err(rejected);
                        }
                    }
                }
            }
            for view in &mut views {
                view.overrides.prune_empty();
            }
            if self.fail_manifest_write {
                return Err(HostError::ManifestWrite);
            }
            std::fs::write(
                &self.manifest_path,
                manifest_text(&transaction.manifest_after),
            )
            .map_err(|_| HostError::ManifestWrite)?;
            self.project.views = views;
            self.project.section = transaction.manifest_after.clone();
            self.project.revision = DocumentRevision(self.project.revision.0 + 1);
            Ok(self.project.revision)
        }
    }

    #[test]
    fn missing_git_fails_unless_explicitly_allowed() {
        use std::ffi::OsStr;
        let home = TempDir::new("no-git");
        let empty_bin = home.path().join("bin");
        std::fs::create_dir_all(&empty_bin).expect("빈 bin");
        let path = Some(empty_bin.into_os_string());
        // 명시적 허용(`1`)일 때만 건너뜁니다.
        assert!(FixtureGit::locate(path.clone(), Some(OsStr::new("1")), home.path()).is_none());
        // 기본값과 다른 값은 테스트 실패(검증 없이 통과하지 않음).
        for allow in [None, Some(OsStr::new("0")), Some(OsStr::new("yes"))] {
            let path = path.clone();
            let result =
                std::panic::catch_unwind(|| FixtureGit::locate(path, allow, home.path()).is_some());
            assert!(result.is_err(), "{allow:?}: git이 없는데 실패하지 않음");
        }
        // PATH 자체가 없어도 같은 규칙입니다.
        assert!(std::panic::catch_unwind(|| FixtureGit::locate(None, None, home.path())).is_err());
    }

    fn setup(dir: &Path) -> (WorkingTreeHost, Option<FixtureGit>) {
        let project = project();
        let manifest_path = dir.join("stellar.yaml");
        std::fs::write(&manifest_path, manifest_text(&project.section)).expect("manifest");
        let git = FixtureGit::find(dir);
        if let Some(git) = &git {
            // fixture 준비용 초기 commit(테스트 setup).
            git.ok(dir, &["init", "-q", "-b", "main"]);
            git.ok(dir, &["config", "user.name", "Fixture"]);
            git.ok(dir, &["config", "user.email", "fixture@example.com"]);
            git.ok(dir, &["add", "stellar.yaml"]);
            git.ok(dir, &["commit", "-q", "-m", "init"]);
        }
        (
            WorkingTreeHost {
                project,
                manifest_path,
                fail_manifest_write: false,
            },
            git,
        )
    }

    fn catalog() -> RegistryCatalog {
        crate::upgrade::fixture::catalog_v1_v2()
    }

    #[test]
    fn apply_and_undo_leave_only_working_tree_changes() {
        let dir = TempDir::new("apply");
        let (mut host, git) = setup(dir.path());
        let original = host.project.clone();
        let committed = std::fs::read(&host.manifest_path).expect("읽기");
        let head = git
            .as_ref()
            .map(|g| g.ok(dir.path(), &["rev-parse", "HEAD"]));

        let catalog = catalog();
        let plan = plan_profile_upgrade(
            &host,
            &KeyMap,
            &catalog,
            &id("android-35"),
            v(2),
            &VariantLayer::none(),
        )
        .expect("plan");
        // 영향: [0] modal backdrop(유지), [1] clipped clip rect(삭제된 capability → node 제외).
        assert_eq!(plan.affected[0].usage.node, sid(MODAL));
        assert_eq!(plan.affected[1].usage.node, sid(CLIPPED));
        let approved = approve(
            plan,
            vec![
                (1, UsageResolution::ExcludeNode),
                (0, UsageResolution::Keep),
            ],
        )
        .expect("승인");

        let applied = apply_upgrade(&mut host, &approved).expect("적용");
        assert_eq!(applied.new_revision, DocumentRevision(43));
        assert_eq!(
            applied.transaction.edits,
            vec![OverrideEdit::Exclusion {
                view: 0,
                profile: id("android-35"),
                node: sid(CLIPPED),
                excluded: true,
            }]
        );
        // 문서와 매니페스트가 함께 바뀌었습니다.
        let po = host.project.views[0]
            .overrides
            .profile(&id("android-35"))
            .expect("android override");
        assert!(po.excluded_nodes().contains(&sid(CLIPPED)));
        let pins: Vec<u32> = host
            .project
            .section
            .profiles()
            .iter()
            .map(|p| p.registry_version.get())
            .collect();
        assert_eq!(pins, vec![1, 1, 1, 2]);
        let written = std::fs::read(&host.manifest_path).expect("읽기");
        assert_ne!(written, committed);
        // Git: working tree 변경만 있고 commit은 없습니다.
        if let (Some(git), Some(head)) = (&git, &head) {
            assert_eq!(
                git.ok(dir.path(), &["status", "--porcelain=v1"]),
                " M stellar.yaml\n"
            );
            assert_eq!(&git.ok(dir.path(), &["rev-parse", "HEAD"]), head);
            assert_eq!(git.ok(dir.path(), &["rev-list", "--count", "HEAD"]), "1\n");
        }
        // 적용 후 검사는 새 release로: backdrop 근사, clipped node 제외.
        let report = validate_view(
            &host.project.views[0],
            &KeyMap,
            &host.project.section,
            &catalog,
            &VariantLayer::none(),
        );
        let android = report.profile(&id("android-35")).expect("android");
        assert!(android.findings().iter().all(|f| f.node != sid(CLIPPED)));
        assert!(android.findings().iter().any(|f| f.node == sid(MODAL)
            && f.subject == CapabilitySubject::Property(p::CSS_BACKDROP_FILTER)
            && f.code == CapabilityDiagnosticCode::Approximated));

        // undo: 문서·매니페스트·파일 bytes가 원래대로, Git working tree도 깨끗합니다.
        let revision = undo_upgrade(&mut host, &applied).expect("undo");
        assert_eq!(revision, DocumentRevision(44));
        assert_eq!(host.project.views, original.views);
        assert_eq!(host.project.section, original.section);
        assert_eq!(std::fs::read(&host.manifest_path).expect("읽기"), committed);
        if let (Some(git), Some(head)) = (&git, &head) {
            assert_eq!(git.ok(dir.path(), &["status", "--porcelain=v1"]), "");
            assert_eq!(&git.ok(dir.path(), &["rev-parse", "HEAD"]), head);
        }
        // 같은 undo를 다시 하거나 옛 plan을 다시 적용하면 stale입니다.
        assert_eq!(
            undo_upgrade(&mut host, &applied),
            Err(ApplyError::StaleRevision)
        );
        assert_eq!(
            apply_upgrade(&mut host, &approved).map(|a| a.new_revision),
            Err(ApplyError::StaleRevision)
        );
    }

    #[test]
    fn clear_profile_override_round_trips_and_failures_change_nothing() {
        let dir = TempDir::new("clear");
        let (mut host, _git) = setup(dir.path());
        // release 3: Android 35에서 status bar color가 Unsupported가 됩니다.
        let mut release_three_rules = release_two().capabilities().rules().to_vec();
        for rule in &mut release_three_rules {
            if rule.subject == CapabilitySubject::Property(p::ANDROID_STATUS_BAR_COLOR) {
                rule.android = StatusLadder::steps(vec![
                    StatusStep {
                        from: ProfileLevel::MIN,
                        status: CapabilityStatus::Native,
                    },
                    StatusStep {
                        from: ProfileLevel::android(35),
                        status: CapabilityStatus::Unsupported,
                    },
                ]);
            }
        }
        let release_three = ProfileRegistry::new(
            v(3),
            *release_two().defaults(),
            CapabilityTable::new(release_three_rules).expect("release 3"),
        );
        let catalog = RegistryCatalog::new(vec![
            crate::builtin::registry_v1().expect("release 1"),
            release_two(),
            release_three,
        ])
        .expect("catalog");
        let plan = plan_profile_upgrade(
            &host,
            &KeyMap,
            &catalog,
            &id("android-35"),
            v(3),
            &VariantLayer::none(),
        )
        .expect("plan");
        let status_bar = plan
            .affected
            .iter()
            .position(|a| {
                a.usage.subject == CapabilitySubject::Property(p::ANDROID_STATUS_BAR_COLOR)
            })
            .expect("status bar 영향");
        let element = plan.affected.iter().position(|a| a.usage.path.is_none());
        assert!(element.is_none(), "element 영향 없음");

        // 승인 오류.
        assert_eq!(
            approve(plan.clone(), vec![(99, UsageResolution::Keep)]),
            Err(ApprovalError::InvalidIndex(99))
        );
        assert_eq!(
            approve(
                plan.clone(),
                vec![
                    (0, UsageResolution::Keep),
                    (0, UsageResolution::ExcludeNode)
                ]
            ),
            Err(ApprovalError::DuplicateResolution(0))
        );
        // 공통값에서 온 속성(modal backdrop)은 override 지우기를 거부합니다.
        let backdrop = plan
            .affected
            .iter()
            .position(|a| a.usage.node == sid(MODAL))
            .expect("modal 영향");
        let not_override = approve(
            plan.clone(),
            vec![(backdrop, UsageResolution::ClearProfileOverride)],
        )
        .expect("승인");
        let before = host.project.clone();
        assert_eq!(
            apply_upgrade(&mut host, &not_override),
            Err(ApplyError::NotAnOverride { index: backdrop })
        );
        assert_eq!(host.project, before);

        // host 실패(매니페스트 기록 실패)는 문서도 바꾸지 않습니다.
        let approved = approve(
            plan.clone(),
            vec![(status_bar, UsageResolution::ClearProfileOverride)],
        )
        .expect("승인");
        host.fail_manifest_write = true;
        assert_eq!(
            apply_upgrade(&mut host, &approved),
            Err(ApplyError::Host(HostError::ManifestWrite))
        );
        assert_eq!(host.project, before);
        host.fail_manifest_write = false;

        // 매니페스트가 plan 이후 바뀌면 stale.
        let mut moved = host.project.section.clone();
        moved.set_default(id("ios")).expect("기본 profile");
        let saved = std::mem::replace(&mut host.project.section, moved);
        assert_eq!(
            apply_upgrade(&mut host, &approved),
            Err(ApplyError::StaleManifest)
        );
        host.project.section = saved;

        // override 지우기 적용 → undo로 값 복원.
        let applied = apply_upgrade(&mut host, &approved).expect("적용");
        let root_status = crate::validate::fixture::path(ROOT, KEY_STATUS_BAR);
        assert!(host.project.views[0]
            .overrides
            .profile(&id("android-35"))
            .and_then(|po| po.get(OverrideScope::Always, &root_status))
            .is_none());
        assert_eq!(
            applied.undo.edits,
            vec![applied.transaction.edits[0].inverse()]
        );
        undo_upgrade(&mut host, &applied).expect("undo");
        assert_eq!(host.project.views, before.views);
        assert_eq!(host.project.section, before.section);
    }
}
