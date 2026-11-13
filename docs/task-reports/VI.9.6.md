# 작업 보고서: VI.9.6 Fragment modifier 삽입 거부

- WBS: VI.9.6
- 작업명: Fragment에 modifier 삽입을 거부
- 상태: 완료 (`[x]`)
- 선행: VI.4.2, VI.9.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs` — `ModifierError::FragmentOwner`, `can_own_modifiers`,
  `validate_modifier_owner`, `ModifierStack::insert_on`·`push_back_on`과 테스트.
- `docs/task-reports/VI.9.6.md`(신규), 계획서(VI.9.6 `[ ]`→`[x]`).

## 구현 요약

- **정책**: Fragment는 zero-size 구조 marker(VI.4.1)라 **모든 종류의** modifier를
  거부합니다. VI.4.2 금지 시각 속성 분류에 없는 Boolean·Corner 등도 포함합니다.
  변형·효과가 필요하면 Frame 변환(VI.4.4) 후 추가하도록 오류 메시지가 안내합니다.
- **typed error**: `ModifierError::FragmentOwner { kind }` — 시도한 modifier 종류를
  보존합니다.
- **command 경로**: `ModifierStack::insert_on(owner, entry)`·`push_back_on(owner, id, kind)`가
  소유 node 종류를 먼저 검사하고, 실패 시 stack을 바꾸지 않습니다. owner 검사는 kind·payload
  검사보다 먼저 수행합니다. modifier 삽입 command(VII.2.6)는 이 경로를 사용합니다.
- stack 자체는 소유 node 종류를 모르므로, 저장된 문서에서 Fragment가 modifier를 가진
  경우는 문서 validator(VI.10.1)가 다시 거부하도록 문서화했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 142 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과 |

| 검사 | 결과 |
|---|---|
| 12개 modifier kind 전부 Fragment 대상 `push_back_on`·`insert_on` 거부, stack bytes 불변(`fragment_modifier_command_fixtures_fail`) | 통과 |
| owner 검사가 kind/payload 검사보다 우선(`fragment_rejection_precedes_other_entry_checks`) | 통과 |
| Fragment 외 6개 node kind는 12개 kind 모두 허용(`non_fragment_owners_accept_every_modifier_kind`) | 통과 |

## 차이(작업 지시와 다른 점)

- command 계층(Phase VII)이 아직 없어 "command fixture"는 command가 사용할 document 삽입
  경로(`insert_on`·`push_back_on`)에 대한 fixture로 검증했습니다.

## 남은 위험 / 한계

- `ModifierStack::insert`·`push_back`(owner 미확인 경로)은 decode 등 내부 용도로 남아
  있습니다. 저장 문서의 Fragment modifier는 VI.10.1 validator가 막아야 합니다.
