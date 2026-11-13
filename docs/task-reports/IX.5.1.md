# 작업 보고서: IX.5.1 컴포넌트 local 상태 변수

- WBS: IX.5.1
- 작업명: component local state variable에 StableId, name, type, initial expression 정의
- 상태: 완료 (`[x]`)
- 선행: VIII.2.5(`[x]`), IX.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_machine.rs`(신규) — `StateVariableId`, `StateVariable`, `StateMachine`,
  `PropertyScope`, `StateMachineError`, `state_value_path`/`state_reference`, 상태 머신 key(0x0Bxx),
  `ComponentLibrary::set_state_machine`, 테스트 fixture(Toggle 컴포넌트).
- `crates/document/src/component/definition.rs` — `ComponentEntry`에 상태 머신, library encode/decode에 상태
  머신 추가, `ComponentDefinitionError::StateMachine`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.1.md`(신규), 계획서(IX.5.1 `[ ]`→`[x]`).

## 구현 요약

- **상태 변수**: `StateVariable::new(id, name, value_type, initial, scope)`. initial은 필수이며
  `CheckedExpression`으로 type 검사합니다. 추론 타입이 선언 타입에 대입되지 않으면
  `InitialTypeMismatch { expected, found }`, 알 수 없는 참조 등은 `InitialInvalid`입니다.
- **initial 참조 범위**: `PropertyScope`는 그 컴포넌트의 Property(`ReferenceTarget::ComponentProperty`)만
  타입을 제공합니다. 인스턴스마다 Property 값으로 초기값이 정해지고(예: `initiallyOn`), 다른 상태 변수·token
  참조는 거부됩니다(초기화 순서 의존 제거).
- **참조 경로**: 표현식은 상태 변수 값을 `Property(state_value_path(var))`(root = 변수 StableId, key
  `STATE_VALUE` 0x0B01)로 읽습니다. 다음 작업(transition 조건·action)이 이 경로를 씁니다.
- **저장**: 컴포넌트 entry마다 property record 뒤에 상태 머신(변수 id 오름차순)을 기록합니다. 디코딩은 각
  변수를 그 컴포넌트 Property 환경으로 재검사합니다. 이름은 Property와 같은 규칙이고 머신 안에서 유일해야
  합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 266 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Boolean 변수에 String initial·String property 참조 initial 거부, 다른 상태 변수·없는 property 참조 거부, property 참조·nullable 변수 통과(`initial_type_mismatch_is_rejected`) | 통과 |
| id·이름 중복 거부, library round-trip byte-identical, 없는 컴포넌트 거부(`machine_round_trips_in_library_and_rejects_duplicates`) | 통과 |

## 차이(작업 지시와 다른 점)

- initial이 참조할 수 있는 대상을 그 컴포넌트 Property로 제한했습니다(위 "initial 참조 범위").

## 남은 위험 / 한계

- 상태 머신은 컴포넌트 정의에만 붙습니다. ViewDefinition의 상태 머신(기술 아키텍처 16장 구조도)은 View 변수
  (IX.7) 쪽 후속입니다.
