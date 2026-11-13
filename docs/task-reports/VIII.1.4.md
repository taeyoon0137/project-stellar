# 작업 보고서: VIII.1.4 함수 signature registry

- WBS: VIII.1.4
- 작업명: color, string, min/max/clamp/round/lerp/remap 함수 signature registry 정의
- 상태: 완료 (`[x]`)
- 선행: VIII.1.2(`[x]`), III.4.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/functions.rs`(신규) — `FunctionId`(16개, 안정 numeric ID), `FunctionSignature`,
  `Param`, `ParamType`(고정/타입 변수), `TypeConstraint::Numeric`, `FunctionCategory`,
  `FunctionLookupError`, `CallTypeError`, `check_call`, registry 문서 표, 단위 테스트.
- `crates/expressions/src/rules.rs` — `OperandPosition::Argument(i)`.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.1.4.md`(신규), 계획서(VIII.1.4 `[ ]`→`[x]`).

## 구현 요약

- **닫힌 registry**: 수치 9개(`min`, `max`, `clamp`, `abs`, `round`, `floor`, `ceil`, `lerp`, `remap`),
  색상 4개(`mix`, `with_alpha`, `to_srgb`, `to_display_p3`), 문자열 3개(`concat`, `replace`,
  `format_number`). numeric ID는 분류별 대역(`0x00xx`/`0x01xx`/`0x02xx`)으로 고정했고 golden test로 잠갔습니다.
  미등록 이름(대소문자·공백 포함)·ID는 `FunctionLookupError`로 거부합니다.
- **signature**: 인자는 고정 타입 또는 수치 타입 변수(`T`, `U` — Number·Length·Percentage·Angle·Duration).
  같은 변수는 호출 안에서 같은 타입이어야 하며, `remap`은 입력 단위 `T`와 출력 단위 `U`를 분리합니다.
  GUI socket용 인자 이름을 포함합니다.
- **검사(`check_call`)**: 인자 수 불일치 → `CallTypeError::Arity`. 그 외에는 왼쪽부터 검사해 첫 위반 인자를
  `OperandPosition::Argument(i)`와 허용 타입 목록으로 보고합니다. 타입 변수는 처음 만난 인자 타입에 묶입니다.
  nullable 인자는 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (29 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **unknown 함수 fixture 실패**(`unknown_function_fixtures_fail`): `""`, `sqrt`, `pow`, `MIN`, 앞뒤 공백, `now`, `random`, `read_file`, `fetch`, `eval`, `exec`, `git`, `loop` 이름과 미등록 ID 8개 거부 | 통과 |
| **인자 수 fixture 실패**(`arity_fixtures_fail`): 16개 함수 모두 인자 +1/−1에서 `Arity` | 통과 |
| **타입 오류 fixture 실패**(`argument_type_fixtures_report_exact_index`): 14개 fixture의 오류 인자 index·허용 목록·실제 타입 정확(타입 변수 충돌, 고정 타입 불일치, nullable 거부 포함) | 통과 |
| 정상 호출 결과 타입 17건(`valid_calls_infer_result_type`) | 통과 |
| ID·이름 유일, tag/name round-trip, 결과 변수가 인자에 등장(`registry_is_closed_unique_and_round_trips`), numeric ID golden | 통과 |

## 차이(작업 지시와 다른 점)

- 아키텍처 §14.1 함수 목록의 `abs`, `floor`, `ceil`을 함께 등록했습니다.
- 색공간 변환(III.4.2 연계)은 `to_srgb`/`to_display_p3` 두 함수로, 투명도는 `with_alpha`로 정의했습니다.
  평가 의미(선형광 혼합, primaries 변환)는 evaluator(VIII.4.4)에서 구현합니다.

## 남은 위험 / 한계

- 단위 변환 함수(예: Percentage→Number)와 nullable 처리 함수는 등록하지 않았습니다. 추가 시 새 ID를 끝에 할당해야 합니다.
- `format_number`의 소수 자릿수 범위 등 값 수준 제약은 evaluator(VIII.4.4)에서 typed error로 처리합니다.
