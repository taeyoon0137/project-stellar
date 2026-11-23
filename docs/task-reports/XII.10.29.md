# 작업 보고서: XII.10.29 기존 Path의 벡터 편집 모드

- WBS: XII.10.29
- 작업명: 기존 Path의 벡터 편집 모드
- 상태: 완료 (`[x]`) — 코어. point·handle 표시 그리기와 셸 입력 연결은 UI 재설계 때
- 결정: ADR-0117 결정 7
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/path_edit.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)

## 구현 요약

- `VectorEditMode::enter`·`from_navigation`: 선택 엔진의 더블클릭·Return 결과(`Navigation::EditContent`)가 Path면 들어가고, Shape면 `NotAPath`입니다. 변환은 하지 않습니다(Convert to Path만 변환). 잠금은 `commands::lock`. 모드는 창별 값이고 Escape는 값을 버리는 것입니다.
- `knobs`: `IdentifiedPath`의 point·handle을 id와 local 위치로 나열합니다. 분기형 network는 문서 모델에 없습니다.
- `path_world`: 저장 위치 변환(`place::local_transform`)과 가장 가까운 scene 행 조상의 world를 합칩니다.
- `hit`: 화면 반지름 `RADIUS_HANDLE_PT`(ADR-0104 표) 안의 가장 가까운 point·handle(같으면 point).
- `PointDrag`: update는 순수 계산, `commit`이 `EditPathPoint`(`MovePoint`·`MoveHandle`) 하나를 history 한 항목으로 기록합니다. 움직이지 않으면 0건입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::path_edit` | 통과(2): Path만 들어가고 Shape(Rectangle)는 `NotAPath`·payload·save bytes 불변, point hit 반지름(배율 반영)과 handle hit, 100 update 동안 revision·history 불변, 확정 history +1과 point 위치·id, undo 한 번 복원, 움직이지 않은 drag 0건 |

## 차이(작업 지시와 다른 점)

- 없음. point·handle 표시 그리기는 셸 재설계 때 합니다.

## 남은 위험 / 한계

- 지금 캔버스 scene 목록(`CanvasScene`)은 Path node 행을 만들지 않습니다. 그래서 `path_world`는 문서 저장 위치와 조상 행 world로 계산합니다. scene이 Path 행을 갖게 되면 그 행의 world를 쓰게 바꿔야 합니다.
