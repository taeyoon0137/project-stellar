# 작업 보고서: VII.3.4 삭제 node 선택 정리 system event

- WBS: VII.3.4
- 작업명: node 삭제 시 각 WindowViewState에서 삭제된 ID를 선택에서 제거하는 system event, 사용자 history 항목 없음
- 상태: 완료 (`[x]`)
- 선행: VII.2.2, VII.3.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/selection.rs` — `SelectionPruned` system event, `WindowViewState::{prune_removed, prune_missing}`, 테스트.
- `docs/task-reports/VII.3.4.md`(신규), 계획서(VII.3.4 `[ ]`→`[x]`).

## 구현 요약

- **`prune_removed(&ChangeSet)`**: command 결과의 removed nodes(VII.1.4 순 효과)를 선택과 진행 중인 영역 선택 preview에서 지웁니다.
- **`prune_missing(&DocumentModel)`**: 재로드·외부 변경 반영 뒤 문서에 없는 id를 지웁니다(다른 창이 저장한 generation을 읽은 창).
- 결과는 사용자 history event(`SetSelection`)가 아닌 **system event `SelectionPruned { window, removed, before, after }`**이며, 지울
  것이 없으면 `None`입니다. primary가 빠지면 primary 규칙(VII.3.1)을 다시 적용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 62 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 3개 창(각자 독립 문서): A에서 subtree 삭제 → change set으로 정리, B는 같은 삭제가 반영된 문서로 재로드 후 누락 정리(영역 preview 포함), C 불변 → 모든 창의 선택·preview에 stale id 없음, primary 재적용, 삭제 없는 change set은 무동작(`deletion_prunes_every_window_without_user_history`) | 통과 |

## 차이(작업 지시와 다른 점)

- 정리 결과가 history에 기록되지 않는다는 점은 이 작업에서는 반환 타입(system event)으로 구분했고, history crate 통합 검사는
  VII.5.5(undo 중 selection 복원 reentrancy)·VII.4.6에서 함께 확인합니다.

## 남은 위험 / 한계

- 삭제 undo 시 선택을 되살리는 정책은 history의 selection 복원(VII.5.5)이 담당합니다.
