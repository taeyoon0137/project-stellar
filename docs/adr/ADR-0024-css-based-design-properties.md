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
  실제 코드로 변환됩니다. 이 세 방향은 ADR-0025가 정합니다.
- 편집 UX는 전문 UI 디자인 도구 사용자가 기대하는 관례를 참고하되, 다른 제품의 외형·문구·독점
  세부는 옮기지 않습니다. UI는 macOS 네이티브 바탕 위의 Stellar 고유 형태입니다
  (`docs/design/macos-shell-design.md`).

현재 코드는 다음 상태입니다(2026-11-14 HEAD 기준 읽기 조사).

- 속성 식별자는 숫자 `PropertyKey`입니다. Frame은 `0x01xx`, Image는 `0x06xx`, ComponentInstance는
  `0x0Axx`, Modifier는 `0x1000 | kind << 4 | field`입니다. registry 표시 이름은 일부만 CSS와 비슷한
  kebab-case입니다(`corner-radius`, `layout-mode`).
- 일부 key는 값 여러 개를 담은 합성 key입니다. Frame `size`(0x0101)는 너비·높이, `transform`(0x0102)은
  행렬 6성분, `border`(0x0105)는 색·두께, `corner_radius`(0x0106)는 4 모서리, modifier
  `SHADOW_OFFSET`(0x1092)은 x·y입니다. 값 타입은 commands `PropertyValueType`의 Size·Transform·
  OptionalBorder·CornerRadius·Offset뿐입니다(`crates/commands/src/property.rs`). 속성 경로를 저장하는
  참조(ValueBinding target, View Variant·Component variant override property patch)의 값 타입인
  `TypedValue`·`ExpressionType`에는 이런 합성 타입이 없습니다(`crates/core-types/src/typed_value.rs`,
  `crates/expressions/src/types.rs`). semantic snapshot도 `corner_radius` 4값을 경로 하나에 둡니다
  (`crates/semantic-diff/src/build.rs`).
- 영속 형식에 들어간 것은 Frame·Shape·Path·Text·Image payload, modifier stack 12종, core 값 타입,
  토큰·binding·component입니다. 레이아웃 어휘(Free·Stack·Grid·Overlay·Constraint, `SizingMode`,
  `FlexItem`, `GridItem`, padding)는 `crates/layout` 엔진 입력에만 있고 문서 저장 record가 없습니다.
  Frame `layout_mode`가 가리킬 layout 정의 record도 없습니다. document→layout 연결(X.1.2)도 아직
  없습니다.
- Text payload는 내용과 paragraph·run의 style index만 저장합니다. `TextStyle`·`ParagraphStyle` 표는
  저장 형식이 없습니다(`crates/document/src/text.rs` `TextPayload::encode`).
- schema migration runner(V.8)는 있지만 제품 경로에 연결되어 있지 않습니다. step을 등록하는 곳은
  test와 xtask의 noop v1→v2뿐이고(`tools/xtask/src/migration_fixtures.rs`), project 열기
  (`crates/project/src/saved_project.rs` `SavedProjectLoader::open`)는 migration 없이 current
  generation을 엽니다. `FramePayload::decode`는 version 없이 잉여 bytes를 거부합니다
  (`crates/document/src/frame.rs`).
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
   Stellar가 받아들인 선언은 같은 CSS 선언과 같은 계산 결과를 내야 합니다. 레이아웃의 기준 브라우저,
   1/64 px 반올림과 잔여 분배 허용 범위는 `docs/design/property_model.md` 1.3절이 정합니다. 이 규칙을
   지킬 수 없는 동작은 "Stellar 확장"으로 registry에 명시하고, Web Target Profile에서 `approximated`로
   진단합니다. 허용하는 확장 목록은 `docs/design/property_model.md` 14장이 전부입니다.
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
   저장합니다. 지금 Frame transform 행렬의 이동 성분을 위치로 쓰는 모델은 schema migration(계획
   IX.11.21의 제품 문서 migration registry, V.8 runner 위)으로 옮깁니다.
   - migration은 행렬의 이동 성분만 옮깁니다. 부모가 Free 방식인 자식은 `left`·`top`으로 옮깁니다.
     지금 저장 형식에는 layout 정의 record가 없으므로 모든 자식이 이 경우입니다. 부모가 flow 방식
     (Stack·Grid·Overlay)인 자식은 `translate`로 옮깁니다. CSS는 static 요소의 `left`·`top`을
     무시하기 때문입니다.
   - 부모가 없는 Frame(View root Frame, ComponentDefinition root)의 이동 성분은 페이지 캔버스 위
     배치이고 CSS containing block 안의 위치가 아닙니다. 그래서 위치 longhand로 옮기지 않고 x·y 두
     longhand로 된 stellar-only 캔버스 배치 속성으로 옮깁니다. Web export는 이 값을 출력하지 않습니다.
   - 선형 부분은 `transform: matrix(a, b, c, d, 0, 0)`으로 bit 그대로 보존합니다. `rotate`·`scale`로
     분해해 저장하지 않습니다. 분해에는 atan2·sin·cos가 필요한데 ADR-0006은 libm 초월함수 결과를
     플랫폼 의존으로 봅니다. 다시 합성한 f32 행렬도 원래 행렬과 bit 단위로 같지 않습니다.
   - 인스펙터가 보여 주는 migration된 노드의 회전·배율은 행렬에서 계산한 표시 전용 파생값입니다.
     저장하지 않고 golden이나 scene 입력에도 쓰지 않습니다. 새 편집은 `rotate`·`scale`·`translate`
     longhand를 씁니다.
   - 기존 노드의 외형을 보존하려고 migration된 노드에는 `transform-origin: 0 0`을 명시 저장합니다.
     새 노드의 기본값은 CSS initial value `50% 50%`입니다.
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
    syntax는 TokenType, initial-value는 기본 환경에서 평가한 기본값)에 대응합니다. 문서 안의 참조는 계속
    StableId입니다. 텍스트 표현과 export에서만 `var(--name)`을 씁니다. CSS 이름은 TokenName에서
    결정론적으로 파생하고, 충돌하면 진단합니다. 등록 속성은 `inherits: true`로 냅니다. `inherits: false`이면
    `:root`에 둔 값이 자손에 상속되지 않아 자손의 `var()`가 initial-value를 받기 때문입니다. CSS
    initial-value는 계산상 독립적인 값이어야 해서 `var()`를 담을 수 없으므로, default expression을 기본
    환경에서 평가한 literal을 씁니다. 다른 토큰을 참조하는 값과 환경 조건 값은 `:root`와 `:root`를 감싼
    `@media` 선언으로만 냅니다. 환경 key는 media feature에 대응합니다. `interaction_state`는 평가 환경
    전체에 하나뿐인 key라 요소마다 판정되는 상태 pseudo-class와 뜻이 다르므로 토큰 export 대상에서 빼고,
    Control 상태 selector 규칙(계획 XXII.2.6)으로 냅니다. CSS 상속처럼 컴포넌트 경계를 암묵적으로
    넘는 공급은 이 ADR 범위 밖입니다. 아키텍처 §14.3의 명시적 경계 규칙을 유지하며, context 모델은
    ADR-0025 결정 3(Component Context)이 정합니다. Context를 Web에서 등록 custom property 상속으로
    낮출 때(ADR-0025 결정 4.3)도 이 결정의 `inherits` 정책과 같은 이름 공간·충돌 규칙을 씁니다
    (`docs/design/property_model.md` 10장).
12. **비웹 타깃은 CSS 의미에서 낮춥니다.** CSS 의미가 원본 의미이고 Web은 그 표현 타깃 중
    하나입니다. Apple·Android adapter는 CSS 의미를 기준으로 낮추고(lowering), 표현할 수 없으면
    Target Profile capability 진단(아키텍처 §17)을 냅니다.
13. **포트 단위.** 노드 기반 프로토타이핑 포트, Component Property 타입, binding 대상은 registry의 CSS
    longhand 단위로 노출합니다. 포트 하나는 longhand 하나(결정 14에 따라 PropertyKey 하나)이고, 값
    타입은 registry의 값 문법입니다. 그래프 모델과 포트 타입(`ExpressionType`)은 ADR-0025 결정 1·2가
    정합니다. 값 문법과 `ExpressionType`의 대응은 ADR-0025 결정 1.2가 정합니다. `width`처럼
    `<length>`·`<percentage>`·keyword를 함께 받는 합집합 문법은 구성 타입별 `ExpressionType`의
    집합이 되고, 포트·binding 대상은 그 집합에 속한 타입의 식만 받습니다. 대응하는 구성 타입이 없는
    longhand(예: `transform` 행렬)는 포트·binding 대상이 아닙니다.
14. **longhand 하나는 PropertyKey 하나입니다.** 영속 record의 longhand마다 PropertyKey를 하나
    할당하고, longhand 주소는 key 하나로 끝나는 `PropertyPath`입니다. 경로의 두 번째 key(sub-key)로
    longhand를 가리키지 않습니다. 결정 4의 merge 단위, 결정 13의 포트·binding 대상, semantic diff의
    속성 단위가 모두 이 key입니다.
    - 합성 key(Frame `size` 0x0101, `transform` 0x0102, `border` 0x0105, `corner_radius` 0x0106,
      modifier `SHADOW_OFFSET` 0x1092)는 longhand별 새 key로 나눕니다. 기존 숫자는 재사용하지 않고
      **묶음 key**로 남깁니다. 묶음 key는 기존 `SetProperty` 직렬화와 history journal을 읽고 실행하기
      위해서만 registry에 있습니다. 실행 결과는 해당 longhand 전체를 한 번에 설정한 것과 같습니다.
      새 command, 인스펙터 묶음 편집, merge, 포트는 longhand key를 씁니다. `transform`·`border`는
      payload 의미가 바뀌므로 해당 schema migration(IX.11.9, IX.11.10) 이후 문서에서 묶음 key를 새
      command로 받지 않습니다.
    - Stellar 값 하나가 변형에 따라 서로 다른 CSS longhand 조합으로 나가는 key는 나누지 않고 Stellar
      고유 이름의 단일 속성으로 둡니다. Frame `clipping`(0x0103, Disabled·FrameBounds·Path)이 이
      경우입니다. Stellar clip은 하나라 `overflow-x`·`overflow-y`를 따로 바꿀 수 없고, FrameBounds는
      `overflow: clip` + `overflow-clip-margin`, Path는 `clip-path`로 나갑니다(`docs/design/property_model.md`
      5장). merge·semantic diff 단위는 key 하나이고, 값에 NodeRef가 있어 대응 `ExpressionType`이 없으므로
      포트·binding 대상이 아닙니다. registry 이름은 Stellar 고유 이름(`clipping`)이고 변형별 CSS 대응은
      매핑 metadata의 변환 규칙으로 기록합니다(IX.11.2).
    - payload bytes는 key를 담지 않으므로 key 분할만으로는 파일 bytes가 바뀌지 않습니다. 바뀌는 것은
      registry, command 경로, semantic snapshot 경로입니다.
    - 묶음 key를 가리키는 저장된 참조는 자동 변환하지 않습니다. 참조 값 타입(`TypedValue`·
      `ExpressionType`)에 합성 타입이 없어 타입이 맞는 값을 가질 수 없고, 값을 longhand로 나누는
      규칙을 추측하면 의미를 지어내게 되기 때문입니다. bytes를 그대로 두고 validator가 진단하며,
      사용자가 longhand 참조로 다시 연결합니다.
    - 근거: 지금 경로를 만드는 코드가 key 하나를 전제합니다. `SetProperty`는 key 하나와 값 하나이고
      (`crates/commands/src/property.rs`), 변경 경로는 `PropertyPath::with_keys(node, [key])`이며,
      semantic snapshot은 key 하나에 값 하나를 둡니다(`crates/semantic-diff/src/build.rs`). longhand
      key의 값은 `Length`·`Color`·`Enum` 같은 기존 값 타입 하나로 표현됩니다. key가 둘 이상인 경로는
      이미 component override 경로에서 nested 단계를 뜻하고 거부됩니다(`NestedOverridePath`,
      `crates/document/src/component/encapsulation.rs`). longhand sub-key를 섞으면 경로가 longhand인지
      nested 단계인지 registry 없이 판정할 수 없습니다. Frame 대역 `0x01xx`는 256자리 중 7개만
      씁니다.

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
- **합성 key 유지 + `PropertyPath` sub-key로 longhand 주소(예: `[CORNER_RADIUS, TOP_LEFT]`):** 기존
  key 숫자를 longhand 주소의 앞부분으로 계속 쓰고 새 key 할당이 줄어드는 장점이 있습니다. 하지만
  command, merge, 포트, binding, 인스펙터가 모두 두 단계 경로를 다뤄야 하고, 경로가 longhand인지
  nested override 단계인지 registry 없이 판정할 수 없어 탈락했습니다(결정 14).
- **migration에서 행렬을 `rotate`·`scale`로 분해해 저장:** 초월함수 결과가 플랫폼에 따라 달라
  결정론적 golden을 만들 수 없고, 다시 합성한 행렬이 원래 행렬과 bit 단위로 같지 않아
  탈락했습니다(결정 8).

## Consequences

- Web adapter(XXII.2)의 속성 변환은 대부분 항등 매핑이 됩니다. Apple·Android adapter도 같은 의미
  표를 입력으로 씁니다.
- 인스펙터 입력, 복사·붙여넣기, 노드 그래프 포트가 같은 CSS 값 문법을 공유합니다. CSS를 아는
  사용자는 그 지식을 그대로 씁니다.
- 레이아웃 엔진의 기본값·의미 migration과 X.11.1 적합성 corpus 기대값 갱신이 필요합니다.
- Frame 위치·transform 분리, Frame border longhand, Text style 표 저장 형식 추가는 schema
  migration이 필요합니다. 그 전에 제품 문서 migration registry를 project 열기에 연결해야 합니다
  (IX.11.21).
- 합성 key를 longhand key로 나눕니다(IX.11.22). 묶음 key를 가리키는 저장 참조는 진단 대상이 됩니다.
- export 정렬 중 IX.11.18(modifier stack 선형화 계획)과 IX.11.20(SVG 값 표기 정렬)은 내보내기(Phase
  XX)·코드 생성(Phase XXII)이 로드맵인데도 초기 릴리스 Phase IX에 둡니다. IX.11.18은 파일을 출력하지 않는
  core 계획 함수이며, 초기 capability matrix(IX.11.19)가 modifier 항목의 Web 상태를 정하는 근거입니다.
  IX.11.20은 이미 구현된 SVG export(XX.3.1)의 값 표기를 IX.11.13 formatter 한 곳으로 모으는 정리이고 SVG
  출력 bytes를 바꾸지 않습니다. 같은 값의 표기 규칙이 두 곳에 생기지 않게 formatter와 같은 Phase에서
  맞춥니다. 실제 HTML·CSS 출력은 로드맵 XXII.2가 합니다.
- 인스펙터 섹션을 CSS 범주로 재구성해야 합니다.
- CSS가 직접 표현하지 않는 디자인 도구 개념(채움, 내용 맞춤)은 preset 계층이 필요합니다.
- CSS 명세가 바뀌면 추적해야 합니다. 이 부담은 Web Target Profile의 capability 버전 고정(IX.10.4)으로
  제한합니다.

## Compatibility

- 이 ADR 자체는 파일 바이트, 공개 API, FFI를 바꾸지 않습니다.
- 기존 `PropertyKey` 숫자는 유지합니다. 새 longhand에는 새 key를 할당하고 기존 key를 재사용하지
  않습니다. 합성 key 다섯 개는 결정 14의 묶음 key로 남습니다.
- modifier key 대역에는 여유가 적습니다. 규칙 `0x1000 | (kind << 4) | field`에서 kind nibble 0(공통)과
  1\~12를 쓰고 있어 새 kind 자리는 13\~15 세 개뿐이고, kind마다 field는 1\~15입니다(field 0 예약,
  `crates/document/src/modifier_metadata.rs` 번호 규칙 test). 결정 14로 파라미터마다 field를 쓰고
  CSS 파라미터(spread·inset, stroke join·dash·miter, `backdrop-filter`)를 더하면 소진될 수 있습니다.
  소진되면 기존 번호를 옮기지 않고, node 종류 대역(`kind << 8`)과 semantic snapshot 전용 대역
  (`0xE0xx`\~`0xEFxx`)과 겹치지 않는 새 modifier 대역을 ADR로 예약합니다. sub-key 방식은 결정 14에서
  탈락했으므로 확장 방법으로 쓰지 않습니다.
- Frame payload(위치·transform 분리, border longhand)와 Text(style 표 저장 형식, Typography record)
  변경은 IX.11.21이 만드는 제품 문서 migration registry의 step과 golden fixture 갱신으로 진행합니다.
  schema version 증가를 공개 포맷(Phase XXVIII) 전에 한 번으로 묶을지는 IX.11.21이 정합니다. Text
  `letter-spacing` 이동은 style 표 저장 형식(IX.11.11)을 정하기 전의 타입 변경이라 파일 migration이
  아닙니다. IX.11.11이 저장하기 시작하는 `line-height`는 처음부터 CSS 의미로 정의하므로, text 엔진을
  그 의미에 맞추는 IX.11.12도 저장 bytes의 뜻을 바꾸지 않아 파일 migration이 아닙니다.
- 인스펙터 snapshot에 CSS 이름 문자열을 실어야 하면 FFI 표면이 바뀝니다. 그 변경은 해당 구현
  행에서 ADR-0014 절차로 다룹니다.
- 파일 포맷 공개 명세(Phase XXVIII)에는 CSS 매핑 표를 부록으로 포함합니다.

### Typography value v1 (IX.11.11)

`TypedValue::Typography`는 tag `0x0C`와 기존 외부 길이 envelope를 유지합니다. 내부 `STTY` + u8 version 1은
가족 이름의 순서와 선택적 고정 FontFaceRef, font-size, weight, style, stretch, line-height, letter-spacing을 기록합니다.
FontFaceRef는 X.8.1의 canonical bytes를 공유합니다. 이름마다 길이(u32 LE)와 UTF-8, face 유무(u8)를 기록하며,
빈 가족 목록과 medium은 design profile이 해석할 CSS initial입니다. medium의 기기별 px나 설치된 family를 저장 시 추정하지 않습니다.
크기는 1/64 logical px, 굵기·너비(%)·각도(deg)·배수·자간(px 또는 em)은 유한한 canonical f32입니다.
normal line-height와 inline box별 CSS half-leading, normal letter-spacing과 명시적 0을 구분합니다.
읽을 수 없는 버전·기존 blob은 원래 bytes를 갖고 `typography-unrecognized`를 반환합니다.

## Performance

- registry CSS metadata는 정적 표이므로 런타임 비용이 없습니다.
- CSS 텍스트 파싱은 입력 확정과 붙여넣기 때만 실행하고 인터랙션 경로(T0\~T5)에는 들어가지 않습니다.
- flex item 자동 최소 크기(`min-width: auto`)는 intrinsic 측정을 추가로 요구합니다. 별도 행
  (IX.11.23)으로 구현하고 증분 layout benchmark(X.7.4)로 방문 범위를 확인합니다.

## Security

- CSS 텍스트 파서는 신뢰할 수 없는 입력(클립보드, 외부 파일)을 받습니다. 입력 길이 상한, 재귀 없는
  고정 문법, 허용 목록에 있는 속성만 받기, `url()`과 `@import` 거부(외부 리소스 로드 없음)를
  지킵니다. 파서를 fuzz 대상(XVI.6)에 추가합니다.

## Migration

1. 합성 key를 longhand key로 나누고(IX.11.22) registry CSS metadata(IX.11.1\~IX.11.3)를 채웁니다.
   파일 변경은 없습니다.
2. 레이아웃 엔진 의미를 CSS에 맞춥니다(IX.11.4, IX.11.5, IX.11.23). 영속 layout record(IX.11.6)는
   IX.11.4·IX.11.5와 제품 문서 migration registry(IX.11.21) 뒤에 정의합니다. CSS 적합성 fixture(IX.11.8)는
   IX.11.4·IX.11.5·IX.11.23 뒤에 엔진 결과를 고정하며 IX.11.6의 선행이 아닙니다.
3. 제품 문서 migration registry(IX.11.21)를 project 열기에 연결한 뒤 Frame·Text schema migration
   (IX.11.9, IX.11.10, IX.11.11)을 그 step으로 구현합니다. 원본 generation을 보존하고, 자동 commit하지
   않으며, 외형 보존 fixture를 둡니다(아키텍처 §36).
4. 값 formatter·parser·복사·토큰 이름(IX.11.13\~IX.11.16)과 export 정렬(IX.11.18\~IX.11.20)은 2·3단계 행에
   의존하지 않고 1단계(와 같은 4단계 행)만 선행으로 둡니다. 인스펙터 재구성(IX.11.17)은 1\~3단계(sizing
   preset IX.11.7, Frame 위치·transform IX.11.9)와 parser(IX.11.14)를 선행으로 둡니다.
