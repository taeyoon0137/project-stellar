//! Anchor·Constraint 배치 (X.5.2).
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
}
