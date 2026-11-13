# 작업 보고서: IX.5.5 원자적 이벤트 dispatch

- WBS: IX.5.5
- 작업명: 이벤트 dispatch와 condition 평가, action commit을 원자적으로 구현
- 상태: 완료 (`[x]`)
- 선행: IX.5.3(이 lane에서 `[x]`), IX.5.4(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_dispatch.rs`(신규) — `dispatch`, `DispatchOutcome`, `DispatchError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.5.md`(신규), 계획서(IX.5.5 `[ ]`→`[x]`).

## 구현 요약

1. 이벤트 payload가 이벤트 종류와 맞는지 확인합니다(nested action은 payload 없음).
2. 인스턴스 현재 상태를 snapshot으로 고정하고, 평가 입력을 유효 Property 값 + snapshot 상태 값 + (그
   transition의) 이벤트 payload로 구성합니다.
3. 그 이벤트를 trigger로 가진 transition을 목록 순서(우선순위)로 보며 조건을 평가하고, 처음 참인 transition
   하나만 실행합니다. 조건 없는 transition은 항상 참입니다.
4. 실행할 transition의 모든 `SetState` 값을 **snapshot 기준으로** 평가합니다(앞 action 결과가 뒤 action
   입력이 되지 않음). emit 목록은 action 순서를 따릅니다.
5. 모든 값이 준비된 뒤에만 `StateStore::commit`(IX.5.4, 전부 또는 전무)으로 한꺼번에 반영합니다.

조건·값 평가 오류(0으로 나누기, 평가 한도 초과 등)나 payload 불일치는 상태를 전혀 바꾸지 않고
`DispatchError`를 돌려줍니다. 평가는 `CheckedExpression`의 bytecode 한도(VIII.4.5)를 따릅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 278 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| isOn 반전(성공) 뒤 `count / 0`(실패) → `Action` 오류, isOn 포함 상태 전체 불변(`error_during_actions_leaves_state_unchanged`) | 통과 |
| 조건 평가 오류·payload 불일치 → 상태 불변(`condition_error_and_payload_mismatch_leave_state_unchanged`) | 통과 |
| 우선순위: 조건 거짓이면 다음 transition, 참이면 앞 transition만 실행, 반응 없는 이벤트는 빈 결과(`first_matching_transition_fires_and_commits`) | 통과 |
| `isOn = not isOn` 뒤 `wasOn = isOn` → wasOn은 이전 값(snapshot 기준), Input payload가 변수로 들어감(`values_are_computed_from_the_pre_event_snapshot`) | 통과 |

## 차이(작업 지시와 다른 점)

- 같은 이벤트에 여러 transition이 참이어도 첫 transition 하나만 실행합니다(결정론). 의도하지 않은 가림은
  IX.5.7 진단이 알립니다.

## 남은 위험 / 한계

- emit된 semantic action을 상위 인스턴스의 `NestedAction` trigger로 전달하는 연쇄 dispatch는 preview runtime
  (Phase XII)이 이 함수를 바깥 인스턴스 key로 다시 호출하는 방식으로 연결합니다. 연쇄 깊이 한도는 그때
  정합니다.
