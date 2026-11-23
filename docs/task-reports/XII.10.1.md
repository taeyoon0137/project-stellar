# 작업 보고서: XII.10.1 캔버스 도구 closed enum과 도구 상태 기계

- WBS: XII.10.1
- 작업명: 캔버스 도구 10종 `CanvasTool`, 창별 현재 도구, 일시 전환(Space Hand, ⌘ 깊은 선택), Escape 복귀
- 상태: 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XII.3.6, VII.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/tool.rs`(신규) — `CanvasTool`, `Modifiers`, `CanvasKey`, `ToolInput`, `ToolEvent`, `CanvasToolState`.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 도구 10종(Move, Frame, Rectangle, Ellipse, Line, Polygon, Text, Image, Hand, Zoom)을 닫힌 enum으로 두고 도구 키(ADR-0104 결정
  1.5), 만들기 도구 여부, XII.3.6 상호작용 도구(`InteractionTool`) 대응을 함께 둡니다.
- `CanvasToolState`는 고른 도구와 일시 전환을 가진 창별 상태 기계입니다. 제스처가 없을 때 Space를 누르는 동안 Hand, 제스처 중
  Space는 제스처에 넘김(`SpaceInGesture`), Hand 제스처 중 Space를 떼면 제스처가 끝날 때 복귀, ⌘를 누르는 동안 Move 도구의 깊은
  선택, 초점 상실 시 수식키·Space 해제, 생성 완료 뒤 Move, Image 파일 고르기 취소 시 이전 도구입니다.
- Escape는 도구 기계가 소비하지 않고(`NotConsumed`) 선택 엔진의 Escape 순서(ADR-0104 결정 3.5)가 `reset_to_move`를 부릅니다.
- 도구 상태는 문서·history와 연결이 없는 값이라 저장·history 대상이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::tool` | 7 passed(tool 4 + tool_units 3) |

| 검사 | 결과 |
|---|---|
| 도구 10종 키가 서로 다르고 대소문자 무관, Polygon 키 없음, 만들기 도구 7종(`ten_tools_have_distinct_shortcuts_and_roles`) | 통과 |
| 도구 전환·키 반복 무시·Space 일시 Hand·Escape 복귀·⌘ 깊은 선택·생성 뒤 Move·Image 취소 복귀(`switching_temporary_switches_and_escape_fixture`) | 통과 |
| 제스처 중 Space는 제스처로, 팬 중 Space 뗌은 제스처 끝에 복귀, 초점 상실 해제(`space_during_gesture_goes_to_the_gesture_and_hand_waits_for_the_gesture_end`) | 통과 |
| 전환 전후 save bytes·history 길이·문서 revision 동일(`tool_changes_leave_save_bytes_and_history_alone`) | 통과 |

## 차이(작업 지시와 다른 점)

- 현재 도구를 commands `WindowViewState`가 아니라 runtime 창별 캔버스 상태(`CanvasToolState`)에 둡니다(ADR-0104 결정 3.1).
  `WindowViewState`는 history가 소유하는 선택 상태이고, 도구는 history와 무관합니다.
- 도구 목록의 근거 문서(도구·메뉴 명세 6.2)는 ADR-0098로 지워져 계획서 문구로 정했습니다(ADR-0104 결정 1).

## 남은 위험 / 한계

- 창 session(`ProjectWindowSession`)이 이 상태를 소유하게 잇는 일과 셸 키 입력 연결은 UI 재설계 때 합니다.
