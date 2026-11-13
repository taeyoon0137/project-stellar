# 작업 보고서: VII.4.6 선택·문서 편집 통합 시간순 stack

- WBS: VII.4.6
- 작업명: Selection event와 document edit를 동일 시간순 스택에 넣고, Undo는 마지막 종류와 무관하게 한 entry씩 되돌림
- 상태: 완료 (`[x]`)
- 선행: VII.3.2, VII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/history.rs` — `record_selection`, `undo`, `UndoOutcome`, 통합 stack fixture 테스트 2건.
- `docs/task-reports/VII.4.6.md`(신규), 계획서(VII.4.6 `[ ]`→`[x]`).

## 구현 요약

- **`record_selection`**: `SetSelection`(VII.3.2)을 view 상태에 적용하고 문서 편집과 **같은 stack**에 선택 항목으로 기록합니다(같은
  sequence 공간). 걸러진 결과가 현재 선택과 같으면 기록하지 않습니다. 영역 선택(VII.3.3)의 최종 event도 이 경로로 한 건만 기록합니다.
- **`undo`**: 마지막 항목 하나를 종류와 무관하게 되돌립니다 — 문서 항목은 역 transaction을 현재 revision으로 rebase해 원자 실행하고,
  사라진 node를 선택에서 정리합니다(VII.3.4 system event, 항목 없음). 선택 항목은 이전 선택으로 되돌립니다. 역 실행이 실패하면 항목을
  stack에 남기고 오류를 반환합니다. preview 진행 중에는 거부합니다.
- redo stack은 VII.5.3에서 추가하므로 이번 undo는 되돌린 항목을 결과(`UndoOutcome`)로 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 16 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 선택[11] → 이름 편집 → 선택[12,13]: stack 종류 [선택, 문서, 선택]·sequence 1..3, undo 1회씩 선택[11](문서 불변) → 이름 원복(선택 불변) → 빈 선택, 빈 stack undo는 `None`(`selection_edit_selection_undo_in_order`) | 통과 |
| 같은 선택 미기록, 없는 id 필터, 삽입 undo 시 선택에서 삭제 id만 정리하고 항목 미생성(`unchanged_selection_is_not_recorded_and_undo_prunes_removed_nodes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- redo와 창별 분리는 VII.5.x에서 추가합니다.
