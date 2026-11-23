# 작업 보고서: XIX.6.3 user CancelAgentEdit는 rollback 없이 종료 절차, 중지 후 세션 되돌리기 미리보기

- WBS: XIX.6.3
- 작업명: user CancelAgentEdit는 rollback 없이 종료 절차, 중지 후 세션 되돌리기 미리보기
- 상태: 완료 (`[x]`)
- 선행: XIX.6.4, XIX.6.9
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/agent_history/mod.rs` — `cancel`, `cancel_and_preview_revert`.
- `crates/mcp-core/src/agent_history/tests.rs` — 중지 test.

## 구현 요약

- 사용자 중지는 `UserStopped` 종료 절차입니다. "중지하고 되돌리기"는 종료 절차를 마친 뒤 세션 단위 되돌리기 미리보기를 만들 뿐이고 문서를 바꾸지 않습니다. 사용자가 승인하면 일반 되돌리기 적용 경로를 씁니다. 별도 rollback 경로는 없습니다.

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
| 중지 fixture: 적용 batch 유지, lease 0, 중단 history entry 1(`cancel_keeps_batches_and_cancel_with_revert_opens_preview_without_change`) | 통과 |
| 중지하고 되돌리기: 미리보기 승인 전 revision 변화 0, 승인 후 비충돌 속성 baseline 일치(같은 test) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 메뉴·명령 연결은 XXIX.4.7입니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.3)` 커밋(브랜치 `lane/agent-history`).
