# 작업 보고서: XII.10.40 한 제스처 한 history 적합성 검사

- WBS: XII.10.40
- 작업명: 선택 변경과 편집이 한 제스처에서 일어나면 undo 한 번으로 둘 다 되돌리고, 취소·hover·camera·도구 전환·편집 범위 이동은 기록하지 않음
- 상태: `[~]` 부분 완료 보고. 기록 규칙과 적합성 fixture는 통과. 선행 XII.10.9(크기 조절)·XII.10.13(이동)이 아직 없어 실제 이동·크기 command
  대신 Frame `width` preview로 드래그 편집을 재생했습니다.
- 선행: VII.4.2, VII.4.4, VII.4.6, XII.3.7, XII.10.9(미착수), XII.10.13(미착수)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/select_gesture.rs`(신규) — `GestureRecorder`(`begin`, `select`, `preview`, `finish`, `cancel`), `GestureRecord`.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 규칙은 ADR-0104 결정 5입니다. 제스처가 문서 항목을 만들면 그 항목의 `SelectionRestore`가 제스처 전·후 선택을 담고 Selection 항목은
  없습니다. 편집 없이 선택만 바뀌면 Selection 항목 하나, 취소하면 문서·선택을 제스처 전으로 돌리고 기록하지 않습니다. `HistoryEntry`
  타입은 바꾸지 않았습니다.
- `GestureRecorder`는 `WindowHistory` 공개 API(`begin_pointer_gesture`, `update_preview`, `end_preview`, `cancel_preview`,
  `record_selection`)와 `WindowViewState::restore_selection`으로 이 규칙을 냅니다. preview 중 선택은 `SetSelection::filtered`와 같은
  규칙으로 거른 뒤 항목 없이 view에만 반영하고, 끝에서 편집이 없으면 제스처 전 선택으로 되돌린 다음 `record_selection`으로 한 항목을
  남깁니다. 그래서 undo는 제스처 전 선택으로 돌아갑니다.
- 제스처 재생은 실제 선택 엔진(`plan_press` → `SelectAndDrag`·`KeepSelection`)을 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::select_gesture` | 3 passed |

| 검사 | 결과 |
|---|---|
| ADR-0104 결정 5 표: 클릭(선택 바뀜) 선택 1, 같은 선택 클릭 0, 선택 node 드래그 문서 1, 마퀴 선택 1, hover·camera 팬·줌·도구 전환·범위만 이동 0, Escape 해제·더블클릭·Return·Tab 각 선택 1(`gesture_table_entry_counts`) | 통과 |
| 선택하지 않은 node 즉시 500 update 드래그: 항목 1건(제스처 전·후 선택 포함), undo 1회로 save bytes·선택 모두 제스처 전, redo로 다시 적용(`immediate_drag_of_an_unselected_node_undoes_document_and_selection_together`) | 통과 |
| 움직이지 않은 click 선택 1건과 undo의 제스처 전 선택, 선택 변경·20 update 뒤 취소 0건·선택·문서 원복, 편집 없는 취소도 원복(`click_without_movement_records_one_selection_and_cancel_restores_everything`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 문구는 `WindowHistory`에 "pointer gesture 안에서 선택을 항목 없이 view에 반영하는 API"를 더하라고 했습니다. history crate는 이 작업의
  소유 범위 밖이라, 같은 의미를 runtime `GestureRecorder`가 `WindowHistory` 공개 API로 냅니다(ADR-0104 결정 5.2). 제스처 중 선택 반영은 이
  기록기만 합니다.
- 캔버스 명세 19장 표는 ADR-0098로 지워져 ADR-0104 결정 5 표로 대신했습니다.
- 크기 조절·이동 command(XII.10.9, XII.10.13)가 아직 없어 드래그 편집을 Frame `width` `SetProperty` preview로 대신했습니다. 그 두 행이
  들어오면 같은 기록기로 fixture를 실제 command로 바꿔 다시 확인해야 합니다.

## 남은 위험 / 한계

- 크기·이동 제스처(XII.10.9, XII.10.13)가 이 기록기를 쓰도록 연결하는 일과 실제 command로의 재검증이 이 행을 닫는 데 남은 일입니다.
- `WindowViewState::restore_selection`은 undo·redo 복원용으로 문서화된 API입니다. 제스처 중 선택에 쓰는 것은 ADR-0104가 정한 예외입니다.
