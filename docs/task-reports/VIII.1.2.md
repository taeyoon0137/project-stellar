# 작업 보고서: VIII.1.2 산술 type rule table

- WBS: VIII.1.2
- 작업명: 산술 type rule table을 코드와 문서로 정의 — 허용 조합만 등록
- 상태: 완료 (`[x]`)
- 선행: VIII.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/rules.rs`(신규) — 공통 오류 `TypeRuleError`, `OperandPosition`, `Expectation`.
- `crates/expressions/src/arithmetic.rs`(신규) — `ArithmeticOp`, `ArithmeticRule`, `NegationRule`,
  `ARITHMETIC_RULES`(33행), `NEGATION_RULES`(5행), `arithmetic_rule`, `check_arithmetic`,
  `check_negation`, 모듈 문서 규칙 표, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.1.2.md`(신규), 계획서(VIII.1.2 `[ ]`→`[x]`).

## 구현 요약

- **등록 조합만 허용**: `Number op Number → Number`(`+ - * / %` 5개)와 단위
  `U ∈ {Length, Percentage, Angle, Duration}`마다 `U+U→U`, `U−U→U`, `U×Number→U`, `Number×U→U`,
  `U÷Number→U`, `U÷U→Number`, `U%U→U`(7개)를 등록합니다(총 33개). 단항 부호 반전은 Number와 4개
  단위만 허용합니다. 암묵 변환·승격은 없고, 표에 없는 조합(`Length + Color`, `Length × Length`,
  `Length + Number`, 단위 혼합, 문자열·색상·enum 산술 등)은 type error입니다.
- **nullable 금지**: 피연산자가 `Optional<T>`이면 등록 조합이어도 거부합니다.
- **오류 위치**: 왼쪽 타입이 그 연산의 어떤 규칙에도 왼쪽으로 등장하지 않거나 nullable이면 `Lhs`, 그
  외 짝이 맞지 않으면 `Rhs`를 보고하고, 기대 타입 목록(정렬·중복 제거)을 함께 돌려줍니다.
- **코드와 문서 일치**: 규칙 표를 모듈 rustdoc 표로 작성했고, 테스트가 `include_str!`로 이 문서 표를
  직접 읽어 코드 표(`ARITHMETIC_RULES`, `NEGATION_RULES`)와 집합 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (14 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **모든 허용·금지 조합 table-driven**(`every_combination_is_allowed_only_if_registered`): 5개 연산 × 14개 타입(enum 2종 포함) × 14개 = 980 조합 각각이 등록 표에 있을 때만 통과하고 결과 타입이 표와 일치, 허용 33·거부 947 | 통과 |
| nullable 피연산자는 등록 조합이어도 왼쪽·오른쪽 위치로 정확히 거부 | 통과 |
| 문서 표 = 코드 표(`documented_table_matches_code_table`), 중복 행 없음 | 통과 |
| 아키텍처 §14.1 예(`length+length→length`, `length×scalar→length`, `length÷length→number`, `length+color→error`) | 통과 |
| 오류 위치·기대 목록(`Color+Length`→Lhs, `Length+Percentage`·`Length×Length`·`Number÷Length`→Rhs) | 통과 |
| 부호 반전 14개 타입 전수(`negation_rules_are_exhaustive`) | 통과 |

## 차이(작업 지시와 다른 점)

- "문서"는 별도 문서 파일 대신 `arithmetic.rs` 모듈 rustdoc 표로 두었습니다. 이 lane의 수정 허용
  경로가 `crates/expressions/**`이고, 테스트로 문서와 코드의 일치를 강제하기 위해서입니다.
- 나머지(`%`) 규칙과 단항 부호 반전 규칙을 함께 정의했습니다. 아키텍처 §14.1의 지원 연산에 나머지가
  있고, Unary AST node(VIII.2.2)가 부호 반전 규칙을 필요로 하기 때문입니다.

## 남은 위험 / 한계

- `Percentage × Length` 같은 "비율 적용"이나 단위 변환은 등록하지 않았습니다. 필요하면 명시 함수
  (VIII.1.4 이후)로 추가해야 합니다.
- 평가 시 overflow·0 나누기 처리는 evaluator(VIII.4.2) 소관입니다.
