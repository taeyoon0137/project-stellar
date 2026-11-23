# 작업 보고서: XXX.3.3 tick 처리 순서

- WBS: XXX.3.3
- 작업명: 입력 이벤트 → 펄스 dispatch(원자 commit) → MotionSpec 시작·재지정 → 영향받는 binding 재평가, 펄스 깊이 상한.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 6.3·6.4
- 선행: XXX.3.1, XXX.3.2, XXX.1.4, IX.5.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/session.rs`(신규).
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/preview_support/mod.rs`, `preview_tick_order.rs`(신규).

## 구현 요약

- `PreviewSession::advance`가 ADR-0094 결정 6.3 순서를 지킵니다. dispatch는 작업 사본에 하고 깊이 상한을 넘으면 그 tick의 상태 변화를 모두 버립니다.
- MotionSpec 시작값은 지금 표시 값(진행 중이면 현재 보간 값), 끝값은 재평가한 binding 값입니다. `PulseRouter`(기본: nested → 부모 `NestedAction`)가 연쇄를 잇습니다.
- tick마다 표시 값과 루트 상태를 trace bytes에 기록합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `preview_tick_order.rs::same_event_log_and_ticks_give_byte_identical_trace`: 같은 입력·tick 열 trace bytes 동일.
- `infinite_pulse_chain_hits_limit_and_discards_state`: echo router 무한 연쇄 → `PulseDepthExceeded`, 상태 변화 0, binding 평가 0.
- `motion_interpolates_and_retargets_from_current_value`, `drag_gesture_reaches_state_machine_as_axis_events`.

## 차이(작업 지시와 다른 점)

- 끝값은 binding 재평가 뒤에 정하므로 MotionSpec 시작(시작값 캡처)과 끝값 결정이 재평가 앞뒤로 나뉩니다. 계획서 순서와 같습니다(ADR-0094 결정 6.3).

## 남은 위험 / 한계

- 프리뷰 binding 평가는 루트 인스턴스 정의의 binding만 다룹니다. nested instance 속성 평가는 후속입니다.
