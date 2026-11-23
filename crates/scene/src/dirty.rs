//! layout·binding·modifier invalidation → scene dirty set (XI.2.1, ADR-0084 결정 2).
//!
//! scene은 document·layout·commands를 import하지 않으므로(II.2.8), 하류가 알리는 무효화를 scene 쪽 값
//! ([`SceneInvalidation`])으로 받아 [`SceneDirtySet`]으로 바꿉니다. 변환 규칙은 다음과 같습니다.
//!
//! | 입력 | scene dirty |
//! |---|---|
//! | paint class만(색·불투명도·blend·shadow·blur·stroke 색) | [`SceneDirtyKind::Paint`] |
//! | geometry class(크기·path·모서리·transform·clip·표시·잠금) | [`SceneDirtyKind::Geometry`] |
//! | layout class(배치 결과가 바뀐 node) | [`SceneDirtyKind::Geometry`] |
//! | 구조 변경(자식 추가·삭제·이동·종류 변환) | 그 node의 [`SceneDirtyKind::Geometry`]. 자식 목록은 증분 builder가 비교합니다 |
//! | View Variant·컴포넌트 정의 교체, 해석 규칙을 모르는 변경 | [`SceneDirtyKind::Subtree`] |
//! | class 없음(이름 변경 등) | 없음 |
//!
//! - 같은 node에 여러 입력이 오면 더 큰 종류가 이깁니다(`Paint < Geometry < Subtree`).
//! - 다른 node의 geometry를 입력으로 쓰는 node(Boolean 결과, ADR-0084 결정 7)는 [`SceneDependents`]가 알려 줍니다.
//!   operand의 geometry 이상 dirty는 결과 node의 [`SceneDirtyKind::Geometry`]가 됩니다. paint만 바뀐 operand는 결과에
//!   영향이 없습니다(operand는 단독 표시하지 않고 결과는 host의 paint로 칠합니다).
//! - 조상의 transform·clip·표시·잠금이 바뀌어 자손의 world 값이 달라지는 전파는 dirty set이 아니라 증분 builder가
//!   scope 비교로 처리합니다([`crate::incremental`]). dirty set은 "자기 입력이 바뀐 node"만 담습니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;

/// node 하나의 scene dirty 종류. 순서가 있으며 큰 값이 작은 값을 포함합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SceneDirtyKind {
    /// paint stack만 바뀌었습니다. content·world·clip·hit 값은 그대로입니다.
    Paint,
    /// 자기 geometry·transform·clip·hit 값이 바뀌었습니다. 자손은 scope가 달라질 때만 다시 만듭니다.
    Geometry,
    /// subtree 전체를 다시 만듭니다(구조 변경·Variant·컴포넌트 정의 교체).
    Subtree,
}

impl SceneDirtyKind {
    /// 진단용 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            SceneDirtyKind::Paint => "paint",
            SceneDirtyKind::Geometry => "geometry",
            SceneDirtyKind::Subtree => "subtree",
        }
    }
}

/// document 속성 metadata의 무효화 class 집합(VI.9.7과 같은 class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InvalidationClasses {
    /// 평가된 geometry 또는 visual bounds.
    pub geometry: bool,
    /// layout 입력.
    pub layout: bool,
    /// paint.
    pub paint: bool,
}

impl InvalidationClasses {
    /// paint만.
    pub const PAINT: InvalidationClasses = InvalidationClasses {
        geometry: false,
        layout: false,
        paint: true,
    };

    /// geometry + paint.
    pub const GEOMETRY: InvalidationClasses = InvalidationClasses {
        geometry: true,
        layout: false,
        paint: true,
    };

    /// 구조 변경(geometry + layout + paint).
    pub const STRUCTURE: InvalidationClasses = InvalidationClasses {
        geometry: true,
        layout: true,
        paint: true,
    };

    /// 비어 있는지.
    pub const fn is_empty(self) -> bool {
        !self.geometry && !self.layout && !self.paint
    }
}

/// 무효화 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvalidationOrigin {
    /// command가 node 속성을 바꿨습니다(VII.1.4 change set).
    Property,
    /// binding 재평가 결과 값이 바뀌었습니다(VIII).
    Binding,
    /// modifier stack 속성이 바뀌었습니다(VI.9.7).
    Modifier,
    /// 증분 layout 결과 rect가 바뀌었습니다(X.7).
    Layout,
}

/// scene이 받는 무효화 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneInvalidation {
    /// 속성·binding·modifier·layout 변경.
    Node {
        /// 바뀐 node.
        node: StableId,
        /// 출처.
        origin: InvalidationOrigin,
        /// 무효화 class.
        classes: InvalidationClasses,
    },
    /// layout 결과: 위치·크기 변경 여부. 둘 다 거짓이면 dirty가 아닙니다.
    LayoutRect {
        /// 바뀐 node.
        node: StableId,
        /// 위치(부모 좌표 원점)가 바뀌었는지.
        moved: bool,
        /// border box 크기가 바뀌었는지.
        resized: bool,
    },
    /// 잠금(hit test 대상) 전환. document 무효화 class는 비어 있지만 scene hit metadata가 바뀝니다.
    HitTarget {
        /// 바뀐 node.
        node: StableId,
    },
    /// 자식 구성 변경(삽입·삭제·이동·종류 변환).
    Structure {
        /// 자식이 바뀐 부모(또는 종류가 바뀐 node).
        node: StableId,
    },
    /// subtree를 다시 펼칩니다(View Variant 교체, 컴포넌트 정의 교체).
    Subtree {
        /// subtree root.
        node: StableId,
    },
}

impl SceneInvalidation {
    /// 입력 node와 scene dirty 종류. dirty가 아니면 `None`.
    pub fn classify(&self) -> Option<(StableId, SceneDirtyKind)> {
        match *self {
            SceneInvalidation::Node { node, classes, .. } => {
                if classes.geometry || classes.layout {
                    Some((node, SceneDirtyKind::Geometry))
                } else if classes.paint {
                    Some((node, SceneDirtyKind::Paint))
                } else {
                    None
                }
            }
            SceneInvalidation::LayoutRect {
                node,
                moved,
                resized,
            } => (moved || resized).then_some((node, SceneDirtyKind::Geometry)),
            SceneInvalidation::HitTarget { node } | SceneInvalidation::Structure { node } => {
                Some((node, SceneDirtyKind::Geometry))
            }
            SceneInvalidation::Subtree { node } => Some((node, SceneDirtyKind::Subtree)),
        }
    }
}

/// 다른 node의 geometry를 입력으로 쓰는 node 조회(Boolean 결과 등).
pub trait SceneDependents {
    /// `node`의 geometry를 읽는 node들.
    fn dependents_of(&self, node: StableId) -> Vec<StableId>;
}

/// 의존 node가 없는 조회.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDependents;

impl SceneDependents for NoDependents {
    fn dependents_of(&self, _node: StableId) -> Vec<StableId> {
        Vec::new()
    }
}

/// node별 scene dirty 종류(StableId 순).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneDirtySet {
    nodes: BTreeMap<StableId, SceneDirtyKind>,
}

/// 의존 전파 깊이 상한. Boolean 결과가 다른 Boolean의 operand인 chain도 이 깊이 안에서 끝납니다.
pub const MAX_DEPENDENT_DEPTH: usize = 64;

impl SceneDirtySet {
    /// 빈 집합.
    pub fn new() -> SceneDirtySet {
        SceneDirtySet::default()
    }

    /// 무효화 목록을 dirty set으로 바꿉니다(모듈 문서 규칙).
    pub fn from_invalidations<'a>(
        invalidations: impl IntoIterator<Item = &'a SceneInvalidation>,
        dependents: &dyn SceneDependents,
    ) -> SceneDirtySet {
        let mut set = SceneDirtySet::new();
        for invalidation in invalidations {
            if let Some((node, kind)) = invalidation.classify() {
                set.insert(node, kind);
            }
        }
        set.propagate_dependents(dependents);
        set
    }

    /// `node`를 `kind`로 표시합니다. 이미 더 큰 종류면 그대로 둡니다.
    pub fn insert(&mut self, node: StableId, kind: SceneDirtyKind) {
        let entry = self.nodes.entry(node).or_insert(kind);
        if kind > *entry {
            *entry = kind;
        }
    }

    /// 다른 집합을 합칩니다.
    pub fn merge(&mut self, other: &SceneDirtySet) {
        for (&node, &kind) in &other.nodes {
            self.insert(node, kind);
        }
    }

    /// geometry 이상 dirty node의 의존 node를 geometry dirty로 더합니다.
    fn propagate_dependents(&mut self, dependents: &dyn SceneDependents) {
        let mut frontier: Vec<StableId> = self
            .nodes
            .iter()
            .filter(|(_, kind)| **kind >= SceneDirtyKind::Geometry)
            .map(|(node, _)| *node)
            .collect();
        for _ in 0..MAX_DEPENDENT_DEPTH {
            let mut next = Vec::new();
            for node in frontier {
                for dependent in dependents.dependents_of(node) {
                    if self.get(dependent) < Some(SceneDirtyKind::Geometry) {
                        self.insert(dependent, SceneDirtyKind::Geometry);
                        next.push(dependent);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
    }

    /// node의 dirty 종류.
    pub fn get(&self, node: StableId) -> Option<SceneDirtyKind> {
        self.nodes.get(&node).copied()
    }

    /// dirty node 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// (node, 종류) 목록(StableId 순).
    pub fn iter(&self) -> impl Iterator<Item = (StableId, SceneDirtyKind)> + '_ {
        self.nodes.iter().map(|(node, kind)| (*node, *kind))
    }

    /// 종류별 개수 `[paint, geometry, subtree]`.
    pub fn counts(&self) -> [u64; 3] {
        let mut counts = [0u64; 3];
        for kind in self.nodes.values() {
            counts[*kind as usize] += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn node(n: u64, origin: InvalidationOrigin, classes: InvalidationClasses) -> SceneInvalidation {
        SceneInvalidation::Node {
            node: id(n),
            origin,
            classes,
        }
    }

    #[test]
    fn paint_only_property_is_paint_dirty() {
        // 검증 기준(XI.2.1): paint-only fixture.
        let set = SceneDirtySet::from_invalidations(
            &[
                node(1, InvalidationOrigin::Property, InvalidationClasses::PAINT),
                node(2, InvalidationOrigin::Binding, InvalidationClasses::PAINT),
                node(3, InvalidationOrigin::Modifier, InvalidationClasses::PAINT),
            ],
            &NoDependents,
        );
        assert_eq!(set.len(), 3);
        assert!(set.iter().all(|(_, kind)| kind == SceneDirtyKind::Paint));
        assert_eq!(set.counts(), [3, 0, 0]);
    }

    #[test]
    fn geometry_and_layout_classes_are_geometry_dirty() {
        // 검증 기준(XI.2.1): geometry fixture.
        let set = SceneDirtySet::from_invalidations(
            &[
                node(
                    1,
                    InvalidationOrigin::Property,
                    InvalidationClasses::GEOMETRY,
                ),
                node(
                    2,
                    InvalidationOrigin::Binding,
                    InvalidationClasses {
                        geometry: false,
                        layout: true,
                        paint: false,
                    },
                ),
                SceneInvalidation::LayoutRect {
                    node: id(3),
                    moved: true,
                    resized: false,
                },
                SceneInvalidation::LayoutRect {
                    node: id(4),
                    moved: false,
                    resized: false,
                },
                SceneInvalidation::HitTarget { node: id(5) },
                node(
                    6,
                    InvalidationOrigin::Property,
                    InvalidationClasses::default(),
                ),
            ],
            &NoDependents,
        );
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![
                (id(1), SceneDirtyKind::Geometry),
                (id(2), SceneDirtyKind::Geometry),
                (id(3), SceneDirtyKind::Geometry),
                (id(5), SceneDirtyKind::Geometry),
            ]
        );
    }

    #[test]
    fn subtree_invalidation_is_subtree_dirty_and_the_largest_kind_wins() {
        // 검증 기준(XI.2.1): subtree fixture.
        let set = SceneDirtySet::from_invalidations(
            &[
                node(1, InvalidationOrigin::Property, InvalidationClasses::PAINT),
                SceneInvalidation::Subtree { node: id(1) },
                node(
                    1,
                    InvalidationOrigin::Modifier,
                    InvalidationClasses::GEOMETRY,
                ),
                SceneInvalidation::Structure { node: id(4) },
                SceneInvalidation::Subtree { node: id(2) },
                node(
                    3,
                    InvalidationOrigin::Modifier,
                    InvalidationClasses::GEOMETRY,
                ),
                node(3, InvalidationOrigin::Property, InvalidationClasses::PAINT),
            ],
            &NoDependents,
        );
        assert_eq!(set.get(id(1)), Some(SceneDirtyKind::Subtree));
        assert_eq!(set.get(id(2)), Some(SceneDirtyKind::Subtree));
        assert_eq!(set.get(id(3)), Some(SceneDirtyKind::Geometry));
        // 구조 변경은 그 node의 geometry dirty입니다(자식 목록 비교는 증분 builder).
        assert_eq!(set.get(id(4)), Some(SceneDirtyKind::Geometry));
        assert_eq!(set.counts(), [0, 2, 2]);
    }

    struct Booleans;

    impl SceneDependents for Booleans {
        fn dependents_of(&self, node: StableId) -> Vec<StableId> {
            // 10, 11은 20의 operand, 20은 30의 operand.
            if node == id(10) || node == id(11) {
                vec![id(20)]
            } else if node == id(20) {
                vec![id(30)]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn operand_geometry_dirties_boolean_results_but_paint_does_not() {
        let set = SceneDirtySet::from_invalidations(
            &[node(
                10,
                InvalidationOrigin::Property,
                InvalidationClasses::GEOMETRY,
            )],
            &Booleans,
        );
        assert_eq!(set.get(id(20)), Some(SceneDirtyKind::Geometry));
        assert_eq!(set.get(id(30)), Some(SceneDirtyKind::Geometry));
        let paint = SceneDirtySet::from_invalidations(
            &[node(
                11,
                InvalidationOrigin::Property,
                InvalidationClasses::PAINT,
            )],
            &Booleans,
        );
        assert_eq!(paint.len(), 1);
        assert_eq!(paint.get(id(20)), None);
    }
}
