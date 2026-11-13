# 작업 보고서: VII.5.3 redo stack 폐기 규칙

- WBS: VII.5.3
- 작업명: Undo 후 같은 창에서 새 command가 확정되면 redo stack 폐기, 다른 창 저장 변화는 해당 창 history를 즉시 폐기하지 않음
- 상태: 완료 (`[x]`)
- 선행: VII.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/history.rs` — redo stack, `redo`, `redo_entries`/`redo_len`, undo가 redo로 이동, 새 확정(문서 기록·병합·선택 기록·
  preview 확정) 시 redo 폐기.
- `crates/history/src/window.rs` — `WindowHistory::redo`, `WindowHistories::on_generation_saved`(다른 창 history 비폐기, stale 후보 반환),
  테스트 2건.
- `docs/task-reports/VII.5.3.md`(신규), 계획서(VII.5.3 `[ ]`→`[x]`).

## 구현 요약

- **redo**: undo한 항목은 redo stack으로 갑니다. `redo`는 문서 항목의 정방향을 현재 revision으로 rebase해 원자 실행(삭제 node는 선택에서
  정리)하거나 선택 항목의 정방향 선택을 적용하고 항목을 undo stack으로 되돌립니다. 실패하면 항목을 redo stack에 남깁니다.
- **폐기 규칙**: 같은 창에서 새 항목이 확정되면(일반 기록, grouping 병합, 선택 기록, preview 확정) redo 갈래를 버립니다. 기록하지 않은 동작
  (같은 선택, 취소된 preview)은 redo를 건드리지 않습니다.
- **다른 창 저장**: `on_generation_saved(saved_by)`는 다른 창의 history를 바꾸지 않고, 저장 충돌·재로드 정책(§22.2, XII.7)이 처리할 stale
  후보 창 목록만 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 20 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| branch fixture: e1·e2·e3 → undo 2회(redo 2) → B 저장·B 편집 후에도 A undo 1·redo 2 유지(stale 후보 = [A]) → redo/undo 왕복 → 새 e4 확정 시 redo 0 → 끝까지 undo = 원본, 끝까지 redo = e1·e4(`redo_branch_is_discarded_by_new_commit_but_not_by_other_window_save`) | 통과 |
| 선택 항목 undo/redo, 새 선택 기록이 redo 갈래 폐기(`selection_entries_redo_and_new_selection_discards_branch`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 다른 창 저장 후 이 창의 stale 표시·재로드 시 history 폐기(UseLatestDisk)는 XII.7.6·XIII.3.1의 몫입니다.
