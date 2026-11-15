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
