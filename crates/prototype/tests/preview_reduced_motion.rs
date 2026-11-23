//! reduced motion 대체 동작 (XXX.3.5).

mod preview_support;
mod support;

use preview_support::*;
use stellar_document::motion::{MotionSet, ReducedMotion};
use stellar_prototype::session::SessionConfig;
use support::*;

#[test]
fn reduced_motion_uses_fallback() {
    // 검증 기준(XXX.3.5): 환경 on/off fixture 결과 정확.
    let doc = document();
    let mut off = session(&doc, SessionConfig::default());
    off.advance(vec![activate()]);
    assert_eq!(length_of(off.displayed(&left(KNOB))), 0.0, "off: 보간 시작");
    let mut on = session(
        &doc,
        SessionConfig {
            reduced_motion: true,
            ..SessionConfig::default()
        },
    );
    let report = on.advance(vec![activate()]);
    assert_eq!(
        length_of(on.displayed(&left(KNOB))),
        24.0,
        "on + instant: 즉시 끝값"
    );
    assert_eq!(report.active_motions, 0);
    // 대체 동작이 play면 reduced motion에서도 재생합니다.
    let mut play_doc = document();
    let mut motions = MotionSet::new();
    motions
        .insert(motion(200.0, ReducedMotion::Play))
        .expect("motion");
    play_doc.motions = motions;
    let mut play = session(
        &play_doc,
        SessionConfig {
            reduced_motion: true,
            ..SessionConfig::default()
        },
    );
    let report = play.advance(vec![activate()]);
    assert_eq!(report.active_motions, 2);
    // 세션 중 켜기.
    let mut toggled = session(&doc, SessionConfig::default());
    toggled.set_reduced_motion(true);
    toggled.advance(vec![activate()]);
    assert_eq!(length_of(toggled.displayed(&left(KNOB))), 24.0);
}

#[test]
fn turning_reduced_motion_on_finishes_running_instant_motion() {
    // 검증 기준(XXX.3.5): 세션 중 켜면 진행 중인 `instant` motion은 끝값으로 바로 바뀝니다.
    let doc = document();
    let mut s = session(&doc, SessionConfig::default());
    s.advance(vec![activate()]);
    for _ in 0..6 {
        s.advance(Vec::new());
    }
    let mid = length_of(s.displayed(&left(KNOB)));
    assert!(mid > 0.0 && mid < 24.0, "{mid}");
    s.set_reduced_motion(true);
    assert_eq!(length_of(s.displayed(&left(KNOB))), 24.0);
    assert_eq!(s.advance(Vec::new()).active_motions, 0);
}
