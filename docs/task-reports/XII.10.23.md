# 작업 보고서: XII.10.23 순서와 묶음 명령

- WBS: XII.10.23
- 작업명: 순서와 묶음 명령
- 상태: 완료 (`[x]`) — 코어 완료(도움말·접근성 알림 문구는 셸)
- 결정: ADR-0105 결정 7
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/commands/src/arrange/{order,group}.rs`(신규)
- `crates/runtime/src/canvas/transform/arrange.rs`(`order`, `group_selection`, `frame_selection_of`, `ungroup_selection`)

## 구현 요약

- 순서 4종은 부모마다 고른 node의 상대 순서를 지키며 OrderKey만 바꾸는 `MoveNode`입니다(아직 옮기지 않은 형제 key와 겹치지 않음).
- 흐름 컨테이너 자식이면 `flow_order_changed`로 흐름 순서 변경을 알립니다(셸 도움말·접근성 알림의 근거).
- Group은 Fragment 삽입과 자식 MoveNode(원래 key 유지), Frame Selection은 배경 없는 Frame 삽입과 자식 위치 보정, Ungroup은 자식을 같은 자리로 옮기고 빈 묶음 삭제입니다. Frame을 풀면 world 위치를 보존합니다.
- 이웃한 node를 묶었다 풀면 순서 key까지 문서 레코드가 같습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib -- arrange::order arrange::group` | 통과: 순서 4종 OrderKey fixture와 undo, 흐름 알림, group 후 ungroup 레코드 동일, Frame Selection·Frame Ungroup world 위치 |
| `cargo test -p stellar-runtime --lib -- canvas::transform::arrange` | 통과: 창 history·선택과 함께 같은 결과 |

## 차이(작업 지시와 다른 점)

- "semantic snapshot 동일"은 semantic snapshot의 원천인 문서 레코드 전체 동일로 확인했습니다(더 강한 조건).
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- 떨어져 있던 node를 묶었다 풀면 묶음 자리에 이웃해 남습니다(되돌리기는 Undo).
