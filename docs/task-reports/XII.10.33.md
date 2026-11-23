# 작업 보고서: XII.10.33 캔버스 텍스트 편집

## WBS

- WBS: XII.10.33
- 작업명: Text 도구 자동·고정 폭 생성, 편집 모드, 입력기 조합, 편집 중 typography 변경 코어
- 상태: 부분 완료 (`[~]`) — 코어(공통 text buffer, runtime 편집기, 폭 규칙)와 검증 열 fixture는 끝났습니다.
  2026-11-24 text-instance-2 작업으로 빈 새 Text 정리와 앱 Text 입력 경로 하나로 합치기를 끝냈습니다. 셸 입력
  연결(`NSTextInputClient`), Text 상자 폭을 쓰는 scene 그리기가 남았고, 선행 XII.10.26이 `[ ]`입니다.
- 선행: XII.10.32, XII.5.4, XII.5.6, X.10.3, XII.10.26
- 결정: ADR-0118 결정 1·2·3, ADR-0129 결정 1·2(ADR-0118 결정 8 대체)

## 변경 파일

2026-11-24 text-instance-2 작업:

- `crates/runtime/src/canvas/text/edit.rs`(수정) — 기록 host trait `TextEditHost`와 코어 경로 `HistoryHost`. 편집기
  기록 메서드가 host를 받습니다. `finish`가 이 편집에서 만든 Text가 비었으면 `delete-subtree` 항목 하나를 기록하고
  `EditEnd::Removed`를 돌려줍니다.
- `crates/runtime/src/canvas/text/edit_tests.rs`(수정) — host 인자 반영, 빈 새 Text 정리 fixture.
- `crates/runtime/src/app/text_input.rs`(다시 씀) — 창마다 `CanvasTextEditor` 하나를 두고 `AppRuntime` Text 요청을
  편집기 호출로 바꿉니다. 창 session host가 Undo 이름표(`TYPING`·`CHANGE_PROPERTY`·`DELETE`)를 붙입니다. 옛 별도
  구현(조합 범위 직접 기록, `GroupingKey::TextInput` 직접 조립, 표시·잠금 직접 판정)을 지웠습니다. FFI가 쓰는 메서드
  이름·인자는 그대로이고 `end_canvas_text`·`delete_canvas_text`·`move_canvas_caret`·`set_canvas_text_style`과
  `TextTarget::Placement`를 더했습니다.
- `docs/adr/ADR-0129-text-instance-editing-followups.md`(신규).

2026-11-23 canvas-text-instance 작업:

- `crates/text/src/edit.rs`(신규) — `TextEditBuffer`: 본문·선택·입력기 조합, grapheme 단위 삭제·caret 이동, 단어·문단
  이동, 외부 변경 뒤 본문 다시 읽기.
- `crates/text/src/lib.rs`(수정) — `edit` mod 등록.
- `crates/runtime/src/canvas/text/edit.rs`(신규) — `plan_text_tool`(클릭 자동 폭, 끌기 고정 폭), `text_width_mode`,
  `edge_width_command`(변 끌기 자동 → 고정 폭), `CanvasTextEditor`(새 Text 첫 확정 생성, 확정 입력 기록, 조합
  미리보기, Escape·끝내기, typography 한 transaction).
- `crates/runtime/src/canvas/text/edit_tests.rs`(신규) — fixture.
- `crates/runtime/src/canvas/text.rs`(수정) — `edit` mod 등록. `TextPreview`를 공개 타입으로 바꾸고 읽기 메서드를
  더했습니다.
- `docs/adr/ADR-0118-canvas-text-instance-keyboard-core.md`(신규), `docs/task-reports/XII.10.33.md`(신규).

## 명령과 종료 코드

2026-11-24 text-instance-2 작업:

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-runtime --lib -- app::text_input canvas::text::edit` | 0 (20 passed) |
| `cargo clippy -p stellar-document -p stellar-commands -p stellar-runtime -p stellar-ffi -p stellar-ops --all-targets --locked -- -D warnings` | 0 |
| `cargo check -p stellar-ffi --tests` | 0 (FFI 무변경) |

2026-11-23 canvas-text-instance 작업:

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-text --lib edit::` | 0 (6 passed) |
| `cargo test -p stellar-runtime --lib canvas::text::edit` | 0 (11 passed) |
| `cargo clippy -p stellar-text -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음. 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 테스트

- 한글 조합 fixture(`hangul_composition_on_new_text_records_once_on_commit`): ㅎ→하→한 조합 중에는 문서
  revision·history가 그대로이고 미리보기만 바뀝니다. 확정 때 새 Text를 자동 폭(`width: max-content`)으로 만들고
  선택합니다. 이어진 조합 확정까지 history 1건이며, Undo 한 번에 생성과 입력이 함께 되돌아갑니다.
- 일본어 조합 fixture(`japanese_conversion_on_existing_text_is_one_replace`, buffer
  `japanese_conversion_replaces_the_selection_once`): 선택 위 かな→漢字 변환은 확정 전까지 transient입니다. 바깥
  클릭 확정 때 교체 하나와 Text 선택 기록 하나가 생깁니다.
- 빈 생성 취소(`new_text_ended_without_input_records_nothing`): 조합 중 Escape는 조합만 버리고, 다음 Escape로 끝내면
  history 0건이고 문서 레코드가 그대로입니다.
- 편집 중 glyph source(`composition_preview_glyphs_come_from_the_common_text_engine`): 조합 미리보기를 scene에 넣으면
  같은 글꼴 cache·공통 text layout으로 조합 본문의 glyph 수(3, 1)가 그려집니다.
- 고정 폭·변 끌기(`fixed_width_creation_and_edge_drag_turns_auto_into_fixed`), 연속 입력 묶기와 undo 뒤 다시 읽기
  (`typing_and_backspace_group_and_follow_undo`), typography 한 transaction(선택·조합·caret 세 경우와 표에 없는 번호
  거부, `typography_change_during_editing_is_one_transaction`, `style_before_creation_is_part_of_the_creating_entry`,
  `style_on_a_composition_equal_to_the_replaced_text_writes_only_the_style`), 잠금 거부(`locked_text_and_locked_parent_are_not_editable`).
- buffer: grapheme 삭제(ZWJ emoji, 결합 문자, 국기), 단어·문단 이동, 빈 marked text 취소, 외부 변경 뒤 caret 위치.
- 검토 보완(2026-11-24): 기록이 실패한 확정 입력은 buffer를 되돌리고 실패한 다음 입력 style을 버립니다
  (`failed_record_restores_the_buffer_and_drops_the_pending_style`). 편집 중 다른 경로로 잠긴 Text는 다음 입력을
  거부합니다(`text_locked_during_editing_stops_further_input`). buffer의 grapheme·단어 경계 계산을 본문 길이에
  선형으로 바꿨습니다(이전에는 경계마다 앞부분을 다시 세어 제곱 비용이었습니다).
- 남은 경고 0.

## 성능

- 해당 없음. 편집 연산은 본문 길이에 선형입니다. IME 조합 replay는 XII.10.43의 몫입니다.

## 차이

- 셸(AppKit `NSTextInputClient`, 더블클릭·Return 입력 연결)은 사용자 방향(2026-11-23: 엔진만, UI 재설계 전)에 따라
  만들지 않았습니다. 더블클릭·Return의 편집 요청은 선택 엔진의 `SessionOutput::EditContent`이고, 편집기는 그 요청을
  `CanvasTextEditor::begin_existing`으로 받습니다.
- 계획 문구가 가리키는 `docs/design/property_model.md`는 지워졌습니다. 자동 폭의 CSS 표현(`width: max-content`)과
  고정 폭(`width: <px>`, 높이 미설정)은 ADR-0118 결정 1로 정했습니다.
- 편집 중 typography 변경은 표에 있는 문자 style 번호만 씁니다. style 표 항목을 만드는 command가 없습니다.

## 테스트(2026-11-24 text-instance-2)

- 빈 새 Text 정리(`emptied_new_text_is_removed_with_one_entry_and_existing_text_stays`): 새 Text에 "ab"를 넣고 두 번
  지운 뒤 끝내면 `EditEnd::Removed`, 문서에서 사라지고 history가 1건 늘며 선택이 비워집니다. Undo 한 번이 빈 Text와
  선택을 되살리고, 나머지 Undo가 문서를 처음으로 되돌립니다. 기존 Text는 비워도 남고 선택됩니다.
- 앱 경로(`app::text_input::tests`, `AppRuntime` 요청과 완료 event로 확인):
  - 한글 조합 미리보기 중 history·저장 상태 불변. 교체 확정과 이어지는 삽입이 입력 종류별로 묶이고 Undo 이름표가
    `TYPING`.
  - 같은 값 확정 무기록, 범위 밖·다른 View·낡은 revision·Text 아닌 node·잠긴 조상 거부.
  - 새 Text 첫 확정 생성·선택, 입력 없이 끝내면 무생성.
  - 고정 폭 새 Text를 비우고 끝내면 삭제 항목(`DELETE`) 하나. Undo가 되살리고 선택합니다.
  - caret 이동·grapheme 삭제·조합 취소·style 변경(`CHANGE_PROPERTY`)·끝난 token 거부.
  - 기본 Text style 없음 거부.

## 남은 위험

- scene의 text layout이 Text `width`를 아직 쓰지 않습니다(`max_width` 없음). 고정 폭 줄바꿈은 layout 결과 그리기
  연결 뒤에 화면에 보입니다.
- FFI Text ABI의 문서 주석(`crates/ffi/src/text_input.rs`)은 옛 의미(확정이 조합을 끝냄)로 적혀 있습니다. 셸 FFI를
  바꾸지 않는 방향이라 고치지 않았습니다. 셸 재설계 때 `end_canvas_text`를 쓰는 ABI로 바꿔야 합니다.
- 빈 Text 정리는 별도 Undo 항목입니다(ADR-0129 결정 1).

## 후속 수정 (2026-11-24 instance-integrity: 한 요청의 끝내기·입력 CommandId 분리, ADR-0133 결정 5)

### 배경

`app::text_input`의 `begin()`은 다른 대상의 남은 편집을 끝낼 때(빈 새 Text 정리, 남은 조합 확정) 요청 ID로 만든
CommandId를 썼고, 같은 요청의 이어지는 입력(`replace_canvas_text`)도 같은 CommandId를 썼습니다. 또 끝내기 뒤에 요청
revision을 다시 비교해, 끝내기가 무엇을 기록하면 그 요청은 끝내기만 남기고 stale로 실패했습니다. 새 Text를 입력했다가 모두
지운 채 다른 Text에 바로 입력하는 첫 요청이 실패했습니다.

### 변경 파일

- `crates/runtime/src/app/text_input.rs` — `end_command_id`(요청 CommandId + 용도 표지 `text-end`), `finish`가 CommandId를
  인자로 받음, 끝내기 뒤 revision 비교를 빼고 새 대상 확인을 끝내기 뒤 문서로 함.
- `docs/adr/ADR-0133-instance-structure-integrity.md` 결정 5.

### 실행한 명령

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib app::text_input` | 통과(7 passed) |
| `cargo clippy -p stellar-runtime --all-targets --locked -- -D warnings` | 통과 |

### 테스트

- `ending_a_previous_edit_and_typing_in_one_request_use_distinct_command_ids`: 새 Text에 "a"를 입력하고 지운 뒤 다른 Text
  11에 바로 입력하는 `replace_canvas_text` 요청 하나가 성공하고, 빈 새 Text 정리와 입력 두 history 항목을 서로 다른
  CommandId로 기록합니다(창 history의 모든 문서 항목 CommandId가 서로 다름). 고치기 전에는 이 요청이 stale로 실패했습니다.
