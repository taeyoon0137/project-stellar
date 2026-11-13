# 작업 보고서: VII.1.5 transaction guard

- WBS: VII.1.5
- 작업명: apply 도중 오류 시 변경 전 arena state로 되돌리는 transaction guard(전체 document clone 금지)
- 상태: 완료 (`[x]`)
- 선행: VII.1.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/edit.rs` — primitive마다 역 primitive를 쌓는 journal, `rollback`, `commit`, `RollbackError`,
  테스트 전용 primitive fault injection(`inject_fault_after`).
- `crates/commands/src/guard.rs`(신규) — RAII `TransactionGuard`(commit 없이 끝나면 drop에서 rollback), 테스트.
- `crates/commands/src/state.rs` — `execute`가 guard로 apply를 감싸고 실패 시 rollback, `rollback_with`.
- `crates/commands/src/command.rs` — `CommandError::RollbackFailed`.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.1.5.md`(신규), 계획서(VII.1.5 `[ ]`→`[x]`).

## 구현 요약

- **journal**: `EditContext`의 성공한 primitive마다 역 primitive를 기록합니다 — 삽입 → leaf 삭제, leaf 삭제 → 레코드와
  modifier stack 재삽입, payload·이름·stack 교체 → 이전 값. **바뀐 부분만** 담으며 전체 문서를 복제하지 않습니다
  (비용은 primitive 수에 비례). 모델이 거부한 primitive는 journal에 남지 않습니다.
- **rollback**: journal을 역순으로 재생합니다. runtime 핸들은 재할당될 수 있으나 의미 상태(레코드·semantic snapshot)는
  변경 전과 같습니다. 재생 실패(내부 불일치)는 나머지를 계속 재생한 뒤 실패 수·첫 원인을 `RollbackError`로 보고합니다.
- **`TransactionGuard`**: `commit`하지 않고 scope를 벗어나면(오류 조기 반환·unwind) drop에서 rollback합니다.
  `DocumentState::execute`는 apply 실패 시 rollback 후 원래 오류를 반환하고 revision을 유지합니다. 되돌리기마저
  실패하면 `RollbackFailed { original, rollback }`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 22 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| random valid document 60 seed × 무작위 primitive 24개, 1..=12번째 이후 fault injection → rollback 후 semantic snapshot 동일, journal ≤ fault 지점, 실제 변경 seed > 30(`mid_apply_fault_rolls_back_to_identical_semantic_snapshot`) | 통과 |
| commit 없이 drop → 원복, journal 길이 = primitive 수, commit 시 변경 유지(`dropped_guard_rolls_back_and_commit_keeps_changes`) | 통과 |
| 변경 후 실패하는 command → 문서·revision 무변경(`failing_command_leaves_model_and_revision_unchanged`) | 통과 |

## 차이(작업 지시와 다른 점)

- fault injection은 프로덕션 코드에 넣지 않고 테스트 빌드 전용 hook(`inject_fault_after`, `TestProbe::fail_after_apply`)으로
  두었습니다.

## 남은 위험 / 한계

- `DocumentModel`이 공통 필드(부모·순서·표시·잠금) 제자리 변경 API를 공개하지 않아, 그런 변경은 이후 command가
  삭제·재삽입 primitive 조합으로 구현합니다(비용 O(subtree)). document crate에 제자리 setter가 생기면 journal에
  해당 역 primitive를 추가해 교체할 수 있습니다.
