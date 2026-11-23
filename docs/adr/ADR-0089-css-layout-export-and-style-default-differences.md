# ADR-0089: 저장 View 내보내기에 layout longhand와 기본값 세트 차이를 싣는다

## Status

Accepted (2026-11-23).

- ADR-0082 결정 6(codegen 차이 출력)과 ADR-0086 결정 12(codegen-css lane에 넘긴 출력)를 구현하는 결정입니다.
- 계획 행 IX.11.30(SD.7)과 XXII.2.2의 Web·CSS 부분이 대상입니다.

## Context

- 저장 View를 HTML+CSS로 내보내는 경로는 `ops::web_frame::export_web_frame_node` 하나입니다(`stellar export-web`,
  MCP `document.export_web_frame`). 이 경로는 고정 Free Frame만 냈고 layout record(ADR-0086)를 읽지 않았습니다.
  - 그래서 `display: flex` 같은 명시 값이 있는 Frame도 기록이 없는 것처럼 나갔습니다.
- 생략한 longhand는 활성 기본값 세트(ADR-0082)로 풉니다. 대상 플랫폼에도 고유 기본값이 있습니다.
  - Web 브라우저의 기본값은 내장 `css` 세트와 같습니다.
  - React Native의 기본값은 내장 `react-native` 세트와 같습니다.
  - 출력은 이 둘의 차이만 명시해야 실제 코드와 가까워집니다.
- 문서 모델에는 아직 위치·transform 분리(IX.11.9)가 없습니다.
  - Frame 위치는 저장 transform의 이동 성분입니다.
  - 캔버스는 아직 CSS 레이아웃 결과로 그리지 않습니다(SD.2).
- Stellar Frame border는 레이아웃 공간을 차지하지 않습니다(scene이 Inside로 그림, 속성 모델 5장 border 행).
  CSS border는 공간을 차지합니다.
- crate 그래프상 `stellar-codegen`은 문서 crate를 읽지 못합니다. 값 해석·formatter는 문서 crate에 있습니다.

## Decision

1. **경로는 하나입니다.** 새 tree walk·값 parser·formatter를 두지 않습니다.
   - `export_web_frame_node`가 node마다 `ops::export_css::plan_element`를 부릅니다.
   - 값은 문서 해석 함수 `style_defaults::resolve_kind`로 풉니다. CSS 텍스트는 문서 formatter
     (`layout_record::format_value`, `css::value::format_length`)로 씁니다.
   - `stellar-codegen`은 규칙 텍스트 조립(`web::layout`)과 RN 낮춤 규칙(`react_native`)만 맡습니다. 둘 다 문서
     타입을 받지 않고 이름·정규 값 텍스트만 받습니다.
   - React Native 출력(`export_css::export_react_native`)도 같은 Web 투영의 계획을 씁니다. 검증·진단도 Web과
     같습니다.
2. **출력 형식 두 개.**
   - `fixed-free-frame-html`: 출력 tree에 layout record가 하나도 없고, 활성 세트가 출력 구획에서 `css` 세트와
     같은 경우입니다. 기존 고정 Free Frame 출력 그대로입니다. 기존 golden과 bytes가 같습니다.
   - `css-layout-html`: 그 밖의 경우입니다. 결정 3·4·5의 규칙으로 냅니다.
   - 결과에 `format`을 싣습니다. CLI `export-web`과 MCP도 이 값을 그대로 냅니다.
3. **요소 규칙에는 명시 값만 씁니다.**
   - layout longhand는 명시 값(layout record, payload 크기)만 요소 규칙(`#<id>`)에 씁니다.
   - 예외는 배치 규칙(결정 4), border 보정(결정 6), 숨김(`display: none`)이 정한 값입니다.
   - 명시 값은 세트 값과 같아도 씁니다(ADR-0082 결정 5).
   - paint 선언(`z-index: 0`, transform, clip, 배경, 테두리, 반경)은 기존 formatter(`FrameStyle`)가 layout 선언
     뒤에 붙입니다.
4. **배치 규칙(IX.11.9 전 연결 규칙).** 부모 상자는 Fragment를 건너뛴 가장 가까운 상자입니다.
   - 부모가 흐름 부모이고 자식의 `position`이 `static`·`relative`이면 흐름 item(`flow`)입니다.
     - 흐름 부모는 풀린 `display`가 `flex`·`grid`이거나 `display`를 명시한 부모입니다.
     - 자식이 `position`을 명시해도 흐름 item입니다.
   - 그렇지 않고 inset을 명시했으면 `inset`입니다. `position: absolute`에 record의 inset을 씁니다.
   - 둘 다 아니면 `free`입니다. 기존 Free 자식처럼 transform 이동을 `position: absolute` + `left`·`top`으로 냅니다.
     IX.11.9 행이 정한 migration 방향(Free 자식의 이동 → `left`·`top`)과 같습니다.
   - `flow`·`inset`의 transform 이동은 시각 이동으로 matrix에 남깁니다. IX.11.9의 flow 자식 `translate`와 같습니다.
   - 내보내는 root는 `root`입니다. 부모가 없어 item longhand(inset, `flex-*`, `align-self`, `justify-self`,
     `grid-*-start`·`-end`)를 내지 않고 `ignored-root-item-longhand` 정보 진단을 냅니다.
   - 모든 요소를 positioned로 내는 stacking 규칙(속성 모델 3장)에 따라 `static`은 `relative`로 냅니다.
     - `static`에서 효과가 없던 명시 inset은 `relative`에서 효과가 생기므로 내지 않습니다
       (`ignored-static-inset`).
   - `free` 요소가 있으면 `fallback-free-position`, `flow`·`inset` 요소가 있으면 `layout-not-on-canvas` 정보
     진단을 root에 냅니다.
5. **기본값 차이는 특이도 0 기반 규칙입니다**(ADR-0082 결정 6).
   - 활성 세트가 `css` 세트와 다른 longhand만 냅니다(`style_defaults::differences`).
   - 출력에 나오는 구획 모두에서 같은 값인 차이는 `:where(#<root>, #<root> *) { … }` 하나에 둡니다.
   - 구획마다 다른 차이는 `:where(#<root>.stellar-<구획>, #<root> .stellar-<구획>) { … }`에 둡니다. 그때만 요소에
     `class="stellar-<구획>"`을 붙입니다.
     - `class`는 HTML 전역 속성이라 정적 접근성 검사의 허용 속성에 더했습니다.
   - `:where()`의 특이도는 0이고 요소 규칙 `#<id>`는 1-0-0이므로 저자 규칙이 언제나 이깁니다.
   - 차이가 없으면 기반 규칙도 class도 내지 않습니다.
   - root 선택자는 ADR-0082 예시의 class 대신 root 요소 id를 씁니다. `:where()` 안이라 특이도가 같고, 새 속성이
     필요 없습니다.
6. **Stellar border 보정.** Frame에 border b가 있으면 CSS border가 공간을 차지하지 않은 것과 같은 상자가 되도록
   다음 세 가지를 바꿉니다.
   - `box-sizing: border-box`를 냅니다.
   - 풀린 값이 `content-box`이고 그 축의 padding 합 p가 0보다 크면 크기 longhand를 바꿉니다
     (`width`·`height`·`min-*`·`max-*`·부모 main 축의 `flex-basis`).
     - `<length>` L → `(L + p)px`
     - `<percentage>` P → `calc(P% + ppx)`
   - padding은 각 변에서 b를 뺍니다(0 미만은 0). 흐름 부모이고 padding이 b보다 작으면 `approximated-border-padding`
     진단을 냅니다(속성 모델 5장 border 행의 migration 규칙과 같음).
   - `inset` 요소의 px inset은 부모 border를 빼 `calc(<v> - <b>px)`로 냅니다. 백분율 inset은 그대로 내고
     `approximated-border-inset`을 냅니다.
7. **React Native 출력.** 같은 계획을 `StyleSheet.create` 모듈로 냅니다(`stellar export-styles --target react-native`).
   - longhand는 명시 값이거나 활성 세트 값이 `react-native` 세트와 다를 때만 냅니다. 그래서 `css` 세트 프로젝트는
     `flexShrink: 1`, `boxSizing: 'content-box'`, `position: 'static'` 같은 차이가 명시됩니다. `react-native` 세트
     프로젝트는 명시 값만 남습니다.
   - 낮춤 규칙(`codegen::react_native::lower`)은 Target Profile 상태(Native·Approximated·Unsupported)를 돌려줍니다.
     - CSS `normal` 정렬은 flex에서의 동작 값으로 바꿉니다(`justify-content` → `flex-start`, `align-*` → `stretch`,
       gap → 0).
     - `start`·`end`는 `flex-start`·`flex-end`로 근사합니다.
     - grid와 `justify-items`·`justify-self`, intrinsic 크기 keyword는 Unsupported입니다.
     - `display: block`과 `min-*: auto`는 Approximated이고 값을 쓰지 않습니다.
   - block container의 flex container longhand와 block 부모 자식의 flex item longhand는 CSS에서 효과가 없으므로
     내지 않습니다.
   - 배치: `free` → `position: 'absolute'` + `left`·`top`(저장 이동). `flow`·`inset`의 이동 → `transform` translate.
   - transform 선형 부분은 낼 수 없어 진단합니다. paint 속성은 내지 않습니다(`react-native-layout-only`).
   - RN 진단은 실패가 아닙니다. Web 투영에 오류가 있을 때만 모듈이 `null`입니다.
8. **CLI·MCP.**
   - CLI `stellar export-styles <프로젝트> <문서> <View> [--frame <id>] [--target web|react-native]`를 둡니다.
     - `web`: 기반 규칙과 요소별 선언(이름·값·출처 `explicit`·`placement`·`converted`·`visibility`)을 냅니다.
     - `react-native`: StyleSheet 모듈을 냅니다.
   - `stellar export-web`은 결과의 `format`을 그대로 냅니다.
   - MCP `document.export_web_frame`을 확장합니다.
     - 결과에 `format`, `style_defaults`, `base_rules`, `styles`를 더합니다.
     - 입력 `target: "react-native"`이면 `react_native` 모듈을 더합니다.
     - 새 tool 이름은 만들지 않습니다.
9. **활성 세트 원본.** 내보내기는 `stellar.yaml`의 `style_defaults`와 사용자 세트 파일을 직접 평탄화합니다
   (`manifest_style_defaults::active_set`). 문서 모델의 세트 이름 거울은 아직 열 때 채워지지 않기 때문입니다
   (ADR-0086 결정 10). 세트를 풀지 못하면 root에 `invalid-style-defaults` 오류를 냅니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| layout 전용 새 export 함수와 CLI 명령 | 채택하지 않음. tree walk·검증·진단이 두 벌이 됩니다 |
| 요소 규칙에 40개 longhand를 모두 명시 | 채택하지 않음. 실제 코드와 멀고, 세트를 바꿔도 출력 차이가 드러나지 않습니다 |
| 기존 고정 출력도 새 규칙으로 바꿈 | 채택하지 않음. record 없는 문서의 출력이 이유 없이 바뀌고 기존 golden·브라우저 확인이 무효가 됩니다 |
| 모든 자식을 CSS 흐름으로 냄(transform 이동을 무시) | 채택하지 않음. IX.11.9 전 Free 자식의 화면 위치를 잃습니다 |
| 구획 차이를 한 공통 규칙에 몰아 둠 | 채택하지 않음. `react-native`의 `display: flex`는 Frame 구획 값이라 Shape 요소의 계산값이 문서와 달라집니다 |
| codegen이 CSS 값 텍스트를 다시 parse해 보정 | 채택하지 않음. 값 parser가 두 벌이 됩니다. 보정은 typed 값을 가진 ops가 문서 formatter로 씁니다 |
| RN을 별도 MCP tool로 | 채택하지 않음. 작업 정의가 기존 export tool 확장을 정했고, tool 수와 권한 표가 늘어납니다 |

## Consequences

- record가 있는 View는 명시 layout 값과 세트 차이가 HTML+CSS·RN 스타일로 나갑니다. record가 없는 `css` 세트 View는
  예전과 같습니다.
- 흐름 배치는 캔버스와 다를 수 있습니다. 캔버스가 레이아웃 결과로 그리게 되면(SD.2) 같아집니다. 진단으로 알립니다.
- 한계가 있습니다. 후속 작업이 필요합니다.
  - Text·Image·Path 등 아직 export가 받지 않는 node는 기존처럼 실패합니다.
  - 이 출력은 CodegenIR(XXII.1.1)을 거치지 않습니다. IR이 layout record를 받게 되면 같은 계획을 IR lowering으로
    옮겨야 합니다.
  - RN 출력은 layout만 다룹니다. 요소 구조(View·Text JSX)와 paint는 XXII 후속 adapter 몫입니다.
  - `docs/spec/web-frame-export.md`는 이 lane 소유가 아니라 고치지 않았습니다. `css-layout-html` 형식 설명을 더해야
    합니다.

## Compatibility

- 저장 형식·FFI 변경은 없습니다.
- `WebFrameHtml`에 필드가 늘었습니다(`format`, `style_defaults`, `styles`, `base_rules`). f32를 담은 계획 때문에
  `Eq` 파생을 뺐습니다(`PartialEq`는 유지).
- CLI에 `export-styles` 명령이 늘었습니다. `export-web`의 `format`은 layout 출력에서 `css-layout-html`입니다.
- MCP `document.export_web_frame` 입력 schema에 선택 `target`이 늘었습니다. 기존 입력은 그대로 받습니다.

## Performance

- node마다 해석 한 번(40개 고정 배열 조회)과 선언 목록 하나를 더합니다. 세트 평탄화는 요청당 한 번입니다.
- 출력 크기는 layout 출력에서 명시 값만큼 늘고, 기본값 차이는 기반 규칙 하나로 끝납니다.

## Security

- 새 입력은 `stellar.yaml`·사용자 세트 파일이며, 기존 제한 loader와 경로 guard를 그대로 씁니다.
- 출력 값은 문서 정규 값과 고정 keyword뿐입니다. RN 문자열은 따옴표·역슬래시를 escape합니다.

## Migration

- 데이터 migration은 없습니다.
- IX.11.9가 들어오면 결정 4의 연결 규칙을 위치 longhand 해석으로 바꿉니다. 그때 `free` 배치는 migration된 `left`·`top`
  명시 값이 됩니다.
- IX.11.10이 들어와 border가 레이아웃 공간을 차지하게 되면 결정 6을 지웁니다.
