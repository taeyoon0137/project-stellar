# 작업 보고서: XXX.1.4 포인터 제스처 EventKind 확장

- WBS: XXX.1.4
- 작업명: hover, pointer down·up, drag(변위), scroll, long press EventKind와 release velocity 규칙.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 7, ADR-0025 결정 2.10
- 선행: IX.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/event.rs`: tag 0x09\~0x13 추가(기존 tag 불변).
- `crates/prototype/src/gesture.rs`(신규): 제스처 판정기와 velocity.
- `crates/prototype/src/lib.rs`: module 등록.

## 구현 요약

- ADR-0094 결정 7: 2D 값 타입 대신 축별 이벤트(`dragX`·`dragY`·`dragEndX`·`dragEndY`·`scrollX`·`scrollY`, Length payload). 같은 tick 안에서 x 다음 y.
- drag 시작 4px, long press 60 tick, velocity 창 4 tick을 registry 상수로 고정합니다. velocity는 tick별 변위의 f32 순차 합 / tick 수입니다.
- 탭은 `pointerUp` 다음 `activate`, drag 끝은 `dragEnd*` 다음 `pointerUp`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo xtask validate-adr --base edfea9db` | 통과. |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `component::event::tests::kind_golden_table`(기존 8개 불변), `gesture_kind_golden_table`(새 11개).
- `gesture::tests::same_log_same_velocity_bytes`: 같은 입력 로그의 velocity bits 동일.
- `drag_emits_axis_events_x_then_y_and_release_velocity`, `long_press_fires_once_without_activate`, `tap_activates`, `scroll_and_order_check`.
- `crates/prototype/tests/preview_tick_order.rs::drag_gesture_reaches_state_machine_as_axis_events`.

## 차이(작업 지시와 다른 점)

- `event.rs`는 작업 정의의 소유 파일 목록 밖이지만 행 정의가 EventKind 확장을 요구해 추가만 했습니다(exhaustive match가 이 파일 밖에 없음을 확인).

## 남은 위험 / 한계

- 셸의 InputEvent → 포인터 입력 변환(XXX.4.1)은 UI 재설계 때 연결합니다.
