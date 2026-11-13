# 작업 보고서: VI.9.7 modifier property metadata와 무효화 class

- WBS: VI.9.7
- 작업명: modifier property metadata와 geometry/layout/paint invalidation class 정의
- 상태: 완료 (`[x]`)
- 선행: VI.9.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier_metadata.rs`(신규) — `InvalidationClass`, `InvalidationSet`,
  `keys`(18개), `ModifierPropertyType`, `InvalidationRule`, `ModifierPropertyMetadata`,
  `MODIFIER_PROPERTIES`, `modifier_property`, `properties_of`, `kind_invalidation`,
  `invalidation_of`, `modifier_payload_keys`, `register_modifier_property_names`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.9.7.md`(신규), 계획서(VI.9.7 `[ ]`→`[x]`).

## 구현 요약

- **무효화 class**: `Geometry`(평가된 geometry·visual bounds), `Layout`, `Paint` 3종과
  bitset `InvalidationSet`. Frame registry(VI.3.5)의 `Invalidation{layout, paint}`를 같은
  집합으로 옮기는 `From` 변환을 제공해 command 결과(VII.1.4)가 한 집합으로 합칠 수 있습니다.
- **key 규칙**: `0x1000 | (kind << 4) | field` — kind nibble 0은 공통(enabled `0x1001`,
  order `0x1002`), 1..=12는 `ModifierKind` 값. node 종류 대역(Frame `0x01xx`, Image `0x06xx`)과
  겹치지 않습니다. core-types의 전역 registry는 건드리지 않았습니다.
- **registry**: 공통 2 + 종류별 payload 필드 16 = 18항목(key 오름차순 상수 표). 각 항목은
  소유 종류, 이름, 값 타입, 기본값 설명, 무효화 규칙을 가집니다.
- **무효화 정책**: modifier는 layout 이후 평가되는 비파괴 stack(§12.6)이므로 어떤 속성도
  layout을 무효화하지 않습니다. paint만 = fill/stroke/shadow 색, blend mode, opacity.
  geometry+paint = transform, boolean op/operands, stroke-expand, corner, mask/clip source,
  stroke 두께, shadow offset/blur, blur radius(visual bounds·overflow 변경). 공통 항목
  (enabled 전환·순서 이동)은 `KindUnion` 규칙으로 해당 종류 payload 무효화의 합집합입니다.
- **누락 검사**: `modifier_payload_keys`가 `ModifierPayload`를 wildcard arm과 `..` 없이
  구조 분해하므로 variant·필드 추가·제거 시 컴파일이 실패하고, 테스트가 종류별
  registry↔payload 필드 1:1과 고아 항목 부재를 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 150 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 12개 kind 전부 payload 필드↔registry 1:1, 고아 항목 없음(`registry_covers_every_payload_field_without_omission`) | 통과 |
| 공통 항목이 모든 kind에 적용(`common_properties_apply_to_every_kind`) | 통과 |
| key 번호 규칙·유일·정렬, 이름 유일(`keys_follow_numbering_rule_and_are_unique_sorted`) | 통과 |
| 값 타입·기본값이 `default_for`·`push_back`과 일치(`value_types_and_defaults_match_payload_defaults`) | 통과 |
| 무효화 class 고정, layout 무효화 0, kind별 합집합 golden(`invalidation_classes_are_fixed`) | 통과 |
| KindUnion 해석·소유 종류 불일치 `None`, 집합 연산, 이름 registry 등록 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- "modifier는 layout을 무효화하지 않음"은 §12.6 비파괴 stack 해석에 따른 정책입니다.
  layout 엔진(Phase X)이 modifier 결과를 layout 입력으로 쓰기로 하면 registry 값을 바꿔야
  합니다.
- shadow offset/blur, blur radius의 geometry 분류는 visual bounds(overflow) 기준의 보수적
  선택입니다. scene dirty 변환(XI.2.1)에서 hit-test geometry와 visual bounds를 분리할 경우
  class를 세분화할 수 있습니다.
