//! 공통 구조 + profile별 희소 override resolver (IX.10.7).
//!
//! 하나의 공통 구조(Base 값)를 원본으로 두고 Target Profile별 차이만 희소 override로 저장합니다(기술 아키텍처 17.2).
//! 구조 override는 StableId 기반입니다(node 제외).
//!
//! 우선순위(뒤가 이김):
//!
//! 1. **공통값**([`CommonValues`]).
//! 2. **profile override**([`OverrideScope::Always`]).
//! 3. **View Variant override**([`VariantLayer`] — IX.7.4~IX.7.6 resolver가 path마다 정한 승자 값).
//! 4. **profile × variant override**([`OverrideScope::Variant`]) — 활성 variant 적용 순서(일반 → 구체)대로, 뒤가 이김.
//!
//! 즉 상태(variant) 변화는 모든 profile에 적용되고, profile만의 상태 차이는 profile × variant override로 표현합니다.
//! 활성이 아닌 variant의 profile × variant override는 적용하지 않습니다.
//!
//! [`resolve_for_profile`]은 결과와 함께 선택 profile의 Binding 환경([`ProfileEnvironment`])을 돌려줍니다. 결과의 모든
//! 값은 출처([`ValueOrigin`])를 가집니다. 제외된 node를 root로 하는 값은 결과에서 빠집니다(하위 node 제외 판정은
//! [`ResolvedView::is_excluded`]가 parent 조회로 하고, View 전체 판정은 결과를 재사용하는
//! `ResolvedView::excluded_among`이 합니다).

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;

use crate::environment::ProfileEnvironment;
use crate::manifest::ProjectTargetProfiles;
use crate::profile::TargetProfileId;
use crate::spec::TargetProfile;

/// 공통 구조 값(Base).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CommonValues {
    values: BTreeMap<PropertyPath, TypedValue>,
}

impl CommonValues {
    /// 빈 값 표.
    pub fn new() -> CommonValues {
        CommonValues::default()
    }

    /// 값을 설정합니다(이전 값 반환).
    pub fn set(&mut self, path: PropertyPath, value: TypedValue) -> Option<TypedValue> {
        self.values.insert(path, value)
    }

    /// path의 값.
    pub fn get(&self, path: &PropertyPath) -> Option<&TypedValue> {
        self.values.get(path)
    }

    /// 모든 값(path 오름차순).
    pub fn iter(&self) -> impl Iterator<Item = (&PropertyPath, &TypedValue)> {
        self.values.iter()
    }
}

/// profile override 적용 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideScope {
    /// 항상(variant와 무관).
    Always,
    /// 해당 View Variant가 활성일 때(variant StableId).
    Variant(StableId),
}

/// profile 하나의 희소 override.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ProfileOverrides {
    values: BTreeMap<(OverrideScope, PropertyPath), TypedValue>,
    excluded_nodes: BTreeSet<StableId>,
}

impl ProfileOverrides {
    /// override 값을 설정합니다(이전 값 반환).
    pub fn set(
        &mut self,
        scope: OverrideScope,
        path: PropertyPath,
        value: TypedValue,
    ) -> Option<TypedValue> {
        self.values.insert((scope, path), value)
    }

    /// override 값을 지웁니다(지운 값 반환).
    pub fn clear(&mut self, scope: OverrideScope, path: &PropertyPath) -> Option<TypedValue> {
        self.values.remove(&(scope, path.clone()))
    }

    /// override 값.
    pub fn get(&self, scope: OverrideScope, path: &PropertyPath) -> Option<&TypedValue> {
        self.values.get(&(scope, path.clone()))
    }

    /// 모든 override((범위, path) 오름차순).
    pub fn iter(&self) -> impl Iterator<Item = (OverrideScope, &PropertyPath, &TypedValue)> {
        self.values
            .iter()
            .map(|((scope, path), value)| (*scope, path, value))
    }

    /// node를 이 profile에서 제외합니다(새로 제외했으면 `true`).
    pub fn exclude_node(&mut self, node: StableId) -> bool {
        self.excluded_nodes.insert(node)
    }

    /// node 제외를 해제합니다(해제했으면 `true`).
    pub fn include_node(&mut self, node: StableId) -> bool {
        self.excluded_nodes.remove(&node)
    }

    /// 명시적으로 제외한 node.
    pub fn excluded_nodes(&self) -> &BTreeSet<StableId> {
        &self.excluded_nodes
    }

    /// override가 하나도 없는지.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.excluded_nodes.is_empty()
    }
}

/// View의 profile별 override 표.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ProfileOverrideSet {
    by_profile: BTreeMap<TargetProfileId, ProfileOverrides>,
}

impl ProfileOverrideSet {
    /// 빈 표.
    pub fn new() -> ProfileOverrideSet {
        ProfileOverrideSet::default()
    }

    /// profile의 override(없으면 `None`).
    pub fn profile(&self, id: &TargetProfileId) -> Option<&ProfileOverrides> {
        self.by_profile.get(id)
    }

    /// profile의 override를 수정용으로 얻습니다(없으면 빈 항목을 만듦).
    pub fn profile_mut(&mut self, id: &TargetProfileId) -> &mut ProfileOverrides {
        self.by_profile.entry(id.clone()).or_default()
    }

    /// 빈 profile 항목을 지웁니다(canonical 형태).
    pub fn prune_empty(&mut self) {
        self.by_profile.retain(|_, overrides| !overrides.is_empty());
    }

    /// 모든 (profile, override)(id 오름차순).
    pub fn iter(&self) -> impl Iterator<Item = (&TargetProfileId, &ProfileOverrides)> {
        self.by_profile.iter()
    }

    /// 프로젝트 profile 목록에 없는 profile의 override(id 오름차순, 비어 있지 않은 항목만).
    pub fn unknown_profiles(&self, section: &ProjectTargetProfiles) -> Vec<TargetProfileId> {
        self.by_profile
            .iter()
            .filter(|(id, overrides)| !overrides.is_empty() && section.get(id).is_none())
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// View Variant resolver(IX.7.4~IX.7.6) 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct VariantLayer {
    active: Vec<StableId>,
    values: BTreeMap<PropertyPath, (StableId, TypedValue)>,
}

impl VariantLayer {
    /// 활성 variant가 없는 층.
    pub fn none() -> VariantLayer {
        VariantLayer::default()
    }

    /// 활성 variant(적용 순서: 일반 → 구체)와 path별 승자 값(값을 정한 variant, 값)으로 만듭니다.
    pub fn new(
        active: Vec<StableId>,
        values: BTreeMap<PropertyPath, (StableId, TypedValue)>,
    ) -> VariantLayer {
        VariantLayer { active, values }
    }

    /// 활성 variant(적용 순서).
    pub fn active(&self) -> &[StableId] {
        &self.active
    }
}

/// 해석 값의 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueOrigin {
    /// 공통값.
    Common,
    /// profile override.
    Profile,
    /// View Variant override(variant).
    Variant(StableId),
    /// profile × variant override(variant).
    ProfileVariant(StableId),
}

/// 해석 값.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedValue {
    /// 값.
    pub value: TypedValue,
    /// 출처.
    pub origin: ValueOrigin,
}

/// profile 하나로 해석한 View 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedView {
    /// profile.
    pub profile: TargetProfileId,
    /// path별 값.
    pub values: BTreeMap<PropertyPath, ResolvedValue>,
    /// profile에서 명시적으로 제외한 node.
    pub excluded_nodes: BTreeSet<StableId>,
    /// Binding 환경 입력.
    pub environment: ProfileEnvironment,
}

impl ResolvedView {
    /// node가 제외되었는지(자신 또는 조상이 제외). `parent_of`로 부모를 찾고, 순환에 대비해 `max_depth` 단계까지만
    /// 올라갑니다(넘으면 제외되지 않은 것으로 봄).
    pub fn is_excluded(
        &self,
        node: StableId,
        parent_of: impl Fn(StableId) -> Option<StableId>,
        max_depth: usize,
    ) -> bool {
        let mut current = Some(node);
        for _ in 0..=max_depth {
            let Some(id) = current else {
                return false;
            };
            if self.excluded_nodes.contains(&id) {
                return true;
            }
            current = parent_of(id);
        }
        false
    }

    /// `nodes` 중 제외된 node(자신 또는 조상이 제외)의 집합. node마다 [`ResolvedView::is_excluded`]를 부르는 것과 같은
    /// 판정이지만(`max_depth` = node 수), 한 번 판정한 node의 결과를 재사용해 `parent_of`를 node마다 최대 한 번만
    /// 부릅니다. 깊이 D인 트리에서 node마다 root까지 올라가는 O(N·D) 대신 O(N log N)입니다. 제외 node가 없는 순환에
    /// 걸린 node는 제외되지 않은 것으로 봅니다(`is_excluded`와 같음).
    pub fn excluded_among(
        &self,
        nodes: impl IntoIterator<Item = StableId>,
        parent_of: impl Fn(StableId) -> Option<StableId>,
    ) -> BTreeSet<StableId> {
        let mut known: BTreeMap<StableId, bool> = BTreeMap::new();
        let mut chain: Vec<StableId> = Vec::new();
        let mut on_chain: BTreeSet<StableId> = BTreeSet::new();
        let mut excluded = BTreeSet::new();
        for node in nodes {
            chain.clear();
            on_chain.clear();
            let mut current = Some(node);
            let verdict = loop {
                let Some(id) = current else {
                    break false;
                };
                if self.excluded_nodes.contains(&id) {
                    break true;
                }
                if let Some(&known_verdict) = known.get(&id) {
                    break known_verdict;
                }
                if !on_chain.insert(id) {
                    // 제외 node 없는 순환.
                    break false;
                }
                chain.push(id);
                current = parent_of(id);
            };
            for id in chain.drain(..) {
                known.insert(id, verdict);
            }
            if verdict {
                excluded.insert(node);
            }
        }
        excluded
    }
}

/// 공통값에 선택 profile의 override와 variant 층을 우선순위대로 적용합니다(모듈 문서).
pub fn resolve_for_profile(
    common: &CommonValues,
    overrides: &ProfileOverrideSet,
    profile: &TargetProfile,
    variants: &VariantLayer,
) -> ResolvedView {
    let profile_overrides = overrides.profile(&profile.id);
    let mut values: BTreeMap<PropertyPath, ResolvedValue> = common
        .iter()
        .map(|(path, value)| {
            (
                path.clone(),
                ResolvedValue {
                    value: value.clone(),
                    origin: ValueOrigin::Common,
                },
            )
        })
        .collect();
    let mut apply = |path: &PropertyPath, value: &TypedValue, origin: ValueOrigin| {
        values.insert(
            path.clone(),
            ResolvedValue {
                value: value.clone(),
                origin,
            },
        );
    };
    // override를 한 번만 훑어 `Always`와 활성 variant별 묶음으로 나눕니다(활성 variant마다 전체를 다시 훑지 않음).
    let active: BTreeSet<StableId> = variants.active.iter().copied().collect();
    let mut by_variant: BTreeMap<StableId, Vec<(&PropertyPath, &TypedValue)>> = BTreeMap::new();
    if let Some(po) = profile_overrides {
        for (scope, path, value) in po.iter() {
            match scope {
                OverrideScope::Always => apply(path, value, ValueOrigin::Profile),
                OverrideScope::Variant(variant) if active.contains(&variant) => {
                    by_variant.entry(variant).or_default().push((path, value));
                }
                OverrideScope::Variant(_) => {}
            }
        }
    }
    for (path, (variant, value)) in &variants.values {
        apply(path, value, ValueOrigin::Variant(*variant));
    }
    for variant in &variants.active {
        for (path, value) in by_variant.get(variant).into_iter().flatten() {
            apply(path, value, ValueOrigin::ProfileVariant(*variant));
        }
    }
    let excluded_nodes = profile_overrides
        .map(|po| po.excluded_nodes.clone())
        .unwrap_or_default();
    values.retain(|path, _| !excluded_nodes.contains(&path.root()));
    ResolvedView {
        profile: profile.id.clone(),
        values,
        excluded_nodes,
        environment: ProfileEnvironment::for_profile(Some(profile)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::fixtures::{android, apple, web};
    use crate::environment::{changed_keys, EnvironmentKeyCode};
    use crate::spec::ApplePlatform;
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::value::FiniteF32;

    fn sid(n: u8) -> StableId {
        let mut bytes = [0u8; 16];
        bytes[15] = n;
        StableId::from_bytes(bytes)
    }

    fn path(node: u8, key: u32) -> PropertyPath {
        PropertyPath::with_keys(sid(node), vec![PropertyKey(key)])
    }

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("finite"))
    }

    fn text(v: &str) -> TypedValue {
        TypedValue::String(v.to_string())
    }

    const NODE: u8 = 1;
    const BADGE: u8 = 2;
    const COMPACT: u8 = 100;
    const ERROR: u8 = 101;
    const RADIUS: u32 = 1;
    const PADDING: u32 = 2;
    const LABEL: u32 = 3;
    const FILL: u32 = 4;
    const SHADOW: u32 = 5;

    struct Fixture {
        common: CommonValues,
        overrides: ProfileOverrideSet,
        ios: TargetProfile,
        site: TargetProfile,
        droid: TargetProfile,
    }

    fn fixture() -> Fixture {
        let ios = apple("ios", ApplePlatform::Ios, 26, 1);
        let site = web("web", 2024, 1);
        let droid = android("android", 35, 1);
        let mut common = CommonValues::new();
        common.set(path(NODE, FILL), text("red"));
        common.set(path(NODE, RADIUS), num(4.0));
        common.set(path(NODE, PADDING), num(16.0));
        common.set(path(NODE, LABEL), text("A"));
        common.set(path(BADGE, FILL), text("blue"));
        let mut overrides = ProfileOverrideSet::new();
        let apple_overrides = overrides.profile_mut(&ios.id);
        apple_overrides.set(OverrideScope::Always, path(NODE, RADIUS), num(10.0));
        apple_overrides.set(OverrideScope::Always, path(NODE, PADDING), num(12.0));
        apple_overrides.set(OverrideScope::Always, path(NODE, SHADOW), text("soft"));
        apple_overrides.set(
            OverrideScope::Variant(sid(COMPACT)),
            path(NODE, LABEL),
            text("C"),
        );
        apple_overrides.set(
            OverrideScope::Variant(sid(ERROR)),
            path(NODE, LABEL),
            text("E-apple"),
        );
        overrides.profile_mut(&droid.id).exclude_node(sid(BADGE));
        Fixture {
            common,
            overrides,
            ios,
            site,
            droid,
        }
    }

    fn compact_layer() -> VariantLayer {
        let mut values = BTreeMap::new();
        values.insert(path(NODE, PADDING), (sid(COMPACT), num(8.0)));
        values.insert(path(NODE, LABEL), (sid(COMPACT), text("B")));
        VariantLayer::new(vec![sid(COMPACT)], values)
    }

    fn get(view: &ResolvedView, p: PropertyPath) -> Option<(TypedValue, ValueOrigin)> {
        view.values
            .get(&p)
            .map(|resolved| (resolved.value.clone(), resolved.origin))
    }

    #[test]
    fn common_profile_and_variant_priority() {
        let f = fixture();
        let compact = compact_layer();
        // Apple + compact: 공통 < profile < variant < profile×variant.
        let view = resolve_for_profile(&f.common, &f.overrides, &f.ios, &compact);
        assert_eq!(
            get(&view, path(NODE, FILL)),
            Some((text("red"), ValueOrigin::Common))
        );
        assert_eq!(
            get(&view, path(NODE, RADIUS)),
            Some((num(10.0), ValueOrigin::Profile))
        );
        assert_eq!(
            get(&view, path(NODE, PADDING)),
            Some((num(8.0), ValueOrigin::Variant(sid(COMPACT))))
        );
        assert_eq!(
            get(&view, path(NODE, LABEL)),
            Some((text("C"), ValueOrigin::ProfileVariant(sid(COMPACT))))
        );
        assert_eq!(
            get(&view, path(NODE, SHADOW)),
            Some((text("soft"), ValueOrigin::Profile))
        );
        // Web + compact: profile override가 없으므로 공통값과 variant만.
        let view = resolve_for_profile(&f.common, &f.overrides, &f.site, &compact);
        assert_eq!(
            get(&view, path(NODE, RADIUS)),
            Some((num(4.0), ValueOrigin::Common))
        );
        assert_eq!(
            get(&view, path(NODE, PADDING)),
            Some((num(8.0), ValueOrigin::Variant(sid(COMPACT))))
        );
        assert_eq!(
            get(&view, path(NODE, LABEL)),
            Some((text("B"), ValueOrigin::Variant(sid(COMPACT))))
        );
        assert_eq!(get(&view, path(NODE, SHADOW)), None);
        // Apple, variant 없음: 활성이 아닌 variant의 profile×variant override는 적용하지 않습니다.
        let view = resolve_for_profile(&f.common, &f.overrides, &f.ios, &VariantLayer::none());
        assert_eq!(
            get(&view, path(NODE, PADDING)),
            Some((num(12.0), ValueOrigin::Profile))
        );
        assert_eq!(
            get(&view, path(NODE, LABEL)),
            Some((text("A"), ValueOrigin::Common))
        );
    }

    #[test]
    fn later_active_variant_wins_in_profile_variant_layer() {
        let f = fixture();
        let mut values = BTreeMap::new();
        values.insert(path(NODE, LABEL), (sid(ERROR), text("E")));
        // 적용 순서: compact(일반) → error(구체).
        let layer = VariantLayer::new(vec![sid(COMPACT), sid(ERROR)], values);
        let view = resolve_for_profile(&f.common, &f.overrides, &f.ios, &layer);
        assert_eq!(
            get(&view, path(NODE, LABEL)),
            Some((text("E-apple"), ValueOrigin::ProfileVariant(sid(ERROR))))
        );
        let view = resolve_for_profile(&f.common, &f.overrides, &f.site, &layer);
        assert_eq!(
            get(&view, path(NODE, LABEL)),
            Some((text("E"), ValueOrigin::Variant(sid(ERROR))))
        );
    }

    #[test]
    fn excluded_among_matches_is_excluded_with_linear_parent_lookups() {
        use std::cell::Cell;
        let wide = |n: u32| {
            let mut bytes = [0u8; 16];
            bytes[12..].copy_from_slice(&n.to_be_bytes());
            StableId::from_bytes(bytes)
        };
        // 깊은 선형 트리(0 ← 1 ← … ← N-1)에서 node 하나(중간)를 제외합니다.
        const N: u32 = 2_000;
        let mut overrides = ProfileOverrideSet::new();
        let f = fixture();
        overrides.profile_mut(&f.droid.id).exclude_node(wide(N / 2));
        let view = resolve_for_profile(&f.common, &overrides, &f.droid, &VariantLayer::none());
        let calls = Cell::new(0usize);
        let parent = |id: StableId| {
            calls.set(calls.get() + 1);
            let n = u32::from_be_bytes(id.to_bytes()[12..].try_into().expect("4 bytes"));
            (n > 0 && n < N).then(|| wide(n - 1))
        };
        let excluded = view.excluded_among((0..N).map(wide), parent);
        // 제외 node와 그 하위 node만 제외됩니다.
        assert_eq!(excluded, (N / 2..N).map(wide).collect::<BTreeSet<_>>());
        // parent 조회는 node마다 최대 한 번입니다(조상마다 다시 올라가지 않음).
        assert!(
            calls.get() <= N as usize,
            "parent 조회 {}회 > node {N}개",
            calls.get()
        );

        // 순환·모르는 parent·제외된 모르는 조상에서도 is_excluded와 같은 판정입니다.
        let mut overrides = ProfileOverrideSet::new();
        overrides.profile_mut(&f.droid.id).exclude_node(sid(50));
        overrides.profile_mut(&f.droid.id).exclude_node(sid(7));
        let view = resolve_for_profile(&f.common, &overrides, &f.droid, &VariantLayer::none());
        let parent = |id: StableId| match id.to_bytes()[15] {
            1 => Some(sid(2)),
            2 => Some(sid(1)), // 제외 없는 순환
            3 => Some(sid(4)),
            4 => Some(sid(5)),
            5 => Some(sid(3)), // 순환
            6 => Some(sid(7)),
            7 => Some(sid(6)),  // 제외 node가 있는 순환
            8 => Some(sid(50)), // 목록에 없는 제외 조상
            9 => Some(sid(60)), // 목록에 없는 조상
            10 => Some(sid(8)),
            _ => None,
        };
        let nodes: Vec<StableId> = (1..=12).map(sid).collect();
        let bulk = view.excluded_among(nodes.iter().copied(), parent);
        let one_by_one: BTreeSet<StableId> = nodes
            .iter()
            .copied()
            .filter(|n| view.is_excluded(*n, parent, nodes.len()))
            .collect();
        assert_eq!(bulk, one_by_one);
        assert_eq!(bulk, [6, 7, 8, 10].map(sid).into_iter().collect());
    }

    #[test]
    fn excluded_nodes_and_environment_follow_profile() {
        let f = fixture();
        let none = VariantLayer::none();
        let droid = resolve_for_profile(&f.common, &f.overrides, &f.droid, &none);
        assert_eq!(get(&droid, path(BADGE, FILL)), None);
        assert!(droid.excluded_nodes.contains(&sid(BADGE)));
        // 하위 node 제외: BADGE의 자식 3, 손자 4.
        let parent = |id: StableId| match id.to_bytes()[15] {
            4 => Some(sid(3)),
            3 => Some(sid(BADGE)),
            _ => None,
        };
        assert!(droid.is_excluded(sid(4), parent, 16));
        assert!(!droid.is_excluded(sid(NODE), parent, 16));
        // 순환 parent에서도 끝납니다.
        assert!(!droid.is_excluded(sid(9), |_| Some(sid(9)), 16));
        let ios = resolve_for_profile(&f.common, &f.overrides, &f.ios, &none);
        assert_eq!(
            get(&ios, path(BADGE, FILL)),
            Some((text("blue"), ValueOrigin::Common))
        );
        // 환경: profile 전환 시 바뀐 key만.
        let changed: Vec<u32> = changed_keys(&ios.environment, &droid.environment)
            .into_iter()
            .map(|k| k.0)
            .collect();
        assert_eq!(changed, vec![1, 2, 3]);
        assert_eq!(
            droid.environment.get(EnvironmentKeyCode::API_LEVEL),
            Some(&num(35.0))
        );
    }

    #[test]
    fn override_set_bookkeeping() {
        let f = fixture();
        let mut overrides = f.overrides.clone();
        let section =
            ProjectTargetProfiles::new(vec![f.ios.clone()], Some(f.ios.id.clone())).expect("절");
        // 프로젝트에 없는 android override는 보고합니다.
        assert_eq!(
            overrides.unknown_profiles(&section),
            vec![f.droid.id.clone()]
        );
        let po = overrides.profile_mut(&f.droid.id);
        assert!(po.include_node(sid(BADGE)));
        assert!(po.is_empty());
        assert!(overrides.unknown_profiles(&section).is_empty());
        overrides.prune_empty();
        assert!(overrides.profile(&f.droid.id).is_none());
        let apple_po = overrides.profile_mut(&f.ios.id);
        assert_eq!(
            apple_po.clear(OverrideScope::Always, &path(NODE, RADIUS)),
            Some(num(10.0))
        );
        assert_eq!(
            apple_po.get(OverrideScope::Always, &path(NODE, RADIUS)),
            None
        );
    }
}
