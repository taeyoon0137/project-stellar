# 작업 보고서: XIX.5.2 proposed transaction이 lease 범위를 벗어나는 StableId를 수정하면 거부

- WBS: XIX.5.2
- 작업명: proposed transaction이 lease 범위를 벗어나는 StableId를 수정하면 거부
- 상태: 완료 (`[x]`)
- 선행: XIX.4.2, XIX.5.1
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs` — 에이전트 주체 판정(`Actor::Agent`).
- `crates/mcp-core/src/attached/edit.rs`(신규) — `proposal_transaction`, `apply_approved`.
- `crates/mcp-core/src/attached/edit_tests.rs`(신규).
- `crates/mcp-core/src/attached/mod.rs`(신규), `crates/mcp-core/src/lib.rs`.

## 구현 요약

- proposal은 XIX.5.1 해석기와 `decode_commands`로 닫힌 enum transaction이 되고, 출처는 lease의 편집 세션(`AgentProvenance`)입니다.
- 판정은 사용자 편집과 같은 `LeaseTable::judge`입니다. 바뀐 node는 존재하는 쪽(적용 전·후) 모두 자기 lease 안이어야 하고, 자기 lease root 삭제와 끝난 lease의 proposal은 거부합니다.
- `apply_approved`는 `execute_leased`로 적용하므로 범위 밖이면 문서와 revision이 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | 185 passed, 5 ignored(자식 프로세스 전용 1건 포함) |
| `cargo test -p stellar-mcp-core --locked` | 모든 test target 통과(lib 78 passed) |
| `cargo clippy -p stellar-commands -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base edfea9db` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |

| 검사 | 결과 |
|---|---|
| scope escape fixture 실패: 범위 밖 속성·조상·밖으로 이동·밖 node 끌어오기·밖에 추가·자기 root 삭제·다른 lease 안·없는 lease 거부, 범위 안 속성·추가·자손 삭제 허용(`agent_scope_escape_is_rejected`) | 통과 |
| attached 경로에서 범위 밖 proposal은 미리보기 판정 위반, 적용은 `out-of-scope`로 거부되고 문서·revision 불변, 범위 안은 적용(`scope_escape_fails_in_preview_and_apply`) | 통과 |

## 차이(작업 지시와 다른 점)

- attached proposal의 `base_revision`(저장 generation 기준)과 창 세션 revision의 대조는 앱 몫으로 남겼습니다. `set-property`·`insert-component-instance` 입력은 `decode_commands` 규칙대로 attached 경로에서 거부합니다.

## 남은 위험 / 한계

- 앱 runtime이 mcp-core에 닿을 수 없으므로 attached 적용 경로는 에이전트 편집 구현 crate로 옮겨야 앱에서 씁니다(ADR-0093 결정 12).

## 근거 커밋

- `537dbbd7`
- `f8ad5150`
