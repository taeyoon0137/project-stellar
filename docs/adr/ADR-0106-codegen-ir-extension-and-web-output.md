# ADR-0106: 코드 생성 IR 확장과 Web 출력

## Status

Accepted (2026-11-23). ADR-0025 결정 4(실제 코드 변환)와 ADR-0049(codegen crate)가 구현 행에 미룬 IR 확장 형식,
profile kind별 lowering matrix, Web 출력 규칙, 생성 파일 쓰기 규칙을 고정합니다. 대상 행은 XXII.1.2\~XXII.1.7,
XXII.2.1, XXII.2.3, XXII.2.5, XXII.2.6, XXII.5.1, XXII.5.2입니다. Apple·Android adapter(XXII.3, XXII.4)와 문서에서
IR 입력을 채우는 연결(XXII.1.1 재개)은 범위 밖입니다.

## Context

- crate 그래프(ADR-0049, `docs/architecture/crate-dependency-graph.md`)상 `stellar-codegen`은 core-types,
  target-profile, accessibility, layout만 읽습니다. 문서 record, 상태 머신, `MotionSpec`, 토큰 평가기, 표현식 AST,
  CSS 이름 파생(`stellar_document::css::custom_property`), CSS 값 formatter(`stellar_document::css::value`)는 codegen이
  직접 부를 수 없습니다. `stellar-ops`는 document와 codegen을 함께 읽지만 expressions·bindings·prototype은 읽지
  못합니다.
- Context 해석기(XXX.7.4)와 ContextKey resource(XXX.7.1\~XXX.7.3)는 아직 없습니다. 토큰 저장(bindings)은 문서
  열기 경로에 연결되어 있지 않습니다.
- ADR-0025 결정 4.2는 속성 매핑을 capability registry 한 곳에서 관리하고, Web 항목은 IX.11.19 matrix를 그대로
  쓰며, 코드 생성은 Apple·Android lowering 규칙(XXII.1.4)과 Context·`MotionSpec` 항목(XXII.1.7)만 더한다고
  정했습니다.

## Decision

### 1. IR 입력은 지역 모델로 받고, 해석 결과를 그대로 보존합니다

IR 입력은 지금처럼 codegen 지역 타입입니다(ADR-0049 결정 6). 새 입력은 다음과 같습니다.

1. **CSS 기반 속성 사용**(`CssPropertyUse`): node마다 그 node가 쓰는 registry 항목(범위, 이름, CSS 매핑 상태,
   값이 있는 PropertyPath)입니다. 문서 registry(`css::registry`)를 읽는 상위(ops)가 채웁니다
   (`stellar_ops::codegen::css_property_use`).
2. **상태 머신**(`StateMachineDecl`): 소유자, 상태 변수, semantic action, transition(trigger, 조건식, 순서 있는
   action)입니다. 표현식은 닫힌 지역 트리 `IrExpr`(literal, 상태·토큰·속성·Component Property·환경·Context 참조,
   단항·이항 연산 기호, 함수 이름 호출, `if`, `match`)로 받습니다. 연산 기호와 함수 이름은 expressions crate가
   정한 `symbol()`·`name()` 문자열을 그대로 씁니다. 문서 AST(`ExpressionAst`)를 낮추려면 expressions 타입을 이름으로
   써야 하는데 ops는 expressions에 닿지 못합니다(crate 그래프). 그래서 이 낮춤은 문서 → IR 연결(XXII.1.1 재개)이
   expressions에 닿는 위치에서 합니다. `MotionSpec`은 문서 타입만 쓰므로 ops가 옮깁니다(`stellar_ops::codegen::motion`).
3. **`MotionSpec`**(`MotionDecl`): 대상 PropertyPath, 시작 조건, 지속 시간, 지연, timing function, reduced motion
   대체 동작을 문서 값 그대로 받습니다. f32는 bit 그대로 보존하고 snapshot에 최단 왕복 표기로 씁니다.
4. **Context 해석 결과**(`ContextInput`): key(이름, 값 종류, default 값 식), provision(node, key, 값 식, 펼친 트리
   위치별 provision 깊이 집합), 소비(node, 속성 경로, key, 출처 = provision node 또는 key default, 건넌 slot
   경계), slot host(host node, instance node, 공개 key 목록)입니다. 해석은 XXX.7.4 해석기가 하고 IR은 결과만
   보존합니다. 생성 코드는 다시 해석하지 않습니다. 해석기가 생기기 전에는 합성 입력으로만 검증합니다.
5. **토큰 내보내기 정보**(`TokenDecl`의 추가 필드): 기본 환경 평가 literal, alias 대상 토큰, media 조건부 값,
   `interaction_state` 조건부 값입니다. 토큰 평가기(bindings)가 환경을 바꿔 평가한 결과를 넣습니다.

IR 검사는 참조 무결성(선언된 상태·토큰·key, View 안 node)만 합니다. 의미 검사(상태 머신 진단, Context
validator)는 각 원본 모듈이 이미 합니다.

### 2. node별 mapping 진단(XXII.1.2)

`IrNode::mappings`는 node가 출력에 쓰는 모든 대상의 상태를 Native 포함 전부 담습니다. 출처는 둘입니다.

- IX.10.8 validator 진단(element·contract·capability 속성). `IrNode::capabilities`를 그대로 옮깁니다.
- CSS 기반 속성 사용(결정 1.1)을 결정 3의 matrix로 target profile kind에 대해 판정한 값.

순서는 (출처, subject, path) 오름차순이고 같은 입력은 같은 목록입니다. snapshot에는 `mapping` 줄로 씁니다.

### 3. 코드 생성 lowering matrix(XXII.1.4, XXII.1.7)

`stellar_codegen::matrix`가 profile kind별 상태와 lowering 규칙을 한 곳에 둡니다. IR lowering과 adapter는 이 matrix만
참조합니다.

- **Web**: CSS 기반 속성은 `stellar_target_profile::css_matrix::web_capability` 결과를 그대로 씁니다(다시 정의하지
  않음). 규칙은 `same-name-css`이고 진단 없음(Native) 또는 matrix 근거 진단(Approximated·Unsupported)입니다.
- **Apple·Android**: registry 항목마다 상태와 규칙(`RuleId`)을 명시 표로 둡니다. 표에 없는 항목은 허용하지 않고,
  ops test가 문서 registry 전체·layout longhand 전체에 대해 두 profile 모두 항목이 있는지 검사합니다. 1:1이 아닌
  규칙(modifier 적용 순서, 블렌드 합성 색공간, 자유 배치·constraint, 기본값 차이, path 굽기, 래스터화, 구조 변환)은
  규칙마다 진단 code를 함께 둡니다. 규칙은 입력 값 텍스트를 받아 대상 API 텍스트와 진단을 돌려주는 순수 함수이고,
  규칙마다 입력·출력·진단 fixture가 있습니다.
  - modifier 적용 순서: SwiftUI view modifier와 Compose `Modifier` chain은 선언 순서대로 감싸므로 stack 순서를
    그대로 지킵니다(`modifier-chain`, Native). Web의 고정 순서 선형화는 IX.11.18 계획을 그대로 씁니다.
  - 블렌드·겹친 opacity·blur·shadow: 플랫폼 합성은 sRGB encoded 공간이라 Approximated(`srgb-composite`).
  - 자유 배치(`position: absolute`·inset): SwiftUI `.offset`/`.position`, Compose `Modifier.offset`로 낮추고
    Approximated(`absolute-offset`, 흐름에서 빠지는 의미가 다름).
  - 기본값 차이: CSS 초기값과 플랫폼 기본값이 다른 항목(stack spacing, alignment 등)은 항상 명시 값을 냅니다
    (`explicit-default`, Native).
- **Context·`MotionSpec` 항목**(XXII.1.7): 닫힌 목록 `MotionFeature`(timing linear·cubic-bezier·steps·spring,
  reduced motion instant·play, 시작 조건 becomes·changes, 지연, Color 보간)와 `ContextFeature`(스타일 값 전달,
  비스타일 값 전달, 같은 key 재읽기, slot 재공급)를 Web·Apple·Android마다 상태·규칙·진단으로 둡니다. CSS 기반
  속성 항목은 바꾸지 않습니다(golden으로 불변 검사).
  - Web spring은 Approximated입니다. 생성 CSS는 `linear()` 표본 곡선이고, 표본은 호출자가 넘기는 `EasingSampler`가
    엔진 evaluator(`stellar_prototype::timing::eased_progress`)로 계산합니다. sampler가 없으면 Unsupported 진단을
    내고 그 motion은 즉시 값으로 바뀝니다. codegen은 spring을 다시 적분하지 않습니다.
  - Web Color 보간은 `srgb`·`display-p3` encoded premultiplied 보간이고 CSS transition은 보간 공간을 지정할 수 없어
    Approximated입니다(ADR-0025 결정 1.2).

### 4. HTML 구조(XXII.2.1)

`stellar_codegen::web::structure`가 IR node 하나에 element 하나(Fragment는 없음, 자식은 가장 가까운 element
조상에 붙음)를 정합니다. element 종류와 Control contract로 tag를 고릅니다.

| 입력 | element |
|---|---|
| Frame | `div` |
| Fragment | 없음 |
| Text | `span`(텍스트 내용) |
| Image | 자식이 없으면 `img`, 있으면 `div` |
| Shape·Path·ComponentInstance | `div` |
| Button contract | `button type="button"` |
| TextInput contract | 자식·텍스트가 없으면 `input type="text"`, 아니면 `div` |
| Modal contract | `dialog` |
| List contract | `ul`, 그 element 자식은 `li` |
| SingleSelectionGroup contract | `div` |

contract가 둘 이상이면 contract 번호 오름차순 첫 contract가 tag를 정합니다. 접근성 mapper(XXII.2.4)는 이 구조
위에서 접근성 entry가 있는 element만 role에 맞게 고칩니다. 접근성 entry가 없는 element의 tag는 구조 결과를
그대로 씁니다(같은 판정을 두 곳에 두지 않음).

### 5. 토큰 CSS(XXII.2.3)

- 이름은 문서 `derive_names`(IX.11.16), 값 텍스트는 문서 `literal_text`, `:root` 블록은 문서 `root_block`으로
  만듭니다. 조립은 `stellar_ops::export_web_tokens`가 합니다.
- `@property`: ops는 expressions의 `ExpressionType`을 이름으로 쓸 수 없어(crate 그래프) 문서 `property_rule`을 부를
  수 없습니다. 등록 문법은 literal 값 variant로 정하고(등록 타입은 literal variant와 선언 타입이 같아야 하므로 같은
  결과), 문서 함수와 문법 표·규칙 template이 같은지 ops test가 문서 소스와 대조합니다.
- media 조건부 값: `color_scheme`·`contrast`·`reduced_motion` 조건 조합마다 `:root`를 감싼 `@media` 블록을 냅니다.
  key마다 기본이 아닌 값이 하나뿐이라 조합은 key 부분집합입니다. 조건에 나온 key 집합의 부분집합만 보고(평가기는 식이
  읽는 key만 바꿔 평가하므로 그 밖의 key는 값을 바꾸지 않음), 값이 없는 부분집합은 기본 값입니다. 블록 순서는 조건 수
  오름차순, 같은 수면 key 순서이고, 그때까지의 cascade 결과와 같은 블록은 내지 않습니다.
  `viewport_*`·`density`·`locale` 조건은 지금 표현하지 않고 `token-environment-unsupported` 진단을 냅니다.
- CSS 값 텍스트가 없는 토큰(문서 `literal_text`가 `None`: String·Gradient·Shadow·Typography·Boolean·Enum·
  ResourceRef)은 생성 상수(`tokens.js`)입니다. 문서 formatter가 이 타입들의 CSS 텍스트를 내지 않으므로 등록하지 않은
  custom property로는 내지 않습니다. 상수는 기본 환경 값만 내고, 조건부 값이 있으면 진단합니다.
- `interaction_state`에 따라 값이 달라지는 토큰은 `:root`·`@media` 선언에서 빼고 결정 7이 낸다.

### 6. Context의 Web 출력(XXII.2.5)

- CSS 값 텍스트가 있는 key 값은 provision element 규칙의 custom property 선언으로 내고, 상속이 하위 트리에
  전달합니다. 등록 정책은 토큰과 같습니다(`inherits: true`, 결정 5의 함수).
- 같은 key를 다시 읽는 provision은 깊이 이름(`<key 이름>-<d>`, d = 자기 포함 조상 provision 수, default는 0)에 값을
  쓰고 key 이름으로 다시 공급합니다. 값 식 안의 같은 key 읽기는 `<key 이름>-<d-1>`입니다. default 값은 root
  element에서 깊이 0 이름과 key 이름에 씁니다. 깊이 이름과 slot 보존 이름(`<key 이름>-outer`)은 토큰·key 이름과 한
  이름 공간에서 충돌을 검사하고 `name-collision` 진단을 냅니다.
- 한 provision의 깊이 집합 원소가 둘 이상이면 그 key 전체를 생성 코드 context 구조(`context.js`)로 내고
  `context-depth-varies` 정보 진단을 냅니다. CSS로 낼 수 없는 값 식(조건식·환경 참조 등)의 key도 같은 구조로 냅니다.
- slot host: Slot이 공개하지 않은 key 중 instance와 host 사이에 provision이 있는 key는 instance element에서
  `<key 이름>-outer: var(<key 이름>)`으로 보존하고 slot host element에서 `<key 이름>: var(<key 이름>-outer)`로 다시
  공급합니다.
- `context.js`는 provision별 값 식과 소비 지점별 출처(IR이 보존한 결과)를 정적 표로 내며, 실행 시 해석하지
  않습니다.

### 7. 상태 머신·motion·상호작용 상태의 Web 출력(XXII.2.6)

- `MotionSpec`은 대상 element 규칙의 `transition-*` longhand로 냅니다. `Becomes` 시작 조건은
  `[data-s-<상태 hex>="<값>"]` 조상 selector 아래 규칙, `Changes`는 element 규칙입니다(CSS transition은 바뀐 뒤
  style의 선언을 쓰므로 그 상태로 들어갈 때만 재생). reduced motion `instant`는
  `@media (prefers-reduced-motion: reduce)` 블록에서 `transition-property: none`, `play`는 그대로입니다.
- 상태 전환은 `state.js`가 냅니다. transition trigger 이벤트를 DOM 이벤트(`click`·`input`·`focus`·`blur`·`keydown`·
  `change`·`open`·`close`)로 등록하고, 조건식과 action을 `IrExpr`에서 낮춘 JavaScript로 평가하며, 상태 값을 root
  element `data-s-*` 속성에 씁니다. semantic action emit은 `CustomEvent`입니다.
- `interaction_state` 조건부 토큰 값은 그 토큰을 쓰는 Control element의 상태 selector에 해당 속성 선언으로 냅니다.
  대응은 Control contract 상태로 정합니다: Button은 hover·pressed·focused·disabled, TextInput은 hover·focused·
  disabled, 그 밖의 contract는 hover·focused입니다. selector는 hover → `:hover`, focused → `:focus-visible`,
  pressed → `:active`, disabled → `:disabled`입니다. contract가 지원하지 않는 상태 값은
  `interaction-state-unsupported` 진단을 냅니다.

### 8. 생성 파일 순서·식별자·formatting(XXII.5.1)

- Web 출력 파일 집합은 경로 byte 오름차순 목록(`GeneratedFiles`)입니다: `context.js`, `index.html`, `state.js`,
  `styles.css`, `tokens.css`, `tokens.js`. 빈 파일은 만들지 않습니다.
- 모든 파일은 UTF-8, LF, 두 칸 들여쓰기, 끝 줄바꿈 하나입니다. 생성 파일 머리에 생성 marker 주석을 씁니다.
- 식별자: HTML id는 `s-<StableId 32자리 hex>`, CSS 이름은 IX.11.16 escape, JavaScript 식별자는 ASCII 문자·숫자·`_`만
  남기고 나머지를 `_`로 바꾼 뒤 숫자 시작이나 예약어면 `_`를 앞에 붙이며, 겹치면 `_<StableId hex>`를 붙입니다.
  JavaScript 문자열은 JSON 문자열 escape입니다.

### 9. 생성 파일 쓰기(XXII.5.2)

`stellar_ops::export_web_write`가 두 방식만 허용합니다.

1. **디렉터리 교체**: 출력 디렉터리가 없거나 비었거나 생성 manifest(`.stellar-generated`)가 있고 그 목록 밖
   파일이 없을 때만 씁니다. 같은 부모 아래 임시 디렉터리에 전부 쓰고 fsync한 뒤 rename으로 교체합니다. 목록 밖
   사용자 파일이 있으면 아무것도 바꾸지 않고 거부합니다.
2. **marker 구간 갱신**: 이미 있는 파일 안의 `stellar:generated:begin <이름>`과 `stellar:generated:end <이름>`
   주석 사이만 바꿉니다. marker가 없거나 짝이 맞지 않거나 겹치면 거부합니다. 파일은 임시 파일에 쓴 뒤 rename으로
   교체합니다.

생성 코드를 다시 읽어 IR이나 문서로 가져오는 API는 없습니다(XXII.1.3). marker 구간을 찾는 일은 사용자 파일을
보존하기 위한 위치 판정이며 생성 코드의 의미를 해석하지 않습니다.

## Alternatives

- **codegen이 document·expressions에 의존**: CSS 이름·값 formatter를 직접 부를 수 있지만 ADR-0049의 단방향 경계와
  crate 그래프 표를 바꿔야 합니다. 이 범위에서는 ops가 문서 함수로 조립하는 쪽을 택했습니다.
- **Apple·Android 상태를 CSS 매핑 상태에서 유도**: ADR-0025 결정 1.3이 금지합니다.
- **spring을 codegen이 직접 표본화**: 엔진 evaluator와 다른 두 번째 적분기가 생겨 결과가 갈라집니다.
- **Context를 모두 생성 코드로**: CSS 상속으로 낼 수 있는 스타일 값까지 script가 필요해져 ADR-0025 결정 4.3과
  다릅니다.
- **출력 디렉터리 덮어쓰기**: 사용자 파일을 잃을 수 있어 거부했습니다.

## Consequences

- IR이 상태 머신·motion·Context·토큰 내보내기 정보를 보존하므로 adapter는 문서를 보지 않고 출력합니다.
- ops는 layout 타입을 이름으로 쓸 수 없어 `build_ir`를 부를 수 없습니다. adapter 입력용 `IrTree::from_nodes`(preorder
  node 목록 검사, 접근성 entry 없음)로 ops test가 IR을 만듭니다. 전체 IR은 계속 `build_ir`만 만듭니다.
- `state.js`의 `read`는 상태 값만 제공합니다. 토큰·속성·환경·Context를 읽는 transition은 진단하고 뺍니다.
- 문서 → IR 입력 연결(XXII.1.1 재개), Context 해석기(XXX.7.4), 토큰 저장 연결이 생기면 같은 입력 타입을 채우면
  됩니다.
- `@property` 문법 표는 ops에 한 번 더 있고 소스 대조 test로만 같음을 지킵니다. ops가 expressions 타입을 쓸 수
  있게 되면 문서 `property_rule` 호출로 바꿉니다.
- 상태에 따라 바뀌는 속성 binding의 Web 출력은 아직 없어 transition은 선언만 되고 값 변화는 다른 경로가 만듭니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. 보호 경로를 바꾸지 않습니다.
- codegen 공개 API에 입력 필드가 늘어납니다(`CodegenSources`의 `css_properties`·`machines`·`motions`·`contexts`,
  `TokenDecl`의 `alias`·`variants`). IR semantic snapshot 머리는 `codegen-ir v3`입니다.
- 접근성 mapper(XXII.2.4)는 구조(XXII.2.1) 위에서 동작합니다. 기존 golden HTML은 바뀌지 않았습니다.

## Performance

- IR 구성과 출력은 node·입력 수에 선형입니다. media 조합은 조건 key 3개(최대 2×3×2 조합)로 제한됩니다.
- 앱 상호작용 경로에 들어가지 않습니다.

## Security

- 생성 JavaScript는 IR의 닫힌 표현식만 낮추고 문자열은 JSON escape합니다. 문서에 실행 코드를 넣지 않는 원칙은
  그대로입니다.
- 파일 쓰기는 출력 디렉터리 안 상대 경로만 받고 `..`·절대 경로·symlink 대상을 거부합니다.

## Migration

- 기존 IR snapshot golden을 v3로 다시 만듭니다. 저장 데이터 migration은 없습니다.
