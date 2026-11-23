//! View 변수·Variant의 영속 형식 보조 (XVI.11.1, ADR-0126).
//!
//! View 하나의 변수와 Variant([`ViewVariantModel`], IX.7)는 문서 데이터입니다. 저장 위치와 형식은 ADR-0126입니다.
//!
//! - 문서 레코드: [`DocumentRecords::view_variants`](crate::records::DocumentRecords::view_variants), 모델:
//!   [`DocumentModel::view_variants`](crate::model::DocumentModel::view_variants).
//! - chunk: View root subtree의 확장 절([`crate::guides::persist`]). record bytes는 [`ViewVariantModel::encode`] 그대로입니다.
//! - 텍스트 원본과 semantic snapshot은 변수 하나·Variant 하나 단위로 나눠 씁니다. 이 module은 그 단위의 canonical bytes와
//!   단위 목록에서 model을 다시 조립하는 함수를 둡니다. 조립은 orphan 조건(삭제된 변수를 쓰는 조건)을 그대로 보존합니다.

use crate::component::codec::{put_bytes, put_id, put_u8, Reader};
use crate::component::variant_override::VariantOverrideSet;
use crate::view_variant::commands::{VariantCommandError, ViewVariantCommand, ViewVariantModel};
use crate::view_variant::condition::VariantCondition;
use crate::view_variant::resolver::{ViewVariant, ViewVariantId};
use crate::view_variant::variable::{ViewVariable, ViewVariableError, ViewVariableId};

/// 변수도 Variant도 없으면 참(저장할 model이 없음).
pub fn is_empty_model(model: &ViewVariantModel) -> bool {
    model.variables().is_empty() && model.variants().next().is_none()
}

/// 변수 하나의 canonical bytes([`ViewVariableSet::encode`](crate::view_variant::variable::ViewVariableSet::encode) 항목과 같은 형식).
pub fn variable_bytes(variable: &ViewVariable) -> Vec<u8> {
    let mut buf = Vec::new();
    variable.encode_into(&mut buf);
    buf
}

/// [`variable_bytes`]의 역. 정규형이 아닌 bytes는 거부합니다.
pub fn variable_from_bytes(bytes: &[u8]) -> Result<ViewVariable, ViewVariableError> {
    let mut reader = Reader::new(bytes);
    let variable = ViewVariable::decode_from(&mut reader)?;
    reader.expect_end()?;
    if variable_bytes(&variable) != bytes {
        return Err(ViewVariableError::Malformed {
            reason: "정규형이 아닌 View 변수 bytes",
        });
    }
    Ok(variable)
}

/// Variant 하나의 canonical bytes([`ViewVariantModel::encode`]의 Variant 항목과 같은 형식: id, 조건, override bytes).
pub fn variant_bytes(variant: &ViewVariant) -> Vec<u8> {
    let mut buf = Vec::new();
    put_variant(&mut buf, variant);
    buf
}

fn put_variant(buf: &mut Vec<u8>, variant: &ViewVariant) {
    put_id(buf, variant.id.stable_id());
    variant.condition.encode_into(buf);
    put_bytes(buf, &variant.overrides.encode());
}

fn read_variant(reader: &mut Reader<'_>) -> Result<ViewVariant, VariantCommandError> {
    let id = ViewVariantId::new(reader.id()?);
    let condition = VariantCondition::decode_from(reader)?;
    let overrides =
        VariantOverrideSet::decode(reader.bytes()?).map_err(VariantCommandError::Overrides)?;
    Ok(ViewVariant::new(id, condition, overrides))
}

/// [`variant_bytes`]의 역. 남는 bytes·정규형이 아닌 bytes는 거부합니다.
pub fn variant_from_bytes(bytes: &[u8]) -> Result<ViewVariant, VariantCommandError> {
    let mut reader = Reader::new(bytes);
    let variant = read_variant(&mut reader)?;
    reader.expect_end()?;
    if variant_bytes(&variant) != bytes {
        return Err(VariantCommandError::Malformed {
            reason: "정규형이 아닌 Variant bytes",
        });
    }
    Ok(variant)
}

const COMMAND_CREATE_VARIABLE: u8 = 1;
const COMMAND_REPLACE_VARIABLE: u8 = 2;
const COMMAND_DELETE_VARIABLE: u8 = 3;
const COMMAND_CREATE_VARIANT: u8 = 4;
const COMMAND_REPLACE_VARIANT: u8 = 5;
const COMMAND_DELETE_VARIANT: u8 = 6;
const COMMAND_RESTORE_VARIANT: u8 = 7;

/// View 변수·Variant command 하나의 canonical bytes(history·command payload, ADR-0126). tag `u8` 뒤에 변수 bytes
/// ([`variable_bytes`]), Variant bytes([`variant_bytes`]) 또는 id 16 bytes입니다.
pub fn encode_command(command: &ViewVariantCommand) -> Vec<u8> {
    let mut buf = Vec::new();
    match command {
        ViewVariantCommand::CreateVariable(variable) => {
            put_u8(&mut buf, COMMAND_CREATE_VARIABLE);
            variable.encode_into(&mut buf);
        }
        ViewVariantCommand::ReplaceVariable(variable) => {
            put_u8(&mut buf, COMMAND_REPLACE_VARIABLE);
            variable.encode_into(&mut buf);
        }
        ViewVariantCommand::DeleteVariable(id) => {
            put_u8(&mut buf, COMMAND_DELETE_VARIABLE);
            put_id(&mut buf, id.stable_id());
        }
        ViewVariantCommand::CreateVariant(variant) => {
            put_u8(&mut buf, COMMAND_CREATE_VARIANT);
            put_variant(&mut buf, variant);
        }
        ViewVariantCommand::ReplaceVariant(variant) => {
            put_u8(&mut buf, COMMAND_REPLACE_VARIANT);
            put_variant(&mut buf, variant);
        }
        ViewVariantCommand::DeleteVariant(id) => {
            put_u8(&mut buf, COMMAND_DELETE_VARIANT);
            put_id(&mut buf, id.stable_id());
        }
        ViewVariantCommand::RestoreVariant(variant) => {
            put_u8(&mut buf, COMMAND_RESTORE_VARIANT);
            put_variant(&mut buf, variant);
        }
    }
    buf
}

/// [`encode_command`]의 역. 남는 bytes는 거부합니다.
pub fn decode_command(bytes: &[u8]) -> Result<ViewVariantCommand, VariantCommandError> {
    let mut reader = Reader::new(bytes);
    let command = match reader.u8()? {
        COMMAND_CREATE_VARIABLE => {
            ViewVariantCommand::CreateVariable(ViewVariable::decode_from(&mut reader)?)
        }
        COMMAND_REPLACE_VARIABLE => {
            ViewVariantCommand::ReplaceVariable(ViewVariable::decode_from(&mut reader)?)
        }
        COMMAND_DELETE_VARIABLE => {
            ViewVariantCommand::DeleteVariable(ViewVariableId::new(reader.id()?))
        }
        COMMAND_CREATE_VARIANT => ViewVariantCommand::CreateVariant(read_variant(&mut reader)?),
        COMMAND_REPLACE_VARIANT => ViewVariantCommand::ReplaceVariant(read_variant(&mut reader)?),
        COMMAND_DELETE_VARIANT => {
            ViewVariantCommand::DeleteVariant(ViewVariantId::new(reader.id()?))
        }
        COMMAND_RESTORE_VARIANT => ViewVariantCommand::RestoreVariant(read_variant(&mut reader)?),
        _ => {
            return Err(VariantCommandError::Malformed {
                reason: "알 수 없는 View 변수 command tag",
            })
        }
    };
    reader.expect_end()?;
    Ok(command)
}

/// View `view`의 변수와 Variant로 model을 조립합니다. 변수 이름 중복·같은 id는 거부하고, Variant 조건은 검사하지 않습니다
/// (저장된 orphan 조건을 그대로 되살림).
pub fn model_from_parts(
    view: stellar_core_types::ids::StableId,
    variables: impl IntoIterator<Item = ViewVariable>,
    variants: impl IntoIterator<Item = ViewVariant>,
) -> Result<ViewVariantModel, VariantCommandError> {
    let mut model = ViewVariantModel::new(view);
    for variable in variables {
        ViewVariantCommand::CreateVariable(variable).apply(&mut model)?;
    }
    for variant in variants {
        if model.variant(variant.id).is_some() {
            return Err(VariantCommandError::DuplicateVariant {
                variant: variant.id,
            });
        }
        ViewVariantCommand::RestoreVariant(variant).apply(&mut model)?;
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_variant::resolver::fixtures::*;
    use crate::view_variant::variable::fixtures::*;

    #[test]
    fn parts_rebuild_the_same_model_including_orphan_conditions() {
        let mut model = ViewVariantModel::new(stable(VIEW));
        for command in [
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::CreateVariable(payment_variable()),
            ViewVariantCommand::CreateVariant(variant(COMPACT, compact())),
            ViewVariantCommand::CreateVariant(variant(ERROR, error())),
            ViewVariantCommand::DeleteVariable(vid(PAYMENT)),
        ] {
            command.apply(&mut model).unwrap();
        }
        assert!(!model.diagnostics().is_empty(), "orphan 조건");
        let variables: Vec<ViewVariable> = model
            .variables()
            .iter()
            .map(|v| variable_from_bytes(&variable_bytes(v)).unwrap())
            .collect();
        let rebuilt = model_from_parts(
            stable(VIEW),
            variables,
            model.variants().cloned().collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(rebuilt, model);
        assert!(!is_empty_model(&rebuilt));
        assert!(is_empty_model(&ViewVariantModel::new(stable(VIEW))));
        let mut bytes = variable_bytes(&device_variable());
        bytes.push(0);
        assert!(variable_from_bytes(&bytes).is_err());
        assert!(model_from_parts(
            stable(VIEW),
            [],
            [variant(COMPACT, compact()), variant(COMPACT, compact())]
        )
        .is_err());
        for variant in rebuilt.variants() {
            assert_eq!(
                &variant_from_bytes(&variant_bytes(variant)).unwrap(),
                variant
            );
        }
    }

    #[test]
    fn commands_round_trip_through_bytes() {
        let commands = [
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::ReplaceVariable(payment_variable()),
            ViewVariantCommand::DeleteVariable(vid(PAYMENT)),
            ViewVariantCommand::CreateVariant(variant(COMPACT, compact())),
            ViewVariantCommand::ReplaceVariant(variant(ERROR, error())),
            ViewVariantCommand::DeleteVariant(variant_id(ERROR)),
            ViewVariantCommand::RestoreVariant(variant(COMPACT, compact())),
        ];
        for command in commands {
            let bytes = encode_command(&command);
            assert_eq!(decode_command(&bytes).unwrap(), command);
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(decode_command(&longer).is_err());
        }
        assert!(decode_command(&[0x7f]).is_err());
    }
}
