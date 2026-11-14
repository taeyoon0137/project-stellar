//! profile upgrade plan: old/new capability registry의 semantic diff와 영향받는 node·속성 (IX.10.9).
//!
//! [`plan_profile_upgrade`]는 profile 하나의 registry pin을 `from`(현재 pin)에서 `to`로 올릴 때의 영향을 미리 계산합니다.
//! 프로젝트는 읽기 전용 [`ProjectSnapshot`]으로만 받으므로 plan 계산은 문서 revision·매니페스트·View를 바꿀 수 없습니다.
//! 적용은 사용자가 plan을 승인한 뒤 IX.10.10이 합니다.
//!
//! **semantic diff**([`diff_registries`])는 이 profile에서 실제로 의미가 바뀌는 항목만 담습니다(subject 오름차순).
//!
//! | 종류 | code | 조건 |
//! |---|---|---|
//! | [`CapabilityChangeKind::AddCapability`] | 1 | new release에만 subject가 있음 |
//! | [`CapabilityChangeKind::RemoveCapability`] | 2 | old release에만 subject가 있음 |
//! | [`CapabilityChangeKind::ChangeStatus`] | 5 | 두 release에 있고 이 profile의 상태 또는 Deprecated 대체 subject가 다름 |
//!
//! code는 XIV.3.2 `SemanticChangeKind`의 AddObject(1)·RemoveObject(2)·SetProperty(5)와 같은 번호입니다(crate graph상
//! semantic-diff crate를 쓸 수 없어 지역 enum으로 둠). 다른 profile에만 영향이 있는 규칙 변경은 이 profile의 diff에 넣지
//! 않습니다.
//!
//! **영향받는 사용**([`AffectedUsage`])은 프로젝트 모든 View에서 이 profile이 쓰는 capability(IX.10.8
//! [`collect_usages`]) 중 diff에 있는 subject입니다(View 순서 → 사용 순서).

use std::fmt;

use stellar_core_types::revision::DocumentRevision;

use crate::capability::{CapabilityStatus, CapabilitySubject, CapabilityTable};
use crate::manifest::ProjectTargetProfiles;
use crate::overrides::VariantLayer;
use crate::profile::TargetProfileId;
use crate::registry::{ProfileRegistryVersion, RegistryCatalog};
use crate::spec::TargetProfile;
use crate::validate::{collect_usages, CapabilityUsage, PropertyCapabilityMap, ViewModel};

/// plan 계산이 읽는 프로젝트(읽기 전용).
pub trait ProjectSnapshot {
    /// 현재 문서 revision.
    fn revision(&self) -> DocumentRevision;
    /// 매니페스트 Target Profile 절.
    fn target_profiles(&self) -> &ProjectTargetProfiles;
    /// 프로젝트의 View(결정론적 순서).
    fn views(&self) -> &[ViewModel];
}

/// registry 변경 종류(XIV.3.2 code와 같은 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityChangeKind {
    /// subject 추가(AddObject).
    AddCapability = 1,
    /// subject 삭제(RemoveObject).
    RemoveCapability = 2,
    /// 상태·대체 subject 변경(SetProperty).
    ChangeStatus = 5,
}

impl CapabilityChangeKind {
    /// 안정 numeric code.
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// 이 profile에서의 registry 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityChange {
    /// 종류.
    pub kind: CapabilityChangeKind,
    /// 대상.
    pub subject: CapabilitySubject,
    /// old release의 상태(없으면 `None`).
    pub old_status: Option<CapabilityStatus>,
    /// new release의 상태(없으면 `None`).
    pub new_status: Option<CapabilityStatus>,
    /// new release에서 Deprecated일 때 대체 subject.
    pub new_replacement: Option<CapabilitySubject>,
}

/// 영향받는 사용 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AffectedUsage {
    /// View index([`ProjectSnapshot::views`] 순서).
    pub view: usize,
    /// 사용(node·subject·path).
    pub usage: CapabilityUsage,
    /// 변경 종류.
    pub change: CapabilityChangeKind,
    /// old 상태.
    pub old_status: Option<CapabilityStatus>,
    /// new 상태.
    pub new_status: Option<CapabilityStatus>,
}

/// profile upgrade plan(미리보기).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePlan {
    /// profile.
    pub profile: TargetProfileId,
    /// 현재 pin.
    pub from: ProfileRegistryVersion,
    /// 목표 release.
    pub to: ProfileRegistryVersion,
    /// plan을 계산한 문서 revision(적용 시 stale 검사).
    pub base_revision: DocumentRevision,
    /// plan을 계산한 매니페스트 절(적용 시 stale 검사).
    pub base_section: ProjectTargetProfiles,
    /// 이 profile의 registry semantic diff.
    pub registry_changes: Vec<CapabilityChange>,
    /// 영향받는 node·속성.
    pub affected: Vec<AffectedUsage>,
}

impl UpgradePlan {
    /// pin을 `to`로 바꾼 매니페스트 절(기준 절 기준, 다른 profile·기본 profile 보존).
    pub fn upgraded_section(&self) -> ProjectTargetProfiles {
        let mut section = self.base_section.clone();
        if let Some(profile) = section.get(&self.profile).cloned() {
            let mut upgraded = profile;
            upgraded.registry_version = self.to;
            section.replace_profile(upgraded);
        }
        section
    }
}

/// plan 계산 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradePlanError {
    /// 프로젝트에 없는 profile.
    UnknownProfile(TargetProfileId),
    /// 목표가 현재 pin보다 새 release가 아님.
    NotAnUpgrade {
        /// 현재 pin.
        pinned: ProfileRegistryVersion,
        /// 목표.
        target: ProfileRegistryVersion,
    },
    /// 앱에 없는 release(현재 pin 또는 목표).
    ReleaseUnavailable(ProfileRegistryVersion),
}

impl fmt::Display for UpgradePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpgradePlanError::UnknownProfile(id) => write!(f, "프로젝트에 없는 profile: {id}"),
            UpgradePlanError::NotAnUpgrade { pinned, target } => write!(
                f,
                "목표 registry 버전 {target}이 현재 pin {pinned}보다 새 버전이 아닙니다"
            ),
            UpgradePlanError::ReleaseUnavailable(version) => {
                write!(f, "앱에 registry 버전 {version}이 없습니다")
            }
        }
    }
}

impl std::error::Error for UpgradePlanError {}

/// 이 profile에서 old → new release의 semantic diff(subject 오름차순, 모듈 문서 표).
pub fn diff_registries(
    old: &CapabilityTable,
    new: &CapabilityTable,
    profile: &TargetProfile,
) -> Vec<CapabilityChange> {
    let replacement = |table: &CapabilityTable, subject| match table.status(profile, subject) {
        Some(CapabilityStatus::Deprecated) => table.rule(subject).and_then(|r| r.replacement),
        _ => None,
    };
    let (old_rules, new_rules) = (old.rules(), new.rules());
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = Vec::new();
    while i < old_rules.len() || j < new_rules.len() {
        let old_subject = old_rules.get(i).map(|r| r.subject);
        let new_subject = new_rules.get(j).map(|r| r.subject);
        let (subject, kind) = match (old_subject, new_subject) {
            (Some(a), Some(b)) if a == b => {
                i += 1;
                j += 1;
                (a, CapabilityChangeKind::ChangeStatus)
            }
            (Some(a), Some(b)) if a < b => {
                i += 1;
                (a, CapabilityChangeKind::RemoveCapability)
            }
            (Some(a), None) => {
                i += 1;
                (a, CapabilityChangeKind::RemoveCapability)
            }
            (_, Some(b)) => {
                j += 1;
                (b, CapabilityChangeKind::AddCapability)
            }
            (None, None) => break,
        };
        let old_status = old.status(profile, subject);
        let new_status = new.status(profile, subject);
        let new_replacement = replacement(new, subject);
        if kind == CapabilityChangeKind::ChangeStatus
            && old_status == new_status
            && replacement(old, subject) == new_replacement
        {
            continue;
        }
        changes.push(CapabilityChange {
            kind,
            subject,
            old_status,
            new_status,
            new_replacement,
        });
    }
    changes
}

/// profile 하나의 upgrade plan을 계산합니다. 프로젝트는 읽기만 합니다.
pub fn plan_profile_upgrade(
    project: &dyn ProjectSnapshot,
    map: &dyn PropertyCapabilityMap,
    catalog: &RegistryCatalog,
    profile_id: &TargetProfileId,
    target: ProfileRegistryVersion,
    variants: &VariantLayer,
) -> Result<UpgradePlan, UpgradePlanError> {
    let section = project.target_profiles();
    let profile = section
        .get(profile_id)
        .ok_or_else(|| UpgradePlanError::UnknownProfile(profile_id.clone()))?;
    let pinned = profile.registry_version;
    if target <= pinned {
        return Err(UpgradePlanError::NotAnUpgrade { pinned, target });
    }
    let old = catalog
        .get(pinned)
        .ok_or(UpgradePlanError::ReleaseUnavailable(pinned))?;
    let new = catalog
        .get(target)
        .ok_or(UpgradePlanError::ReleaseUnavailable(target))?;
    let registry_changes = diff_registries(old.capabilities(), new.capabilities(), profile);
    let mut affected = Vec::new();
    for (index, view) in project.views().iter().enumerate() {
        for usage in collect_usages(view, map, profile, variants) {
            if let Some(change) = registry_changes.iter().find(|c| c.subject == usage.subject) {
                affected.push(AffectedUsage {
                    view: index,
                    usage,
                    change: change.kind,
                    old_status: change.old_status,
                    new_status: change.new_status,
                });
            }
        }
    }
    Ok(UpgradePlan {
        profile: profile_id.clone(),
        from: pinned,
        to: target,
        base_revision: project.revision(),
        base_section: section.clone(),
        registry_changes,
        affected,
    })
}

#[cfg(test)]
pub(crate) mod fixture {
    //! IX.10.9~IX.10.10 공용: release 1 + 변경한 release 2 catalog, 메모리 프로젝트.
    use super::*;
    use crate::builtin::{self, properties as p};
    use crate::capability::{CapabilityPropertyId, CapabilityRule, StatusLadder};
    use crate::registry::ProfileRegistry;

    /// release 2에 새로 추가한 속성.
    pub const CSS_ANCHOR_POSITIONING: CapabilityPropertyId = CapabilityPropertyId(0x0001_0005);

    pub fn v(n: u32) -> ProfileRegistryVersion {
        ProfileRegistryVersion::new(n).expect("버전")
    }

    /// release 2: Android backdrop 근사 가능, Apple subgrid 근사 가능, `css.clip_rect` 삭제, anchor positioning 추가.
    pub fn release_two() -> ProfileRegistry {
        let mut rules = builtin::rules_v1();
        rules.retain(|rule| rule.subject != CapabilitySubject::Property(p::CSS_CLIP_RECT));
        for rule in &mut rules {
            if rule.subject == CapabilitySubject::Property(p::CSS_BACKDROP_FILTER) {
                rule.android = StatusLadder::constant(CapabilityStatus::Approximated);
            }
            if rule.subject == CapabilitySubject::Property(p::CSS_SUBGRID) {
                rule.apple = StatusLadder::constant(CapabilityStatus::Approximated);
            }
        }
        rules.push(CapabilityRule::uniform(
            CapabilitySubject::Property(CSS_ANCHOR_POSITIONING),
            CapabilityStatus::Unsupported,
        ));
        ProfileRegistry::new(
            v(2),
            builtin::defaults_v1().expect("기본값"),
            CapabilityTable::new(rules).expect("release 2 표"),
        )
    }

    pub fn catalog_v1_v2() -> RegistryCatalog {
        RegistryCatalog::new(vec![
            builtin::registry_v1().expect("release 1"),
            release_two(),
        ])
        .expect("catalog")
    }

    /// 메모리 프로젝트.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MemoryProject {
        pub revision: DocumentRevision,
        pub section: ProjectTargetProfiles,
        pub views: Vec<ViewModel>,
    }

    impl ProjectSnapshot for MemoryProject {
        fn revision(&self) -> DocumentRevision {
            self.revision
        }
        fn target_profiles(&self) -> &ProjectTargetProfiles {
            &self.section
        }
        fn views(&self) -> &[ViewModel] {
            &self.views
        }
    }

    pub fn project() -> MemoryProject {
        MemoryProject {
            revision: DocumentRevision(42),
            section: crate::validate::fixture::section(),
            views: vec![crate::validate::fixture::view()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::builtin::properties as p;
    use crate::manifest::KEY_TARGET_PROFILES;
    use crate::node::ManifestNode;
    use crate::validate::fixture::{
        id, sid, KeyMap, CLIPPED, GRID, KEY_BACKDROP, KEY_CLIP_RECT, KEY_SUBGRID, MODAL,
    };
    use CapabilityChangeKind::{AddCapability, ChangeStatus, RemoveCapability};
    use CapabilityStatus::{Approximated as A, Unsupported as U};
    use CapabilitySubject::Property as P;

    fn section_bytes(section: &ProjectTargetProfiles) -> Vec<u8> {
        ManifestNode::Mapping(section.to_entries()).canonical_bytes()
    }

    fn affected_rows(
        plan: &UpgradePlan,
    ) -> Vec<(u8, CapabilitySubject, Option<u32>, CapabilityChangeKind)> {
        plan.affected
            .iter()
            .map(|a| {
                (
                    a.usage.node.to_bytes()[15],
                    a.usage.subject,
                    a.usage.path.as_ref().map(|path| path.keys()[0].0),
                    a.change,
                )
            })
            .collect()
    }

    #[test]
    fn upgrade_preview_leaves_project_unchanged() {
        let project = project();
        let before = project.clone();
        let bytes_before = section_bytes(&project.section);
        let catalog = catalog_v1_v2();

        let android = plan_profile_upgrade(
            &project,
            &KeyMap,
            &catalog,
            &id("android-35"),
            v(2),
            &VariantLayer::none(),
        )
        .expect("plan");
        // 미리보기 후 revision·매니페스트 bytes·View 모두 그대로입니다.
        assert_eq!(project.revision, DocumentRevision(42));
        assert_eq!(section_bytes(&project.section), bytes_before);
        assert_eq!(project, before);
        assert_eq!(android.base_revision, DocumentRevision(42));
        assert_eq!((android.from, android.to), (v(1), v(2)));

        // Android 35의 semantic diff: backdrop 상태 변경, clip rect 삭제, anchor 추가(subject 순).
        assert_eq!(
            android.registry_changes,
            vec![
                CapabilityChange {
                    kind: ChangeStatus,
                    subject: P(p::CSS_BACKDROP_FILTER),
                    old_status: Some(U),
                    new_status: Some(A),
                    new_replacement: None,
                },
                CapabilityChange {
                    kind: RemoveCapability,
                    subject: P(p::CSS_CLIP_RECT),
                    old_status: Some(A),
                    new_status: None,
                    new_replacement: None,
                },
                CapabilityChange {
                    kind: AddCapability,
                    subject: P(CSS_ANCHOR_POSITIONING),
                    old_status: None,
                    new_status: Some(U),
                    new_replacement: None,
                },
            ]
        );
        // Apple 전용 subgrid 변경은 Android diff에 없습니다.
        assert!(android
            .registry_changes
            .iter()
            .all(|c| c.subject != P(p::CSS_SUBGRID)));
        assert_eq!(
            affected_rows(&android),
            vec![
                (
                    MODAL,
                    P(p::CSS_BACKDROP_FILTER),
                    Some(KEY_BACKDROP),
                    ChangeStatus
                ),
                (
                    CLIPPED,
                    P(p::CSS_CLIP_RECT),
                    Some(KEY_CLIP_RECT),
                    RemoveCapability
                ),
            ]
        );

        // iOS: subgrid 상태 변경(grid 영향), backdrop은 iOS에서 변화 없음.
        let ios = plan_profile_upgrade(
            &project,
            &KeyMap,
            &catalog,
            &id("ios"),
            v(2),
            &VariantLayer::none(),
        )
        .expect("plan");
        assert_eq!(
            ios.registry_changes
                .iter()
                .map(|c| (c.kind, c.subject))
                .collect::<Vec<_>>(),
            vec![
                (ChangeStatus, P(p::CSS_SUBGRID)),
                (RemoveCapability, P(p::CSS_CLIP_RECT)),
                (AddCapability, P(CSS_ANCHOR_POSITIONING)),
            ]
        );
        assert_eq!(
            affected_rows(&ios),
            vec![
                (GRID, P(p::CSS_SUBGRID), Some(KEY_SUBGRID), ChangeStatus),
                (
                    CLIPPED,
                    P(p::CSS_CLIP_RECT),
                    Some(KEY_CLIP_RECT),
                    RemoveCapability
                ),
            ]
        );
        assert_eq!(project, before);

        // 승인 전 계산한 절: 대상 profile pin만 2, 나머지·기본 profile 보존.
        let upgraded = android.upgraded_section();
        let pins: Vec<u32> = upgraded
            .profiles()
            .iter()
            .map(|p| p.registry_version.get())
            .collect();
        assert_eq!(pins, vec![1, 1, 1, 2]);
        assert_eq!(upgraded.default_id(), project.section.default_id());
        assert_eq!(section_bytes(&project.section), bytes_before);
        assert!(ManifestNode::Mapping(upgraded.to_entries())
            .get(KEY_TARGET_PROFILES)
            .is_some());
        assert_eq!(sid(MODAL), android.affected[0].usage.node);
    }

    #[test]
    fn plan_errors() {
        let project = project();
        let catalog = catalog_v1_v2();
        let plan = |profile: &str, target: u32| {
            plan_profile_upgrade(
                &project,
                &KeyMap,
                &catalog,
                &id(profile),
                v(target),
                &VariantLayer::none(),
            )
        };
        assert_eq!(
            plan("missing", 2),
            Err(UpgradePlanError::UnknownProfile(id("missing")))
        );
        assert_eq!(
            plan("web", 1),
            Err(UpgradePlanError::NotAnUpgrade {
                pinned: v(1),
                target: v(1)
            })
        );
        assert_eq!(
            plan("web", 3),
            Err(UpgradePlanError::ReleaseUnavailable(v(3)))
        );
        // 같은 release끼리의 diff는 비어 있습니다.
        let release = catalog.get(v(1)).expect("release 1");
        let profile = project.section.get(&id("web")).expect("web");
        assert!(
            diff_registries(release.capabilities(), release.capabilities(), profile).is_empty()
        );
        assert_eq!(
            [AddCapability, RemoveCapability, ChangeStatus].map(CapabilityChangeKind::code),
            [1, 2, 5]
        );
    }
}
