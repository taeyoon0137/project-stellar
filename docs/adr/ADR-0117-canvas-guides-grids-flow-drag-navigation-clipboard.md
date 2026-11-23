# ADR-0117: 눈금자 가이드·레이아웃 그리드 저장 위치와 캔버스 편집 엔진 2의 코어 규칙

## Status

Accepted (2026-11-23). 계획 XII.10.18(가이드·그리드 저장 위치 ADR)을 닫고, XII.10.17·XII.10.19·XII.10.20·XII.10.24·
XII.10.26·XII.10.29·XII.10.36·XII.10.37·XII.10.38·XII.10.43의 코어 부분(셸 UI·FFI 제외)을 정합니다. 캔버스 명세 열린 결정
C1(눈금자 표현)과 C6(Grid 명시 배치 자식 drag)을 이 ADR에서 닫습니다.

## Context

- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만들고 Rust 테스트·CLI·MCP로 검증합니다. 셸 UI는 지워졌고(ADR-0098)
  셸 FFI(`crates/ffi`, `stellar_ffi.h`)는 이번 작업에서 더하지 않습니다. 계획서 행이 가리키는 캔버스 명세·도구 명세는
  지워졌으므로 계획서 문구와 ADR-0074·0082·0086·0104·0105로 판단합니다.
- 앞선 코어: 도구 상태 기계와 단위(ADR-0104), 스냅 엔진·변형·흐름 자식 판정(ADR-0105, `stellar_commands::arrange::align::
  flow_parent`), 창별 viewport(`stellar_commands::view_tabs::CanvasViewport`), 같은 문서 subtree 클립보드
  (`stellar_commands::paste::ClipboardSubtree`), Path point 편집 command(`EditPathPoint`, ADR-0079).
- 스냅 엔진은 가이드·그리드 선을 호출자가 넘기는 선 목록으로만 받습니다(ADR-0105 결정 3). 그 선의 저장 위치가
  없었습니다.
- crate 그래프: runtime·commands는 layout crate에 의존할 수 없습니다(ADR-0105). runtime은 diagnostics에 의존하지
  않습니다(ADR-0084 결정 8).
- 이 작업의 소유 범위는 `crates/runtime/src/canvas/**`, `crates/runtime/src/input*`, `crates/commands/src/selection*`,
  `crates/document/src/guides*`입니다. 문서 모델·저장·command 목록·Semantic Diff는 소유 밖입니다.

## Decision

### 1. 스냅·표시 토글과 일시 해제 (XII.10.17)

1. 토글은 창 session 상태 `stellar_runtime::canvas::view_options::CanvasViewOptions`입니다. 문서·Git·history에 넣지
   않습니다. 새 창의 초기값은 앱 설정이 정하고(셸이 넘김) 코어 기본값은 모두 켬(눈금자만 끔)입니다.

| 토글 | 공유 이름 | 기본 |
|---|---|---|
| 객체 스냅 | `view.snap_to_objects` | 켬 |
| 픽셀 그리드 스냅 | `view.snap_to_pixel_grid` | 켬 |
| 스마트 가이드 표시 | `view.show_smart_guides` | 켬 |
| 눈금자 표시(⌘R) | `view.show_rulers` | 끔 |
| 레이아웃 그리드 표시 | `view.show_layout_grids` | 켬 |
| 픽셀 그리드 표시 | `view.show_pixel_grid` | 켬 |

2. **일시 해제 수식키는 Control(⌃)입니다.** 끄는 동안 Control을 누르고 있으면 그 update는 가이드·그리드·객체·픽셀
   스냅을 모두 하지 않고(스냅 결과 없음) 스마트 가이드도 그리지 않습니다. ADR-0104 결정 2 표에서 Shift·Option·⌘·Space가
   이미 쓰였고, Control은 캔버스 제스처에 의미가 없습니다. 판정 함수는 `CanvasViewOptions::snapping`입니다.
3. View 메뉴 `ShellCommand` 등록과 `ShortcutPolicy` 검사는 셸 재설계 때 합니다(ADR-0098). 공유 이름은 위 표가
   원본입니다.

### 2. 가이드·레이아웃 그리드 저장 위치 (XII.10.18)

1. **둘 다 문서 데이터입니다.** 가이드와 레이아웃 그리드는 설계 기준선이며 협업자·에이전트(CLI·MCP)가 같은 값을 봐야
   합니다. 창별 view 상태에 두면 다른 장비·다른 사람에게 보이지 않고, Git으로 공유되지 않아 실제 구현 handoff(12열
   그리드 같은 기준)를 잃습니다. 문서 데이터로 두는 비용(diff 잡음)은 Semantic Diff가 별도 범주로 보여 주는 것으로
   줄입니다.
2. **소유자는 Frame node입니다.** 가이드는 View root Frame(캔버스 가이드, View root 좌표) 또는 최상위 Frame(그 Frame
   local 좌표)에 속합니다. 레이아웃 그리드는 어느 Frame에나 붙습니다. ViewDefinition에 두지 않는 이유: 최상위 Frame을
   옮기면 그 Frame의 가이드도 같이 움직여야 하고, 같은 Frame을 다른 View에서 봐도 같은 기준선이어야 합니다.
3. 값 모델은 `stellar_document::guides`입니다.
   - `RulerGuide { id: StableId, axis: x|y, position: f32 }` — `axis: x`는 세로선(x 좌표), `y`는 가로선. 위치는 소유
     Frame local CSS px이고 유한값, 절댓값 1e7 이하입니다.
   - `LayoutGrid { kind: columns|rows|square, count, size, gap, margin, align, visible }` — CSS 이름을 씁니다.
     `count`(1–1000, `square`는 없음), `size`(`auto` = 남는 폭을 나눠 늘림, 또는 px; `square`는 칸 한 변 px),
     `gap`(트랙 사이 px), `margin`(정렬 기준 가장자리에서의 바깥 여백 px, `center`는 쓰지 않음), `align`(`stretch`|`start`|
     `center`|`end`, CSS `justify-content`/`align-content` 키워드). 레이아웃 그리드는 설계 기준이라 자식 배치(flex·grid
     layout 입력)에 들어가지 않습니다(ADR-0074: 비CSS 배치 개념을 만들지 않음).
   - `NodeGuides { owner, guides, grids }` — Frame 하나의 기록. guide는 id 순서 무관 집합(직렬화는 id 오름차순),
     grid는 순서 있는 목록(그리는 순서)입니다. owner 하나당 guide 1,024개, grid 16개가 상한입니다.
   - 편집은 `GuideEdit`(Add·Move·Remove·SetGrids)이고 `NodeGuides::apply`가 역 편집을 돌려줍니다. 이 값이 그대로
     history command의 정·역이 됩니다.
   - canonical bytes(record v1)와 텍스트 원본 표기(`guide x 120px <id>`, `grid columns 12 auto gap 20px margin 32px
     stretch`)는 `stellar_document::guides`가 정합니다. 텍스트 원본 한 줄의 값은 CSS 길이 formatter
     (`css::value`)를 씁니다.
4. **연결(이 작업 범위 밖, 후속 행).** 문서 모델 필드(`DocumentModel`이 owner별 `NodeGuides`를 들고 node 삭제 시 함께
   지우고 undo로 되살림), command `SetNodeGuides { owner, edit }`(역은 `apply`의 역 편집), 저장 chunk kind·record kind
   예약(`core-types::reserved`, 보호 경로이므로 이 ADR 개정 절과 함께), 텍스트 원본 직렬화 위치(소유 Frame의 node
   텍스트 원본 안 `guides` 블록), Semantic Diff 범주 `guides`(guide id별 추가·이동·삭제, owner별 grid 목록 값 하나),
   Semantic Merge 규칙(다른 guide 동시 편집은 충돌 없음, 같은 guide를 양쪽에서 옮기거나 한쪽 이동·한쪽 삭제는 충돌, grid
   목록은 양쪽이 바꾸면 값 충돌)입니다. CLI·MCP는 같은 command를 씁니다.
5. **Migration.** 새 기록은 선택 값입니다. 기록이 없으면 가이드·그리드가 없는 것이고 기존 파일은 그대로 읽습니다. 새
   chunk를 모르는 옛 앱은 알 수 없는 chunk 규칙(조용히 버리지 않고 거부)을 따릅니다.

### 3. 눈금자와 가이드 (XII.10.19)

1. **C1: 눈금자는 셸이 화면 고정 띠로 그리고, 눈금 값은 코어가 camera와 같은 viewport에서 계산합니다.** 코어
   `canvas::ruler::ruler_ticks`가 viewport·화면 크기·원점으로 큰 눈금(라벨 있음)과 작은 눈금의 화면 좌표를 냅니다.
   scene layer로 그리지 않는 이유: 눈금자는 camera와 함께 움직이지 않는 화면 크롬이고, 같은 viewport 값에서 계산하면
   camera와 어긋날 수 없습니다(fixture가 world→view 변환과 같은 값을 확인). 라벨 글꼴은 셸 시스템 UI 글꼴입니다
   (ADR-0105 C2와 같음).
2. 큰 눈금 간격은 1·2·5×10ⁿ CSS px 중 화면 간격이 64pt 이상인 가장 작은 값이고, 작은 눈금은 큰 간격을 5등분(2 계열은
   4등분)합니다. 라벨은 원점 기준 CSS px 정수(소수 배율에서는 필요한 만큼의 소수) 표기입니다.
3. 눈금 원점은 primary 선택(없으면 첫 선택)의 최상위 Frame(View root의 직계 Frame 조상) local 원점이고, 선택이 없거나
   최상위 Frame이 없으면 View root 원점입니다. 최상위 Frame이 회전·기울어져 있으면 축이 맞지 않으므로 View root
   원점을 씁니다.
4. 가이드 제스처(`canvas::ruler::GuideDrag`): 눈금자에서 누르고 끌면 새 가이드 preview, 캔버스에서 놓으면 `Add`, 다시
   눈금자 위에서 놓으면 취소. 기존 가이드 선 위(화면 3pt 안)를 누르고 끌면 `Move`, 눈금자 위에서 놓으면 `Remove`.
   새 가이드의 소유자는 pointer 아래 최상위 Frame(회전 없음)이고 없으면 View root입니다. 끄는 동안 위치는 픽셀
   스냅(정수 소유자 px)을 따르고 Control이 일시 해제합니다. Escape는 취소이고 편집이 없습니다.
5. 가이드 선은 스냅 엔진의 `SnapLines::guides`로 들어갑니다(`canvas::ruler::guide_snap_lines`: 소유자 local → 대상 부모
   local, 축 정렬 변환에서만). 가이드 편집 한 제스처는 history 한 항목입니다(결정 2.4의 command 연결 뒤).

### 4. 레이아웃 그리드와 픽셀 그리드 (XII.10.20)

1. 트랙 계산은 `stellar_document::guides::LayoutGrid::tracks(width, height)` 하나입니다. `auto` 크기는 정렬과 관계없이
   늘림이고, 양쪽 `margin`을 뺀 길이에서 `gap`을 빼고 `count`로 나눕니다. 고정 `size`는 `align`에 따라 시작·가운데·끝에 놓습니다(`stretch`+고정
   `size`는 `start`처럼 margin 뒤부터). `square`는 원점부터 `size`+`gap` 간격으로 상자를 채웁니다.
2. 스냅 선은 트랙의 시작·끝 가장자리입니다(`canvas::layout_grid::grid_snap_lines`). 표시 여부(`visible`, 토글)와
   관계없이 스냅은 설정(객체 스냅이 아니라 그리드 선 후보 자체)을 따르며, Control 일시 해제는 따릅니다.
3. 픽셀 그리드 overlay는 배율이 ADR-0104 표의 800% 이상이고 토글이 켜졌을 때만 가시 world 영역의 정수 좌표 선을
   화면 좌표로 냅니다(`canvas::layout_grid::pixel_grid_lines`). 선 수는 화면 크기/배율에 비례해 유계입니다.
4. 레이아웃 그리드는 문서 layout 입력에 없으므로 그리드 유무로 scene·layout 결과가 바뀌지 않습니다.

### 5. 흐름 컨테이너 안 드래그 (XII.10.24)

1. 대상은 `flow_parent`가 `Flex`·`Grid`인 흐름 자식입니다. 삽입 위치(`canvas::flow_drag::insertion_index`)는 끄는 node를
   뺀 형제들의 화면 상자(게시된 scene)를 흐름 순서(OrderKey)로 보고, 교차축으로 겹치는 상자를 한 줄로 묶은 뒤
   pointer가 있는 줄에서 주축 중심이 pointer보다 앞인 형제 수를 셉니다. `flex-wrap`의 여러 줄과 Grid 자동 배치(읽는
   순서 행)를 같은 규칙으로 다룹니다. `*-reverse`는 주축 비교를 뒤집고 `wrap-reverse`는 줄 순서를 뒤집습니다.
2. 증분 layout preview는 흐름 순서 permutation reflow입니다: 끄는 node를 새 자리에 넣은 순서로 같은 줄의 원래 칸
   시작점과 관측 간격을 다시 나눠 형제의 preview 상자를 냅니다. runtime은 layout crate를 부를 수 없어 정확한 CSS
   layout이 아니라 같은 크기 항목에서 정확하고 크기가 다르면 근사입니다. 놓은 뒤 화면은 실제 layout이 정합니다.
3. 놓으면 OrderKey를 바꾸는 `MoveNode` 하나(같은 부모, 이웃 사이 key)를 history 한 항목으로 기록합니다. 같은 자리면
   기록하지 않습니다.
4. 흐름 컨테이너 밖으로 끌어내면 대상 부모 규칙으로 옮깁니다(자유 배치 Frame이면 ADR-0105 결정 2의 world 위치 보존
   재부모화). `position: absolute` 자식은 흐름 자식이 아니라 기존 자유 배치 이동 경로를 씁니다. 그 저장 위치는 ADR-0105
   결정 1(IX.11.9 step 등록 전에는 `transform`)을 따르며, inset longhand로 쓰는 일은 IX.11.9 step과 함께 합니다.
5. ComponentInstance 내부(인스턴스 투영 행 안) 재정렬은 거부합니다(`instance-slot`).
6. **C6: Grid 명시 배치 자식(`grid-column-start`·`grid-row-start` 중 하나라도 `auto`가 아님)은 캔버스 drag로 순서를
   바꾸지 않고 거부합니다(`grid-explicit-placement`).** 그 자식의 자리는 OrderKey가 아니라 줄 번호 값이 정하므로
   OrderKey를 바꾸면 화면이 움직이지 않고, 줄 번호를 쓰려면 grid track 기하가 필요한데 runtime scene에는 없습니다.
   줄 번호는 속성 값 입력(CSS 값 그대로)으로 바꿉니다. 자동 배치 형제의 삽입 index 계산에서는 명시 배치 자식을
   건너뜁니다.

### 6. 생성 도구 (XII.10.26)

1. `canvas::create::CreateGesture`는 Frame·Rectangle·Ellipse·Line·Polygon 생성 드래그의 순수 계산입니다. 좌표는 대상
   부모 local CSS px입니다. Shift: 상자 도구는 정비례(긴 변 기준), Line은 45도 단위. Option: 시작점이 중심. 드래그 중
   Space를 누르는 동안: 상자 크기를 그대로 두고 시작점을 pointer와 함께 옮깁니다.
2. 클릭(끌기 시작 거리 3pt 미만)은 기본 크기 100×100 px(Line은 길이 100px 가로선)을 클릭 점에 왼쪽 위(Option이면
   중심)로 만듭니다.
3. 대상 부모는 pointer 아래 가장 깊은 잠기지 않은 Frame(인스턴스 투영 행 제외), 없으면 View root입니다. 흐름
   컨테이너면 결정 5.1의 삽입 index로 OrderKey를 정하고 위치(Frame `transform`, Transform modifier)를 쓰지 않습니다.
4. 칠은 셸이 넘기는 앱 설정 값(`ShapePaint`, ADR-0071 D7)이고 코어는 기본 색을 두지 않습니다. Frame 배경은
   `ShapePaint::fill`입니다.
5. `InsertNode`와 새 선택을 한 history 항목으로 기록하고(`insert_and_select`), 도구 기계에
   `ToolInput::CreationFinished`를 넣어 Move로 돌아갑니다.
6. 앞선 셸 경로 `app::frame_insert`(ADR-0071)는 같은 저장 모양(Transform → Fill → Stroke modifier 순서)을 씁니다. 새
   UI는 이 ADR의 `canvas::create`에 연결하고 그때 옛 경로를 지웁니다(ADR-0104 결정 6과 같은 처리).

### 7. Path 벡터 편집 모드 (XII.10.29)

1. `canvas::path_edit::VectorEditMode`는 창별 상태이고 저장·history 대상이 아닙니다. Path node에서 Return·더블클릭
   (`Navigation::EditContent`)으로 들어가고 Escape로 나옵니다. Shape는 들어가지 않습니다(`not-a-path`). Shape를 Path로
   바꾸는 일은 명시적 Convert to Path(VII.2.7)뿐입니다. 분기형 vector network는 없습니다(문서 Path 모델 그대로).
2. point·handle hit는 node world 변환으로 옮긴 화면 거리(반지름 `RADIUS_HANDLE_PT`)입니다. point 드래그 update는 순수
   계산이고, pointer up에서 `EditPathPoint { MovePoint }` 하나를 history 한 항목으로 기록합니다. 취소는 0건입니다.

### 8. 확대 제스처 입력과 줌·팬 (XII.10.36, XII.10.37)

1. runtime 캔버스 입력 모델 `stellar_runtime::input_gesture::NavigationInput`에 확대 제스처(`Magnify { phase,
   magnification, location }`, phase는 began·changed·ended·cancelled)와 `SmartMagnify { location }`, 스크롤(`Scroll`,
   precise·phase·momentum phase·수식키)을 둡니다. 셸 `StellarMacEvent`·FFI `InputEventKind` 확장과 version 올림(ADR-0014
   절)은 FFI를 건드리지 않는 이번 방향 때문에 셸 재설계 때 합니다. 그때 FFI 변환 결과를 이 타입으로 옮깁니다.
2. camera 연결은 `canvas::navigate::Navigator` 하나이고 `CanvasViewport`만 바꿉니다(문서 geometry·history 불변).
   - 핀치: 배율 `× (1 + magnification)`, 커서 기준 연속 줌.
   - 정밀 스크롤(트랙패드 두 손가락): 팬. 관성(momentum) phase도 같은 팬이고 사용자가 새 제스처를 시작하면 남은
     관성 event를 버립니다.
   - ⌘+스크롤: 커서 기준 줌, 배율 `× 2^(−dy/100)`(정밀) 또는 한 칸당 `× 2^(∓1/4)`(마우스 휠).
   - 마우스(정밀 아님) Shift+휠: 세로 값을 가로 팬으로.
   - Space 드래그·Hand 도구: pointer 화면 이동만큼 팬.
   - Zoom 도구: 클릭은 다음 줌 단계(ADR-0104 표), Option-클릭은 이전 단계(커서 기준), 영역 드래그(3pt 이상)는 그 world
     영역을 화면에 맞춤.
   - smart magnify: 선택 world 경계를 화면에 맞춤(여백 화면 짧은 변의 5%)과 이전 viewport 복귀를 번갈아 합니다. 선택이
     없으면 배율 100%와 이전 viewport를 번갈아 합니다.
   - 배율은 viewport 상하한(2%–25600%)으로 맞춥니다.

### 9. 캔버스 클립보드 (XII.10.38)

1. pasteboard 형식은 두 가지입니다. Stellar 전용 type `app.stellar.canvas-subtree.v1`(bytes = `canvas::clipboard::
   CanvasClipboard` 봉투: 형식 header, 출처 프로젝트·문서·View id, 선택 root별 `ClipboardSubtree` bytes와 world 상자,
   참조한 Component 정의·asset id 목록)과 일반 텍스트(선택 subtree의 Text node 내용을 문서 순서로 줄바꿈 이어 붙임,
   Text가 없으면 쓰지 않음)입니다. 플랫폼 pasteboard 접근은 셸이 합니다.
2. Copy는 다른 선택의 자손을 뺀 root들(선택 순서)을 담고 잠긴 node도 복사합니다. Cut은 Copy 뒤 잠기지 않은 root의
   `DeleteSubtree`를 한 항목으로 기록합니다(잠긴 root가 있으면 Cut 전체 거부).
3. Paste 위치: (1) 선택이 자식을 가질 수 있는 잠기지 않은 Frame 하나면 그 안(원래 local 위치 유지), (2) 아니면 같은
   문서의 같은 View이고 원본 world 상자 합이 지금 화면에 보이면 원래 부모 아래 원래 위치(원래 부모가 없으면 View
   root), (3) 아니면 View root 아래 화면 가운데(상자 합의 중심이 화면 중심)입니다. 흐름 컨테이너 안이면 마지막 흐름
   순서에 넣습니다. 새 ID는 `ClipboardSubtree::plan_with`(IdRemapTable)로 받고, 붙여 넣은 root 전체를 새 선택으로 한
   history 항목에 기록합니다.
4. 다른 문서·다른 프로젝트에서 온 봉투: 지금 `ClipboardSubtree`는 같은 문서 subtree만 받습니다. 붙여넣기 전에
   진단을 만들어 돌려줍니다 — 대상 프로젝트에 없는 Component 정의를 참조하는 인스턴스(`component-missing`), 대상에
   없는 asset(`asset-missing`), 그리고 문서가 다르면 `cross-document`입니다. 규칙: 다른 문서 봉투는 `cross-document`로
   거부하고 일반 텍스트 형식이 있으면 셸이 그 텍스트를 Text node로 붙일 수 있습니다. 다른 문서 subtree 붙여넣기
   (참조 재연결, asset 복사)는 commands `ClipboardSubtree` v2와 asset import(XIII.5.2)가 생기면 이 진단 목록을 그대로
   써서 엽니다.

### 10. 캔버스 편집 성능 replay (XII.10.43)

1. `canvas::edit_replay`는 2만 가시 node fixture에서 hover, 클릭 선택, 마퀴, 스냅 포함 이동, flex 컨테이너 안 크기
   조절, 회전, Path point 드래그 scenario를 각 1,000 sample 이상 재생하고 결과 JSON(`stellar.canvas-edit-replay.v1`:
   scenario, sample 수, p50·p99·max µs, 목표·상한, 판정)을 냅니다. hover·클릭 선택·핸들 표시는 목표 5ms·상한 8ms(p99),
   나머지는 Present 예약 10ms 이하(p99)로 판정합니다. 측정 구간은 입력 하나의 코어 처리(hit·선택·preview 계산·확정
   command 계획)입니다.
2. IME 조합 입력 scenario는 Text 편집 경로가 scene 입력에 이어지지 않아(ADR-0096 결정 3과 같은 이유) 결과에
   `not_connected`로 남기고 판정에서 빼지 않습니다(그 scenario는 통과가 아님).
3. 기준 장비 판정과 xtask analyzer(`stellar.performance-result.v2`) 연결은 XVI.2.3·XVI.4.2 경로에서 합니다. 이 결과는
   개발 장비 측정 기록입니다.

### 11. 선택 중복 제거

`stellar_commands::selection::Selection::new`의 중복 제거를 hash 집합으로, `contains`를 정렬 사본의 이분 탐색으로
바꿨습니다. ADR-0104 Performance가 남긴 마퀴 preview 갱신 제곱 비용 문제를 닫습니다.

## Alternatives

- 가이드·그리드를 창별 view 상태로: Git·협업자·에이전트가 같은 기준선을 볼 수 없고 장비를 바꾸면 사라져 채택하지
  않았습니다.
- 가이드를 ViewDefinition에: 최상위 Frame을 옮기면 Frame 기준 가이드가 따라가지 않고, 같은 Frame을 다른 View에서 볼 때
  기준이 달라져 채택하지 않았습니다.
- 레이아웃 그리드를 CSS grid 컨테이너 값으로: 자식 배치를 바꾸게 되어 "배치를 바꾸지 않는 설계 기준"이라는 행 정의와
  어긋납니다.
- 눈금자를 scene transient layer로: 화면 고정 크롬을 camera scene에 넣으면 camera 변환을 되돌리는 계산이 한 번 더
  필요하고, 셸 hit(눈금자에서 끌기)와 그리기 위치가 갈라집니다.
- 일시 해제 수식키를 ⌘로: ⌘는 이동 드래그의 재부모화 방지(ADR-0104)라 의미가 겹칩니다.
- Grid 명시 배치 자식 drag를 줄 번호 쓰기로: runtime scene에 grid track 기하가 없어 셀을 정확히 정할 수 없습니다.
- 흐름 preview에 layout crate 호출: crate 그래프가 금지합니다. 문서 쪽 layout을 runtime 밖에서 부르는 경로는 놓은 뒤
  실제 화면이 맡습니다.

## Consequences

- 가이드·그리드 값과 계산, 눈금, 가이드 제스처, 흐름 drag, 생성, 벡터 편집, 줌·팬, 클립보드가 셸 없이 Rust 호출로
  재현됩니다.
- 가이드·그리드의 문서 모델·command·저장·Diff·Merge 연결(결정 2.4)이 남아 있어 XII.10.19·XII.10.20은 그 연결 전까지
  부분 완료입니다. 그동안 가이드 편집은 `NodeGuides` 값과 역 편집으로만 검증합니다.
- `app::frame_insert`와 `canvas::create`, `app::node_paste`와 `canvas::clipboard`가 셸 연결 전까지 함께 있습니다.

## Compatibility

- 이번 작업은 파일 포맷·command 종류·FFI를 바꾸지 않습니다. 결정 2의 새 기록은 후속 연결에서 추가합니다(선택 값,
  migration 없음).
- 새 pasteboard 형식은 새 형식이고 기존 `STELLAR-CLIP\x01` bytes는 봉투 안에 그대로 들어갑니다.

## Performance

- 눈금 계산은 화면 크기/간격에 비례(수십 개), 가이드 스냅 선 변환은 가이드 수에 선형입니다.
- 흐름 삽입 index는 형제 수에 선형(제스처 시작 때 정렬)입니다.
- 결정 10의 측정값은 작업 보고서 XII.10.43에 남깁니다.

## Security

- 클립보드 봉투는 크기 상한(16 MiB, root 4,096개)과 형식 header를 검사하고 closed command codec(`ClipboardSubtree::
  decode`)으로만 해석합니다.
- 가이드·그리드 값은 유한값과 상한을 검사합니다. 비유한 입력은 거부합니다.

## Migration

없습니다. 이번 작업은 저장 형식을 바꾸지 않습니다.
