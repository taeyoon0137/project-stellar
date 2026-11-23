# 작업 보고서: XXX.3.1 고정 tick 논리 시계 PreviewClock

- WBS: XXX.3.1
- 작업명: 프리뷰 세션 고정 tick 논리 시계, 평가 경로 시스템 시각 접근 금지.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 6.1
- 선행: IX.5.4, XVI.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/{Cargo.toml,src/lib.rs,src/clock.rs}`(신규): 새 crate `stellar-prototype`과 논리 시계.
- `Cargo.toml`, `Cargo.lock`: workspace member 등록.
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`: crate graph 정책·표에 행 추가.

## 구현 요약

- 120 tick/s 고정. tick→ms는 f64 사칙연산, 디스플레이 프레임→tick은 정수 연산입니다. 시계는 시스템 시각을 읽지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo test -p xtask crate_graph` | 통과(crate graph 정책에 `stellar-prototype` 등록). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `clock::tests::clock_is_monotonic_and_pure`, `frames_map_to_ticks`.
- `crates/prototype/tests/inventory.rs::engine_has_no_system_time_file_or_network_access`(XXX.6.3 commit): 평가 경로 inventory 0.
- `preview_tick_order.rs::same_event_log_and_ticks_give_byte_identical_trace`(XXX.3.3 commit): 같은 tick 열 같은 결과.

## 차이(작업 지시와 다른 점)

- 선행 XVI.2.1(상호작용 trace)은 아직 없습니다. 셸의 프레임→tick 대응 함수만 두고 trace 연결은 하지 않았습니다.
- crate graph 정책 파일은 소유 밖이지만 새 crate 등록에 필요한 한 행만 더했습니다.

## 남은 위험 / 한계

- 셸 표시 루프 연결은 UI 재설계(XXX.4.1) 몫입니다.
