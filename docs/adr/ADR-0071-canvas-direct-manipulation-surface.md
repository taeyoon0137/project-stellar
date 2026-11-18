# ADR-0071: 캔버스 직접 조작 C ABI

## Status

Accepted (2026-11-15, 구현 기록)

## Context

UX 명세 5장(캔버스 편집 모델)은 도구 10종의 생성, 선택, 이동, 복제, 크기·반경 핸들, 카메라 조작을 캔버스에서
직접 하도록 정합니다. 사용자 결정 D7은 새 도형의 기본 채우기·선 색을 앱 설정(Settings ▸ 캔버스)에 두고, 코어
생성 command에 스타일을 입력으로 넘기며, 코어 안에 기본 색을 하드코딩하지 않도록 정합니다. 계층 규칙은 편집
의미·값 해석·선택을 코어가 갖고, 셸은 표시와 입력 전달만 하는 것입니다.

기존 C ABI(ADR-0014, ADR-0028)의 도형 생성은 Fill·Stroke를 코어 기본값(`ModifierPayload::default_for`)으로
만들었고, Rectangle만 별도 함수(`stellar_window_session_insert_rectangle`)로 만들었습니다. 이 ADR은 이 변경과
이후 캔버스 직접 조작 함수를 행별로 기록합니다. 함수는 `crates/ffi/src/canvas_gesture.rs`에 모읍니다.

## Decision

### UX.1.5 — 새 도형의 칠 입력(D7)

1. `StellarShapeStyle { StellarCanvasColor fill; StellarCanvasColor stroke; float stroke_width; }`를 추가합니다.
   색공간 tag 0은 "없음"입니다. stroke가 있으면 `stroke_width`는 유한한 양수여야 합니다.
2. `stellar_window_session_insert_shape`는 끝에서 두 번째 인자로 `const StellarShapeStyle *style`을 받습니다(null
   거부). 코어는 받은 값으로 Transform → Fill → Stroke 순서의 modifier를 만들고, 기본 색을 두지 않습니다. Line은
   stroke가 없으면, 닫힌 도형은 fill·stroke가 모두 없으면 생성하지 않습니다(보이지 않고 고를 수 없는 도형 방지).
3. Rectangle도 이 함수(ShapeKind tag 1)로 만듭니다. `stellar_window_session_insert_rectangle`과 runtime
   `insert_canvas_rectangle`은 같은 날 정리 통합이 먼저 삭제했습니다(ADR-0028). 정리 통합이
   `crates/ffi/src/window_session.rs`에 둔 값 전달 `StellarShapeStyle`·`insert_shape`는 이 파일의 포인터 전달 함수
   하나로 합쳤습니다.
4. 앱 설정의 의미(닫힌 도형은 채우기, Line은 선 색 1px)는 셸이 정해 넘깁니다. 값은 사용자 앱 설정에만 있고
   프로젝트 파일에는 쓰지 않습니다.
5. D7 해석(Claude 결정, 2026-11-15 확정): "도형 기본 채우기"는 닫힌 도형(Rectangle·Ellipse·Polygon)의 `fill`, "선
   기본 색"은 Line 도구로 만든 선의 `stroke`(1px)입니다. 닫힌 도형에는 기본 선을 넣지 않습니다. D7이 두 설정을 "새
   Rectangle·Ellipse·Polygon·Line 생성"에 쓰도록 정했고, 닫힌 도형에 기본 선까지 넣으면 사각형마다 지워야 하는 선이
   생기기 때문입니다. UX 명세 흐름 2의 문구와 Settings 레이블("Default Line Tool Color")을 이 해석에 맞췄습니다. 코어는
   셸이 넘긴 값만 쓰므로 해석이 바뀌어도 셸 설정 해석만 고칩니다.
6. 새 Frame의 기본 배경은 흰색(`#FFFFFF`, 불투명)으로 두고 Settings로 넘기지 않습니다(Claude 결정). D7이 설정으로
   정한 대상은 새 도형의 채우기·선뿐이고, 사용자가 정의하지 않은 설정 항목을 더하지 않기 위해서입니다. 지금은
   `frame_insert.rs`의 상수이며, 사용자가 Frame 배경 설정을 요청하면 D7과 같이 셸이 생성 요청에 넘기게 바꿉니다.

### UX.3.1 — 연속 줌

1. `stellar_window_session_zoom_by(handle, session, x, y, factor, out_request)`를 추가합니다. 화면 중심에서
   `(x, y)` point인 점을 고정하고 배율을 `factor`(유한한 양수)배 합니다. 배율 상하한(2%–25600%)은 viewport가
   정하고, 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다.
2. 셸은 트랙패드 핀치(`1 + magnification`)와 ⌘ 스크롤(`exp(0.01 × Δy pt)`)을 이 함수로 보냅니다. 두 손가락·휠
   스크롤 팬은 정리 통합의 코어 해석 경로(`stellar_window_session_submit_mac_event`, ADR-0028 스크롤 절)를 그대로
   씁니다. 그 경로의 팬 판정에서 Shift를 빼서 Shift+휠(시스템이 가로 delta로 바꿈)도 가로 팬이 되게 했습니다.
   Space를 누르는 동안의 임시 Hand는 Hand 도구와 같은 `stellar_window_session_pan`을 씁니다.
3. 카메라 조작은 문서·history·저장 상태를 바꾸지 않습니다.

### UX.3.6 — 선택

1. `stellar_window_canvas_pick(handle, session, view, leaf, flags, out_request)`: 셸은 포인터 아래 가장 위에 그린
   행의 node(`leaf`, null은 모든 Frame 밖)와 수식키 flag(`STELLAR_PICK_TOGGLE` Shift, `STELLAR_PICK_DEEP` ⌘,
   `STELLAR_PICK_ENTER` 더블클릭)만 보냅니다. 코어가 고를 node를 정합니다.
   - 기본: 지금 선택과 같은 깊이(선택한 node의 부모가 leaf의 조상이면 그 부모의 자식)를 고르고, 없으면 root의
     직계 자식을 고릅니다. root 배경이면 root, 모든 Frame 밖이면 선택을 비웁니다.
   - ⌘는 leaf 자체, 더블클릭은 선택한 node 안의 자식, Shift는 넣고 빼기(빈 곳이면 그대로)입니다.
   - 누름·놓음 단계(검토 수정): 셸은 node를 누를 때 `STELLAR_PICK_PRESS`, 움직이지 않고 놓을 때 `STELLAR_PICK_RELEASE`를
     더합니다. 누름에서 고를 node가 이미 선택에 있으면 코어는 선택을 그대로 두고, 바꿀 내용(하나로 줄이기, Shift면
     빼기)을 그 창 상태에 미룹니다. 놓음은 바로 앞 누름이 미룬 변경만 적용하고, 미룬 것이 없으면 그대로입니다. 끌어서
     편집하거나 다른 선택 요청이 오면 미룬 변경은 버립니다. 그래서 영역·Shift로 고른 여러 node를 그중 하나로 끌거나
     Option으로 복제할 수 있습니다. 두 flag를 함께 주면 `INVALID_ARGUMENT`입니다.
2. `stellar_window_canvas_pick_area(handle, session, view, x0, y0, x1, y1, flags, out_request)`: world 영역으로 root의
   직계 자식 가운데 보이고 잠기지 않은 node를 고릅니다. 기본은 겹침, `STELLAR_AREA_CONTAIN`(Option)은 완전 포함,
   `STELLAR_AREA_ADD`(Shift)는 지금 선택에 더하기입니다. 경계는 코어가 평가한 캔버스 행의 world 상자입니다.
3. `stellar_window_canvas_selection(handle, session, items, capacity, out_count, out_revision)`: 최신 게시 snapshot의
   `StellarCanvasSelectionItem`(id, kind, flags, world, local rect, radii)을 씁니다. flags는 이동·크기·반경 핸들
   가능 여부와 고정 크기 여부(`STELLAR_CANVAS_ITEM_*`)이며 코어가 잠금·숨김·흐름 부모·modifier 구성으로 정합니다.
   셸은 이 값으로 외곽선·핸들·크기 라벨만 그립니다. runtime `WindowSessionSummary`에 `canvas_selection`을 더합니다.
   이동 가능 flag는 이동 command와 같은 판정(편집 범위 + 위치 Transform·인스턴스 배치 규칙)으로 정해, 미리보기로
   움직였다가 놓으면 제자리로 돌아오는 node가 없습니다. 항목은 scene과 선택이 같으면 다시 계산하지 않습니다(팬·줌 게시).
4. 선택은 기존과 같이 그 창 history의 선택 항목으로 기록하고 문서·저장 상태는 바꾸지 않습니다.

### UX.3.7 — 이동·복제·화살표

1. `stellar_window_canvas_move(handle, session, view, anchor, dx, dy, flags, out_request)`: 지금 선택에서 다른 선택의
   자손을 뺀 node 전부를 한 transaction(Undo 한 번)으로 옮깁니다. Frame·Rectangle·Ellipse·Line·Polygon·Text·Image·
   배치한 인스턴스가 대상입니다. Frame은 `transform`, 배치한 인스턴스는 배치가 기록한 Transform 하나, 그 밖의 node는
   위치 Transform modifier(켜진 modifier 앞부분 Transform 가운데 마지막 것, 없으면 맨 앞에 새로)를 바꿉니다. 위치
   modifier 규칙은 `stellar_commands::inspector::position_modifier` 하나이며 속성 탭 `left`·`top`(ADR-0070)도 같은
   함수를 씁니다.
2. 기본 단위는 world이며 코어가 각 node의 부모 행 world 선형부 역변환으로 부모 공간 이동량을 구합니다.
   `STELLAR_MOVE_PARENT_SPACE`는 부모 공간 단위(화살표 1px, Shift+화살표 10px)입니다. XII.10.2 ADR이 아직 없어
   명세 5.4의 값을 셸이 임시로 씁니다. UX.3.7 기준 "화살표 단위가 XII.10.2 ADR과 같음"은 미충족이며, 그 ADR이 정해지면
   단위를 코어 상수로 옮깁니다.
3. `anchor`는 드래그를 시작한 행입니다. 지금 선택이 그 행을 덮지 않으면(Shift로 뺀 node를 끌면) 거부합니다. document
   thread가 요청을 차례로 처리하므로 누를 때 보낸 선택 요청이 먼저 적용됩니다.
4. `STELLAR_MOVE_DUPLICATE`(Option 드래그)는 각 node의 복제본을 원본 바로 위(다음 형제 앞) 순서로 만들고 복제본을
   옮긴 뒤 복제본을 선택합니다. 복제·이동·선택이 history 항목 하나입니다. root Frame은 복제하지 않습니다.
5. 셸은 이 경로만 씁니다. 단일 Frame·Shape 이동 `stellar_window_session_move_frame`과 runtime `move_canvas_frame`·
   `FrameEdit::Move` 분기는 같은 이동의 중복 경로라 삭제했습니다(검토 수정). 그 테스트(인스턴스 배치 이동, Rectangle
   modifier 보존)는 이 경로의 테스트로 옮겼습니다.

### UX.2.9 — 크기 핸들과 크기 라벨

1. `stellar_window_canvas_resize(handle, session, view, node, x, y, width, height, out_request)`: 하나뿐인 선택 node의
   상자를 지금 local 좌표의 새 상자로 바꿉니다(크기 > 0, Undo 한 번). 셸은 핸들을 끈 결과 상자만 보내고(Shift 비율
   유지, Option 중심 기준은 셸의 입력 해석), 어떤 값을 바꿀지는 코어가 정합니다.
   - 크기는 속성 탭(ADR-0070)과 같은 longhand key의 `SetProperty`로, 값이 바뀐 key만 씁니다. 같은 값을 캔버스와 속성
     탭에서 고치면 같은 command가 됩니다.
   - Frame: `width`·`height`와, 원점이 바뀌면 `transform`(`T ∘ translate(x, y)`).
   - Rectangle·Ellipse·Polygon: shape `width`·`height`. Image: image `width`·`height`. 원점이 바뀌면 위치 Transform
     modifier(하나뿐일 때, 없으면 맨 앞에 새로)를 옮기고, Ellipse·Polygon의 local bounds 원점과 변 수·회전·반경은 그대로
     둡니다.
   - Line: 두 끝점을 이전 상자에서 새 상자로 같은 비율로 옮겨 `x1`·`y1`·`x2`·`y2`로 씁니다.
   - 처음 구현의 `RestoreNodeRecords`(레코드 통째 교체) 경로는 삭제했습니다. 역 command가 레코드 전체를 되돌려 그 사이에
     바뀐 이름·잠금까지 덮어쓸 수 있었고, 속성 단위 의미 변경으로 잡히지 않았습니다.
   - Text는 저장 형식에 고정 폭이 없어 크기 핸들이 없고(XII.10.33), 인스턴스는 정의 크기를 따라 없습니다.
2. `stellar_window_canvas_line_end(handle, session, view, node, end, x, y, out_request)`: Line 끝점 핸들 하나를 옮깁니다.
   바뀐 `x1`·`y1`(시작) 또는 `x2`·`y2`(끝)만 `SetProperty`로 씁니다.
3. `StellarCanvasSelectionItem`에 `line[4]`(Line의 local 시작·끝점)를 더합니다. 셸은 단일 선택에 크기 핸들 8개(화면
   상자가 24pt보다 작으면 모서리 4개) 또는 Line 끝점 2개를 그리고, 선택 아래에 `320 × 200` 라벨을 그립니다. 고정 크기가
   아닌 축은 자동 폭 Text의 CSS 값(`width: max-content`, `height: auto`, canvas_editing.md)으로 `max-content × auto`처럼
   보입니다. 핸들·라벨은 캔버스 overlay라 렌더 결과에 들어가지 않습니다.

### UX.2.10 — 모서리 반경 핸들

1. `stellar_window_canvas_radius(handle, session, view, node, corner, radius, out_request)`: 하나뿐인 선택 node의
   반경을 바꿉니다(Undo 한 번). `corner`는 -1이면 네 모서리, 0–3이면 tl·tr·br·bl 하나(Option)입니다.
   - Frame·Rectangle: `border-top-left-radius` 등 `border-*-radius` longhand(Frame key, Rectangle은 shape key)를 바뀐
     것만 `SetProperty`로 씁니다. Polygon: 반경 key 하나(`corner`는 무시)입니다. 모두 속성 탭과 같은 key입니다.
   - 반경은 유한한 0 이상 원본 값이며, 상자보다 큰 값은 평가할 때 줄어듭니다.
2. 반경 핸들 표시 여부는 선택 항목 flag(`STELLAR_CANVAS_ITEM_RADIUS`, `…_RADIUS_PER_CORNER`)로 코어가 정합니다. 셸은 화면
   상자가 40pt 이상일 때 모서리 안쪽(반경, 최소 12pt)에 핸들을 둡니다. 처음 구현이 끄는 동안 보이던 값 라벨
   (`border-radius: 12px`, Polygon은 `radius:`)은 명세 5.4·흐름 2에 없는 UI라 삭제했습니다.

### 검토 뒤 남은 범위

- UX.1.5: 고정 폭 Text 드래그 생성(XII.10.33)은 아직 없습니다. Text 도구는 클릭 생성만 합니다.
- UX.1.5: Image는 드래그로 크기를 정할 수 없고, 도구를 고른 뒤 Frame을 클릭해야 파일 선택이 열립니다(명세 5.1의
  "고르는 즉시 파일 선택", U11과 다름). UX.3.3을 선행으로 연결했습니다.
- UX.2.9: Text 크기 핸들(XII.10.33)과 다중 선택 경계 상자 핸들이 없습니다. 핸들을 끄는 동안 도형은 그대로이고 윤곽·
  라벨만 따라갑니다(놓을 때 한 번 반영). 끄는 동안 코어 편집을 미리 적용하는 preview(`begin_pointer_gesture`)는
  UX.3.8 스냅과 함께 합니다.
- 셸이 해석하는 편집 의미(크기 핸들 Shift 비율·Option 중심·최소 1px, Line 끝점 45°, 반경 값과 반올림, 화살표 단위,
  클릭 생성 기본 크기 100과 Shift·Option, root Frame 드래그 판정)는 Context의 "해석은 코어, 셸은 전달" 규칙과 다른 임시
  구조입니다. 같은 핸들 조작을 CLI·MCP·코어 테스트가 재현할 수 없습니다. 코어 이전은 WBS UX.3.8의 범위·검증 기준에
  넣어 추적합니다. 반경 미리보기와 확정이 달랐던 반올림은 셸에서 같은 값으로 맞췄습니다(2026-11-15 검토).
- 캔버스 크기·반경·끝점 ABI는 revision·history 위치를 검사하지 않습니다(UX.3.14).
- UX.3.6: 영역 선택은 root의 직계 자식만 봅니다. 안쪽 Frame 안에서 시작한 영역 선택은 아직 없습니다.
- 편집 실패(잠긴 node가 섞인 선택의 화살표 등)는 조용히 무시되고, 이유를 도구 막대 위 한 줄 안내로 보이지 않습니다.

### C ABI 표

새 함수와 상수는 ADR-0028의 창 session 표면에도 기록합니다.

## Alternatives

- 코어에 기본 색 표를 두고 셸이 덮어쓰기: D7("코어 하드코딩 없음")에 어긋나 채택하지 않았습니다.
- Rectangle 전용 함수에 style을 더 붙이기: 같은 생성 경로가 둘로 남아 삭제했습니다.

## Consequences

- 새 Rectangle·Ellipse·Polygon·Line의 칠이 Settings ▸ 캔버스 값과 같습니다. 설정을 바꾸면 다음 생성부터 적용됩니다.
- 셸이 칠을 넘기지 않으면 생성이 거부되므로, 칠 없는 도형이 문서에 생기지 않습니다.

## Compatibility

- `stellar_window_session_insert_shape` 시그니처가 바뀝니다(인자 추가). 같은 저장소의 셸과 함께 바뀌며 외부 사용자는
  없습니다. `stellar_window_session_insert_rectangle`은 삭제합니다.
- 파일 포맷은 바뀌지 않습니다. 저장되는 modifier는 기존 Fill·Stroke입니다.

## Performance

생성 경로에 상수 비용 검사만 더합니다. 입력 경로 예산에 영향이 없습니다.

## Security

새 입력은 색·두께 값뿐이며 유한 값·범위를 FFI 경계에서 검사합니다.

## Migration

없습니다. 기존 문서의 도형은 그대로 읽힙니다.

## 개정 기록

- 2026-11-15: "검토 뒤 남은 범위"의 '생성(Frame·도형·Text)은 아직 프로젝트 신뢰를 요구' 항목을 지웠습니다. 정리
  통합의 D1 편집 게이트 제거(ADR-0028 "제한 모드의 문서 편집")와 합친 뒤 runtime·셸 생성 경로에 신뢰 검사가
  없습니다.
- 2026-11-15: UX.3.1이 팬 판정에서 Shift를 뺀 뒤 `crates/ffi/src/canvas.rs`의 수정자 배열이 rustfmt 형식과 달라
  `cargo xtask check-core`의 fmt 단계가 실패했습니다. 줄 맞춤만 고쳤고 C ABI·동작은 바뀌지 않습니다.
- 2026-11-18(UX.3.14): `stellar_window_canvas_resize`·`stellar_window_canvas_radius`·
  `stellar_window_canvas_line_end`는 누른 시점의 `revision: u64`와 `history_sequence: u64`를 node 뒤에 받습니다.
  document thread에서 현재 창 값과 다르면 적용 전 stale 오류로 거부합니다. 셸은 선택 핸들을 누를 때 읽은
  `StellarSelectionTarget`을 놓을 때까지 보관해 두 값을 전달합니다. 상대 이동 ABI는 그대로입니다.
