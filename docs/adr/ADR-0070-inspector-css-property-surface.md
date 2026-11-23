# ADR-0070: 속성 패널의 CSS 속성 표면과 범용 편집 C ABI

## Status

Accepted (2026-11-15, UX.2.1·UX.2.2 구현 기록, 같은 날 검토 지적 반영)

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
   Text style longhand는 Typography record(IX.11.11)가 생긴 뒤 같은 규칙으로 더합니다. 따라서 UX.2.1의 Text
   부분은 끝나지 않았고, Text 선택은 지금 Layer·Position 행과 `color`만 보입니다.
2. **속성 표면(UX.2.2)**: `crates/commands/src/inspector.rs`가 선택 node들의 행(섹션, CSS 이름, 저장 key,
   flag, 정규 값 텍스트)과 편집 command를 만듭니다. 값 parser·formatter는 `css_value.rs`이며 속성 모델 2장
   입력만 받고 그 밖은 `unsupported-unit`·`unsupported-value`·`unknown-token`·`forbidden-value`·
   `invalid-value`로 거부합니다. 저장 위치는 코어가 정합니다.
   - 행은 UX 명세 6.2절 표에 있는 속성만 냅니다. key가 있어도 표에 없는 `stroke-linecap`·`image-rendering`은
     행으로 내지 않습니다(key와 저장은 그대로). Path는 앱 캔버스가 아직 그리지 않고 만드는 도구도 없어 Layer
     행만 둡니다. 칠하기·선 행을 두면 바꿔도 보이지 않는 값이 되므로, Path 투영이 생길 때 넣습니다.
   - 선택이 없으면 행 없이 이름 자리에 현재 Page 요약(`Page · View`)을 둡니다(UX 명세 6.1).
   - Frame: payload longhand(`width`, `height`, `background-color`, `border`, `overflow`, 4모서리).
   - Shape·Image: 위 payload key.
   - `fill`·`stroke`·`stroke-width`와 Text `color`: 첫 활성 Fill·Stroke modifier. 없으면 렌더 순서
     (Transform → Fill → Stroke)에 맞춰 넣고, `none`이면 뺍니다. 행은 늘 있습니다.
   - 색 기본값은 코어에 두지 않습니다(D7: 셸 설정). 선이 없는 node에 `stroke` 색을 넣으면 두께는 CSS
     `stroke-width` 초기값 1px로 시작합니다. 선이 없는 node의 `stroke-width`만 바꾸는 입력과, 테두리 없는
     Frame에 두께나 색 하나만 적은 `border`는 거부합니다. 테두리가 있으면 빠진 쪽은 지금 값을 이어 씁니다.
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
     더합니다(혼합 값의 화살표). phase 0 확정, 1 연속 중간 값, 2 연속 끝, 3 연속 취소, 4 반복 입력(결정 4).
     값 문법 오류는 요청을 만들지 않고 `InvalidArgument`와 `code: 이유` 진단을 씁니다. 완료는
     `STELLAR_EVENT_WINDOW_VIEW_CHANGED` 또는 `STELLAR_EVENT_REQUEST_FAILED`입니다.
   - 모든 편집은 history sequence와 active View를 검사하고, 연속 조작 중이 아니면 revision도 검사합니다.
     history sequence는 선택 변경에도 바뀌므로 표시한 선택과 다른 선택에는 적용되지 않습니다. 취소(phase 3)만은
     검사 없이 열린 preview를 닫습니다.
4. **Undo 단위**: 확정 입력은 한 history 항목입니다.
   - history preview(VII.4.2·VII.4.5)는 실제 포인터 제스처인 레이블 scrub에만 씁니다. 포인터를 누르고 있는
     동안이라 preview 중에 다른 창 요청(Undo·캔버스 선택)이 끼지 않습니다. 매 갱신마다 시작 전 상태로 되돌린 뒤
     시작 값 기준으로 다시 계산하고, 끝(phase 2)에서 한 항목으로 확정하며 취소(phase 3)는 항목을 만들지 않습니다.
   - preview를 남기지 않습니다: 취소는 표시 상태와 무관하게 preview를 닫고, 검사에 걸린 끝·중간 요청도 preview를
     닫은 뒤 거부합니다. 셸은 끝·취소를 큐에서 버리지 않고 기다림 없이 바로 보내며, 선택이 사라졌으면 끝 대신
     취소를 보냅니다.
   - 화살표 ±1·±10과 색 견본·색 패널 변경은 반복 입력(phase 4)입니다. preview 없이 요청마다 바로 기록하고, 같은
     선택·같은 속성 저장 key의 직전 반복 입력과 0.5초 안이면 history grouping(VII.4.4 키보드 반복 정책)으로 한
     항목에 합칩니다. 그 사이 Undo·선택·다른 편집은 막히지 않습니다. `left`·`top`은 같은 저장 key(이동 성분)라
     0.5초 안에 이어 바꾸면 이동 한 번으로 합쳐집니다. Escape는 입력 중인 텍스트만 버리며, 이미 적용한 화살표
     값은 Undo로 되돌립니다.
5. **잠금(D8)**: 자신이나 조상이 잠긴 node는 `locked` 행 외에는 읽기 전용이고 편집을 거부합니다. 속성 패널의
   판정은 `stellar_commands::lock::NodeLock`입니다. 편집 가능 판정은 세 곳에 더 따로 있습니다. 캔버스는 runtime
   `editable_ancestry`, Layers의 이름·표시·잠금 토글과 삭제·붙여넣기는 runtime 검사(`sidebar`의 조상 검사 등),
   `stellar apply`는 ops의 결과 비교(`stellar_ops::apply`의 `locked_change`)를 씁니다. 잠긴 node·잠긴 조상 아래
   수정과 조상이 잠기지 않은 node의 `locked` 토글만 허용하는 D8 규칙은 같지만, root Frame·흐름 부모·숨긴 조상
   처리는 서로 다릅니다. 판정을 하나로 모으는 일은 UX.3.11이고, user command validator 거부(XIX.4.3)도 그 판정을
   씁니다(2026-11-15 교차 검토: 처음 문구 "판정은 `NodeLock` 하나"는 정리 통합의 공용 판정을 전제로 했으나, 그
   커밋을 옮기지 않아 사실과 달랐습니다). 이름·표시·잠금 편집은 Layers와 같은 command(`rename_node`·`set_visibility`·
   `set_locked`, VII.2.5 `SetProperty` specialization)입니다. 편집은 신뢰와 무관합니다(D1).
   복제·복사는 원본 subtree를 바꾸지 않으므로 선택 root와 그 조상(자신 포함)만 확인하고 잠긴 자손은 막지
   않습니다. ⌘D·Copy(runtime `unlocked_selected_roots`), 캔버스 Option 드래그 복제(`editable_ancestry`),
   `stellar apply`의 `duplicate-subtree`가 이 규칙으로 같습니다. 삭제·잘라내기는 잠긴 자손이 부모와 함께
   사라지면 안 되므로 자손까지 봅니다(`editable_selected_roots`). 그래서 Cut은 Copy와 Delete 능력이 모두 있을
   때만 켭니다(2026-11-15 최종 통합 검토: 처음에는 ⌘D·Copy만 잠긴 자손을 거부해 Option 드래그·CLI와 달랐습니다.
   D8은 잠긴 node의 수정을 막는 규칙이라 복제 쪽을 맞췄습니다). 자기 자신이 잠긴 node의 복제는 앱이 거부하고
   CLI는 허용하는 차이가 남아 있고, UX.3.11에서 함께 모읍니다.
6. **ABI 기준 하나**: 속성마다 C ABI 함수를 새로 두지 않습니다. 속성 패널의 선택 snapshot은
   `stellar_window_inspector`, 편집은 `stellar_window_edit_property` 하나가 기준입니다.
   - 이 lane에서 셸의 Frame 전용 경로(`CoreRuntime.resizeFrame`·`selectedFrameRadii`·`setFrameCorner`·
     `setFrameBackground`, `CanvasFrameScene.selectedInspectorFrame`)를 지웠습니다. FFI 쪽
     `stellar_window_session_resize_frame`·`_set_frame_background`·`_set_frame_corner`·
     `stellar_window_selected_frame_corners`와 runtime 래퍼는 같은 시기의 정리 통합(Inspector 선택 snapshot과
     Frame 속성 편집)이 지우므로 여기서 다시 지우지 않습니다.
   - 정리 통합이 더한 `stellar_window_selected_node`(`StellarSelectedNode`, runtime `SelectedNode`·`SelectedFrame`)와
     `stellar_window_session_set_frame_property`(`StellarFrameValue`, runtime `FrameValue`·
     `set_canvas_frame_property`)는 두 함수와 역할이 겹칩니다. 정리 통합 위에 UX 1단계를 합친 diff에서 이 둘과
     셸 호출자(`CoreRuntime.selectedNode`, 이벤트 펌프 `editFrame`, `ShellFrameInspectorSource`)를 지웠습니다.
     셸 Inspector는 `stellar_window_inspector` 값을 읽고, ADR-0028의 해당 절에 이 ADR로 대체했다고 적었습니다.
7. **제스처 중 구성 고정(UX.2.12)**: 속성 패널은 캔버스 쪽 질의 하나(`CanvasView.frameGestureActive`)와 패널
   scrub 중인 행 집합으로 제스처 여부를 판단합니다. 캔버스의 제스처 상태는 모두 그 질의에 넣습니다. UX 1단계
   통합에서 이동·생성(`frameDrag`), Hand·Space 임시 Hand, Zoom 드래그에 더해 영역 선택(`marquee`)과 핸들 드래그
   (`handleDrag`: 크기 조절 8개, 모서리 반경, Line 끝점, ADR-0071)를 넣었습니다.

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
  형식은 IX.11.6·토큰 연결이 정합니다. 따라서 UX.2.6의 "`50%`·`fit-content`·`var(--x)`가 정규화되어 저장됨"
  기준은 아직 충족하지 않습니다(`320`·`1.5turn`만 저장, `1.5turn`은 회전 longhand가 생기기 전까지
  `polygon-rotation`에서만).
- semantic snapshot의 Shape·Image 전용 key(`0xE3xx`·`0xE6xx`) 교체는 IX.11.3에 남깁니다.

## Compatibility

- 새 C ABI 함수 2개와 구조체 3개, flag 상수를 더합니다. 기존 함수·상태 코드·event 의미는 바꾸지 않습니다.
  phase 4(반복 입력)와 선택 없음 때의 Page 요약 이름은 같은 날 검토 지적으로 더했으며 구조체 크기는 같습니다.
- `SetProperty` 값 tag 18·19는 추가이며 기존 journal bytes의 의미는 그대로입니다. 저장 형식은 바뀌지 않습니다
  (PropertyKey는 payload bytes에 담기지 않음).
- 헤더는 `cargo xtask generate-ffi-header`로 템플릿에서 생성합니다.

## Performance

- 읽기는 게시된 snapshot 복사뿐이라 UI thread에서 문서 질의가 없습니다. 연속 조작은 요청마다 preview 되돌림과
  적용 한 번이며 비용은 선택 크기에 비례합니다.
- 행 snapshot은 node마다 modifier stack을 한 번 훑어 요약하고(Transform 합성, 첫 Fill·Stroke) 속성 행은 그 요약을
  읽습니다. 값은 해석한 값끼리 비교해 혼합 여부를 정하고 정규 텍스트는 행마다 한 번만 만듭니다. 잠금은 공통
  column의 조상 사슬만 따라갑니다. 비용은 선택 node 수 × (modifier 수 + 조상 수 + 속성 수)이며 문서 크기와
  무관합니다.
- 측정(2026-11-15, Apple M3 Max, Release `cargo test --release`의 임시 측정, REF-MAC 판정 아님): Fill·Stroke·
  Transform modifier가 있는 Rectangle 1만 개를 모두 고른 snapshot은 7회 중앙값 6.8ms(6.5–7.2ms)입니다. 위 요약·값
  비교 전에는 17.2ms(16.7–18.0ms)였습니다. node 하나 선택은 중앙값 2.4µs입니다. 같은 1만 개에 `fill` 하나를 넣는
  command 만들기는 약 6ms, transaction 적용은 14–18ms(command 경로 비용)입니다. 1만 개 선택에서는 편집 한 번이
  document thread를 20ms 넘게 점유하므로, 큰 선택의 행 수 상한·지연 계산은 Select All(UX.3)을 통합할 때 이 수치로
  다시 정합니다.
- 입력→Present 10ms 계약 측정은 하지 않았습니다(UX.2.12 미완료 기준). Release 빌드의 입력 trace로 따로 합니다.

## Security

- 입력 텍스트는 256 byte 상한, 재귀 없는 문법입니다. 외부 리소스 값(`url()` 등)은 `forbidden-value`로 거부합니다.

## Migration

- 데이터 migration은 없습니다. 셸은 Frame 전용 편집 함수 대신 새 함수를 씁니다.

## 개정(2026-11-18, UX.3.11)

- `stellar_commands::lock::edit_permission`을 속성 탭, 캔버스 편집, Layers 토글·이름·삭제·복제, CLI apply가 공유합니다. 자신이나 조상이 잠기면 수정할 수 없고, 자신의 잠금 토글은 조상이 잠기지 않았을 때 허용합니다. 숨김은 편집을 막지 않습니다.
- root Frame은 속성과 잠금·표시를 바꿀 수 있지만 삭제·이동·복제할 수 없습니다. 삭제는 잠긴 자손이 있으면 막고, 복제는 원본과 조상의 잠금만 확인합니다. CLI의 복제도 같은 원본 검사를 합니다.
- 흐름 레이아웃의 자식은 위치 속성 변경과 캔버스 이동을 거부합니다. 활성 View 소속과 modifier·clipping 관련 캔버스 제약은 runtime에서 별도로 확인합니다.
- 보완(2026-11-23): 잠금 규칙은 `EditIntent::blocked_by` 하나이고, 일반 편집은 특정 key를 대리로 쓰지 않고 `EditIntent::Edit`로 나타냅니다. 속성 탭은 node마다 `NodeAccess`(잠금·root·흐름 부모)를 한 번 계산해 행 판정에 다시 씁니다. CLI apply는 적용 전 레코드에서 `NodeLock::resolve`로 같은 규칙을 계산하며, 모델을 다시 만들지 않고, 이번 transaction이 새로 넣은 노드는 잠금 대상이 아니라 그 부모만 검사합니다. Layers의 View 소속 검사(`in_view`)는 잠금을 보지 않습니다.
- 측정(2026-11-23, Apple M3 Max, Release `cargo test --release -p stellar-commands --lib measure_snapshot_10k_rectangles -- --ignored`, REF-MAC 판정 아님): Fill·Stroke·Transform이 있는 Rectangle 1만 개 선택 snapshot은 7회 중앙값 7.5ms입니다. 같은 측정에서 판정 통합 전(node별 `NodeLock` 재사용)은 7.1ms, 행×node마다 `edit_permission`을 부르던 2026-11-18 구현은 39.7ms였습니다.

## 개정(2026-11-18, UX.3.13)

- Select All은 활성 View root의 보이고 잠기지 않은 직계 자식을 코어에서 골라 창 선택 history에 기록합니다. 영역 선택과 같은 자식 필터를 씁니다. 셸은 레이어 페이지를 순회하지 않습니다.
- 1만 개 선택의 Inspector snapshot과 편집 비용은 위 Performance 절의 2026-11-15 측정값(각각 6.8ms, 명령 생성 약 6ms와 적용 14–18ms)을 참고합니다. 이번 행에서는 테스트·벤치 실행이 금지되어 새 측정이나 비용 상한 검증을 하지 못했습니다. 대량 선택의 편집 비용은 여전히 20ms를 넘을 수 있습니다.

## 철회(2026-11-23, UX.2.13 테두리 행)

- 2026-11-18 UX.2.13 개정의 `border-width`·`border-color` 행을 제거합니다. 두 행은 longhand flag를 달았지만 묶음 key `BORDER`를 함께 써서 ADR-0024의 "longhand 하나 = 속성 하나"와 UX.2.13 완료 기준(변 하나를 바꾸면 그 longhand만 저장)을 따르지 않았습니다. `border-color: none`을 테두리 제거로 해석해 CSS 문법에 없는 값을 앱·CLI·MCP 공용 값 문법에 넣었고, 기존 `border` 묶음 행이 이미 한쪽 값만 받아 나머지를 이어 쓰므로 같은 일을 하는 이중 경로였습니다.
- 테두리 longhand는 따로 ADR을 세워 storage·document 저장 key를 12개로 나누고(IX.11.22) text source에 longhand 한 줄 표기를 정한 뒤, 속성 행을 진짜 longhand key로 만듭니다. 제거는 CSS 그대로 `border-style: none` 또는 묶음 `border: none`으로 합니다.
