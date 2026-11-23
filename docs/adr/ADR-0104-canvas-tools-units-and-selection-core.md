# ADR-0104: 캔버스 도구·수식키·단위와 선택 엔진 코어

## Status

Accepted (2026-11-23). 계획 XII.10.2(가이드 열린 결정 O10)를 닫고, XII.10.1·XII.10.3\~XII.10.8·XII.10.40의 코어 구조를
정합니다.

## Context

- 계획 XII.10은 캔버스 편집 UX를 정합니다. 그 근거였던 UI 문서(캔버스 명세, 도구·메뉴 명세, 가이드)는 ADR-0098로
  지워졌습니다. 사용자 방향(2026-11-23)은 지금 엔진·기능만 만들고 UI는 나중에 처음부터 다시 설계하는 것입니다. 그래서
  이 ADR은 계획서 문구와 기존 ADR만으로 값을 정하고, 지워진 문서의 "가이드 12장 기록"은 이 ADR의 표로 대신합니다.
- 도구 목록은 계획 개정 기록(2026-11-15: Pen 도구 삭제 뒤 "도구 10종")과 행 문구(XII.10.26 생성 도구 다섯,
  XII.10.27 Image 도구, 캔버스 텍스트, Hand·Zoom)에서 정합니다.
- 기존 코어 부품: 창별 선택과 영역 선택 preview(`stellar_commands::selection`, VII.3), 창별 history와 pointer gesture
  preview(`stellar_history::window`, VII.4·VII.5), scene hit test(XI.3.3·XI.3.4), transient handle layer(XI.3.5),
  도구 상호작용 상태 기계(`stellar_scene::interaction`, XII.3.6), 잠금 판정(`stellar_commands::lock`).
- ADR-0071의 선택 규칙(`app::canvas_gesture`의 "지금 선택과 같은 깊이" 추정)은 지운 셸을 위한 경로였고 편집 범위를
  상태로 두지 않았습니다. ADR-0074는 캔버스 padding·gap 핸들을 두지 않기로 했습니다.

## Decision

### 1. 도구 10종과 일시 전환 (XII.10.1)

1. `stellar_runtime::canvas::tool::CanvasTool`은 닫힌 enum입니다: Move, Frame, Rectangle, Ellipse, Line, Polygon, Text,
   Image, Hand, Zoom. trait object나 열린 확장점은 없습니다.
2. 현재 도구는 창별 `CanvasToolState`에 둡니다. 저장·history 대상이 아닙니다.
3. 일시 전환: 제스처가 없을 때 Space를 누르는 동안 Hand입니다. 제스처 중에 누른 Space는 Hand로 바꾸지 않고
   제스처에 넘깁니다(생성 드래그의 시작점 이동). Hand 제스처 중에 Space를 떼면 제스처가 끝날 때 돌아갑니다. ⌘를
   누르는 동안 Move 도구는 깊은 선택입니다. 초점을 잃으면 수식키와 Space 상태를 지웁니다.
4. Escape는 Move가 아닌 도구를 Move로 돌립니다(Escape 전체 순서는 결정 3.5). 생성 도구는 생성을 마치면, Image 도구는
   파일 고르기를 취소하면 Move와 이전 도구로 돌아갑니다.
5. 도구 키(수식키 없이 한 번 누름, 키 반복 무시): V Move, F Frame, R Rectangle, O Ellipse, L Line, T Text, H Hand,
   Z Zoom, ⇧⌘K Image. Polygon은 단일 키가 없습니다(Pen을 지운 P 자리에 다른 도구를 두면 다른 도구 관례와 헷갈립니다).
   비례 확대(Scale) 명령의 단일 키는 K입니다(XII.10.12).

### 2. 이동·크기 단위와 수식키 (XII.10.2, 열린 결정 O10)

코어 상수는 `stellar_runtime::canvas::tool_units`에 있고 아래 표와 같은 값이어야 합니다(테스트가 이 표를 읽어
비교합니다).

| 항목 | 값 |
|---|---|
| 화살표 이동 | 1 px (부모 좌표 CSS px) |
| Shift-화살표 이동 | 10 px |
| 키보드 크기 조절 키 | ⌘-화살표: → ↓는 오른쪽·아래 변을 늘리고 ← ↑는 줄임 |
| 키보드 크기 조절 단위 | ⌘ 1 px, ⌘⇧ 10 px |
| 끌기 시작 거리 | 3 pt (화면) |
| 스냅 허용 오차 | 5 pt (화면) |
| 줌 하한 | 2% |
| 줌 상한 | 25600% |
| 줌 단계 | 2, 5, 10, 25, 50, 100, 200, 400, 800, 1600, 3200, 6400, 12800, 25600 (%) |
| 픽셀 그리드 표시 | 800% 이상 |
| 크기·반경 핸들 숨김 | 선택 상자 화면 짧은 변 8 pt 미만 |
| 변 핸들 숨김 | 선택 상자 화면 짧은 변 24 pt 미만 |
| 반경 핸들 표시 | 단일 선택이고 화면 짧은 변 40 pt 이상 |
| 모서리 핸들 반변 | 4 pt |
| 변 핸들 반폭 | 4 pt |
| 회전 영역 반지름 | 16 pt |
| 반경 핸들 반지름 | 5 pt |
| 반경 핸들 안쪽 거리 | 12 pt |

수식키 의미:

| 수식키 | 누름(클릭·마퀴 시작) | 이동 드래그 | 크기 조절 | 회전 | 생성 드래그 |
|---|---|---|---|---|---|
| Shift | 선택 토글(마퀴는 더하기) | 축 고정 | 비율 유지 | 15도 단위 | 정비례·45도 |
| Option | 다중 선택의 선택된 node는 primary 지정, 마퀴는 완전 포함 | 시작 때 누르면 복제 | 중심 기준 | 없음 | 중심 기준 |
| ⌘ | 깊은 선택(클릭·마퀴) | 누르는 동안 재부모화 방지 | 없음 | 없음 | 없음 |
| Space | 제스처가 없으면 누르는 동안 Hand | 없음 | 없음 | 없음 | 시작점 이동 |

- 재부모화 방지 수식키는 ⌘입니다. 깊은 선택(⌘-클릭) 뒤 그대로 끌면 부모를 지키는 쪽이 의도와 맞습니다.
- 줌 하한·상한은 기존 viewport(`view_tabs::MIN_ZOOM`·`MAX_ZOOM`)와 상호작용 기본값(`InteractionConfig`)을 그대로
  씁니다. 끌기 시작 거리도 `InteractionConfig::DEFAULT_DRAG_THRESHOLD`를 씁니다. 같은 값을 다른 곳에 두지 않습니다.
- ADR-0071 UX.3.7의 "XII.10.2 ADR 전 셸 임시 화살표 단위"는 이 표의 값으로 대체합니다.

### 3. 선택 엔진 (XII.10.3\~XII.10.7)

1. **상태 위치.** 편집 범위(`EditingScope`)와 hover(`HoverState`)는 도구 상태와 함께 runtime 창별 캔버스 상태에
   둡니다. 계획 문구는 `WindowViewState`라고 썼지만 그 타입은 history가 소유하는 commands crate의 선택 상태이고, 도구·
   편집 범위·hover는 history와 무관한 캔버스 개념입니다. commands crate에 캔버스 UI 개념을 넣지 않고, 확정 선택만
   지금처럼 `WindowViewState`에 둡니다. 창 session(`ProjectWindowSession`)이 이 상태를 소유하게 잇는 일은 셸 연결과
   함께 합니다. 창 하나의 입력(pointer down·move·up·cancel, 키, 수식키, 초점)은 `canvas::select_session::
   CanvasSelectSession`이 이 상태와 아래 순수 함수로 차례로 처리하고, 끄는 동안의 문서 편집은 호출자가 그 session의
   preview로 넣습니다.
2. **hit 입력.** 선택 함수는 `PointStack`(커서 아래 node, 위에 그린 것부터, 숨김 제외, 잠금 표시 포함)을 받습니다.
   잠기지 않은 node는 narrow-phase hit test(XI.3.4)의 결과이고, 잠긴 node(scene `pointer-events: none`)는 같은 공간
   인덱스 후보를 local hit bounds와 clip으로 판정합니다(겹친 레이어 목록 전용, broad-phase 정밀도). 인스턴스 안 scene
   node의 source는 가장 바깥 인스턴스라 인스턴스는 한 덩어리로 맞습니다.
3. **클릭.** 커서 아래 가장 위의 잠기지 않은 node를 leaf라 합니다. leaf가 View root이거나 없으면 빈 곳입니다. leaf가
   편집 범위 S 또는 S의 조상이면 leaf 자신을 고르고 범위는 leaf의 부모입니다. 아니면 leaf 조상 사슬에서 처음 만나는
   S의 조상(자신 포함) A를 새 범위로 하고 A의 직계 자식을 고릅니다. 그래서 S 안의 클릭은 S의 직계 자식(Fragment는
   묶음, 인스턴스는 한 덩어리)이고, S 밖 클릭은 가장 가까운 공통 조상으로 범위를 맞춥니다. ⌘는 leaf 자신(선택할 수
   없으면 가장 가까운 선택 가능 조상)을 고르고 범위는 그 부모입니다. Shift는 토글(빈 곳은 그대로), Option은 다중
   선택에서 이미 선택된 node를 primary로 지정합니다. 빈 곳 클릭은 선택을 비우고 범위를 View root로 돌립니다. ⌘A는 범위의
   선택 가능 자식 전체입니다. 결과는 `SetSelection` 하나(바뀌지 않으면 없음)입니다.
4. **깊은 선택과 범위 이동.** 더블클릭은 고른 node가 이미 선택된 컨테이너(Frame, Fragment)이면 그 안으로 들어가 커서
   아래 직계 자식을 고르고(빈 곳이면 선택 없이 범위만 들어감), Return은 선택한 컨테이너의 선택 가능 자식 전체를
   고릅니다. Text·Path에서는 내용 편집 요청(`EditContent`)을 돌려주고 XII.10.29·XII.10.33이 받습니다. Shift-Return은
   부모, Tab·Shift-Tab은 primary 부모의 선택 가능 자식을 OrderKey 순으로 순환합니다(선택이 없으면 범위의 첫·마지막
   자식). Shift-더블클릭은 깊은 node를 더합니다. 사이드바 등 밖에서 선택이 바뀌면 범위를 선택한 node 부모들의 가장
   가까운 공통 조상으로 맞춥니다. 범위 변경은 저장·history에 없습니다.
5. **Escape 순서.** 진행 중 제스처 취소 → Move가 아닌 도구에서 Move로 → 범위가 View root가 아니면 범위 node를 고르고
   범위를 그 부모로 → View root 범위에서 선택이 있으면 선택 해제 → 아무것도 하지 않음.
6. **select-through (XII.10.5).** 누른 점이 현재 선택 bounds(선택 node world bounds의 합) 안이면 선택을 그대로 두고,
   끌면 선택을 옮기며 움직이지 않고 놓으면 그 점을 클릭 규칙으로 고릅니다. 밖이면서 고를 node가 있으면 누름에서
   선택을 바꾸고(기록은 제스처 끝에, 결정 5) 끌면 그 선택을 옮깁니다. 고를 node가 없으면 마퀴입니다. 겹친 레이어
   목록(Select Overlapping)은 `PointStack` 전체(잠금 포함, 위에 그린 것부터)입니다.
7. **마퀴 (XII.10.6).** 후보는 시작 때 한 번 만듭니다. 기본은 범위의 선택 가능 직계 자식, ⌘는 범위 아래 선택 가능
   자식이 없는 가장 깊은 node입니다. 기본은 영역과 겹침(회전 node는 world 사각형으로 분리축 판정), Option은 완전 포함,
   Shift는 시작 전 선택에 더하기(`AreaSelectionMode::Add`)입니다. 진행 중 결과는 `WindowViewState` 영역 선택 preview이고
   pointer up에서 `SetSelection` 하나만 만듭니다.
8. **hover (XII.10.7).** hover 대상은 같은 입력으로 클릭이 고를 node(클릭 규칙의 고를 node 계산을 그대로 씀)입니다.
   캔버스 hover와 사이드바 행 hover는 같은 `HoverState` 하나를 바꿉니다. hover 윤곽은 world 사각형 transient 값이며
   history·저장에 없습니다.

### 4. 변형 핸들 hit layer (XII.10.8)

1. 우선순위는 반경 > 크기 조절 모서리 > 크기 조절 변 > 회전 > 본체입니다. ADR-0074 결정 3으로 padding·gap 핸들 자리는
   없습니다. 반경 핸들은 XI.3.5 layer의 modifier control point(가장 높은 우선순위)로 넣어 같은 hit 규칙을 씁니다.
2. 핸들 위치는 node local 사각형 꼭짓점을 world로 옮긴 값이라 회전한 node도 자기 축을 따릅니다. 반경 핸들은 local
   모서리에서 안쪽으로 `max(실효 반경, 안쪽 거리)`만큼(화면 pt를 local 단위로 바꿔) 들어간 점이며 변 길이 절반을
   넘지 않습니다. 반경 핸들은 Frame·Rectangle·Polygon 단일 선택에만 있습니다.
3. 숨김 기준은 결정 2 표입니다. 크기 조절할 수 없는 축(길이 0)의 핸들은 XI.3.5처럼 만들지 않습니다.
4. 핸들 layer는 scene snapshot·저장과 무관한 transient 값입니다.

### 5. 한 제스처 한 history (XII.10.40)

1. 규칙: 제스처가 문서 항목을 만들면 그 항목의 `SelectionRestore`가 제스처 전·후 선택을 담고 Selection 항목은 만들지
   않습니다. 편집 없이 끝나고 선택이 바뀌었으면 Selection 항목 하나를 기록합니다. 취소하면 제스처 전 선택으로 돌리고
   기록하지 않습니다. `HistoryEntry` 타입은 바꾸지 않습니다.
2. 구현은 runtime `canvas::select_gesture::GestureRecorder`가 `WindowHistory` 공개 API(`begin_pointer_gesture`,
   `update_preview`, `end_preview`, `cancel_preview`, `record_selection`)와 `WindowViewState::restore_selection`으로
   합니다. 계획 문구의 "`WindowHistory`에 API 추가"와 같은 의미를 history crate를 바꾸지 않고 냅니다. 제스처 중 선택
   반영은 이 recorder만 합니다.
3. 제스처별 history 항목 수:

| 제스처 | 문서 항목 | 선택 항목 |
|---|---|---|
| 움직이지 않은 클릭(선택이 바뀜) | 0 | 1 |
| 움직이지 않은 클릭(선택 그대로) | 0 | 0 |
| 선택하지 않은 node 즉시 드래그 | 1(제스처 전·후 선택 포함) | 0 |
| 선택한 node 드래그 | 1 | 0 |
| 마퀴(선택이 바뀜) | 0 | 1 |
| 취소한 제스처 | 0 | 0 |
| hover, camera 팬·줌, 도구 전환, 범위만 바뀌는 이동 | 0 | 0 |
| Escape 선택 해제, 더블클릭·Return·Tab 선택 이동 | 0 | 1 |

### 6. 기존 경로

`app::canvas_gesture`의 ADR-0071 선택 경로는 지운 셸의 C ABI 뒤에 있는 옛 규칙입니다. 새 UI는 이 ADR의
`canvas::select` 경로에 연결하고, 그 연결 때 옛 선택 경로를 지웁니다. 그 전까지 두 경로가 있지만 셸이 없어 옛 경로를
부르는 화면은 없습니다.

## Alternatives

- 도구·편집 범위를 commands `WindowViewState`에 둠: history가 소유하는 타입에 history와 무관한 캔버스 상태가 섞이고,
  commands crate가 캔버스 UI 개념을 갖게 되어 채택하지 않았습니다.
- 편집 범위 없이 "지금 선택과 같은 깊이"로 추정(ADR-0071): 선택이 비면 깊이를 잃고 Escape·Tab 규칙을 정할 수 없어
  채택하지 않았습니다.
- 재부모화 방지를 Space로: Space는 제스처 밖 일시 Hand와 생성 드래그 시작점 이동에 이미 쓰여 의미가 겹칩니다.
- 화살표 단위를 줌에 따라 바꿈: CSS px 값과 화면 결과가 줌마다 달라져 실제 구현에 가까운 값이라는 원칙과 어긋납니다.

## Consequences

- 선택 규칙이 runtime 순수 함수로 모여 CLI·MCP·새 UI가 같은 규칙을 씁니다.
- 셸이 생기기 전까지 창 session에 캔버스 상태를 붙이는 연결은 남아 있습니다.
- 잠긴 node의 겹친 레이어 목록 판정은 broad-phase 정밀도입니다.

## Compatibility

파일 포맷·FFI·CLI·MCP 영향 없음. 도구·범위·hover·핸들 상태는 저장되지 않습니다.

## Performance

마퀴 후보와 bounds는 제스처 시작 때 한 번 만들고 update는 후보 수에 선형이며, hit가 직전과 같으면 preview를 다시
만들지 않습니다. hover는 hit test 한 번과 조상 사슬 계산입니다. 2만 가시 node 기준 p99 8ms(목표 5ms)를 release 측정
테스트로 확인합니다. 마퀴 preview 갱신은 `WindowViewState::update_area_selection`이 만드는 `Selection::new`의 중복 제거가
원소 수의 제곱 비용이라, 영역이 1만 개 가까이 덮으면 갱신 한 번이 8ms를 넘습니다(이 ADR의 후보 판정은 p99 1ms 미만).
그 중복 제거를 정렬·hash 기반으로 바꾸는 일은 commands crate 선택 모델의 후속 작업입니다.

## Security

영향 없음.

## Migration

없음. ADR-0071의 셸 임시 화살표 단위와 선택 추정은 셸 재설계 때 이 ADR 경로로 바꿉니다.
