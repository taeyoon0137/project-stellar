# ADR-0076: 레이아웃 엔진 입력을 CSS longhand 값으로 바꾸고 CSS 배치 의미로 계산

## Status

Accepted (2026-11-23). ADR-0074 결정 6(레이아웃 엔진 입력을 CSS 값으로 다시 모델링)을 구현하는 결정입니다. 대상 행은
IX.11.4(Flexbox), IX.11.5(Grid·positioning), IX.11.23(flex item 자동 최소 크기), IX.11.8(CSS 적합성 fixture),
X.10.4(Text intrinsic·baseline hook), X.3.6(baseline 정렬)입니다.

## Context

- `crates/layout` 입력은 엔진 고유 어휘였습니다. 배치 방식은 `ContainerLayout::{Free, Stack, Grid, Overlay, Constraint}`,
  축 크기는 `SizingMode::{Fixed, Fill, HugContent, Percentage}`, flex item은 `FlexItem`, 부모 기준 고정은 `Constraint`
  등식이었습니다. ADR-0074는 이 어휘를 CSS 값 입력으로 바꾸고, 영속 layout record(IX.11.6)는 CSS longhand만 저장하며
  둘 사이에 크기 방식 변환 계층을 두지 않도록 정했습니다.
- 속성 모델(`docs/design/property_model.md`) 4장은 엔진 값과 CSS의 차이를 적어 두었습니다. 대표적으로 `flex-shrink`
  생략값 0, `align-items` 생략값 Start, Stretch가 HugContent 자식도 늘림, Free containing block이 content box, absolute
  자식이 부모 Hug 크기에 기여, bare `fr` 미지원, fr 합이 1보다 작을 때의 분배, 자동 최소 크기 없음입니다.
- 이 crate는 문서 registry에 의존하지 않습니다(core-types만 의존). record → 엔진 입력 변환은 css-persistent-records
  lane(ADR-0087 예정)이 붙입니다.
- 받아들인 CSS 선언의 결과는 CSS와 같아야 합니다(ADR-0024 결정 3). 판정 기준은 Chromium 계열 고정 버전 하나입니다
  (속성 모델 1.3절, 계획 IX.11.8).

## Decision

1. **엔진 입력은 `stellar_layout::style`의 CSS 값 타입입니다.** node 하나는 `LayoutNode { id, children, style:
   LayoutStyle, content, transform }`입니다. `LayoutStyle`은 다음 longhand를 담습니다: `display`(`block` · `flex` ·
   `grid` · `none`), `position`(`static` · `relative` · `absolute`), `top`·`right`·`bottom`·`left`, `width`·`height`·
   `min-*`(`auto` · `<length>` · `<percentage>` · `min-content` · `max-content` · `fit-content`), `max-*`(`none`과 같은 값),
   `aspect-ratio`, `padding-*`(px), `flex-direction`, `flex-wrap`, `justify-content`, `align-content`, `align-items`,
   `justify-items`, `row-gap`·`column-gap`, `flex-grow`·`flex-shrink`·`flex-basis`, `align-self`·`justify-self`,
   `grid-template-columns`·`-rows`, `grid-auto-columns`·`-rows`, `grid-auto-flow`, `grid-column-start`·`-end`,
   `grid-row-start`·`-end`. 이 타입은 document registry 타입에 의존하지 않습니다. 오류·진단·dirty registry의 속성 이름도
   CSS longhand 이름입니다(`LayoutProperty`).
2. **엔진 고유 배치·크기 방식 이름을 없앱니다.** Free는 `position: absolute` + inset, Overlay는 한 칸 grid(모든 자식
   `grid-area: 1 / 1`), Constraint의 부모 변 기준 고정은 inset입니다. 형제·baseline 기준 배치는 CSS 값이 없어 받지
   않습니다(ADR-0074 결정 5). `SizingMode`, `Fixed`·`Fill`·`HugContent`, `Stack`, `Overlay`, `Constraint`, `FlexItem`은
   crate 공개 API와 출력에서 사라집니다. 표시 여부는 `display: none`입니다.
3. **생략값은 CSS initial value입니다**(`flex-shrink: 1`, `align-items`·`align-content`·`justify-content`·`justify-items`
   `normal`, `min-*: auto`, `max-*: none`, `grid-auto-rows`·`grid-auto-columns: auto`). 예외는 `display` 하나이며 생략값은
   `div`의 UA 기본값 `block`입니다(Stellar에는 inline formatting context가 없음). 고정 값은 `box-sizing: border-box`,
   `margin: 0`, `border: 0`, `direction: ltr`, `writing-mode: horizontal-tb`, `overflow: visible`입니다.
4. **containing block과 root.** 모든 box는 absolute 자식의 containing block이고 그 영역은 부모 **padding box**입니다
   (border가 없어 border box와 같음). Web export가 모든 요소를 positioned로 내보내는 규칙(속성 모델 3장)과 같은 결과입니다.
   root는 containing block이 없으므로 위치 (0, 0), 가로 가용 공간 `ROOT_AVAILABLE_PX`(1,000,000 px), 세로 indefinite로
   배치합니다. 그래서 root `auto` 너비는 max-content(가용 공간 안), root 자신의 백분율 크기는 `auto`입니다.
5. **계산 모델.** node 배치는 요청(`known` 크기, 자신의 백분율 기준, `auto` 가용 공간, 콘텐츠만 측정 여부)의 순수
   함수이고, 결과(크기·첫 baseline·자식 배치·진단)를 (node, subtree input hash, 정규화한 요청, 측정기 revision)으로
   memo·cache합니다. 결과에 쓰이지 않는 요청 값(백분율을 쓰지 않는 축의 기준, `known` 축의 가용 공간)은 정규화합니다.
   너비만 묻는 intrinsic 요청은 높이·자식 배치를 계산하지 않습니다. 자식 배치는 StableId로 가리킵니다. transform은
   subtree hash에 들어가지 않습니다. 최종 결과는 root 요청에서 자식 배치 요청을 따라 pre-order로 조립합니다.
6. **배치 의미.** `block`은 CSS 2 block flow(margin 없음), `flex`는 CSS Flexbox 1 §9, `grid`는 CSS Grid 1 §8·§11,
   positioning은 CSS Positioned Layout 3을 따릅니다. 행별 세부:
   - IX.11.4: stretch(`normal` 포함)는 cross 크기 속성이 `auto`인 item만 늘립니다. `row-gap`·`column-gap`은 축 고정
     이름입니다. 분배 정렬 fallback은 Box Alignment 3대로 `space-between` → `flex-start`, `space-around`·
     `space-evenly` → `safe center`(남는 공간이 음수면 시작 쪽)입니다. 음수 남는 공간에서 시작 쪽에 놓이는 것은
     Chromium 157 fixture(`flex/negative-free-space-*`)로 확인했습니다.
   - IX.11.5: grid item 생략 정렬은 `normal`(stretch, 크기 `auto` 축만), `justify-self`·`align-self`를 받습니다. track
     최솟값 `auto`를 두어 bare `fr`은 `minmax(auto, <n>fr)`, `minmax(0, <n>fr)`은 그대로입니다. fr 크기는 CSS Grid
     §11.7(합이 1보다 작으면 1로 나눔, indefinite면 factor 1 이하 track은 나누지 않음)입니다. `auto` track을 track
     어휘에 더하므로 `grid-auto-rows`의 CSS initial value `auto`를 그대로 표현합니다. 영속 record(IX.11.6)는
     `grid-auto-rows`를 생략할 수 있고 생략값은 `auto`입니다.
   - IX.11.23: flex item `min-*: auto`는 scroll container가 아닌 item의 main 축 콘텐츠 기반 최소 크기(min-content
     크기를 definite max로 제한한 값과 지정 크기 중 작은 값, 비율 item은 비율 변환 포함)입니다.
   - X.10.4·X.3.6: 첫 baseline은 block이면 baseline을 가진 첫 in-flow 자식, flex면 첫 줄에서 baseline 정렬에 참여하는
     item 또는 첫 item, grid면 첫 행의 첫 item에서 옵니다. baseline이 없는 item은 border box 아래 변으로 합성합니다.
     Text intrinsic(min-content·max-content 너비, 너비에서의 높이, 첫 baseline)은 측정 hook(`ContentMeasurer`)으로
     들어옵니다. `align-items`·`align-self: baseline`은 가로 main 축 flex container에서 받습니다.
7. **받아들이는 부분집합.** CSS가 받지만 같은 결과를 보장하지 않는 값은 `LayoutError::Unsupported`로 거부합니다: grid
   item의 `baseline` 정렬, 가로 축 baseline 정렬(`justify-*: baseline`), grid container 안 absolute 자식의 명시 line
   배치. root의 `display: none`은 `InvalidValue`입니다.
8. **진단.** 이전 HugContent 순환·constraint 진단 대신 `LayoutDiagnostic::CyclicPercentage`(콘텐츠로 정해진 너비의
   containing block 안 백분율 크기, CSS Sizing 3 §5.2.1)만 둡니다. CSS처럼 크기 계산에서는 백분율을 `auto`로 보고 정해진
   크기로 다시 해석합니다.
9. **적합성 판정.** 기대값은 Chromium 계열 고정 버전(Google Chrome 157.0.8103.79, headless)으로 생성한 border box
   rect(1/64 px)를 `crates/layout/tests/css/expected.json`에 정적으로 기록한 값입니다. 생성기
   (`crates/layout/tests/css/generate.py`, Python 표준 라이브러리)와 브라우저는 개발 도구이며 빌드·테스트 의존성이
   아닙니다. 테스트는 브라우저를 실행하지 않습니다. `basic` 묶음은 1 LayoutUnit, `distribution` 묶음은 같은 줄 item 수
   이하 LayoutUnit 차이를 허용합니다. 이 ADR이 개발 도구 도입 승인 기록이고, `config/dependencies.yaml`(cargo ecosystem
   형식) 등록은 이 lane 소유 경로 밖이라 통합 때 정합니다.
10. **증분 레이아웃.** dirty node에서 위로 올라가 **배치 경계**(부모가 border-box 크기·baseline만 읽는 node: `width`·
    `height`가 모두 `<length>`이고 absolute이거나 부모가 `display: block`)에서 다시 배치합니다. flex·grid item은 자동 최소
    크기·track 크기가 콘텐츠를 읽으므로 경계가 아닙니다. 다시 배치한 subtree 안에서 cache 결과가 같은 자식은 방문하지
    않습니다.
11. **codegen IR.** `stellar_codegen::ir::IrLayout`은 `LayoutStyle`을 그대로 담고 semantic snapshot은 CSS 생략값과 다른
    longhand를 `name: value` 텍스트로 냅니다(snapshot `codegen-ir v2`).

## Alternatives

| 대안 | 판단 |
|---|---|
| 기존 엔진을 두고 CSS 값 → 엔진 값 변환 계층만 추가 | 채택하지 않음. ADR-0074가 금지한 두 번째 모델이 영구히 남고, stretch 대상·자동 최소 크기·padding box containing block·fr 규칙처럼 CSS와 다른 의미를 변환으로 맞출 수 없습니다 |
| 외부 레이아웃 crate(Taffy 등) 도입 | 채택하지 않음. 1/64 px 정수 산술·잔여 subpixel 순서·결정성 계약(X.1.5)과 측정 hook·증분 계약(X.7)을 이 crate가 소유해야 하고, 의존성 승인 범위가 넓어집니다 |
| 테스트 때 브라우저를 실행해 기대값 비교 | 채택하지 않음. CI·로컬 재현성이 브라우저 설치와 버전에 묶입니다. 정적 기록과 고정 버전 표기로 같은 판정을 냅니다 |
| `display` 생략값을 CSS initial value `inline`으로 | 채택하지 않음. inline formatting context가 없어 결과가 CSS와 달라집니다. 디자인 node는 `div` 계열이라 UA 기본값 `block`이 같은 결과입니다 |
| absolute containing block을 가장 가까운 positioned 조상으로 | 채택하지 않음. 모든 요소를 positioned로 내보내는 export 규칙에서는 부모와 같고, 문서 모델에 positioned 개념을 따로 두지 않습니다 |

## Consequences

- `stellar-layout` 공개 API가 바뀝니다(`style` 모듈 신설, `sizing`·`stack`·`free`·`grid`(이전)·`overlay`·
  `constraint` 모듈 삭제, `LayoutNode` 필드 교체, `LayoutProperty` CSS 이름, `LayoutDiagnostic` 교체). 사용처는
  `stellar-codegen` 하나이며 같은 커밋 범위에서 고칩니다.
- X.11.1 corpus는 같은 seed·난수 순서로 CSS longhand를 내는 생성기로 옮겼고(`corpus.rs`), X.11.2 component corpus는
  rect가 모두 같고 container 첫 baseline만 더해졌습니다. 바뀐 fixture와 이유는 행 보고서에 있습니다.
- 증분 레이아웃 지표(`UpdateStats`)는 `replaced` 대신 `relayout_roots`를 냅니다.
- 속성 모델 4장(특히 4.2절 `space-around`·`space-evenly` 확인 결과, 4.4절 `grid-auto-rows` 결정)은 이 ADR과 행
  보고서를 근거로 속성 모델 소유 lane이 고칩니다.

## Compatibility

- 파일 바이트·FFI·저장 형식 변경 없음(영속 layout record가 아직 없음). 런타임 비교용 canonical bytes는 입력 `SLYI` v2,
  결과 `SLYR` v2로 올렸습니다(저장하지 않는 형식). codegen semantic snapshot은 `codegen-ir v2`입니다.

## Performance

- X.7.4 benchmark(200,201 node, 단일 leaf 너비 edit 64회)에서 edit당 방문 3 node·재사용 107로 이전과 같습니다. 시간은
  같은 기계·같은 부하에서 이전 엔진과 함께 측정해 행 보고서에 기록합니다.

## Security

- 신뢰할 수 없는 입력의 작업량은 tree 깊이·node 수 상한(`MAX_LAYOUT_DEPTH`, view 상한)과 grid track 상한
  (`MAX_GRID_TRACKS`)으로 제한합니다. fixture 생성기는 로컬 파일만 읽고 쓰며 빌드에 포함되지 않습니다.

## Migration

- 저장된 layout 데이터가 없어 문서 migration은 없습니다. record → 엔진 입력 변환(1:1, 크기 방식 계층 없음)은
  css-persistent-records lane이 붙입니다. 기존 엔진 테스트·corpus·benchmark·codegen fixture는 같은 커밋 범위에서 CSS
  입력으로 옮겼습니다.
