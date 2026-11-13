# 작업 보고서: IX.6.2 Button 계약

- WBS: IX.6.2
- 작업명: Button contract에 activate, focused, disabled, pressed 의미 정의
- 상태: 완료 (`[x]`)
- 선행: IX.6.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/button.rs`(신규) — `BUTTON` 종류(0x0001), field 상수, `button_contract`,
  `validate_button`.
- `crates/document/src/control/mod.rs` — 모듈 등록, 종류 검사기 공용 `FieldChecker`(필수 field·값 타입·action·
  상호작용 검사 도우미).
- `docs/task-reports/IX.6.2.md`(신규), 계획서(IX.6.2 `[ ]`→`[x]`).

## 구현 요약

- **의미 field**: `activate`(1, semantic action — 활성화의 의미, 필수), `focused`(2)·`disabled`(3)·`pressed`(4)
  (Boolean, 상수·Property·상태 변수).
- **검사**(`validate_button`):
  - `activate` 누락 → `MissingRequired` **진단**(저장은 가능, export·접근성 매핑 전에 채워야 함).
  - 선언되지 않은 action → `UnresolvedReference`(오류). 상태 field가 Boolean이 아니면 `TypeMismatch`, 값이 아닌
    갈래(node 등)면 `WrongValueKind`(오류).
  - 상호작용: focus 불가능이면 `NotFocusable`, `Activate`를 처리하지 않으면 `MissingInteraction`(진단).
  - 모르는 field는 `UnknownField`(진단), Button이 아닌 종류는 `UnknownKind`.
- 결과 순서는 field id 순 → 상호작용 → 모르는 field라 결정적입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 289 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| `activate` 없는 Button → `MissingRequired`(Diagnostic) 1건(`missing_required_action_is_diagnosed`) | 통과 |
| 미선언 action, node 갈래 disabled, String 상태 pressed, focus 불가·Activate 미처리 → 5건 순서·내용 정확, 다른 종류 거부(`wrong_values_and_interaction_are_reported`) | 통과 |
| 상태·Property·상수로 채운 완전한 Button → 0건(`complete_button_has_no_issues`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `focused`·`pressed`를 runtime 입력(포인터·focus 이벤트)으로 갱신하는 연결은 preview runtime(Phase XII)과
  접근성 매핑(IX.8.4)에서 합니다.
