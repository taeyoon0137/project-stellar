//! 선택 profile의 Binding 환경 입력 (IX.10.7).
//!
//! 표현식 환경 key registry(VIII.7.1)의 profile 관련 key 세 개에 값을 줍니다. crate graph상 expressions crate를 쓸 수
//! 없어 numeric key와 enum 타입 번호를 지역 상수로 두며, 값은 VIII.7.1 표와 같아야 합니다.
//!
//! | key | 이름 | 값 |
//! |---|---|---|
//! | 1 | `platform` | Enum(`0xE000_0001`, Web=1·Apple=2·Android=3, profile 없음=generic 0) |
//! | 2 | `os_version` | Number — Apple은 `minimum_os`의 `major × 1000 + minor`(예: `17.4` → 17004), 그 밖은 0 |
//! | 3 | `api_level` | Number — Android API level, 그 밖은 0 |
//!
//! **`os_version` 인코딩**: `major.minor`를 10진 소수로 읽으면 `17.1`과 `17.10`이 같은 값이 되고 `10.15 < 10.9`처럼
//! 순서가 뒤집힙니다. 그래서 정수 `major × 1000 + minor`로 둡니다(patch는 넣지 않음). major `MAX_OS_MAJOR`·minor
//! `MAX_OS_MINOR` 이하에서 서로 다른 버전은 서로 다른 값이고 수 순서가 버전 순서와 같으며, 최댓값 9,999,999는
//! f32로 정확합니다. `stellar.yaml`의 Apple `minimum_os`는 이 범위 밖을 거부하고(IX.10.2), 직접 만든 범위 밖 값은
//! 상한으로 포화합니다(순서는 뒤집지 않음). 표현식은 같은 인코딩으로 비교합니다(예: iOS 17.4 이상은
//! `os_version >= 17004`).
//! profile을 바꿀 때 [`changed_keys`]는 값이 실제로 바뀐 key만 돌려줍니다. bindings의 환경 reverse index(VIII.7.3)는 이
//! key 집합으로 해당 key를 참조한 expression만 무효화합니다(예: iOS 26 → macOS 26은 무효화 없음).

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::FiniteF32;

use crate::spec::{ProfileSpec, TargetProfile};

/// 환경 key 번호(VIII.7.1 `EnvironmentKey`와 같은 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentKeyCode(pub u32);

impl EnvironmentKeyCode {
    /// `platform`.
    pub const PLATFORM: EnvironmentKeyCode = EnvironmentKeyCode(1);
    /// `os_version`.
    pub const OS_VERSION: EnvironmentKeyCode = EnvironmentKeyCode(2);
    /// `api_level`.
    pub const API_LEVEL: EnvironmentKeyCode = EnvironmentKeyCode(3);

    /// profile이 정하는 key(번호 오름차순).
    pub const PROFILE_KEYS: [EnvironmentKeyCode; 3] = [
        EnvironmentKeyCode::PLATFORM,
        EnvironmentKeyCode::OS_VERSION,
        EnvironmentKeyCode::API_LEVEL,
    ];
}

/// `platform` enum 타입 번호(VIII.7.1 `PLATFORM_TYPE`).
pub const PLATFORM_ENUM_TYPE: u32 = 0xE000_0001;
/// `platform` variant: profile 없음(일반 미리보기).
pub const PLATFORM_GENERIC: u32 = 0;

/// `os_version` 인코딩이 정확히 담는 최대 major.
pub const MAX_OS_MAJOR: u16 = 9_999;
/// `os_version` 인코딩이 정확히 담는 최대 minor.
pub const MAX_OS_MINOR: u16 = 999;

/// profile이 정한 환경 값.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileEnvironment {
    values: BTreeMap<EnvironmentKeyCode, TypedValue>,
}

impl ProfileEnvironment {
    /// profile의 환경 값. `None`이면 profile 없음(generic, 0, 0 — VIII.7.2 missing 기본값과 같음).
    pub fn for_profile(profile: Option<&TargetProfile>) -> ProfileEnvironment {
        let (platform, os_version, api_level) = match profile.map(|p| p.spec) {
            None => (PLATFORM_GENERIC, FiniteF32::ZERO, FiniteF32::ZERO),
            Some(ProfileSpec::Web(_)) => (
                u32::from(crate::profile::TargetProfileKind::Web.code()),
                FiniteF32::ZERO,
                FiniteF32::ZERO,
            ),
            Some(ProfileSpec::Apple(apple)) => (
                u32::from(crate::profile::TargetProfileKind::Apple.code()),
                os_version_code(apple.minimum_os.major(), apple.minimum_os.minor()),
                FiniteF32::ZERO,
            ),
            Some(ProfileSpec::Android(android)) => (
                u32::from(crate::profile::TargetProfileKind::Android.code()),
                FiniteF32::ZERO,
                finite(f32::from(android.api_level.get())),
            ),
        };
        let mut values = BTreeMap::new();
        values.insert(
            EnvironmentKeyCode::PLATFORM,
            TypedValue::Enum(EnumValue::new(PLATFORM_ENUM_TYPE, platform)),
        );
        values.insert(
            EnvironmentKeyCode::OS_VERSION,
            TypedValue::Number(os_version),
        );
        values.insert(EnvironmentKeyCode::API_LEVEL, TypedValue::Number(api_level));
        ProfileEnvironment { values }
    }

    /// key의 값.
    pub fn get(&self, key: EnvironmentKeyCode) -> Option<&TypedValue> {
        self.values.get(&key)
    }

    /// 모든 (key, 값)(key 오름차순).
    pub fn values(&self) -> &BTreeMap<EnvironmentKeyCode, TypedValue> {
        &self.values
    }
}

/// 두 환경에서 값이 다른 key(한쪽에만 있는 key 포함).
pub fn changed_keys(
    old: &ProfileEnvironment,
    new: &ProfileEnvironment,
) -> BTreeSet<EnvironmentKeyCode> {
    old.values
        .keys()
        .chain(new.values.keys())
        .filter(|key| old.values.get(key) != new.values.get(key))
        .copied()
        .collect()
}

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).unwrap_or(FiniteF32::ZERO)
}

/// `os_version` 값: `major × 1000 + minor`(모듈 문서). 상한 밖 성분은 상한으로 포화합니다.
fn os_version_code(major: u16, minor: u16) -> FiniteF32 {
    let major = u32::from(major.min(MAX_OS_MAJOR));
    let minor = u32::from(minor.min(MAX_OS_MINOR));
    // 최댓값 9_999_999 < 2^24이므로 f32 변환이 정확합니다.
    let code = major * 1_000 + minor;
    finite(code as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::fixtures::{android, apple, web};
    use crate::spec::ApplePlatform;

    fn number(env: &ProfileEnvironment, key: EnvironmentKeyCode) -> f32 {
        match env.get(key) {
            Some(TypedValue::Number(n)) => n.get(),
            other => panic!("숫자가 아님: {other:?}"),
        }
    }

    fn platform(env: &ProfileEnvironment) -> u32 {
        match env.get(EnvironmentKeyCode::PLATFORM) {
            Some(TypedValue::Enum(value)) => {
                assert_eq!(value.enum_type(), PLATFORM_ENUM_TYPE);
                value.variant()
            }
            other => panic!("enum이 아님: {other:?}"),
        }
    }

    #[test]
    fn profile_environment_values_match_key_registry() {
        // VIII.7.1 key·enum 번호와 같은 값입니다.
        assert_eq!(EnvironmentKeyCode::PROFILE_KEYS.map(|k| k.0), [1, 2, 3]);
        assert_eq!(PLATFORM_ENUM_TYPE, 0xE000_0001);

        let generic = ProfileEnvironment::for_profile(None);
        assert_eq!(platform(&generic), 0);
        assert_eq!(number(&generic, EnvironmentKeyCode::OS_VERSION), 0.0);
        assert_eq!(number(&generic, EnvironmentKeyCode::API_LEVEL), 0.0);

        let ios = ProfileEnvironment::for_profile(Some(&apple("i", ApplePlatform::Ios, 26, 1)));
        assert_eq!(platform(&ios), 2);
        assert_eq!(number(&ios, EnvironmentKeyCode::OS_VERSION), 26_000.0);
        assert_eq!(number(&ios, EnvironmentKeyCode::API_LEVEL), 0.0);

        let droid = ProfileEnvironment::for_profile(Some(&android("a", 35, 1)));
        assert_eq!(platform(&droid), 3);
        assert_eq!(number(&droid, EnvironmentKeyCode::OS_VERSION), 0.0);
        assert_eq!(number(&droid, EnvironmentKeyCode::API_LEVEL), 35.0);

        let site = ProfileEnvironment::for_profile(Some(&web("w", 2024, 1)));
        assert_eq!(platform(&site), 1);
    }

    fn apple_os(major: u16, minor: u16) -> ProfileEnvironment {
        use crate::profile::TargetProfileId;
        use crate::registry::ProfileRegistryVersion;
        use crate::spec::{AppleProfile, DottedVersion};
        let profile = TargetProfile::new(
            TargetProfileId::new("ios").expect("id"),
            ProfileRegistryVersion::new(1).expect("pin"),
            ProfileSpec::Apple(AppleProfile {
                platform: ApplePlatform::Ios,
                minimum_os: DottedVersion::new(major, minor, 0),
            }),
        );
        ProfileEnvironment::for_profile(Some(&profile))
    }

    #[test]
    fn os_version_preserves_version_order_and_identity() {
        let os = |major, minor| number(&apple_os(major, minor), EnvironmentKeyCode::OS_VERSION);
        // `major × 1000 + minor` 정수 인코딩: 표현식은 이 값과 비교합니다.
        assert_eq!(os(17, 4), 17_004.0);
        assert_eq!(os(10, 15), 10_015.0);
        assert_eq!(os(26, 0), 26_000.0);
        assert_eq!(os(0, 0), 0.0);
        // 수 순서 = 버전 순서(10.9 < 10.10 < 10.15, 17.1 < 17.2 < 17.10).
        let ascending = [
            (10, 9),
            (10, 10),
            (10, 15),
            (11, 0),
            (17, 1),
            (17, 2),
            (17, 10),
            (26, 0),
            (MAX_OS_MAJOR, MAX_OS_MINOR),
        ];
        for pair in ascending.windows(2) {
            let (a, b) = (os(pair[0].0, pair[0].1), os(pair[1].0, pair[1].1));
            assert!(a < b, "{:?} → {a} < {:?} → {b}", pair[0], pair[1]);
        }
        // 서로 다른 버전은 다른 값이라 profile을 바꾸면 os_version이 무효화됩니다.
        let keys = changed_keys(&apple_os(17, 1), &apple_os(17, 10))
            .into_iter()
            .map(|k| k.0)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![2]);
        // 상한 안의 가장 큰 값도 f32로 정확합니다.
        assert_eq!(os(MAX_OS_MAJOR, MAX_OS_MINOR), 9_999_999.0);
        // 상한 밖(직접 만든 profile)은 상한으로 포화해 순서를 뒤집지 않습니다.
        assert_eq!(os(17, 1000), os(17, MAX_OS_MINOR));
        assert!(os(17, 1000) < os(18, 0));
        assert_eq!(os(u16::MAX, u16::MAX), os(MAX_OS_MAJOR, MAX_OS_MINOR));
    }

    #[test]
    fn switching_profiles_reports_only_changed_keys() {
        let env = |p: Option<&TargetProfile>| ProfileEnvironment::for_profile(p);
        let ios = apple("i", ApplePlatform::Ios, 26, 1);
        let macos = apple("m", ApplePlatform::MacOs, 26, 1);
        let ios17 = apple("i17", ApplePlatform::Ios, 17, 1);
        let droid31 = android("a31", 31, 1);
        let droid35 = android("a35", 35, 1);
        let site = web("w", 2024, 1);
        let site_old = web("w", 2022, 1);
        let keys = |a: &ProfileEnvironment, b: &ProfileEnvironment| {
            changed_keys(a, b)
                .into_iter()
                .map(|k| k.0)
                .collect::<Vec<_>>()
        };
        // 같은 Apple OS 버전의 다른 플랫폼: 무효화 없음.
        assert_eq!(
            keys(&env(Some(&ios)), &env(Some(&macos))),
            Vec::<u32>::new()
        );
        assert_eq!(keys(&env(Some(&ios)), &env(Some(&ios17))), vec![2]);
        assert_eq!(keys(&env(Some(&droid31)), &env(Some(&droid35))), vec![3]);
        assert_eq!(keys(&env(Some(&ios)), &env(Some(&site))), vec![1, 2]);
        assert_eq!(keys(&env(Some(&droid35)), &env(Some(&site))), vec![1, 3]);
        assert_eq!(
            keys(&env(Some(&site)), &env(Some(&site_old))),
            Vec::<u32>::new()
        );
        assert_eq!(keys(&env(None), &env(Some(&ios))), vec![1, 2]);
    }
}
