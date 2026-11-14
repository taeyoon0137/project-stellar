# ADR-0026: ValueBinding 저장 형식과 환경 입력 registry

## Status

Accepted (소급, 2026-11-14). 이미 구현된 결정을 소급 기록합니다. 아래 커밋은 새 binary 저장 형식과
numeric registry를 정했지만 같은 diff에 ADR이 없었습니다(사후 리뷰 EB-4). 동작은 새로 추가하지
않습니다. 다만 코드가 상한을 적지 않은 환경 enum 대역(4절)과 앞으로의 변경 규칙(2\~5절의 "변경
규칙")은 이 ADR이 정합니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| VIII.6.1 | `6fc6aa3` | `ValueBinding` canonical bytes(`BINDING_FORMAT_VERSION = 1`) |
| VIII.7.1 | `a61f9aa` | 환경 입력 key ID 1\~11, 환경 enum 타입 ID와 variant 번호 |
| VIII.7.2 | `6f170d3` | `EvaluationEnvironment`와 missing key 기본값 정책 |

## Context

- I.3.5는 storage format과 numeric registry 변경에 ADR을 요구합니다. 그런데
  `tools/xtask/src/adr_policy.rs`의 보호 경로는 `crates/core-types/`, `crates/storage/`,
  `crates/ffi/`, `schemas/`, `config/performance-budgets.yaml`뿐입니다. 그래서 `crates/bindings/`와
  `crates/expressions/`의 변경은 `validate-adr`에 걸리지 않았습니다. VIII.6.1·VIII.7.1·VIII.7.2
  보고서는 모두 후속 ADR이 필요하다고 적었습니다.
- binding은 아키텍처 §14.2의 통합 Value Binding이고 orphan 처리는 §14.4를 따릅니다. 저장 primitive
  규칙은 V.1.1(ADR-0010), 스키마 버전과 migration은 §36과 V.8(ADR-0011)입니다.
- binding이 담는 하위 bytes는 이미 다른 결정이 정했습니다. `StableId` 16바이트(ADR-0007),
  `PropertyPath::to_canonical_bytes`(III.2.2, ADR-0007), `TypedValue` type tag와 bytes(III.3.5,
  ADR-0008), canonical AST(VIII.2.5, `crates/expressions/src/codec.rs`)가 그것입니다. ADR-0007과
  ADR-0008은 `PropertyPath`·`TypedValue`의 big-endian bytes를 디스크 표현으로 쓸지 정하지
  않았습니다.
- `5d0f2ce` 기준으로 이 bytes를 프로젝트 파일에 쓰는 코드는 없습니다. `stellar-bindings`는
  core-types와 expressions에만 의존합니다. storage·document crate는 `stellar-bindings`에 의존하지
  않습니다.

## Decision

### 1. ValueBinding canonical bytes (버전 1)

`ValueBinding::to_bytes`와 `ValueBinding::from_bytes`(`crates/bindings/src/binding.rs`)의
배치입니다.

```text
binding   := version:u16 LE (= 1) ++ id[16] ++ blob(target) ++ type_spec ++ owner ++ blob(source)
id        := BindingId의 StableId 16바이트(RFC 9562 바이트 순서, ADR-0007)
target    := PropertyPath canonical bytes(III.2.2)
           = root[16] ++ key_count:u32 BE ++ (key:u32 BE) * key_count ++ tail
tail      := 0x00 | 0x01 ++ index:u32 BE | 0x02 ++ len:u32 BE ++ UTF-8 bytes
type_spec := nullable:u8 (0 = Required, 1 = Optional) ++ value_tag:u8 (0x01~0x0D)
             [++ enum_type:u32 LE  (value_tag = 0x04일 때만)]
owner     := kind:u8 (0x01 = Node, 0x02 = Component) ++ id[16]
source    := canonical AST bytes(VIII.2.5, 아래)
blob      := len:u32 LE ++ bytes
```

- `value_tag`는 `TypedValue` type tag(ADR-0008)와 같은 번호입니다(`codec::value_type_tag`). Enum
  타입에는 `EnumTypeId`를 붙입니다.
- 하위 형식은 다시 인코딩하지 않고 그대로 blob에 담습니다. 따라서 binding이 직접 쓰는 정수(version,
  blob 길이, `enum_type`)만 little-endian(V.1.1)입니다. `StableId`는 RFC 9562 바이트 순서이고,
  `PropertyPath` 내부 정수와 AST literal 안 `TypedValue`의 길이·Enum 필드는 big-endian입니다.
  ADR-0007·ADR-0008이 열어 둔 질문에 대해, canonical AST와 binding 형식 안에서는 하위 bytes를
  "그대로 포함"하는 것으로 정합니다.
- binding이 의존하는 canonical AST 부분은 다음과 같습니다(전체 배치는 `codec.rs` 모듈 문서).

  ```text
  ast        := version:u16 LE (= 1, AST_FORMAT_VERSION) ++ node_count:u32 LE ++ node*
  node       := id[16] ++ op[3] ++ payload ++ child_count:u32 LE ++ child*
  reference  := 0x01 ++ token_id[16]
              | 0x02 ++ blob(PropertyPath canonical bytes)
              | 0x03 ++ component_property_id[16]
              | 0x04 ++ environment_key:u32 LE
  ```

  node는 root부터 child slot 순서의 전위 순회 순서로 기록하고, 공유 node는 한 번만 씁니다.
- 디코더 규칙:
  - 버전이 1이 아니면(0 포함) `UnsupportedVersion`입니다.
  - 입력이 중간에 끊기거나 길이 프리픽스가 남은 입력보다 길면 할당 전에 `Truncated`입니다. bytes가
    남으면 `TrailingBytes`입니다.
  - 미지 nullable flag, value tag, owner 종류는 각각 `UnknownNullability`, `UnknownTypeTag`,
    `UnknownOwner`입니다.
  - `PropertyPath`는 다시 직렬화한 bytes가 입력과 같아야 하며, 다르면 `InvalidTarget`입니다.
  - source는 AST decoder(닫힌 연산 집합, AST 구조 검증, canonical 순서)를 통과해야 하며, 실패하면
    `Source(CodecError)`입니다.
  - 마지막으로 binding 전체를 다시 인코딩한 bytes가 입력과 같아야 합니다(`NonCanonical`). 그래서
    binding 하나에는 bytes 표현이 하나뿐입니다.
  - 인코딩할 때 blob 길이가 `u32`를 넘으면 `LengthOverflow`입니다.
- 디코더가 검사하지 않는 것: `enum_type` 값(아무 `u32`나 받음), `expected_type`이 target 속성의 실제
  선언 타입과 같은지(호출자 책임), source의 type 검사(`validate_types`가 따로 함), source가 참조하는
  environment key가 등록되었는지.
- 형식에 넣지 않은 것: 마지막 계산값(§14.4), orphan 원인(`OrphanCause`), `BindingSet`·`BindingStore`·
  `OrphanedBinding` 컨테이너, binding을 chunk에 배치하는 방식. 이것들의 bytes 형식은 정하지
  않았습니다.

### 2. 버전과 마이그레이션(V.8)

구현된 것:

- `BINDING_FORMAT_VERSION`(u16, 현재 1)은 binding record 하나의 형식 버전입니다. source 안의
  `AST_FORMAT_VERSION`(u16, 현재 1)과 독립입니다. storage generation manifest의 `schema_version`
  (ADR-0011, 최신 2)과 `stellar.yaml`의 `CURRENT_SCHEMA_VERSION`(현재 1)과도 별개입니다.
- 디코더는 현재 버전만 받고 다른 버전을 변환하지 않습니다. 재인코딩 일치 검사가 있어 옛 버전
  bytes를 그대로 받아들이는 경로가 없습니다.
- binding record에는 `minimum_reader_version`과 `minimum_writer_version`이 없습니다.

변경 규칙:

1. 다음은 모두 형식 변경이며 `BINDING_FORMAT_VERSION`을 올립니다: 필드 순서·폭·엔디언, tag 의미,
   새 tag(owner 종류·value tag) 추가. 옛 디코더는 새 tag를 미지 tag로 거부하므로 tag 추가도 형식
   변경입니다. 기존 tag 번호는 재사용하거나 바꾸지 않습니다.
2. source AST 형식만 바뀌면 `AST_FORMAT_VERSION`만 올립니다. binding 머리의 버전은 binding 자체
   배치가 바뀔 때만 올립니다.
3. 저장된 binding이 있는 상태에서 형식 버전을 올리면 storage 문서 스키마 버전도 함께 올립니다.
   그리고 옛 record를 옛 디코더로 읽어 현재 encoder로 다시 쓰는 `MigrationStep`(V.8.2)을 제품
   migration registry(ADR-0011)에 등록합니다. 실행은 ADR-0011 규칙을 그대로 따릅니다.
   - temp generation에서 변환하고 모든 검증을 마친 뒤 pointer를 전환합니다(V.8.3, V.8.4).
   - 원본 세대는 바꾸지 않습니다.
   - 자동 commit하지 않습니다(V.8.5).
   - 옛 형식 golden fixture를 `cargo xtask test-migrations`(V.8.6)에 추가합니다.
4. 구버전 앱 보호는 binding record가 아니라 storage 스키마 판정(V.8.1)이 맡습니다. 새 형식을 쓰는
   세대는 `minimum_writer_version`을 올려 구버전 앱이 편집하지 못하게 합니다(§36). 자동
   다운그레이드는 하지 않습니다. ADR-0011에 따르면 `minimum_reader_version`·`minimum_writer_version`을
   디스크 어디에 기록할지 아직 정하지 않았습니다. binding 저장을 연결하기 전에 정해야 합니다.

### 3. 환경 입력 key registry

`crates/expressions/src/environment.rs`가 정의합니다. key는 `EnvironmentKey(u32)`이고, canonical
AST의 reference payload `0x04`에 `u32` LE로 기록됩니다.

| ID | 이름 | 값 타입 |
|---|---|---|
| 1 | `platform` | Enum(`0xE000_0001`) |
| 2 | `os_version` | Number |
| 3 | `api_level` | Number |
| 4 | `color_scheme` | Enum(`0xE000_0002`) |
| 5 | `contrast` | Enum(`0xE000_0003`) |
| 6 | `reduced_motion` | Boolean |
| 7 | `viewport_width` | Length |
| 8 | `viewport_height` | Length |
| 9 | `locale` | String(BCP 47 tag) |
| 10 | `density` | Number(display scale) |
| 11 | `interaction_state` | Enum(`0xE000_0004`) |

- 0은 할당하지 않습니다(`key_info(EnvironmentKey::new(0))`은 `None`). 이름은 snake_case이고
  유일합니다(`key_by_name`).
- 환경 참조의 타입은 항상 `Required(value_type)`입니다(5절).
- golden test `key_numeric_ids_match_golden`이 ID·이름·타입·variant 수 표를 고정합니다.
  `registry_lookup_and_types`는 번호가 1부터 빈틈없이 이어지는지와 이름이 유일한지 검사합니다.

변경 규칙:

- 새 key는 다음 번호(12, 13, …)를 받습니다. 기존 key의 번호, 이름, 값 타입은 바꾸지 않고 번호를
  재사용하지 않습니다. 값 타입을 바꿔야 하면 새 key를 만듭니다.
- 더 쓰지 않는 key도 registry에서 지우지 않습니다. 저장된 표현식이 그 key로 계속 type 검사를
  통과해야 하고, 연속 번호 검사도 그대로 유지해야 하기 때문입니다.

### 4. 환경 enum 타입 ID와 variant 번호

- `EnumTypeId`는 `TypedValue::Enum`의 `enum_type`과 같은 `u32` 번호 공간입니다. 저장소 전체의
  할당표는 아직 없습니다.
- 환경 enum 타입 대역은 `0xE000_0000`\~`0xE000_FFFF`(상위 16 bit가 `0xE000`)입니다. 코드 문서는
  "`0xE000_0000` 대역"이라고만 적었으므로 이 ADR이 상한을 정합니다. `0xE000_0000`은 할당하지
  않습니다. 다른 enum registry는 이 대역에 할당하지 않습니다.

| 타입 ID | key | variant(번호) |
|---|---|---|
| `0xE000_0001` | `platform` | generic(0), web(1), apple(2), android(3) |
| `0xE000_0002` | `color_scheme` | light(0), dark(1) |
| `0xE000_0003` | `contrast` | standard(0), high(1) |
| `0xE000_0004` | `interaction_state` | default(0), hover(1), pressed(2), focused(3), disabled(4) |

- variant 번호는 0부터 연속이며 개수는 `EnvironmentKeyInfo::variants`입니다. `check_value`는 개수
  이상인 variant를 `InvalidVariant`로 거부합니다.
- `5d0f2ce` 기준 대역 밖 할당은 document List 계약(IX.6.5)의 `0x000C_0001`\~`0x000C_0003`
  (`ITEM_IDENTITY_ENUM`, `SELECTION_MODE_ENUM`, `NAVIGATION_ENUM`)입니다. 충돌은 없습니다. 테스트
  fixture는 작은 번호(예: 3, 7, `0x0D01`)를 씁니다. `crates/semantic-diff/src/keys.rs`의
  `0xE000`\~`0xEFFF`는 `PropertyKey` 대역으로, 다른 번호 공간입니다.

변경 규칙:

- 새 환경 enum 타입은 대역 안의 다음 번호(`0xE000_0005`, …)를 받습니다.
- 새 variant는 끝에 추가하고 기존 번호를 바꾸거나 재사용하지 않습니다. variant 추가도 registry
  변경이므로 이 ADR을 갱신하거나 대체합니다.

### 5. missing key 기본값 정책

`EvaluationEnvironment`(불변 `Arc<BTreeMap>`)는 명시한 값만 보관합니다. 값이 없는 등록 key는 null이
아니라 `missing_default`가 고정한 기본값으로 평가합니다.

| key | 기본값 |
|---|---|
| `platform` | generic(0) |
| `os_version`, `api_level` | 0 |
| `color_scheme` | light(0) |
| `contrast` | standard(0) |
| `reduced_motion` | `false` |
| `viewport_width` × `viewport_height` | 1280px × 800px |
| `locale` | `"und"`(BCP 47 미정) |
| `density` | 1 |
| `interaction_state` | default(0) |

- 미등록 key에는 기본값이 없습니다. `value`는 `None`을 돌려주고, 그 참조는 모르는 대상입니다.
- 명시 값은 모두 생성할 때 registry 타입(같은 enum 타입 포함)과 variant 범위를 검사합니다.
- `changed_keys`는 평가 값이 실제로 다른 key만 돌려줍니다. 기본값과 같은 값을 명시해도 변경으로
  보지 않습니다.

변경 규칙:

- 기본값을 바꾸면 저장된 표현식의 평가 결과가 바뀌므로 기본값은 registry와 같은 계약입니다.
  바꾸려면 이 ADR을 갱신하거나 대체합니다.
- Target Profile(IX.10)별 값(예: profile 기본 viewport)은 `missing_default`를 바꾸지 않습니다.
  호출자가 명시 값으로 넣습니다.

## Alternatives

- **외부 직렬화 crate(serde·bincode 등):** 외부 의존성 승인이 필요하고, 값 하나에 bytes 표현 하나를
  보장하지 않습니다. 미선택.
- **하위 bytes(`PropertyPath`·`TypedValue`)를 little-endian으로 다시 인코딩:** core-types 형식과
  별도로 두 번째 형식이 생기고 두 decoder를 계속 맞춰야 합니다. 그대로 포함했습니다. 미선택.
- **디코더가 옛 버전을 직접 읽기(버전별 분기):** 한 binding에 한 표현이라는 원칙과 맞지 않습니다.
  V.8은 변환을 새 generation migration으로 하도록 정합니다. 미선택.
- **binding record마다 minimum reader/writer 버전 기록:** storage 스키마 판정(V.8.1)과 겹칩니다.
  미선택.
- **environment key를 이름 문자열로 저장:** 이름 변경에 약하고 bytes가 커집니다. `PropertyKey`처럼
  numeric ID를 썼습니다. 미선택.
- **missing key를 null(`Optional`)로 평가:** 모든 환경 참조에 null 처리가 필요해집니다(VIII.7.2).
  미선택.

## Consequences

- 같은 binding은 항상 같은 bytes입니다. 그래서 bytes 비교와 hash로 변경을 판정할 수 있습니다.
- 저장 형식에는 environment key 이름이 아니라 번호가 들어갑니다. 표현식 bytes는 이름 문자열에
  의존하지 않습니다.
- 한 record 안에 엔디언이 섞입니다. binding·AST 머리는 LE이고, `PropertyPath`·`TypedValue` 내부는
  BE입니다. 같은 `EnumTypeId` 값이 type 위치(`type_spec`, AST Null literal)에서는 LE로, Enum literal
  값 안에서는 BE로 기록됩니다.
- 구버전 reader는 새 environment key를 미등록으로 봅니다. type 검사에서는 모르는 참조 오류가 됩니다.
  `BindingWorld`가 key registry로 타입을 알려 주면 orphan validator는 그 binding을 `SourceMissing`
  orphan으로 판정합니다. key 추가는 binding·AST 배치를 바꾸지 않습니다. 하지만 binding 저장을 연결한
  뒤에는 storage 스키마 버전과 `minimum_writer_version`을 올려 구버전 편집을 막아야 합니다(2절 규칙
  4).
- 구버전 reader는 새 enum variant를 `EvaluationEnvironment`에 넣지 못합니다(`InvalidVariant`). 저장된
  AST 안의 새 variant literal은 decode와 type 검사를 통과하지만, 구버전 환경 값과는 같아지지
  않습니다.
- orphan 손상 기본값의 안전값 표(VIII.8.3, `fallback.rs`)는 이 ADR 범위 밖입니다.

이 ADR을 쓰며 확인한, 코드·보고서와 다른 점입니다(코드는 바꾸지 않았습니다).

- `binding.rs` 모듈 문서와 VIII.6.1 보고서는 형식이 "V.1.1 규칙(little-endian)"을 따른다고 적었습니다.
  실제로 LE는 binding이 직접 쓰는 정수에만 해당하고, 포함된 `PropertyPath`·`TypedValue` bytes는
  BE입니다(1절). `decode_property_path` 주석에는 BE가 적혀 있습니다.
- ADR-0007·ADR-0008은 `PropertyPath`·`TypedValue` bytes가 "core-types 밖에서 사용되지 않는다"고
  적었습니다. 실제로는 canonical AST(VIII.2.2, VIII.2.5)와 binding 형식(VIII.6.1)이 이 bytes를 그대로
  포함합니다.
- VIII.7.1 보고서는 "현재 저장소에 다른 enum 타입 ID registry 없음"이라고 적었습니다. `5d0f2ce`에는
  document List 계약의 `0x000C_0001`\~`0x000C_0003`이 있습니다. 대역 밖이라 충돌은 없습니다.
- `environment.rs`의 "`0xE000_0000` 대역" 문구에는 상한이 없습니다. 이 ADR의 상한(`0xE000_FFFF`)을
  코드 주석에 반영하는 일은 후속 작업입니다.
- `ValueBinding`에는 고정 golden bytes test가 없습니다. test는 round-trip, 머리 2바이트 버전, 손상
  거부만 확인합니다. 그래서 배치가 바뀌어도 round-trip만 맞으면 통과합니다. canonical AST
  (`canonical_bytes_golden`)와 key registry(`key_numeric_ids_match_golden`)에는 golden test가 있습니다.
  binding golden fixture 추가는 후속 작업입니다.
- `adr_policy.rs` 보호 경로에 `crates/bindings/`·`crates/expressions/`가 없습니다. 이 형식이나
  registry를 바꿔도 `validate-adr`가 실패하지 않습니다. 보호 경로 추가는 후속 작업입니다.
- canonical AST 형식(VIII.2.5, `AST_FORMAT_VERSION = 1`) 자체에도 ADR이 없습니다. 이 ADR은 binding이
  의존하는 부분(머리 버전, environment key 기록)만 다룹니다.

## Compatibility

- 파일 포맷: `ValueBinding` 버전 1 배치, environment key ID 1\~11, 환경 enum 타입 ID와 variant 번호,
  missing 기본값이 계약입니다. 바꾸려면 위 변경 규칙을 따르고 이 ADR을 갱신하거나 대체합니다. 현재
  이 bytes를 담은 프로젝트 파일은 없습니다.
- 공개 API: `BINDING_FORMAT_VERSION`, `ValueBinding::to_bytes`·`from_bytes`, `BindingCodecError`,
  `environment` 모듈의 key 상수·`*_TYPE` 상수·variant 모듈, `key_info`, `key_by_name`,
  `missing_default`, `check_value`, `EvaluationEnvironment`가 이 결정에 묶입니다. 이 ADR은 API를
  바꾸지 않습니다.
- FFI: 영향 없습니다.

## Performance

- 인코딩·디코딩 비용은 입력 크기에 비례합니다. 디코드할 때 source AST를 두 번 더 인코딩합니다(AST
  decoder의 canonical 검사 1회, binding 재인코딩 검사 1회).
- `key_info`는 상수 시간 `match`이고, `key_by_name`은 key 11개를 선형 탐색합니다.
  `EvaluationEnvironment::with`·`without`은 명시 값 map(최대 11개)을 복제합니다.
- 성능 예산 파일은 바꾸지 않습니다.

## Security

- 디코더는 신뢰할 수 없는 bytes를 받아도 panic하지 않고 구조화 오류를 돌려줍니다. 경계는 checked
  연산으로 검사하고, 길이 프리픽스가 남은 입력보다 길면 할당 전에 거부합니다. 모든 길이의 잘림과
  지정 위치 손상을 거부하는 test가 있습니다. 무작위 byte 변형 test는 AST codec에만 있습니다.
- source는 닫힌 연산 집합(VIII.1.5)만 받습니다.
- 환경 값은 생성할 때 타입과 variant 범위를 검사합니다. 미등록 key는 오류이며 기본값으로 바꾸지
  않습니다.

## Migration

- 해당 없음. 이 결정 전에도 후에도 binding bytes를 담은 프로젝트 파일이나 세대는 없습니다. 저장을
  처음 연결할 때는 버전 1로 씁니다. 그 뒤의 변경은 2절 규칙을 따릅니다.
