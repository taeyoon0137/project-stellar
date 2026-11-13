# 작업 보고서: IX.5.6 nested 컴포넌트 상태 캡슐화 validator

- WBS: IX.5.6
- 작업명: nested component state는 직접 수정할 수 없고 공개 Property/Event chain을 통해서만 연결되게 validator 구현
- 상태: 완료 (`[x]`)
- 선행: IX.4.2(이 lane에서 `[x]`), IX.5.3(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_encapsulation.rs`(신규) — `StateEncapsulationError`,
  `ComponentLibrary::check_state_encapsulation`/`add_transition`/`state_variable_owner`, library 디코딩 검사,
  Switch(nested Toggle) fixture.
- `crates/document/src/component/state_machine.rs` — `StateMachineError::Encapsulation`.
- `crates/document/src/component/definition.rs` — library 디코딩이 상태 머신 캡슐화를 검사.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.6.md`(신규), 계획서(IX.5.6 `[ ]`→`[x]`).

## 구현 요약

- **거부하는 직접 경로**(library 단위 검사, 상태 머신 규칙보다 먼저):
  - `ForeignStateWrite`: 다른 컴포넌트 상태 머신의 변수를 `SetState`로 씀.
  - `ForeignStateRead`: 조건·값 표현식이 다른 컴포넌트 상태 변수를 읽음.
  - `ForeignEventSource`: trigger source가 다른 컴포넌트 정의 tree의 node(nested instance 내부 node)임.
  - `NotNestedInstance`: `NestedAction`의 source가 자기 정의 tree의 ComponentInstance가 아님.
- **허용 경로**: nested instance가 emit한 공개 semantic action에 반응하는 `Trigger::NestedAction`(Event chain)과
  상위 정의의 공개 Property(drilling, IX.4). 상태 머신은 자기 변수만 읽고 씁니다.
- `ComponentLibrary::add_transition`은 캡슐화 → 상태 머신 규칙(IX.5.3) 순으로 검사하고 실패하면 library를
  바꾸지 않습니다. library 디코딩은 모든 컴포넌트를 읽은 뒤 각 transition의 캡슐화를 검사합니다.
- 어떤 컴포넌트에도 없는 node·action(삭제·외부 library 갱신)은 오류가 아니라 IX.5.7 진단 대상입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 281 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Switch가 nested Toggle의 `isOn`을 직접 set → `ForeignStateWrite{owner: Toggle}`, library bytes 불변(`direct_nested_state_write_fixture_fails`) | 통과(거부) |
| nested 상태 직접 read, nested 내부 knob 이벤트 직접 관찰, 인스턴스 아닌 nested action source 거부(`foreign_reads_and_internal_event_sources_fail`) | 통과 |
| nested `toggled` action에 반응해 자기 상태 set·자기 action emit 허용, library round-trip, 변수 소유 컴포넌트 조회(`public_event_chain_is_allowed_and_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 공개 Property 경로(상위 Property → nested Property binding)는 drilling(IX.4.1·IX.4.3~IX.4.6) 차단 해제 후
  연결됩니다.
