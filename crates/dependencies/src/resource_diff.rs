//! library resource Semantic Diff (XV.7.2).
//!
//! 같은 library의 두 snapshot(현재·후보, XV.7.1)을 **ComponentDefinition·Property·Token 단위**로
//! 비교합니다. 비교 기준은 StableId입니다. 이름이 바뀌어도 같은 StableId면 같은 resource의 변경
//! (`Modified { Name }`)이고, 전체 blob 변경으로 축약하지 않습니다(XIV.3.7과 같은 원칙).
//!
//! | 단위 | 판정 |
//! |---|---|
//! | component | 추가·삭제, 또는 이름·slot·외부 참조·property 순서 변경 |
//! | property | 추가·삭제, 또는 이름·타입·default·공개 여부 변경(component가 양쪽에 있을 때만) |
//! | TokenSet | 추가·삭제 |
//! | token | 추가·삭제, 또는 이름·타입·값·외부 참조·소속 TokenSet 변경 |
//!
//! component 자체가 추가·삭제되면 그 property는 따로 나열하지 않습니다(instance override는 component
//! 단위로 orphan). token은 component와 독립적으로 참조되므로 TokenSet이 사라져도 token마다 삭제로
//! 나열합니다. 결과는 결정적 순서입니다: component(그 아래 property) → TokenSet → token, 각 StableId 순.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::declaration::{CommitId, LibraryId};
use crate::external_ref::ExternalResourceRef;
use crate::library_snapshot::{
    ExportedComponent, ExportedProperty, ExportedToken, LibrarySnapshot,
};

/// 변경 단위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceUnit {
    /// ComponentDefinition.
    Component {
        /// component StableId.
        component: StableId,
    },
    /// component property.
    Property {
        /// 소속 component.
        component: StableId,
        /// property StableId.
        property: StableId,
    },
    /// TokenSet.
    TokenSet {
        /// TokenSet StableId.
        set: StableId,
    },
    /// TokenDefinition.
    Token {
        /// token StableId.
        token: StableId,
    },
}

impl ResourceUnit {
    /// 정렬 key: component(그 아래 property) → TokenSet → token.
    fn sort_key(&self) -> (u8, StableId, u8, StableId) {
        match *self {
            ResourceUnit::Component { component } => (0, component, 0, StableId::NIL),
            ResourceUnit::Property {
                component,
                property,
            } => (0, component, 1, property),
            ResourceUnit::TokenSet { set } => (1, set, 0, StableId::NIL),
            ResourceUnit::Token { token } => (2, token, 0, StableId::NIL),
        }
    }
}

impl PartialOrd for ResourceUnit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ResourceUnit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl fmt::Display for ResourceUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceUnit::Component { component } => write!(f, "component {component}"),
            ResourceUnit::Property {
                component,
                property,
            } => write!(f, "component {component} property {property}"),
            ResourceUnit::TokenSet { set } => write!(f, "token set {set}"),
            ResourceUnit::Token { token } => write!(f, "token {token}"),
        }
    }
}

/// 바뀐 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangedField {
    /// 표시 이름.
    Name,
    /// 타입 표기.
    Type,
    /// default expression(property).
    Default,
    /// 공개 여부(property).
    Visibility,
    /// 값(token default expression).
    Value,
    /// slot 목록(component).
    Slots,
    /// property 선언 순서(component, 같은 property 집합).
    PropertyOrder,
    /// 외부 참조 집합.
    References,
    /// 소속 TokenSet(token).
    TokenSet,
}

/// 변경 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// 후보에만 있음.
    Added,
    /// 현재에만 있음.
    Removed,
    /// 양쪽에 있고 필드가 바뀜(비어 있지 않음).
    Modified(BTreeSet<ChangedField>),
}

/// 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceChange {
    /// 단위.
    pub unit: ResourceUnit,
    /// 종류.
    pub kind: ChangeKind,
}

/// 두 snapshot 사이의 resource diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryResourceDiff {
    /// library.
    pub library_id: LibraryId,
    /// 현재 commit.
    pub from: CommitId,
    /// 후보 commit.
    pub to: CommitId,
    /// 변경(결정적 순서).
    pub changes: Vec<ResourceChange>,
}

impl LibraryResourceDiff {
    /// 변경이 없는지.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// 단위의 변경.
    pub fn change_for(&self, unit: &ResourceUnit) -> Option<&ChangeKind> {
        self.changes
            .binary_search_by(|c| c.unit.cmp(unit))
            .ok()
            .map(|i| &self.changes[i].kind)
    }

    /// 삭제된 property(현재 component가 후보에도 있을 때) 목록. update 전 override 검증(XV.7.3)의 입력입니다.
    pub fn removed_properties(&self) -> impl Iterator<Item = (StableId, StableId)> + '_ {
        self.changes.iter().filter_map(|c| match (c.unit, &c.kind) {
            (
                ResourceUnit::Property {
                    component,
                    property,
                },
                ChangeKind::Removed,
            ) => Some((component, property)),
            _ => None,
        })
    }
}

/// diff 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffError {
    /// 두 snapshot이 다른 library임.
    LibraryMismatch {
        /// 현재 library.
        from: LibraryId,
        /// 후보 library.
        to: LibraryId,
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffError::LibraryMismatch { from, to } => {
                write!(f, "서로 다른 library({from}, {to})는 비교할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for DiffError {}

fn reference_set(references: &[ExternalResourceRef]) -> BTreeSet<&ExternalResourceRef> {
    references.iter().collect()
}

fn component_fields(from: &ExportedComponent, to: &ExportedComponent) -> BTreeSet<ChangedField> {
    let mut fields = BTreeSet::new();
    if from.name != to.name {
        fields.insert(ChangedField::Name);
    }
    if from.slots != to.slots {
        fields.insert(ChangedField::Slots);
    }
    if reference_set(&from.references) != reference_set(&to.references) {
        fields.insert(ChangedField::References);
    }
    // 양쪽에 공통인 property의 상대 순서가 바뀌었는지(추가·삭제는 property 단위로 보고).
    let to_ids: BTreeSet<StableId> = to.properties.iter().map(|p| p.id).collect();
    let from_ids: BTreeSet<StableId> = from.properties.iter().map(|p| p.id).collect();
    let common_from: Vec<StableId> = from
        .properties
        .iter()
        .map(|p| p.id)
        .filter(|id| to_ids.contains(id))
        .collect();
    let common_to: Vec<StableId> = to
        .properties
        .iter()
        .map(|p| p.id)
        .filter(|id| from_ids.contains(id))
        .collect();
    if common_from != common_to {
        fields.insert(ChangedField::PropertyOrder);
    }
    fields
}

fn property_fields(from: &ExportedProperty, to: &ExportedProperty) -> BTreeSet<ChangedField> {
    let mut fields = BTreeSet::new();
    if from.name != to.name {
        fields.insert(ChangedField::Name);
    }
    if from.type_signature != to.type_signature {
        fields.insert(ChangedField::Type);
    }
    if from.default != to.default {
        fields.insert(ChangedField::Default);
    }
    if from.visibility != to.visibility {
        fields.insert(ChangedField::Visibility);
    }
    fields
}

fn token_fields(
    (from_set, from): (StableId, &ExportedToken),
    (to_set, to): (StableId, &ExportedToken),
) -> BTreeSet<ChangedField> {
    let mut fields = BTreeSet::new();
    if from.name != to.name {
        fields.insert(ChangedField::Name);
    }
    if from.type_signature != to.type_signature {
        fields.insert(ChangedField::Type);
    }
    if from.value != to.value {
        fields.insert(ChangedField::Value);
    }
    if reference_set(&from.references) != reference_set(&to.references) {
        fields.insert(ChangedField::References);
    }
    if from_set != to_set {
        fields.insert(ChangedField::TokenSet);
    }
    fields
}

/// StableId 합집합의 한 항목.
enum Pair<'a, V> {
    /// 양쪽에 있음.
    Both(&'a V, &'a V),
    /// 후보에만 있음.
    Added,
    /// 현재에만 있음.
    Removed,
}

/// 두 map을 StableId 합집합 순으로 짝짓습니다.
fn pair_up<'a, V>(
    from: &'a BTreeMap<StableId, V>,
    to: &'a BTreeMap<StableId, V>,
) -> Vec<(StableId, Pair<'a, V>)> {
    let ids: BTreeSet<StableId> = from.keys().chain(to.keys()).copied().collect();
    ids.into_iter()
        .filter_map(|id| match (from.get(&id), to.get(&id)) {
            (Some(a), Some(b)) => Some((id, Pair::Both(a, b))),
            (None, Some(_)) => Some((id, Pair::Added)),
            (Some(_), None) => Some((id, Pair::Removed)),
            (None, None) => None,
        })
        .collect()
}

fn components_of(snapshot: &LibrarySnapshot) -> BTreeMap<StableId, &ExportedComponent> {
    snapshot
        .components()
        .map(|c| (c.export().id.id(), c.export()))
        .collect()
}

fn properties_of(component: &ExportedComponent) -> BTreeMap<StableId, &ExportedProperty> {
    component.properties.iter().map(|p| (p.id, p)).collect()
}

fn tokens_of(snapshot: &LibrarySnapshot) -> BTreeMap<StableId, (StableId, &ExportedToken)> {
    snapshot
        .tokens()
        .map(|t| (t.export().id.id(), (t.set_key().resource, t.export())))
        .collect()
}

/// 현재 snapshot에서 후보 snapshot으로의 resource diff를 계산합니다.
pub fn diff_library_snapshots(
    current: &LibrarySnapshot,
    candidate: &LibrarySnapshot,
) -> Result<LibraryResourceDiff, DiffError> {
    if current.library_id() != candidate.library_id() {
        return Err(DiffError::LibraryMismatch {
            from: current.library_id(),
            to: candidate.library_id(),
        });
    }
    let mut changes = Vec::new();
    let mut push = |unit: ResourceUnit, kind: ChangeKind| {
        if matches!(&kind, ChangeKind::Modified(fields) if fields.is_empty()) {
            return;
        }
        changes.push(ResourceChange { unit, kind });
    };

    let (from_components, to_components) = (components_of(current), components_of(candidate));
    for (component, pair) in pair_up(&from_components, &to_components) {
        let unit = ResourceUnit::Component { component };
        match pair {
            Pair::Both(a, b) => {
                push(unit, ChangeKind::Modified(component_fields(a, b)));
                let (from_props, to_props) = (properties_of(a), properties_of(b));
                for (property, pair) in pair_up(&from_props, &to_props) {
                    let unit = ResourceUnit::Property {
                        component,
                        property,
                    };
                    let kind = match pair {
                        Pair::Both(pa, pb) => ChangeKind::Modified(property_fields(pa, pb)),
                        Pair::Added => ChangeKind::Added,
                        Pair::Removed => ChangeKind::Removed,
                    };
                    push(unit, kind);
                }
            }
            Pair::Added => push(unit, ChangeKind::Added),
            Pair::Removed => push(unit, ChangeKind::Removed),
        }
    }

    let sets_of = |s: &LibrarySnapshot| -> BTreeMap<StableId, ()> {
        s.token_sets().map(|set| (set.id, ())).collect()
    };
    let (from_sets, to_sets) = (sets_of(current), sets_of(candidate));
    for (set, pair) in pair_up(&from_sets, &to_sets) {
        match pair {
            Pair::Both(..) => {}
            Pair::Added => push(ResourceUnit::TokenSet { set }, ChangeKind::Added),
            Pair::Removed => push(ResourceUnit::TokenSet { set }, ChangeKind::Removed),
        }
    }

    let (from_tokens, to_tokens) = (tokens_of(current), tokens_of(candidate));
    for (token, pair) in pair_up(&from_tokens, &to_tokens) {
        let kind = match pair {
            Pair::Both((sa, a), (sb, b)) => ChangeKind::Modified(token_fields((*sa, a), (*sb, b))),
            Pair::Added => ChangeKind::Added,
            Pair::Removed => ChangeKind::Removed,
        };
        push(ResourceUnit::Token { token }, kind);
    }

    changes.sort_by_key(|c| c.unit);
    Ok(LibraryResourceDiff {
        library_id: current.library_id(),
        from: current.commit().clone(),
        to: candidate.commit().clone(),
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::{commit, library};
    use crate::external_ref::fixtures::{component_ref, token_ref};
    use crate::graph::DependencyNodeKey;
    use crate::library_snapshot::fixtures::{component, property, rid, snapshot, token, token_set};
    use crate::library_snapshot::{ExportVisibility, ExportedTokenSet, LibraryExports};

    fn fields(list: &[ChangedField]) -> ChangeKind {
        ChangeKind::Modified(list.iter().copied().collect())
    }

    fn c(id: u16) -> ResourceUnit {
        ResourceUnit::Component { component: rid(id) }
    }

    fn p(component: u16, property: u16) -> ResourceUnit {
        ResourceUnit::Property {
            component: rid(component),
            property: rid(property),
        }
    }

    fn t(id: u16) -> ResourceUnit {
        ResourceUnit::Token { token: rid(id) }
    }

    fn set(id: u16) -> ResourceUnit {
        ResourceUnit::TokenSet { set: rid(id) }
    }

    fn v1() -> LibrarySnapshot {
        let mut button = component(
            1,
            "Button",
            vec![
                property(11, "label", "String", b"\"OK\""),
                property(12, "size", "Number", b"1"),
                property(13, "tone", "String", b"\"a\""),
                property(14, "legacy", "Bool", b"false"),
                property(15, "icon", "String", b"\"\""),
            ],
        );
        button.slots = vec![rid(16)];
        let mut card = component(2, "Card", vec![property(21, "title", "String", b"\"\"")]);
        card.references = vec![component_ref("icons", library(9), rid(90)).into()];
        let unchanged = component(3, "Divider", vec![property(31, "w", "Number", b"1")]);
        let removed = component(4, "Old", vec![property(41, "x", "Number", b"1")]);
        snapshot(
            DependencyNodeKey::new(library(1), commit(0x01)),
            LibraryExports {
                components: vec![button, card, unchanged, removed],
                token_sets: vec![
                    token_set(
                        50,
                        vec![
                            token(51, "brand", "Color", b"#f00"),
                            token(52, "ink", "Color", b"#000"),
                            token(53, "space", "Number", b"4"),
                            token(54, "same", "Number", b"8"),
                            token(55, "moving", "Number", b"2"),
                        ],
                    ),
                    token_set(60, vec![token(61, "gone", "Number", b"1")]),
                ],
            },
        )
    }

    fn v2() -> LibrarySnapshot {
        let mut size = property(12, "size", "String", b"1"); // 타입 변경
        size.visibility = ExportVisibility::Private; // 공개 여부 변경
        let mut button = component(
            1,
            "Button v2", // 이름 변경
            vec![
                property(13, "tone", "String", b"\"b\""), // default 변경, 순서 앞으로
                property(11, "caption", "String", b"\"OK\""), // 이름 변경
                size,
                property(15, "icon", "String", b"\"\""),
                property(17, "variant", "String", b"\"\""), // 추가
                                                            // 14(legacy) 삭제
            ],
        );
        button.slots = vec![rid(16), rid(18)]; // slot 추가
        let mut card = component(2, "Card", vec![property(21, "title", "String", b"\"\"")]);
        card.references = vec![component_ref("icons", library(9), rid(91)).into()]; // 참조 변경
        let unchanged = component(3, "Divider", vec![property(31, "w", "Number", b"1")]);
        let added = component(5, "New", vec![property(58, "y", "Number", b"1")]);
        let mut ink = token(52, "ink", "Color", b"#000");
        ink.references = vec![token_ref("base", library(8), rid(80)).into()];
        snapshot(
            DependencyNodeKey::new(library(1), commit(0x02)),
            LibraryExports {
                components: vec![button, card, unchanged, added],
                token_sets: vec![
                    token_set(
                        50,
                        vec![
                            token(51, "brand", "Color", b"#0f0"), // 값 변경
                            ink,                                  // 참조 변경
                            token(53, "spacing", "Number", b"4"), // 이름 변경
                            token(54, "same", "Number", b"8"),
                            token(56, "fresh", "Number", b"3"), // 추가
                        ],
                    ),
                    token_set(70, vec![token(55, "moving", "Number", b"2")]), // set 이동
                ],
            },
        )
    }

    #[test]
    fn resource_change_fixture_is_exact() {
        let diff = diff_library_snapshots(&v1(), &v2()).unwrap();
        assert_eq!(diff.library_id, library(1));
        assert_eq!(diff.from, commit(0x01));
        assert_eq!(diff.to, commit(0x02));
        let expected = vec![
            (
                c(1),
                fields(&[
                    ChangedField::Name,
                    ChangedField::Slots,
                    ChangedField::PropertyOrder,
                ]),
            ),
            (p(1, 11), fields(&[ChangedField::Name])),
            (
                p(1, 12),
                fields(&[ChangedField::Type, ChangedField::Visibility]),
            ),
            (p(1, 13), fields(&[ChangedField::Default])),
            (p(1, 14), ChangeKind::Removed),
            (p(1, 17), ChangeKind::Added),
            (c(2), fields(&[ChangedField::References])),
            (c(4), ChangeKind::Removed),
            (c(5), ChangeKind::Added),
            (set(60), ChangeKind::Removed),
            (set(70), ChangeKind::Added),
            (t(51), fields(&[ChangedField::Value])),
            (t(52), fields(&[ChangedField::References])),
            (t(53), fields(&[ChangedField::Name])),
            (t(55), fields(&[ChangedField::TokenSet])),
            (t(56), ChangeKind::Added),
            (t(61), ChangeKind::Removed),
        ];
        let actual: Vec<_> = diff
            .changes
            .iter()
            .map(|c| (c.unit, c.kind.clone()))
            .collect();
        assert_eq!(actual, expected);
        // 바뀌지 않은 component(3)·property(15, 21, 31)·token(54)은 없음, 삭제 component(4)의 property는 따로 없음.
        for unit in [
            c(3),
            p(1, 15),
            p(2, 21),
            p(3, 31),
            p(4, 41),
            p(5, 58),
            t(54),
        ] {
            assert_eq!(diff.change_for(&unit), None, "{unit}");
        }
        assert_eq!(diff.change_for(&p(1, 14)), Some(&ChangeKind::Removed));
        assert_eq!(
            diff.removed_properties().collect::<Vec<_>>(),
            [(rid(1), rid(14))]
        );
    }

    #[test]
    fn identical_snapshots_diff_empty_and_reverse_swaps_added_removed() {
        let same = diff_library_snapshots(&v1(), &v1()).unwrap();
        assert!(same.is_empty());

        let forward = diff_library_snapshots(&v1(), &v2()).unwrap();
        let backward = diff_library_snapshots(&v2(), &v1()).unwrap();
        assert_eq!(forward.changes.len(), backward.changes.len());
        for change in &forward.changes {
            let reversed = backward.change_for(&change.unit).expect("양방향 같은 단위");
            match (&change.kind, reversed) {
                (ChangeKind::Added, ChangeKind::Removed)
                | (ChangeKind::Removed, ChangeKind::Added) => {}
                (ChangeKind::Modified(a), ChangeKind::Modified(b)) => assert_eq!(a, b),
                other => panic!("{}: {other:?}", change.unit),
            }
        }
    }

    #[test]
    fn declaration_order_of_exports_does_not_change_diff() {
        let mut shuffled = v2();
        // 같은 내용을 다른 export 순서로 만든 snapshot(토큰 set 순서·component 순서 반대).
        let mut components: Vec<_> = shuffled.components().map(|c| c.export().clone()).collect();
        components.reverse();
        let mut token_sets: Vec<_> = shuffled
            .token_sets()
            .map(|s| ExportedTokenSet {
                id: s.id,
                tokens: s.tokens.iter().rev().cloned().collect(),
            })
            .collect();
        token_sets.reverse();
        let exports = LibraryExports {
            components,
            token_sets,
        };
        shuffled = snapshot(shuffled.node().clone(), exports);
        assert_eq!(
            diff_library_snapshots(&v1(), &v2()).unwrap(),
            diff_library_snapshots(&v1(), &shuffled).unwrap()
        );
    }

    #[test]
    fn different_libraries_are_not_compared() {
        let other = snapshot(
            DependencyNodeKey::new(library(2), commit(0x02)),
            LibraryExports::default(),
        );
        assert_eq!(
            diff_library_snapshots(&v1(), &other),
            Err(DiffError::LibraryMismatch {
                from: library(1),
                to: library(2),
            })
        );
    }
}
