# 작업 보고서: XII.10.24 흐름 컨테이너 안 드래그

- WBS: XII.10.24
- 작업명: 흐름 컨테이너(flex·grid) 안 드래그의 삽입 위치, 증분 preview, OrderKey 확정
- 상태: 부분 완료 (`[~]`) — 코어 완료, 삽입 표시·preview 그리기(셸)와 `position: absolute` 자식의 inset 쓰기 남음
- 결정: ADR-0117 결정 5(C6 포함)
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/flow_drag.rs`(신규), `crates/runtime/src/canvas/flow_drag/tests.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)

## 구현 요약

- 배치 문맥은 기존 `stellar_commands::arrange::align::flow_parent` 하나로 판정합니다(Fragment를 건너뛴 컨테이너, `*-reverse`, 활성 기본값 세트).
- `insertion_index`: 끄는 node를 뺀 형제의 scene 상자를 흐름 순서로 보고 교차축으로 겹치는 상자를 줄로 묶은 뒤, pointer가 있는(없으면 가장 가까운) 줄에서 주축 중심이 앞인 형제 수를 셉니다. flex 가로·세로·wrap·reverse와 Grid 자동 배치(읽는 순서)가 같은 규칙입니다.
- `reflow_preview`: 흐름 순서 permutation reflow로 형제가 비키는 preview 상자를 냅니다. 줄마다 원래 항목 수·시작점·관측 간격을 지킵니다.
- `FlowDrag`: 시작 때 한 번 형제·키를 읽고 update는 순수 계산(문서·history 불변). 놓으면 같은 컨테이너면 OrderKey만 바꾸는 `MoveNode` 하나, 다른 흐름 컨테이너면 그 삽입 index의 `MoveNode`, 자유 배치 Frame이면 기존 `MoveDrag`의 world 위치 보존 재부모화(ADR-0105)입니다. 같은 자리면 기록하지 않습니다. 잠금은 `commands::lock::edit_permission`으로 판정합니다.
- **C6 결정과 근거**: Grid 명시 배치 자식(줄 번호 `<integer>`가 있는 `grid-*-start/end`)은 drag로 순서를 바꾸지 않고 `grid-explicit-placement`로 거부하며, 자동 배치 형제의 삽입 index에서 뺍니다. 그 자식의 자리는 OrderKey가 아니라 줄 번호가 정해 OrderKey를 바꾸면 화면이 움직이지 않고, 줄 번호를 쓰려면 runtime scene에 없는 grid track 기하가 필요하기 때문입니다. 줄 번호는 CSS 값 입력으로 바꿉니다.
- 인스턴스 투영 행 위로 놓기·그 안 재정렬은 `instance-slot`으로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::flow_drag` | 통과(7): flex 가로·세로·reverse 삽입 index, wrap·Grid 자동 배치 index, preview 비키기, 50 update 뒤 놓기 = `MoveNode` 1건(history +1, 같은 자리 0건), 자유 Frame 끌어내기와 인스턴스 투영 행 거부, C6 명시 배치 거부와 자동 형제 index 제외, 자유 배치 자식은 흐름 drag 아님 |

## 차이(작업 지시와 다른 점)

- "형제를 증분 레이아웃 preview로 비키게"는 layout crate를 runtime에서 부를 수 없어(crate 그래프) 흐름 순서 permutation reflow로 냈습니다. 같은 크기 항목에서 정확하고, 크기가 다르거나 `justify-content`가 분배 값이면 근사입니다. 놓은 뒤 화면은 실제 layout이 정합니다.
- 흐름에서 빠진 자식(`position: absolute`)은 흐름 drag가 아니라 기존 자유 배치 이동 경로입니다. 그 저장 위치는 ADR-0105 결정 1(IX.11.9 step 전에는 `transform`)이라 "inset은 IX.11.6 영속 record 값으로 씀"은 IX.11.9 step 등록 때 합니다.
- 삽입 표시·preview 그리기는 셸 재설계 때 합니다. 코어는 표시 상자와 preview 상자 값을 냅니다.

## 남은 위험 / 한계

- 지금 scene은 흐름 자식을 layout 결과가 아니라 저장 위치로 그립니다(ADR-0086 결정 3). scene이 layout 결과를 그리게 되면 같은 함수가 실제 흐름 상자를 받습니다.
