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
2. `stellar_window_canvas_pick_area(handle, session, view, x0, y0, x1, y1, flags, out_request)`: world 영역으로 root의
   직계 자식 가운데 보이고 잠기지 않은 node를 고릅니다. 기본은 겹침, `STELLAR_AREA_CONTAIN`(Option)은 완전 포함,
   `STELLAR_AREA_ADD`(Shift)는 지금 선택에 더하기입니다. 경계는 코어가 평가한 캔버스 행의 world 상자입니다.
3. `stellar_window_canvas_selection(handle, session, items, capacity, out_count, out_revision)`: 최신 게시 snapshot의
   `StellarCanvasSelectionItem`(id, kind, flags, world, local rect, radii)을 씁니다. flags는 이동·크기·반경 핸들
   가능 여부와 고정 크기 여부(`STELLAR_CANVAS_ITEM_*`)이며 코어가 잠금·숨김·흐름 부모·modifier 구성으로 정합니다.
   셸은 이 값으로 외곽선·핸들·크기 라벨만 그립니다. runtime `WindowSessionSummary`에 `canvas_selection`을 더합니다.
4. 선택은 기존과 같이 그 창 history의 선택 항목으로 기록하고 문서·저장 상태는 바꾸지 않습니다.

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
