//! Context command와 binding orphan 종단 시험(XXX.7.8).

mod context_common;

use std::collections::BTreeMap;

use context_common::*;
use stellar_bindings::binding::BindingId;
use stellar_bindings::context::ContextBindingWorld;
use stellar_bindings::orphan::{BindingStore, BindingWorld, OrphanCause};
use stellar_commands::command::Command;
use stellar_commands::context::{ContextCommand, ContextEdit};
use stellar_commands::edit::EditContext;
use stellar_core_types::property::PropertyPath;
use stellar_document::context::fixtures::world::*;
use stellar_document::context::fixtures::*;
use stellar_document::context::resolve::ContextSite;
use stellar_document::context::validate::ContextConsumer;
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::types::TypeSpec;

struct Targets;

impl BindingWorld for Targets {
    fn target_type(&self, _: &PropertyPath) -> Option<TypeSpec> {
        Some(number_type())
    }
    fn reference_type(&self, _: &ReferenceTarget) -> Option<TypeSpec> {
        None
    }
}

#[test]
fn deleting_a_consumed_key_orphans_only_its_consumers_and_inverse_restores_resolution() {
    // 검증 기준(XXX.7.8): orphan·default fallback fixture 정확, apply/inverse.
    let (mut model, library) = sample_world();
    let consumers = vec![
        ContextConsumer {
            binding: sid(1),
            site: ContextSite::scene(sid(CONTENT)),
            expected: number_type(),
            source: context_read(1, sid(D)),
        },
        ContextConsumer {
            binding: sid(2),
            site: ContextSite::scene(sid(FRAME)),
            expected: number_type(),
            source: context_read(2, sid(N)),
        },
    ];
    let mut store = BindingStore::new(bindings(&consumers));
    let command = Command::EditContext(ContextCommand {
        edit: ContextEdit::DeleteKey { key: sid(D) },
    });
    let inverse = command.invert(&model).unwrap();
    {
        let mut ctx = EditContext::new(&mut model);
        command.apply(&mut ctx).unwrap();
        let _ = ctx.into_changes();
    }
    let types: BTreeMap<_, _> = model
        .context()
        .keys()
        .map(|key| (key.id(), key.value_type()))
        .collect();
    let orphaned = store.orphan_broken(&ContextBindingWorld {
        inner: &Targets,
        keys: &types,
    });
    assert_eq!(orphaned, vec![BindingId::new(sid(1))]);
    assert!(matches!(
        store.orphan(BindingId::new(sid(1))).unwrap().cause(),
        OrphanCause::SourceMissing { target: ReferenceTarget::Context(key), .. } if *key == sid(D)
    ));
    assert!(store.live().get(BindingId::new(sid(2))).is_some());
    // 역 command가 key·provision·공개 목록을 되살리면 해석도 원래대로입니다.
    {
        let mut ctx = EditContext::new(&mut model);
        inverse.apply(&mut ctx).unwrap();
        let _ = ctx.into_changes();
    }
    let graph = context_graph(&model, &library, &[], &consumers);
    let evaluator = evaluator(&graph, &consumers);
    assert_eq!(
        value(&evaluator, &consumers[0]),
        Some(stellar_core_types::typed_value::TypedValue::Number(
            stellar_core_types::value::FiniteF32::new(2.0).unwrap()
        ))
    );
}
