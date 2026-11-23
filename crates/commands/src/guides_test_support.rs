#![cfg(any(test, feature = "test-support"))]
//! **테스트 전용** 가이드·View 변수 편집 command 생성기(ADR-0126, VII.6.1). property test·fuzz가 [`crate::test_support`]를
//! 거쳐 씁니다. 프로덕션 경로는 이 모듈을 호출하지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::guides::{
    GridAlign, GridKind, GuideAxis, GuideEdit, LayoutGrid, RulerGuide, TrackSize,
};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_document::test_support::random_document::SplitMix64;
use stellar_document::view_variant::commands::ViewVariantCommand;
use stellar_document::view_variant::condition::VariantCondition;
use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};
use stellar_document::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};

use crate::command::Command;
use crate::guides::{EditViewVariants, SetNodeGuides};
use crate::test_support::FreshIds;

fn pick<T: Copy>(rng: &mut SplitMix64, items: &[T]) -> Option<T> {
    (!items.is_empty()).then(|| items[rng.below(items.len() as u64) as usize])
}

/// 무작위 Frame 하나의 가이드 편집(추가·이동·삭제·그리드 교체 가운데 하나).
pub(crate) fn random_guides_edit(
    model: &DocumentModel,
    rng: &mut SplitMix64,
    ids: &mut FreshIds,
) -> Option<Command> {
    let frames: Vec<StableId> = model
        .to_records()
        .ok()?
        .nodes
        .iter()
        .filter(|n| matches!(n.payload, NodePayload::Frame(_)))
        .map(|n| n.id)
        .collect();
    let owner = pick(rng, &frames)?;
    let existing: Vec<StableId> = model
        .node_guides(owner)
        .map(|r| r.guides().map(|g| g.id).collect())
        .unwrap_or_default();
    let position = rng.below(2_000) as f32 / 4.0 - 100.0;
    let edit = match (rng.below(4), pick(rng, &existing)) {
        (1, Some(id)) => GuideEdit::Move { id, position },
        (2, Some(id)) => GuideEdit::Remove { id },
        (3, _) => {
            let kinds = [GridKind::Columns, GridKind::Rows, GridKind::Square];
            let kind = kinds[rng.below(3) as usize];
            let grid = LayoutGrid {
                kind,
                count: 1 + rng.below(12) as u32,
                size: if kind == GridKind::Square || rng.below(2) == 0 {
                    TrackSize::Px(1.0 + rng.below(64) as f32)
                } else {
                    TrackSize::Auto
                },
                gap: rng.below(40) as f32,
                margin: rng.below(40) as f32,
                align: [
                    GridAlign::Stretch,
                    GridAlign::Start,
                    GridAlign::Center,
                    GridAlign::End,
                ][rng.below(4) as usize],
                visible: rng.below(3) != 0,
            }
            .validated()
            .ok()?;
            GuideEdit::SetGrids((0..rng.below(3)).map(|_| grid).collect())
        }
        _ => GuideEdit::Add(
            RulerGuide::new(
                ids.fresh(),
                if rng.below(2) == 0 {
                    GuideAxis::X
                } else {
                    GuideAxis::Y
                },
                position,
            )
            .ok()?,
        ),
    };
    let command = SetNodeGuides { owner, edit };
    command.validate(model).ok()?;
    Some(Command::SetNodeGuides(command))
}

/// 무작위 View 하나의 변수·Variant 편집(Boolean 변수 생성·삭제, Variant 생성·삭제).
pub(crate) fn random_view_variants_edit(
    model: &DocumentModel,
    rng: &mut SplitMix64,
    ids: &mut FreshIds,
) -> Option<Command> {
    let views: Vec<(StableId, StableId)> = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| p.views.iter().map(|v| (v.id, v.root_frame)))
        .collect();
    let (view, root) = pick(rng, &views)?;
    let current = model.view_variants(view);
    let variables: Vec<ViewVariableId> = current
        .map(|m| m.variables().iter().map(|v| v.id()).collect())
        .unwrap_or_default();
    let variants: Vec<ViewVariantId> = current
        .map(|m| m.variants().map(|v| v.id).collect())
        .unwrap_or_default();
    let command = match (rng.below(4), pick(rng, &variables), pick(rng, &variants)) {
        (1, Some(variable), _) => {
            let mut overrides = VariantOverrideSet::new();
            overrides.set_property(
                PropertyPath::with_keys(root, vec![PropertyKey(0x0110)]),
                TypedValue::Boolean(rng.below(2) == 0),
            );
            ViewVariantCommand::CreateVariant(ViewVariant::new(
                ViewVariantId::new(ids.fresh()),
                VariantCondition::new()
                    .equals(variable, VariableValue::Value(TypedValue::Boolean(true))),
                overrides,
            ))
        }
        (2, Some(variable), _) => ViewVariantCommand::DeleteVariable(variable),
        (3, _, Some(variant)) => ViewVariantCommand::DeleteVariant(variant),
        _ => ViewVariantCommand::CreateVariable(
            ViewVariable::new(
                ViewVariableId::new(ids.fresh()),
                format!("flag{}", rng.below(1_000_000)),
                TypeSpec::required(ExpressionType::Boolean),
                VariableValue::Value(TypedValue::Boolean(false)),
                [],
            )
            .ok()?,
        ),
    };
    let command = EditViewVariants { view, command };
    command.validate(model).ok()?;
    Some(Command::EditViewVariants(Box::new(command)))
}
