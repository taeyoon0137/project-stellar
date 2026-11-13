# 작업 보고서: VIII.1.3 비교·논리·조건식 type rule

- WBS: VIII.1.3
- 작업명: 비교·논리·조건식 type rule 정의 — `if` 두 branch는 같거나 명시적 공통 타입
- 상태: 완료 (`[x]`)
- 선행: VIII.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/logic.rs`(신규) — `ComparisonOp`, `LogicalOp`, `common_type`,
  `check_comparison`, `check_logical`, `check_not`, `check_if`, `MatchArmType`, `check_match`, 규칙 표 문서, 단위 테스트.
- `crates/expressions/src/rules.rs` — 위치 `Condition`/`Then`/`Else`/`Scrutinee`/`Pattern(i)`/`Arm(i)`/`Default`,
  `TypeClass`(Equatable·Ordered·Matchable), `Expectation::{Class, CommonWith, DistinctPattern, AtLeastOneArm}`.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.1.3.md`(신규), 계획서(VIII.1.3 `[ ]`→`[x]`).

## 구현 요약

- **비교**: `==`/`!=`는 합성 값(Gradient·Shadow·Typography)을 제외한 같은 값 타입끼리, nullable 허용
  (`Optional<T> == T` 가능). `<`/`<=`/`>`/`>=`는 비-nullable 같은 수치·단위 타입끼리만. 결과는 `Boolean`.
  서로 다른 enum 타입은 비교할 수 없습니다.
- **논리**: `and`/`or`/`not`은 비-nullable `Boolean`만.
- **조건식**: `if` 조건은 비-nullable `Boolean`, 두 branch는 `common_type`이 있어야 합니다. `match`는
  비-nullable 이산 대상(Boolean·Number·String·Enum), 대상과 같은 타입의 서로 다른 pattern 값, arm 1개 이상,
  arm/기본 branch의 공통 타입을 요구합니다.
- **명시적 공통 타입**: `T ~ T → T`, `T ~ Optional<T> → Optional<T>` 두 경우만 등록했습니다. 수치 승격·단위
  변환 같은 암묵 공통 타입은 없습니다.
- **오류 위치**: `Lhs`/`Rhs`/`Operand`, `if`는 `Condition` 또는 뒤 branch `Else`, `match`는 `Scrutinee`,
  `Pattern(i)`(타입 불일치·중복), 앞선 arm들과 공통 타입이 없는 첫 `Arm(i)`, `Default`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (22 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **mismatch fixture 오류 위치 정확**(`mismatch_fixtures_report_exact_position`): if 조건 Number·Optional<Boolean>→`Condition`, Length/Color·Number/Length branch→`Else`, `Color<Color`→`Lhs`, `Length<Percentage`→`Rhs`, nullable 순서 비교 Lhs/Rhs, `Gradient==Gradient`→`Lhs`, `Enum(1)==Enum(2)`→`Rhs`, `Number and Boolean`→`Lhs`, `Boolean or Optional<Boolean>`→`Rhs`, `not String`→`Operand` | 통과 |
| match fixture(`match_rules_and_error_positions`): 대상 Color·Optional·Length→`Scrutinee`, arm 없음→`Scrutinee`, 다른 enum pattern→`Pattern(1)`, 중복 pattern→`Pattern(2)`(DistinctPattern), String pattern→`Pattern(0)`, Color arm→`Arm(2)`(CommonWith Length), Color 기본 branch→`Default` | 통과 |
| 비교 6개 × 28 TypeSpec × 28 전수(`comparison_rules_are_exhaustive`), 논리·not 전수 | 통과 |
| 공통 타입 표 28×28 전수·교환 법칙, 암묵 승격 없음(`common_type_table`) | 통과 |
| nullable 동등 비교 허용(`equality_accepts_nullable_operands`) | 통과 |

## 차이(작업 지시와 다른 점)

- 아키텍처 §14.1의 `switch`/`match`는 하나의 `match` 규칙(리터럴 pattern + 필수 기본 branch)으로 정의했습니다.
  이후 Match AST node(VIII.2.2)가 이 규칙을 사용합니다.
- "명시적 공통 타입"은 등록 표(`T~T`, `T~Optional<T>`)로 해석했습니다. 사용자 지정 공통 타입 표기는 두지 않았습니다.

## 남은 위험 / 한계

- `Optional<T>`을 비-nullable로 바꾸는 연산(예: coalesce)은 없습니다. nullable 값은 동등 비교와 branch
  결과로만 쓰입니다. 필요하면 함수 registry(VIII.1.4 이후)에 별도 행으로 추가해야 합니다.
- 오류의 AST node StableId 매핑은 type checker(VIII.2.4) 소관입니다.
