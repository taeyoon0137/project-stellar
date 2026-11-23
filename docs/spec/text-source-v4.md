# 텍스트 원본 쓰기 v4

[ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md)의 형식입니다. 단일 View 원본(`stellar text-source export|check`), 컴포넌트 원본(`stellar component-source`)이 이 버전만 씁니다. View v1·v2와 컴포넌트 v1~v3은 읽기 전용입니다([단일 View v1·v2](text-view-source-v1.md), [컴포넌트 v1~v3](component-source-v1.md)).

구현 원본은 [`source_v4/`](../../crates/project/src/document_io/source_v4/)입니다.

## 공통 규칙

- 문법은 기존 제한 YAML 하위 집합과 checked canonical writer입니다(파일당 1 MiB, 중첩 64). 모든 scalar는 문자열로 읽고 codec이 해석합니다.
- 영속 속성 하나 = 키 하나·한 줄입니다. 키는 document 속성 registry(`frame_metadata`·`payload_metadata`·`modifier_metadata`)의 `name`이고 줄 순서는 registry 순서입니다. registry가 이름을 바꾸면 파일 키도 따라갑니다.
- 모르는 키·빠진 키·중복 키·모르는 값·모르는 버전은 거부합니다. 텍스트로 정확히 복원할 수 없는 의미가 있으면 쓰지 않습니다.
- node 목록은 StableId 오름차순이고 형제 표시 순서는 `order_key`가 보존합니다. modifier 목록은 `order` 순서입니다.

## 값 문법

| 종류 | 문법 | 예 |
|---|---|---|
| 길이 | `<number>px` | `1440px` |
| 각도 | `<number>deg` | `-32.5deg` |
| 백분율·시간 | `<number>%`, `<number>ms` | `50%` |
| 수 | 최단 10진(지수 없음) | `0.5` |
| 정수 | 10진(앞자리 0 없음) | `9` |
| 색 | `color(srgb r g b)`, `color(display-p3 r g b)`, 알파가 1이 아니면 `/ a` | `color(display-p3 0.25 0.5 1 / 0.75)` |
| 행렬 | `matrix(a, b, c, d, tx, ty)` | `matrix(1, 0, 0, 1, 12.5, -4)` |
| 참조 | 소문자 하이픈 UUID, 없으면 `none` | |
| 참조 목록 | 공백 구분 UUID, 비면 `none` | |
| 순서 키 | byte당 소문자 hex 두 자리 | `80` |

float은 `f32`가 정확히 왕복하는 최단 표기만 받습니다. `1440.0px`, `01`처럼 같은 값의 다른 표기는 거부합니다.

## node

```yaml
-
  id: "<StableId>"
  parent: none | "<StableId>"
  order_key: "80"
  visible: "true"
  locked: "false"
  display_name: "..."
  kind: frame | fragment | rectangle | ellipse | line | polygon | path | text | image | component-instance
  payload: <종류별>
  modifiers: []      # 문서 node만. 컴포넌트 정의 tree node에는 없습니다.
```

### Frame

registry 순서의 키입니다. 지금 registry 이름으로는 `transform`, `clipping`, `background`, border longhand 열두 줄, `layout-mode`, `width`, `height`, `border-top-left-radius`, `border-top-right-radius`, `border-bottom-right-radius`, `border-bottom-left-radius`입니다.

- `background`: 색 또는 `transparent`(배경 없음).
- 테두리: `border-{top,right,bottom,left}-{width,style,color}` 열두 줄. 모델은 네 변 공통 테두리 하나이므로 네 변이 같아야 합니다. 테두리가 있으면 `<길이>`·`solid`·`<색>`, 없으면 CSS 초기값 `medium`·`none`·`currentcolor`입니다.
- `clipping`(stellar-only): `disabled`, `frame-bounds`, `path <StableId>`.
- `layout-mode`(stellar-only): 참조 또는 `none`.

### 도형

| `kind` | 키 |
|---|---|
| `rectangle` | `width`, `height`, 네 모서리 반경 |
| `ellipse` | `width`, `height`, `origin-x`, `origin-y` |
| `line` | `x1`, `y1`, `x2`, `y2`, `stroke-linecap`(`butt`·`round`·`square`) |
| `polygon` | `width`, `height`, `polygon-sides`, `polygon-rotation`(deg), `polygon-corner-radius`, `origin-x`, `origin-y` |

`origin-x`·`origin-y`는 registry에 key가 없는 경계 원점입니다(codec 키).

### Path

```yaml
payload:
  fill-rule: nonzero | evenodd
  commands:
    - "M <point> x y"
    - "L <point> x y"
    - "C <handle1> <handle2> <point> c1x c1y c2x c2y x y"
    - Z
```

명령 하나가 한 줄이고 point·handle StableId를 보존합니다.

### Text

```yaml
payload:
  content: "확인\n다음"
  paragraphs:
    - "0"
    - "0"
  style-runs:
    - "0 2 0"      # start end style (Unicode scalar offset)
```

### Image

registry 키 `width`, `height`, `object-fit`(`fill`·`contain`·`cover`·`none`), `image-rendering`(`auto`·`pixelated`) 뒤에 codec 키 `asset`(asset StableId), `crop`(`x y width height`), `color-space`(`none`·`srgb`·`display-p3`)를 씁니다.

### 컴포넌트 인스턴스

```yaml
payload:
  component: "<StableId>"
  property-overrides:
    -
      path: none | "<StableId> ..."
      property: "<StableId>"
      type: boolean | number | string | length | percentage | angle | duration | color
      value: "true"
  slot-overrides:
    -
      slot: "<StableId>"
      content: none | "<StableId> ..."
```

이 목록 밖의 값 타입(enum·gradient·shadow·typography·resource-ref)은 쓰지 않습니다.

### modifier

```yaml
-
  id: "<StableId>"
  kind: transform | boolean | stroke-expand | corner | mask | clip | fill | stroke | shadow | blur | blend | opacity
  enabled: "true"
  order: "40"
  <registry 키>: <값>
```

공통 키 `enabled`·`order` 뒤에 그 종류의 registry 키를 씁니다(지금 이름 예: `fill.color`, `stroke.color`·`stroke.width`, `shadow.color`·`shadow.offset`(`<x>px <y>px`)·`shadow.blur`, `blend.mode`(`normal`·`multiply`·`screen`·`overlay`·`darken`·`lighten`), `boolean.op`(`union`·`subtract`·`intersect`·`exclude`)·`boolean.operands`).

## 단일 View 원본

```yaml
schema_version: "4"
document:
  id: "<StableId>"
  display_name: "..."
  text-styles: none | <Text style 표>
page:
  id: "<StableId>"
  display_name: "..."
view:
  id: "<StableId>"
  display_name: "..."
  root_frame: "<StableId>"
nodes: [...]
```

문서·페이지·View가 하나씩이어야 합니다. 등록 글꼴이 있는 Text style 표는 asset catalog가 필요해 이 파일로 쓰지 않습니다.

### Text style 표

```yaml
document: "<문서 StableId>"
color: inherit | <색>
default-typography: none | "<StableId>"
fonts: []                  # 문서가 등록한 글꼴 file hash 목록(asset catalog 참조)
typography:
  -
    id: "<StableId>"
    font-family: none | "\"Pretendard\", \"system-ui\""
    font-size: medium | <px>
    font-stretch: "100%"
    font-style: normal | italic | oblique <deg>
    font-weight: "400"
    letter-spacing: normal | <px> | <em>
    line-height: normal | <수> | <px>
paragraphs:
  -
    index: "0"
    line-height: auto | <수> | <px>
    text-align: start | center | end | justify
    typography: none | "<StableId>"
    wrapping: wrap | nowrap
runs:
  -
    index: "0"
    color: inherit | <색>
    font-feature-settings: normal | "\"liga\" 1, \"kern\" 0"
    font-variation-settings: normal | "\"wght\" 400"
    letter-spacing: inherit | normal | <px> | <em>
    typography: none | "<StableId>"
```

고정 face를 지정한 family와 해석할 수 없는 typography bytes는 쓰지 않습니다.

## 컴포넌트 원본

```yaml
format: stellar-component
schema_version: "4"
component:
  id: "<StableId>"
  name: "..."
  root: "<root Frame StableId>"
  local-tokens: none | "<TokenRef> ..."
nodes: [...]               # root 포함, StableId 오름차순, modifiers 없음
```

## Git 병합

줄이 병합 단위입니다. 사이에 바뀌지 않은 줄이 있는 두 속성(예: `background`와 `width`)은 일반 `git merge`로 합쳐집니다. registry 순서에서 바로 이웃한 두 줄(`width`·`height`, 이웃한 모서리)을 서로 다른 브랜치가 바꾸면 Git 텍스트 충돌로 남고 자동 해결은 merge driver(XXIV.2.1) 이후입니다.
