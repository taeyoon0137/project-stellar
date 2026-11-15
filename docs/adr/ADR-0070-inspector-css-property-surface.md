# ADR-0070: 속성 패널의 CSS 속성 표면과 범용 편집 C ABI

## Status

Accepted (2026-11-15, UX.2.1·UX.2.2 구현 기록)

## Context

UX 명세 6장과 흐름 2는 Frame·Rectangle·Ellipse·Line·Polygon·Text·Image 모두와 다중 선택이 속성 탭에 CSS
속성 행으로 나오고, 크기·색·반경을 바꿀 수 있으며 모두 Undo가 되기를 요구합니다. 지금 코어 속성 registry는
Frame payload만 다루고(`crates/commands/src/property.rs`), 셸 인스펙터는 Frame 하나를 고를 때만 값을 만들며
속성마다 C ABI 함수를 따로 둡니다(`stellar_window_session_resize_frame`, `_set_frame_corner`,
`_set_frame_background`). 함수마다 값 형식과 history 검사가 달라 오래된 snapshot의 편집을 일관되게 막지 못하고,
속성이 늘 때마다 ABI가 늘어납니다.

ADR-0024는 디자인 속성의 이름·값 문법을 CSS longhand로 정하고(longhand 하나 = 속성 하나, 결정 14), 셸은 표시와
입력 전달만 맡습니다(아키텍처 §8.2). I.3.5 정책에 따라 `crates/ffi/` 변경을 이 ADR에 기록합니다.

## Decision

1. **payload key(UX.2.1)**: `crates/document/src/payload_metadata.rs`에 Shape(`0x03xx`)·Image(`0x06xx`)
   longhand key를 node 종류 상위 byte 규칙으로 할당합니다. `width` 0x0301, `height` 0x0302,
   `border-{top-left,top-right,bottom-right,bottom-left}-radius` 0x0303–0x0306(Rectangle), `x1`·`y1`·`x2`·`y2`
   0x0307–0x030a(Line, Stellar 전용), `stroke-linecap` 0x030b, `polygon-sides` 0x030c·`polygon-rotation`
   0x030d·`polygon-corner-radius` 0x030e(Polygon, Stellar 전용), Image `width` 0x0602·`height` 0x0603·
   `object-fit` 0x0604·`image-rendering` 0x0605. `SetProperty`는 payload를 보고 도형 종류에 없는 key를
   거부합니다. 값 타입에 부호 있는 수(`Number`, tag 18)와 keyword 저장 tag(`Keyword`, tag 19)를 더합니다.
   Text style longhand는 Typography record(IX.11.11)가 생긴 뒤 같은 규칙으로 더합니다.
2. **속성 표면(UX.2.2)**: `crates/commands/src/inspector.rs`가 선택 node들의 행(섹션, CSS 이름, 저장 key,
   flag, 정규 값 텍스트)과 편집 command를 만듭니다. 값 parser·formatter는 `css_value.rs`이며 속성 모델 2장
   입력만 받고 그 밖은 `unsupported-unit`·`unsupported-value`·`unknown-token`·`forbidden-value`·
   `invalid-value`로 거부합니다. 저장 위치는 코어가 정합니다.
   - Frame: payload longhand(`width`, `height`, `background-color`, `border`, `overflow`, 4모서리).
   - Shape·Image: 위 payload key.
   - `fill`·`stroke`·`stroke-width`와 Text `color`: 첫 활성 Fill·Stroke modifier. 없으면 렌더 순서
     (Transform → Fill → Stroke)에 맞춰 넣고, `none`이면 뺍니다. 행은 늘 있습니다.
   - `left`·`top`: Frame은 `transform` 이동 성분, 그 밖은 앞쪽 Transform modifier 이동 성분입니다. 캔버스 이동과
     같은 저장 위치이며, 위치 longhand(IX.11.6·IX.11.9)가 생기면 그 key로 옮깁니다.
   - 묶음 행 `border-radius`는 longhand 4개의 shorthand이고 편집은 longhand 변경 집합 하나입니다.
3. **C ABI**:
   - `stellar_window_inspector(handle, session, info, rows, row_capacity, text, text_capacity)`: 창의 확정 선택
     행을 게시된 상태 snapshot에서 읽습니다(document thread를 기다리지 않음). `StellarInspectorInfo`(56 byte:
     version, count, classes, row_count, revision, history_sequence, view\[16\], name_len, text_len)와
     `StellarInspectorRow`(24 byte: key, flags, section, name_len, value_len, choices_len). flag는
     `STELLAR_INSPECTOR_*`(MIXED, READ_ONLY, COLOR, GROUP, LONGHAND, STELLAR_ONLY, NUMERIC, KEYWORD, BOOL).
   - `stellar_window_edit_property(handle, session, edit, name, name_len, value, value_len, out_request,
     diagnostic, diagnostic_capacity, out_diagnostic_len)`: `StellarPropertyEdit`(48 byte: view\[16\],
     revision, history_sequence, delta, mode, phase). mode 0은 CSS 값 텍스트, 1은 각 node 현재 값에 delta를
     더합니다(혼합 값의 화살표). phase 0 확정, 1 연속 중간 값, 2 연속 끝, 3 연속 취소. 값 문법 오류는 요청을
     만들지 않고 `InvalidArgument`와 `code: 이유` 진단을 씁니다. 완료는 `STELLAR_EVENT_WINDOW_VIEW_CHANGED` 또는
     `STELLAR_EVENT_REQUEST_FAILED`입니다.
   - 모든 편집은 history sequence와 active View를 검사하고, 연속 조작 중이 아니면 revision도 검사합니다.
     history sequence는 선택 변경에도 바뀌므로 표시한 선택과 다른 선택에는 적용되지 않습니다.
4. **Undo 단위**: 확정 입력은 한 history 항목입니다. 연속 조작은 history preview(VII.4.2·VII.4.5)로 문서에
   반영하고, 매 갱신마다 시작 전 상태로 되돌린 뒤 시작 값 기준으로 다시 계산합니다. 끝(phase 2)에서 한 항목으로
   확정하고 취소(phase 3)는 항목을 만들지 않습니다.
5. **잠금(D8)**: 자신이나 조상이 잠긴 node는 `locked` 행 외에는 읽기 전용이고 편집을 거부합니다. 편집은 신뢰와
   무관합니다(D1).
6. 속성마다 C ABI 함수를 새로 두지 않습니다. 기존 Frame 전용 편집 함수는 셸이 더 쓰지 않으며, 정리 통합의 단일
   인스펙터 편집 함수와 합칠 때 제거합니다.

## Alternatives

- **속성마다 함수**: 지금 방식입니다. 값 형식·history 검사가 갈라지고 ABI가 속성 수만큼 늘어 탈락했습니다.
- **PropertyKey와 이진 값을 넘기는 함수**: 셸이 node 종류별 key와 값 타입(Frame `transform` 행렬, modifier 위치
  등)을 알아야 해 셸에 문서 의미가 생깁니다. 셸이 CSS 이름·값 텍스트만 다루게 했습니다.
- **셸이 값 계산(화살표·scrub)**: 혼합 값에서 node마다 다른 기준 값을 셸이 알아야 합니다. delta를 코어로 넘깁니다.

## Consequences

- 셸은 문서 의미 없이 행 이름·값 텍스트·flag만 표시하고, 입력 텍스트·delta·단계만 보냅니다.
- 행 snapshot은 창 상태를 게시할 때 확정 선택·revision·history 위치가 바뀐 경우에만 다시 만들고, 비용은 선택
  크기에 비례합니다.
- 고정 px만 저장하는 속성에 `%`·`fit-content`·`var()`를 넣으면 문법은 알아보되 적용하지 않고 진단합니다. 저장
  형식은 IX.11.6·토큰 연결이 정합니다.
- semantic snapshot의 Shape·Image 전용 key(`0xE3xx`·`0xE6xx`) 교체는 IX.11.3에 남깁니다.

## Compatibility

- 새 C ABI 함수 2개와 구조체 3개, flag 상수를 더합니다. 기존 함수·상태 코드·event 의미는 바꾸지 않습니다.
- `SetProperty` 값 tag 18·19는 추가이며 기존 journal bytes의 의미는 그대로입니다. 저장 형식은 바뀌지 않습니다
  (PropertyKey는 payload bytes에 담기지 않음).
- 헤더는 `cargo xtask generate-ffi-header`로 템플릿에서 생성합니다.

## Performance

- 읽기는 게시된 snapshot 복사뿐이라 UI thread에서 문서 질의가 없습니다. 연속 조작은 요청마다 preview 되돌림과
  적용 한 번이며 비용은 선택 크기에 비례합니다. 입력→Present 10ms 계약 측정은 Release 빌드에서 따로 합니다.

## Security

- 입력 텍스트는 256 byte 상한, 재귀 없는 문법입니다. 외부 리소스 값(`url()` 등)은 `forbidden-value`로 거부합니다.

## Migration

- 데이터 migration은 없습니다. 셸은 Frame 전용 편집 함수 대신 새 함수를 씁니다.
