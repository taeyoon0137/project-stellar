# 작업 보고서: XIX.6.10 출처 표시 상태 전이

- WBS: XIX.6.10
- 작업명: 출처 표시 상태 전이
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XIX.6.8이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 선행: XIX.6.8
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/provenance.rs` — `keep`, `observe`, `on_history`, `mark_reverted`, `UserUiAction`(XIX.6.7 커밋에 함께 들어감).
- `crates/mcp-core/src/provenance_tests.rs` — 상태 전이 test.

## 구현 요약

- "유지"는 문서·history를 바꾸지 않고 표시만 지웁니다. 다른 출처(사용자·다른 세션·외부)가 표시된 속성을 바꾸면 "이후 변경됨", 값이 적용 직후로 돌아오면 "검토 대기"입니다. 그룹 entry Undo는 "실행 취소됨", Redo는 이전 상태, 되돌리기 확정은 "되돌림", 그 Undo는 이전 상태, Redo는 다시 "되돌림"입니다.
- "유지"와 되돌리기 확정은 `UserUiAction` 표식을 받는 앱 사용자 UI 경로 전용이고, MCP catalog에는 이 색인을 바꾸는 tool이 없습니다.

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
| 유지 전후 revision·저장 bytes·history 동일(`keep_clears_display_without_touching_document_or_history`) | 통과 |
| 이후 변경·Undo/Redo·되돌리기 전이(`later_change_undo_redo_and_revert_transitions`) | 통과 |
| MCP catalog에 출처·되돌리기 쓰기 tool 0(`mcp_catalog_has_no_provenance_or_revert_write_path`) | 통과 |

## 차이(작업 지시와 다른 점)

- `UserUiAction`은 타입으로 호출자를 막지 못하는 문서화된 표식입니다. 쓰기 경로 전체 inventory는 XIX.9.7입니다.

## 남은 위험 / 한계

- 앱이 Undo·Redo 뒤 `on_history`·`observe`를 부르는 연결은 XXIX.3.1입니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.10)` 커밋(브랜치 `lane/agent-history`).
