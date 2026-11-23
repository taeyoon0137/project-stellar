# 작업 보고서: XII.7.2 stale 저장 두 선택지 core result

## WBS

- WBS: XII.7.2
- 작업명: stale 저장 두 선택지 core result
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.7.1
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/window_session.rs`: `StaleSaveChoice`(두 값, code·wire), `WriterProvenance`, `StaleResolution`

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
| `window_session::tests::stale_save_choices_are_exactly_two_with_frozen_codes` | 통과 — enum golden(code·wire 값 고정, 모르는 값 거부, 변형 두 개) |
| `window_session::tests::writer_provenance_decides_between_two_choices_and_reconciliation` | 통과 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- lane 결정 2에 따라 외부 writer와 겹친 stale은 두 선택지 대신 `ReconciliationPending`을 돌려줍니다. 두 선택지 enum 자체는 두 값뿐입니다.

## 남은 위험

- 셸 시트(UX.4.3)는 만들지 않았습니다(사용자 방향 2026-11-23).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.7.2)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `061db9bc` Feat: stale 저장 두 선택지 core enum과 writer 출처 판정 추가 (XII.7.2)
