# 작업 보고서: XIII.1.5 Save Now의 대기 autosave 취소·합치기

## WBS

- WBS: XIII.1.5
- 작업명: Save Now의 대기 autosave 취소·합치기
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.1.4
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- 코드 변경 없음(ADR-0072 §4 구현). 이 보고서는 검증 기록입니다.

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
| `app::autosave::tests::save_now_merges_the_pending_autosave` | 통과 — Save Now 뒤 이중 쓰기 없음 |
| `app::autosave::tests::save_now_waits_for_the_autosave_in_flight` | 통과 — 진행 중 쓰기를 기다린 뒤 남은 편집만 저장 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- 없음.
