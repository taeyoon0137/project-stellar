# 작업 보고서: IX.6.4 Modal 계약

- WBS: IX.6.4
- 작업명: Modal contract에 overlay, focus scope, dismiss event 의미 정의
- 상태: 완료 (`[x]`)
- 선행: IX.6.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/modal.rs`(신규) — `MODAL` 종류(0x0003), field 상수, `modal_contract`,
  `validate_modal`.
- `crates/document/src/control/mod.rs` — 모듈 등록, `FieldChecker::node`(node field 존재·Frame 포함 검사).
- `docs/task-reports/IX.6.4.md`(신규), 계획서(IX.6.4 `[ ]`→`[x]`).

## 구현 요약

- **의미 field**: `overlay`(1, node), `focusScopeRoot`(2, node, 필수), `dismiss`(3, 닫힘의 semantic action),
  `open`(4, Boolean 열림 상태).
- **검사**(`validate_modal`):
  - focus scope root 누락 → `MissingRequired` **진단**.
  - overlay·focus scope root가 없는 node면 `UnresolvedReference`, 계약 Frame 자신·자손이 아니면 `OutsideFrame`
    (오류 — focus가 Frame 밖으로 새는 scope는 허용하지 않음).
  - dismiss는 선언된 action이어야 하고, 상호작용 계약은 `Close`(dismiss 이벤트)를 처리해야 합니다(진단).
- Modal Frame 자체는 focus를 받지 않는 것이 기본입니다(focus는 scope 안 control이 받음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 295 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| focus scope root 없는 Modal → `MissingRequired`(Diagnostic) 1건(`missing_focus_scope_root_is_diagnosed`) | 통과 |
| 없는 overlay node, Frame 밖 focus scope root, Close 미처리 → 3건 정확(`scope_outside_frame_and_missing_dismiss_handling_are_reported`) | 통과 |
| overlay·scope·dismiss·open 모두 채운 Modal → 0건(`complete_modal_has_no_issues`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- focus trap·초기 focus 위치·dismiss 후 focus 복귀 같은 runtime 동작은 preview runtime·접근성(IX.8, Phase XII)의
  몫입니다.
