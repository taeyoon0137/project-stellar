# 작업 보고서: XII.7.7 자동 저장 stale 감지 시 중지와 알림 한 번

## WBS

- WBS: XII.7.7
- 작업명: 자동 저장 stale 감지 시 중지와 알림 한 번
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.7.1
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/autosave/schedule.rs`: `take_stale_alert`·`mark_stale`
- `crates/runtime/src/app/autosave.rs`: `take_stale_alerts`·`resolve_stale`, stale 다음 할 일 보관

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --locked` | 0 — lib 418 passed·1 ignored, 통합 테스트(text_session 8 포함) 전부 통과 |
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `autosave::schedule::tests::stale_alert_is_requested_once_per_stale_period_and_nothing_is_saved_before_a_choice` | 통과 |
| `app::autosave::pipeline_tests::stale_autosave_requests_one_alert_and_writes_nothing_before_a_choice` | 통과 — 반복 알림 없음, flush 포함 선택 전 write 없음 |
| `app::autosave::tests::stale_window_stops_autosaving_without_writing` | 통과(두 창을 모두 먼저 편집하도록 고침) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 알림은 셸 event가 아니라 코어 API(`take_stale_alerts`)로 꺼냅니다(FFI 추가 금지).

## 남은 위험

- clean 창은 다른 창 저장을 외부 변경으로 받으므로(ADR-0083 결정 3) 기존 stale 시험의 순서를 바꿨습니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.7.7)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `516ce4d9` Feat: 자동 저장 stale 알림을 stale 구간마다 한 번만 요청 (XII.7.7)
- `ba9dc68e` Feat: 앱 runtime에 stale 선택·외부 변경 감시 반영·닫기와 Git UI flush·trace 연결 (XII.7.7·UX.4.1·UX.4.4·XIII.1.6·XIV.2.8·XIII.1.7)
- `958506e3` Fix: 자동 저장 없는 runtime에서 stale 알림을 꺼내도 stale 상태를 유지
