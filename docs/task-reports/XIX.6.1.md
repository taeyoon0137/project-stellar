# 작업 보고서: XIX.6.1 승인 batch마다 inverse transaction 보존(full document clone 금지)

- WBS: XIX.6.1
- 작업명: 승인 batch마다 inverse transaction 보존(full document clone 금지)
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XIX.5.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 선행: XIX.5.5
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_batch.rs`(신규) — `AgentBatchRecord`(정·역 transaction, change set, 생성·삭제 node, 지문), `fingerprint`, `fingerprint_keys`.
- `crates/commands/src/agent_batch_tests.rs`(신규).
- `crates/commands/src/lib.rs` — module 등록.
- `crates/mcp-core/src/agent_history/tests.rs` — 1000 command batch semantic snapshot 복원 test(XIX.6.2 커밋에 들어감).

## 구현 요약

- 승인 batch를 적용한 결과(`AppliedTransaction`)에서 역 transaction과 change set을 그대로 보존합니다. 문서 사본(`DocumentModel`·`DocumentRecords`)을 담는 필드가 없어 기록 크기는 batch 크기에 비례합니다.
- 같은 기록에 StableId+PropertyPath별 적용 직후 값 지문을 둡니다(되돌리기 충돌 판정 입력, ADR-0100 결정 1).

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
| 1000 command batch 역 transaction 적용 후 문서 레코드가 baseline과 같음(`capture_keeps_inverse_without_document_copy_and_inverse_restores_records`) | 통과 |
| attached 경로 1000 command batch 역 적용 후 semantic diff 0(`thousand_command_batch_inverse_restores_baseline_semantic_snapshot`) | 통과 |
| 속성 key·생성·삭제 node 레코드 key와 지문(`property_changes_are_keys_with_post_apply_fingerprints`, `created_and_deleted_nodes_have_record_keys`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 등록되지 않은 속성(layout longhand 등)과 구조만 바뀐 node는 node 레코드 전체 지문이라 같은 node의 다른 속성 변경도 충돌로 봅니다(보수적).
- 큰 batch의 지문 계산 비용은 재지 않았습니다(change set 크기에 비례).

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.1)` 커밋(브랜치 `lane/agent-history`).
