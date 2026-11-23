# 작업 보고서: XIII.1.6 창 닫기·앱 비활성화 flush timeout과 창 닫기 결정 상태

## WBS

- WBS: XIII.1.6
- 작업명: 창 닫기·앱 비활성화 flush timeout과 창 닫기 결정 상태
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.1.5
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/autosave/flush.rs`(신규): `FlushOutcome`·`FlushBlock`
- `crates/runtime/src/app/autosave.rs`: `flush_for_close`·`flush_for_deactivate`

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
| `app::autosave::pipeline_tests::close_flush_returns_a_decision_state_and_never_discards_edits` | 통과 — 실패(`write`)·시간 초과 fixture, 편집 보존, 넘긴 flush는 끝까지 저장 |
| `app::autosave::pipeline_tests::close_flush_of_a_stale_window_returns_the_two_choices` | 통과 |
| `autosave::flush::tests::only_clean_and_saved_may_close_and_codes_are_frozen` | 통과 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 셸 닫기 watchdog과 값 합치기(ADR-0072 §1)는 셸 변경이라 하지 않았습니다. 코어는 호출자가 넘긴 제한 시간을 씁니다.

## 남은 위험

- `flush_for_deactivate`는 제한 시간 안에서 기다리는 API입니다. 기존 기다리지 않는 `flush_autosave`는 그대로입니다.
