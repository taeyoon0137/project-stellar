# 기본값 세트(style defaults)

이 문서는 명시하지 않은 CSS longhand가 어떤 값으로 풀리는지 정합니다. 결정은 ADR-0082와 ADR-0086에 있습니다.

## 1. 범위

- 원본(텍스트 원본, chunk)에는 사용자가 명시한 longhand만 있습니다. 명시하지 않은 longhand는 활성 기본값 세트의
  값으로 풉니다.
- 이 표가 다루는 longhand는 명시·미설정 구분이 있는 layout longhand 40개입니다(`crates/document/src/layout_record.rs`
  `LayoutLonghand`).
  - payload 속성(`background-color`, 테두리, payload 크기 등)은 모델에서 언제나 값이 있는 명시 속성입니다.
    그래서 세트가 끼어들지 않습니다.
  - Frame·Rectangle·Ellipse·Polygon·Image의 `width`·`height`는 payload 크기이고, 언제나 명시 값입니다.
- 세트는 초기값만 정합니다. 상속 규칙은 CSS 그대로입니다(ADR-0082 결정 7).
- 해석 함수는 `stellar_document::css::style_defaults::resolve` 하나입니다. 레이아웃 입력, CLI 조회, codegen이 같은
  함수를 씁니다.

## 2. 내장 세트

| 세트 | 뜻 |
|---|---|
| `css` | 기본 세트입니다. 저자 스타일이 없을 때 브라우저가 계산하는 값입니다. CSS initial value에 HTML UA 값을 더했습니다 |
| `react-native` | React Native(Yoga) View·Text·Image의 기본 동작을 CSS longhand 값으로 적은 세트입니다. `css`를 물려받습니다 |

### 2.1 노드 종류 구획

| 구획 | node 종류 | 대응 요소 |
|---|---|---|
| `frame` | Frame, ComponentInstance | `div` / RN `View` |
| `text` | Text | `span`을 상자 하나로 배치 / RN `Text` |
| `image` | Image | `img`를 상자 하나로 배치 / RN `Image` |
| `shape` | Shape, Path | `svg` 상자 / RN `View` 안 도형 |

Fragment는 상자가 없어 구획이 없습니다.

### 2.2 표

값이 모든 구획에서 같으면 하나만 적습니다. 다르면 `구획=값` 목록입니다. `=`는 `css` 값을 물려받는다는 뜻입니다.
이 표는 `snapshot_text()`와 같아야 합니다(`spec_document_table_matches_snapshot` test).

| longhand | css | react-native |
|---|---|---|
| `position` | `static` | `relative` |
| `box-sizing` | `content-box` | `border-box` |
| `top` | `auto` | = |
| `right` | `auto` | = |
| `bottom` | `auto` | = |
| `left` | `auto` | = |
| `width` | `auto` | = |
| `height` | `auto` | = |
| `min-width` | `auto` | `0px` |
| `min-height` | `auto` | `0px` |
| `max-width` | `none` | = |
| `max-height` | `none` | = |
| `aspect-ratio` | `auto` | = |
| `flex-grow` | `0` | = |
| `flex-shrink` | `1` | `0` |
| `flex-basis` | `auto` | = |
| `align-self` | `auto` | = |
| `justify-self` | `auto` | = |
| `grid-column-start` | `auto` | = |
| `grid-column-end` | `auto` | = |
| `grid-row-start` | `auto` | = |
| `grid-row-end` | `auto` | = |
| `display` | `block` | frame=`flex`, text=`block`, image=`block`, shape=`block` |
| `padding-top` | `0px` | = |
| `padding-right` | `0px` | = |
| `padding-bottom` | `0px` | = |
| `padding-left` | `0px` | = |
| `flex-direction` | `row` | frame=`column`, text=`row`, image=`row`, shape=`row` |
| `flex-wrap` | `nowrap` | = |
| `justify-content` | `normal` | = |
| `align-content` | `normal` | frame=`flex-start`, text=`normal`, image=`normal`, shape=`normal` |
| `align-items` | `normal` | = |
| `justify-items` | `normal` | = |
| `row-gap` | `normal` | = |
| `column-gap` | `normal` | = |
| `grid-template-columns` | `none` | = |
| `grid-template-rows` | `none` | = |
| `grid-auto-columns` | `auto` | = |
| `grid-auto-rows` | `auto` | = |
| `grid-auto-flow` | `row` | = |

### 2.3 근거

- `css` 값은 CSS initial value입니다. 예외는 두 가지입니다.
  - `display`: CSS initial은 `inline`입니다. Stellar에는 inline formatting context가 없고 Frame은 `div`이므로
    `block`입니다(ADR-0076 결정 3, UA stylesheet).
  - `justify-items`: CSS initial은 `legacy`입니다. 정렬 keyword가 없는 `legacy`는 `normal`과 같은 결과라
    `normal`로 적습니다.
- `react-native` 값과 근거는 다음과 같습니다.

| longhand | 값 | 근거 |
|---|---|---|
| `position` | `relative` | RN 문서: 모든 View의 기본 position은 relative |
| `box-sizing` | `border-box` | RN 문서: `boxSizing` 기본값 `border-box` |
| `flex-shrink` | `0` | RN 문서: `flexShrink` 기본값 0 |
| `min-width`·`min-height` | `0px` | Yoga 동작: flex item 자동 최소 크기가 없음 |
| `display`(frame) | `flex` | RN 문서: View는 flex container |
| `flex-direction`(frame) | `column` | RN 문서: `flexDirection` 기본값 `column` |
| `align-content`(frame) | `flex-start` | RN 문서: `alignContent` 기본값 `flex-start` |

- 확인하지 못한 항목은 `css` 값을 그대로 물려받습니다.
  - Text·Image·Shape 구획의 `display`·`flex-direction`·`align-content`가 여기에 해당합니다. 자식이 없는 leaf라
    배치 결과에 영향이 없다고 보고 `css` 값을 둡니다.
  - `align-items`(RN `stretch`)와 `justify-content`(RN `flex-start`)는 flex container에서 CSS `normal`과 결과가 같아
    따로 적지 않습니다.
- RN 값은 Yoga를 실행한 fixture로 고정하지 못했습니다. 대신 conformance test가 다음을 확인합니다.
  - `react-native` 세트로 푼 문서와, 같은 값을 `css` 세트 위에 명시한 문서가 같은 rect를 냅니다.
  - `css` 세트의 `content-box` 결과가 기준 브라우저(Chrome) 기대값과 같습니다.

## 3. 선택: `stellar.yaml`

```yaml
style_defaults: react-native
```

- 키가 없으면 `css`입니다. 값은 내장 세트 이름이거나 사용자 세트 이름입니다.
- 이 키를 모르는 옛 앱은 매니페스트의 모르는 키 거부 규칙대로 프로젝트를 열지 않습니다(ADR-0082 Compatibility).
- 바꾸는 방법은 두 가지입니다.
  - 앱 편집 history: `SetStyleDefaults` command입니다. undo 한 번으로 되돌립니다.
  - headless: `stellar style-defaults set <프로젝트> <세트>`입니다. Git diff에서 한 줄로 보입니다.

## 4. 사용자 세트 파일

경로는 `design/style-defaults/<name>.stellar.yaml`입니다. `<name>`은 `[a-z][a-z0-9-]*`(64자 이하)이고 내장 세트
이름은 쓸 수 없습니다.

```yaml
extends: react-native
flex-shrink: 1
frame:
  row-gap: 8px
  column-gap: 8px
text:
  min-width: auto
```

- `extends`는 필수입니다. 사슬은 내장 세트에서 끝나야 하고 사용자 세트는 최대 16단계입니다.
- 나머지 최상위 키는 longhand 이름이고 값은 그 longhand의 CSS 값 한 줄입니다.
- `frame`·`text`·`image`·`shape` 구획은 그 종류에만 적용됩니다.
  - 같은 세트 안에서는 구획 값이 공통 값보다 우선합니다.
  - 자식 세트의 값은 부모 세트의 어떤 값보다 우선합니다.
- 다음은 거부합니다.
  - 순환하는 `extends`, 없는 세트, 모르는 longhand, 모르는 구획 이름, 한 구획에 같은 longhand 두 번
  - 문법 오류, `var()`, 계산식, 전역 keyword(`inherit` 등). 기본값은 값 자체여야 합니다.
- 파일 크기와 깊이 제한은 다른 텍스트 원본과 같습니다(제한 YAML loader).

## 5. 출처 표시

조회 결과는 값마다 출처를 돌려줍니다.

| 출처 | 뜻 |
|---|---|
| `explicit` | 사용자가 명시한 값(layout record 또는 payload) |
| `default(<세트>)` | 활성 세트의 값 |

- 명시한 값이 세트 값과 같아도 `explicit`입니다. 저장할 때 자동으로 지우지 않습니다.
- 명시를 지우는 일은 `UnsetProperty` command입니다.
- CLI: `stellar style-defaults resolve <프로젝트> <문서 id> <node id>`

## 6. codegen 차이 출력

생성기는 대상 고유 기본값과 활성 세트의 차이만 명시 선언으로 냅니다(ADR-0082 결정 6). 차이 목록은
`style_defaults::differences(active, target, 구획)`이 돌려줍니다.

- Web(대상 고유 기본값 `css`)에서 활성 세트가 `react-native`이면 frame 구획 차이는 다음과 같습니다.
  - `position: relative; box-sizing: border-box; min-width: 0px; min-height: 0px; flex-shrink: 0`
  - `display: flex; flex-direction: column; align-content: flex-start`
- 실제 출력(`:where()` 기반 규칙, RN 스타일 객체)은 codegen-css lane이 이 API로 만듭니다(ADR-0086).
