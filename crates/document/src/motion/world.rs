//! MotionSpec 대상 해석·보간 끝값 검사·orphan 진단 (XXX.1.1, XXX.1.6, ADR-0094 결정 1·4).
//!
//! - [`MotionWorld`]: 소유자 안에서 대상 node 종류와 상태 변수 타입을 알려 주는 조회 trait입니다. 컴포넌트 소유자는
//!   [`LibraryWorld`]가 정의 tree로, View 소유자는 문서 모델로 답합니다.
//! - [`check_targets`]: 생성·변경 시 대상이 있고 포트 타입 집합에 보간할 수 있는 타입이 있는지 검사합니다.
//! - [`diagnose`]: 저장된 MotionSpec의 대상이 사라졌는지(orphan) 등을 진단합니다. 대상 삭제는 MotionSpec을 지우지
//!   않습니다.
//! - [`check_endpoints`]: 보간 시작값·끝값이 같은 타입이고(Color는 같은 색공간) 보간할 수 있는지 검사합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use super::spec::{
    is_interpolable, MotionOwner, MotionSpec, MotionSpecError, MotionSpecId, StartCondition,
};
use crate::component::definition::ComponentLibrary;
use crate::component::state_machine::StateVariableId;
use crate::model::DocumentModel;
use crate::prototype::socket::{property_port, PropertyPort, TargetKind};

/// MotionSpec 대상 조회.
pub trait MotionWorld {
    /// 소유자 `owner` 안의 node `root` 종류. 없으면 `None`.
    fn target_kind(&self, owner: MotionOwner, root: StableId) -> Option<TargetKind>;
    /// 소유자 `owner`의 상태 변수 타입. 없으면 `None`.
    fn variable_type(&self, owner: MotionOwner, variable: StateVariableId) -> Option<TypeSpec>;
}

/// 컴포넌트 library와(있으면) View 문서 모델로 답하는 [`MotionWorld`].
#[derive(Clone, Copy)]
pub struct LibraryWorld<'a> {
    /// 컴포넌트 library.
    pub library: &'a ComponentLibrary,
    /// View 소유자 대상 문서(없으면 View 대상은 모두 없음).
    pub model: Option<&'a DocumentModel>,
}

impl MotionWorld for LibraryWorld<'_> {
    fn target_kind(&self, owner: MotionOwner, root: StableId) -> Option<TargetKind> {
        match owner {
            MotionOwner::Component(component) => self
                .library
                .get(component)?
                .tree()
                .node(root)
                .map(|node| TargetKind::Node(node.kind)),
            MotionOwner::View(_) => self
                .model?
                .payload(root)
                .map(|payload| TargetKind::Node(payload.kind())),
        }
    }

    fn variable_type(&self, owner: MotionOwner, variable: StateVariableId) -> Option<TypeSpec> {
        match owner {
            MotionOwner::Component(component) => self
                .library
                .get(component)?
                .state_machine()
                .variable(variable)
                .map(|v| v.value_type()),
            // View 최상위 트리는 상태 머신이 없습니다(상태 변수는 컴포넌트 정의가 가집니다).
            MotionOwner::View(_) => None,
        }
    }
}

/// orphan 원인.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanCause {
    /// 대상 node가 소유자 안에 없습니다.
    MissingNode,
    /// node는 있지만 그 속성 포트가 없습니다(진단 코드).
    MissingProperty(&'static str),
}

/// MotionSpec 진단.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionDiagnostic {
    /// 대상이 사라진 MotionSpec(삭제하지 않고 남김).
    OrphanTarget {
        /// MotionSpec.
        motion: MotionSpecId,
        /// 대상.
        target: PropertyPath,
        /// 원인.
        cause: OrphanCause,
    },
    /// 대상 포트에 보간할 수 있는 타입이 없습니다.
    NotInterpolable {
        /// MotionSpec.
        motion: MotionSpecId,
        /// 대상.
        target: PropertyPath,
    },
    /// 시작 조건의 상태 변수가 없습니다.
    MissingVariable {
        /// MotionSpec.
        motion: MotionSpecId,
        /// 변수.
        variable: StateVariableId,
    },
    /// 시작 조건 값이 변수 타입과 맞지 않습니다.
    ConditionTypeMismatch {
        /// MotionSpec.
        motion: MotionSpecId,
        /// 변수 타입.
        expected: TypeSpec,
    },
}

impl MotionDiagnostic {
    /// 안정 진단 코드.
    pub const fn code(&self) -> &'static str {
        match self {
            MotionDiagnostic::OrphanTarget { .. } => "motion-orphan-target",
            MotionDiagnostic::NotInterpolable { .. } => "motion-not-interpolable",
            MotionDiagnostic::MissingVariable { .. } => "motion-missing-variable",
            MotionDiagnostic::ConditionTypeMismatch { .. } => "motion-condition-type",
        }
    }

    /// 진단 대상 MotionSpec.
    pub const fn motion(&self) -> MotionSpecId {
        match self {
            MotionDiagnostic::OrphanTarget { motion, .. }
            | MotionDiagnostic::NotInterpolable { motion, .. }
            | MotionDiagnostic::MissingVariable { motion, .. }
            | MotionDiagnostic::ConditionTypeMismatch { motion, .. } => *motion,
        }
    }
}

impl fmt::Display for MotionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.code(), self.motion())
    }
}

/// 대상 경로의 포트. 경로 key가 하나라고 가정합니다([`MotionSpec::new`]가 보장).
pub fn target_port(
    world: &dyn MotionWorld,
    owner: MotionOwner,
    target: &PropertyPath,
) -> Result<PropertyPort, OrphanCause> {
    let kind = world
        .target_kind(owner, target.root())
        .ok_or(OrphanCause::MissingNode)?;
    let key = *target.keys().first().ok_or(OrphanCause::MissingNode)?;
    property_port(kind, key).map_err(|diag| OrphanCause::MissingProperty(diag.code))
}

/// MotionSpec 하나의 진단(대상 순서, 그 뒤 시작 조건).
pub fn diagnose(spec: &MotionSpec, world: &dyn MotionWorld) -> Vec<MotionDiagnostic> {
    let mut out = Vec::new();
    for target in spec.targets() {
        match target_port(world, spec.owner(), target) {
            Err(cause) => out.push(MotionDiagnostic::OrphanTarget {
                motion: spec.id(),
                target: target.clone(),
                cause,
            }),
            Ok(port) => {
                if !port.types.iter().any(|t| is_interpolable(*t)) {
                    out.push(MotionDiagnostic::NotInterpolable {
                        motion: spec.id(),
                        target: target.clone(),
                    });
                }
            }
        }
    }
    let variable = spec.start().variable();
    match world.variable_type(spec.owner(), variable) {
        None => out.push(MotionDiagnostic::MissingVariable {
            motion: spec.id(),
            variable,
        }),
        Some(expected) => {
            if let StartCondition::Becomes { value, .. } = spec.start() {
                if !expected.accepts(Some(value)) {
                    out.push(MotionDiagnostic::ConditionTypeMismatch {
                        motion: spec.id(),
                        expected,
                    });
                }
            }
        }
    }
    out
}

/// 생성·변경 전 검사: 진단이 하나라도 있으면 거부합니다(저장된 spec의 orphan은 [`diagnose`]로만 보고합니다).
pub fn check_targets(spec: &MotionSpec, world: &dyn MotionWorld) -> Result<(), MotionCheckError> {
    match diagnose(spec, world).into_iter().next() {
        None => Ok(()),
        Some(MotionDiagnostic::NotInterpolable { target, .. }) => {
            let found = target_port(world, spec.owner(), &target)
                .ok()
                .and_then(|port| port.types.iter().next().copied())
                .unwrap_or(ExpressionType::Boolean);
            Err(MotionCheckError::Spec(MotionSpecError::NotInterpolable {
                target,
                found,
            }))
        }
        Some(other) => Err(MotionCheckError::Diagnostic(other)),
    }
}

/// 생성·변경 거부 사유.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionCheckError {
    /// spec 자체 오류.
    Spec(MotionSpecError),
    /// 대상·조건 진단.
    Diagnostic(MotionDiagnostic),
}

impl fmt::Display for MotionCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MotionCheckError::Spec(error) => error.fmt(f),
            MotionCheckError::Diagnostic(diag) => diag.fmt(f),
        }
    }
}

impl std::error::Error for MotionCheckError {}

/// 보간 끝값 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointError {
    /// 시작값과 끝값의 타입이 다릅니다(합집합 포트의 다른 구성 타입 등).
    TypeMismatch {
        /// 시작값 타입.
        start: ExpressionType,
        /// 끝값 타입.
        end: ExpressionType,
    },
    /// 보간할 수 없는 타입입니다.
    NotInterpolable(ExpressionType),
    /// Color 끝값의 색공간이 다릅니다(변환에 초월함수가 필요해 보간하지 않습니다).
    ColorSpaceMismatch,
    /// 값이 없습니다(null).
    Missing,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndpointError::TypeMismatch { start, end } => {
                write!(f, "시작값({start:?})과 끝값({end:?})의 타입이 다릅니다")
            }
            EndpointError::NotInterpolable(t) => write!(f, "{t:?} 값은 보간할 수 없습니다"),
            EndpointError::ColorSpaceMismatch => f.write_str("시작색과 끝색의 색공간이 다릅니다"),
            EndpointError::Missing => f.write_str("보간할 값이 없습니다"),
        }
    }
}

impl std::error::Error for EndpointError {}

impl EndpointError {
    /// 안정 진단 코드.
    pub const fn code(&self) -> &'static str {
        match self {
            EndpointError::TypeMismatch { .. } => "motion-endpoint-type",
            EndpointError::NotInterpolable(_) => "motion-endpoint-not-interpolable",
            EndpointError::ColorSpaceMismatch => "motion-color-space-mismatch",
            EndpointError::Missing => "motion-endpoint-missing",
        }
    }
}

/// 보간 시작값 `start`와 끝값 `end`를 검사합니다.
pub fn check_endpoints(
    start: Option<&TypedValue>,
    end: Option<&TypedValue>,
) -> Result<ExpressionType, EndpointError> {
    let (Some(start), Some(end)) = (start, end) else {
        return Err(EndpointError::Missing);
    };
    let (a, b) = (ExpressionType::of(start), ExpressionType::of(end));
    if a != b {
        return Err(EndpointError::TypeMismatch { start: a, end: b });
    }
    if !is_interpolable(a) {
        return Err(EndpointError::NotInterpolable(a));
    }
    if let (TypedValue::Color(x), TypedValue::Color(y)) = (start, end) {
        if x.space() != y.space() {
            return Err(EndpointError::ColorSpaceMismatch);
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Length;

    use super::*;

    fn color(space: ColorSpace) -> TypedValue {
        TypedValue::Color(Color::new(space, 1.0, 0.0, 0.0, 1.0).expect("색"))
    }

    #[test]
    fn color_endpoints_in_different_spaces_are_rejected() {
        // 검증 기준(XXX.1.1): 색공간이 다른 Color 끝값 fixture 거부.
        let srgb = color(ColorSpace::Srgb);
        let p3 = color(ColorSpace::DisplayP3);
        assert_eq!(
            check_endpoints(Some(&srgb), Some(&p3)),
            Err(EndpointError::ColorSpaceMismatch)
        );
        assert_eq!(
            check_endpoints(Some(&srgb), Some(&srgb)),
            Ok(ExpressionType::Color)
        );
    }

    #[test]
    fn non_interpolable_and_mixed_endpoints_are_rejected() {
        // 검증 기준(XXX.1.1): 보간할 수 없는 대상 타입 fixture 거부.
        let yes = TypedValue::Boolean(true);
        assert_eq!(
            check_endpoints(Some(&yes), Some(&yes)),
            Err(EndpointError::NotInterpolable(ExpressionType::Boolean))
        );
        let length = TypedValue::Length(Length::new(10.0).expect("길이"));
        let percent = TypedValue::Percentage(
            stellar_core_types::value::Percentage::new(50.0).expect("백분율"),
        );
        assert!(matches!(
            check_endpoints(Some(&length), Some(&percent)),
            Err(EndpointError::TypeMismatch { .. })
        ));
        assert_eq!(
            check_endpoints(None, Some(&length)),
            Err(EndpointError::Missing)
        );
    }
}
