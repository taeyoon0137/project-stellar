# 작업 보고서: IX.6.3 TextInput 계약

- WBS: IX.6.3
- 작업명: TextInput contract에 editable value, placeholder, focus, IME input 의미 정의
- 상태: 완료 (`[x]`)
- 선행: IX.6.1(이 lane에서 `[x]`), VI.7.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/text_input.rs`(신규) — `TEXT_INPUT` 종류(0x0002), field 상수,
  `text_input_contract`, `validate_text_input`.
- `crates/document/src/control/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.6.3.md`(신규), 계획서(IX.6.3 `[ ]`→`[x]`).

## 구현 요약

- **의미 field**: `value`(1, 편집 값 — String 상태 변수만, 필수), `placeholder`(2, String), `focused`(3, Boolean),
  `imeInput`(4, Boolean — IME 조합 입력 허용, 없으면 허용).
- **검사**(`validate_text_input`):
  - 편집 값 binding 타입이 String이 아니면 `TypeMismatch` **오류**.
  - 편집 값이 상수·Property면 입력이 쓸 수 없으므로 `NotWritable` 오류, 없는 상태 변수는 `UnresolvedReference`.
  - 편집 값 누락은 `MissingRequired` 진단. focus 불가능·`Input` 미처리는 진단.
- 값의 텍스트 모델은 VI.7.1 Text payload와 같은 UTF-8 `String`(core-types `TypedValue::String`)입니다. Input 이벤트
  payload(IX.5.2)도 String이라 dispatch가 그대로 상태 변수에 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 292 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Boolean 상태 변수를 편집 값으로 → `TypeMismatch`(Error), String 상수·Property → `NotWritable`, 없는 변수 → `UnresolvedReference`(`non_string_editable_binding_is_an_error`) | 통과(오류) |
| String 상태 + placeholder + imeInput → 0건(`string_state_binding_is_valid`) | 통과 |
| 값 누락·focus 불가·Input 미처리 → 진단 3건(`missing_value_and_interaction_are_diagnosed`) | 통과 |

## 차이(작업 지시와 다른 점)

- 편집 값은 상태 변수만 허용했습니다(입력이 값을 써야 하므로). 상위가 값을 제어하는 controlled input은 상위
  Property → 상태 변수 drilling(IX.4)으로 표현합니다.

## 남은 위험 / 한계

- IME 조합 중 중간 문자열(marked text)의 표현은 셸 텍스트 입력 계층(Phase XII)이 정하며, 계약은 허용 여부만
  담습니다.
