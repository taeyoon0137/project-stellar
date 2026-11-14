# 저장된 단일 View의 고정 Frame·Rectangle HTML+CSS

상태: 제한된 제품 경로 구현. 일반 `stellar` CLI의 `export-web` 명령입니다.
Phase XXII 전체 코드 생성, XXIV.1.4 일반 render/export 완료를 뜻하지 않습니다.

## 입력과 실행

```sh
stellar query /path/to/project project
stellar export-web /path/to/project DOCUMENT_UUID VIEW_UUID > /tmp/view-result.json
# 위 명령의 종료 코드가 0일 때만 추출합니다.
jq -jer '.result.html' /tmp/view-result.json > /tmp/view.html
```

입력은 `ProjectSnapshot`이 연 **저장 generation의 명시한 문서·View**입니다. 앱에서 편집 중인 active
document를 내보내려면 먼저 저장하고 그 문서 ID와 View ID를 지정합니다. CLI가 앱의 활성 창을 추측하거나
미저장 편집을 읽지 않습니다. `ViewDefinition.root_frame` 아래만 출력하며 다른 View의 node는 포함하지
않습니다. 문서 loader는 같은 문서의 모든 View를 검증하므로, 선택 밖 View의 저장 손상까지 무시하지는 않습니다.

ADR-0059는 텍스트 원본 전환 방향을 정하지만 현재 기본 loader는 여전히 generation입니다.
`text-source check/apply`의 opt-in 파일을 이 명령으로 직접 열거나 기본 저장 경로를 전환하지 않습니다.

출력은 기존 `stellar-cli/result` v1 JSON envelope입니다. `result`는 `revision`, `document`, `view`,
`root_frame`, `format: "fixed-free-frame-html"`, `html`을 가집니다. `html`은 doctype·UTF-8 meta·viewport·
내장 CSS를 가진 독립 문서입니다. 외부 stylesheet·script·font·asset·네트워크 요청이 없습니다.
CLI 자체는 파일을 만들거나 Git·프로젝트를 수정하지 않습니다. 리다이렉션 위치는 호출자가 정합니다.

새 프로젝트가 저장하는 문서 manifest v2의 기본 글꼴 표는 Frame과 Rectangle의 기하·색에 영향을 주지 않습니다. 선택한 View에 실제 Text 레이어가 있으면 해당 노드 ID와 함께 오류를 반환합니다.

## 허용 부분집합과 근거

정책은 ADR-0024·0025와 `docs/design/property_model.md` 3·4.6·5·8.2절, 실제 값은
`crates/document/src/frame.rs`, `records.rs`, `fragment.rs`, `core-types/src/color.rs`가 기준입니다.
현재 저장 모델의 값만 투영하고 property bundle·variant·longhand key·layout record를 정의하거나 바꾸지
않습니다. longhand는 출력 CSS의 표현이며 새 문서 스키마가 아닙니다.

| 저장된 입력 | 출력 |
|---|---|
| `layout_mode: None` Frame | 고정 Free 배치. root는 `position: relative`, 자식은 `absolute` |
| `size` | `width`, `height` px; `box-sizing: border-box` |
| 자식 affine의 tx·ty | `left`와 `top`에 이동의 정수부에서 가장 가까운 Frame 부모의 border 두께를 뺀 값을 둡니다. 소수부는 같은 부모 좌표계의 CSS matrix 이동에 둡니다. Fragment는 좌표계를 만들지 않습니다 |
| root affine의 tx·ty | 페이지 캔버스 배치이므로 생략. 다른 선형 성분은 보존 |
| affine a·b·c·d | `matrix(a, b, c, d, tx소수부, ty소수부)`와 `transform-origin: 0px 0px`. root의 마지막 두 값은 0입니다. 회전·scale 분해 없음 |
| 단색 background | `background-color`. 없음은 `transparent` |
| border | 네 변의 width·style·color longhand. 동일 두께, `solid`; 없음은 0px·none·transparent |
| 네 corner radius | 좌상·우상·우하·좌하 각각 px longhand. CSS의 겹침 반경 정규화 적용 |
| `Clipping::Disabled` | `overflow-x/y: visible` |
| `Clipping::FrameBounds` | `overflow-x/y: clip; overflow-clip-margin: border-box`로 바깥 경계까지 자식을 표시하고, 둥근 모서리가 있으면 `clip-path: border-box`로 그 윤곽을 적용 |
| `visible: false` | `display: none`과 기존 접근성 mapper의 숨김 처리. Fragment 조상 숨김도 상속 |
| Fragment | element·좌표계 없이 자식을 제자리에서 펼침 |
| OrderKey | DOM 순서. 모든 Frame에 `z-index: 0`으로 stacking context 생성 |
| StableId | 기존 `web::aria::element_id`의 안정적인 ID |
| 표시 이름·잠금 | 편집 메타데이터. 이름을 Text·button·role·label로 해석하지 않음 |

색은 저장된 sRGB/Display P3 태그와 **straight RGBA**를 유지합니다. 속성 모델 8.2에 따라 모든 채널이
8-bit 왕복되는 sRGB만 hex로 출력하고, 나머지는 `color(srgb ...)` 또는 `color(display-p3 ...)`입니다.
확장 RGB 채널도 clamp하거나 색공간을 변환하지 않습니다. 숫자는 기존 codegen의 최단 f32 왕복 formatter를
사용합니다. 이는 선언 값의 보존이며 브라우저의 gamut mapping·래스터화·alpha 합성 픽셀까지 동일하다는
뜻은 아닙니다. 부분 투명 색은 node ID가 있는 `browser-color-compositing` 경고를 반환합니다.

브라우저 기준은 **Chromium, DPR 1, zoom 100%**, CSS Color 4와
`overflow-clip-margin: border-box`와 `clip-path: border-box` 지원입니다. 고정 CSS 값의 범위를 다음처럼 제한합니다.

- 크기·자식 위치·반경은 절댓값 1,000,000px 이하입니다. 크기·반경은 모델처럼 비음수입니다.
- affine 선형 성분은 절댓값 1024 이하이며 누적 변환 후 네 모서리는 유한한 ±1,000,000px 안이어야 합니다.
  출력하지 않는 root 캔버스 tx·ty에는 이 제한을 적용하지 않습니다.
- 테두리는 비음수 **정수 px**이고 두 배 두께가 너비·높이를 넘지 않아야 합니다. 소수 폭은 device-pixel
  snapping으로 바뀌고, 과도한 두께는 CSS의 최소 border box가 저장 크기보다 커지므로 거부합니다.
- 위치의 정수부는 0 방향으로 절삭합니다. 소수부는 CSS transform에서 더하므로 확대 전에 LayoutUnit으로 반올림되지 않습니다.
- 너비·높이가 1/64px 격자 밖이면 인접한 두 격자점 중 먼 쪽까지의 거리를 사용해 누적 affine 이후 각 축의 오차 상한을 계산합니다.
  화면에서 1/64px를 넘을 수 있으면 해당 node에 `unsupported-frame-value`를 반환합니다. 예를 들어 128배 확대에서
  너비 10.005px는 브라우저가 10px로 배치해 화면 너비가 0.64px 달라지므로 거부합니다. zoom·DPR 변경이나
  모든 브라우저 버전에 대한 적합성을 보장하지 않습니다. 출력 문서에 다른 CSS를 주입하는 경우도 범위 밖입니다.

CSS 근거: [border width 계산](https://www.w3.org/TR/css-backgrounds-3/#border-width),
[overflow clip edge](https://www.w3.org/TR/css-overflow-3/#overflow-clip-margin),
[색공간과 color()](https://www.w3.org/TR/css-color-4/#color-function).

## Rectangle Shape

실제 `ShapeGeometry::Rectangle`도 같은 고정 CSS formatter로 출력합니다. `size`와 네 개의
`corner_radius`는 geometry에서 읽고 CSS의 반경 겹침 정규화를 적용합니다. Shape는 border·자식 clip이
없으며 Fill이 없으면 투명합니다. 이름에서 접근성 label을 만들지 않는 장식용 `div`입니다.

로컬 main의 `runtime::canvas::shape::rectangle_style`과 동일하게 OrderKey 순서의 **활성 Transform
0개 이상 → 활성 단색 Fill 최대 1개**만 허용합니다. Transform은 `identity.then(first).then(next)`로
합성합니다. Fill 뒤 Transform, 두 번째 Fill, Stroke·Opacity·그 밖의 활성 modifier는 오류입니다.
비활성 항목은 실제 renderer처럼 건너뜁니다. Frame의 modifier 거부 규칙은 그대로 유지합니다.
색공간·straight RGBA·부분 투명 경고·부모 Frame border 보정·숨김·선택 View 범위·숫자 한계는
기존 Frame과 같습니다. 실제 renderer처럼 누적 affine의 역변환이 없는 Rectangle은 거부합니다.

저장된 generation을 여는 실제 CLI fixture는 modifier 순서·비활성·무채우기·색·반경·변환·실패·
선택 밖 subtree·프로젝트 바이트 불변을 검사합니다. snapshot은 이후 modifier를 저장해도 고정됩니다.
`rectangle_saved_cli` 테스트에 `STELLAR_WEB_EVIDENCE_DIR`을 지정하면 호스트 브라우저 비교용
`rectangle.html`과 CLI envelope `rectangle.json`을 해당 디렉터리에 남깁니다. 변환된 실효 반경에서 모서리의 안티앨리어싱은 브라우저와 캔버스에 따라 달라질 수 있습니다. 둥근
Rectangle은 node ID를 포함한 `browser-rectangle-curve` 경고를 반환합니다. 직각 Rectangle에는
이 경고를 반환하지 않으며, 경고만으로 HTML 출력을 실패시키지 않습니다.

## 실패와 한계

선택 View의 숨겨진 node도 검증합니다. 오류가 하나라도 있으면 `html: null`, 종료 코드 1,
`web-export-failed`이며 부분 HTML을 성공 결과로 반환하지 않습니다. 진단 message에 해당 node ID를 넣습니다.

| 조건 | 진단 |
|---|---|
| 저장된 layout 참조(정의·해석 경로 없음) | `unsupported-layout` |
| Text·Rectangle 이외 Shape·Path·Image | `unsupported-node` |
| Frame modifier stack(비활성 항목도 포함), Rectangle의 미지원 활성 modifier·순서 | `unsupported-modifier` |
| Path clipping | `unsupported-clip` |
| 숫자·테두리·변환 범위 위반 | `unsupported-frame-value` |
| 설정된 target profile(저장 profile ↔ capability pin 연결 없음) | `unsupported-target-profile` |
| document-manifest/view-tree 외 chunk | `unsupported-record` |
| 문서 manifest schema 1·2 또는 View/subtree schema 1을 벗어난 경우 | `unsupported-record-schema` |
| 기존 접근성 mapper/정적 검사 오류 | `accessibility-mapping`, `html-accessibility-check` |

해석하지 못하는 project chunk·profile은 선택 View에 대한 영향 범위를 판정할 수 없으므로 **선택 root ID**에
진단하고 보수적으로 실패합니다. node 의미·control·binding·token·Variant·Context·Motion을 빈 가상 데이터로
채우지 않습니다. 현재 지원하는 Frame/Fragment/Rectangle 저장 레코드에는 그런 의미가 연결되지 않아 `fixed-free-frame-only`
범위 경고를 항상 출력합니다. Text는 문자열만으로 글꼴·paragraph·run 스타일을 복구할 수 없어 거부합니다.
반응형 layout·동작하는 control·Text UI는 후속 구현입니다. 기존 `export-accessibility`는 별도 명령으로 유지됩니다.

없는 프로젝트·저장 generation·문서·View는 66, 손상·미지원 generation schema는 65, 인자 오류는 64입니다.
인자 오류는 기존 CLI 규칙대로 stdout에 JSON을 출력하지 않습니다.

## 구현 경계와 검증

`ops::web_frame`이 저장 model을 읽고 선택 범위와 지원 여부를 판단합니다. 구조는 기존 접근성 생성기 →
snapshot → `build_accessibility_ir` → `web::aria::map_accessibility` → 정적 검사를 사용합니다.
`web::frame::FrameStyle`은 요소 하나의 core 값만 받는 CSS formatter 입력입니다. 새 트리 IR·PropertyKey·
capability registry·저장 형식이 아니며 `CodegenSources`의 없는 layout/profile을 꾸며내지 않습니다.
의존 간선과 외부 의존성은 늘리지 않습니다.

```sh
export CARGO_TARGET_DIR="$PWD/target/web-frame"
cargo test -p stellar-codegen -p stellar-ops -p stellar-cli
STELLAR_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
STELLAR_WEB_EVIDENCE_DIR="$CARGO_TARGET_DIR/evidence" \
cargo test -p stellar-cli --test web_frame -- --include-ignored --nocapture
```

일반 테스트는 실제 두 View를 저장하고 실제 `stellar` subprocess에서 HTML golden·반복 stdout/stderr·
revision·선택 범위·실패·프로젝트와 Git index/refs/objects 바이트 불변을 검사합니다. 별도 snapshot 테스트는
다른 저장 이후에도 이전 revision이 고정되는지와 root 이동만 바꿔도 HTML이 같음을 확인합니다.
Chrome 테스트는 두 기본 View, 고배율 소수 이동 View와 두꺼운 테두리·비대칭 모서리 View에 별도 임시 profile을 사용하고 테스트에만 script를 주입해 computed style·bounding rect·
elementFromPoint를 확인합니다. 제품 HTML에는 script가 없습니다. 실행한 Chrome 버전과 JSON 증거는 위
evidence 디렉터리에 남습니다. 이 테스트가 ignore 상태인 일반 테스트만 실행한 경우 브라우저 검증 완료로
보고하지 않습니다.
