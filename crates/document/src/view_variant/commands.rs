//! View 변수·Variant 생성·변경·삭제 command (IX.7.8).
//!
//! [`ViewVariantModel`]은 한 View의 변수 모음(IX.7.1)과 Variant 목록(IX.7.4)입니다. [`ViewVariantCommand`]는 closed
//! enum command이며, 적용 전에 전부 검증하고(실패 시 model 불변) 정확히 되돌리는 inverse command를 돌려줍니다.
//!
//! - 변수 삭제는 그 변수를 조건에 쓰는 Variant를 **지우지 않습니다**. 그 Variant는 남고 [`ViewVariantModel::diagnostics`]
//!   가 [`VariantDiagnostic::OrphanCondition`]으로 알립니다(orphan Variant의 그 term은 만족되지 않으므로 적용되지
//!   않음, IX.7.2). 변수 타입·허용 값 변경으로 조건 값이 더 이상 허용되지 않으면 `InvalidConditionValue` 진단입니다.
//! - Variant 생성·교체는 조건을 현재 변수에 대해 엄격히 검사합니다. `RestoreVariant`는 삭제·교체의 inverse 전용으로,
//!   orphan 조건을 가진 Variant도 검사 없이 그대로 되살립니다.
//! - Variant override의 scene 참조·캡슐화 검사는 base scene이 있는 계층(IX.7.3 `validate_references`, IX.7.7
//!   `validate_variant_encapsulation`)이 합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;

use super::condition::{ConditionError, VariantCondition};
use super::resolver::{ViewVariant, ViewVariantId};
use super::variable::{ViewVariable, ViewVariableError, ViewVariableId, ViewVariableSet};
use crate::component::codec::{put_bytes, put_count, put_id, Malformed, Reader};
use crate::component::variant_override::{VariantOverrideError, VariantOverrideSet};

/// 한 View의 변수와 Variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVariantModel {
    variables: ViewVariableSet,
    variants: BTreeMap<ViewVariantId, ViewVariant>,
}

/// model 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantDiagnostic {
    /// Variant 조건이 삭제된(없는) 변수를 씁니다.
    OrphanCondition {
        /// Variant.
        variant: ViewVariantId,
        /// 없는 변수.
        variable: ViewVariableId,
    },
    /// Variant 조건 값이 변수에 더 이상 허용되지 않습니다(타입·허용 값 변경).
    InvalidConditionValue {
        /// Variant.
        variant: ViewVariantId,
        /// 변수.
        variable: ViewVariableId,
    },
}

/// command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantCommandError {
    /// 변수 규칙 위반.
    Variable(ViewVariableError),
    /// 조건 규칙 위반.
    Condition(ConditionError),
    /// 같은 id의 Variant가 있습니다.
    DuplicateVariant {
        /// Variant.
        variant: ViewVariantId,
    },
    /// Variant가 없습니다.
    UnknownVariant {
        /// Variant.
        variant: ViewVariantId,
    },
    /// override bytes 오류.
    Overrides(VariantOverrideError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for VariantCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariantCommandError::Variable(err) => write!(f, "{err}"),
            VariantCommandError::Condition(err) => write!(f, "{err}"),
            VariantCommandError::DuplicateVariant { variant } => {
                write!(f, "Variant {variant}가 이미 있습니다")
            }
            VariantCommandError::UnknownVariant { variant } => {
                write!(f, "Variant {variant}가 없습니다")
            }
            VariantCommandError::Overrides(err) => write!(f, "{err}"),
            VariantCommandError::Malformed { reason } => {
                write!(f, "View variant model binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for VariantCommandError {}

impl From<ViewVariableError> for VariantCommandError {
    fn from(err: ViewVariableError) -> VariantCommandError {
        VariantCommandError::Variable(err)
    }
}

impl From<ConditionError> for VariantCommandError {
    fn from(err: ConditionError) -> VariantCommandError {
        VariantCommandError::Condition(err)
    }
}

impl From<Malformed> for VariantCommandError {
    fn from(err: Malformed) -> VariantCommandError {
        VariantCommandError::Malformed { reason: err.0 }
    }
}

impl ViewVariantModel {
    /// View의 빈 model.
    pub fn new(view: StableId) -> ViewVariantModel {
        ViewVariantModel {
            variables: ViewVariableSet::new(view),
            variants: BTreeMap::new(),
        }
    }

    /// 변수 모음.
    pub fn variables(&self) -> &ViewVariableSet {
        &self.variables
    }

    /// Variant.
    pub fn variant(&self, id: ViewVariantId) -> Option<&ViewVariant> {
        self.variants.get(&id)
    }

    /// Variant(id 오름차순).
    pub fn variants(&self) -> impl Iterator<Item = &ViewVariant> + '_ {
        self.variants.values()
    }

    /// orphan·무효 조건 진단(Variant id → 변수 id 순).
    pub fn diagnostics(&self) -> Vec<VariantDiagnostic> {
        let mut out = Vec::new();
        for variant in self.variants.values() {
            for (variable, term) in variant.condition.terms() {
                match self.variables.get(variable) {
                    None => out.push(VariantDiagnostic::OrphanCondition {
                        variant: variant.id,
                        variable,
                    }),
                    Some(record) => {
                        if let super::condition::ConditionTerm::Equals(value) = term {
                            if !record.accepts(value) {
                                out.push(VariantDiagnostic::InvalidConditionValue {
                                    variant: variant.id,
                                    variable,
                                });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// canonical binary bytes: 변수 모음 + Variant(id 오름차순: id, 조건, override bytes).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_bytes(&mut buf, &self.variables.encode());
        put_count(&mut buf, self.variants.len());
        for variant in self.variants.values() {
            put_id(&mut buf, variant.id.stable_id());
            variant.condition.encode_into(&mut buf);
            put_bytes(&mut buf, &variant.overrides.encode());
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. orphan 조건은 보존합니다(진단 대상).
    pub fn decode(bytes: &[u8]) -> Result<ViewVariantModel, VariantCommandError> {
        let mut reader = Reader::new(bytes);
        let variables = ViewVariableSet::decode(reader.bytes()?)?;
        let mut model = ViewVariantModel {
            variables,
            variants: BTreeMap::new(),
        };
        let count = reader.count()?;
        let mut previous: Option<ViewVariantId> = None;
        for _ in 0..count {
            let id = ViewVariantId::new(reader.id()?);
            if previous.is_some_and(|p| p >= id) {
                return Err(VariantCommandError::Malformed {
                    reason: "Variant가 id 오름차순이 아님",
                });
            }
            previous = Some(id);
            let condition = VariantCondition::decode_from(&mut reader)?;
            let overrides = VariantOverrideSet::decode(reader.bytes()?)
                .map_err(VariantCommandError::Overrides)?;
            model
                .variants
                .insert(id, ViewVariant::new(id, condition, overrides));
        }
        reader.expect_end()?;
        Ok(model)
    }
}

/// View 변수·Variant command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewVariantCommand {
    /// 변수를 만듭니다.
    CreateVariable(ViewVariable),
    /// 같은 id 변수를 교체합니다(이름·타입·default·허용 값 변경).
    ReplaceVariable(ViewVariable),
    /// 변수를 지웁니다. 참조 Variant는 남고 orphan 진단이 됩니다.
    DeleteVariable(ViewVariableId),
    /// Variant를 만듭니다(조건 엄격 검사).
    CreateVariant(ViewVariant),
    /// 같은 id Variant를 교체합니다(조건 엄격 검사).
    ReplaceVariant(ViewVariant),
    /// Variant를 지웁니다.
    DeleteVariant(ViewVariantId),
    /// inverse 전용: Variant를 검사 없이 그대로 둡니다(있으면 교체).
    RestoreVariant(ViewVariant),
}

impl ViewVariantCommand {
    /// 검증 후 적용하고 inverse command를 반환합니다. 실패 시 model은 바뀌지 않습니다.
    pub fn apply(
        &self,
        model: &mut ViewVariantModel,
    ) -> Result<ViewVariantCommand, VariantCommandError> {
        match self {
            ViewVariantCommand::CreateVariable(variable) => {
                model.variables.insert(variable.clone())?;
                Ok(ViewVariantCommand::DeleteVariable(variable.id()))
            }
            ViewVariantCommand::ReplaceVariable(variable) => {
                let previous = model.variables.replace(variable.clone())?;
                Ok(ViewVariantCommand::ReplaceVariable(previous))
            }
            ViewVariantCommand::DeleteVariable(id) => {
                let previous = model.variables.remove(*id)?;
                Ok(ViewVariantCommand::CreateVariable(previous))
            }
            ViewVariantCommand::CreateVariant(variant) => {
                if model.variants.contains_key(&variant.id) {
                    return Err(VariantCommandError::DuplicateVariant {
                        variant: variant.id,
                    });
                }
                variant.condition.validate(&model.variables)?;
                model.variants.insert(variant.id, variant.clone());
                Ok(ViewVariantCommand::DeleteVariant(variant.id))
            }
            ViewVariantCommand::ReplaceVariant(variant) => {
                if !model.variants.contains_key(&variant.id) {
                    return Err(VariantCommandError::UnknownVariant {
                        variant: variant.id,
                    });
                }
                variant.condition.validate(&model.variables)?;
                let previous = model.variants.insert(variant.id, variant.clone()).ok_or(
                    VariantCommandError::UnknownVariant {
                        variant: variant.id,
                    },
                )?;
                Ok(ViewVariantCommand::RestoreVariant(previous))
            }
            ViewVariantCommand::DeleteVariant(id) => {
                let previous = model
                    .variants
                    .remove(id)
                    .ok_or(VariantCommandError::UnknownVariant { variant: *id })?;
                Ok(ViewVariantCommand::RestoreVariant(previous))
            }
            ViewVariantCommand::RestoreVariant(variant) => {
                Ok(match model.variants.insert(variant.id, variant.clone()) {
                    Some(previous) => ViewVariantCommand::RestoreVariant(previous),
                    None => ViewVariantCommand::DeleteVariant(variant.id),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::condition::ConditionTerm;
    use super::super::resolver::fixtures::*;
    use super::super::variable::fixtures::*;
    use super::super::variable::VariableValue;
    use super::*;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    fn with_patch(mut variant: ViewVariant) -> ViewVariant {
        variant.overrides.set_property(
            PropertyPath::with_keys(stable(1), vec![PropertyKey(0x0110)]),
            TypedValue::Boolean(true),
        );
        variant
    }

    #[test]
    fn commands_apply_and_inverse_exactly() {
        // 검증 기준(IX.7.8): apply/inverse 통과.
        let mut model = ViewVariantModel::new(stable(VIEW));
        let initial = model.encode();
        let renamed_device = ViewVariable::new(
            vid(DEVICE),
            "formFactor".into(),
            device_variable().value_type(),
            device(0),
            device_variable().allowed_values().collect::<Vec<_>>(),
        )
        .unwrap();
        let commands = vec![
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::CreateVariable(payment_variable()),
            ViewVariantCommand::CreateVariant(with_patch(variant(COMPACT, compact()))),
            ViewVariantCommand::CreateVariant(variant(
                COMPACT_ERROR,
                compact().equals(vid(PAYMENT), payment(2)),
            )),
            ViewVariantCommand::ReplaceVariable(renamed_device),
            ViewVariantCommand::ReplaceVariant(variant(COMPACT, compact())),
            ViewVariantCommand::DeleteVariant(variant_id(COMPACT_ERROR)),
            ViewVariantCommand::DeleteVariable(vid(PAYMENT)),
        ];
        let mut inverses = Vec::new();
        let mut snapshots = vec![model.encode()];
        for command in &commands {
            inverses.push(command.apply(&mut model).unwrap());
            snapshots.push(model.encode());
        }
        // model 저장 round-trip.
        assert_eq!(ViewVariantModel::decode(&model.encode()).unwrap(), model);
        // inverse를 역순으로 적용하면 각 단계 상태로 정확히 돌아갑니다.
        for (index, inverse) in inverses.iter().enumerate().rev() {
            let redo = inverse.apply(&mut model).unwrap();
            assert_eq!(model.encode(), snapshots[index], "단계 {index} undo");
            // redo(inverse의 inverse)도 다시 같은 상태를 만듭니다.
            redo.apply(&mut model).unwrap();
            assert_eq!(model.encode(), snapshots[index + 1], "단계 {index} redo");
            inverse.apply(&mut model).unwrap();
        }
        assert_eq!(model.encode(), initial);
    }

    #[test]
    fn deleting_a_referenced_variable_leaves_orphan_diagnostics() {
        let mut model = ViewVariantModel::new(stable(VIEW));
        for command in [
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::CreateVariable(payment_variable()),
            ViewVariantCommand::CreateVariant(variant(ERROR, error())),
            ViewVariantCommand::CreateVariant(variant(
                COMPACT_ERROR,
                compact().equals(vid(PAYMENT), payment(2)),
            )),
        ] {
            command.apply(&mut model).unwrap();
        }
        assert!(model.diagnostics().is_empty());
        let undo = ViewVariantCommand::DeleteVariable(vid(PAYMENT))
            .apply(&mut model)
            .unwrap();
        // Variant는 지워지지 않고 orphan 진단으로 남습니다.
        assert_eq!(model.variants().count(), 2);
        assert_eq!(
            model.diagnostics(),
            vec![
                VariantDiagnostic::OrphanCondition {
                    variant: variant_id(ERROR),
                    variable: vid(PAYMENT),
                },
                VariantDiagnostic::OrphanCondition {
                    variant: variant_id(COMPACT_ERROR),
                    variable: vid(PAYMENT),
                },
            ]
        );
        // orphan model도 저장·로드됩니다.
        assert_eq!(ViewVariantModel::decode(&model.encode()).unwrap(), model);
        // orphan Variant의 삭제 undo는 검사 없이 정확히 복원됩니다.
        let restore = ViewVariantCommand::DeleteVariant(variant_id(ERROR))
            .apply(&mut model)
            .unwrap();
        assert!(matches!(restore, ViewVariantCommand::RestoreVariant(_)));
        restore.apply(&mut model).unwrap();
        assert_eq!(model.diagnostics().len(), 2);
        // 변수 삭제 undo 후 진단 0건.
        undo.apply(&mut model).unwrap();
        assert!(model.diagnostics().is_empty());
    }

    #[test]
    fn invalid_commands_leave_model_unchanged() {
        let mut model = ViewVariantModel::new(stable(VIEW));
        ViewVariantCommand::CreateVariable(device_variable())
            .apply(&mut model)
            .unwrap();
        let before = model.encode();
        let failing = [
            // 없는 변수 조건·허용 밖 값.
            ViewVariantCommand::CreateVariant(variant(ERROR, error())),
            ViewVariantCommand::CreateVariant(variant(
                COMPACT,
                VariantCondition::new().equals(vid(DEVICE), device(9)),
            )),
            // 중복 변수·없는 변수·없는 Variant.
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::DeleteVariable(vid(PAYMENT)),
            ViewVariantCommand::ReplaceVariable(payment_variable()),
            ViewVariantCommand::DeleteVariant(variant_id(ERROR)),
            ViewVariantCommand::ReplaceVariant(variant(ERROR, VariantCondition::new())),
        ];
        for command in failing {
            assert!(command.apply(&mut model).is_err(), "{command:?}");
        }
        assert_eq!(model.encode(), before);
        // 변수 타입 변경으로 조건 값이 무효가 되면 진단입니다.
        ViewVariantCommand::CreateVariant(variant(
            COMPACT,
            compact().with(vid(DEVICE), ConditionTerm::Equals(device(0))),
        ))
        .apply(&mut model)
        .unwrap();
        let boolean_device = ViewVariable::new(
            vid(DEVICE),
            "device".into(),
            TypeSpec::required(ExpressionType::Boolean),
            VariableValue::Value(TypedValue::Boolean(false)),
            [],
        )
        .unwrap();
        ViewVariantCommand::ReplaceVariable(boolean_device)
            .apply(&mut model)
            .unwrap();
        assert_eq!(
            model.diagnostics(),
            vec![VariantDiagnostic::InvalidConditionValue {
                variant: variant_id(COMPACT),
                variable: vid(DEVICE),
            }]
        );
    }
}
