//! 앱 내장 profile registry release (IX.10.5).
//!
//! release 1의 capability 표와 새 profile 기본값입니다. 내장 release의 내용은 버전이 나간 뒤 바꾸지 않습니다
//! (IX.10.4 — 바꿀 내용은 새 버전으로 추가). 상태 matrix test가 내용 변경을 감지합니다.
//!
//! 대표 속성([`properties`])은 capability가 플랫폼마다 다른 속성만 담습니다. 모든 플랫폼에서 Native인 일반 속성은
//! 이 표에 넣지 않고, 필요할 때 공통 namespace에 추가합니다.

use crate::capability::{
    CapabilityPropertyId, CapabilityRule, CapabilityStatus, CapabilitySubject, CapabilityTable,
    CapabilityTableError, ContractCode, ElementKind, ProfileLevel, StatusLadder, StatusStep,
};
use crate::registry::{
    NewProfileDefaults, ProfileRegistry, ProfileRegistryVersion, RegistryCatalog,
};
use crate::spec::{
    AndroidApiLevel, ApplePlatform, CapabilitySetVersion, DottedVersion, WebBaseline,
};

/// 대표 capability 속성.
pub mod properties {
    use crate::capability::CapabilityPropertyId;

    /// 모서리 반경(공통).
    pub const CORNER_RADIUS: CapabilityPropertyId = CapabilityPropertyId(0x0000_0001);
    /// drop shadow(공통).
    pub const DROP_SHADOW: CapabilityPropertyId = CapabilityPropertyId(0x0000_0002);
    /// layer blur(공통).
    pub const LAYER_BLUR: CapabilityPropertyId = CapabilityPropertyId(0x0000_0003);
    /// 선형 gradient(공통).
    pub const LINEAR_GRADIENT: CapabilityPropertyId = CapabilityPropertyId(0x0000_0004);
    /// clip path(공통).
    pub const CLIP_PATH: CapabilityPropertyId = CapabilityPropertyId(0x0000_0005);
    /// CSS `backdrop-filter`.
    pub const CSS_BACKDROP_FILTER: CapabilityPropertyId = CapabilityPropertyId(0x0001_0001);
    /// CSS container query.
    pub const CSS_CONTAINER_QUERY: CapabilityPropertyId = CapabilityPropertyId(0x0001_0002);
    /// CSS subgrid.
    pub const CSS_SUBGRID: CapabilityPropertyId = CapabilityPropertyId(0x0001_0003);
    /// CSS `clip`(rect, 사용 중단 — `clip-path`로 대체).
    pub const CSS_CLIP_RECT: CapabilityPropertyId = CapabilityPropertyId(0x0001_0004);
    /// Apple SF Symbol.
    pub const APPLE_SF_SYMBOL: CapabilityPropertyId = CapabilityPropertyId(0x0002_0001);
    /// Apple material vibrancy.
    pub const APPLE_MATERIAL_VIBRANCY: CapabilityPropertyId = CapabilityPropertyId(0x0002_0002);
    /// Apple continuous corner(smooth corner).
    pub const APPLE_CONTINUOUS_CORNER: CapabilityPropertyId = CapabilityPropertyId(0x0002_0003);
    /// Android ripple indication.
    pub const ANDROID_RIPPLE: CapabilityPropertyId = CapabilityPropertyId(0x0003_0001);
    /// Android dynamic color.
    pub const ANDROID_DYNAMIC_COLOR: CapabilityPropertyId = CapabilityPropertyId(0x0003_0002);
    /// Android status bar color(API 35부터 사용 중단).
    pub const ANDROID_STATUS_BAR_COLOR: CapabilityPropertyId = CapabilityPropertyId(0x0003_0003);

    /// 모든 대표 속성(번호 오름차순).
    pub const ALL: [CapabilityPropertyId; 15] = [
        CORNER_RADIUS,
        DROP_SHADOW,
        LAYER_BLUR,
        LINEAR_GRADIENT,
        CLIP_PATH,
        CSS_BACKDROP_FILTER,
        CSS_CONTAINER_QUERY,
        CSS_SUBGRID,
        CSS_CLIP_RECT,
        APPLE_SF_SYMBOL,
        APPLE_MATERIAL_VIBRANCY,
        APPLE_CONTINUOUS_CORNER,
        ANDROID_RIPPLE,
        ANDROID_DYNAMIC_COLOR,
        ANDROID_STATUS_BAR_COLOR,
    ];
}

/// release 1 버전.
pub const V1: ProfileRegistryVersion = match ProfileRegistryVersion::new(1) {
    Ok(version) => version,
    Err(_) => panic!("release 1 버전은 1 이상입니다"),
};

use CapabilityStatus::{Approximated as A, Deprecated as D, Native as N, Unsupported as U};

fn ladder(steps: &[(ProfileLevel, CapabilityStatus)]) -> StatusLadder {
    StatusLadder::steps(
        steps
            .iter()
            .map(|&(from, status)| StatusStep { from, status })
            .collect(),
    )
}

fn constant(status: CapabilityStatus) -> StatusLadder {
    StatusLadder::constant(status)
}

fn rule(
    subject: CapabilitySubject,
    web: StatusLadder,
    apple: StatusLadder,
    android: StatusLadder,
) -> CapabilityRule {
    CapabilityRule {
        subject,
        web,
        apple,
        apple_platforms: Vec::new(),
        android,
        replacement: None,
    }
}

/// release 1 capability 규칙 목록.
pub fn rules_v1() -> Vec<CapabilityRule> {
    use properties as p;
    let min = ProfileLevel::MIN;
    let prop = CapabilitySubject::Property;
    let contract = CapabilitySubject::Contract;

    let mut rules: Vec<CapabilityRule> = ElementKind::ALL
        .into_iter()
        .map(|kind| CapabilityRule::uniform(CapabilitySubject::Element(kind), N))
        .collect();

    rules.push(CapabilityRule::uniform(contract(ContractCode::BUTTON), N));
    rules.push(CapabilityRule::uniform(
        contract(ContractCode::TEXT_INPUT),
        N,
    ));
    rules.push(rule(
        contract(ContractCode::MODAL),
        ladder(&[(min, A), (ProfileLevel::web(2022), N)]),
        constant(N),
        constant(N),
    ));
    rules.push(CapabilityRule::uniform(contract(ContractCode::LIST), N));
    let mut group = rule(
        contract(ContractCode::SINGLE_SELECTION_GROUP),
        constant(N),
        constant(A),
        constant(N),
    );
    group.apple_platforms = vec![(ApplePlatform::MacOs, constant(N))];
    rules.push(group);

    rules.push(CapabilityRule::uniform(prop(p::CORNER_RADIUS), N));
    rules.push(rule(
        prop(p::DROP_SHADOW),
        constant(N),
        constant(N),
        ladder(&[(min, A), (ProfileLevel::android(28), N)]),
    ));
    rules.push(rule(
        prop(p::LAYER_BLUR),
        constant(N),
        constant(N),
        ladder(&[(min, U), (ProfileLevel::android(31), N)]),
    ));
    rules.push(CapabilityRule::uniform(prop(p::LINEAR_GRADIENT), N));
    rules.push(CapabilityRule::uniform(prop(p::CLIP_PATH), N));
    rules.push(rule(
        prop(p::CSS_BACKDROP_FILTER),
        ladder(&[(min, A), (ProfileLevel::web(2024), N)]),
        constant(N),
        constant(U),
    ));
    rules.push(rule(
        prop(p::CSS_CONTAINER_QUERY),
        ladder(&[(min, U), (ProfileLevel::web(2023), N)]),
        constant(A),
        constant(A),
    ));
    rules.push(rule(
        prop(p::CSS_SUBGRID),
        ladder(&[(min, U), (ProfileLevel::web(2023), N)]),
        constant(U),
        constant(U),
    ));
    let mut clip_rect = rule(
        prop(p::CSS_CLIP_RECT),
        constant(D),
        constant(A),
        constant(A),
    );
    clip_rect.replacement = Some(prop(p::CLIP_PATH));
    rules.push(clip_rect);
    rules.push(rule(
        prop(p::APPLE_SF_SYMBOL),
        constant(A),
        constant(N),
        constant(A),
    ));
    rules.push(rule(
        prop(p::APPLE_MATERIAL_VIBRANCY),
        constant(A),
        constant(N),
        constant(U),
    ));
    rules.push(rule(
        prop(p::APPLE_CONTINUOUS_CORNER),
        constant(A),
        constant(N),
        constant(A),
    ));
    rules.push(rule(
        prop(p::ANDROID_RIPPLE),
        constant(A),
        constant(A),
        constant(N),
    ));
    rules.push(rule(
        prop(p::ANDROID_DYNAMIC_COLOR),
        constant(U),
        constant(U),
        ladder(&[(min, U), (ProfileLevel::android(31), N)]),
    ));
    rules.push(rule(
        prop(p::ANDROID_STATUS_BAR_COLOR),
        constant(A),
        constant(U),
        ladder(&[(min, N), (ProfileLevel::android(35), D)]),
    ));
    rules
}

/// release 1 새 profile 기본값.
pub fn defaults_v1() -> Result<NewProfileDefaults, crate::node::ValueError> {
    Ok(NewProfileDefaults {
        apple_minimum_os: DottedVersion::new(26, 0, 0),
        android_api_level: AndroidApiLevel::new(35)?,
        android_ui_system_version: DottedVersion::new(1, 7, 0),
        web_baseline: WebBaseline::new(2024)?,
        web_capability_set_version: CapabilitySetVersion::new(1)?,
    })
}

/// 내장 release 구성 오류(내장 데이터 결함).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinError {
    /// 기본값 범위 위반.
    Defaults(crate::node::ValueError),
    /// capability 표 구조 위반.
    Capabilities(CapabilityTableError),
    /// catalog 구성 위반.
    Catalog(crate::registry::CatalogError),
}

/// release 1.
pub fn registry_v1() -> Result<ProfileRegistry, BuiltinError> {
    Ok(ProfileRegistry::new(
        V1,
        defaults_v1().map_err(BuiltinError::Defaults)?,
        CapabilityTable::new(rules_v1()).map_err(BuiltinError::Capabilities)?,
    ))
}

/// 앱 내장 catalog.
pub fn catalog() -> Result<RegistryCatalog, BuiltinError> {
    RegistryCatalog::new(vec![registry_v1()?]).map_err(BuiltinError::Catalog)
}

/// 표가 알려진 element 종류·contract 종류·대표 속성을 빠짐없이 담는지 검사합니다. 누락 subject 목록을 돌려줍니다.
pub fn missing_subjects(table: &CapabilityTable) -> Vec<CapabilitySubject> {
    ElementKind::ALL
        .into_iter()
        .map(CapabilitySubject::Element)
        .chain(
            ContractCode::ALL
                .into_iter()
                .map(CapabilitySubject::Contract),
        )
        .chain(properties::ALL.into_iter().map(CapabilitySubject::Property))
        .filter(|subject| table.rule(*subject).is_none())
        .collect()
}

/// 대표 속성의 이름(`namespace.name`). 표에 없는 id는 `None`.
pub fn property_name(id: CapabilityPropertyId) -> Option<&'static str> {
    use properties as p;
    Some(match id {
        p::CORNER_RADIUS => "common.corner_radius",
        p::DROP_SHADOW => "common.drop_shadow",
        p::LAYER_BLUR => "common.layer_blur",
        p::LINEAR_GRADIENT => "common.linear_gradient",
        p::CLIP_PATH => "common.clip_path",
        p::CSS_BACKDROP_FILTER => "css.backdrop_filter",
        p::CSS_CONTAINER_QUERY => "css.container_query",
        p::CSS_SUBGRID => "css.subgrid",
        p::CSS_CLIP_RECT => "css.clip_rect",
        p::APPLE_SF_SYMBOL => "apple.sf_symbol",
        p::APPLE_MATERIAL_VIBRANCY => "apple.material_vibrancy",
        p::APPLE_CONTINUOUS_CORNER => "apple.continuous_corner",
        p::ANDROID_RIPPLE => "android.ripple",
        p::ANDROID_DYNAMIC_COLOR => "android.dynamic_color",
        p::ANDROID_STATUS_BAR_COLOR => "android.status_bar_color",
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 profile 생성기.
    use crate::profile::TargetProfileId;
    use crate::registry::ProfileRegistryVersion;
    use crate::spec::{
        AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, CapabilitySetVersion,
        DottedVersion, ProfileSpec, TargetProfile, WebBaseline, WebProfile,
    };

    fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("id")
    }

    /// Web profile(release `pin`).
    pub fn web(name: &str, baseline: u16, pin: u32) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(pin).expect("pin"),
            ProfileSpec::Web(WebProfile {
                baseline: WebBaseline::new(baseline).expect("baseline"),
                capability_set_version: CapabilitySetVersion::new(1).expect("set"),
            }),
        )
    }

    /// Apple profile(release `pin`).
    pub fn apple(name: &str, platform: ApplePlatform, major: u16, pin: u32) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(pin).expect("pin"),
            ProfileSpec::Apple(AppleProfile {
                platform,
                minimum_os: DottedVersion::new(major, 0, 0),
            }),
        )
    }

    /// Android profile(release `pin`).
    pub fn android(name: &str, api: u16, pin: u32) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(pin).expect("pin"),
            ProfileSpec::Android(AndroidProfile {
                api_level: AndroidApiLevel::new(api).expect("api"),
                ui_system_version: DottedVersion::new(1, 7, 0),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{android, apple, web};
    use super::properties as p;
    use super::*;
    use crate::spec::TargetProfile;

    /// 상태 matrix의 열(profile).
    fn columns() -> Vec<(&'static str, TargetProfile)> {
        vec![
            ("web-2022", web("web-2022", 2022, 1)),
            ("web-2024", web("web-2024", 2024, 1)),
            ("ios-26", apple("ios-26", ApplePlatform::Ios, 26, 1)),
            ("macos-26", apple("macos-26", ApplePlatform::MacOs, 26, 1)),
            ("android-27", android("android-27", 27, 1)),
            ("android-31", android("android-31", 31, 1)),
            ("android-35", android("android-35", 35, 1)),
        ]
    }

    #[test]
    fn release_one_status_matrix_is_golden() {
        let registry = registry_v1().expect("내장 release");
        let table = registry.capabilities();
        use CapabilitySubject::{Contract as C, Element as E, Property as P};
        // 열: web-2022, web-2024, ios-26, macos-26, android-27, android-31, android-35.
        let expected: Vec<(CapabilitySubject, [CapabilityStatus; 7])> = vec![
            (E(ElementKind::FRAME), [N, N, N, N, N, N, N]),
            (E(ElementKind::FRAGMENT), [N, N, N, N, N, N, N]),
            (E(ElementKind::SHAPE), [N, N, N, N, N, N, N]),
            (E(ElementKind::PATH), [N, N, N, N, N, N, N]),
            (E(ElementKind::TEXT), [N, N, N, N, N, N, N]),
            (E(ElementKind::IMAGE), [N, N, N, N, N, N, N]),
            (E(ElementKind::COMPONENT_INSTANCE), [N, N, N, N, N, N, N]),
            (P(p::CORNER_RADIUS), [N, N, N, N, N, N, N]),
            (P(p::DROP_SHADOW), [N, N, N, N, A, N, N]),
            (P(p::LAYER_BLUR), [N, N, N, N, U, N, N]),
            (P(p::LINEAR_GRADIENT), [N, N, N, N, N, N, N]),
            (P(p::CLIP_PATH), [N, N, N, N, N, N, N]),
            (P(p::CSS_BACKDROP_FILTER), [A, N, N, N, U, U, U]),
            (P(p::CSS_CONTAINER_QUERY), [U, N, A, A, A, A, A]),
            (P(p::CSS_SUBGRID), [U, N, U, U, U, U, U]),
            (P(p::CSS_CLIP_RECT), [D, D, A, A, A, A, A]),
            (P(p::APPLE_SF_SYMBOL), [A, A, N, N, A, A, A]),
            (P(p::APPLE_MATERIAL_VIBRANCY), [A, A, N, N, U, U, U]),
            (P(p::APPLE_CONTINUOUS_CORNER), [A, A, N, N, A, A, A]),
            (P(p::ANDROID_RIPPLE), [A, A, A, A, N, N, N]),
            (P(p::ANDROID_DYNAMIC_COLOR), [U, U, U, U, U, N, N]),
            (P(p::ANDROID_STATUS_BAR_COLOR), [A, A, U, U, N, N, D]),
            (C(ContractCode::BUTTON), [N, N, N, N, N, N, N]),
            (C(ContractCode::TEXT_INPUT), [N, N, N, N, N, N, N]),
            (C(ContractCode::MODAL), [N, N, N, N, N, N, N]),
            (C(ContractCode::LIST), [N, N, N, N, N, N, N]),
            (
                C(ContractCode::SINGLE_SELECTION_GROUP),
                [N, N, A, N, N, N, N],
            ),
        ];
        // 표 전체가 matrix에 있고(누락·여분 없음) 모든 칸이 기대값과 같습니다.
        let mut listed: Vec<CapabilitySubject> = expected.iter().map(|(s, _)| *s).collect();
        listed.sort();
        let registered: Vec<CapabilitySubject> =
            table.rules().iter().map(|rule| rule.subject).collect();
        assert_eq!(listed, registered);
        let columns = columns();
        for (subject, row) in &expected {
            for ((name, profile), want) in columns.iter().zip(row) {
                assert_eq!(
                    table.status(profile, *subject),
                    Some(*want),
                    "{subject} @ {name}"
                );
            }
        }
        // Modal은 Baseline 2022 이전 Web에서 근사, 사용 중단 속성은 대체 subject를 가집니다.
        assert_eq!(
            table.status(&web("w", 2021, 1), C(ContractCode::MODAL)),
            Some(A)
        );
        assert_eq!(
            table.rule(P(p::CSS_CLIP_RECT)).and_then(|r| r.replacement),
            Some(P(p::CLIP_PATH))
        );
        // 네 상태가 모두 matrix에 나타납니다.
        for status in CapabilityStatus::ALL {
            assert!(
                expected.iter().any(|(_, row)| row.contains(&status)),
                "{status}"
            );
        }
    }

    #[test]
    fn builtin_registry_covers_known_subjects() {
        let registry = registry_v1().expect("내장 release");
        assert!(missing_subjects(registry.capabilities()).is_empty());
        assert!(!missing_subjects(&CapabilityTable::empty()).is_empty());
        let catalog = catalog().expect("catalog");
        assert_eq!(catalog.latest().version(), V1);
        for id in p::ALL {
            let name = property_name(id).expect("이름");
            let namespace = match id.namespace() {
                CapabilityPropertyId::NAMESPACE_COMMON => "common.",
                CapabilityPropertyId::NAMESPACE_CSS => "css.",
                CapabilityPropertyId::NAMESPACE_APPLE => "apple.",
                CapabilityPropertyId::NAMESPACE_ANDROID => "android.",
                other => panic!("알 수 없는 namespace {other}"),
            };
            assert!(name.starts_with(namespace), "{name}");
        }
        assert_eq!(property_name(CapabilityPropertyId(0x0009_0001)), None);
    }
}
