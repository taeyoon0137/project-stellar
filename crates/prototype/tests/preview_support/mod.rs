//! 프리뷰 세션 시험 공용 helper.

#![allow(dead_code)]

use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::Trigger;
use stellar_prototype::edit::PrototypeDocument;
use stellar_prototype::gesture::PointerInput;
use stellar_prototype::session::{PreviewSession, SessionConfig, TickInput};

use super::support::*;

pub fn activate() -> TickInput {
    TickInput::Event {
        trigger: Trigger::Event {
            kind: EventKind::Activate,
            source: sid(ROOT),
        },
        payload: None,
    }
}

pub fn session(doc: &PrototypeDocument, config: SessionConfig) -> PreviewSession<'_> {
    PreviewSession::new(
        &doc.library,
        &doc.tokens,
        &doc.bindings,
        &doc.motions,
        toggle(),
        config,
    )
    .expect("세션")
}

pub fn length_of(value: Option<&Option<TypedValue>>) -> f32 {
    match value {
        Some(Some(TypedValue::Length(l))) => l.get(),
        other => panic!("길이 아님: {other:?}"),
    }
}

/// 입력 열(tick마다 입력 목록)을 돌려 trace bytes와 tick report를 얻습니다.
pub fn run(doc: &PrototypeDocument, script: &[Vec<TickInput>]) -> (Vec<u8>, Vec<usize>) {
    let mut s = session(doc, SessionConfig::default());
    let mut evaluations = Vec::new();
    for inputs in script {
        evaluations.push(s.advance(inputs.clone()).binding_evaluations);
    }
    (s.trace_bytes().to_vec(), evaluations)
}

pub fn script() -> Vec<Vec<TickInput>> {
    let mut script: Vec<Vec<TickInput>> = vec![Vec::new(); 40];
    script[1] = vec![activate()];
    // 진행 중에 다시 토글(현재 값에서 재지정).
    script[10] = vec![activate()];
    // 포인터 탭(제스처 판정 → activate).
    script[30] = vec![TickInput::Pointer {
        source: sid(ROOT),
        input: PointerInput::Down { x: 1.0, y: 1.0 },
    }];
    script[32] = vec![TickInput::Pointer {
        source: sid(ROOT),
        input: PointerInput::Up { x: 1.0, y: 1.0 },
    }];
    script
}
