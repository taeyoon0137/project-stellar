//! 레이아웃 오류 (X.1.1).
//!
//! 입력 구조·값 검증, 계산 중 수치 범위 초과, canonical bytes 디코딩 실패를 하나의
//! [`LayoutError`]로 보고합니다. 오류는 문서를 바꾸지 않으며 부분 결과를 만들지 않습니다.

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::geometry::Axis;
use crate::intrinsic::ContentKind;
use crate::property::LayoutProperty;
use crate::rounding::LayoutUnitError;

/// 레이아웃 계산의 크기·작업량 상한 종류.
///
/// 상한은 신뢰할 수 없는 입력이 계산 시간·메모리를 무한히 늘리지 못하게 하는 결정적 값입니다.
/// 같은 입력은 플랫폼·빌드와 무관하게 같은 상한에 걸립니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutLimit {
    /// grid line 번호가 `u16` 범위를 넘습니다. 한 축의 track은
    /// [`crate::grid::MAX_GRID_TRACKS`]개(line 1~65535)까지입니다.
    GridLines,
    /// grid cell 수(열 × 행)가 [`crate::grid::MAX_GRID_CELLS`]를 넘습니다.
    GridCells,
    /// grid 배치·track sizing 작업량이 [`crate::grid::MAX_GRID_WORK`]를 넘습니다.
    GridWork,
}

impl fmt::Display for LayoutLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LayoutLimit::GridLines => "grid line 범위(u16)",
            LayoutLimit::GridCells => "grid cell 수",
            LayoutLimit::GridWork => "grid 배치 작업량",
        })
    }
}

/// 레이아웃 오류 (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// 같은 StableId의 node가 둘 이상입니다.
    DuplicateNode {
        /// 중복 node.
        node: StableId,
    },
    /// root node가 입력에 없습니다.
    MissingRoot {
        /// 지정된 root.
        root: StableId,
    },
    /// 자식 목록이 존재하지 않는 node를 가리킵니다.
    MissingChild {
        /// 부모.
        parent: StableId,
        /// 없는 자식.
        child: StableId,
    },
    /// node가 둘 이상의 부모(또는 같은 부모에 두 번) 자식으로 등록되었습니다.
    MultipleParents {
        /// 대상 node.
        node: StableId,
    },
    /// 부모·자식 관계가 순환합니다(root가 자식으로 등록된 경우 포함).
    Cycle {
        /// 순환에 포함된 node.
        node: StableId,
    },
    /// root에서 도달할 수 없는 node가 있습니다.
    Unreachable {
        /// 도달 불가 node.
        node: StableId,
    },
    /// tree 깊이가 레이아웃 재귀 제한을 넘습니다.
    DepthLimitExceeded {
        /// 제한을 넘은 node.
        node: StableId,
        /// 허용 최대 깊이.
        limit: usize,
    },
    /// 깊은 tree를 계산할 전용 stack thread를 만들 수 없습니다(자원 부족).
    DeepStackUnavailable {
        /// tree 깊이.
        depth: usize,
    },
    /// intrinsic 콘텐츠를 가진 leaf에 자식이 있습니다.
    ChildrenOnContentLeaf {
        /// 대상 node.
        node: StableId,
    },
    /// 음수일 수 없는 속성 값이 음수입니다.
    NegativeValue {
        /// 대상 node.
        node: StableId,
        /// 속성.
        property: LayoutProperty,
    },
    /// 속성 값이 레이아웃 수치(subpixel) 범위를 벗어납니다.
    OutOfRange {
        /// 대상 node.
        node: StableId,
        /// 속성.
        property: LayoutProperty,
    },
    /// 0이 아닌 가중치 값(flex grow·shrink, grid fr, aspect ratio 성분)이 1/64 격자에서 0으로
    /// 반올림됩니다(1/128 미만). 조용히 0으로 바꾸지 않고 거부합니다.
    BelowResolution {
        /// 대상 node.
        node: StableId,
        /// 속성.
        property: LayoutProperty,
    },
    /// sizing mode 값·조합이 허용되지 않습니다 (X.6.1).
    InvalidSizing {
        /// 대상 node.
        node: StableId,
        /// 축.
        axis: Axis,
        /// 사유.
        reason: &'static str,
    },
    /// 한 축의 제약이 과다합니다(양 끝 anchor + definite 크기 등, X.2.2).
    OverConstrained {
        /// 대상 node.
        node: StableId,
        /// 축.
        axis: Axis,
    },
    /// Grid track·배치 설정이 잘못되었습니다 (X.4).
    InvalidGrid {
        /// 대상 node.
        node: StableId,
        /// 사유.
        reason: &'static str,
    },
    /// 계산 크기·작업량 상한을 넘었습니다.
    LimitExceeded {
        /// 상한에 걸린 node.
        node: StableId,
        /// 상한 종류.
        limit: LayoutLimit,
    },
    /// constraint 값·조합이 잘못되었습니다 (X.5).
    InvalidConstraint {
        /// 대상 node.
        node: StableId,
        /// 사유.
        reason: &'static str,
    },
    /// constraint sibling 참조가 순환합니다 (X.5.4).
    ConstraintCycle {
        /// 순환에 속한 node.
        node: StableId,
        /// 축.
        axis: Axis,
    },
    /// 콘텐츠 intrinsic 측정이 실패했거나 결과가 잘못되었습니다 (X.6.4).
    ContentMeasure {
        /// 측정 node.
        node: StableId,
        /// 콘텐츠 종류.
        kind: ContentKind,
        /// 사유.
        reason: &'static str,
    },
    /// 레이아웃 결과 box의 불변식이 깨졌습니다.
    InvalidBox {
        /// 대상 node.
        node: StableId,
        /// 사유.
        reason: &'static str,
    },
    /// 결과에 같은 node의 box가 둘 이상입니다.
    DuplicateBox {
        /// 중복 node.
        node: StableId,
    },
    /// 계산 중 레이아웃 수치 오류(범위 초과 등).
    Arithmetic {
        /// 계산 중이던 node.
        node: StableId,
        /// 원인.
        error: LayoutUnitError,
    },
    /// canonical bytes 형식이 잘못되었습니다.
    Malformed {
        /// 사유.
        reason: &'static str,
    },
    /// canonical bytes에 비유한 float(NaN·±무한대)이 있습니다.
    NonFinite {
        /// 위치 설명.
        field: &'static str,
    },
    /// 증분 레이아웃 edit가 없는 node를 지우려 했습니다 (X.7.3).
    UnknownNode {
        /// 없는 node.
        node: StableId,
    },
    /// 직전 증분 레이아웃 계산이 실패해 결과가 없습니다 (X.7.3). 다음 edit 적용이 전체 계산으로 복구합니다.
    LayoutUnavailable,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::DuplicateNode { node } => {
                write!(f, "레이아웃 node가 중복되었습니다: {node}")
            }
            LayoutError::MissingRoot { root } => {
                write!(f, "레이아웃 root node가 없습니다: {root}")
            }
            LayoutError::MissingChild { parent, child } => {
                write!(f, "node {parent}의 자식 {child}가 입력에 없습니다")
            }
            LayoutError::MultipleParents { node } => {
                write!(f, "node {node}가 둘 이상의 부모 자식으로 등록되었습니다")
            }
            LayoutError::Cycle { node } => {
                write!(f, "레이아웃 tree가 node {node}에서 순환합니다")
            }
            LayoutError::Unreachable { node } => {
                write!(f, "node {node}는 root에서 도달할 수 없습니다")
            }
            LayoutError::DepthLimitExceeded { node, limit } => {
                write!(f, "node {node}가 레이아웃 최대 깊이 {limit}를 넘습니다")
            }
            LayoutError::DeepStackUnavailable { depth } => {
                write!(
                    f,
                    "깊이 {depth} tree를 계산할 레이아웃 stack thread를 만들 수 없습니다"
                )
            }
            LayoutError::ChildrenOnContentLeaf { node } => {
                write!(f, "intrinsic 콘텐츠 leaf {node}는 자식을 가질 수 없습니다")
            }
            LayoutError::NegativeValue { node, property } => {
                write!(f, "node {node}의 {property} 값은 음수일 수 없습니다")
            }
            LayoutError::OutOfRange { node, property } => {
                write!(
                    f,
                    "node {node}의 {property} 값이 레이아웃 수치 범위를 벗어납니다"
                )
            }
            LayoutError::BelowResolution { node, property } => {
                write!(
                    f,
                    "node {node}의 {property} 값이 0이 아니지만 레이아웃 해상도(1/64)에서 0으로 반올림됩니다"
                )
            }
            LayoutError::InvalidSizing { node, axis, reason } => {
                write!(f, "node {node}의 {axis} sizing이 잘못되었습니다: {reason}")
            }
            LayoutError::OverConstrained { node, axis } => {
                write!(
                    f,
                    "node {node}의 {axis} 배치 제약이 과다합니다(우선순위로 해소하지 않음)"
                )
            }
            LayoutError::InvalidGrid { node, reason } => {
                write!(f, "node {node}의 grid 설정이 잘못되었습니다: {reason}")
            }
            LayoutError::LimitExceeded { node, limit } => {
                write!(f, "node {node}의 레이아웃이 {limit} 상한을 넘습니다")
            }
            LayoutError::InvalidConstraint { node, reason } => {
                write!(f, "node {node}의 constraint가 잘못되었습니다: {reason}")
            }
            LayoutError::ConstraintCycle { node, axis } => {
                write!(f, "node {node}의 {axis} constraint 참조가 순환합니다")
            }
            LayoutError::ContentMeasure { node, kind, reason } => {
                write!(f, "node {node}의 {kind} 콘텐츠 측정 실패: {reason}")
            }
            LayoutError::InvalidBox { node, reason } => {
                write!(f, "node {node}의 레이아웃 box가 잘못되었습니다: {reason}")
            }
            LayoutError::DuplicateBox { node } => {
                write!(f, "레이아웃 결과에 node {node}의 box가 중복되었습니다")
            }
            LayoutError::Arithmetic { node, error } => {
                write!(f, "node {node} 레이아웃 계산 실패: {error}")
            }
            LayoutError::Malformed { reason } => {
                write!(f, "레이아웃 bytes 형식 오류: {reason}")
            }
            LayoutError::NonFinite { field } => {
                write!(f, "레이아웃 bytes의 {field} 값이 유한하지 않습니다")
            }
            LayoutError::UnknownNode { node } => {
                write!(f, "증분 레이아웃 입력에 node {node}가 없습니다")
            }
            LayoutError::LayoutUnavailable => {
                f.write_str("직전 증분 레이아웃 계산이 실패해 결과가 없습니다")
            }
        }
    }
}

impl std::error::Error for LayoutError {}
