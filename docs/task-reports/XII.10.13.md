# 작업 보고서: XII.10.13 이동 드래그

- WBS: XII.10.13
- 작업명: 이동 드래그
- 상태: 부분 완료 (`[~]`) — 축 고정·복제·재부모화 완료, Slot 배치·저장 위치(IX.11.9) 남음
- 결정: ADR-0105 결정 1·2
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/transform/drag.rs`(신규)
- 소유 밖 최소 수정: `crates/runtime/src/app/canvas_gesture.rs`의 `move_command`가 `stellar_commands::arrange::place::translate`를 부르게 함(같은 저장 규칙의 이중 구현 제거, 기존 test 통과)

## 구현 요약

- Shift는 더 많이 움직인 축만 남깁니다. 스냅을 켜면 update마다 선택 경계를 맞춥니다.
- 같은 부모 안 이동·Option 복제는 기존 `CanvasEdit::Move` 경로 하나로 확정합니다(복제·이동·선택이 한 항목).
- `drop_target`: 자유 배치 Frame 위면 world 위치를 지키며 `MoveNode`로 부모를 바꿉니다. 흐름 컨테이너는 `flow-container`, 인스턴스 투영 행은 `instance-slot`으로 거부합니다.
- update는 문서를 바꾸지 않고 확정은 history 1건입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::drag` | 통과(5): 축 고정·500 update 뒤 1건, Option 복제, 재부모화 world 위치·undo, 흐름 컨테이너 거부, 스냅 보정 |
| `cargo test -p stellar-runtime --lib -- app::canvas_gesture` | 통과(19): `move_command` 연결 뒤 기존 이동·복제·UUIDv7 test |

## 차이(작업 지시와 다른 점)

- 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다. 그래서 "이동 후 `transform`·`translate` 불변", "item record 위치 longhand뿐" 기준은 미충족입니다.
- 공개 Slot 배치는 Slot 편집 행이 없어 인스턴스 안 놓기를 모두 거부합니다. 인스턴스 fixture로 거부를 확인하지 못했습니다(행 판정만 test).
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- IX.11.9 뒤 다시 판정해야 합니다.
