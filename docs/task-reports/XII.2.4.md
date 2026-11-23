# 작업 보고서: XII.2.4 같은 project path 여러 창의 독립 base generation·memory model

## WBS

- WBS: XII.2.4
- 작업명: 같은 project path 여러 창의 독립 base generation·memory model
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 선행 XII.2.2만 남긴 계획 개정(ADR-0006)으로 Windows 행(XII.2.3)은 선행이 아닙니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) — macOS 기준. Windows 판정은 ADR-0006을 따릅니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.2.2, XII.2.3 — XII.2.3(Windows)은 ADR-0006에 따라 빼고 macOS 기준으로 진행합니다(ADR-0083 결정 5).
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/window_session.rs`: 창 열기와 다시 읽기가 쓰는 `read_disk_document`(current generation 검증 + 작업 문서 로드)로 정리
- `crates/runtime/src/window_session/stale_tests.rs`(신규): 두 창 시나리오

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
| `window_session::tests::two_sessions_do_not_share_state` | 통과(기존, XII.2.1) |
| `window_session::stale_tests::second_window_keeps_its_base_and_its_save_is_stale` | 통과 — A의 편집·저장이 B 메모리 문서에 나타나지 않음, 두 창의 base generation이 따로 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 선행 XII.2.3(Windows)을 기다리지 않았습니다(ADR-0006, ADR-0083 결정 5).

## 남은 위험

- Windows 창 세션 판정은 하지 않았습니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.2.4)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `5e1edbf4` Task(XII.2.1): 창별 독립 문서와 이력 세션 선별 통합
- `75f2b026` Feat: 창 session의 stale 두 선택 실행과 외부 변경 command 반영 추가 (XII.7.1·XII.7.5·XII.7.6·UX.4.1)
