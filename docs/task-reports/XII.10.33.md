# 작업 보고서: XII.10.33 캔버스 텍스트 편집

## WBS

- WBS: XII.10.33
- 작업명: Text 도구 자동·고정 폭 생성, 편집 모드, 입력기 조합, 편집 중 typography 변경 코어
- 상태: 부분 완료 (`[~]`) — 코어(공통 text buffer, runtime 편집기, 폭 규칙)와 검증 열 fixture는 끝났습니다. 셸 입력
  연결(`NSTextInputClient`), Text 상자 폭을 쓰는 scene 그리기가 남았고, 선행 XII.10.26이 `[ ]`입니다.
- 선행: XII.10.32, XII.5.4, XII.5.6, X.10.3, XII.10.26
- 결정: ADR-0118 결정 1·2·3·8

## 변경 파일

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

## 남은 위험

- scene의 text layout이 Text `width`를 아직 쓰지 않습니다(`max_width` 없음). 고정 폭 줄바꿈은 layout 결과 그리기
  연결 뒤에 화면에 보입니다.
- 옛 `app::text_input` 경로가 셸 재연결 전까지 함께 있습니다(ADR-0118 결정 8).
- 새로 만든 Text를 입력한 뒤 모두 지우고 끝내면 빈 Text가 남습니다(ADR-0118 Consequences).
