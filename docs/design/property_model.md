# 디자인 속성 모델(CSS 기반)

- 상태: 초안(2026-11-14)
- 결정: ADR-0024(CSS 기반 디자인 속성 어휘, 제품 소유자 방향 P3)
- 범위: 인스펙터가 편집하는 모든 디자인 속성의 정본 어휘, 기존 Stellar 속성과 CSS의 대응, 단위·값
  문법, 정규 텍스트 형식, 인스펙터 범주, Stellar 고유로 남는 부분
- 구현: 계획 IX.11(CSS 기반 속성 체계)
- 상위 기준: `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`(이하 "아키텍처"), ADR-0024,
  `docs/design/ui/inspector.md`(이하 "인스펙터 명세"), `docs/design/macos-shell-design.md`(이하 "가이드")

근거 표기는 가이드 0.1절을 따릅니다. `[아키텍처 §n]`, `[ADR-000n]`, `[계획 ID]` 외에 `[CSS <명세>]`는
W3C CSS 명세의 해당 모듈입니다(예: `[CSS Flexbox]`, `[CSS Grid]`, `[CSS Color 4]`). 코드 근거는
`crates/` 아래 파일 이름으로 적습니다.

이 문서는 아키텍처·계획·ADR을 바꾸지 않습니다. 어긋나면 그 문서들이 우선이고 이 문서를 고칩니다.

---

## 0. 요약

- Stellar의 디자인 속성 이름, 값 문법, 단위, 상자·레이아웃 의미는 CSS longhand를 따릅니다. [ADR-0024 결정 1]
- 저장은 바이너리 chunk와 숫자 `PropertyKey` 그대로입니다. CSS 이름은 registry metadata입니다. [ADR-0024 결정 2]
- Stellar는 CSS의 부분집합을 받아들입니다. 받아들인 선언의 결과는 CSS와 같아야 하고, 예외는 14장
  목록뿐입니다. [ADR-0024 결정 3]
- 영속 record는 longhand만 저장합니다. shorthand는 편집·입력·복사 표현입니다. [ADR-0024 결정 4]
- longhand 하나는 PropertyKey 하나입니다. 값 여러 개를 담은 지금의 합성 key는 longhand key로 나누고
  묶음 key로 남깁니다. [ADR-0024 결정 14]
- 매핑 비용이 가장 낮은 곳은 아직 영속 형식이 없는 레이아웃과 Typography입니다. 비용이 가장 높은
  곳은 Frame 위치·transform, Frame border, Text paragraph 속성, modifier stack 순서입니다.

## 1. 원칙과 표기

### 1.1 매핑 상태

각 registry 항목은 다음 네 상태 중 하나를 가집니다. [계획 IX.11.1]

| 상태 | 뜻 | Web export |
|---|---|---|
| `exact` | 같은 이름·값·의미 | 그대로 출력 |
| `converted` | 결정론적 변환으로 같은 결과(이름·값 형태가 다름) | 변환 후 출력 |
| `approximated` | CSS로 같은 결과를 보장할 수 없음 | 근사 출력 + 진단 |
| `stellar-only` | 대응 CSS 속성이 없음 | 구조 변환·path 굽기·래스터화 또는 출력 없음 |

추가 표시는 다음과 같습니다.

- `initial-differs`: 이미 저장되는 속성의 기본값이 CSS initial value와 다릅니다. export는 그 값을
  항상 명시적으로 출력합니다. [ADR-0024 결정 6]
- `subset`: CSS가 받는 값 일부를 Stellar가 거부합니다(구조화 진단). 받아들인 값의 의미는 같습니다.

### 1.2 migration 분류

| 분류 | 뜻 | 절차 |
|---|---|---|
| `none` | 변경 없음 | — |
| `rename` | registry 표시 이름·semantic diff 표시 문자열만 바뀜 | metadata 변경, 파일 바이트 불변 |
| `key` | 합성 key를 longhand key로 나눔(ADR-0024 결정 14) | 새 key 할당, 옛 key는 묶음 key로 유지, payload 바이트 불변, semantic snapshot 경로 갱신 [계획 IX.11.22] |
| `semantic` | 저장 바이트는 그대로, 엔진 해석이 CSS로 바뀜 | 엔진 수정 + corpus·golden 갱신 |
| `schema` | payload 구조가 바뀜 | 제품 문서 migration registry step + golden 갱신 [계획 IX.11.21, V.8.6] |

### 1.3 부분집합 규칙의 판정

- 기준 브라우저 엔진은 Chromium 계열 하나로 고정합니다. Chromium은 레이아웃 좌표를 1/64 px 고정소수
  (LayoutUnit)로 계산해 Stellar `LayoutUnit`과 해상도가 같습니다. Firefox는 1/60 px 단위를 써서 기준으로
  삼지 않습니다. 판정은 고정 버전으로 생성한 기대값 fixture로 합니다. [계획 IX.11.8]
- 기본 fixture는 남는 공간과 분배 몫이 1/64 px 격자에서 나누어떨어지는 입력만 씁니다. 이 fixture에서
  "같은 결과"는 같은 입력 트리와 같은 CSS 선언에서 border box rect가 1 LayoutUnit 안에서 같다는
  뜻입니다.
- 나누어떨어지지 않는 입력은 `distribution` 묶음으로 따로 둡니다. Stellar는 잔여 subpixel을 visual
  order 앞 자식부터 1씩 주고 가운데 정렬은 ties away from zero로 반올림합니다. [계획 X.1.5, X.3.3]
  브라우저의 분배·반올림은 이와 달라 자식이 많으면 위치 차이가 누적됩니다. 그래서 이 묶음의 허용
  범위는 한 줄(flex line 또는 grid 축 하나)의 item 수만큼의 LayoutUnit입니다. 이 차이는 반올림 범위로
  보고 14장 확장에 넣지 않습니다.
- 텍스트 줄 나눔·glyph 위치는 글꼴 엔진이 다르므로 이 판정에 넣지 않습니다. 텍스트 박스는 고정
  intrinsic 크기를 가진 대체 요소(replaced element)로 두고 판정합니다.

## 2. 단위와 값 문법

| Stellar 타입(`core-types`) | CSS 값 타입 | 정규 단위 | 입력에서 받는 형태 | 비고 |
|---|---|---|---|---|
| `Length` | `<length>` | `px` | `px`, 단위 없는 `0` | Stellar 논리 픽셀 = CSS px. 레이아웃은 1/64 px 정수 `LayoutUnit`, ties away from zero [계획 X.1.5] |
| `Percentage` | `<percentage>` | `%` | `%` | 저장 50 = 50%. 기준 길이는 속성별 CSS 정의를 따릅니다 |
| `Angle` | `<angle>` | `deg` | `deg`, `rad`, `grad`, `turn` | 입력은 deg로 정규화합니다. 단위 고정은 기존 바이트 의미를 바꾸지 않습니다(인스펙터가 이미 ° 표시) |
| `Duration` | `<time>` | `ms` | `ms`, `s` | 같음(인스펙터가 이미 ms 표시) |
| `Scalar`·Number | `<number>` | 없음 | 10진 수 | 지수 표기는 입력에서만 받습니다 |
| Enum | keyword | — | 대소문자 무시 ASCII keyword | 속성별 허용 keyword는 registry가 정합니다 |
| String | `<string>` | — | 따옴표 문자열 | |
| `Color` | `<color>` | 8.2절 | hex, `rgb()`, `color(srgb …)`, `color(display-p3 …)`, `transparent` | 이름 색(`red` 등)은 받되 sRGB로 정규화합니다 |
| Grid track | `<track-size>` | `px`, `fr` | 4.4절 | `fr`은 grid track에서만 씁니다 |

받지 않는 형태와 진단 코드는 다음과 같습니다. [ADR-0024 결정 5]

| 입력 | 진단 | 이유 |
|---|---|---|
| `em`(registry가 허용하지 않은 곳), `rem`, `vw`·`vh` 계열, `cqw` 계열, `ch`, `ex` | `unsupported-unit` | 기준 font-size·viewport·container를 Target Profile·환경 key로 아직 정의하지 않았습니다 |
| 길이와 백분율을 섞는 `calc()`, `min()`·`max()`·`clamp()` 안의 혼합 | `unsupported-value` | 산술 규칙표가 Length+Percentage를 금지합니다(`arithmetic.rs`). 15장 O3. Constraint CenterX·CenterY export(4.5절)가 이 형태라 붙여넣기 왕복에서 건너뜀 보고 대상입니다 [계획 IX.11.15] |
| `var()` 중 문서 scope에 없는 토큰 이름 | `unknown-token` | 10장 |
| `url()`, `@import`, `image()` 등 외부 리소스 | `forbidden-value` | 외부 리소스를 로드하지 않습니다 [ADR-0024 Security] |
| `inherit`·`initial`·`unset`·`revert` | `unsupported-value` | 문서 모델에 cascade가 없습니다. 기본값으로 되돌리기는 reset 명령이 맡습니다 |

`em`을 허용하는 곳은 CSS가 요소 자신의 font-size를 기준으로 정의하고 Stellar가 이미 상대값으로
저장하는 속성뿐입니다. 현재는 `letter-spacing` 하나입니다(7장).

## 3. 공통 레이어(`records.rs` `NodeRecord`)

| Stellar 속성 | CSS | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| `visible` | `visible = false` → `display: none` | `converted` | flow에서 빠지므로 `visibility: hidden`이 아닙니다 [계획 X.3.5] | `none` |
| `locked` | — | `stellar-only` | 편집 메타데이터 | `none` |
| `display_name` | — | `stellar-only` | 레이어 이름. export는 주석·식별자 힌트로만 씁니다 | `none` |
| StableId | — | `stellar-only` | 참조 식별자. Web export의 `id`·class는 StableId에서 결정론적으로 만듭니다 [계획 XX.3.3] | `none` |
| 자식 순서(`OrderKey`) | DOM 순서 = paint 순서 | `subset` | CSS `order`와 `z-index`를 따로 쓰지 않습니다. DOM 순서로 출력하는 것만으로는 paint 순서가 같지 않으므로 아래 stacking 규칙으로 냅니다. 14장 E3 | `none` |

Web export의 stacking 규칙(IX.11.18): CSS는 `z-index: auto`인 positioned 요소를 in-flow 비positioned
형제보다 나중 단계에서 그립니다. Free 부모는 `position: relative`, Free 자식은 `position: absolute`로
나가므로, 규칙이 없으면 Stack 안에서 Free Frame 뒤에 오는 일반 형제가 CSS에서는 Free Frame 아래에
그려집니다. Stellar에서는 형제가 위에 그려집니다. 그래서 export는 모든 요소가 stacking context를 만들고
같은 층(`z-index: 0`)에서 DOM 순서로 그려지게 합니다. flex·grid item과 positioned 요소에는
`z-index: 0`을, 그 밖의 요소에는 `position: relative; z-index: 0`을 씁니다. 이 규칙은 Blend의 합성
범위를 부모 stacking context 안으로 제한하므로 그 영향도 IX.11.18 fixture로 확인합니다.

## 4. 레이아웃

현재 레이아웃 어휘는 `crates/layout` 엔진 입력에만 있습니다(`input.rs` `LayoutNode`). 영속 record는
IX.11.6이 CSS longhand로 정의합니다. 그래서 이 장의 migration은 대부분 `semantic`(엔진과 corpus
갱신)이고 파일 migration은 없습니다.

### 4.1 container 방식

| Stellar(`LayoutContainer`) | CSS | 상태 | 비고 |
|---|---|---|---|
| `Free` | 부모 `position: relative`, 자식 `position: absolute` + `top`·`right`·`bottom`·`left` | `converted` | containing block을 content box에서 padding box로 바꿉니다(IX.11.5). 4.5절 |
| `Stack` | `display: flex` | `exact`(4.2절 정렬 후) | 4.2절 |
| `Grid` | `display: grid` | `subset` | 4.4절 |
| `Overlay` | `display: grid`, 모든 자식 `grid-area: 1 / 1` | `converted` | 정렬은 `justify-items`·`align-items`. Hug = 자식 최대 크기도 CSS 해석과 같습니다 |
| `Constraint` | CSS anchor positioning 계열 | `stellar-only` | 부모 edge 제약만 inset으로 `converted`. 형제·baseline은 Target capability로 판정합니다 [계획 XXII.2.2] |

`box-sizing`은 `border-box`로 고정하며 편집 속성이 아닙니다. 지금 엔진의 `SizingMode`가 border box를
기준으로 하는 해석과 같습니다. [ADR-0024 결정 7]

### 4.2 Stack ↔ Flexbox

| Stellar(`stack.rs`, `flex.rs`) | CSS longhand | 현재 기본값 | CSS initial value | 차이·비고 | migration |
|---|---|---|---|---|---|
| `axis` + `direction` | `flex-direction: row \| row-reverse \| column \| column-reverse` | Forward | `row` | `Reverse`는 정방향 배치의 거울상이라 `*-reverse` + `flex-start`와 같습니다(`stack.rs` 배치 4단계) | `rename` |
| `main_alignment` | `justify-content` | Start | `normal`(= `flex-start`) | Start/Center/End는 `flex-start`/`center`/`flex-end`입니다. 남는 공간이 음수일 때 Stellar는 SpaceBetween→Start, SpaceAround·SpaceEvenly→Center입니다. CSS Box Alignment는 `space-around`·`space-evenly`의 fallback을 `safe center`로 정해, 넘치면 시작 쪽에 놓일 수 있습니다. 기준 브라우저 결과를 IX.11.8 fixture로 확인한 뒤 같음 여부와 migration 분류를 확정합니다 | `rename`(확인 뒤 필요하면 `semantic`) |
| `cross_alignment` | `align-items` | Start | `normal`(flex에서 `stretch`) | 기본값이 다릅니다. 영속 record 생략값은 CSS initial value를 쓰고, 새 Stack 생성 preset은 명시 값으로 저장합니다 | `semantic`(IX.11.4) |
| (없음) | `align-self` | — | `auto` | 자식별 cross 정렬 override를 추가합니다 | `semantic`(IX.11.4) |
| `wrap` | `flex-wrap: nowrap \| wrap` | NoWrap | `nowrap` | `wrap-reverse`는 받지 않습니다(`subset`) | `rename` |
| `line_alignment` | `align-content` | Start | `normal`(flex에서 `stretch`) | 기본값이 다릅니다 | `semantic`(IX.11.4) |
| `gap`, `cross_gap` | `column-gap`, `row-gap` | 0 | `normal`(flex에서 0) | Stellar는 main·cross 기준, CSS는 축 고정 이름입니다. 영속 record는 CSS 이름으로 저장하고 엔진 입력에서 axis에 따라 바꿉니다. 음수는 CSS처럼 거부합니다 | `semantic`(IX.11.4) |
| `FlexItem.grow` | `flex-grow` | 0 | 0 | 같음 | `rename` |
| `FlexItem.shrink` | `flex-shrink` | 0 | 1 | 기본값이 다릅니다 | `semantic`(IX.11.4) |
| `FlexItem.basis` | `flex-basis: auto \| <length> \| <percentage>` | Auto | `auto` | `content`는 받지 않습니다(`subset`) | `rename` |
| `SizeLimits` min | `min-width`, `min-height` | 0 | `auto` | CSS flex item의 자동 최소 크기(min-content 아래로 줄지 않음)가 없습니다. CSS 의미로 맞춥니다. intrinsic 측정과 성능 확인이 필요해 별도 행입니다 | `semantic`(IX.11.23) |
| `SizeLimits` max | `max-width`, `max-height` | 없음 | `none` | Length만 받습니다. `%`는 영속 record에서 추가합니다 | `rename` |
| `AspectRatio` | `aspect-ratio: <width> / <height>` | 없음 | `auto` | 한 축이 definite이고 다른 축이 indefinite일 때 적용하는 규칙은 CSS와 같습니다. 두 축이 모두 indefinite이면 Stellar는 가로를 intrinsic 크기로 정하고 세로를 비율로 계산하며 콘텐츠 높이를 무시합니다(`sizing.rs`). CSS는 layout 방식에 따라 stretch-fit 너비나 비율 축의 자동 최소 크기(min-content 하한)를 씁니다. 이 경우는 `approximated`이고 14장 E9입니다 [계획 X.6.3] | `rename` |
| `padding` | `padding-top`·`-right`·`-bottom`·`-left` | 0 | 0 | px만 받습니다. `%`는 CSS 정의(containing block 너비 기준)로만 추가합니다 | `rename` |
| baseline 정렬 | `align-items: baseline` | 미구현 | — | X.3.6 이후 추가합니다 | — |
| 숨긴 자식 | `display: none` | flow 제외 | — | 3장 | `none` |
| zero-size 자식 | — | gap 참여 | gap 참여 | 같음 | `none` |

### 4.3 sizing preset ↔ longhand

`SizingMode`의 네 값은 저장 형식이 아니라 인스펙터 preset입니다. preset은 아래 longhand를 한 번에
씁니다. 거꾸로 저장된 longhand가 표의 한 행과 정확히 같으면 인스펙터는 그 preset을 표시하고, 어느
행과도 맞지 않으면 "Custom"을 표시합니다. [계획 IX.11.7]

| preset | 부모 방식·축 | 쓰는 longhand(가로 축 예) |
|---|---|---|
| Fixed | 모든 방식 | `width: <length>`, Stack main 축이면 `flex: 0 0 auto`(`flex-grow: 0`, `flex-shrink: 0`, `flex-basis: auto`) |
| Percentage | 모든 방식 | `width: <percentage>` |
| HugContent | 모든 방식 | `width: fit-content`, Stack main 축이면 `flex-grow: 0`, `flex-shrink: 0`, `flex-basis: auto` |
| Fill | Stack main 축 | `flex-grow: 1`, `flex-shrink: 1`, `flex-basis: 0px`, `min-width: 0px`, `width: auto` |
| Fill | Stack cross 축 | `align-self: stretch`, `width: auto` |
| Fill | Grid·Overlay | `justify-self: stretch`, `width: auto` |
| Fill | Free | `left`·`right`를 함께 지정, `width: auto` |

- Fill이 `min-width: 0px`을 명시하는 것은 CSS flex item 자동 최소 크기(IX.11.23) 때문에 생기는 넘침을
  막기 위해서입니다. 이 차이는 사용자에게 숨기지 않고 Custom 판정에 반영합니다.
- 지금 엔진은 main 축 Fill과 다른 FlexItem 값을 함께 쓰면 오류입니다. 영속 형식에서는 preset이
  longhand로 풀리므로 이 오류 조건은 사라지고 Custom이 됩니다.
- `width: fit-content`는 CSS 의미(`min(max-content, max(min-content, stretch))`)를 따릅니다. 지금
  HugContent 해석과 다른 경우는 IX.11.4·IX.11.5에서 corpus로 확인합니다.

### 4.4 Grid ↔ CSS Grid

| Stellar(`grid.rs`, `grid_tracks.rs`) | CSS longhand·값 | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| `columns`, `rows` | `grid-template-columns`, `grid-template-rows` | `subset` | track은 `<length>`, `<flex>`(fr), `min-content`, `max-content`, `minmax()`만 받습니다. `auto`, `fit-content()`, `%`, `repeat()`, `auto-fill`·`auto-fit`, named line은 거부합니다 | `rename` |
| `TrackSize::Fraction(f)` | `minmax(0, <f>fr)` | `converted` | 엔진은 Fraction을 (최솟값 0, 최댓값 f fr)로 봅니다(`grid_tracks.rs` 규칙 절). CSS bare `<f>fr`은 `minmax(auto, <f>fr)`이라 콘텐츠 최소 크기 아래로 줄지 않으므로 다릅니다. export는 항상 `minmax(0, <f>fr)`로 냅니다 | `rename` |
| bare `<flex>` 입력 | `minmax(auto, <flex>)` | 없음 | parser는 bare `fr`을 CSS 의미(track 최솟값 `auto` = item 최소 기여)로 받습니다. 엔진 `TrackBreadth`에 최솟값 `auto`를 추가합니다(`grid.rs`). 그 전에는 bare `fr`을 `unsupported-value`로 거부합니다 | `semantic`(IX.11.5) |
| `column_gap`, `row_gap` | `column-gap`, `row-gap` | `exact` | | `rename` |
| `implicit_row` | `grid-auto-rows` | `exact` | `grid-auto-columns`는 받지 않습니다 | `rename` |
| 자동 배치 | `grid-auto-flow: row` | `subset` | `column`·`dense`는 받지 않습니다 | `none` |
| `GridPlacement::Auto{span}` | `grid-column: span <n>` 등 | `exact` | | `rename` |
| `GridPlacement::Span{start, span}` | `grid-column-start: <n>`, `grid-column-end: span <n>` | `exact` | 음수 line은 거부합니다 | `rename` |
| `GridPlacement::Lines{start, end}` | `grid-column-start`, `grid-column-end` | `exact` | | `rename` |
| 명시 열을 넘는 배치 | 암시 열 생성 | `subset` | Stellar는 오류로 거부합니다 | `none` |
| cell 안 정렬 | `justify-items`·`align-items`, `justify-self`·`align-self` | 없음 | 지금은 영역 시작에 놓고 Fill일 때만 채웁니다. CSS initial value(`normal` = stretch)로 맞춥니다 | `semantic`(IX.11.5) |
| `grid-template-areas` | — | 없음 | 15장 후보 | — |

### 4.5 Free·Overlay·Constraint ↔ positioning

| Stellar(`free.rs`, `overlay.rs`, `constraint.rs`) | CSS | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| `FreePosition` start anchor | `left`, `top` | `converted` | Length 또는 Percentage | `rename` |
| `FreePosition` end anchor | `right`, `bottom` | `converted` | | `rename` |
| 시작 anchor가 없는 축 | 양쪽 anchor가 모두 없으면 `left: 0px`·`top: 0px`, end anchor만 있는 Fill이면 시작 쪽 `0px` 명시 | `converted` | Stellar는 시작 anchor가 없으면 시작 위치 0을 씁니다(`free.rs` 해석표). CSS는 `left`·`top`이 `auto`면 static position(content box 시작)을 쓰고, end anchor만 있는 `width: auto`는 shrink-to-fit이 되므로 이 두 경우에 `0px`을 명시합니다. end anchor만 있고 크기가 definite이면 시작 쪽을 `auto`로 둡니다. 명시하면 over-constrained가 되어 CSS가 `right`·`bottom`을 무시하기 때문입니다 | `none` |
| anchor 기준 | containing block = padding box | 다름 | 지금은 부모 content box입니다. CSS에 맞춥니다. 백분율 위치·크기의 기준 길이도 content rect(X.2.3)에서 padding box로 바뀝니다 | `semantic`(IX.11.5) |
| start + end + definite 크기 | over-constrained 해석 | `subset` | Stellar는 오류로 거부합니다 [계획 X.2.2] | `none` |
| start + end + 크기 auto | `left`+`right`, `width: auto` | `exact` | 4.3절 Free Fill | `rename` |
| end anchor 없는 자식의 Hug 기여 | absolute 자식은 부모 크기에 기여하지 않음 | 다름 | CSS에 맞춥니다. 내용에 맞춰 커지는 묶음이 필요하면 Stack·Grid·Overlay를 씁니다 | `semantic`(IX.11.5) |
| Overlay `horizontal`·`vertical` | `justify-items`·`align-items: start \| center \| end` | 다름 | 지금 기본값은 Start(`overlay.rs`)이고 CSS initial value는 `normal`(grid item에서 stretch)입니다. Stack cross 정렬과 같게 영속 record 생략값은 CSS initial value, 새 Overlay 생성 preset은 명시 값으로 저장합니다 | `semantic`(IX.11.5) |
| Constraint Parent edge(Left·Right·Top·Bottom) | inset | `converted` | | `none` |
| Constraint CenterX·CenterY | `left: calc(50% + <offset>)` + `translate: -50%` | `converted` | 결과 위치가 같습니다. 붙여넣기 parser는 길이·백분율 혼합 `calc()`를 받지 않으므로(2장) 복사→붙여넣기 왕복에서 건너뜀으로 보고합니다 [계획 IX.11.15] | `none` |
| Constraint Sibling·Baseline | anchor positioning | `stellar-only` | Web profile capability가 없으면 `unsupported` 진단 | `none` |
| `position: sticky`·`fixed` | — | 없음 | 스크롤·viewport 모델이 없어 받지 않습니다 | — |

### 4.6 transform과 레이아웃

- transform은 레이아웃 rect 계산 뒤 적용하고 flow 크기에 영향을 주지 않습니다. CSS transform과
  같습니다. [계획 X.2.4]
- 위치를 transform 이동 성분으로 저장하지 않습니다. 5장 `transform` 행과 IX.11.9를 봅니다.
- scene 입력의 local transform(`NodeInput.transform`, "layout 위치 포함", `crates/scene/src/builder.rs`)은
  layout 결과 위치로 옮긴 뒤 `transform-origin`을 기준으로 `translate` → `rotate` → `scale` →
  `transform` 순서로 합성합니다(CSS Transforms 2). 이 합성 함수는 IX.11.9가 만들고, layout 결과는
  document→layout 연결(X.1.2)에서 받습니다.
- 부모가 flow 방식(Stack·Grid·Overlay)인 자식은 static 요소라 CSS `left`·`top`이 무시됩니다. 그런
  자식의 시각 이동은 `translate`로 표현합니다.

## 5. Frame(`frame.rs`, `frame_metadata.rs`)

| Stellar 속성(key) | CSS | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| `size`(0x0101) | `width`, `height` | `converted` | Fixed 크기만 표현합니다. 합성 key라 `width`·`height` key로 나누고 0x0101은 묶음 key로 남깁니다(ADR-0024 결정 14). 영속 layout record가 sizing longhand를 가지면 Frame size를 그 Fixed 경우로 흡수합니다. 방식은 IX.11.6 ADR이 정합니다 | `key`(IX.11.22), `schema`(IX.11.6) |
| `transform`(0x0102) 이동 성분 | 위치(Free 자식 `left`·`top`, flow 자식 `translate`) | 다름 | 지금 인스펙터는 위치 X·Y를 transform 이동 성분으로 편집합니다(인스펙터 명세 5.2). CSS에서 위치는 레이아웃 속성입니다. migration은 이동 성분만 옮깁니다. 지금 저장 형식에는 layout 정의 record가 없어(`frame.rs` `layout_mode` 주석) 모든 자식이 Free 자식입니다. 화면 위치는 layout 반올림 때문에 1 LayoutUnit 안에서 보존됩니다 | `schema`(IX.11.9), `key` |
| `transform`(0x0102) 선형 부분 | `transform: matrix(a, b, c, d, 0, 0)`, `transform-origin` | `converted` | migration은 선형 부분을 bit 그대로 행렬로 보존하고 `rotate`·`scale`로 분해하지 않습니다. 분해에 쓰는 atan2·sin·cos는 플랫폼 의존 결과입니다(ADR-0006). 인스펙터의 회전·배율 표시는 행렬에서 계산한 표시 전용 파생값이며 저장하지 않습니다. 새 편집은 `rotate`·`scale`·`translate` longhand를 씁니다. 지금 원점은 border box 좌상단(`0 0`)이고 CSS initial value는 `50% 50%`입니다. migration된 노드는 `0 0`을 명시 저장합니다. 0x0102는 묶음 key로 남고 migration 이후 새 command로 받지 않습니다 | `schema`(IX.11.9), `key` |
| `clipping`(0x0103) Disabled | `overflow: visible` | `exact` | | `rename` |
| `clipping` FrameBounds | `overflow: clip` + `overflow-clip-margin: border-box` | `converted`(capability), `initial-differs` | Stellar는 자식을 border box(0..size)와 바깥 corner radius로 자릅니다(`crates/scene/src/builder.rs` `FrameClipInput::Bounds`). CSS `overflow: clip`의 기본 clip 경계는 padding box와 안쪽 radius라 border가 0보다 크면 다릅니다. `overflow-clip-margin: border-box`로 경계를 border box로 맞춥니다. 이 값을 지원하지 않는 Web profile에서는 `approximated`입니다 [계획 IX.11.19] | `rename` |
| `clipping` Path(NodeRef) | `clip-path` | `converted` | Web은 inline SVG `clipPath`를 `url(#id)`로 참조합니다 | `none` |
| `background`(0x0104) | `background-color`(없음 = `transparent`) | `exact` | 한 겹 단색만 있습니다. gradient·image layer는 15장 O2 | `rename` |
| `border`(0x0105) | `border-{top,right,bottom,left}-{width,style,color}` | `converted` | 4변 같은 값, style `solid` 하나, 없음 = `none`입니다. 지금은 레이아웃 공간을 차지하지 않습니다(scene에서 Inside로 그림). CSS처럼 border box 안쪽을 padding box로 만듭니다. 레이아웃이 아직 문서와 연결되지 않아 flow 결과에는 영향이 없고, Free 자식의 화면 위치는 migration이 부모 border 두께만큼 보정해 보존합니다. 12개 longhand에 key를 나누고 0x0105는 묶음 key로 남깁니다 | `schema`(IX.11.10), `key` |
| `corner_radius`(0x0106) | `border-{top-left,top-right,bottom-right,bottom-left}-radius` | `exact` | px 원형 반경만 받습니다(타원 x/y·`%`는 `subset`). bounds를 넘으면 모든 반경을 같은 비율로 줄이는 규칙은 CSS와 같습니다. 지금은 4 모서리를 key 하나와 semantic snapshot 경로 하나에 둡니다(`crates/semantic-diff/src/build.rs`). 모서리마다 key를 나누고 0x0106은 묶음 key로 남깁니다 | `key`(IX.11.22) |
| `layout_mode`(0x0107) | `display` + container longhand 묶음 | `stellar-only`(참조 구조) | 참조하는 layout 정의 resource는 CSS class와 비슷합니다. 저장 위치는 IX.11.6 ADR이 정합니다 | `schema`(IX.11.6) |

## 6. Shape·Path·Image

Shape·Path에는 fill·stroke 필드가 없고 칠하기는 modifier가 맡습니다(9장). Shape·Path·Text 속성에는
아직 `PropertyKey`가 없습니다. IX.11.3이 key와 CSS 매핑을 함께 정의합니다. key는 longhand마다 하나이고
(ADR-0024 결정 14, 예: Rectangle `width`·`height`), semantic diff가 임시로 쓰는 snapshot 전용 key
(`0xE3xx`·`0xE4xx`·`0xE6xx`, `crates/semantic-diff/src/keys.rs`)를 대체합니다.

| Stellar 속성 | CSS·SVG 표현 | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| Rectangle `size` | `width`, `height` | `converted` | | `rename` |
| Rectangle `corner_radius` | `border-*-radius` | `exact` | SVG `rect`는 반경 하나만 받으므로 SVG에서는 path로 냅니다 | `rename` |
| Ellipse `bounds` | 요소 크기 + `border-radius: 50%` 또는 SVG `ellipse` | `converted` | | `rename` |
| Line `start`·`end` | SVG `x1`·`y1`·`x2`·`y2` | `stellar-only` | CSS 상자 속성이 아닙니다 | `none` |
| Line `cap` | `stroke-linecap: butt \| round \| square` | `exact` | | `rename` |
| Polygon `sides`·`rotation_degrees`·`corner_radius` | — | `stellar-only` | export에서 path로 굽습니다 | `none` |
| Path command | SVG `d`, CSS `path()` | `converted` | MoveTo·LineTo·CubicTo·Close | `none` |
| Path `winding` | `fill-rule: nonzero \| evenodd` | `exact` | | `rename` |
| Image `asset` | export에서 `<img src>` | `stellar-only` | AssetRef 참조 | `none` |
| Image `size` | `width`, `height` | `converted` | | `rename` |
| Image `fit` | `object-fit: fill \| contain \| cover \| none` | `exact`, `initial-differs` | 기본값 Contain, CSS initial value `fill`. `scale-down`·`object-position`은 없습니다 | `rename` |
| Image `crop` | — | `stellar-only` | export는 wrapper + `object-position`·크기로 근사합니다(`approximated`) | `none` |
| Image `sampling` | `image-rendering: auto \| pixelated` | `approximated` | `auto`가 선형 보간을 보장하지 않습니다 | `rename` |
| Image `color_space_override` | — | `stellar-only` | export에서 래스터화합니다 | `none` |

## 7. Text와 Typography

지금 Text payload는 내용, paragraph style index, run 범위와 style index만 저장합니다. `TextStyle`·
`ParagraphStyle` 표에는 저장 형식이 없습니다(`crates/document/src/text.rs` `TextPayload::encode`). 그래서
아래 style 속성의 위치 변경은 파일 migration이 아니라 저장 형식을 정하기 전의 타입 변경이고, style 표
저장 형식 추가가 Text payload의 schema 변경입니다(IX.11.11).

| Stellar 속성(`text.rs`) | CSS | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| `content` | 텍스트 내용 | `stellar-only` | UserText. SVG export는 outline path입니다 [계획 XX.3.2] | `none` |
| `TextStyle.fill` | `color` | `exact` | 없으면 상속하는 규칙이 CSS `color` 상속과 같습니다 | `rename` |
| `TextStyle.features` | `font-feature-settings` | `exact` | | `rename` |
| `TextStyle.variations` | `font-variation-settings` | `exact` | | `rename` |
| `TextStyle.typography` | Typography record | 미정 | 지금은 불투명 placeholder입니다. IX.11.11이 아래 longhand로 정의합니다. 없으면 문단 typography, 그다음 문서 기본 typography를 상속합니다(`TextStyle.typography` 주석의 "없으면 paragraph/문서 기본") | `schema`(IX.11.11) |
| (없음) 문단 typography | 문단 요소 자신의 `font-*` | 없음 | `ParagraphStyle`에 Typography 참조를 추가합니다. CSS에서 root inline box strut의 글꼴은 block container 자신의 글꼴이므로, 줄 높이 계산의 strut 글꼴 출처가 됩니다 | `schema`(IX.11.11) |
| `ParagraphStyle.alignment` | `text-align: start \| center \| end \| justify` | `exact` | 마지막 줄 처리도 CSS 기본과 같습니다 | `rename` |
| `line_height` Auto | `line-height: normal` | `approximated` | CSS `normal`은 UA가 정합니다. Stellar는 font metric + half-leading입니다 | `rename` |
| `line_height` Multiplier | `line-height: <number>` | 다름 | Stellar는 줄 안 최대 글꼴 크기 × 배율, CSS는 inline box마다 자기 font-size × 배율입니다. 혼합 크기 run에서 결과가 다릅니다. CSS 의미에는 root inline box strut가 필요하고, strut는 문단 typography의 첫 사용 가능 글꼴과 font-size로 만듭니다 | `semantic`(IX.11.12) |
| `line_height` Fixed | `line-height: <length>` | `exact` | | `rename` |
| `letter_spacing_em` | `letter-spacing: <number>em` | 다름 | Stellar는 paragraph 단위, CSS는 inline(run) 단위입니다. text 엔진이 아직 이 값을 쓰지 않습니다. run `TextStyle`의 선택 필드(없으면 Typography 값)로 옮기고 Typography resource를 복제하지 않습니다. 저장된 값이 없어 파일 migration은 없습니다 | `none`(IX.11.11 타입 변경) |
| `wrapping` Wrap | `white-space: pre-wrap` + `overflow-wrap: break-word` | `converted` | 문서 모델이 공백을 보존하므로 CSS 기본(`normal`, 공백 접기)과 다릅니다. export는 항상 명시합니다 | `rename` |
| `wrapping` NoWrap | `white-space: pre` | `converted` | 강제 줄바꿈은 유지합니다 | `rename` |
| text 엔진 `paragraph_spacing` | — | `stellar-only` | 문서 필드가 없습니다. 필요하면 문단 사이 `margin-block`으로 근사합니다 | — |
| text 엔진 세로 정렬 | `align-content`(block container) | `approximated` | 문서 필드가 없습니다. Web profile capability로 판정합니다 | — |

Typography record(IX.11.11)는 다음 CSS longhand로 정의합니다. 생략값은 CSS initial value입니다.

| longhand | 값 | 비고 |
|---|---|---|
| `font-family` | family 이름 목록 | 결정론을 위해 해석 결과는 `FontFaceRef`(file hash + face index)로 고정합니다 [계획 X.8.1]. 누락 글꼴은 임의 대체하지 않습니다 [계획 X.8.3] |
| `font-size` | `<length>` px | text 엔진 `FontSize`(1/64 subpixel)와 같은 해상도로 저장합니다 |
| `font-weight` | 1\~1000 수 | keyword(`normal`, `bold`)는 입력에서만 받습니다 |
| `font-style` | `normal \| italic \| oblique <angle>` | |
| `font-stretch` | `<percentage>` | variable font `wdth` 축과 연결합니다 |
| `line-height` | 위 표 | Typography가 기본값을 주고 paragraph가 override합니다 |
| `letter-spacing` | `<length>` 또는 `em` | Typography가 기본값을 주고 run `TextStyle`이 override합니다 |
| `text-decoration-line`·`-style`·`-color`·`-thickness` | 15장 O8 후보 | 첫 정의에는 넣지 않습니다 |

기존 `TypedValue::Typography` 값은 구조가 정해지지 않은 불투명 blob입니다(`crates/core-types/src/typed_value.rs`).
type tag는 유지하고 payload를 Typography canonical encoding으로 정의합니다. 새 encoding으로 decode되지
않는 기존 blob은 해석할 근거가 없으므로 migration하지 않습니다. bytes를 보존하고
`typography-unrecognized` 진단을 내며 조용히 바꾸지 않습니다.

## 8. 값 타입·색

### 8.1 `TypedValue` ↔ CSS 값 타입

| TypedValue(`typed_value.rs`) | CSS 값 타입 | 상태 | 비고 |
|---|---|---|---|
| Boolean | — | `stellar-only` | CSS 속성 값이 아닙니다. 토큰 export는 10장 |
| Number | `<number>` | `exact` | |
| String | `<string>` | `exact` | |
| Enum | keyword | `exact` | |
| Length | `<length>` | `exact` | px |
| Percentage | `<percentage>` | `exact` | |
| Angle | `<angle>` | `exact` | deg |
| Duration | `<time>` | `exact` | ms. 애니메이션·전환 모델은 아직 없습니다 |
| Color | `<color>` | `exact` | 8.2절 |
| Gradient | `<gradient>` | 불가 | stop 목록만 있고 종류·각도·기하가 없어 CSS 텍스트를 만들 수 없습니다. 15장 O2 |
| Shadow | `<shadow>` 한 겹 | `converted` | offset x·y, blur, color. spread와 `inset`이 없습니다 |
| Typography | font longhand 묶음 | 미정 | 7장 |
| ResourceRef | — | `stellar-only` | 텍스트 표현에서 `url()`을 만들지 않습니다 |

### 8.2 색과 정규 텍스트 형식

- 의미: `ColorSpace`(sRGB·Display P3) 태그 + straight alpha RGBA입니다. CSS Color 4 `color()`와 같고,
  [0,1] 범위 밖 채널도 허용하는 점도 같습니다. [CSS Color 4]
- 정규 출력(결정론, IX.11.13):
  1. sRGB이고 네 채널이 모두 [0,1] 안이며, 각 채널을 `round(v × 255) / 255`로 되돌린 f32가 원래 값과
     같으면 소문자 hex `#rrggbb`(alpha 1) 또는 `#rrggbbaa`로 씁니다.
  2. 그 밖의 sRGB는 `color(srgb r g b)` 또는 `color(srgb r g b / a)`로 씁니다.
  3. Display P3는 `color(display-p3 r g b)` 또는 `color(display-p3 r g b / a)`로 씁니다. export는
     이미 이 표기를 씁니다.
  4. 수는 f32를 왕복하는 가장 짧은 10진 표기로 씁니다. 지수 표기를 쓰지 않고, 끝의 0을 빼고, `-0`은
     `0`으로 씁니다.
- 합성: Stellar는 선형 공간에서 premultiplied 합성을 합니다(아키텍처 §10.3). 웹은 sRGB 합성이므로
  Blend·겹친 Opacity·Blur 결과는 Web profile에서 `approximated`입니다.

### 8.3 그 밖의 정규 텍스트 형식

- 길이 `<n>px`, 백분율 `<n>%`, 각도 `<n>deg`, 시간 `<n>ms`입니다. 0도 단위를 붙입니다(`0px`).
- keyword는 소문자입니다. 여러 값은 공백 하나, 목록은 `, `로 구분합니다.
- 선언 블록은 `property: value;`를 한 줄에 하나씩, registry 순서로 씁니다.
- shorthand로 접은 표현은 표시·복사 옵션일 뿐이고 정규 형식은 longhand입니다(11장).

## 9. Modifier와 효과(`modifier.rs`, `modifier_metadata.rs`)

modifier stack은 Stellar 고유 구조로 유지합니다. 파라미터 이름과 값 문법은 CSS를 따릅니다.
[ADR-0024 결정 9]

| Modifier(key) | CSS | 상태 | 차이·비고 | migration |
|---|---|---|---|---|
| 공통 enabled(0x1001) | — | `stellar-only` | 비활성 항목은 출력하지 않습니다 | `none` |
| 공통 order(0x1002) | 9.1절 | `stellar-only` | | `none` |
| Transform(0x1011) | 추가 `transform: matrix(a, b, c, d, tx, ty)` | `converted` | 원점 문제는 Frame `transform`과 같습니다 | `rename` |
| Boolean(0x1021, 0x1022) | — | `stellar-only` | Union·Subtract·Intersect·Exclude. export에서 path로 굽습니다 | `none` |
| StrokeExpand(0x1031) | — | `stellar-only` | path로 굽습니다 | `none` |
| Corner(0x1041) | — | `stellar-only` | Frame·Rectangle 반경은 `border-*-radius`가 맡습니다. path 모서리 처리는 굽습니다 | `none` |
| Mask(0x1051) | `mask-image: url(#id)` | `converted` | Web은 inline SVG `mask`를 참조합니다 | `none` |
| Clip(0x1061) | `clip-path: url(#id)` | `converted` | | `none` |
| Fill(0x1071) | Frame: 배경 layer(`background-image: linear-gradient(c, c)`), Shape·Path: `fill` | `converted` | 단색만 있습니다 | `rename` |
| Stroke(0x1081, 0x1082) | Shape·Path: `stroke`, `stroke-width`, Frame: `box-shadow: inset 0 0 0 <w> <c>` 또는 `outline` + 음수 `outline-offset` | `converted` | 정렬은 node 종류로 정해집니다(Frame=Inside, Shape·Path=Center). `stroke-linejoin`·`stroke-dasharray`·`stroke-miterlimit`는 이름만 예약합니다 [계획 XI.6.2] | `rename` |
| Shadow(0x1091\~0x1093) | Frame: `box-shadow`, 그 밖: `filter: drop-shadow()` | `approximated` | spread·inset이 없습니다. Stellar blur 반경과 CSS 표준편차의 관계는 렌더러 효과 구현(XI.10.4)에서 고정합니다. CSS는 `box-shadow` blur를 반경(표준편차의 2배)으로, `blur()`를 표준편차로 정의합니다. offset(0x1092)은 x·y를 담은 합성 key라 offset x·y key로 나누고 0x1092는 묶음 key로 남깁니다 | `rename`, `key`(IX.11.22) |
| Blur(0x10A1) | `filter: blur()` | `approximated` | 같음 | `rename` |
| Blend(0x10B1) | `mix-blend-mode: normal \| multiply \| screen \| overlay \| darken \| lighten` | `approximated` | keyword는 같고 합성 색공간이 다릅니다. CSS 16종 중 나머지는 추가 후보입니다. 그룹 경계는 `isolation: isolate`로 냅니다 | `rename` |
| Opacity(0x10C1) | `opacity`(0\~1) | `exact` | 그룹 opacity입니다. 겹침 합성 색공간은 8.2절 | `rename` |
| (없음) | `backdrop-filter` | 없음 | 새 modifier 후보. CSS 이름으로 추가하되 별도 WBS가 필요합니다 | — |

### 9.1 stack 순서와 CSS 고정 순서

CSS는 한 요소에 효과를 고정 순서로 적용합니다. `transform` → paint(`box-shadow`, background, border,
내용) → `filter` 목록(순서 있음) → `clip-path` → `mask` → `opacity` → `mix-blend-mode`(뒤 콘텐츠와
합성) 순서입니다. Web export의 선형화 규칙(IX.11.18)은 다음과 같습니다.

1. 활성 modifier를 stack 순서로 읽고, 위 CSS 순서를 거스르지 않는 가장 긴 앞 구간을 요소 하나에
   담습니다.
2. 이어지는 Shadow·Blur는 하나의 `filter` 목록으로 합칩니다. 목록은 순서가 있으므로 Blur 뒤의
   Shadow는 `drop-shadow()`로 냅니다. Frame의 `box-shadow`는 앞에 filter 계열이 없을 때만 씁니다.
3. 순서를 거스르는 지점마다 바깥 wrapper 요소를 하나 만들고, 남은 구간을 wrapper에 담습니다.
   wrapper를 만들면 그 노드는 `approximated`(구조 변경)로 진단합니다.
4. `stellar-only` modifier(Boolean·StrokeExpand·Corner)가 있으면 그 지점까지를 path로 굽습니다. 굽기가
   불가능하면 최소 하위 트리만 래스터화합니다. [계획 XX.4.1]
5. 같은 stack은 항상 같은 요소 구조를 만듭니다.

## 10. 토큰과 환경

| Stellar | CSS | 비고 |
|---|---|---|
| `TokenDefinition`(StableId, TokenName, TokenType, 기본 expression) | `@property --<name> { syntax; inherits: true; initial-value }` + `:root { --<name>: <value>; }` | 문서 안 참조는 StableId입니다. 텍스트 표현과 export만 `var(--<name>)`을 씁니다. `inherits`는 아래 등록 속성 정책을 따릅니다 [ADR-0024 결정 11] |
| TokenType Length·Percentage·Angle·Duration·Number·Color | `<length>`·`<percentage>`·`<angle>`·`<time>`·`<number>`·`<color>` | |
| TokenType String·Gradient·Shadow·Typography | `*`(등록하지 않은 custom property) 또는 생성 상수 | XXII.2.3이 정합니다 |
| TokenType Boolean·Enum | 생성 상수 | CSS 속성 값으로 쓰지 않습니다 |
| project·library·document scope | 출력 파일·layer 단위 | scope 우선순위는 ID 해석이 정합니다 [계획 VIII.5.2] |
| cycle | 저장 전 거부 | CSS는 cycle을 계산 시 무효로 만듭니다. Stellar가 더 엄격합니다 [계획 VIII.5.3] |
| 환경 `color_scheme`·`contrast`·`reduced_motion` | `prefers-color-scheme`·`prefers-contrast`·`prefers-reduced-motion` | 조건부 토큰 값은 `:root` 아래 `@media` 블록으로 냅니다 |
| 환경 `viewport_width`·`viewport_height`·`density` | media feature `width`·`height`·`resolution` | |
| 환경 `interaction_state` | `:hover`·`:focus-visible`·`:active`·`:disabled` | Control contract 상태와 함께 판정합니다 |
| 환경 `locale` | `:lang()` | |
| 환경 `platform`·`os_version`·`api_level` | — | Target Profile 선택이 맡습니다 |

등록 custom property 정책(토큰과 Context 공통, IX.11.16):

1. Stellar가 내보내는 등록 custom property(`@property`)는 모두 `inherits: true`입니다. `inherits: false`인
   등록 속성은 자손에 상속되지 않으므로, `:root`에 값을 두면 자손의 `var()`는 그 값 대신
   `initial-value`를 받습니다. `:root`를 감싼 `@media` 조건부 값도 같은 이유로 자손에 닿지 않습니다.
2. 토큰 값은 `:root`와 `:root`를 감싼 `@media` 블록에서만 선언합니다. 다른 요소가 토큰 이름을 다시
   선언하지 않으므로 상속은 `:root` 값을 그대로 전달하기만 합니다.
3. Context(ADR-0025 결정 3)를 Web에서 등록 custom property 상속으로 낮출 때(ADR-0025 결정 4.3, 계획
   XXII.2.5)도 같은 `inherits: true` 정책과 아래 이름 규칙을 씁니다. provision이 있는 요소에서 값을
   선언하고 상속이 하위 트리에 전달합니다. 같은 key를 다시 읽는 provision을 낮추는 방법 등 세부는
   XXII.2.5가 정합니다.
4. 등록하지 않은 custom property(`syntax`가 `*`인 경우)는 CSS에서 원래 상속되므로 따로 선언하지
   않습니다.

CSS 이름 파생 규칙(IX.11.16):

1. TokenName과 ContextKey 이름의 공백·`/`·`.`을 `-`로 바꿉니다.
2. 결과를 CSS identifier 직렬화 규칙으로 escape하고 앞에 접두사를 붙입니다. 토큰은 `--`, ContextKey는
   `--context-`입니다. 대소문자는 보존합니다(custom property 이름은 대소문자를 구분합니다).
3. 충돌 판정은 같은 export scope의 토큰 이름과 ContextKey 이름을 한 집합으로 봅니다. 접두사가 달라도
   토큰 `context/accent`는 `--context-accent`가 되어 ContextKey `accent`와 겹칠 수 있기 때문입니다.
4. 이름이 겹치면 `name-collision` 진단을 내고, StableId 오름차순으로 첫 항목을 뺀 나머지에 `-` +
   StableId 32자리 소문자 hex를 붙입니다. StableId 앞 8자리는 UUIDv7의 시각 ms 48비트 중 상위
   32비트라(`crates/core-types/src/ids.rs` `new_v7`) 약 65초 안에 만든 항목끼리 같을 수 있어 쓰지
   않습니다.
5. 파생은 이름, 종류(토큰·ContextKey), StableId만 입력으로 받는 순수 함수입니다. rename하면 CSS 이름만
   바뀌고 문서 안 참조(StableId)는 그대로입니다.

CSS custom property 상속은 트리 아래로 값을 공급하는 메커니즘입니다. 하지만 아키텍처 §14.3·§15.1은
컴포넌트 경계를 넘는 암묵적 참조를 금지합니다. 그래서 상속형 공급(context)의 문서 모델은 이 문서
범위 밖이며, ADR-0025 결정 3(Component Context)이 선언형 공급·소비 인터페이스로 정합니다. 이 문서는
그 Web 출력이 쓰는 등록 속성 정책과 이름 규칙만 정합니다.

## 11. shorthand ↔ longhand

인스펙터는 아래 shorthand로 편집할 수 있습니다(예: 4변을 묶은 padding 필드, 묶음 풀기 버튼).
입력은 항상 CSS 확장 규칙으로 longhand로 펼쳐 하나의 CommandTransaction으로 적용합니다. [ADR-0024 결정 4]

| shorthand | longhand | 비고 |
|---|---|---|
| `padding` | `padding-top`·`-right`·`-bottom`·`-left` | 1\~4값 규칙 |
| `inset` | `top`·`right`·`bottom`·`left` | |
| `gap` | `row-gap`, `column-gap` | |
| `flex` | `flex-grow`, `flex-shrink`, `flex-basis` | `flex: 1`은 `1 1 0%`, `auto`는 `1 1 auto`, `none`은 `0 0 auto` |
| `flex-flow` | `flex-direction`, `flex-wrap` | |
| `place-items`·`place-content`·`place-self` | `align-*`, `justify-*` | |
| `border` | 12개(`border-*-width`·`-style`·`-color`) | |
| `border-width`·`border-style`·`border-color` | 4변 | |
| `border-radius` | 4 모서리 | `/` 뒤 타원 반경은 거부합니다 |
| `grid-row`·`grid-column`·`grid-area` | `grid-*-start`·`grid-*-end` | |
| `grid-template` | `grid-template-rows`·`-columns` | areas가 있으면 거부합니다 |
| `overflow` | `overflow-x`, `overflow-y` | Stellar clip은 하나이므로 두 값이 같을 때만 받습니다 |
| `background` | `background-color` | 색 외 성분이 있으면 거부합니다 |
| `font` | font longhand + `line-height` | CSS처럼 생략한 longhand를 initial value로 되돌립니다 |
| `white-space`, `text-wrap` | `white-space-collapse`, `text-wrap-mode`, `text-wrap-style` | Stellar는 `pre-wrap`·`pre` 두 조합만 받습니다 |

## 12. 인스펙터 범주

인스펙터 섹션을 CSS 범주로 다시 묶습니다(IX.11.17). 외형과 컨트롤은 macOS 네이티브 바탕의 Stellar
고유 형태이며 인스펙터 명세와 가이드를 따릅니다. 섹션 순서는 다음과 같습니다.

| 섹션 | 담는 longhand | 대상 |
|---|---|---|
| Layer | 이름, 표시, 잠금 | 모든 노드 |
| Layout | `display`, `flex-*`, `justify-*`, `align-*`, `grid-template-*`, `grid-auto-*` | container Frame |
| Position | `position`, `top`·`right`·`bottom`·`left`, 자식 배치(`align-self`, `grid-*-start`·`-end`) | 부모가 있는 노드 |
| Size | `width`, `height`, `min-*`, `max-*`, `aspect-ratio`, sizing preset | Frame, Shape, Image, Text |
| Spacing | `padding-*`, `row-gap`, `column-gap` | container Frame |
| Overflow | `overflow`, `clip-path` | Frame |
| Typography | font longhand, `line-height`, `letter-spacing`, `text-align`, `white-space`, `color` | Text |
| Background | `background-color` | Frame |
| Border | `border-*`, `border-*-radius` | Frame, Rectangle |
| Transform | `translate`, `rotate`, `scale`, `transform`, `transform-origin`(migration된 노드의 회전·배율은 `transform` 행렬에서 계산한 표시 전용 값, 5장) | Frame |
| Effects | modifier stack(9장) | 그래픽 노드 |
| 종류별 | Shape geometry, Path, Image, Component Property·Slot | 해당 종류 |

편집 관례는 전문 UI 디자인 도구 사용자가 기대하는 동작을 참고합니다. 화면과 문구는 Stellar 고유로
만듭니다.

- sizing은 preset(4.3절)으로 바꾸고, preset으로 표현되지 않으면 Custom으로 보여 줍니다.
- Flex 정렬은 `justify-content`와 `align-items`를 함께 바꾸는 3×3 위치 컨트롤로 편집할 수 있습니다.
  분배(`space-*`)를 고르면 main 축 위치 선택은 비활성이 됩니다.
- 4변·4모서리 값은 묶어서(shorthand) 또는 풀어서(longhand) 편집합니다.
- 수치 필드는 CSS 값 문법을 받습니다. 단위를 붙여 입력할 수 있고(`1.5turn` → `540deg`), 오류는 행
  진단으로 보여 줍니다(IX.11.14).
- 행 레이블은 사람이 읽는 이름을 쓰고, CSS longhand 이름은 툴팁·접근성 도움말·복사 결과로 보여
  줍니다. 레이블에 CSS 이름을 직접 쓸지는 15장 O9입니다.
- 캔버스의 padding·gap 직접 조작 핸들은 같은 longhand를 편집합니다. 캔버스 상호작용 세부는 캔버스
  편집 UX 문서가 정합니다.

## 13. Stellar 고유로 남는 것

- 식별자: StableId, 숫자 `PropertyKey`, `OrderKey`. CSS 이름은 식별자가 아닙니다.
- 저장: 바이너리 chunk·generation·migration(ADR-0010, ADR-0011, ADR-0012). CSS 텍스트를 저장하지
  않습니다.
- 결정론: 1/64 px `LayoutUnit`, ties away from zero, 유한 f32, 자식 visual order 기준 잔여 분배.
- 그래픽 구조: Fragment(상자 없는 구조 노드, 아키텍처 §12.4), modifier stack, Boolean·StrokeExpand·
  Corner, Polygon, Path StableId point.
- 컴포넌트: Component Property·Slot·instance override, View Variant, 상태 머신, Control contract.
- Constraint 레이아웃과 형제·baseline anchor.
- 선형 공간 합성과 Display P3 기본 처리.
- 비웹 타깃: Apple·Android는 CSS 의미에서 낮추고 Target Profile capability로 판정합니다(아키텍처 §17).

## 14. 허용하는 Stellar 확장(부분집합 규칙의 예외)

아래 목록만 CSS와 다른 결과를 허용합니다. 목록에 없는 차이는 결함이며 IX.11 migration 대상입니다.
[ADR-0024 결정 3]

| ID | 확장 | Web export |
|---|---|---|
| E1 | 선형 공간 합성(Blend, 겹친 Opacity, Blur, Shadow) | `approximated` |
| E2 | modifier stack의 자유 순서 | wrapper 분할(9.1절), `approximated` |
| E3 | 자식 visual order = paint 순서(`order`·`z-index` 분리 없음) | DOM 순서로 출력하고 모든 요소가 같은 층(`z-index: 0`)의 stacking context를 만들게 함(3장 stacking 규칙) |
| E4 | Constraint 레이아웃(형제·baseline anchor) | capability에 따라 `converted` 또는 `unsupported` |
| E5 | `initial-differs` 기본값(Frame clip, Image fit) | 명시 출력 |
| E6 | 공백 보존 텍스트 모델 | `white-space` 명시 출력 |
| E7 | Line·Polygon·Path·Boolean 등 SVG 기하 | SVG 또는 path 굽기 |
| E8 | `line-height: normal`의 metric 해석 | `approximated` |
| E9 | 두 축이 모두 indefinite인 `aspect-ratio`의 해석(가로 intrinsic 크기, 콘텐츠 높이 무시) | `approximated` |

## 15. 열린 결정

| ID | 내용 | 정할 곳 |
|---|---|---|
| O1 | 영속 layout record 저장 위치(Frame payload 확장 또는 `layout_mode`가 참조하는 resource)와 Frame `size` 흡수 방식 | IX.11.6 ADR |
| O2 | Gradient 종류·기하(linear·radial·conic)와 다중 background layer | 별도 ADR, scene gradient Paint와 함께 |
| O3 | `<length-percentage>`(길이·백분율 혼합) 값 타입 | ADR-0008 개정 |
| O4 | `margin` 어휘와 음수 간격(겹치는 배치) | 별도 ADR |
| O5 | `em`·`rem`·viewport·container 단위의 기준값 | Target Profile·환경 key 결정과 함께 |
| O6 | FrameBounds clip의 기준 box와 radius 반영 | 결정됨: border box와 바깥 radius(scene 코드 확인, 5장 `clipping` 행). IX.11.2가 registry에 기록 |
| O7 | Shadow·Blur 반경과 CSS 표준편차 변환식 | XI.10.4 |
| O8 | `text-decoration`·`text-transform`·`text-indent`·`text-overflow`·줄 수 제한 | Typography 확장 WBS |
| O9 | 인스펙터 행 레이블에 CSS 이름을 직접 쓸지 | IX.11.17 |
| O10 | Web Target Profile의 CSS 기준선(어떤 CSS 기능을 `native`로 볼지) | IX.10.2, IX.11.19 |
| O11 | `grid-template-areas`, `repeat()`, `auto`·`fit-content()` track. `minmax()` 최솟값 `auto`는 IX.11.5가 다룹니다 | Grid 확장 WBS |
| O12 | Text·Path의 merge 단위. Path command·point 목록은 CSS longhand가 아니므로(6장 `stellar-only`) ADR-0024 결정 4의 대상이 아니고 payload 전체(`PATH_DATA`)를 유지합니다. Text의 style 속성은 IX.11.11이 style 표 저장 형식과 longhand key를 정한 때부터 longhand 단위로 merge하고, 그 전까지는 `TEXT_PAYLOAD` 전체가 단위입니다. content의 텍스트 단위 merge는 XIV 규칙을 따릅니다 | IX.11.3, IX.11.11 |
| O13 | `rotate` 각도를 행렬로 바꿀 때의 sin·cos 구현. libm 초월함수는 플랫폼 의존 결과입니다(ADR-0006). 직각 배수 특수 처리, 저장소에 고정한 근사식 등 결정론 기준을 맞추는 방법 | IX.11.9 |

## 16. 확인하지 못한 것

- Shadow modifier가 Frame에서 border box 형상 그림자인지 alpha 기반 그림자인지, 렌더러 blur 반경
  정의는 렌더러 효과가 미구현(XI.10.4)이라 확인하지 못했습니다.
- 기준 Web profile 브라우저가 `overflow-clip-margin`의 `<visual-box>` 값(`border-box`)을 지원하는지는
  확인하지 않았습니다. 지원 여부는 IX.11.19 capability로 판정합니다.
- 기준 브라우저가 음수 남는 공간에서 `space-around`·`space-evenly`를 `safe center`로 처리하는지는
  확인하지 않았습니다(IX.11.8).
- 지금 HugContent 해석이 CSS `fit-content`와 모든 경우에 같은지는 corpus로 비교하지 않았습니다.
- 레이아웃 기본값을 CSS에 맞출 때 바뀌는 X.11.1 corpus fixture 수는 세지 않았습니다.
- V.8 runner의 step은 chunk payload bytes를 받는 함수라(`crates/storage/src/migration/registry.rs`
  `MigrateFn`) payload 내부 필드 의미 변경을 다룰 수 있습니다. 다만 storage crate는 document에 의존하지
  않고 제품 open 경로에 registry가 연결되어 있지 않아 IX.11.21이 필요합니다(확인함).
- 색 함수 `mix`의 보간 색공간과 CSS `color-mix()`의 대응은 확인하지 않았습니다.
- CSS 쪽 서술은 W3C 명세와 공개 참고 문서를 근거로 했습니다. `filter`, `box-shadow`, `border-radius`,
  gradient의 세부 수식은 해당 구현 행에서 명세로 다시 확인합니다.

## 17. WBS 대응

| 항목 | 행 |
|---|---|
| 합성 key의 longhand key 분할 | IX.11.22 |
| registry CSS 매핑 metadata와 단위 계약 | IX.11.1, IX.11.2, IX.11.3 |
| 레이아웃 의미 정렬(Flex, 자동 최소 크기, Grid·Free·Overlay) | IX.11.4, IX.11.23, IX.11.5 |
| 영속 layout record와 sizing preset | IX.11.6, IX.11.7 |
| CSS 레이아웃 적합성 fixture | IX.11.8 |
| 제품 문서 migration registry와 project 열기 연결 | IX.11.21 |
| 의미가 다른 구현 속성 migration(Frame 위치·transform, border, Text) | IX.11.9, IX.11.10, IX.11.11, IX.11.12 |
| CSS 값 형식화·파싱, 복사·붙여넣기, 토큰 이름 | IX.11.13, IX.11.14, IX.11.15, IX.11.16 |
| 인스펙터 CSS 범주 재구성 | IX.11.17 |
| export 정렬(modifier 선형화, capability matrix, SVG) | IX.11.18, IX.11.19, IX.11.20 |
