//! provision 값의 View Variant·Target Profile override (XXX.7.9, ADR-0025 결정 3.6, ADR-0099 결정 9).
//!
//! provision 값은 PropertyPath `(provision StableId, [PROVISION_VALUE])`로 주소를 가집니다. 리터럴 값 provision은 그 경로의
//! 공통값(Base)이고, View Variant(IX.7.3)와 Target Profile override(IX.10.7)는 같은 경로에 TypedValue를 둡니다. 우선순위
//! 계산은 기존 resolver(`view_variant::resolver`, `stellar_target_profile::overrides::resolve_for_profile`)가 하고, 이 모듈은
//! 그 결과를 provision 값에 다시 넣습니다. 문서 crate는 target-profile에 의존하지 않으므로 둘을 잇는 일은 ops가 합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
use stellar_expressions::types::ExpressionType;

use super::provision::PROVISION_VALUE;
use super::set::{ContextKeys, ContextSet};
use crate::view_variant::resolver::{PatchKey, PatchValue, VariantResolution, ViewVariantId};

/// path가 provision 값 경로면 그 provision StableId.
pub fn provision_of_path(path: &PropertyPath) -> Option<StableId> {
    (path.keys() == [PROVISION_VALUE]
        && *path == PropertyPath::with_keys(path.root(), vec![PROVISION_VALUE]))
    .then_some(path.root())
}

/// 리터럴 값 provision의 공통값(경로 오름차순).
pub fn common_values(set: &ContextSet) -> BTreeMap<PropertyPath, TypedValue> {
    set.provisions()
        .filter_map(|p| p.literal_value().map(|v| (p.value_path(), v.clone())))
        .collect()
}

/// View Variant resolver 결과 가운데 provision 값 경로의 승자 값(값을 정한 variant, 값).
pub fn variant_values(
    resolution: &VariantResolution,
) -> BTreeMap<PropertyPath, (StableId, TypedValue)> {
    resolution
        .applied
        .iter()
        .filter_map(|(key, (variant, value))| match (key, value) {
            (PatchKey::Property(path), PatchValue::Property(value))
                if provision_of_path(path).is_some() =>
            {
                Some((path.clone(), (variant.stable_id(), value.clone())))
            }
            _ => None,
        })
        .collect()
}

/// 같은 구체성 Variant가 같은 provision 값을 서로 다르게 지정한 충돌(적용하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOverrideConflict {
    /// provision.
    pub provision: StableId,
    /// 충돌 group의 구체성.
    pub specificity: usize,
    /// 충돌한 Variant(id 순).
    pub variants: Vec<ViewVariantId>,
}

/// provision 값 경로의 Variant 충돌(provision 순).
pub fn variant_conflicts(resolution: &VariantResolution) -> Vec<ContextOverrideConflict> {
    resolution
        .conflicts
        .iter()
        .filter_map(|conflict| match &conflict.key {
            PatchKey::Property(path) => {
                provision_of_path(path).map(|provision| ContextOverrideConflict {
                    provision,
                    specificity: conflict.specificity,
                    variants: conflict.variants.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

/// override 적용 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextOverrideError {
    /// 경로의 provision이 없습니다.
    UnknownProvision {
        /// provision.
        provision: StableId,
    },
    /// override 값 타입이 key 타입과 다릅니다.
    TypeMismatch {
        /// provision.
        provision: StableId,
    },
}

impl fmt::Display for ContextOverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextOverrideError::UnknownProvision { provision } => {
                write!(f, "override 대상 provision {provision}이 없습니다")
            }
            ContextOverrideError::TypeMismatch { provision } => {
                write!(
                    f,
                    "provision {provision} override 값 타입이 key 타입과 다릅니다"
                )
            }
        }
    }
}

impl std::error::Error for ContextOverrideError {}

/// 해석된 provision 값(경로 → 값)을 넣은 set. 값 식은 리터럴 하나(root node id = provision StableId)로 바뀝니다. provision
/// 값 경로가 아닌 항목은 무시합니다.
pub fn with_overrides<'a>(
    set: &ContextSet,
    keys: &ContextKeys<'_>,
    values: impl IntoIterator<Item = (&'a PropertyPath, &'a TypedValue)>,
) -> Result<ContextSet, ContextOverrideError> {
    let mut out = set.clone();
    for (path, value) in values {
        let Some(provision) = provision_of_path(path) else {
            continue;
        };
        let current = out
            .provision(provision)
            .ok_or(ContextOverrideError::UnknownProvision { provision })?;
        if current.literal_value() == Some(value) {
            continue;
        }
        let key = keys.key(current.key());
        if key.is_some_and(|k| k.value_type().value_type() != ExpressionType::of(value)) {
            return Err(ContextOverrideError::TypeMismatch { provision });
        }
        let root = ExprNodeId::new(provision);
        let ast = ExpressionAst::new(root, vec![ExprNode::literal(root, value.clone())])
            .map_err(|_| ContextOverrideError::TypeMismatch { provision })?;
        let next = current
            .with_value(ast)
            .map_err(|_| ContextOverrideError::TypeMismatch { provision })?;
        out.replace_provision(next)
            .map_err(|_| ContextOverrideError::UnknownProvision { provision })?;
    }
    Ok(out)
}
