//! Anchor·Constraint 배치 (X.5.2~X.5.5).
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
//!
//! # solver (X.5.4)
//!
//! 초기 solver는 **priority 없는 필수 등식**만 지원합니다(스키마에 priority·부등식 필드가 없음).
//! 축마다 의존 순서대로 각 형제의 등식을 2배 단위 정수로 세우고 정확히 풀어 마지막에 한 번 반올림합니다
//! (ties away from zero). 가로를 먼저 풀고, 정해진 너비로 높이를 측정한 뒤 세로를 풉니다.
//! `Cycle`·`Overdetermined`는 오류, `Underdetermined`는 진단 + fallback(미정 크기 = 콘텐츠 크기,
//! 미정 위치 = 첫 등식 또는 0)입니다. Fill 크기가 음수로 풀리면 0으로 자릅니다. 같은 입력은 항상 같은
//! 해를 냅니다(형제 입력 순서·삽입 순서 무관).
//!
//! # 영향 범위 (X.5.5)
//!
//! constraint 참조는 같은 부모의 형제 사이에만 있으므로, 한 형제의 constraint 변경은 그 형제가 속한
//! 무방향 연결 요소(변경 전·후 모두) 안에서만 해를 바꿉니다. [`ConstraintSystem`]은 그 연결 요소만
//! 다시 풀고 다른 요소의 해를 재사용합니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Length;

use crate::codec::{Reader, Writer};
use crate::engine::{at, AxisSizing, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::input::{length_unit, LayoutNode};
use crate::output::LayoutDiagnostic;
use crate::property::LayoutProperty;
use crate::rounding::{div_round, LayoutUnit};

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

// --- solver (X.5.4) ---

/// 형제 한 축의 solver 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxisInput {
    /// constraint로 크기를 정하는지(`Fill`).
    pub(crate) size_unknown: bool,
    /// 크기가 정해진 형제의 크기, 또는 미정 크기 fallback(콘텐츠 크기).
    pub(crate) size: LayoutUnit,
    /// 형제 top에서 첫 baseline까지(세로 축, 콘텐츠 baseline이 있을 때).
    pub(crate) baseline: Option<LayoutUnit>,
}

/// 형제 한 축의 해(content box 기준 시작 위치와 크기).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxisSolution {
    pub(crate) start: LayoutUnit,
    pub(crate) size: LayoutUnit,
}

fn wide(value: LayoutUnit) -> i128 {
    i128::from(value.raw())
}

/// 해의 edge 값(2배 단위).
fn edge_value_doubled(
    edge: AnchorEdge,
    solution: AxisSolution,
    baseline: Option<LayoutUnit>,
) -> i128 {
    let start = wide(solution.start);
    let size = wide(solution.size);
    match edge {
        AnchorEdge::Left | AnchorEdge::Top => 2 * start,
        AnchorEdge::Right | AnchorEdge::Bottom => 2 * (start + size),
        AnchorEdge::CenterX | AnchorEdge::CenterY => 2 * start + size,
        AnchorEdge::Baseline => match baseline {
            Some(b) => 2 * (start + wide(b)),
            None => 2 * (start + size),
        },
    }
}

/// 부모 content box edge 값(2배 단위). `extent`는 content 크기입니다.
fn parent_value_doubled(edge: AnchorEdge, extent: LayoutUnit) -> i128 {
    match edge {
        AnchorEdge::Left | AnchorEdge::Top | AnchorEdge::Baseline => 0,
        AnchorEdge::Right | AnchorEdge::Bottom => 2 * wide(extent),
        AnchorEdge::CenterX | AnchorEdge::CenterY => wide(extent),
    }
}

/// 한 축을 의존 순서대로 풉니다 (X.5.4).
///
/// - `Cycle` → [`LayoutError::ConstraintCycle`], `Overdetermined` → [`LayoutError::OverConstrained`].
/// - `Determined`: 크기가 정해졌으면 등식 하나로 시작 위치를, `Fill`이면 독립 등식 두 개(Cramer)로
///   시작·크기를 정확한 정수 연산으로 구하고 마지막에 한 번 반올림합니다(ties away from zero). 음수
///   크기는 0으로 자릅니다.
/// - `Underdetermined`: [`LayoutDiagnostic::UnderdeterminedConstraint`]를 남기고, 미정 크기는 콘텐츠
///   크기로, 미정 위치는 첫 등식(없으면 content 시작 0)으로 정합니다.
///
/// `only`가 있으면 그 형제들만 풉니다(연결 요소 단위 재계산, X.5.5). 나머지 형제 해는 `solved`에
/// 이미 있어야 합니다.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_axis(
    ids: &[StableId],
    axis: Axis,
    extent: LayoutUnit,
    constraints: &[ResolvedConstraint],
    inputs: &[AxisInput],
    graph: &AxisGraph,
    only: Option<&[usize]>,
    solved: &mut [Option<AxisSolution>],
    diagnostics: &mut Vec<LayoutDiagnostic>,
) -> Result<(), LayoutError> {
    let in_scope = |i: usize| only.is_none_or(|members| members.contains(&i));
    for (i, class) in graph.classes.iter().enumerate() {
        if !in_scope(i) {
            continue;
        }
        match class {
            ConstraintClass::Cycle => {
                return Err(LayoutError::ConstraintCycle { node: ids[i], axis })
            }
            ConstraintClass::Overdetermined => {
                return Err(LayoutError::OverConstrained { node: ids[i], axis })
            }
            ConstraintClass::Determined | ConstraintClass::Underdetermined => {}
        }
    }
    for &i in &graph.order {
        if !in_scope(i) {
            continue;
        }
        let input = inputs[i];
        let at_node = at(ids[i]);
        // 이 형제의 등식: (a, b, rhs) — a·start + b·size = rhs (2배 단위).
        let mut equations: Vec<Equation> = Vec::new();
        for constraint in constraints.iter().filter(|c| c.child == i) {
            let target = match constraint.target {
                ResolvedTarget::Parent(edge) => parent_value_doubled(edge, extent),
                ResolvedTarget::Sibling(j, edge) => {
                    let solution =
                        solved[j].ok_or(LayoutError::ConstraintCycle { node: ids[i], axis })?;
                    edge_value_doubled(edge, solution, inputs[j].baseline)
                }
            };
            let (a, b) = edge_coefficients(constraint.edge, input.baseline.is_some());
            let mut rhs = target + 2 * wide(constraint.offset);
            if constraint.edge == AnchorEdge::Baseline {
                if let Some(baseline) = input.baseline {
                    rhs -= 2 * wide(baseline);
                }
            }
            equations.push((i128::from(a), i128::from(b), rhs));
        }
        let determined = graph.classes[i] == ConstraintClass::Determined;
        if !determined {
            diagnostics.push(LayoutDiagnostic::UnderdeterminedConstraint { node: ids[i], axis });
        }
        let solution = if determined && input.size_unknown {
            // 독립 등식 두 개(rank 2).
            let (e1, e2) = independent_pair(&equations)
                .ok_or(LayoutError::OverConstrained { node: ids[i], axis })?;
            let det = e1.0 * e2.1 - e2.0 * e1.1;
            let start = div_round(e1.2 * e2.1 - e2.2 * e1.1, det).map_err(&at_node)?;
            let size = div_round(e1.0 * e2.2 - e2.0 * e1.2, det).map_err(&at_node)?;
            AxisSolution {
                start,
                size: size.max(LayoutUnit::ZERO),
            }
        } else {
            // 크기가 정해짐(또는 미정 크기는 콘텐츠 크기 fallback): 첫 등식으로 시작 위치.
            let size = input.size;
            let start = match equations.first() {
                Some(&(a, b, rhs)) => div_round(rhs - b * wide(size), a).map_err(&at_node)?,
                None => LayoutUnit::ZERO,
            };
            AxisSolution { start, size }
        };
        solved[i] = Some(solution);
    }
    Ok(())
}

/// 등식 `a·start + b·size = rhs`(2배 단위) 계수 `(a, b, rhs)`.
type Equation = (i128, i128, i128);

/// 행렬식이 0이 아닌 첫 등식 쌍.
fn independent_pair(equations: &[Equation]) -> Option<(Equation, Equation)> {
    for (k, a) in equations.iter().enumerate() {
        for b in &equations[k + 1..] {
            if a.0 * b.1 - b.0 * a.1 != 0 {
                return Some((*a, *b));
            }
        }
    }
    None
}

// --- 엔진 연결 (X.5.4) ---

/// 형제 visual order index 목록과 constraint 해석.
struct ConstraintScene {
    children: Vec<usize>,
    ids: Vec<StableId>,
    resolved: Vec<ResolvedConstraint>,
}

fn scene(engine: &mut Engine<'_>, index: usize) -> Result<ConstraintScene, LayoutError> {
    let children = engine.children(index);
    let ids: Vec<StableId> = children.iter().map(|&c| engine.nodes[c].id).collect();
    let siblings: Vec<SiblingConstraints<'_>> = children
        .iter()
        .map(|&c| SiblingConstraints {
            id: engine.nodes[c].id,
            constraints: &engine.node(c).constraints,
        })
        .collect();
    let (resolved, diagnostics) = resolve_targets(&siblings)?;
    engine.diagnostics.extend(diagnostics);
    Ok(ConstraintScene {
        children,
        ids,
        resolved,
    })
}

/// Constraint container의 content extent(Hug 측정): indefinite 축은 자식 크기 최댓값입니다
/// (위치는 부모 크기에 의존하므로 측정에 쓰지 않음).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let mut extent = [LayoutUnit::ZERO; 2];
    let constraint = ChildConstraint {
        cb: inner,
        fill: [None, None],
        forced: [None, None],
    };
    for child in engine.children(index) {
        let size = engine.resolve_size(child, constraint)?;
        for axis in Axis::BOTH {
            extent[axis.index()] = extent[axis.index()].max(size.get(axis));
        }
    }
    Ok(extent)
}

/// Constraint container의 자식 배치. 가로를 먼저 풀고(정해진 너비로 높이를 측정한 뒤) 세로를
/// 풉니다. 결과는 visual order입니다.
pub(crate) fn place(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
) -> Result<Vec<(usize, LayoutRect)>, LayoutError> {
    let padding = engine.nodes[index].padding;
    let scene = scene(engine, index)?;
    let count = scene.children.len();
    let cb = [Some(inner.width()), Some(inner.height())];
    let mut solutions: [Vec<Option<AxisSolution>>; 2] = [vec![None; count], vec![None; count]];
    for axis in Axis::BOTH {
        let a = axis.index();
        let mut inputs = Vec::with_capacity(count);
        for (k, &child) in scene.children.iter().enumerate() {
            let mut forced = [None, None];
            if axis == Axis::Vertical {
                forced[0] = solutions[0][k].map(|s| s.size);
            }
            let measured = engine.resolve_size(
                child,
                ChildConstraint {
                    cb,
                    fill: [None, None],
                    forced,
                },
            )?;
            let baseline = match axis {
                Axis::Vertical => engine.border_baseline(child, measured)?,
                Axis::Horizontal => None,
            };
            inputs.push(AxisInput {
                size_unknown: engine.nodes[child].sizing[a] == AxisSizing::Fill,
                size: measured.get(axis),
                baseline,
            });
        }
        let axis_constraints: Vec<ResolvedConstraint> = scene
            .resolved
            .iter()
            .filter(|c| c.edge.axis() == axis)
            .copied()
            .collect();
        let size_unknown: Vec<bool> = inputs.iter().map(|i| i.size_unknown).collect();
        let has_baseline: Vec<bool> = inputs.iter().map(|i| i.baseline.is_some()).collect();
        let graph = build_axis_graph(count, &axis_constraints, &size_unknown, &has_baseline);
        let mut diagnostics = Vec::new();
        solve_axis(
            &scene.ids,
            axis,
            inner.get(axis),
            &axis_constraints,
            &inputs,
            &graph,
            None,
            &mut solutions[a],
            &mut diagnostics,
        )?;
        engine.diagnostics.extend(diagnostics);
    }
    let mut placements = Vec::with_capacity(count);
    for (k, &child) in scene.children.iter().enumerate() {
        let (Some(h), Some(v)) = (solutions[0][k], solutions[1][k]) else {
            return Err(LayoutError::ConstraintCycle {
                node: scene.ids[k],
                axis: Axis::Horizontal,
            });
        };
        let size = engine.resolve_size(
            child,
            ChildConstraint {
                cb,
                fill: [None, None],
                forced: [Some(h.size), Some(v.size)],
            },
        )?;
        let x = padding
            .left
            .checked_add(h.start)
            .map_err(at(scene.ids[k]))?;
        let y = padding.top.checked_add(v.start).map_err(at(scene.ids[k]))?;
        placements.push((child, LayoutRect::from_origin_size(x, y, size)));
    }
    Ok(placements)
}

// --- 연결 요소 단위 증분 풀이 (X.5.5) ---

/// 증분 constraint 계산의 형제 입력 (X.5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintSibling {
    /// 형제 node.
    pub id: StableId,
    /// 형제의 constraint(축이 다른 것은 무시).
    pub constraints: Vec<Constraint>,
    /// 이 축 크기를 constraint로 정하는지(`Fill`).
    pub fill: bool,
    /// 정해진 크기 또는 미정 크기 fallback.
    pub size: LayoutUnit,
    /// top에서 첫 baseline까지(세로 축).
    pub baseline: Option<LayoutUnit>,
}

/// 한 형제의 축 해.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstraintPlacement {
    /// content box 기준 시작 위치.
    pub start: LayoutUnit,
    /// 크기.
    pub size: LayoutUnit,
}

/// 한 축의 형제 constraint를 연결 요소 단위로 증분 계산합니다 (X.5.5).
///
/// constraint가 바뀐 형제의 **변경 전·후 연결 요소**만 다시 풀고, 다른 연결 요소의 해는 그대로
/// 둡니다. [`ConstraintSystem::solve_counts`]는 형제별 풀이 횟수로, 무관한 연결 요소의 재계산이 0임을
/// 확인할 수 있습니다.
#[derive(Debug, Clone)]
pub struct ConstraintSystem {
    axis: Axis,
    extent: LayoutUnit,
    siblings: Vec<ConstraintSibling>,
    solutions: Vec<Option<AxisSolution>>,
    solve_counts: Vec<u64>,
    diagnostics: Vec<LayoutDiagnostic>,
}

impl ConstraintSystem {
    /// 형제 전체를 풀어 시스템을 만듭니다. `extent`는 부모 content 크기입니다.
    pub fn new(
        axis: Axis,
        extent: LayoutUnit,
        siblings: Vec<ConstraintSibling>,
    ) -> Result<ConstraintSystem, LayoutError> {
        let count = siblings.len();
        let mut system = ConstraintSystem {
            axis,
            extent,
            siblings,
            solutions: vec![None; count],
            solve_counts: vec![0; count],
            diagnostics: Vec::new(),
        };
        system.solve(None)?;
        Ok(system)
    }

    /// 형제의 해.
    pub fn placement(&self, id: StableId) -> Option<ConstraintPlacement> {
        let index = self.siblings.iter().position(|s| s.id == id)?;
        self.solutions[index].map(|s| ConstraintPlacement {
            start: s.start,
            size: s.size,
        })
    }

    /// 형제별 누적 풀이 횟수(형제 입력 순).
    pub fn solve_counts(&self) -> &[u64] {
        &self.solve_counts
    }

    /// 마지막 풀이의 진단.
    pub fn diagnostics(&self) -> &[LayoutDiagnostic] {
        &self.diagnostics
    }

    /// 형제 하나의 constraint를 바꾸고 영향 연결 요소만 다시 풉니다. 다시 푼 형제 id를 형제 순서로
    /// 반환합니다. 오류가 나면 시스템은 바뀌지 않습니다.
    pub fn set_constraints(
        &mut self,
        id: StableId,
        constraints: Vec<Constraint>,
    ) -> Result<Vec<StableId>, LayoutError> {
        let index = self.siblings.iter().position(|s| s.id == id).ok_or(
            LayoutError::InvalidConstraint {
                node: id,
                reason: "constraint system에 없는 형제입니다",
            },
        )?;
        let before = self.graph()?.1.components;
        let mut next = self.clone();
        next.siblings[index].constraints = constraints;
        let after = next.graph()?.1.components;
        let mut affected: Vec<usize> = Vec::new();
        for component in before.iter().chain(&after) {
            if component.contains(&index) {
                for &member in component {
                    if !affected.contains(&member) {
                        affected.push(member);
                    }
                }
            }
        }
        affected.sort_unstable();
        for &member in &affected {
            next.solutions[member] = None;
        }
        next.solve(Some(&affected))?;
        *self = next;
        Ok(affected.iter().map(|&i| self.siblings[i].id).collect())
    }

    /// 해석된 constraint와 graph.
    fn graph(
        &self,
    ) -> Result<(Vec<ResolvedConstraint>, AxisGraph, Vec<LayoutDiagnostic>), LayoutError> {
        let views: Vec<SiblingConstraints<'_>> = self
            .siblings
            .iter()
            .map(|s| SiblingConstraints {
                id: s.id,
                constraints: &s.constraints,
            })
            .collect();
        let (resolved, diagnostics) = resolve_targets(&views)?;
        let resolved: Vec<ResolvedConstraint> = resolved
            .into_iter()
            .filter(|c| c.edge.axis() == self.axis)
            .collect();
        let size_unknown: Vec<bool> = self.siblings.iter().map(|s| s.fill).collect();
        let has_baseline: Vec<bool> = self.siblings.iter().map(|s| s.baseline.is_some()).collect();
        let graph = build_axis_graph(self.siblings.len(), &resolved, &size_unknown, &has_baseline);
        Ok((resolved, graph, diagnostics))
    }

    fn solve(&mut self, only: Option<&[usize]>) -> Result<(), LayoutError> {
        let (resolved, graph, mut diagnostics) = self.graph()?;
        let ids: Vec<StableId> = self.siblings.iter().map(|s| s.id).collect();
        let inputs: Vec<AxisInput> = self
            .siblings
            .iter()
            .map(|s| AxisInput {
                size_unknown: s.fill,
                size: s.size,
                baseline: s.baseline,
            })
            .collect();
        solve_axis(
            &ids,
            self.axis,
            self.extent,
            &resolved,
            &inputs,
            &graph,
            only,
            &mut self.solutions,
            &mut diagnostics,
        )?;
        for (i, count) in self.solve_counts.iter_mut().enumerate() {
            if only.is_none_or(|members| members.contains(&i)) {
                *count += 1;
            }
        }
        self.diagnostics = diagnostics;
        Ok(())
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

    use crate::engine::compute_layout;
    use crate::input::{ContainerLayout, IntrinsicContent, LayoutContent, SizingMode};
    use crate::output::LayoutResult;
    use crate::test_support::{key, px};
    use stellar_core_types::value::Insets;

    fn left(target: AnchorTarget, offset: f32) -> Constraint {
        Constraint::new(AnchorEdge::Left, target, len(offset))
    }

    fn edge(e: AnchorEdge, target: AnchorTarget, offset: f32) -> Constraint {
        Constraint::new(e, target, len(offset))
    }

    fn of_parent(e: AnchorEdge) -> AnchorTarget {
        AnchorTarget::Parent(e)
    }

    /// Constraint root(300×100, padding 10 → content 280×80) + 자식들(OrderKey는 목록 순서).
    fn constraint_layout(
        width: f32,
        children: Vec<LayoutNode>,
    ) -> Result<LayoutResult, LayoutError> {
        let mut root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::Fixed(len(width)), SizingMode::Fixed(len(100.0)))
            .with_padding(Insets::new(10.0, 10.0, 10.0, 10.0).unwrap())
            .with_container(ContainerLayout::Constraint);
        for (i, child) in children.iter().enumerate() {
            root = root.with_child(child.id, key(i as u32 + 1));
        }
        let mut nodes = vec![root];
        nodes.extend(children);
        compute_layout(&LayoutInput::new(id(1), nodes))
    }

    fn rect_px(result: &LayoutResult, n: u64) -> (f64, f64, f64, f64) {
        let r = result.get(id(n)).unwrap().local_rect();
        (
            r.x().to_px_f64(),
            r.y().to_px_f64(),
            r.width().to_px_f64(),
            r.height().to_px_f64(),
        )
    }

    fn text(n: u64, height: f32, baseline: Option<f32>) -> LayoutNode {
        LayoutNode::new(id(n))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(30.0),
                height: len(height),
                baseline: baseline.map(len),
            }))
    }

    #[test]
    fn deterministic_solution_fixture() {
        // A: 고정 50×20, Left = parent.Left + 10, Top = parent.Top + 5.
        // B: Fill 너비 × 고정 30, Left = A.Right + 8, Right = parent.Right − 12, CenterY = A.CenterY.
        let a = LayoutNode::new(id(2))
            .with_fixed_size(len(50.0), len(20.0))
            .with_constraint(left(of_parent(AnchorEdge::Left), 10.0))
            .with_constraint(edge(AnchorEdge::Top, of_parent(AnchorEdge::Top), 5.0));
        let b = LayoutNode::new(id(3))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(30.0)))
            .with_constraint(left(
                AnchorTarget::Sibling {
                    node: id(2),
                    edge: AnchorEdge::Right,
                },
                8.0,
            ))
            .with_constraint(edge(AnchorEdge::Right, of_parent(AnchorEdge::Right), -12.0))
            .with_constraint(edge(
                AnchorEdge::CenterY,
                AnchorTarget::Sibling {
                    node: id(2),
                    edge: AnchorEdge::CenterY,
                },
                0.0,
            ));
        // B가 visual order 앞이어도(A를 참조) 의존 순서로 풉니다.
        let result = constraint_layout(300.0, vec![b.clone(), a.clone()]).unwrap();
        assert_eq!(rect_px(&result, 2), (20.0, 15.0, 50.0, 20.0));
        // B: x = 10 + 68, 오른쪽 = 280 − 12 → 너비 200, y: A 중심 15 − 15 = 0 → 10.
        assert_eq!(rect_px(&result, 3), (78.0, 10.0, 200.0, 30.0));
        assert!(result.diagnostics().is_empty());
        // 반복·형제 순서와 무관하게 같은 bytes.
        let again = constraint_layout(300.0, vec![b, a]).unwrap();
        assert_eq!(again.encode(), result.encode());
    }

    #[test]
    fn center_tie_and_baseline_solutions_are_exact() {
        // content 너비 100 + 1/64 px(6401 subpixel), 너비 50 → CenterX 시작 1600.5 → 1601(ties away).
        let centered = LayoutNode::new(id(2))
            .with_fixed_size(len(50.0), len(10.0))
            .with_constraint(edge(
                AnchorEdge::CenterX,
                of_parent(AnchorEdge::CenterX),
                0.0,
            ))
            .with_constraint(edge(AnchorEdge::Top, of_parent(AnchorEdge::Top), 0.0));
        let result = constraint_layout(120.0 + 1.0 / 64.0, vec![centered]).unwrap();
        assert_eq!(
            result.get(id(2)).unwrap().local_rect().x().raw(),
            640 + 1601
        );

        // baseline: A(높이 16, baseline 12) Top = 0. B(높이 24, baseline 20) Baseline = A.Baseline →
        // top −8. C(baseline 없음, 높이 10) Baseline = A.Baseline → bottom fallback → top 2.
        let a = text(2, 16.0, Some(12.0))
            .with_constraint(left(of_parent(AnchorEdge::Left), 0.0))
            .with_constraint(edge(AnchorEdge::Top, of_parent(AnchorEdge::Top), 0.0));
        let a_baseline = AnchorTarget::Sibling {
            node: id(2),
            edge: AnchorEdge::Baseline,
        };
        let b = text(3, 24.0, Some(20.0))
            .with_constraint(left(of_parent(AnchorEdge::Left), 40.0))
            .with_constraint(edge(AnchorEdge::Baseline, a_baseline, 0.0));
        let c = text(4, 10.0, None)
            .with_constraint(left(of_parent(AnchorEdge::Left), 80.0))
            .with_constraint(edge(AnchorEdge::Baseline, a_baseline, 0.0));
        let result = constraint_layout(300.0, vec![a, b, c]).unwrap();
        assert_eq!(rect_px(&result, 2).1, 10.0);
        assert_eq!(rect_px(&result, 3).1, 10.0 - 8.0);
        assert_eq!(rect_px(&result, 4).1, 10.0 + 2.0);
    }

    #[test]
    fn errors_and_diagnostics_from_classification() {
        let fixed = |n: u64| LayoutNode::new(id(n)).with_fixed_size(len(10.0), len(10.0));
        // Overdetermined → OverConstrained 오류.
        let over = fixed(2)
            .with_constraint(left(of_parent(AnchorEdge::Left), 0.0))
            .with_constraint(edge(AnchorEdge::Right, of_parent(AnchorEdge::Right), 0.0));
        assert_eq!(
            constraint_layout(300.0, vec![over]),
            Err(LayoutError::OverConstrained {
                node: id(2),
                axis: Axis::Horizontal
            })
        );
        // Cycle → ConstraintCycle 오류.
        let sib = |n: u64, e: AnchorEdge| AnchorTarget::Sibling {
            node: id(n),
            edge: e,
        };
        let a = fixed(2).with_constraint(left(sib(3, AnchorEdge::Right), 0.0));
        let b = fixed(3).with_constraint(left(sib(2, AnchorEdge::Right), 0.0));
        assert!(matches!(
            constraint_layout(300.0, vec![a, b]),
            Err(LayoutError::ConstraintCycle {
                axis: Axis::Horizontal,
                ..
            })
        ));
        // Underdetermined → 진단 + fallback(위치 0, Fill 크기 = 콘텐츠).
        let under = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(10.0)))
            .with_constraint(left(of_parent(AnchorEdge::Left), 5.0));
        let missing = fixed(3)
            .with_constraint(left(sib(9, AnchorEdge::Right), 0.0))
            .with_constraint(edge(AnchorEdge::Top, of_parent(AnchorEdge::Top), 0.0));
        let result = constraint_layout(300.0, vec![under, missing]).unwrap();
        assert_eq!(rect_px(&result, 2), (15.0, 10.0, 0.0, 10.0));
        assert_eq!(rect_px(&result, 3), (10.0, 10.0, 10.0, 10.0));
        assert_eq!(
            result.diagnostics(),
            &[
                LayoutDiagnostic::MissingAnchorTarget {
                    node: id(3),
                    target: id(9)
                },
                LayoutDiagnostic::UnderdeterminedConstraint {
                    node: id(2),
                    axis: Axis::Horizontal
                },
                LayoutDiagnostic::UnderdeterminedConstraint {
                    node: id(3),
                    axis: Axis::Horizontal
                },
                LayoutDiagnostic::UnderdeterminedConstraint {
                    node: id(2),
                    axis: Axis::Vertical
                },
            ]
        );
        // 진단은 결과 bytes로 round-trip.
        assert_eq!(LayoutResult::decode(&result.encode()).unwrap(), result);
    }

    fn sibling_input(n: u64, constraints: Vec<Constraint>) -> ConstraintSibling {
        ConstraintSibling {
            id: id(n),
            constraints,
            fill: false,
            size: px(10),
            baseline: None,
        }
    }

    fn after(n: u64, offset: f32) -> Constraint {
        Constraint::new(
            AnchorEdge::Left,
            AnchorTarget::Sibling {
                node: id(n),
                edge: AnchorEdge::Right,
            },
            len(offset),
        )
    }

    fn from_parent(offset: f32) -> Constraint {
        Constraint::new(
            AnchorEdge::Left,
            AnchorTarget::Parent(AnchorEdge::Left),
            len(offset),
        )
    }

    /// 연결 요소 {A, B}, {C, D}, {E}.
    fn three_components() -> ConstraintSystem {
        ConstraintSystem::new(
            Axis::Horizontal,
            px(200),
            vec![
                sibling_input(2, vec![from_parent(0.0)]),
                sibling_input(3, vec![after(2, 5.0)]),
                sibling_input(4, vec![from_parent(100.0)]),
                sibling_input(5, vec![after(4, 5.0)]),
                sibling_input(6, vec![from_parent(150.0)]),
            ],
        )
        .unwrap()
    }

    #[test]
    fn unrelated_components_are_not_recomputed() {
        let mut system = three_components();
        assert_eq!(system.solve_counts(), &[1, 1, 1, 1, 1]);
        let before_ab = (system.placement(id(2)), system.placement(id(3)));
        let before_e = system.placement(id(6));
        // C의 offset 변경 → {C, D}만 다시 풉니다.
        let resolved = system
            .set_constraints(id(4), vec![from_parent(120.0)])
            .unwrap();
        assert_eq!(resolved, vec![id(4), id(5)]);
        assert_eq!(
            system.solve_counts(),
            &[1, 1, 2, 2, 1],
            "무관 요소 재계산 0"
        );
        assert_eq!(system.placement(id(4)).unwrap().start, px(120));
        assert_eq!(system.placement(id(5)).unwrap().start, px(135));
        assert_eq!(
            (system.placement(id(2)), system.placement(id(3))),
            before_ab
        );
        assert_eq!(system.placement(id(6)), before_e);
    }

    #[test]
    fn merging_and_splitting_components_resolve_both_sides() {
        let mut system = three_components();
        // E가 B 뒤에 붙음 → 변경 전 {E}와 변경 후 {A, B, E}만 다시 풉니다.
        let resolved = system.set_constraints(id(6), vec![after(3, 1.0)]).unwrap();
        assert_eq!(resolved, vec![id(2), id(3), id(6)]);
        assert_eq!(system.solve_counts(), &[2, 2, 1, 1, 2]);
        assert_eq!(system.placement(id(6)).unwrap().start, px(26));
        // B가 A와 끊김 → 변경 전 요소 {A, B, E} 전체를 다시 풉니다(C, D는 그대로).
        let resolved = system
            .set_constraints(id(3), vec![from_parent(50.0)])
            .unwrap();
        assert_eq!(resolved, vec![id(2), id(3), id(6)]);
        assert_eq!(system.solve_counts(), &[3, 3, 1, 1, 3]);
        assert_eq!(system.placement(id(6)).unwrap().start, px(61));
        // 같은 결과를 처음부터 풀어도 같습니다(증분 = 전체 계산).
        let full = ConstraintSystem::new(
            Axis::Horizontal,
            px(200),
            vec![
                sibling_input(2, vec![from_parent(0.0)]),
                sibling_input(3, vec![from_parent(50.0)]),
                sibling_input(4, vec![from_parent(100.0)]),
                sibling_input(5, vec![after(4, 5.0)]),
                sibling_input(6, vec![after(3, 1.0)]),
            ],
        )
        .unwrap();
        for n in 2..=6 {
            assert_eq!(system.placement(id(n)), full.placement(id(n)), "node {n}");
        }
    }

    #[test]
    fn failed_update_leaves_system_unchanged() {
        let mut system = three_components();
        let counts = system.solve_counts().to_vec();
        // A가 B를 참조 → 순환 → 오류, 시스템 불변.
        let result = system.set_constraints(id(2), vec![after(3, 0.0)]);
        assert!(matches!(result, Err(LayoutError::ConstraintCycle { .. })));
        assert_eq!(system.solve_counts(), counts.as_slice());
        assert_eq!(system.placement(id(2)).unwrap().start, px(0));
        assert!(system.set_constraints(id(99), Vec::new()).is_err());
        assert!(system.diagnostics().is_empty());
    }
}
