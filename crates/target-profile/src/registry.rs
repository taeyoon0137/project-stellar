//! profile registry 버전과 프로젝트 pin 분리 (IX.10.4).
//!
//! - **registry**([`ProfileRegistry`]): Stellar 앱이 내장하는 profile·capability 정의의 한 release입니다. 버전
//!   ([`ProfileRegistryVersion`])마다 내용이 고정이며, 앱 업데이트는 새 버전을 **추가**할 뿐 기존 버전의 내용을 바꾸지
//!   않습니다. 앱은 여러 release를 [`RegistryCatalog`]로 함께 가집니다.
//! - **project pin**: 각 `target_profiles[i]`의 `registry_version`([`TargetProfile::registry_version`])입니다. 프로젝트
//!   파일에 기록되며 사용자가 승인한 upgrade(IX.10.9·IX.10.10)로만 바뀝니다.
//!
//! [`RegistryCatalog::resolve`]는 항상 pin한 release로 profile을 해석합니다. 최신 release로 대체(fallback)하지 않고,
//! 앱에 pin한 release가 없으면(더 새 앱에서 만든 프로젝트) [`ResolveError::RegistryVersionUnavailable`]로 거부합니다.
//! 해석은 프로젝트를 읽기만 하므로 앱 업데이트만으로 프로젝트 bytes와 해석 결과가 바뀌지 않습니다. 앱 최신 release는
//! **새로 만드는** profile의 기본값과 pin에만 쓰입니다([`RegistryCatalog::new_profile`]).

use std::fmt;

use crate::capability::CapabilityTable;
use crate::node::{parse_u32, ValueError};
use crate::profile::TargetProfileId;
use crate::spec::{
    AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, CapabilitySetVersion,
    DottedVersion, ProfileSpec, TargetProfile, WebBaseline, WebProfile,
};

/// profile entry 키: registry pin.
pub const KEY_REGISTRY_VERSION: &str = "registry_version";

/// profile registry release 버전(1 이상). 정렬은 release 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileRegistryVersion(u32);

impl ProfileRegistryVersion {
    /// 범위(1 이상)를 검사해 만듭니다.
    pub const fn new(value: u32) -> Result<ProfileRegistryVersion, ValueError> {
        if value == 0 {
            Err(ValueError::OutOfRange)
        } else {
            Ok(ProfileRegistryVersion(value))
        }
    }

    /// 숫자 값.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// 10진 텍스트를 엄격히 읽습니다.
    pub fn parse(text: &str) -> Result<ProfileRegistryVersion, ValueError> {
        ProfileRegistryVersion::new(parse_u32(text)?)
    }
}

impl fmt::Display for ProfileRegistryVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// release가 새 profile에 쓰는 기본 필드 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NewProfileDefaults {
    /// Apple 최소 OS 버전(모든 Apple 플랫폼 공통).
    pub apple_minimum_os: DottedVersion,
    /// Android API level.
    pub android_api_level: AndroidApiLevel,
    /// Android UI 시스템 버전.
    pub android_ui_system_version: DottedVersion,
    /// Web 기준선.
    pub web_baseline: WebBaseline,
    /// Web capability 집합 버전.
    pub web_capability_set_version: CapabilitySetVersion,
}

/// 새 profile 종류 선택.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewProfileKind {
    /// Web.
    Web,
    /// Apple(대상 플랫폼).
    Apple(ApplePlatform),
    /// Android.
    Android,
}

/// profile registry release 하나: 새 profile 기본값과 capability 표(IX.10.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRegistry {
    version: ProfileRegistryVersion,
    defaults: NewProfileDefaults,
    capabilities: CapabilityTable,
}

impl ProfileRegistry {
    /// release를 만듭니다.
    pub fn new(
        version: ProfileRegistryVersion,
        defaults: NewProfileDefaults,
        capabilities: CapabilityTable,
    ) -> ProfileRegistry {
        ProfileRegistry {
            version,
            defaults,
            capabilities,
        }
    }

    /// capability 표.
    pub fn capabilities(&self) -> &CapabilityTable {
        &self.capabilities
    }

    /// 버전.
    pub fn version(&self) -> ProfileRegistryVersion {
        self.version
    }

    /// 새 profile 기본값.
    pub fn defaults(&self) -> &NewProfileDefaults {
        &self.defaults
    }
}

/// catalog 구성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    /// release가 하나도 없음.
    Empty,
    /// 같은 버전이 두 번.
    DuplicateVersion(ProfileRegistryVersion),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::Empty => f.write_str("profile registry release가 없습니다"),
            CatalogError::DuplicateVersion(version) => {
                write!(f, "profile registry 버전 {version}이 중복됩니다")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// profile 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// 프로젝트가 pin한 registry 버전이 이 앱에 없음(최신으로 대체하지 않음).
    RegistryVersionUnavailable {
        /// profile.
        profile: TargetProfileId,
        /// 프로젝트 pin.
        pinned: ProfileRegistryVersion,
        /// 이 앱의 최신 release.
        latest: ProfileRegistryVersion,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::RegistryVersionUnavailable {
                profile,
                pinned,
                latest,
            } => write!(
                f,
                "profile {profile}이 고정한 registry 버전 {pinned}을 이 앱이 알지 못합니다(최신 {latest})"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// pin한 release로 해석한 profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTargetProfile<'c> {
    profile: TargetProfile,
    registry: &'c ProfileRegistry,
}

impl<'c> ResolvedTargetProfile<'c> {
    /// 프로젝트의 profile entry(그대로).
    pub fn profile(&self) -> &TargetProfile {
        &self.profile
    }

    /// pin한 registry release.
    pub fn registry(&self) -> &'c ProfileRegistry {
        self.registry
    }
}

/// 앱이 내장한 registry release 모음(버전 오름차순).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCatalog {
    releases: Vec<ProfileRegistry>,
}

impl RegistryCatalog {
    /// release 목록으로 만듭니다(순서 무관, 버전 중복·빈 목록 거부).
    pub fn new(mut releases: Vec<ProfileRegistry>) -> Result<RegistryCatalog, CatalogError> {
        releases.sort_by_key(ProfileRegistry::version);
        if releases.is_empty() {
            return Err(CatalogError::Empty);
        }
        if let Some(pair) = releases
            .windows(2)
            .find(|pair| pair[0].version == pair[1].version)
        {
            return Err(CatalogError::DuplicateVersion(pair[1].version));
        }
        Ok(RegistryCatalog { releases })
    }

    /// 최신 release.
    pub fn latest(&self) -> &ProfileRegistry {
        // 생성자가 빈 목록을 거부하므로 항상 있습니다.
        &self.releases[self.releases.len() - 1]
    }

    /// 버전의 release.
    pub fn get(&self, version: ProfileRegistryVersion) -> Option<&ProfileRegistry> {
        self.releases
            .binary_search_by_key(&version, ProfileRegistry::version)
            .ok()
            .map(|index| &self.releases[index])
    }

    /// 모든 release(버전 오름차순).
    pub fn releases(&self) -> &[ProfileRegistry] {
        &self.releases
    }

    /// profile을 pin한 release로 해석합니다. profile은 바꾸지 않습니다.
    pub fn resolve(
        &self,
        profile: &TargetProfile,
    ) -> Result<ResolvedTargetProfile<'_>, ResolveError> {
        let registry = self.get(profile.registry_version).ok_or_else(|| {
            ResolveError::RegistryVersionUnavailable {
                profile: profile.id.clone(),
                pinned: profile.registry_version,
                latest: self.latest().version,
            }
        })?;
        Ok(ResolvedTargetProfile {
            profile: profile.clone(),
            registry,
        })
    }

    /// 최신 release의 기본값으로 새 profile을 만들고 최신 버전으로 pin합니다.
    pub fn new_profile(&self, id: TargetProfileId, kind: NewProfileKind) -> TargetProfile {
        let latest = self.latest();
        let d = latest.defaults;
        let spec = match kind {
            NewProfileKind::Web => ProfileSpec::Web(WebProfile {
                baseline: d.web_baseline,
                capability_set_version: d.web_capability_set_version,
            }),
            NewProfileKind::Apple(platform) => ProfileSpec::Apple(AppleProfile {
                platform,
                minimum_os: d.apple_minimum_os,
            }),
            NewProfileKind::Android => ProfileSpec::Android(AndroidProfile {
                api_level: d.android_api_level,
                ui_system_version: d.android_ui_system_version,
            }),
        };
        TargetProfile::new(id, latest.version, spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProjectTargetProfiles;
    use crate::node::ManifestNode;

    fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("유효한 id")
    }

    fn version(n: u32) -> ProfileRegistryVersion {
        ProfileRegistryVersion::new(n).expect("버전")
    }

    fn release(n: u32, apple: u16, api: u16, baseline: u16) -> ProfileRegistry {
        ProfileRegistry::new(
            version(n),
            NewProfileDefaults {
                apple_minimum_os: DottedVersion::new(apple, 0, 0),
                android_api_level: AndroidApiLevel::new(api).expect("api"),
                android_ui_system_version: DottedVersion::new(1, u16::try_from(n).expect("n"), 0),
                web_baseline: WebBaseline::new(baseline).expect("baseline"),
                web_capability_set_version: CapabilitySetVersion::new(n).expect("set"),
            },
            CapabilityTable::empty(),
        )
    }

    fn root(section: &ProjectTargetProfiles) -> ManifestNode {
        let mut pairs = vec![("schema_version".to_string(), ManifestNode::scalar("1"))];
        pairs.extend(section.to_entries());
        pairs.push((
            "dependencies".to_string(),
            ManifestNode::Mapping(Vec::new()),
        ));
        ManifestNode::Mapping(pairs)
    }

    #[test]
    fn app_update_keeps_project_bytes_and_resolved_profiles() {
        let old_app = RegistryCatalog::new(vec![release(1, 26, 35, 2024)]).expect("catalog");
        // 앱 업데이트: 새 release를 추가하고(기본값도 더 새로움) 기존 release는 그대로 둡니다.
        let new_app =
            RegistryCatalog::new(vec![release(2, 27, 36, 2025), release(1, 26, 35, 2024)])
                .expect("catalog");
        assert_eq!(new_app.latest().version(), version(2));

        // 이전 앱에서 만든 프로젝트.
        let section = ProjectTargetProfiles::new(
            vec![
                old_app.new_profile(id("apple-phone"), NewProfileKind::Apple(ApplePlatform::Ios)),
                old_app.new_profile(id("android-phone"), NewProfileKind::Android),
                old_app.new_profile(id("web-modern"), NewProfileKind::Web),
            ],
            Some(id("apple-phone")),
        )
        .expect("절");
        let bytes_before = root(&section).canonical_bytes();

        // 새 앱으로 열기: 읽기 → 해석 → 다시 기록.
        let opened = ProjectTargetProfiles::from_root(&root(&section)).expect("열기");
        for profile in opened.profiles() {
            assert_eq!(profile.registry_version, version(1), "pin 유지");
            let resolved_new = new_app.resolve(profile).expect("새 앱 해석");
            let resolved_old = old_app.resolve(profile).expect("이전 앱 해석");
            assert_eq!(resolved_new, resolved_old, "해석 결과 동일");
            assert_eq!(resolved_new.registry().version(), version(1));
            assert_eq!(resolved_new.profile(), profile);
        }
        let mut rewritten = root(&section);
        opened.write_into_root(&mut rewritten).expect("기록");
        assert_eq!(
            rewritten.canonical_bytes(),
            bytes_before,
            "프로젝트 bytes 동일"
        );

        // 새 앱 최신 release는 새로 만드는 profile에만 쓰입니다.
        let created =
            new_app.new_profile(id("apple-mac"), NewProfileKind::Apple(ApplePlatform::MacOs));
        assert_eq!(created.registry_version, version(2));
        assert_eq!(
            created.spec,
            ProfileSpec::Apple(AppleProfile {
                platform: ApplePlatform::MacOs,
                minimum_os: DottedVersion::new(27, 0, 0),
            })
        );
        assert_eq!(
            opened.get(&id("apple-phone")).map(|p| p.spec),
            Some(ProfileSpec::Apple(AppleProfile {
                platform: ApplePlatform::Ios,
                minimum_os: DottedVersion::new(26, 0, 0),
            }))
        );
    }

    #[test]
    fn pinned_release_missing_from_app_is_rejected_not_replaced() {
        let old_app = RegistryCatalog::new(vec![release(1, 26, 35, 2024)]).expect("catalog");
        let new_app =
            RegistryCatalog::new(vec![release(1, 26, 35, 2024), release(2, 27, 36, 2025)])
                .expect("catalog");
        let newer = new_app.new_profile(id("web"), NewProfileKind::Web);
        assert_eq!(
            old_app.resolve(&newer),
            Err(ResolveError::RegistryVersionUnavailable {
                profile: id("web"),
                pinned: version(2),
                latest: version(1),
            })
        );
        // 중간 버전이 빠진 catalog에서도 다른 버전으로 대체하지 않습니다.
        let gap = RegistryCatalog::new(vec![release(1, 26, 35, 2024), release(3, 28, 37, 2026)])
            .expect("catalog");
        assert!(matches!(
            gap.resolve(&newer),
            Err(ResolveError::RegistryVersionUnavailable { pinned, .. }) if pinned == version(2)
        ));
    }

    #[test]
    fn catalog_rules() {
        assert_eq!(RegistryCatalog::new(Vec::new()), Err(CatalogError::Empty));
        assert_eq!(
            RegistryCatalog::new(vec![release(1, 26, 35, 2024), release(1, 27, 36, 2025)]),
            Err(CatalogError::DuplicateVersion(version(1)))
        );
        let catalog = RegistryCatalog::new(vec![release(3, 1, 1, 2024), release(1, 1, 1, 2024)])
            .expect("catalog");
        let versions: Vec<u32> = catalog
            .releases()
            .iter()
            .map(|r| r.version().get())
            .collect();
        assert_eq!(versions, vec![1, 3]);
        assert!(catalog.get(version(2)).is_none());
        assert_eq!(ProfileRegistryVersion::new(0), Err(ValueError::OutOfRange));
        assert_eq!(ProfileRegistryVersion::parse("12"), Ok(version(12)));
        assert_eq!(
            ProfileRegistryVersion::parse("012"),
            Err(ValueError::InvalidInteger)
        );
    }
}
