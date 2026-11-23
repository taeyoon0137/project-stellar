//! 가이드·View 변수 command test (ADR-0126).

use super::*;
use crate::codec::{decode_command, encode_command};
use crate::command::Command;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::frame::FramePayload;
use stellar_document::guides::{GuideAxis, LayoutGrid, RulerGuide};
use stellar_document::records::NodeRecord;
use stellar_document::view_variant::condition::VariantCondition;
use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};
use stellar_document::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0c, n)
}

fn document() -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), "doc".into());
    let mut page = PageDefinition::new(sid(2), "page".into());
    page.add_view(ViewDefinition::new(sid(3), "view".into(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    let node = |id: u64, parent: Option<u64>, key: u8, payload| NodeRecord {
        id: sid(id),
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{id}"),
        payload,
    };
    model
        .insert_node(node(
            10,
            None,
            0x40,
            NodePayload::Frame(FramePayload::default()),
        ))
        .unwrap();
    model
        .insert_node(node(
            11,
            Some(10),
            0x40,
            NodePayload::Fragment(Default::default()),
        ))
        .unwrap();
    model
}

fn run(model: &mut DocumentModel, command: &Command) -> Command {
    command.validate(model).unwrap();
    let inverse = command.invert(model).unwrap();
    let mut ctx = EditContext::new(model);
    command.apply(&mut ctx).unwrap();
    ctx.commit();
    inverse
}

fn flag(id: u64) -> ViewVariable {
    ViewVariable::new(
        ViewVariableId::new(sid(id)),
        format!("flag{id}"),
        TypeSpec::required(ExpressionType::Boolean),
        VariableValue::Value(TypedValue::Boolean(false)),
        [],
    )
    .unwrap()
}

#[test]
fn guide_gestures_are_one_history_item_each_and_undo_exactly() {
    // 검증 기준(XII.10.19): 가이드 생성·이동·삭제 fixture. 편집 하나가 command 하나이고 역 command가 정확히 되돌립니다.
    let mut model = document();
    let guide = RulerGuide::new(sid(100), GuideAxis::X, 120.0).unwrap();
    let steps = [
        GuideEdit::Add(guide),
        GuideEdit::Move {
            id: sid(100),
            position: 64.5,
        },
        GuideEdit::SetGrids(vec![LayoutGrid::columns(12, 20.0, 32.0).unwrap()]),
        GuideEdit::Remove { id: sid(100) },
    ];
    let mut snapshots = vec![model.to_records().unwrap()];
    let mut inverses = Vec::new();
    for edit in steps {
        let command = Command::SetNodeGuides(SetNodeGuides {
            owner: sid(10),
            edit,
        });
        // command bytes 왕복.
        assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
        inverses.push(run(&mut model, &command));
        snapshots.push(model.to_records().unwrap());
    }
    let last = model.node_guides(sid(10)).unwrap();
    assert_eq!(last.guides().count(), 0);
    assert_eq!(last.grids().len(), 1);
    for (index, inverse) in inverses.iter().enumerate().rev() {
        run(&mut model, inverse);
        assert_eq!(
            model.to_records().unwrap(),
            snapshots[index],
            "단계 {index}"
        );
    }
    assert_eq!(model.node_guides(sid(10)), None);
}

#[test]
fn guides_are_rejected_on_non_frames_and_bad_values() {
    let model = document();
    let on_fragment = Command::SetNodeGuides(SetNodeGuides {
        owner: sid(11),
        edit: GuideEdit::Add(RulerGuide::new(sid(100), GuideAxis::Y, 1.0).unwrap()),
    });
    assert!(on_fragment.validate(&model).is_err());
    let missing = Command::SetNodeGuides(SetNodeGuides {
        owner: sid(10),
        edit: GuideEdit::Remove { id: sid(100) },
    });
    assert!(missing.validate(&model).is_err());
    let infinite = Command::SetNodeGuides(SetNodeGuides {
        owner: sid(10),
        edit: GuideEdit::SetGrids(vec![LayoutGrid {
            gap: f32::INFINITY,
            ..LayoutGrid::columns(4, 0.0, 0.0).unwrap()
        }]),
    });
    assert!(infinite.validate(&model).is_err());
}

#[test]
fn rollback_restores_the_previous_guides_and_view_variants() {
    let mut model = document();
    run(
        &mut model,
        &Command::SetNodeGuides(SetNodeGuides {
            owner: sid(10),
            edit: GuideEdit::Add(RulerGuide::new(sid(100), GuideAxis::X, 4.0).unwrap()),
        }),
    );
    let before = model.to_records().unwrap();
    let mut ctx = EditContext::new(&mut model);
    Command::SetNodeGuides(SetNodeGuides {
        owner: sid(10),
        edit: GuideEdit::Remove { id: sid(100) },
    })
    .apply(&mut ctx)
    .unwrap();
    Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(3),
        command: ViewVariantCommand::CreateVariable(flag(200)),
    }))
    .apply(&mut ctx)
    .unwrap();
    ctx.rollback().unwrap();
    assert_eq!(model.to_records().unwrap(), before);
}

#[test]
fn view_variant_commands_persist_in_records_and_undo_exactly() {
    // 검증 기준(XVI.11.1): Variant 생성이 문서 레코드에 남고 undo가 되돌립니다.
    let mut model = document();
    let create_variable = Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(3),
        command: ViewVariantCommand::CreateVariable(flag(200)),
    }));
    assert_eq!(
        decode_command(&encode_command(&create_variable)).unwrap(),
        create_variable
    );
    let undo_variable = run(&mut model, &create_variable);
    let variant = ViewVariant::new(
        ViewVariantId::new(sid(300)),
        VariantCondition::new().equals(
            ViewVariableId::new(sid(200)),
            VariableValue::Value(TypedValue::Boolean(true)),
        ),
        Default::default(),
    );
    let create_variant = Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(3),
        command: ViewVariantCommand::CreateVariant(variant),
    }));
    assert_eq!(
        decode_command(&encode_command(&create_variant)).unwrap(),
        create_variant
    );
    let undo_variant = run(&mut model, &create_variant);
    let records = model.to_records().unwrap();
    assert_eq!(records.view_variants.len(), 1);
    assert_eq!(records.view_variants[0].variants().count(), 1);
    // 다른 영속 id와 겹치는 새 id는 거부합니다.
    let clash = Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(3),
        command: ViewVariantCommand::CreateVariable(flag(10)),
    }));
    assert!(clash.validate(&model).is_err());
    // 변수·Variant끼리도 겹치면 거부합니다: 이미 있는 변수 id로 Variant를 만들면 semantic snapshot의 Variant 객체 id가
    // 문서 안에서 유일하지 않게 됩니다(ADR-0126 결정 1.4).
    let variant_as_variable = Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(3),
        command: ViewVariantCommand::CreateVariant(ViewVariant::new(
            ViewVariantId::new(sid(200)),
            VariantCondition::new(),
            Default::default(),
        )),
    }));
    assert!(matches!(
        variant_as_variable.validate(&model),
        Err(CommandError::DuplicateId { id }) if id == sid(200)
    ));
    // 없는 View.
    let no_view = Command::EditViewVariants(Box::new(EditViewVariants {
        view: sid(4),
        command: ViewVariantCommand::CreateVariable(flag(201)),
    }));
    assert!(no_view.validate(&model).is_err());
    run(&mut model, &undo_variant);
    run(&mut model, &undo_variable);
    assert!(model.to_records().unwrap().view_variants.is_empty());
}

#[test]
fn external_snapshot_applies_and_undoes_guide_and_view_variant_changes() {
    // 외부 원본(텍스트 편집·Git checkout)의 가이드·View 변수 변경도 외부 반영 command 하나로 들어오고 undo가 되돌립니다.
    let mut model = document();
    run(
        &mut model,
        &Command::SetNodeGuides(SetNodeGuides {
            owner: sid(10),
            edit: GuideEdit::Add(RulerGuide::new(sid(100), GuideAxis::X, 4.0).unwrap()),
        }),
    );
    let before = model.to_records().unwrap();
    let mut incoming_model = DocumentModel::from_records(before.clone()).unwrap().0;
    run(
        &mut incoming_model,
        &Command::SetNodeGuides(SetNodeGuides {
            owner: sid(10),
            edit: GuideEdit::Move {
                id: sid(100),
                position: 8.0,
            },
        }),
    );
    run(
        &mut incoming_model,
        &Command::EditViewVariants(Box::new(EditViewVariants {
            view: sid(3),
            command: ViewVariantCommand::CreateVariable(flag(200)),
        })),
    );
    let incoming = incoming_model.to_records().unwrap();
    let external =
        crate::snapshot_apply::ApplyExternalSnapshot::between(&model, &incoming).unwrap();
    assert_eq!(external.changes.len(), 2);
    let command = Command::ApplyExternalSnapshot(external);
    assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
    let undo = run(&mut model, &command);
    assert_eq!(model.to_records().unwrap(), incoming);
    run(&mut model, &undo);
    assert_eq!(model.to_records().unwrap(), before);
}
