//! tick 처리 순서·결정론·펄스 상한·제스처 (XXX.3.3, XXX.3.1, XXX.1.4).

mod preview_support;
mod support;

use preview_support::*;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::{SemanticActionId, Trigger};
use stellar_document::component::state_store::InstanceKey;
use stellar_prototype::gesture::PointerInput;
use stellar_prototype::session::{Pulse, PulseRouter, SessionConfig, SessionDiagnostic, TickInput};
use support::*;

#[test]
fn same_event_log_and_ticks_give_byte_identical_trace() {
    // 검증 기준(XXX.3.3): 같은 이벤트 로그·tick 열의 값 trace byte-identical. (XXX.3.1: 같은 tick 열 같은 결과)
    let doc = document();
    let (a, _) = run(&doc, &script());
    let (b, _) = run(&document(), &script());
    assert!(!a.is_empty());
    assert_eq!(a, b);
}

#[test]
fn motion_interpolates_and_retargets_from_current_value() {
    let doc = document();
    let mut s = session(&doc, SessionConfig::default());
    assert_eq!(length_of(s.displayed(&left(KNOB))), 0.0);
    let report = s.advance(vec![activate()]);
    assert_eq!(report.state_changes.len(), 1);
    assert_eq!(report.motions_started.len(), 2);
    // 시작 tick은 시작값.
    assert_eq!(length_of(s.displayed(&left(KNOB))), 0.0);
    for _ in 0..6 {
        s.advance(Vec::new());
    }
    let mid = length_of(s.displayed(&left(KNOB)));
    assert!(mid > 0.0 && mid < 24.0, "{mid}");
    // 되돌리기 토글: 지금 값에서 0으로 재지정합니다.
    let report = s.advance(vec![activate()]);
    assert_eq!(report.motions_started.len(), 2);
    let restart = length_of(s.displayed(&left(KNOB)));
    assert!(restart >= mid, "재지정 시작값 {restart} ≥ {mid}");
    for _ in 0..40 {
        s.advance(Vec::new());
    }
    assert_eq!(length_of(s.displayed(&left(KNOB))), 0.0);
    assert_eq!(s.advance(Vec::new()).active_motions, 0);
}

struct Echo;

impl PulseRouter for Echo {
    fn route(&self, instance: &InstanceKey, _action: SemanticActionId) -> Vec<Pulse> {
        vec![Pulse {
            instance: instance.clone(),
            trigger: Trigger::Event {
                kind: EventKind::Activate,
                source: sid(ROOT),
            },
            payload: None,
        }]
    }
}

#[test]
fn infinite_pulse_chain_hits_limit_and_discards_state() {
    // 검증 기준(XXX.3.3): 무한 펄스 fixture 상한 오류·상태 변화 0.
    let doc = document();
    let mut s = session(
        &doc,
        SessionConfig {
            max_dispatch_depth: 8,
            ..SessionConfig::default()
        },
    );
    s.set_router(Box::new(Echo));
    let before = s.state(&s.root().clone()).cloned();
    let report = s.advance(vec![activate()]);
    assert!(report
        .diagnostics
        .contains(&SessionDiagnostic::PulseDepthExceeded { limit: 8 }));
    assert!(report.state_changes.is_empty());
    assert_eq!(s.state(&s.root().clone()).cloned(), before);
    assert_eq!(report.binding_evaluations, 0);
    assert!(report.motions_started.is_empty());
}

#[test]
fn drag_gesture_reaches_state_machine_as_axis_events() {
    // XXX.1.4: drag 축별 이벤트가 상태 머신 dispatch까지 갑니다(반응하는 transition이 없어도 dispatch 수로 확인).
    let doc = document();
    let mut s = session(&doc, SessionConfig::default());
    s.advance(vec![TickInput::Pointer {
        source: sid(KNOB),
        input: PointerInput::Down { x: 0.0, y: 0.0 },
    }]);
    let report = s.advance(vec![TickInput::Pointer {
        source: sid(KNOB),
        input: PointerInput::Move { x: 10.0, y: 0.0 },
    }]);
    assert_eq!(report.dispatched, 2, "dragX, dragY");
    let report = s.advance(vec![TickInput::Pointer {
        source: sid(KNOB),
        input: PointerInput::Up { x: 20.0, y: 0.0 },
    }]);
    assert_eq!(report.dispatched, 3, "dragEndX, dragEndY, pointerUp");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}
