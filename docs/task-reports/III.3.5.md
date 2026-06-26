# 작업 보고서: III.3.5 TypedValue tagged union

- WBS: III.3.5
- 작업명: `TypedValue` tagged union 정의(13개 variant, 안정 type tag, 이진 round-trip)
- 상태: 완료 (`[x]`)
- 선행: III.3.2(단위 newtype), III.3.4(문자열·정규화 경계)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/typed_value.rs` (신규: `TypedValue`, `EnumValue`, `ResourceRef`, placeholder payload 4종, `ValueDecodeError`, `Reader`, 단위테스트)
- `crates/core-types/src/value.rs` (단위 newtype 매크로에 `from_raw(FiniteF32)` const 생성자 추가)
- `crates/core-types/src/lib.rs` (`pub mod typed_value;`)
- `docs/task-reports/III.3.5.md`(신규), 계획서(III.3.5 `[ ]`→`[x]`)

## 구현 요약

- `TypedValue` 13개 variant: Boolean, Number, String, Enum, Length, Percentage, Angle, Duration, Color, Gradient, Shadow, Typography, ResourceRef.
- 각 variant에 **안정 numeric type tag**(`0x01`~`0x0D`)를 부여하고, `to_bytes`(tag 1바이트 + payload) ↔ `from_bytes`로 정확히 round-trip. tag는 저장 포맷 계약이라 변경·재사용하지 않습니다.
- payload 인코딩: float은 `FiniteF32` LE 4바이트(`-0.0`→`0.0`, 비유한 거부), 문자열·blob은 `u32` BE 길이 프리픽스 + bytes, Enum은 `enum_type`/`variant` `u32` BE, ResourceRef는 `StableId` 16바이트.
- `from_bytes`는 입력 전체를 정확히 소비하지 않으면 거부(`TrailingBytes`)하고, 미지 tag·truncation·비유한 float·비 UTF-8·비정상 Boolean을 각각 구조화 오류로 거부합니다.
- `TypedValue`는 `Eq`/`Ord`/`Hash`를 파생하여 canonical 비교(XIV.3.4)에 사용할 수 있습니다(payload가 모두 전순서 타입).
- 보조 타입: `EnumValue`(enum 타입 ID + variant ordinal, 이름은 값에 담지 않음), `ResourceRef`(`StableId` 래핑 값 타입).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (60 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 전 variant binary round-trip(`every_variant_round_trips`) | 통과 |
| type tag 안정성 golden(`type_tags_are_stable_golden`, 13개 tag 고정) | 통과 |
| 단순 variant byte 수준 golden(`golden_bytes_for_simple_variants`) | 통과 |
| 미지 tag·빈 입력·truncation·trailing byte 거부 | 통과 |
| 비유한 float·비 UTF-8·비정상 Boolean 거부 | 통과 |
| `-0.0` Number 디코드 시 `+0.0` 정규화 | 통과 |
| opaque placeholder byte 보존 | 통과 |

## 차이(작업 지시와 다른 점)

- `Color`/`Gradient`/`Shadow`/`Typography` payload 타입은 아직 정식 정의되지 않았습니다(각각 III.4.1, III.4.3, III.4.4, VI.7.2/X.8.1). III.3.5의 검증 대상은 "variant 구성 + 안정 type tag + round-trip"이고 색상 과학·gradient·shadow·typography 구조 설계는 후속 작업 소관이므로, **선점하지 않고** 해당 4개 variant는 불투명(opaque) byte payload placeholder(`ColorValue` 등)로 두었습니다. tag 위치(`0x09`~`0x0C`)와 round-trip envelope만 고정했으며, 후속 작업이 placeholder를 정식 타입으로 교체하면 tag는 그대로 유지됩니다.
- `ResourceRef`는 기존 강한 타입 참조(`crate::reference`)와 별개로, 값으로 쓰이는 리소스 참조이므로 `typed_value` 모듈에 `StableId` 래핑 값 타입으로 정의했습니다.
- `value.rs`에 단위 newtype `from_raw` 생성자를 추가했습니다(round-trip 복원 시 이중 검증을 피하기 위한 최소 보조 API). 기존 III.3.2 compile-fail 계약·테스트는 영향 없음(통과 확인).

## 남은 위험 / 한계

- opaque placeholder는 임의 byte를 허용하므로 그 자체로는 타입 검증을 하지 않습니다. 정식 payload 검증은 III.4.x/VI.7.2/X.8.1에서 placeholder를 실제 타입으로 교체할 때 적용됩니다. 그때 `TypedValue` variant 시그니처는 바뀌지만 type tag는 유지되어야 합니다.
- `String` variant는 일반 UTF-8 `String`을 담습니다. III.3.4의 `UserText`/`NormalizedText` 정규화 경계는 값의 의미(사용자 텍스트 vs 식별자·경로)가 정해지는 상위 계층에서 적용하며, 범용 값 컨테이너인 `TypedValue::String`은 정규화하지 않습니다.
