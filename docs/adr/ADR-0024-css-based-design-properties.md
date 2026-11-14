# ADR-0024: CSS 기반 디자인 속성 어휘

## Status

Accepted (2026-11-14, 제품 소유자 방향 P3)

## Context

제품 소유자는 2026-11-14에 다음 방향을 정했습니다.

- 인스펙터에서 편집하는 디자인 속성은 기본적으로 CSS를 기반으로 합니다. CSS 속성 어휘, 값 문법,
  레이아웃 의미(box model, Flexbox, Grid, positioning, typography, background·border, effect,
  transform, opacity·blend)를 따릅니다.
- 이후 이 속성들은 노드 기반 프로토타이핑(patch·node 그래프가 속성과 인터랙션을 구동)으로
  연결됩니다. 컴포넌트는 property drilling과 context(트리 아래로 공급·소비)를 지원하고, 디자인은
  실제 코드로 변환됩니다.
- 편집 UX는 전문 UI 디자인 도구 사용자가 기대하는 관례를 참고하되, 다른 제품의 외형·문구·독점
  세부는 옮기지 않습니다. UI는 macOS 네이티브 바탕 위의 Stellar 고유 형태입니다
  (`docs/design/macos-shell-design.md`).

현재 코드는 다음 상태입니다(2026-11-14 HEAD 기준 읽기 조사).

- 속성 식별자는 숫자 `PropertyKey`입니다. Frame은 `0x01xx`, Image는 `0x06xx`, ComponentInstance는
  `0x0Axx`, Modifier는 `0x1000 | kind << 4 | field`입니다. registry 표시 이름은 일부만 CSS와 비슷한
  kebab-case입니다(`corner-radius`, `layout-mode`).
- 영속 형식에 들어간 것은 Frame·Shape·Path·Text·Image payload, modifier stack 12종, core 값 타입,
  토큰·binding·component입니다. 레이아웃 어휘(Free·Stack·Grid·Overlay·Constraint, `SizingMode`,
  `FlexItem`, `GridItem`, padding)는 `crates/layout` 엔진 입력에만 있고 문서 저장 record가 없습니다.
  document→layout 연결(X.1.2)도 아직 없습니다.
- `Length`·`Angle`·`Duration`은 단위 없는 `FiniteF32` newtype입니다(ADR-0008). px·°·ms는 인스펙터의
  표시 가정일 뿐 타입 계약이 아닙니다.
- Typography는 불투명 placeholder이고 font family·size·weight·style이 문서 모델에 없습니다.
- 레이아웃 엔진의 일부 기본값·의미가 CSS와 다릅니다. `FlexItem` shrink 기본 0, cross 정렬 기본
  Start, flex item 자동 최소 크기 없음, Free anchor 기준이 content box 등입니다.
- Phase XXII 코드 생성 계획에는 속성 단위 매핑 규칙이 없습니다. 매핑 근거는 Control export hint와
  SVG 매핑표(`crates/export/src/svg/mapping.rs`)뿐입니다.

제약은 다음과 같습니다.

- 저장은 결정론적 바이너리 chunk 형식입니다(ADR-0010, ADR-0011, ADR-0012). 결정론 규칙(아키텍처
  §3.1, 레이아웃 1/64 px 정수 단위)을 유지해야 합니다.
- 캔버스 계산에 운영체제의 실제 CSS 엔진을 쓰지 않습니다(아키텍처 §13).
- modifier stack은 순서 있는 비파괴 구조입니다(아키텍처 §12.6, VI.9 완료).
- 컴포넌트 경계를 넘는 참조는 명시적이어야 합니다(아키텍처 §14.3, §15.1).
- Web·Apple·Android는 Target Profile capability로 판정합니다(아키텍처 §17).

기준 없이 layout record, Typography record, Shape·Path·Text PropertyKey를 각각 만들면 결정마다 다른
어휘로 굳습니다. 그러면 코드 생성, 붙여넣기, 노드 그래프 포트가 매번 별도 변환표를 가져야 합니다.
레이아웃은 아직 영속 형식이 없어서 지금 정렬하는 비용이 가장 낮습니다.

## Decision

1. **정본 어휘는 CSS입니다.** 디자인 속성의 이름, 값 문법, 단위, 상자·레이아웃 의미는 W3C CSS
   명세의 longhand 속성을 정본으로 삼습니다. 새 디자인 속성은 대응하는 CSS longhand가 있으면 그
   이름·문법·의미를 쓰고, 없을 때만 Stellar 고유 속성으로 정의합니다. 고유 속성은 registry에
   `stellar-only`로 표시합니다.
2. **어휘 결정이지 저장 형식 결정이 아닙니다.** 파일은 지금처럼 바이너리 chunk payload와 숫자
   `PropertyKey`로 저장합니다. CSS 텍스트를 파일에 저장하지 않습니다. CSS 이름은 registry
   metadata이며 식별자가 아닙니다. 식별자는 계속 `PropertyKey`이고, CSS 이름을 바꿔도 파일 바이트는
   바뀌지 않습니다.
3. **부분집합 규칙.** Stellar는 CSS가 받는 값 일부를 구조화된 진단과 함께 거부할 수 있습니다. 하지만
   Stellar가 받아들인 선언은 같은 CSS 선언과 같은 계산 결과를 내야 합니다(레이아웃은 1/64 px 반올림
   범위 안). 이 규칙을 지킬 수 없는 동작은 "Stellar 확장"으로 registry에 명시하고, Web Target
   Profile에서 `approximated`로 진단합니다. 허용하는 확장 목록은
   `docs/design/property_model.md` 14장이 전부입니다.
4. **longhand만 저장합니다.** 영속 record는 longhand만 가집니다. shorthand(`padding`, `inset`,
   `gap`, `flex`, `border`, `border-radius`, `grid-area`, `font` 등)는 인스펙터 편집·입력·복사에서만
   쓰는 표현입니다. 입력된 shorthand는 CSS 명세의 확장 규칙으로 longhand 집합으로 펼쳐 하나의
   CommandTransaction으로 적용합니다. 속성 단위 3-way merge(아키텍처 §27.1)의 단위도 longhand입니다.
5. **단위를 고정합니다.**
   - `Length`는 CSS `px`(Stellar 논리 픽셀), `Angle`은 `deg`, `Duration`은 `ms`, `Percentage`는
     `%`(값 50 = 50%), `Scalar`는 CSS `<number>`입니다. 인스펙터가 이미 이 단위로 표시하므로 기존
     바이트의 의미는 바뀌지 않습니다.
   - `fr`은 grid track에서만 씁니다.
   - 입력에서는 `rad`·`grad`·`turn`과 `s`도 받아 정규 단위로 바꿉니다.
   - `em`은 CSS가 요소 자신의 font-size를 기준으로 정의한 속성(예: `letter-spacing`) 중 registry가
     허용한 경우에만 씁니다.
   - `rem`, viewport 단위, container 단위, 길이·백분율을 섞는 `calc()`는 초기 어휘에 넣지 않습니다.
     입력하면 `unsupported-unit` 또는 `unsupported-value`로 진단합니다. 추가하려면 기준 font-size와
     viewport를 Target Profile·환경 key로 정의해야 하므로 별도 ADR이 필요합니다.
6. **기본값.**
   - 아직 영속 형식이 없는 속성(레이아웃 전부, Typography)은 생략값·기본값을 CSS initial value로
     정합니다.
   - 이미 저장되는 속성의 기본값은 바꾸지 않습니다. 바꾸면 기존 저장값의 의미가 조용히 달라지기
     때문입니다. CSS initial value와 다른 기존 기본값(예: Frame clip `FrameBounds` 대 `overflow:
     visible`, Image fit `Contain` 대 `object-fit: fill`)은 registry에 `initial-differs`로 기록하고,
     export는 그 값을 항상 명시적으로 출력합니다.
   - 새 노드를 만들 때 쓰는 생성 preset(예: 새 Stack의 cross 정렬)은 편집 UX 결정입니다. preset
     값은 명시 값으로 저장합니다.
7. **레이아웃 의미는 CSS를 따릅니다.**
   - Stack은 `display: flex`, Grid는 `display: grid`, Overlay는 모든 자식이 같은 영역에 놓이는 단일
     영역 grid, Free는 `position: relative` 부모 안의 `position: absolute` 자식입니다. Free의
     containing block은 CSS처럼 padding box입니다.
   - Constraint는 Stellar 확장입니다. 부모 edge 제약은 inset으로 표현하고, 형제·baseline 제약은 CSS
     anchor positioning 계열 capability로 판정합니다.
   - `SizingMode`(Fixed·Fill·HugContent·Percentage)는 영속 형식이 아니라 인스펙터 preset입니다.
     저장은 `width`·`height`·`flex-*`·`align-self`·`min-*` 등 longhand로 합니다. preset과 longhand의
     대응은 `docs/design/property_model.md` 4.3절입니다.
   - `box-sizing`은 `border-box`로 고정하며 편집 속성이 아닙니다. border는 CSS처럼 레이아웃 공간을
     차지합니다(border box 안쪽이 padding box).
   - 기존 엔진 동작 중 CSS와 다른 것은 IX.11의 migration 행으로 CSS에 맞춥니다.
8. **위치와 transform을 분리합니다.** 위치는 레이아웃 속성(inset 또는 flow)으로, 시각 변형은 CSS
   개별 transform 속성(`translate`, `rotate`, `scale`)과 `transform`, `transform-origin`으로
   저장합니다. 지금 Frame transform 행렬의 이동 성분을 위치로 쓰는 모델은 schema migration(V.8
   경로)으로 옮깁니다. 기존 노드의 외형을 보존하려고 migration된 노드에는 `transform-origin: 0 0`을
   명시 저장합니다. 새 노드의 기본값은 CSS initial value `50% 50%`입니다.
9. **modifier stack은 Stellar 구조로 유지하고, 파라미터는 CSS를 따릅니다.**
   - 각 modifier의 파라미터 이름·문법은 대응 CSS 속성(`box-shadow`·`drop-shadow()`, `filter`,
     `mix-blend-mode`, `opacity`, `clip-path`, `mask`, `transform`, background·border paint)을
     따릅니다.
   - CSS의 고정 적용 순서와 맞는 stack은 1:1로 옮깁니다. 순서가 다르거나 반복되면 Web export에서
     wrapper 요소로 나누거나(`approximated`) 부분 래스터화합니다.
   - Boolean·StrokeExpand·Corner는 `stellar-only`이고, export할 때 path로 굽습니다.
   - backdrop blur 같은 새 효과는 CSS 이름(`backdrop-filter`)으로 추가하되 별도 WBS로 다룹니다.
10. **색.** Color 값은 CSS Color 4 `color()` 의미(색공간 태그 sRGB·Display P3, straight alpha, 범위 밖
    채널 허용)입니다. 정규 텍스트 표현은 `docs/design/property_model.md` 8.2절입니다. 합성은
    아키텍처 §10.3의 선형 공간을 유지합니다. 따라서 Web profile에서 blend·겹친 opacity 결과는
    `approximated`입니다.
11. **토큰.** `TokenDefinition`은 타입 있는 CSS custom property(`@property`로 등록한 것과 같은 의미:
    syntax는 TokenType, initial-value는 기본값)에 대응합니다. 문서 안의 참조는 계속 StableId입니다.
    텍스트 표현과 export에서만 `var(--name)`을 씁니다. CSS 이름은 TokenName에서 결정론적으로
    파생하고, 충돌하면 진단합니다. 환경 key는 media feature와 상태 pseudo-class에 대응합니다. CSS
    상속처럼 컴포넌트 경계를 암묵적으로 넘는 공급은 이 ADR 범위 밖입니다. 아키텍처 §14.3의 명시적
    경계 규칙을 유지하며, context 모델은 별도로 결정합니다.
12. **비웹 타깃은 CSS 의미에서 낮춥니다.** CSS 의미가 원본 의미이고 Web은 그 표현 타깃 중
    하나입니다. Apple·Android adapter는 CSS 의미를 기준으로 낮추고(lowering), 표현할 수 없으면
    Target Profile capability 진단(아키텍처 §17)을 냅니다.
13. **포트 단위.** 노드 기반 프로토타이핑 포트, Component Property 타입, binding 대상은 registry의 CSS
    longhand 단위로 노출합니다. 포트 하나는 longhand 하나이고, 값 타입은 registry의 값 문법입니다.
    그래프 모델 자체는 이 ADR 범위 밖입니다.

세부 매핑 표, 정규 텍스트 형식, 인스펙터 범주는 `docs/design/property_model.md`에 둡니다. 구현은
계획 IX.11(CSS 기반 속성 체계)이 맡습니다.

## Alternatives

- **CSS 텍스트를 저장 형식으로 사용:** 결정론적 chunk diff, StableId 참조, 파서 공격면(아키텍처
  §37.2) 문제 때문에 탈락했습니다. 문자열 치환 기반 `var()` 해석은 타입 검사와 cycle 사전 거부
  (VIII.5.3)와도 맞지 않습니다.
- **자체 어휘를 유지하고 export에서만 CSS로 변환:** 전문 디자인 도구에서 흔한 방식입니다. 하지만
  코드 변환, 붙여넣기, 노드 포트마다 변환표가 중복되고 제품 소유자 방향과 어긋나 탈락했습니다.
- **SwiftUI 또는 Compose 어휘를 정본으로 사용:** 세 타깃 중 공개 표준 명세와 정식 값 문법을 가진
  것은 CSS뿐입니다. 상자·flex·grid 의미도 CSS가 가장 자세히 명세되어 있어 탈락했습니다.
- **modifier stack을 CSS 고정 순서 속성으로 대체:** 비파괴 재정렬 요구(아키텍처 §12.6)와 완료된
  VI.9·VII.2.6 행과 충돌해 탈락했습니다.
- **이미 저장되는 속성의 기본값도 CSS initial value로 변경:** 기본값을 생략해 인코딩한 기존 문서의
  의미가 바뀔 위험이 있어 탈락했습니다.
- **shorthand 저장:** merge 단위가 커지고, longhand 일부만 바꾼 두 변경이 거짓 충돌을 만들어
  탈락했습니다.

## Consequences

- Web adapter(XXII.2)의 속성 변환은 대부분 항등 매핑이 됩니다. Apple·Android adapter도 같은 의미
  표를 입력으로 씁니다.
- 인스펙터 입력, 복사·붙여넣기, 노드 그래프 포트가 같은 CSS 값 문법을 공유합니다. CSS를 아는
  사용자는 그 지식을 그대로 씁니다.
- 레이아웃 엔진의 기본값·의미 migration과 X.11.1 적합성 corpus 기대값 갱신이 필요합니다.
- Frame 위치·transform 분리, Frame border longhand, Text letter-spacing 위치 변경은 schema
  migration이 필요합니다.
- 인스펙터 섹션을 CSS 범주로 재구성해야 합니다.
- CSS가 직접 표현하지 않는 디자인 도구 개념(채움, 내용 맞춤)은 preset 계층이 필요합니다.
- CSS 명세가 바뀌면 추적해야 합니다. 이 부담은 Web Target Profile의 capability 버전 고정(IX.10.4)으로
  제한합니다.

## Compatibility

- 이 ADR 자체는 파일 바이트, 공개 API, FFI를 바꾸지 않습니다.
- 기존 `PropertyKey` 숫자는 유지합니다. 새 longhand에는 새 key를 할당하고 기존 key를 재사용하지
  않습니다.
- Frame payload(위치·transform 분리, border longhand)와 Text(letter-spacing 위치, line-height 의미)
  변경은 schema version 증가, V.8 migration step, golden fixture 갱신으로 진행합니다. 각 행이 새
  schema version을 기록합니다.
- 인스펙터 snapshot에 CSS 이름 문자열을 실어야 하면 FFI 표면이 바뀝니다. 그 변경은 해당 구현
  행에서 ADR-0014 절차로 다룹니다.
- 파일 포맷 공개 명세(Phase XXVIII)에는 CSS 매핑 표를 부록으로 포함합니다.

## Performance

- registry CSS metadata는 정적 표이므로 런타임 비용이 없습니다.
- CSS 텍스트 파싱은 입력 확정과 붙여넣기 때만 실행하고 인터랙션 경로(T0\~T5)에는 들어가지 않습니다.
- flex item 자동 최소 크기(`min-width: auto`)는 intrinsic 측정을 추가로 요구합니다. 증분 layout
  benchmark(X.7.4)로 방문 범위를 확인합니다.

## Security

- CSS 텍스트 파서는 신뢰할 수 없는 입력(클립보드, 외부 파일)을 받습니다. 입력 길이 상한, 재귀 없는
  고정 문법, 허용 목록에 있는 속성만 받기, `url()`과 `@import` 거부(외부 리소스 로드 없음)를
  지킵니다. 파서를 fuzz 대상(XVI.6)에 추가합니다.

## Migration

1. registry CSS metadata(IX.11.1\~IX.11.3)를 먼저 채웁니다. 파일 변경은 없습니다.
2. 레이아웃 엔진 의미를 CSS에 맞추고(IX.11.4, IX.11.5) CSS 적합성 fixture(IX.11.8)로 고정한 뒤 영속
   layout record(IX.11.6)를 정의합니다.
3. Frame·Text schema migration(IX.11.9, IX.11.10, IX.11.12)은 V.8 경로를 씁니다. 원본 generation을
   보존하고, 자동 commit하지 않으며, 외형 보존 fixture를 둡니다.
4. 인스펙터·복사·export 연결(IX.11.13\~IX.11.20)은 위 단계를 선행으로 둡니다.
