# 작업 보고서: XII.10.4 깊은 선택과 편집 범위 이동

- WBS: XII.10.4
- 작업명: ⌘-클릭 깊은 선택, 더블클릭·Return 한 단계 아래, Shift-Return 부모, Tab·Shift-Tab 형제 순환, Shift-더블클릭 깊은 node
  추가, 범위가 최상위가 아닐 때 Escape 한 단계 위, 선택에 맞춘 범위 재조정
- 상태: `[x]` 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XII.10.3(이 작업에서 구현)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/select_navigate.rs`(신규) — `Navigation`, `double_click`, `add_deep`, `enter`, `exit_to_parent`,
  `cycle_sibling`.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 규칙은 ADR-0104 결정 3.4입니다. ⌘-클릭 깊은 선택은 XII.10.3의 `click_pick(deep)`이고, Escape의 범위 위로 단계는 XII.10.3의
  `escape`가 같은 Escape 순서 안에서 합니다.
- 더블클릭은 이미 선택된 컨테이너(Frame, Fragment) 안으로 들어가 커서 아래 직계 자식을 고르고, 커서 아래 자식이 없으면 선택 없이
  범위만 들어갑니다. Return은 선택한 컨테이너의 선택 가능 자식 전체입니다. Text·Path는 내용 편집 요청(`EditContent`)을 돌려주어
  XII.10.29·XII.10.33이 받습니다.
- 편집 범위는 창별 값(`EditingScope`)이고, 새 선택마다 `EditingScope::refit`(선택 부모들의 가장 가까운 공통 조상)으로 맞춥니다.
  사이드바 깊은 선택도 같은 함수로 범위를 맞춥니다. 범위 변경은 저장·history에 없고 선택이 바뀔 때만 `SetSelection` 하나입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::select` | 9 passed |

| 검사 | 결과 |
|---|---|
| 더블클릭 두 번으로 11 → 14 → 17 진입, Shift-Return·Return·Escape 두 번으로 이탈, 몸체 더블클릭은 범위만 진입, Fragment 진입(`enter_exit_and_escape_walk_the_nested_scopes`) | 통과 |
| Tab·Shift-Tab OrderKey 순환(잠김·숨김 건너뜀, 빈 선택은 첫·마지막), Shift-더블클릭 깊은 node 추가와 범위(`tab_cycles_siblings_in_order_key_order_and_shift_double_click_adds_deep`) | 통과 |
| 사이드바 깊은 선택 뒤 범위 14, 범위만 바뀌는 이동은 history 0건, 선택이 바뀌는 이동만 1건, save bytes 불변(`sidebar_deep_selection_refits_scope_and_scope_moves_stay_out_of_history_and_save`) | 통과 |
| Text·Path의 Return·더블클릭은 내용 편집 요청, Rectangle은 그대로(`text_and_path_request_content_editing`) | 통과 |

## 차이(작업 지시와 다른 점)

- 편집 범위를 commands `WindowViewState`가 아닌 runtime 창별 값으로 둡니다(ADR-0104 결정 3.1).
- 캔버스 명세 3.2절은 지워져 Return(자식 전체), Shift-Return의 최상위 처리(그대로), Tab의 빈 선택 처리(첫·마지막 자식)를 ADR-0104에서
  정했습니다.

## 남은 위험 / 한계

- 사이드바 선택 경로와 창 session에 범위 상태를 붙이는 연결은 UI 재설계 때 합니다.
- 인스턴스 안 깊은 선택(읽기 전용 윤곽)은 XII.10.34 몫이라 지금은 인스턴스가 한 덩어리입니다.
