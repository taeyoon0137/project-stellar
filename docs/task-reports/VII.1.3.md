# 작업 보고서: VII.1.3 base DocumentRevision과 precondition 목록

- WBS: VII.1.3
- 작업명: 모든 command에 base DocumentRevision과 precondition 목록 포함
- 상태: 완료 (`[x]`)
- 선행: II.3.4, VII.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/state.rs`(신규) — `DocumentState`(모델 + 현재 `DocumentRevision`).
- `crates/commands/src/request.rs`(신규) — `Precondition`, `CommandRequest`(base revision + precondition + command),
  검사·직렬화.
- `crates/commands/src/command.rs` — `CommandError::RevisionMismatch`, `CommandError::PreconditionFailed`.
- `crates/commands/src/codec.rs` — `u64`, 선택적 id writer/reader helper.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.1.3.md`(신규), 계획서(VII.1.3 `[ ]`→`[x]`).

## 구현 요약

- **`DocumentState`**: 한 창의 인메모리 문서(§22.2)인 모델과 revision(II.3.4 `DocumentRevision`)의 묶음입니다.
  모델 변경 공개 경로는 command 실행뿐이도록 `&mut DocumentModel`을 내주지 않습니다.
- **`CommandRequest`**: 모든 command 실행 단위가 base revision과 precondition 목록을 가집니다. `check`는 base와
  현재 revision이 다르면(낡거나 앞선 요청) `RevisionMismatch`, precondition이 거짓이면 첫 실패 위치와 함께
  `PreconditionFailed`를 반환하며 문서를 바꾸지 않습니다.
- **`Precondition`**: node 존재·부재·종류·부모(`None` = View root). tag `1..=4`는 저장 계약입니다.
- **직렬화**: base revision `u64` LE + precondition 수 + precondition* + command(VII.1.2 형식). 알 수 없는
  precondition tag·node kind, trailing bytes를 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 14 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| revision 5 문서에 base 4·6 요청 → `RevisionMismatch`, base 5 요청 통과(`revision_mismatch_fails`) | 통과 |
| precondition 4종 성립/불성립, 첫 실패 index 보고(`preconditions_are_checked_in_order`) | 통과 |
| 요청 round-trip, revision LE, 알 수 없는 tag·trailing 거부(`request_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (실제 문서 command가 아직 없어 검사 대상 command는 테스트 전용 probe입니다 — VII.1.2 보고서 참조.)

## 남은 위험 / 한계

- undo·redo는 보관된 요청을 그 시점 revision으로 다시 맞춰(rebase) 실행해야 합니다. 그 정책은 history(VII.5)가
  정합니다.
