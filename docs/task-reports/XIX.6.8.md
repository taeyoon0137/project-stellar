# 작업 보고서: XIX.6.8 변경 단위 되돌리기

- WBS: XIX.6.8
- 작업명: 변경 단위 되돌리기
- 상태: 부분 완료 (`[~]`)
- 선행: XIX.6.7, XIX.6.1, XIX.4.3, XIX.5.3
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_revert.rs`(신규) — `plan_revert`, `RevertPlan`, `RevertConflict`, `ConflictKind`, `RevertItem`. `crates/commands/src/agent_revert_tests.rs`(신규).
- `crates/history/src/agent.rs` — `apply_revert`(`execute_leased` + "에이전트 변경 되돌리기" entry).
- `crates/mcp-core/src/attached/revert.rs`(신규) — `preview_revert`, `apply_revert`. `crates/mcp-core/src/attached/revert_tests.rs`(신규). `crates/mcp-core/src/attached/mod.rs`, `crates/commands/src/lib.rs`.

## 구현 요약

- 고른 batch 하나의 역 command를 문서 사본에서 하나씩 적용해, StableId+PropertyPath별 현재 값이 적용 직후 지문과 다른 key(이후 변경, 삭제된 대상, 에이전트가 만든 node 아래 이후 자식, batch 밖 참조)를 충돌로 분류하고 기본 제외합니다. 사용자가 고른 충돌만 복원합니다.
- 결과는 새 User 출처 transaction 하나이고, 미리보기는 XIX.5.3 경로(semantic diff·validation·lease 판정)입니다. 적용은 `execute_leased`라 다른 활성 lease와 겹치면 거부하고, "에이전트 변경 되돌리기" history entry 하나로 기록합니다(Undo 가능).

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
| 비충돌 batch 되돌리기 후 baseline 일치, 계획 전후 revision 동일(`non_conflicting_batch_revert_restores_baseline_without_touching_state`) | 통과 |
| 이후 사용자 편집 속성 기본 보존, 고르면 복원(`later_user_edit_is_conflict_kept_by_default_and_restored_when_chosen`, `change_revert_restores_non_conflicting_and_keeps_later_user_edit`) | 통과 |
| 구조 충돌 기본 현재 유지, 삭제 대상 충돌(`created_node_with_later_child_is_structural_conflict_kept`, `deleted_target_node_is_node_deleted_conflict`) | 통과 |
| 되돌리기 Undo 후 재적용 일치(`revert_entry_is_undoable_and_reapplies_agent_change`) | 통과 |
| 미리보기 전후 revision 동일, 활성 lease 겹침 거부(`revert_overlapping_active_lease_is_shown_blocked_and_rejected`, `revert_overlapping_other_lease_or_stale_is_rejected_without_effect`) | 통과 |
| 결과 autosave 예약 | 미확인: 적용 결과 change set을 돌려주지만 runtime autosave 연결은 XXIX.3.1에서 합니다 |

## 차이(작업 지시와 다른 점)

- 렌더 프리뷰는 UI 재설계 때 합니다.

## 남은 위험 / 한계

- "결과 autosave 예약" 검증 항목은 앱 runtime 연결 전이라 확인하지 못했습니다. 그래서 부분 완료입니다.
- 미리보기는 문서 사본 둘을 만듭니다. 큰 문서 실측은 하지 않았습니다.
- 일부 key만 복원된 batch도 "되돌림"으로 표시합니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.8)` 커밋(브랜치 `lane/agent-history`).
