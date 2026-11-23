# 작업 보고서: XIII.8.7 정상 종료 후 복구 snapshot 삭제와 stale 무시

## WBS

- WBS: XIII.8.7
- 작업명: 정상 종료 후 복구 snapshot 삭제와 stale 무시
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XIII.8.3(`[ ]`)이 `[x]`가 아닙니다. XIII.8.3이 끝나면 근거 커밋 기록만으로 `[x]`입니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.9.4, XIII.8.3
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/app/recovery/lifecycle_tests.rs`(신규)

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
| `app::recovery::lifecycle_tests::normal_save_and_close_remove_the_checkpoint_and_offer_no_prompt` | 통과 — 저장·닫기 뒤 복구본 0, 후보 없음, 낡은 기준 복구본은 다음 시작에서 제안 안 함 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- 없음.
