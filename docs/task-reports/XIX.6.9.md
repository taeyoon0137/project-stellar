# 작업 보고서: XIX.6.9 세션 단위 되돌리기

- WBS: XIX.6.9
- 작업명: 세션 단위 되돌리기
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XIX.6.8이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 선행: XIX.6.8
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_revert_tests.rs` — 세션 test(XIX.6.8 커밋에 함께 들어감).
- `crates/history/src/agent_tests.rs` — 중간 실패 test.
- `crates/mcp-core/src/attached/revert_tests.rs` — 10 batch 세션 되돌리기·실패 test.

## 구현 요약

- 한 세션의 남은(표시 중) batch를 역순으로 `plan_revert`에 넣어 하나의 User 출처 transaction으로 합칩니다. 앞 batch의 충돌은 뒤 batch를 되돌린 사본 상태 기준으로 판정합니다. 적용은 transaction 하나라 전부 성공하거나 전부 미적용이고 history entry 하나입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

브랜치 `lane/agent-history` 최종 상태에서 실행했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | lib 205 passed, 5 ignored; 통합 test 3 passed |
| `cargo test -p stellar-history --locked` | lib 55 passed |
| `cargo test -p stellar-mcp-core --locked` | lib 101 passed |
| `cargo clippy -p stellar-commands -p stellar-history -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |
| `cargo xtask validate-crate-graph` | 0 |

| 검사 | 결과 |
|---|---|
| 10 batch 세션 되돌리기 후 비충돌 속성 baseline 일치, history entry 1, 충돌 목록 정확(`session_revert_of_ten_batches_is_one_entry_with_exact_conflicts`, `session_revert_of_ten_batches_restores_baseline_in_one_transaction`) | 통과 |
| 중간 실패 fixture 효과 0(`session_revert_failing_in_the_middle_has_no_effect`), 낡은 미리보기·history 거부 효과 0(`failure_at_apply_time_has_no_effect`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- "남은 batch"에는 유지·실행 취소·되돌림 batch를 넣지 않습니다(ADR-0100 결정 7).

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.9)` 커밋(브랜치 `lane/agent-history`).
