# 작업 보고서: XII.1.6 thread별 shutdown 순서 고정

## WBS

- WBS: XII.1.6
- 작업명: thread별 shutdown 순서 고정
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 앱 runtime(`AppRuntime`)은 render thread를 소유하지 않아 같은 고정 순서의 부분으로 종료합니다(ADR-0083 결정 6). 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.1.1\~XII.1.5
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/document_thread.rs`: `ShutdownStage`·`RuntimeThreads`·`ShutdownReport`
- `crates/runtime/src/document_thread/shutdown_tests.rs`(신규)
- `crates/runtime/src/app/autosave.rs`: 앱 runtime 종료 순서 문서

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
| `document_thread::shutdown_tests::shutdown_runs_the_fixed_stage_order` | 통과 — 단계 순서, drain 중 render 전달 성공, 공유 상태 해제 |
| `document_thread::shutdown_tests::command_drain_cancels_unstarted_jobs_after_the_timeout` | 통과 |
| `document_thread::shutdown_tests::thousand_lifecycles_have_no_deadlock_or_leak` | 통과 — 1,000 lifecycle, deadlock·누수 없음 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 앱 runtime(`AppRuntime`)은 render thread를 소유하지 않아 render 단계 없이 같은 순서를 따릅니다(ADR-0083 결정 6).

## 남은 위험

- 누수 검사는 공유 Arc 참조 수 기준입니다(OS thread 수 직접 계수 아님).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.1.6)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `b2461c33` Feat: runtime thread 종료 순서를 UI→command→render→worker→destroy로 고정 (XII.1.6)
