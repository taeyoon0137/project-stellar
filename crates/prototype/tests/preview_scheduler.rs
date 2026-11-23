//! tick마다 영향받는 binding만 평가 (XXX.3.4).

mod preview_support;
mod support;

use preview_support::*;
use support::*;

#[test]
fn only_affected_bindings_are_evaluated() {
    // 검증 기준(XXX.3.4): 무관 binding 평가 count 0.
    let doc = document();
    let (_, evaluations) = run(&doc, &script());
    // 토글 tick: background·left 두 개만(knob width는 상태와 무관).
    assert_eq!(evaluations[1], 2);
    assert_eq!(evaluations[10], 2);
    // 입력이 없는 tick(진행 중인 motion 보간만): binding 평가 0.
    for (tick, count) in evaluations.iter().enumerate() {
        if ![1, 10, 32].contains(&tick) {
            assert_eq!(*count, 0, "tick {tick}");
        }
    }
    // 포인터 탭 → activate(뗀 tick).
    assert_eq!(evaluations[32], 2);
}
