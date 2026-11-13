# 작업 보고서: VIII.7.2 EvaluationEnvironment와 missing key 기본값 정책

- WBS: VIII.7.2
- 작업명: `EvaluationEnvironment`를 immutable map으로 정의하고 missing key의 기본값 정책을 key registry에 고정
- 상태: 완료 (`[x]`)
- 선행: VIII.7.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/environment.rs` — `missing_default`(registry 고정 기본값), `check_value`, `EnvironmentError`,
  `EvaluationEnvironment`(불변 `Arc<BTreeMap>`; `from_values`·`with`·`without`·`explicit`·`is_present`·`value`·
  `changed_keys`, `ReferenceValues` 구현)와 테스트 3개, 모듈 문서의 기본값 표.
- `docs/task-reports/VIII.7.2.md`(신규), 계획서(VIII.7.2 `[ ]`→`[x]`).

## 구현 요약

- **불변 map**: `EvaluationEnvironment`는 공유 참조(`Arc`) map이며 변경 API는 새 환경을 돌려줍니다. 복제는 참조 공유라
  가볍고 원래 환경은 바뀌지 않습니다. 모든 값은 생성 시 registry 타입(같은 enum 타입 포함)과 variant 범위를 검사합니다.
- **missing 정책**(모듈 문서 표에 고정): 값이 없는 등록 key는 null이 아니라 registry 기본값으로 평가합니다(platform
  generic, os_version·api_level 0, color_scheme light, contrast standard, reduced_motion false, viewport 1280×800px,
  locale `"und"`, density 1, interaction_state default). 따라서 환경 참조 타입은 항상 `Required`입니다.
- **변경 비교**: `changed_keys`는 평가 값이 다른 key만 돌려줍니다. 기본값과 같은 값을 명시해도 변화가 아닙니다(VIII.7.3
  reverse index 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (128 unit + 9 integration + 3 doctest passed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **missing fixture**(`missing_keys_use_registry_defaults`): 빈 환경에서 11개 key 모두 기본값이 있고 타입·variant 범위 통과, viewport 1280px·locale `und`, 미등록 key `None`, 참조 값 제공 | 통과 |
| **present fixture**(`present_values_override_and_environment_is_immutable`): 명시 값 우선, `with` 후 원래 환경 불변, `changed_keys` 정확(기본값과 같은 명시는 변화 아님, `without`은 변화), 타입·variant·미등록 key·다른 enum 타입 거부 | 통과 |
| 표현식 `if color_scheme == dark then 1 else 2`가 missing → 2, present(dark) → 1(`expressions_evaluate_against_missing_and_present_values`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- viewport 기본값(1280×800)은 Target Profile 미지정 미리보기용 결정입니다. Target Profile(IX.10)이 생기면 profile별
  기본 viewport로 덮어쓸 수 있습니다.
- 기본값 정책은 숫자 registry와 같은 계약이라 변경 시 ADR 기록이 필요합니다(이 lane 허용 경로 밖).
