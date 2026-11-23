# 작업 보고서: XII.10.41 키보드만으로 하는 캔버스 편집 경로

## WBS

- WBS: XII.10.41
- 작업명: 캔버스 포커스 표시, 키 배정, 선택 이동·이동·크기 키 실행, 화면 가운데 삽입, 문맥 메뉴 위치, 핸들 대체 표 코어
- 상태: 부분 완료 (`[~]`) — 코어는 끝났습니다. 검증 열의 UI 자동화 시나리오와 Full Keyboard Access smoke는 셸이
  있어야 해서 남았습니다. 선행 XII.10.39(문맥 메뉴)가 `[ ]`입니다.
- 선행: XII.10.4, XII.10.14, XII.10.39, XII.8.1
- 결정: ADR-0118 결정 7, ADR-0104 결정 2

## 변경 파일

- `crates/runtime/src/canvas/keyboard.rs`(신규) — `CanvasFocus`, `KeyboardKey`, `keyboard_command`, `select_key`,
  `transform_key`, `context_menu_anchor`, `plan_insert_at_center`, `handle_equivalents`.
- `crates/runtime/src/canvas/keyboard_tests.rs`(신규) — fixture.
- `crates/runtime/src/canvas.rs`(수정) — `keyboard` mod 등록.
- `docs/task-reports/XII.10.41.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::keyboard` | 0 (7 passed) |
| `cargo clippy -p stellar-text -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음.

## 테스트

- 키 표(`key_table_follows_the_unit_adr`): 네 화살표의 1·10px 이동과 ⌘·⌘⇧ 1·10px 크기를 확인합니다. Option-화살표는
  쓰지 않습니다. Tab·Return은 선택 엔진으로, Shift-F10·문맥 메뉴 키·⌃Return은 문맥 메뉴로, ⌘A는 전체 선택으로 갑니다.
- 포커스 표시(`focus_ring_shows_only_for_keyboard_use`).
- 선택 이동(`tab_return_and_shift_return_move_the_selection_with_keys_only`): 선택 없음 → Tab 첫 자식, Tab·Shift-Tab
  순환, Return 안으로(선택 가능 자식 전체), Shift-Return 부모, Escape 해제를 키로만 합니다.
- 이동·크기(`arrows_move_and_command_arrows_resize_through_the_keyboard_transform_path`): Shift-→ 10px 이동, ⌘⇧-↓
  높이 +10px이 각각 history 1건입니다. 선택이 없으면 camera만 움직이고 revision이 그대로입니다.
- 화면 가운데 삽입(`insert_plans_center_a_default_size_box_in_parent_space`): 확대·이동한 camera와 원점이 옮겨진 부모에서
  100×100 상자, Line 수평 두 점, Text 자리(자동 폭)의 가운데가 화면 가운데입니다.
- 문맥 메뉴 위치(`context_menu_opens_at_the_selection_or_the_view_center`).
- 핸들 대체(`every_handle_action_has_a_keyboard_or_inspector_equivalent`): 모든 핸들 종류에 대체가 하나 이상 있습니다.
  키 대체는 실제 이동·크기 동작으로 풀리고, 인스펙터 이름은 CSS registry 또는 layout longhand에 있습니다.
- 남은 경고 0.

## 성능

- 해당 없음. 키 배정은 상수 시간이고, 실행은 기존 변형·선택 경로와 같습니다.

## 차이

- 캔버스 명세 20.1절 표와 UI 자동화·Full Keyboard Access smoke는 지워진 문서와 셸에 묶여 있어 실행하지 않았습니다
  (사용자 방향 2026-11-23). 대신 계획 문구의 동작마다 코어 fixture를 두었습니다.
- Insert 메뉴 기본 크기(100px)와 선택 없음 화살표 camera 이동(40pt, Shift 200pt)은 ADR-0118 결정 7로 정했습니다.
- 회전 핸들의 인스펙터 대체는 `transform`입니다. `rotate` longhand는 이름만 있고 영속 record가 없습니다.

## 남은 위험

- 셸이 생기면 이 키 배정을 메뉴 명령·`ShortcutPolicy`와 맞추고 UI 자동화로 다시 확인해야 합니다.
- 화면 가운데 삽입은 기존 `insert_canvas_frame`·`insert_canvas_shape`에 넘기는 계획만 만듭니다. 그 경로는 칠 값을
  셸 설정에서 받습니다.
