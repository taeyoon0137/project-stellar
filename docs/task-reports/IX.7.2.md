# 작업 보고서: IX.7.2 VariantCondition

- WBS: IX.7.2
- 작업명: VariantCondition은 변수별 `Any` 또는 `Equals(value)`만 저장, `Any`와 실제 nullable 값은 별도 tag로 구분
- 상태: 완료 (`[x]`)
- 선행: IX.7.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/condition.rs`(신규) — `ConditionTerm`, `VariantCondition`, `Assignment`,
  `ConditionError`.
- `crates/document/src/view_variant/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.7.2.md`(신규), 계획서(IX.7.2 `[ ]`→`[x]`).

## 구현 요약

- **term**: `Any`(제한 없음, tag 0x00)와 `Equals(value)`뿐입니다. `Equals(Value(v))`는 tag 0x01 + 값 bytes,
  실제 null 값 `Equals(ExplicitNull)`은 tag 0x02라 저장 형식에서 `Any`와 구분됩니다. 기술 아키텍처 16장의
  `device = null` 표기는 `Any`(wildcard)입니다.
- **조건**: 변수 → term map(언급하지 않은 변수는 `Any`). 구체성은 `Equals` term 수입니다(IX.7.4 정렬 기준).
- **matching**: `Any`는 모든 값(실제 null 포함), `Equals(ExplicitNull)`은 실제 null 값만 만족합니다. `Assignment`
  (현재 화면 상태)에 없는 변수는 변수 default 값으로 봅니다. View에 없는 변수의 `Equals` term은 만족하지 않습니다
  (삭제 변수의 orphan 조건, IX.7.8).
- **검증**: 조건 변수가 View에 있어야 하고 `Equals` 값은 변수에 허용돼야 합니다(nullable이 아닌 변수의
  `Equals(ExplicitNull)`, 허용 목록 밖 enum 값 거부).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 308 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| `Any`·`Equals(ExplicitNull)` bytes·tag(0x00·0x02) 다름, `Any`는 null·error 모두·ExplicitNull 조건은 null만 만족, 구체성 0/1, 비nullable 변수의 ExplicitNull 조건 거부(`explicit_null_and_any_are_distinct`) | 통과 |
| 조건 round-trip·절단 거부, 없는 변수·허용 밖 값 거부, default 기반 matching, 할당 검증(`conditions_round_trip_and_validate_against_variables`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 명시적 `Any` term과 생략된 변수는 의미가 같지만 저장 bytes가 다릅니다(작성자 의도 보존). 의미 비교는
  `term()`·`specificity()`로 합니다.
