# ADR-0075: CSS longhand 속성 registry와 값 문법

## Status

Accepted (2026-11-23)

## Context

- ADR-0024는 디자인 속성의 이름·값 문법·단위·레이아웃 의미를 CSS longhand로 정하고(결정 1), 영속 record는
  longhand만 저장하며(결정 4), longhand 하나를 `PropertyKey` 하나로 둔다고 정했습니다(결정 14). ADR-0074는
  Auto Layout·Fixed/Fill/Hug 같은 비CSS 개념을 두지 않는다고 정했습니다.
- 실제 코드 대응(Web export·codegen), 텍스트 원본 키, CLI·MCP payload가 모두 같은 속성 이름을 써야 합니다.
  그 이름과 대응 규칙이 registry 한 곳에 있어야 다른 lane(text-source-codec, codegen-css, agent-protocol-mcp,
  context-motion-core)이 같은 이름을 읽습니다.
- 지금 코드는 일부만 맞습니다. Frame `width`·`height`·`border-*-radius` key와 Shape·Image longhand key는 이미
  있지만, Shadow offset(0x1092)은 x·y를 한 key에 담고, registry 항목에 CSS 매핑 metadata가 없고, 값
  parser·formatter는 속성 패널 전용(`crates/commands/src/css_value.rs`)이며, 선언 블록 복사·붙여넣기와 토큰
  custom property 이름 파생이 없습니다.
- 계획 행: IX.11.22, IX.11.1, IX.11.2, IX.11.3, IX.11.13, IX.11.14, IX.11.16, IX.11.15, IX.12.1.
- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만들고 셸 FFI를 늘리지 않습니다. 기능은 Rust 테스트·CLI·MCP로
  검증합니다.

## Decision

### 1. 합성 key 분할(IX.11.22)

- Frame `size`(0x0101)는 `width`(0x0108)·`height`(0x0109)로, Frame `corner_radius`(0x0106)는
  `border-top-left-radius`(0x010a)·`border-top-right-radius`(0x010b)·`border-bottom-right-radius`(0x010c)·
  `border-bottom-left-radius`(0x010d)로 나뉘어 있습니다(이미 있던 key를 그대로 확정합니다).
- Shadow offset(0x1092)은 같은 modifier 대역의 빈 번호 `shadow-offset-x`(0x1094)·`shadow-offset-y`(0x1095)로
  나눕니다. 값 타입은 부호 있는 길이(`ModifierPropertyType::Coordinate`, command 값 `Number`)입니다.
- 옛 key는 재사용하지 않는 **묶음 key**로 registry에 남깁니다. registry 항목의 `longhands` 필드가 비어 있지
  않으면 묶음 key입니다. 묶음 key는 이전 `SetProperty`·`SetModifierProperty` bytes와 history journal을 그대로
  decode·실행하는 데만 쓰고, 결과는 longhand 전체를 설정한 것과 같습니다. 새 편집 목록
  (`node_property_keys`, `properties_of`)은 묶음 key를 내지 않습니다.
- `CommandKind`와 command codec은 바꾸지 않습니다. 따라서 저장된 command bytes의 의미가 바뀌지 않습니다.
- 저장된 참조(ValueBinding target, View Variant·Component variant override property patch)가 묶음 key를
  가리키면 변환하지 않고 `bundle-key-reference` 진단을 냅니다(`stellar_document::css::bundle`). 그 참조가
  longhand 하나를 뜻하는지 전체를 뜻하는지 bytes만으로 정할 수 없기 때문입니다.
- `transform`(0x0102)·`border`(0x0105) 분할은 IX.11.9·IX.11.10이 같은 규칙으로 합니다.

### 2. 단위 계약(IX.11.1)

- `Length`=`px`, `Angle`=`deg`, `Duration`=`ms`, `Percentage`=`%`(저장 50 = 50%)입니다. `core-types` 문서
  주석과 ADR-0008 개정에 고정합니다. 저장 bytes는 바뀌지 않습니다(지금 저장 값이 이미 이 단위입니다).

### 3. `clipping`

- `clipping`(0x0103)은 CSS longhand로 나누지 않는 Stellar 고유 이름의 단일 속성입니다. 파일·MCP 이름은
  `clipping`을 유지합니다. CSS 대응은 변형별 변환 규칙으로 기록합니다: Disabled → `overflow: visible`,
  FrameBounds → `overflow: clip` + `overflow-clip-margin: border-box`, Path → `clip-path`. 매핑 상태는
  `converted`이고 `initial-differs`입니다(기본값 FrameBounds, CSS initial `overflow: visible`).

### 4. 매핑 metadata(IX.11.1·IX.11.2·IX.11.3)

- 매핑 상태 어휘는 `exact`·`converted`·`approximated`·`stellar-only` 네 개입니다. 추가 표시는
  `initial-differs`·`subset`입니다. 각 registry 항목(Frame·Shape·Path·Text·Image·Modifier·공통 node 필드)은
  `CssMapping`(CSS longhand 이름 목록, 값 문법, 상태, 표시, 차이 설명, 변형별 변환 규칙)을 가집니다.
- registry `name`은 CSS longhand 이름이거나 CSS 대응이 없을 때의 Stellar 이름입니다. 묶음 key의 표시 문자열은
  대응 longhand 목록입니다.
- Mask(0x1051)는 alpha·luminance mode가 정해지기 전(속성 모델 15장 O14) `approximated`입니다.
- CSS longhand 이름 inventory 상수를 두고, 매핑이 쓰는 이름은 모두 inventory에 있어야 합니다.

### 5. 값 formatter·parser(IX.11.13·IX.11.14)

- 값 문법·parser·formatter는 `crates/document/src/css/value.rs`로 옮겨 registry와 같은 crate에 둡니다.
  속성 패널(`crates/commands/src/css_value.rs`)은 같은 구현을 다시 내보냅니다(이중 구현 없음).
- 정규형: 수는 f32를 왕복하는 가장 짧은 10진(지수 없음, `-0`은 `0`), 단위는 2절, keyword는 소문자, sRGB 범위 안
  8비트 정확 색은 `#rrggbb[aa]`, 그 밖은 `color(srgb …)`·`color(display-p3 …)`입니다. 토큰 참조는
  `var(--name)`으로 씁니다.
- parser는 속성 모델 2장 입력 형태만 받고 그 밖은 `unsupported-unit`·`unsupported-value`·`unknown-token`·
  `forbidden-value`(형식이 틀리면 `invalid-value`)로 구조화해 거부합니다. 입력 길이 상한 256 byte, 재귀 없는
  문법입니다. shorthand(속성 모델 11장)는 longhand 선언 목록으로 펼칩니다.

### 6. 선언 블록 복사·붙여넣기(IX.11.15)

- 복사는 선택 node의 속성을 속성 패널 표시 순서(registry 이름)의 `property: value;` 줄로 냅니다. 명시 값만 내며,
  Fill 없이 문서 기본 글자색을 물려받은 Text `color`는 넣지 않습니다(ADR-0082 결정 5: 명시 값은 기본값과 같아도
  명시로 남고, 물려받은 값은 선언이 아님). 지금 복사·붙여넣기 범위는 속성 패널 행이 있는 속성입니다. registry가
  저장하지만 행이 없는 속성(`opacity`·`mix-blend-mode`·`transform`·`stroke-linecap`·`image-rendering`·shadow·blur)은
  복사에 나오지 않고, 붙여넣으면 `unsupported-property`로 건너뜁니다. 붙여넣기는 선언 블록을 해석해
  shorthand를 펼치고, 받을 수 있는 선언을 command로 바꿔 **CommandTransaction 하나**로 적용합니다. 알 수 없는
  속성과 받을 수 없는 값은 이유 코드와 함께 건너뛴 목록으로 돌려줍니다(부분 실패 없음, 조용히 버리지 않음).
- 건너뜀 이유 코드는 값 오류 코드(5절)와 `invalid-declaration`, `unknown-property`(저장 위치 없음),
  `unsupported-property`(registry에 저장 위치가 있지만 붙여넣기 경로가 아직 없음), `not-applicable`(선택 node 종류에 없음, 레이어 속성), `rejected`(잠금 등 편집 규칙)입니다. 복사는 속성 패널과 같은
  이름·정규 값이고 레이어 속성(`name`·`visible`·`locked`)은 넣지 않으며, Stellar 전용 속성은 표시를 달아 함께
  냅니다. `var()`는 값 binding 저장 경로가 생기기 전까지 건너뜁니다.
- shorthand는 펼친 longhand가 모두 저장 위치를 가지면 longhand로(`border-radius`), 아니면 같은 이름의 묶음 행으로
  (`border`·`overflow`, Stellar가 아직 한 값으로 저장) 적용합니다. 선언은 문서 사본에서 순서대로 적용해 뒤 선언이
  이기고, 결과 command는 한 transaction입니다. 값이 지금 값과 같은 `SetProperty`·`SetModifierProperty`는 만들지 않습니다
  (같은 블록을 다시 붙이면 command 0).
- 클립보드 연결은 셸이 맡습니다. CLI `stellar css copy|paste`가 같은 core API를 씁니다(`stellar_ops::css`).

### 7. CSS custom property 이름(IX.11.16)

- 속성 모델 10장 규칙을 이름·종류·StableId만 받는 순수 함수로 구현합니다. 등록 속성은 모두
  `inherits: true`이고 `initial-value`는 기본 환경에서 평가한 literal, alias는 `:root`의 `var()` 선언으로만
  냅니다.

### 8. Component Context 번호 예약(IX.12.1)

- 예약 번호는 `stellar_core_types::reserved::context`에 상수로 고정하고 다른 용도로 쓰지 않습니다. encoder·decoder는
  바꾸지 않습니다. 구현은 Context lane(로드맵 XXX.7)이 같은 번호로 합니다.

| 대상 | 예약 | 근거 |
|---|---|---|
| `ReferenceTarget` Context tag | `0x05` | 기존 tag 0x01\~0x04(`crates/expressions/src/ast.rs`) 다음 |
| `ContextKey`·`ContextProvision` chunk kind | required kind 7(`0x8000_0007`), 이름 `context-set` | 기존 kind 1\~6(`crates/storage/src/chunk/kinds.rs`) 다음. Context key는 문서 구조 해석에 필요하므로 required |
| context-set chunk 안 record kind | `ContextKey` = `0x01`, `ContextProvision` = `0x02` | 공급은 Frame StableId로 가리키는 record라 node payload를 늘리지 않습니다 |
| ComponentDefinition 공개 인터페이스 확장 | 정의 인코딩 끝(local token scope 뒤) tag 있는 확장 section, `ContextRequirement` 목록 tag `0x01` | 지금 인코딩에는 확장 section이 없어 bytes가 바뀌지 않습니다 |
| Slot 공개 Context key 목록 | Slot 정의 확장 section tag `0x01` | ADR-0025 결정 3.11 |
| semantic snapshot key | `0xE705`(컴포넌트 requirement), `0xE716`(Slot Context key), 대역 `0xECxx`(ContextKey·Provision 객체) | `crates/semantic-diff/src/keys.rs`의 빈 번호·빈 대역 |

- chunk kind를 새로 두는 쪽을 골랐습니다. 기존 component-library chunk 안 record로 넣으면 컴포넌트가 없는 문서의
  Context key 정의가 갈 곳이 없기 때문입니다.

### 9. 셸 경계

- FFI(`crates/ffi`, `stellar_ffi.h`)는 추가하지 않습니다. UI 재설계 전까지 셸 경계를 동결합니다.

## Alternatives

- 묶음 key를 지우고 journal을 migration: 이미 저장된 history·journal bytes를 다시 써야 하고 되돌릴 수 없어
  탈락했습니다.
- 묶음 key 참조를 longhand 전체로 자동 변환: 참조가 축 하나만 뜻했을 수 있어 조용한 의미 변경이 됩니다.
- `clipping`을 `overflow-x`·`overflow-y`·`clip-path` longhand로 분할: Stellar clip은 하나이고 세 변형이 서로
  다른 longhand 조합으로 나가므로 분할하면 한 노드에 모순 조합이 생깁니다.
- parser를 commands에 두고 document는 metadata만: codegen·text-source·MCP가 commands에 의존해야 해서 탈락.

## Consequences

- 모든 registry 항목이 CSS 이름 또는 stellar-only 이름과 매핑 상태를 가지며, 누락은 test가 막습니다.
- 다른 lane은 registry API(`stellar_document::css`)에서 이름·문법을 읽습니다.
- semantic snapshot(`crates/semantic-diff/src/build.rs`·`records.rs`)의 Shadow offset·Shape·Image 다값 key
  교체는 그 파일을 소유한 semantic-merge lane이 합니다. 이 ADR의 key는 그대로 씁니다.

## Compatibility

- 저장 bytes(payload·chunk·text 원본)는 바뀌지 않습니다. 이전 command bytes·journal은 그대로 decode·실행됩니다.
- 공개 API: `FramePropertyMetadata`·`ModifierPropertyMetadata`에 `longhands`·`css` 필드가 생기고, modifier
  registry 이름이 CSS 이름으로 바뀝니다. 이름은 식별자가 아니므로(key가 식별자) 저장 형식에 영향이 없습니다.
- FFI 변경 없음.

## Performance

- registry 조회는 상수 배열 선형 탐색이며 항목 수는 수십 개입니다. 선언 블록 parser는 입력 길이에 선형이고
  선언 수에 상한이 있습니다.

## Security

- parser는 외부 리소스(`url()`, `@import`, `image()`)를 `forbidden-value`로 거부합니다. 입력 길이 상한과 재귀
  없는 문법으로 악성 입력의 비용을 제한합니다.

## Migration

- 파일 migration은 없습니다. 묶음 key 참조가 있는 문서는 진단을 받고 사용자가 longhand로 다시 지정합니다.

### 결정 1 보완: snapshot 투영과 묶음 key 참조 검사(IX.11.22·IX.11.3)

- semantic snapshot 투영(`crates/semantic-diff/src/build.rs`)과 역투영(`records.rs`)은 값 여러 개를 한 경로에 담지
  않습니다. Rectangle 크기는 `width`·`height`(0x0301·0x0302), 모서리는 `border-*-radius` 네 longhand(0x0303\~0x0306),
  Line 끝점은 `x1`·`y1`·`x2`·`y2`(0x0307\~0x030a), Image 크기는 `width`·`height`(0x0602·0x0603), Ellipse·Polygon bounds
  크기는 `width`·`height`, Shadow offset은 `shadow-offset-x`·`shadow-offset-y`(0x1094·0x1095)입니다. Ellipse·Polygon
  원점 `(x, y)`만 registry key가 없어 snapshot 전용 경로(`ELLIPSE_BOUNDS`·`POLYGON_BOUNDS` 번호에 원점 두 값)로
  남습니다. snapshot 전용 key `RECT_SIZE`·`RECT_CORNER_RADIUS`·`LINE_START`·`LINE_END`·`IMAGE_SIZE`는 더 투영하지
  않고 번호를 재사용하지 않습니다. snapshot은 저장하지 않으므로 파일 migration은 없습니다.
- 그래서 같은 Frame·Rectangle의 서로 다른 모서리, Shadow offset의 x와 y를 양쪽에서 바꾸면 3-way merge가 conflict 없이
  합칩니다(`crates/semantic-merge/tests/longhand_merge.rs`, 컴포넌트 원본 파일 병합 test). 같은 longhand를 다르게
  바꾸면 그 경로 하나만 conflict입니다.
- 묶음 key 참조 진단(`bundle_key_reference`)은 `VariantOverrideSet::validate_references`의 property patch 검사가
  부릅니다(`VariantOverrideError::BundleKeyPatch`). View Variant와 Component variant override는 같은 patch 묶음을
  쓰므로 이 한 곳이 둘 다 덮습니다. binding target 검사는 bindings가 crate graph상 문서 crate에 의존하지 않으므로
  `BindingContext::targets`(`TargetCheck`)로 호출자가 넘기고, `check_binding`이 type·cycle보다 먼저 부릅니다
  (`BindingCommandError::Target`). 문서와 bindings를 함께 보는 제품 호출자는 아직 없어, 그 호출자를 만들 때 이
  검사에 `bundle_key_reference`를 넘깁니다.

### 결정 4 보완: CLI·MCP 속성 경로 표기와 범위 있는 이름 조회

- headless 출력의 속성 경로(`stellar_ops::query::property_path_label`: `query node`의 `properties[].path`, diff·apply·
  semantic-merge·git 변경 항목의 `path`, MCP diff)는 key마다 registry 이름(`css::registry::name_of`)을 씁니다. 예:
  `width`, `border-top-left-radius`, `border[1]`. registry에 없는 key(semantic snapshot 전용 `0xE0xx`, 인스턴스
  override 등)는 4자리 16진수입니다. key 번호는 범위와 무관하게 유일하므로(대역 규칙) 이름 표기는 key 하나로 정해집니다.
  numeric 표기는 보조 필드 `key`(`property_key_label`, 예: `0x0108`)로 함께 냅니다(`query node` 속성, diff 변경 항목).
  conflict 목록은 `path`(이름)만 냅니다. 대상 객체가 같이 나오므로 이름에서 key가 정해집니다.
- 입력은 이름과 숫자를 모두 받습니다. `stellar apply`의 `set-property`(`node`, `property`, `value`)는 registry 이름 또는
  `0x` key를 받아 이름으로 풀고, 묶음 key는 `bundle-key-reference`로 거부합니다. command는 결정 6의 선언 붙여넣기
  (`paste_declarations`)로 만들고, 앞 command까지 적용한 사본 문서 위에서 계산합니다. 그래서 속성 패널·`stellar css`·
  `apply`가 같은 parser·적용 규칙을 씁니다. 값에 `;`·`{`·`}`가 있으면(선언 둘 이상) 거부합니다.
  numeric key로 받으면 이름으로 풀기 전에 그 key가 대상 node에서 그 이름이 가리키는 key인지 확인합니다(node payload
  범위 → 공통 node 범위 → modifier 범위 순, 붙여넣기 경로와 같은 순서). 이름은 범위 안에서만 유일하므로 Frame node에
  Shape `width`(`0x0301`)나 Transform modifier `transform`(`0x1011`)을 보내면 같은 이름의 다른 속성(`0x0108`,
  `0x0102`)으로 바꿔 쓰지 않고 `not-applicable`로 거부합니다. 값이 이미 같아 command가 없을 때도 같습니다.
- 이름은 범위 안에서만 유일합니다(`transform`은 Frame 0x0102와 Transform modifier 0x1011). 이름으로 key를 찾는 경로는
  `css::registry::key_of(scope, name)`·`entry_by_name`처럼 범위를 주고, 범위 없는 `entries_named`는 모든 후보를 돌려
  줍니다. `PropertyNameRegistry`(III.2.1)는 범위마다 하나씩 만듭니다(`css::registry::name_registry(scope)`). 여러 범위를
  한 registry에 넣으면 역조회가 마지막 등록으로 덮이므로 그렇게 쓰지 않습니다.

### 결정 6 보완: 속성 패널 행이 없는 registry 속성의 복사·붙여넣기(IX.11.15)

- 선언 블록 복사·붙여넣기는 속성 패널 행 밖의 registry 속성도 다룹니다. 대상과 저장 위치:
  - `transform`: 선형 부분만 `matrix(a, b, c, d, 0, 0)`(단위 행렬은 `none`)입니다. 이동 성분은 위치(`left`·`top`)이므로
    이동이 있는 값은 `unsupported-value`입니다(속성 모델 5장 `transform` 행). Frame은 payload transform, 그 밖의 node
    (Fragment 제외)는 위치 Transform modifier(`position_modifier`)이고 이동 성분은 그대로 둡니다. 위치 modifier가 없고
    선형 부분이 단위 행렬이 아니면 Transform modifier를 새로 넣습니다.
  - `stroke-linecap`(Line)·`image-rendering`(Image): node payload keyword `SetProperty`.
  - `shadow-color`·`shadow-blur`·`shadow-offset-x`·`shadow-offset-y`·`blur-radius`·`mix-blend-mode`·`opacity`: 그 종류의
    **첫 활성 modifier**의 속성입니다. 없으면 기본 payload(`ModifierPayload::default_for`)에 값을 쓴 활성 modifier를
    새로 넣습니다. 같은 종류가 여럿이면 첫 활성 하나만 선언으로 나타냅니다(CSS longhand 하나에 값 하나).
- 새 modifier 위치는 CSS 고정 적용 순서(속성 모델 9.1: transform → paint(fill·stroke·shadow, Stellar 전용) → filter(blur)
  → clip-path → mask → opacity → mix-blend-mode)에서 같거나 앞 단계인 마지막 항목 바로 뒤입니다
  (`modifier_ops::css_insert_order`). CSS 순서를 지키던 stack은 넣은 뒤에도 지킵니다. 속성 패널의 Fill·Stroke·위치
  Transform 삽입 규칙(`inspector::paint_order`)은 이 개정에서 바꾸지 않았습니다. 그 규칙은 Stroke를 stack 끝에 넣어
  Opacity·Blend 뒤에 올 수 있으므로, 속성 패널 행을 registry 기반으로 넓힐 때 이 순서 하나로 합칩니다.
- 이름·값 문법은 registry 항목(`css::registry::entry`)에서 읽고, 값은 결정 5의 parser·formatter입니다. CSS longhand
  inventory에 없는 이름(`shadow-*`, `blur-radius`)은 Stellar 전용 표시를 답니다.
- 그래서 이 속성들을 가진 node에서도 copy → paste 왕복의 semantic snapshot이 같고, `stellar apply`의 `set-property`도
  같은 경로로 이 속성을 받습니다. 저장 위치는 있지만 아직 경로가 없는 속성(`transform-origin`, `mask-image`,
  `clip-path` 등)은 `unsupported-property`입니다.
