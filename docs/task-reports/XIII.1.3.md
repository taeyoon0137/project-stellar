# 작업 보고서: XIII.1.3 debounce 재설정·최대 지연 캡처와 document thread 짧은 캡처

## WBS

- WBS: XIII.1.3
- 작업명: debounce 재설정·최대 지연 캡처와 document thread 짧은 캡처
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.1.1, XII.1.1
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/autosave/schedule.rs`: ManualClock fixture 시험

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --locked` | 0 — lib 418 passed·1 ignored, 통합 테스트(text_session 8 포함) 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `autosave::schedule::tests::manual_clock_fixture_debounces_and_caps_with_the_max_delay` | 통과 — 60 ms 간격 command가 이어져도 최대 지연(1초)에 한 번 캡처, 멈추면 debounce 뒤 저장 |
| `app::autosave::tests::edits_are_saved_once_after_the_debounce` | 통과(기존) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 구현은 UX.2.11(ADR-0072 §2·§3)에 있었고 이번에 ManualClock fixture를 더했습니다.

## 남은 위험

- document thread 캡처 시간 실측은 ADR-0072 Performance 값을 따릅니다(이번에 다시 재지 않음).
