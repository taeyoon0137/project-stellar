# 작업 보고서: IX.5.3 Transition

- WBS: IX.5.3
- 작업명: Transition에 source event, condition expression, ordered actions 정의. action은 state set과 semantic action emit으로 제한
- 상태: 완료 (`[x]`)
- 선행: IX.5.2(이 lane에서 `[x]`), VIII.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_machine.rs` — `SemanticActionId`/`SemanticAction`, `TransitionId`,
  `Trigger`, `TransitionAction`(닫힌 enum), `ActionSpec`/`TransitionSpec`, `Transition`, `MachineScope`,
  `event_payload_path`, `StateMachine`의 action·transition 목록과 저장 형식, 오류 variant, 테스트.
- `crates/document/src/component/typed_expr.rs` — 테스트 AST 도우미 `unary`.
- `docs/task-reports/IX.5.3.md`(신규), 계획서(IX.5.3 `[ ]`→`[x]`).

## 구현 요약

- **source event**(`Trigger`): `Event { kind, source }`(이 정의 tree node에서 난 이벤트) 또는
  `NestedAction { instance, action }`(nested instance가 emit한 공개 semantic action — IX.5.6 Event chain).
- **조건식**: 선택. 있으면 `Required(Boolean)`로 type 검사합니다. 참조 환경(`MachineScope`)은 그 컴포넌트
  Property, 이 상태 머신의 상태 변수(`state_value_path`), 이 transition의 이벤트 payload
  (`event_payload_path`, payload가 있는 이벤트 종류일 때만)입니다.
- **ordered actions**: 닫힌 enum `TransitionAction` — `SetState { variable, value }`(값은 변수 타입으로 검사)와
  `Emit { action }`(이 머신이 선언한 action만) 두 갈래뿐입니다. 저장 tag도 0x01·0x02뿐이며 다른 tag는
  디코딩에서 거부하므로 파일·네트워크·임의 코드 action은 표현할 수 없습니다. 조건식·값도 닫힌 표현식
  AST(VIII.1.5)입니다. 한 transition이 같은 변수를 두 번 set하면 거부합니다(결과 모호).
- **우선순위**: transition 목록 순서가 우선순위이며 저장 시 그대로 보존합니다. action·변수·transition id는
  머신 안에서 서로 겹칠 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 271 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 조건 + SetState·Emit 순서 보존, Input payload → String 변수, library round-trip byte-identical(`transition_has_event_condition_and_ordered_actions`) | 통과 |
| 비Boolean 조건·값 타입 불일치·없는 변수·미선언 action·중복 set·payload 없는 이벤트의 payload 참조·id 충돌 거부(`invalid_transitions_are_rejected`) | 통과 |
| action이 두 갈래뿐(wildcard 없는 match), 저장 action tag를 0x00·0x03·0x10·0xFF로 바꾸면 디코딩 거부(`file_or_network_actions_cannot_be_expressed`) | 통과 |

## 차이(작업 지시와 다른 점)

- emit 대상인 semantic action을 상태 머신에 선언하게 했습니다(상위 컴포넌트가 반응할 공개 Event 목록).

## 남은 위험 / 한계

- trigger의 source node·nested action 존재 여부는 정의 tree·다른 컴포넌트 편집으로 바뀔 수 있어 오류가 아니라
  진단(IX.5.7 missing target) 대상으로 남겼습니다.
