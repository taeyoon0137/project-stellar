//! Anchor·Constraint 배치 (X.5.2, X.5.3).
//!
//! # anchor target
//!
//! [`Constraint`]는 "자식의 `edge` = target 위치 + `offset`" 등식 하나입니다. target은
//!
//! - [`AnchorTarget::Parent`]: 부모 content box의 edge(`Left`/`Right`/`CenterX`, `Top`/`Bottom`/
//!   `CenterY`). 부모 baseline은 target이 될 수 없습니다.
//! - [`AnchorTarget::Sibling`]: 같은 Constraint 부모의 다른 자식 edge. 참조는 **StableId**이며 순서·
//!   index를 쓰지 않습니다. `Baseline`은 sibling의 첫 baseline(없으면 bottom edge)입니다.
//!
//! edge는 가로(`Left`·`Right`·`CenterX`) 또는 세로(`Top`·`Bottom`·`CenterY`·`Baseline`) 축에
//! 속하며, 자식 edge와 target edge는 같은 축이어야 합니다.
//!
//! # 미해결 sibling
//!
//! sibling target이 같은 부모의 표시되는 자식이 아니면(없음, 다른 부모, hidden) 레이아웃을 실패시키지
//! 않고 그 constraint를 제외한 뒤 [`LayoutDiagnostic::MissingAnchorTarget`]을 남깁니다.
//!
//! # constraint graph (X.5.3)
//!
//! 축마다 형제를 node, "자식 → target 형제" 참조를 간선으로 하는 graph를 만듭니다. 형제 한 축의
//! 미지수는 시작 위치(항상)와 크기(`Fill`일 때)이고, 그 축 constraint 하나가 등식 하나입니다. edge
//! 계수(2배 단위): 시작 쪽 `2·start`, 끝 쪽 `2·start + 2·size`, 가운데 `2·start + size`, baseline은
//! 콘텐츠 baseline이 있으면 시작 쪽, 없으면 bottom fallback. 분류는
//! [`ConstraintClass`]: 순환(`Cycle`) → 등식 수 > rank(`Overdetermined`) → rank < 미지수
//! (`Underdetermined`) → `Determined` 순으로 정합니다. 의존 순서(target 먼저)와 무방향 연결 요소도
//! 함께 구합니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Length;

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::{length_unit, LayoutNode};
use crate::output::LayoutDiagnostic;
use crate::property::LayoutProperty;
use crate::rounding::LayoutUnit;

/// anchor edge (X.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AnchorEdge {
    /// 왼쪽.
    Left,
    /// 오른쪽.
    Right,
    /// 가로 가운데.
    CenterX,
    /// 위.
    Top,
    /// 아래.
    Bottom,
    /// 세로 가운데.
    CenterY,
    /// 첫 baseline(없으면 bottom).
    Baseline,
}

impl AnchorEdge {
    /// edge가 속한 축.
    pub const fn axis(self) -> Axis {
        match self {
            AnchorEdge::Left | AnchorEdge::Right | AnchorEdge::CenterX => Axis::Horizontal,
            AnchorEdge::Top | AnchorEdge::Bottom | AnchorEdge::CenterY | AnchorEdge::Baseline => {
                Axis::Vertical
            }
        }
    }

    const fn tag(self) -> u8 {
        match self {
            AnchorEdge::Left => 0,
            AnchorEdge::Right => 1,
            AnchorEdge::CenterX => 2,
            AnchorEdge::Top => 3,
            AnchorEdge::Bottom => 4,
            AnchorEdge::CenterY => 5,
            AnchorEdge::Baseline => 6,
        }
    }

    fn from_tag(tag: u8) -> Result<AnchorEdge, LayoutError> {
        Ok(match tag {
            0 => AnchorEdge::Left,
            1 => AnchorEdge::Right,
            2 => AnchorEdge::CenterX,
            3 => AnchorEdge::Top,
            4 => AnchorEdge::Bottom,
            5 => AnchorEdge::CenterY,
            6 => AnchorEdge::Baseline,
            _ => {
                return Err(LayoutError::Malformed {
                    reason: "알 수 없는 anchor edge tag",
                })
            }
        })
    }
}

/// anchor target (X.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnchorTarget {
    /// 부모 content box edge.
    Parent(AnchorEdge),
    /// 같은 부모의 다른 자식 edge(StableId 참조).
    Sibling {
        /// 대상 sibling.
        node: StableId,
        /// 대상 edge.
        edge: AnchorEdge,
    },
}

impl AnchorTarget {
    /// target edge.
    pub const fn edge(&self) -> AnchorEdge {
        match *self {
            AnchorTarget::Parent(edge) | AnchorTarget::Sibling { edge, .. } => edge,
        }
    }
}

/// 필수 등식 제약 하나: `edge = target + offset` (X.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Constraint {
    /// 자식 자신의 edge.
    pub edge: AnchorEdge,
    /// target.
    pub target: AnchorTarget,
    /// target에서의 거리(음수 허용).
    pub offset: Length,
}

impl Constraint {
    /// `edge = target + offset` 제약.
    pub const fn new(edge: AnchorEdge, target: AnchorTarget, offset: Length) -> Constraint {
        Constraint {
            edge,
            target,
            offset,
        }
    }
}

/// constraint 값·조합을 검증합니다(모든 node).
pub(crate) fn validate_constraints(node: &LayoutNode) -> Result<(), LayoutError> {
    for constraint in &node.constraints {
        let reason = if constraint.edge.axis() != constraint.target.edge().axis() {
            Some("constraint의 자기 edge와 target edge는 같은 축이어야 합니다")
        } else if constraint.target == AnchorTarget::Parent(AnchorEdge::Baseline) {
            Some("부모 baseline은 constraint target이 될 수 없습니다")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(LayoutError::InvalidConstraint {
                node: node.id,
                reason,
            });
        }
        length_unit(
            node.id,
            LayoutProperty::Constraints,
            constraint.offset,
            true,
        )?;
    }
    Ok(())
}

const TARGET_PARENT: u8 = 0;
const TARGET_SIBLING: u8 = 1;

/// constraint 목록 canonical bytes.
pub(crate) fn encode_constraints(w: &mut Writer, constraints: &[Constraint]) {
    w.len(constraints.len());
    for constraint in constraints {
        w.u8(constraint.edge.tag());
        match constraint.target {
            AnchorTarget::Parent(edge) => {
                w.u8(TARGET_PARENT);
                w.u8(edge.tag());
            }
            AnchorTarget::Sibling { node, edge } => {
                w.u8(TARGET_SIBLING);
                w.id(node);
                w.u8(edge.tag());
            }
        }
        w.length(constraint.offset);
    }
}

/// constraint 목록 canonical bytes를 읽습니다.
pub(crate) fn decode_constraints(r: &mut Reader<'_>) -> Result<Vec<Constraint>, LayoutError> {
    let count = r.len(7)?;
    let mut constraints = Vec::with_capacity(count);
    for _ in 0..count {
        let edge = AnchorEdge::from_tag(r.u8()?)?;
        let target = match r.u8()? {
            TARGET_PARENT => AnchorTarget::Parent(AnchorEdge::from_tag(r.u8()?)?),
            TARGET_SIBLING => {
                let node = r.id()?;
                AnchorTarget::Sibling {
                    node,
                    edge: AnchorEdge::from_tag(r.u8()?)?,
                }
            }
            _ => {
                return Err(LayoutError::Malformed {
                    reason: "알 수 없는 anchor target tag",
                })
            }
        };
        let offset = r.length("constraint offset")?;
        constraints.push(Constraint::new(edge, target, offset));
    }
    Ok(constraints)
}

/// 해석된 target(sibling은 형제 목록 index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedTarget {
    /// 부모 content box edge.
    Parent(AnchorEdge),
    /// 형제 index와 edge.
    Sibling(usize, AnchorEdge),
}

/// target이 해석된 constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedConstraint {
    /// constraint를 가진 자식(형제 목록 index).
    pub(crate) child: usize,
    /// 자식 edge.
    pub(crate) edge: AnchorEdge,
    /// target.
    pub(crate) target: ResolvedTarget,
    /// offset(subpixel).
    pub(crate) offset: LayoutUnit,
}

/// 형제 하나의 입력: StableId와 constraint 목록.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SiblingConstraints<'a> {
    pub(crate) id: StableId,
    pub(crate) constraints: &'a [Constraint],
}

/// 형제(표시되는 자식, visual order) constraint의 sibling target을 StableId로 해석합니다.
///
/// 해석할 수 없는 sibling target은 constraint를 제외하고 진단을 남깁니다(오류 아님).
pub(crate) fn resolve_targets(
    siblings: &[SiblingConstraints<'_>],
) -> Result<(Vec<ResolvedConstraint>, Vec<LayoutDiagnostic>), LayoutError> {
    let index: HashMap<StableId, usize> = siblings
        .iter()
        .enumerate()
        .map(|(i, sibling)| (sibling.id, i))
        .collect();
    let mut resolved = Vec::new();
    let mut diagnostics = Vec::new();
    for (child, sibling) in siblings.iter().enumerate() {
        for constraint in sibling.constraints {
            let offset = length_unit(
                sibling.id,
                LayoutProperty::Constraints,
                constraint.offset,
                true,
            )?;
            let target = match constraint.target {
                AnchorTarget::Parent(edge) => ResolvedTarget::Parent(edge),
                AnchorTarget::Sibling { node, edge } => match index.get(&node) {
                    Some(&target_index) => ResolvedTarget::Sibling(target_index, edge),
                    None => {
                        diagnostics.push(LayoutDiagnostic::MissingAnchorTarget {
                            node: sibling.id,
                            target: node,
                        });
                        continue;
                    }
                },
            };
            resolved.push(ResolvedConstraint {
                child,
                edge: constraint.edge,
                target,
                offset,
            });
        }
    }
    Ok((resolved, diagnostics))
}

// --- constraint graph 분류 (X.5.3) ---

/// 한 자식·한 축의 constraint 분류 (X.5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstraintClass {
    /// 미지수(시작 위치, `Fill`이면 크기)와 독립 등식 수가 같음.
    Determined,
    /// 독립 등식이 미지수보다 적음(진단 + fallback).
    Underdetermined,
    /// 등식이 독립 등식보다 많음(중복 또는 모순, 오류).
    Overdetermined,
    /// sibling 참조가 순환함(오류).
    Cycle,
}

/// 한 축의 constraint graph 분석 결과 (X.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AxisGraph {
    /// 형제별 분류(형제 index 순).
    pub(crate) classes: Vec<ConstraintClass>,
    /// 순환에 속하지 않는 형제의 의존 순서(target이 먼저, 같은 단계는 visual order).
    pub(crate) order: Vec<usize>,
    /// 무방향 연결 요소(각 요소는 visual order, 요소는 첫 형제 순).
    pub(crate) components: Vec<Vec<usize>>,
}

/// edge의 (시작, 크기) 계수를 2배 단위로 줍니다: 시작 쪽 2·start, 끝 쪽 2·start + 2·size,
/// 가운데 2·start + size. baseline은 콘텐츠 baseline이 있으면 시작 쪽과 같은 형태(start + b),
/// 없으면 bottom fallback(끝 쪽)입니다.
pub(crate) const fn edge_coefficients(edge: AnchorEdge, has_baseline: bool) -> (i64, i64) {
    match edge {
        AnchorEdge::Left | AnchorEdge::Top => (2, 0),
        AnchorEdge::Right | AnchorEdge::Bottom => (2, 2),
        AnchorEdge::CenterX | AnchorEdge::CenterY => (2, 1),
        AnchorEdge::Baseline => {
            if has_baseline {
                (2, 0)
            } else {
                (2, 2)
            }
        }
    }
}

/// 등식 계수 목록의 rank(미지수 1개면 시작만, 2개면 시작·크기).
fn rank(coefficients: &[(i64, i64)], size_unknown: bool) -> usize {
    if coefficients.is_empty() {
        return 0;
    }
    if !size_unknown {
        return 1;
    }
    for (i, a) in coefficients.iter().enumerate() {
        for b in &coefficients[i + 1..] {
            if a.0 * b.1 - a.1 * b.0 != 0 {
                return 2;
            }
        }
    }
    1
}

/// 한 축 constraint graph를 만들고 형제마다 분류합니다.
///
/// - `constraints`: 이 축의 해석된 constraint.
/// - `size_unknown[i]`: 형제 `i`의 이 축 크기가 constraint로 정해지는지(`Fill`).
/// - `has_baseline[i]`: 형제 `i`의 자기 baseline edge가 콘텐츠 baseline인지(세로 축만 의미).
///
/// 분류 우선순위: 순환(`Cycle`) → 등식 > rank(`Overdetermined`) → rank < 미지수
/// (`Underdetermined`) → `Determined`.
pub(crate) fn build_axis_graph(
    count: usize,
    constraints: &[ResolvedConstraint],
    size_unknown: &[bool],
    has_baseline: &[bool],
) -> AxisGraph {
    // 의존 간선: 자식 → target 형제.
    let mut depends: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut coefficients: Vec<Vec<(i64, i64)>> = vec![Vec::new(); count];
    for constraint in constraints {
        coefficients[constraint.child].push(edge_coefficients(
            constraint.edge,
            has_baseline.get(constraint.child).copied().unwrap_or(false),
        ));
        if let ResolvedTarget::Sibling(target, _) = constraint.target {
            if !depends[constraint.child].contains(&target) {
                depends[constraint.child].push(target);
            }
        }
    }

    // Kahn 위상 정렬(준비된 형제 중 가장 앞 visual order부터). 남는 형제는 순환에 있거나 순환에 의존.
    let mut remaining: Vec<usize> = depends.iter().map(Vec::len).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (child, targets) in depends.iter().enumerate() {
        for &target in targets {
            dependents[target].push(child);
        }
    }
    let mut ready: std::collections::BTreeSet<usize> =
        (0..count).filter(|&i| remaining[i] == 0).collect();
    let mut order = Vec::with_capacity(count);
    while let Some(next) = ready.pop_first() {
        order.push(next);
        for &dependent in &dependents[next] {
            remaining[dependent] -= 1;
            if remaining[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }
    let ordered: Vec<bool> = {
        let mut seen = vec![false; count];
        for &i in &order {
            seen[i] = true;
        }
        seen
    };
    // 순환 판정: 정렬되지 않은 형제 중 자기 자신으로 돌아오는 경로가 있는 형제.
    let in_cycle = |start: usize| -> bool {
        let mut stack = depends[start].clone();
        let mut visited = vec![false; count];
        while let Some(node) = stack.pop() {
            if node == start {
                return true;
            }
            if visited[node] {
                continue;
            }
            visited[node] = true;
            stack.extend(depends[node].iter().copied());
        }
        false
    };
    let classes = (0..count)
        .map(|i| {
            if !ordered[i] && in_cycle(i) {
                return ConstraintClass::Cycle;
            }
            let unknowns = if size_unknown.get(i).copied().unwrap_or(false) {
                2
            } else {
                1
            };
            let equations = coefficients[i].len();
            let r = rank(&coefficients[i], unknowns == 2);
            if equations > r {
                ConstraintClass::Overdetermined
            } else if r < unknowns {
                ConstraintClass::Underdetermined
            } else {
                ConstraintClass::Determined
            }
        })
        .collect();

    // 무방향 연결 요소(union-find).
    let mut parent: Vec<usize> = (0..count).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for (child, targets) in depends.iter().enumerate() {
        for &target in targets {
            let a = find(&mut parent, child);
            let b = find(&mut parent, target);
            if a != b {
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut component_of: HashMap<usize, usize> = HashMap::new();
    for i in 0..count {
        let root = find(&mut parent, i);
        match component_of.get(&root) {
            Some(&c) => components[c].push(i),
            None => {
                component_of.insert(root, components.len());
                components.push(vec![i]);
            }
        }
    }
    AxisGraph {
        classes,
        order,
        components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::LayoutInput;
    use crate::test_support::{id, len};

    fn sibling(node: u64, edge: AnchorEdge) -> AnchorTarget {
        AnchorTarget::Sibling {
            node: id(node),
            edge,
        }
    }

    #[test]
    fn targets_cover_parent_sibling_center_and_baseline() {
        let constraints_a = [
            Constraint::new(
                AnchorEdge::Left,
                AnchorTarget::Parent(AnchorEdge::Left),
                len(8.0),
            ),
            Constraint::new(
                AnchorEdge::CenterY,
                AnchorTarget::Parent(AnchorEdge::CenterY),
                len(0.0),
            ),
        ];
        let constraints_b = [
            Constraint::new(AnchorEdge::Left, sibling(2, AnchorEdge::Right), len(4.0)),
            Constraint::new(
                AnchorEdge::Baseline,
                sibling(2, AnchorEdge::Baseline),
                len(0.0),
            ),
            Constraint::new(
                AnchorEdge::CenterX,
                sibling(2, AnchorEdge::CenterX),
                len(-1.5),
            ),
        ];
        let siblings = [
            SiblingConstraints {
                id: id(2),
                constraints: &constraints_a,
            },
            SiblingConstraints {
                id: id(3),
                constraints: &constraints_b,
            },
        ];
        let (resolved, diagnostics) = resolve_targets(&siblings).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(resolved.len(), 5);
        assert_eq!(resolved[0].target, ResolvedTarget::Parent(AnchorEdge::Left));
        assert_eq!(resolved[0].offset.raw(), 8 * 64);
        assert_eq!(
            resolved[2].target,
            ResolvedTarget::Sibling(0, AnchorEdge::Right)
        );
        assert_eq!(resolved[3].edge, AnchorEdge::Baseline);
        assert_eq!(resolved[4].offset.raw(), -96);
        assert_eq!(resolved[4].child, 1);
        // edge 축.
        assert_eq!(AnchorEdge::CenterX.axis(), Axis::Horizontal);
        assert_eq!(AnchorEdge::Baseline.axis(), Axis::Vertical);
    }

    #[test]
    fn missing_sibling_is_diagnostic_not_error() {
        // id 9는 형제가 아님(없음·다른 부모·hidden 모두 형제 목록에 없음).
        let constraints = [
            Constraint::new(AnchorEdge::Left, sibling(9, AnchorEdge::Right), len(0.0)),
            Constraint::new(
                AnchorEdge::Top,
                AnchorTarget::Parent(AnchorEdge::Top),
                len(2.0),
            ),
        ];
        let siblings = [SiblingConstraints {
            id: id(2),
            constraints: &constraints,
        }];
        let (resolved, diagnostics) = resolve_targets(&siblings).unwrap();
        assert_eq!(
            diagnostics,
            vec![LayoutDiagnostic::MissingAnchorTarget {
                node: id(2),
                target: id(9)
            }]
        );
        // 해석 가능한 constraint는 유지.
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].edge, AnchorEdge::Top);
        assert!(diagnostics[0].to_string().contains("anchor"));
    }

    #[test]
    fn constraint_validation_and_round_trip() {
        let node = |constraints: Vec<Constraint>| {
            let mut n = LayoutNode::new(id(1));
            n.constraints = constraints;
            LayoutInput::new(id(1), vec![n])
        };
        // 축이 다른 edge·부모 baseline target 거부.
        let mixed = node(vec![Constraint::new(
            AnchorEdge::Left,
            AnchorTarget::Parent(AnchorEdge::Top),
            len(0.0),
        )]);
        assert!(matches!(
            mixed.validate(),
            Err(LayoutError::InvalidConstraint { .. })
        ));
        let parent_baseline = node(vec![Constraint::new(
            AnchorEdge::Baseline,
            AnchorTarget::Parent(AnchorEdge::Baseline),
            len(0.0),
        )]);
        assert!(matches!(
            parent_baseline.validate(),
            Err(LayoutError::InvalidConstraint { .. })
        ));
        let out_of_range = node(vec![Constraint::new(
            AnchorEdge::Top,
            AnchorTarget::Parent(AnchorEdge::Top),
            len(4.0e7),
        )]);
        assert!(matches!(
            out_of_range.validate(),
            Err(LayoutError::OutOfRange { .. })
        ));
        // round-trip(모든 edge).
        let all = [
            AnchorEdge::Left,
            AnchorEdge::Right,
            AnchorEdge::CenterX,
            AnchorEdge::Top,
            AnchorEdge::Bottom,
            AnchorEdge::CenterY,
            AnchorEdge::Baseline,
        ];
        let constraints: Vec<Constraint> = all
            .iter()
            .map(|edge| Constraint::new(*edge, sibling(7, *edge), len(1.25)))
            .collect();
        let input = node(constraints);
        assert_eq!(input.validate(), Ok(()));
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        let tags: Vec<u8> = all.iter().map(|e| e.tag()).collect();
        assert_eq!(tags, vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(AnchorEdge::from_tag(7).is_err());
    }

    /// 해석된 constraint 하나(테스트용).
    fn rc(child: usize, edge: AnchorEdge, target: ResolvedTarget) -> ResolvedConstraint {
        ResolvedConstraint {
            child,
            edge,
            target,
            offset: LayoutUnit::ZERO,
        }
    }

    fn parent(edge: AnchorEdge) -> ResolvedTarget {
        ResolvedTarget::Parent(edge)
    }

    fn sib(index: usize, edge: AnchorEdge) -> ResolvedTarget {
        ResolvedTarget::Sibling(index, edge)
    }

    use super::ConstraintClass::{Cycle, Determined, Overdetermined, Underdetermined};

    #[test]
    fn determined_and_underdetermined_graph_fixture() {
        // 0: 고정 크기, Left = parent.Left → Determined.
        // 1: Fill, Left = 0.Right, Right = parent.Right → Determined(rank 2).
        // 2: Fill, Left만 → Underdetermined(크기 미정).
        // 3: 고정, constraint 없음 → Underdetermined(위치 미정).
        // 4: Fill, Left + CenterX → Determined((2,0)과 (2,1) 독립).
        let constraints = [
            rc(0, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(1, AnchorEdge::Left, sib(0, AnchorEdge::Right)),
            rc(1, AnchorEdge::Right, parent(AnchorEdge::Right)),
            rc(2, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(4, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(4, AnchorEdge::CenterX, parent(AnchorEdge::CenterX)),
        ];
        let graph = build_axis_graph(
            5,
            &constraints,
            &[false, true, true, false, true],
            &[false; 5],
        );
        assert_eq!(
            graph.classes,
            vec![
                Determined,
                Determined,
                Underdetermined,
                Underdetermined,
                Determined
            ]
        );
        // 의존 순서: 0이 1보다 먼저.
        assert_eq!(graph.order, vec![0, 1, 2, 3, 4]);
        // 연결 요소: {0, 1}, {2}, {3}, {4}.
        assert_eq!(
            graph.components,
            vec![vec![0, 1], vec![2], vec![3], vec![4]]
        );
    }

    #[test]
    fn overdetermined_graph_fixture() {
        // 0: 고정 + Left + Right → 등식 2 > rank 1.
        // 1: Fill + Left + Left(평행, 중복) → 등식 2 > rank 1.
        // 2: Fill + Left + Right + CenterX → 등식 3 > rank 2.
        // 3: 고정 + Top + Baseline(콘텐츠 baseline 있음) → 등식 2 > rank 1.
        // 4: Fill + Top + Baseline(baseline 없음 → bottom fallback) → Determined.
        let constraints = [
            rc(0, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(0, AnchorEdge::Right, parent(AnchorEdge::Right)),
            rc(1, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(1, AnchorEdge::Left, parent(AnchorEdge::CenterX)),
            rc(2, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(2, AnchorEdge::Right, parent(AnchorEdge::Right)),
            rc(2, AnchorEdge::CenterX, parent(AnchorEdge::CenterX)),
            rc(3, AnchorEdge::Top, parent(AnchorEdge::Top)),
            rc(3, AnchorEdge::Baseline, parent(AnchorEdge::Top)),
            rc(4, AnchorEdge::Top, parent(AnchorEdge::Top)),
            rc(4, AnchorEdge::Baseline, parent(AnchorEdge::Bottom)),
        ];
        let graph = build_axis_graph(
            5,
            &constraints,
            &[false, true, true, false, true],
            &[false, false, false, true, false],
        );
        assert_eq!(
            graph.classes,
            vec![
                Overdetermined,
                Overdetermined,
                Overdetermined,
                Overdetermined,
                Determined
            ]
        );
    }

    #[test]
    fn cycle_graph_fixture() {
        // 0 ↔ 1 순환, 2는 자기 참조, 3은 순환(0)에 의존하지만 자신은 순환이 아님, 4는 독립.
        let constraints = [
            rc(0, AnchorEdge::Left, sib(1, AnchorEdge::Right)),
            rc(1, AnchorEdge::Left, sib(0, AnchorEdge::Right)),
            rc(2, AnchorEdge::Left, sib(2, AnchorEdge::Right)),
            rc(3, AnchorEdge::Left, sib(0, AnchorEdge::Right)),
            rc(4, AnchorEdge::Left, parent(AnchorEdge::Left)),
        ];
        let graph = build_axis_graph(5, &constraints, &[false; 5], &[false; 5]);
        assert_eq!(
            graph.classes,
            vec![Cycle, Cycle, Cycle, Determined, Determined]
        );
        // 순환·순환 의존 형제는 순서에서 빠짐.
        assert_eq!(graph.order, vec![4]);
        assert_eq!(graph.components, vec![vec![0, 1, 3], vec![2], vec![4]]);
    }

    #[test]
    fn dependency_order_puts_targets_first() {
        // visual order상 앞인 0이 뒤의 2에 의존: 순서는 1, 2, 0(준비된 것 중 앞 visual order).
        let constraints = [
            rc(0, AnchorEdge::Left, sib(2, AnchorEdge::Right)),
            rc(1, AnchorEdge::Left, parent(AnchorEdge::Left)),
            rc(2, AnchorEdge::Left, sib(1, AnchorEdge::Right)),
        ];
        let graph = build_axis_graph(3, &constraints, &[false; 3], &[false; 3]);
        assert_eq!(graph.order, vec![1, 2, 0]);
        assert_eq!(graph.classes, vec![Determined; 3]);
        assert_eq!(graph.components, vec![vec![0, 1, 2]]);
    }
}
