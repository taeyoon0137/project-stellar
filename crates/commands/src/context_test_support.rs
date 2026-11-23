#![cfg(any(test, feature = "test-support"))]
//! **테스트 전용** Context 편집 command 생성기(XXX.7.8, VII.6.1). property test·fuzz가 [`crate::test_support`]를 거쳐 씁니다.
//! 프로덕션 경로는 이 모듈을 호출하지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_document::context::key::ContextKey;
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::expressions::ast::ExpressionAst;
use stellar_document::expressions::types::TypeSpec;
use stellar_document::model::DocumentModel;

use crate::context::{ContextCommand, ContextEdit};

/// 무작위 문서에 대한 Context 편집 하나(command 생성기, VII.6.1). key가 없으면 key를 만들고, 있으면 rename·default 변경·
/// 삭제·provision 생성·requirement 추가 가운데 하나를 고릅니다.
pub(crate) fn random_edit(
    model: &DocumentModel,
    rng: &mut stellar_document::test_support::random_document::SplitMix64,
    ids: &mut crate::test_support::FreshIds,
) -> Option<crate::command::Command> {
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::context::key::ContextScope;
    use stellar_document::expressions::ast::{ExprNode, ExprNodeId};
    use stellar_document::expressions::types::ExpressionType;

    let number = |value: f32, ids: &mut crate::test_support::FreshIds| {
        let id = ExprNodeId::new(ids.fresh());
        ExpressionAst::new(
            id,
            vec![ExprNode::literal(
                id,
                TypedValue::Number(FiniteF32::new(value).ok()?),
            )],
        )
        .ok()
    };
    let number_type = TypeSpec::required(ExpressionType::Number);
    let keys: Vec<StableId> = model.context().keys().map(ContextKey::id).collect();
    let edit = if keys.is_empty() {
        let id = ids.fresh();
        ContextEdit::CreateKey(
            ContextKey::new(
                id,
                format!("k{}", rng.below(1_000)),
                ContextScope::Document(model.document_id()),
                number_type,
                number(rng.below(10) as f32, ids)?,
            )
            .ok()?,
        )
    } else {
        let key = keys[rng.below(keys.len() as u64) as usize];
        match rng.below(5) {
            0 => ContextEdit::RenameKey {
                key,
                name: format!("r{}", rng.below(1_000_000)),
            },
            1 => ContextEdit::SetKeyDefault {
                key,
                value_type: number_type,
                default: number(rng.below(10) as f32 + 0.5, ids)?,
            },
            2 => ContextEdit::DeleteKey { key },
            3 => {
                let view = model.definition().pages.first()?.views.first()?;
                ContextEdit::CreateProvision(
                    ContextProvision::new(
                        ids.fresh(),
                        ContextOwner::View(view.id),
                        view.root_frame,
                        key,
                        number(rng.below(10) as f32, ids)?,
                    )
                    .ok()?,
                )
            }
            _ => ContextEdit::AddRequirement {
                component: ids.fresh(),
                key,
            },
        }
    };
    let command = ContextCommand { edit };
    command.validate(model).ok()?;
    Some(crate::command::Command::EditContext(Box::new(command)))
}
