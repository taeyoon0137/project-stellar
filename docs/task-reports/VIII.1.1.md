# 작업 보고서: VIII.1.1 ExpressionType과 Optional\<T>

- WBS: VIII.1.1
- 작업명: `ExpressionType`을 `TypedValue`와 1:1로 정의하고 nullable은 별도 `Optional<T>`로 표현
- 상태: 완료 (`[x]`)
- 선행: III.3.5(`TypedValue`, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/types.rs`(신규) — `EnumTypeId`, `ExpressionType`, 봉인 trait `NonNullType`, `Optional<T>`, `TypeSpec`, `compile_fail` doctest 2개·양성 doctest 1개, 단위 테스트.
- `crates/expressions/src/lib.rs` — `pub mod types;` 선언, crate 문서 갱신.
- `docs/task-reports/VIII.1.1.md`(신규), 계획서(VIII.1.1 `[ ]`→`[x]`).

## 구현 요약

- **`ExpressionType`(13 variant)**: `TypedValue`의 variant와 같은 순서·같은 개수로 1:1 대응하는 비-nullable
  값 타입입니다. `ExpressionType::of(&TypedValue)`는 wildcard 없는 match라 `TypedValue`에 variant가
  추가되면 매핑을 갱신하기 전까지 컴파일되지 않습니다. `Enum`은 서로 다른 enum 타입을 구분하도록
  `EnumTypeId`(`EnumValue::enum_type`과 같은 numeric ID)를 가집니다. `matches`는 enum 타입 ID까지 비교합니다.
- **`Optional<T>`**: nullable 타입을 일반 타입과 분리한 별도 타입입니다. `T`는 봉인된 `NonNullType`
  (`ExpressionType`만 구현)이어야 하므로 `Optional<Optional<T>>` 중첩과 외부 구현은 컴파일되지 않습니다.
- **`TypeSpec`**: 표현식 위치의 전체 타입(`Required` / `Optional`)입니다. `accepts(Option<&TypedValue>)`는
  `Required`에서 값 없음을 거부하고 `Optional`에서 허용합니다. `to_optional`은 이미 nullable이면 그대로
  두어 중첩하지 않습니다. 진단용 `Display`는 `Length`, `Enum(7)`, `Optional<Color>` 형식입니다.
- 새 numeric registry 번호를 추가하지 않았습니다. 테스트의 대응 golden은 III.3.5의 안정 type tag
  (`0x01`~`0x0D`)를 `TypedValue::type_tag()`로 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (7 unit passed + 3 doctest passed(2 compile_fail, 1 양성), 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **type mapping 완전성**(`type_mapping_is_complete_and_one_to_one`): golden 표 13개 타입마다 대표 값 → `of` 왕복 일치, 대표 값 type tag가 golden tag와 일치, tag 집합이 `0x01`~`0x0D` 전체(누락·중복 없음), 타입 이름 13개 고유 | 통과 |
| 값 쪽 완전성(`every_typed_value_variant_has_exactly_one_type`): 각 대표 값이 정확히 하나의 타입에만 맞음 | 통과 |
| 컴파일 시점 완전성: `of`(TypedValue→타입)와 테스트 `representative`(타입→값) 모두 wildcard 없는 match | 컴파일 통과 |
| enum 타입 ID 구분(같은 ID만 match) | 통과 |
| `Required`는 값 없음 거부, `Optional`은 허용, 타입 불일치는 둘 다 거부 | 통과 |
| `to_optional` 비중첩, 모든 타입의 Optional 형태 존재 | 통과 |
| `Optional<Optional<ExpressionType>>`·외부 `NonNullType` 구현 `compile_fail` | 통과(컴파일 실패 확인) |

## 차이(작업 지시와 다른 점)

- 아키텍처 §14.1의 지원 타입 예시 중 `Component reference`, `Structured token`은 `TypedValue`에 variant가
  없어 1:1 원칙에 따라 추가하지 않았습니다. 값 variant가 추가되면 `of`가 컴파일 실패로 갱신을 강제합니다.
- `Enum`에 `EnumTypeId`를 붙였습니다. variant 수준의 1:1 대응은 유지하면서, 이후 비교·조건식 type rule
  (VIII.1.3)이 서로 다른 enum 타입을 구분할 수 있게 하기 위해서입니다.

## 남은 위험 / 한계

- 타입 직렬화 형식(타입 tag·Optional 표기 bytes)은 정의하지 않았습니다. canonical AST serializer(VIII.2.5)
  에서 필요하면 `TypedValue` type tag를 재사용하는 방식으로 정해야 합니다.
- 산술·비교·함수 type rule은 VIII.1.2~VIII.1.4 소관입니다. `ResourceRef`의 리소스 종류 구분은
  `TypedValue::ResourceRef`가 종류를 담지 않아 이 작업에서 다루지 않았습니다.
