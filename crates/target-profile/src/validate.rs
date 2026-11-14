//! 한 View를 여러 Target Profile에 대해 동시에 검사하는 capability validator (IX.10.8).
//!
//! 입력은 문서 계층에 의존하지 않는 지역 모델입니다.
//!
//! - [`ViewModel`]: node 목록(문서 순서, parent·element 종류·semantic contract)과 공통값·profile override.
//! - [`PropertyCapabilityMap`]: PropertyPath가 어떤 [`CapabilityPropertyId`]에 해당하는지 알려 주는 trait(문서 계층 구현).
//!
//! profile마다 pin한 registry release로 해석하고(IX.10.4), 공통값 + profile override + variant 층을 적용한 값
//! (IX.10.7)에서 사용 중인 element·contract·속성을 찾아 상태별 진단([`CapabilityFinding`])을 만듭니다. profile에서 제외한
//! node와 그 하위 node는 검사하지 않습니다. 같은 node에서 같은 속성 capability를 여러 path가 쓰면 path 오름차순 첫
//! path로 한 번만 보고합니다.
//!
//! | 상태 | 진단 code | 심각도 |
//! |---|---|---|
//! | Native | `Native`(1) | Note(코드 생성 입력용 기록) |
//! | Approximated | `Approximated`(2) | Info |
//! | Unsupported | `Unsupported`(3) | Error |
//! | Deprecated | `Deprecated`(4) | Warning |
//! | 미등록 | `NotRegistered`(5) | Error |
//!
//! profile이 pin한 release를 앱이 모르면 그 profile은 검사하지 않고 [`ProfileOutcome::RegistryUnavailable`]로 보고합니다.
//! 프로젝트에 없는 profile의 override는 [`MultiProfileReport::unknown_override_profiles`]로 보고합니다. 결과 순서는 입력
//! profile 순서 → node 문서 순서 → (element, contract 번호순, 속성 path순)으로 결정론적입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::capability::{
    CapabilityPropertyId, CapabilityStatus, CapabilitySubject, ContractCode, ElementKind,
};
use crate::manifest::ProjectTargetProfiles;
use crate::overrides::{resolve_for_profile, CommonValues, ProfileOverrideSet, VariantLayer};
use crate::profile::TargetProfileId;
use crate::registry::{ProfileRegistryVersion, RegistryCatalog, ResolveError};
use crate::spec::TargetProfile;

/// View의 node 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ViewNode {
    /// node.
    pub id: StableId,
    /// 부모(root면 `None`).
    pub parent: Option<StableId>,
    /// element 종류.
    pub element: ElementKind,
    /// 붙은 semantic contract.
    pub contracts: Vec<ContractCode>,
}

/// 검사할 View.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewModel {
    /// node(문서 순서).
    pub nodes: Vec<ViewNode>,
    /// 공통값.
    pub common: CommonValues,
    /// profile별 override.
    pub overrides: ProfileOverrideSet,
}

/// PropertyPath → capability 속성 대응(문서 계층 구현).
pub trait PropertyCapabilityMap {
    /// path가 capability 검사 대상 속성이면 그 id.
    fn capability_of(&self, path: &PropertyPath) -> Option<CapabilityPropertyId>;
}

/// 진단 code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityDiagnosticCode {
    /// Native.
    Native = 1,
    /// Approximated.
    Approximated = 2,
    /// Unsupported.
    Unsupported = 3,
    /// Deprecated.
    Deprecated = 4,
    /// registry에 없는 subject.
    NotRegistered = 5,
}

/// 진단 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// 기록(문제 아님).
    Note,
    /// 정보.
    Info,
    /// 경고.
    Warning,
    /// 오류.
    Error,
}

impl CapabilityDiagnosticCode {
    /// 상태(미등록은 `None`)의 진단 code.
    pub const fn for_status(status: Option<CapabilityStatus>) -> CapabilityDiagnosticCode {
        match status {
            Some(CapabilityStatus::Native) => CapabilityDiagnosticCode::Native,
            Some(CapabilityStatus::Approximated) => CapabilityDiagnosticCode::Approximated,
            Some(CapabilityStatus::Unsupported) => CapabilityDiagnosticCode::Unsupported,
            Some(CapabilityStatus::Deprecated) => CapabilityDiagnosticCode::Deprecated,
            None => CapabilityDiagnosticCode::NotRegistered,
        }
    }

    /// 심각도.
    pub const fn severity(self) -> Severity {
        match self {
            CapabilityDiagnosticCode::Native => Severity::Note,
            CapabilityDiagnosticCode::Approximated => Severity::Info,
            CapabilityDiagnosticCode::Deprecated => Severity::Warning,
            CapabilityDiagnosticCode::Unsupported | CapabilityDiagnosticCode::NotRegistered => {
                Severity::Error
            }
        }
    }
}

/// 사용 하나의 capability 진단.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityFinding {
    /// node.
    pub node: StableId,
    /// 대상.
    pub subject: CapabilitySubject,
    /// 속성이면 그 path.
    pub path: Option<PropertyPath>,
    /// 상태(미등록이면 `None`).
    pub status: Option<CapabilityStatus>,
    /// 진단 code.
    pub code: CapabilityDiagnosticCode,
    /// Deprecated의 대체 subject.
    pub replacement: Option<CapabilitySubject>,
}

/// profile 하나의 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileOutcome {
    /// 검사함.
    Validated {
        /// 사용한 registry release(= pin).
        registry_version: ProfileRegistryVersion,
        /// 진단(결정론적 순서).
        findings: Vec<CapabilityFinding>,
    },
    /// pin한 release를 앱이 몰라 검사하지 않음.
    RegistryUnavailable {
        /// pin.
        pinned: ProfileRegistryVersion,
        /// 앱 최신 release.
        latest: ProfileRegistryVersion,
    },
}

/// profile 하나의 보고.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileReport {
    /// profile.
    pub profile: TargetProfileId,
    /// 결과.
    pub outcome: ProfileOutcome,
}

impl ProfileReport {
    /// 진단(검사하지 않았으면 빈 slice).
    pub fn findings(&self) -> &[CapabilityFinding] {
        match &self.outcome {
            ProfileOutcome::Validated { findings, .. } => findings,
            ProfileOutcome::RegistryUnavailable { .. } => &[],
        }
    }

    /// code별 개수.
    pub fn count(&self, code: CapabilityDiagnosticCode) -> usize {
        self.findings().iter().filter(|f| f.code == code).count()
    }

    /// Note가 아닌 진단(문제 목록).
    pub fn problems(&self) -> impl Iterator<Item = &CapabilityFinding> {
        self.findings()
            .iter()
            .filter(|f| f.code.severity() != Severity::Note)
    }
}

/// 여러 profile 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiProfileReport {
    /// profile별 보고(입력 순서).
    pub profiles: Vec<ProfileReport>,
    /// 프로젝트에 없는 profile의 override.
    pub unknown_override_profiles: Vec<TargetProfileId>,
}

impl MultiProfileReport {
    /// profile의 보고.
    pub fn profile(&self, id: &TargetProfileId) -> Option<&ProfileReport> {
        self.profiles.iter().find(|r| &r.profile == id)
    }

    /// Error 심각도 진단이나 검사하지 못한 profile이 있는지.
    pub fn has_errors(&self) -> bool {
        self.profiles.iter().any(|report| match &report.outcome {
            ProfileOutcome::RegistryUnavailable { .. } => true,
            ProfileOutcome::Validated { findings, .. } => findings
                .iter()
                .any(|f| f.code.severity() == Severity::Error),
        })
    }
}

/// 프로젝트의 모든 profile에 대해 View를 동시에 검사합니다.
pub fn validate_view(
    view: &ViewModel,
    map: &dyn PropertyCapabilityMap,
    section: &ProjectTargetProfiles,
    catalog: &RegistryCatalog,
    variants: &VariantLayer,
) -> MultiProfileReport {
    let mut report = validate_profiles(view, map, section.profiles(), catalog, variants);
    report.unknown_override_profiles = view.overrides.unknown_profiles(section);
    report
}

/// 주어진 profile 목록(입력 순서)에 대해 View를 검사합니다.
pub fn validate_profiles(
    view: &ViewModel,
    map: &dyn PropertyCapabilityMap,
    profiles: &[TargetProfile],
    catalog: &RegistryCatalog,
    variants: &VariantLayer,
) -> MultiProfileReport {
    let parents: BTreeMap<StableId, Option<StableId>> =
        view.nodes.iter().map(|n| (n.id, n.parent)).collect();
    let reports = profiles
        .iter()
        .map(|profile| ProfileReport {
            profile: profile.id.clone(),
            outcome: validate_one(view, map, profile, catalog, variants, &parents),
        })
        .collect();
    MultiProfileReport {
        profiles: reports,
        unknown_override_profiles: Vec::new(),
    }
}

fn validate_one(
    view: &ViewModel,
    map: &dyn PropertyCapabilityMap,
    profile: &TargetProfile,
    catalog: &RegistryCatalog,
    variants: &VariantLayer,
    parents: &BTreeMap<StableId, Option<StableId>>,
) -> ProfileOutcome {
    let resolved = match catalog.resolve(profile) {
        Ok(resolved) => resolved,
        Err(ResolveError::RegistryVersionUnavailable { pinned, latest, .. }) => {
            return ProfileOutcome::RegistryUnavailable { pinned, latest };
        }
    };
    let table = resolved.registry().capabilities();
    let values = resolve_for_profile(&view.common, &view.overrides, profile, variants);
    let parent_of = |id: StableId| parents.get(&id).copied().flatten();
    let included: BTreeSet<StableId> = view
        .nodes
        .iter()
        .filter(|n| !values.is_excluded(n.id, parent_of, view.nodes.len()))
        .map(|n| n.id)
        .collect();

    // node별 속성 사용(path 오름차순 첫 path, capability 중복 제거).
    let mut properties: BTreeMap<StableId, Vec<(CapabilityPropertyId, PropertyPath)>> =
        BTreeMap::new();
    for path in values.values.keys() {
        let node = path.root();
        if !included.contains(&node) {
            continue;
        }
        let Some(id) = map.capability_of(path) else {
            continue;
        };
        let used = properties.entry(node).or_default();
        if !used.iter().any(|(existing, _)| *existing == id) {
            used.push((id, path.clone()));
        }
    }

    let finding = |node: StableId, subject: CapabilitySubject, path: Option<PropertyPath>| {
        let status = table.status(profile, subject);
        let replacement = match status {
            Some(CapabilityStatus::Deprecated) => table.rule(subject).and_then(|r| r.replacement),
            _ => None,
        };
        CapabilityFinding {
            node,
            subject,
            path,
            status,
            code: CapabilityDiagnosticCode::for_status(status),
            replacement,
        }
    };
    let mut findings = Vec::new();
    for node in view.nodes.iter().filter(|n| included.contains(&n.id)) {
        findings.push(finding(
            node.id,
            CapabilitySubject::Element(node.element),
            None,
        ));
        let mut contracts = node.contracts.clone();
        contracts.sort();
        contracts.dedup();
        for code in contracts {
            findings.push(finding(node.id, CapabilitySubject::Contract(code), None));
        }
        if let Some(used) = properties.get(&node.id) {
            for (id, path) in used {
                findings.push(finding(
                    node.id,
                    CapabilitySubject::Property(*id),
                    Some(path.clone()),
                ));
            }
        }
    }
    ProfileOutcome::Validated {
        registry_version: resolved.registry().version(),
        findings,
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! IX.10.8~IX.10.10 공용 multi-profile View fixture.
    use super::*;
    use crate::builtin::fixtures::{android, apple, web};
    use crate::builtin::properties as p;
    use crate::overrides::OverrideScope;
    use crate::spec::ApplePlatform;
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::typed_value::TypedValue;

    pub fn sid(n: u8) -> StableId {
        let mut bytes = [0u8; 16];
        bytes[15] = n;
        StableId::from_bytes(bytes)
    }

    pub fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("id")
    }

    /// PropertyKey 번호 = capability 속성 id의 하위 16bit + namespace × 100(테스트 대응표).
    pub struct KeyMap;

    pub const KEY_RADIUS: u32 = 1;
    pub const KEY_SHADOW: u32 = 2;
    pub const KEY_SHADOW_COLOR: u32 = 3;
    pub const KEY_BACKDROP: u32 = 101;
    pub const KEY_SUBGRID: u32 = 103;
    pub const KEY_CLIP_RECT: u32 = 104;
    pub const KEY_SF_SYMBOL: u32 = 201;
    pub const KEY_STATUS_BAR: u32 = 303;
    pub const KEY_LEGACY: u32 = 900;
    pub const KEY_PLAIN: u32 = 999;

    impl PropertyCapabilityMap for KeyMap {
        fn capability_of(&self, path: &PropertyPath) -> Option<CapabilityPropertyId> {
            Some(match path.keys().first()?.0 {
                KEY_RADIUS => p::CORNER_RADIUS,
                KEY_SHADOW | KEY_SHADOW_COLOR => p::DROP_SHADOW,
                KEY_BACKDROP => p::CSS_BACKDROP_FILTER,
                KEY_SUBGRID => p::CSS_SUBGRID,
                KEY_CLIP_RECT => p::CSS_CLIP_RECT,
                KEY_SF_SYMBOL => p::APPLE_SF_SYMBOL,
                KEY_STATUS_BAR => p::ANDROID_STATUS_BAR_COLOR,
                KEY_LEGACY => CapabilityPropertyId(0x0009_0001),
                _ => return None,
            })
        }
    }

    pub fn path(node: u8, key: u32) -> PropertyPath {
        PropertyPath::with_keys(sid(node), vec![PropertyKey(key)])
    }

    pub fn text(v: &str) -> TypedValue {
        TypedValue::String(v.to_string())
    }

    pub const ROOT: u8 = 1;
    pub const BUTTON: u8 = 2;
    pub const MODAL: u8 = 3;
    pub const LABEL: u8 = 4;
    pub const ICON: u8 = 5;
    pub const GRID: u8 = 6;
    pub const CELL: u8 = 7;
    pub const CLIPPED: u8 = 8;

    /// 프로젝트 profile: Web Baseline 2022, iOS 26, Android 30, Android 35(모두 release 1 pin).
    pub fn section() -> ProjectTargetProfiles {
        ProjectTargetProfiles::new(
            vec![
                web("web", 2022, 1),
                apple("ios", ApplePlatform::Ios, 26, 1),
                android("android-30", 30, 1),
                android("android-35", 35, 1),
            ],
            Some(id("web")),
        )
        .expect("절")
    }

    /// node 8개의 View.
    pub fn view() -> ViewModel {
        let node = |n: u8, parent: Option<u8>, element: ElementKind, contracts: &[ContractCode]| {
            ViewNode {
                id: sid(n),
                parent: parent.map(sid),
                element,
                contracts: contracts.to_vec(),
            }
        };
        let nodes = vec![
            node(ROOT, None, ElementKind::FRAME, &[]),
            node(
                BUTTON,
                Some(ROOT),
                ElementKind::FRAME,
                &[ContractCode::BUTTON],
            ),
            node(
                MODAL,
                Some(ROOT),
                ElementKind::FRAME,
                &[ContractCode::MODAL],
            ),
            node(LABEL, Some(MODAL), ElementKind::TEXT, &[]),
            node(ICON, Some(BUTTON), ElementKind::IMAGE, &[]),
            node(GRID, Some(ROOT), ElementKind::FRAME, &[ContractCode::LIST]),
            node(CELL, Some(GRID), ElementKind::FRAME, &[]),
            node(CLIPPED, Some(ROOT), ElementKind::SHAPE, &[]),
        ];
        let mut common = CommonValues::new();
        common.set(path(ROOT, KEY_RADIUS), text("8"));
        common.set(path(BUTTON, KEY_SHADOW), text("0 1 2"));
        common.set(path(BUTTON, KEY_SHADOW_COLOR), text("black"));
        common.set(path(MODAL, KEY_BACKDROP), text("blur(20)"));
        common.set(path(GRID, KEY_SUBGRID), text("rows"));
        common.set(path(CELL, KEY_RADIUS), text("4"));
        common.set(path(CLIPPED, KEY_CLIP_RECT), text("rect(0 0 10 10)"));
        common.set(path(LABEL, KEY_PLAIN), text("hello"));
        let mut overrides = ProfileOverrideSet::new();
        // iOS: icon만 SF Symbol 사용.
        overrides.profile_mut(&id("ios")).set(
            OverrideScope::Always,
            path(ICON, KEY_SF_SYMBOL),
            text("star"),
        );
        // Android 35: status bar color, grid 제외(하위 cell 포함), 미등록 capability 사용.
        let a35 = overrides.profile_mut(&id("android-35"));
        a35.set(
            OverrideScope::Always,
            path(ROOT, KEY_STATUS_BAR),
            text("#000"),
        );
        a35.set(OverrideScope::Always, path(ROOT, KEY_LEGACY), text("x"));
        a35.exclude_node(sid(GRID));
        // 프로젝트에 없는 profile의 override.
        overrides
            .profile_mut(&id("legacy-tv"))
            .exclude_node(sid(ROOT));
        ViewModel {
            nodes,
            common,
            overrides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::builtin::fixtures::web;
    use crate::builtin::{catalog, properties as p};
    use CapabilityDiagnosticCode::{
        Approximated as A, Deprecated as D, Native as N, NotRegistered as R, Unsupported as U,
    };
    use CapabilitySubject::{Contract as C, Element as E, Property as P};

    type Row = (u8, CapabilitySubject, Option<u32>, CapabilityDiagnosticCode);

    fn rows(report: &ProfileReport) -> Vec<Row> {
        report
            .findings()
            .iter()
            .map(|f| {
                (
                    f.node.to_bytes()[15],
                    f.subject,
                    f.path.as_ref().map(|path| path.keys()[0].0),
                    f.code,
                )
            })
            .collect()
    }

    #[test]
    fn multi_profile_fixture_produces_expected_diagnostics() {
        let catalog = catalog().expect("내장 catalog");
        let report = validate_view(
            &view(),
            &KeyMap,
            &section(),
            &catalog,
            &VariantLayer::none(),
        );
        assert_eq!(
            report
                .profiles
                .iter()
                .map(|r| r.profile.as_str())
                .collect::<Vec<_>>(),
            vec!["web", "ios", "android-30", "android-35"]
        );
        // 공통 node 순서: root, button, modal, label, icon, grid, cell, clipped.
        let web_expected: Vec<Row> = vec![
            (ROOT, E(ElementKind::FRAME), None, N),
            (ROOT, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (BUTTON, E(ElementKind::FRAME), None, N),
            (BUTTON, C(ContractCode::BUTTON), None, N),
            (BUTTON, P(p::DROP_SHADOW), Some(KEY_SHADOW), N),
            (MODAL, E(ElementKind::FRAME), None, N),
            (MODAL, C(ContractCode::MODAL), None, N),
            (MODAL, P(p::CSS_BACKDROP_FILTER), Some(KEY_BACKDROP), A),
            (LABEL, E(ElementKind::TEXT), None, N),
            (ICON, E(ElementKind::IMAGE), None, N),
            (GRID, E(ElementKind::FRAME), None, N),
            (GRID, C(ContractCode::LIST), None, N),
            (GRID, P(p::CSS_SUBGRID), Some(KEY_SUBGRID), U),
            (CELL, E(ElementKind::FRAME), None, N),
            (CELL, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (CLIPPED, E(ElementKind::SHAPE), None, N),
            (CLIPPED, P(p::CSS_CLIP_RECT), Some(KEY_CLIP_RECT), D),
        ];
        assert_eq!(rows(&report.profiles[0]), web_expected);

        let ios_expected: Vec<Row> = vec![
            (ROOT, E(ElementKind::FRAME), None, N),
            (ROOT, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (BUTTON, E(ElementKind::FRAME), None, N),
            (BUTTON, C(ContractCode::BUTTON), None, N),
            (BUTTON, P(p::DROP_SHADOW), Some(KEY_SHADOW), N),
            (MODAL, E(ElementKind::FRAME), None, N),
            (MODAL, C(ContractCode::MODAL), None, N),
            (MODAL, P(p::CSS_BACKDROP_FILTER), Some(KEY_BACKDROP), N),
            (LABEL, E(ElementKind::TEXT), None, N),
            (ICON, E(ElementKind::IMAGE), None, N),
            (ICON, P(p::APPLE_SF_SYMBOL), Some(KEY_SF_SYMBOL), N),
            (GRID, E(ElementKind::FRAME), None, N),
            (GRID, C(ContractCode::LIST), None, N),
            (GRID, P(p::CSS_SUBGRID), Some(KEY_SUBGRID), U),
            (CELL, E(ElementKind::FRAME), None, N),
            (CELL, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (CLIPPED, E(ElementKind::SHAPE), None, N),
            (CLIPPED, P(p::CSS_CLIP_RECT), Some(KEY_CLIP_RECT), A),
        ];
        assert_eq!(rows(&report.profiles[1]), ios_expected);

        let android30_expected: Vec<Row> = vec![
            (ROOT, E(ElementKind::FRAME), None, N),
            (ROOT, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (BUTTON, E(ElementKind::FRAME), None, N),
            (BUTTON, C(ContractCode::BUTTON), None, N),
            (BUTTON, P(p::DROP_SHADOW), Some(KEY_SHADOW), N),
            (MODAL, E(ElementKind::FRAME), None, N),
            (MODAL, C(ContractCode::MODAL), None, N),
            (MODAL, P(p::CSS_BACKDROP_FILTER), Some(KEY_BACKDROP), U),
            (LABEL, E(ElementKind::TEXT), None, N),
            (ICON, E(ElementKind::IMAGE), None, N),
            (GRID, E(ElementKind::FRAME), None, N),
            (GRID, C(ContractCode::LIST), None, N),
            (GRID, P(p::CSS_SUBGRID), Some(KEY_SUBGRID), U),
            (CELL, E(ElementKind::FRAME), None, N),
            (CELL, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (CLIPPED, E(ElementKind::SHAPE), None, N),
            (CLIPPED, P(p::CSS_CLIP_RECT), Some(KEY_CLIP_RECT), A),
        ];
        assert_eq!(rows(&report.profiles[2]), android30_expected);

        // Android 35: grid·cell 제외, status bar Deprecated, 미등록 capability.
        let android35_expected: Vec<Row> = vec![
            (ROOT, E(ElementKind::FRAME), None, N),
            (ROOT, P(p::CORNER_RADIUS), Some(KEY_RADIUS), N),
            (
                ROOT,
                P(p::ANDROID_STATUS_BAR_COLOR),
                Some(KEY_STATUS_BAR),
                D,
            ),
            (
                ROOT,
                P(CapabilityPropertyId(0x0009_0001)),
                Some(KEY_LEGACY),
                R,
            ),
            (BUTTON, E(ElementKind::FRAME), None, N),
            (BUTTON, C(ContractCode::BUTTON), None, N),
            (BUTTON, P(p::DROP_SHADOW), Some(KEY_SHADOW), N),
            (MODAL, E(ElementKind::FRAME), None, N),
            (MODAL, C(ContractCode::MODAL), None, N),
            (MODAL, P(p::CSS_BACKDROP_FILTER), Some(KEY_BACKDROP), U),
            (LABEL, E(ElementKind::TEXT), None, N),
            (ICON, E(ElementKind::IMAGE), None, N),
            (CLIPPED, E(ElementKind::SHAPE), None, N),
            (CLIPPED, P(p::CSS_CLIP_RECT), Some(KEY_CLIP_RECT), A),
        ];
        assert_eq!(rows(&report.profiles[3]), android35_expected);

        // 상태별 개수와 문제 목록.
        let web_report = &report.profiles[0];
        assert_eq!(
            [N, A, U, D, R].map(|code| web_report.count(code)),
            [14, 1, 1, 1, 0]
        );
        assert_eq!(web_report.problems().count(), 3);
        let deprecated = web_report
            .findings()
            .iter()
            .find(|f| f.code == D)
            .expect("deprecated");
        assert_eq!(deprecated.replacement, Some(P(p::CLIP_PATH)));
        assert_eq!(deprecated.code.severity(), Severity::Warning);
        assert_eq!(
            report.profiles[3]
                .findings()
                .iter()
                .find(|f| f.code == R)
                .map(|f| f.code.severity()),
            Some(Severity::Error)
        );
        assert_eq!(report.unknown_override_profiles, vec![id("legacy-tv")]);
        assert!(report.has_errors());
        // 결정론: 다시 검사해도 같습니다.
        assert_eq!(
            validate_view(
                &view(),
                &KeyMap,
                &section(),
                &catalog,
                &VariantLayer::none()
            ),
            report
        );
    }

    #[test]
    fn profile_pinned_to_unknown_release_is_reported_not_validated() {
        let catalog = catalog().expect("내장 catalog");
        let newer = web("web-next", 2024, 7);
        let report = validate_profiles(&view(), &KeyMap, &[newer], &catalog, &VariantLayer::none());
        assert_eq!(
            report.profiles[0].outcome,
            ProfileOutcome::RegistryUnavailable {
                pinned: ProfileRegistryVersion::new(7).expect("v"),
                latest: ProfileRegistryVersion::new(1).expect("v"),
            }
        );
        assert!(report.profiles[0].findings().is_empty());
        assert!(report.has_errors());
        // 문제 없는 View.
        let clean = ViewModel {
            nodes: vec![ViewNode {
                id: sid(1),
                parent: None,
                element: ElementKind::TEXT,
                contracts: vec![ContractCode::BUTTON, ContractCode::BUTTON],
            }],
            ..ViewModel::default()
        };
        let report = validate_profiles(
            &clean,
            &KeyMap,
            &[web("web", 2024, 1)],
            &catalog,
            &VariantLayer::none(),
        );
        assert_eq!(report.profiles[0].findings().len(), 2, "contract 중복 제거");
        assert!(!report.has_errors());
        assert_eq!(
            CapabilityDiagnosticCode::for_status(None),
            CapabilityDiagnosticCode::NotRegistered
        );
    }
}
