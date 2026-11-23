# 작업 보고서: XII.10.24 흐름 컨테이너 안 드래그

- WBS: XII.10.24
- 작업명: 흐름 컨테이너(flex·grid) 안 드래그의 삽입 위치, 증분 preview, OrderKey 확정
- 상태: 부분 완료 (`[~]`) — 코어 완료. 2026-11-24 layout-render-wire: preview를 실제 증분 layout으로 만들고 scene 대체 입력을 내며, `position: absolute` 자식 inset 쓰기를 더했습니다. 셸 그리기(삽입 표시 선·preview 표시)는 UI 재설계 때 합니다.
- 결정: ADR-0117 결정 5(C6 포함), ADR-0125 결정 7·8
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/flow_drag.rs`(신규), `crates/runtime/src/canvas/flow_drag/tests.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)
- 2026-11-24 layout-render-wire:
  - `crates/runtime/src/canvas/flow_drag.rs`, `crates/runtime/src/canvas/flow_drag/tests.rs`: 상자를 layout 결과 source에서 읽고 실제 layout preview를 냅니다. `reflow_preview`를 지웠습니다.
  - `crates/runtime/src/canvas/flow_inset.rs`, `crates/runtime/src/canvas/flow_inset/tests.rs`(신규): inset 이동.
  - `crates/runtime/src/canvas/scene_layout.rs`(신규): 활성 View의 layout 반영 scene source(`CanvasLayout`).
  - `crates/runtime/src/canvas.rs`: mod 등록 두 줄.
  - scene 쪽 preview 함수는 `crates/scene/src/document.rs`·`crates/scene/src/document/layout.rs`(ADR-0125, IX.11.9 보고서)에 있습니다.

## 구현 요약

- 배치 문맥은 기존 `stellar_commands::arrange::align::flow_parent` 하나로 판정합니다(Fragment를 건너뛴 컨테이너, `*-reverse`, 활성 기본값 세트).
- `insertion_index`: 끄는 node를 뺀 형제의 화면 상자를 흐름 순서로 보고 교차축으로 겹치는 상자를 줄로 묶은 뒤, pointer가 있는(없으면 가장 가까운) 줄에서 주축 중심이 앞인 형제 수를 셉니다. flex 가로·세로·wrap·reverse와 Grid 자동 배치(읽는 순서)가 같은 규칙입니다.
- 상자 원천(2026-11-24): 형제·컨테이너·끄는 node의 화면 상자는 layout 결과로 그린 scene source(`CanvasLayout::world_box`)에서 읽습니다. 흐름 자식의 화면 위치는 layout 결과이고 저장 transform은 그 뒤의 `translate`입니다(ADR-0125 결정 3).
- preview(2026-11-24): `FlowDrag::update`는 끄는 node를 삽입 자리(다음 자동 형제 바로 앞, 없으면 끝)에 둔 layout 자식 순서를 `CanvasLayout::reorder_preview`에 넘깁니다. 복사한 증분 layout을 다시 배치해 상자가 바뀐 node를 다시 변환하고, 새 흐름 순서의 world 상자 목록과 `ScenePreview`(그 node의 대체 입력, `IncrementalScene::set_preview`에 넘김)를 `FlowUpdate::Inside`로 냅니다. 크기가 다른 항목, `justify-content` 분배, wrap도 놓은 뒤 화면과 같습니다. 이전 permutation 근사(`reflow_preview`)는 지웠습니다.
- `FlowDrag`: 시작 때 한 번 형제·키를 읽고 update는 순수 계산(문서·history 불변). 놓으면 같은 컨테이너면 OrderKey만 바꾸는 `MoveNode` 하나, 다른 흐름 컨테이너면 그 삽입 index의 `MoveNode`, 자유 배치 Frame이면 기존 `MoveDrag`의 world 위치 보존 재부모화(ADR-0105)입니다. 같은 자리면 기록하지 않습니다. 잠금은 `commands::lock::edit_permission`으로 판정합니다.
- inset 이동(2026-11-24): `InsetDrag`는 layout record에 `position`을 명시해 Layout 규칙이고 풀린 `position`이 `absolute`인 node를 inset longhand `SetLayoutProperty`로 옮깁니다(저장 transform 불변). 축마다 현재 anchor(시작만 → 시작, 끝만 → 끝을 반대 방향, 둘 다 → 같이 옮겨 크기 유지, 둘 다 `auto` → 시작에 지금 위치 + 이동량 px)를 쓰고, 백분율 inset은 containing block 기준 백분율로 둡니다. world 이동량은 부모 world 선형 부분의 역으로 부모 local로 바꿉니다. 끄는 동안 `preview`는 node를 옮긴 `ScenePreview`이고 놓으면 한 transaction입니다.
- **C6 결정과 근거**: Grid 명시 배치 자식(줄 번호 `<integer>`가 있는 `grid-*-start/end`)은 drag로 순서를 바꾸지 않고 `grid-explicit-placement`로 거부하며, 자동 배치 형제의 삽입 index에서 뺍니다. 그 자식의 자리는 OrderKey가 아니라 줄 번호가 정해 OrderKey를 바꾸면 화면이 움직이지 않고, 줄 번호를 쓰려면 runtime scene에 없는 grid track 기하가 필요하기 때문입니다. 줄 번호는 CSS 값 입력으로 바꿉니다.
- 인스턴스 투영 행 위로 놓기·그 안 재정렬은 `instance-slot`으로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::flow_drag` | 통과(7): flex 가로·세로·reverse 삽입 index, wrap·Grid 자동 배치 index, preview 비키기, 50 update 뒤 놓기 = `MoveNode` 1건(history +1, 같은 자리 0건), 자유 Frame 끌어내기와 인스턴스 투영 행 거부, C6 명시 배치 거부와 자동 형제 index 제외, 자유 배치 자식은 흐름 drag 아님 |
| `cargo test -p stellar-runtime --lib canvas::flow` (2026-11-24) | 통과(9). flow_drag 6개: 형제 상자가 layout 결과(x 10·30·50), preview가 실제 layout(31·32 앞으로, 30 x 50)과 대체 입력 3건, history 1건. flow_inset 3개: 시작 anchor 이동과 history 1건·저장 transform 불변·이동 없음 0건, 끝 anchor·늘이기·백분율, 명시 absolute가 아닌 node 거부 |
| `cargo test -p stellar-scene --lib document::layout::tests::reorder_preview` (2026-11-24) | 통과: 순서를 바꾼 preview의 상자, 문서·원래 결과 불변, 형제 집합이 다르면 거부 |

## 차이(작업 지시와 다른 점)

- (2026-11-24로 해소) 이전에는 layout crate를 runtime에서 부를 수 없어 permutation reflow 근사로 preview를 냈습니다. 지금은 scene crate의 layout 결과(ADR-0125)로 냅니다. runtime은 layout crate에 직접 의존하지 않습니다.
- (2026-11-24로 해소) `position: absolute` 자식의 inset 쓰기를 `InsetDrag`로 더했습니다. 위치를 저장 transform으로 둔 옛 자유 배치 node는 여전히 기존 자유 배치 이동 경로(ADR-0105 결정 1)이고, IX.11.9 step이 그 위치를 inset으로 옮기면 `InsetDrag` 대상이 됩니다.
- 삽입 표시·preview 그리기는 셸 재설계 때 합니다. 코어는 표시 상자, preview 상자, scene 대체 입력을 냅니다.

## 남은 위험 / 한계

- 옛 캔버스 행 목록(`CanvasScene`)은 저장 transform으로 그립니다. 끌어내기 대상 판정(`MoveDrag`)과 인스턴스 투영 행 판정은 그 행 목록을 읽습니다. UI 재설계 때 `CanvasLayout` source로 바꿉니다.
- 셸 제스처 라우터가 없어 흐름 drag·inset drag·자유 이동 중 무엇을 고를지는 호출자가 정합니다(`FlowDrag::begin`의 `not-flow-child`, `InsetDrag::begin`의 `not-absolute`로 구분).
- `flow_insert_key`(생성 도구·붙여넣기)는 삽입 한 번마다 View를 변환합니다. 창 단위 `CanvasLayout` 보관은 UI 재설계 때 연결합니다.

## 근거 커밋

- `72512f81` Feat: 흐름 drag preview를 실제 증분 layout으로 만들고 absolute 자식 inset 이동 추가
- `61f00f59` Feat: scene 문서 변환이 layout 결과 위치·크기로 그리기(scene preview 함수)
