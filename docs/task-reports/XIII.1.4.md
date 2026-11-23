# 작업 보고서: XIII.1.4 worker 직렬화·쓰기와 오래된 base 결과 폐기

## WBS

- WBS: XIII.1.4
- 작업명: worker 직렬화·쓰기와 오래된 base 결과 폐기
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.1.4, V.5.2
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/window_session.rs`: `write_timed`(직렬화·쓰기 시간)
- `crates/runtime/src/window_session/stale_tests.rs`

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `window_session::stale_tests::worker_write_on_an_outdated_base_is_discarded_without_switching_the_pointer` | 통과 — 다른 thread의 쓰기 결과가 오래된 base면 stale, current pointer 불변 |
| `app::autosave::tests::save_now_waits_for_the_autosave_in_flight` | 통과(기존) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 구현은 UX.2.11(ADR-0072 §3)에 있었고 이번에 stale 결과 폐기 시험을 더했습니다.

## 남은 위험

- 없음.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIII.1.4)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `3405128a` Fix: 자동 저장 쓰기를 worker로 옮기고 상태를 셸에 노출
- `d93d536e` Test: 자동 저장 ManualClock fixture와 오래된 base 쓰기 결과 폐기 시험 추가 (XIII.1.3·XIII.1.4)
