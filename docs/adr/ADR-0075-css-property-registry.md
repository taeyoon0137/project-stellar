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

- 복사는 선택 node의 속성을 registry 순서의 `property: value;` 줄로 냅니다. 붙여넣기는 선언 블록을 해석해
  shorthand를 펼치고, 받을 수 있는 선언을 command로 바꿔 **CommandTransaction 하나**로 적용합니다. 알 수 없는
  속성과 받을 수 없는 값은 이유 코드와 함께 건너뛴 목록으로 돌려줍니다(부분 실패 없음, 조용히 버리지 않음).
- 건너뜀 이유 코드는 값 오류 코드(5절)와 `invalid-declaration`, `unknown-property`(저장 위치 없음),
  `not-applicable`(선택 node 종류에 없음, 레이어 속성), `rejected`(잠금 등 편집 규칙)입니다. 복사는 속성 패널과 같은
  이름·정규 값이고 레이어 속성(`name`·`visible`·`locked`)은 넣지 않으며, Stellar 전용 속성은 표시를 달아 함께
  냅니다. `var()`는 값 binding 저장 경로가 생기기 전까지 건너뜁니다.
- shorthand는 펼친 longhand가 모두 저장 위치를 가지면 longhand로(`border-radius`), 아니면 같은 이름의 묶음 행으로
  (`border`·`overflow`, Stellar가 아직 한 값으로 저장) 적용합니다. 선언은 문서 사본에서 순서대로 적용해 뒤 선언이
  이기고, 결과 command는 한 transaction입니다.
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
