# 작업 보고서: VII.1.6 CommandTransaction

- WBS: VII.1.6
- 작업명: 여러 command를 묶는 `CommandTransaction` — 전부 성공 또는 전부 롤백
- 상태: 완료 (`[x]`)
- 선행: VII.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/transaction.rs`(신규) — `CommandTransaction`, `TransactionError`, `AppliedTransaction`,
  `DocumentState::execute_transaction`, 내부 `run_requests`, 직렬화, `rebased`.
- `crates/commands/src/state.rs` — 단일 요청 `execute`를 요청 하나짜리 transaction 경로로 통일.
- `crates/commands/src/command.rs` — `CommandError::EmptyTransaction`.
- `crates/commands/src/codec.rs` — `Writer::raw`, `Reader::rest` helper.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.1.6.md`(신규), 계획서(VII.1.6 `[ ]`→`[x]`).

## 구현 요약

- **원자 실행**: 한 transaction guard(VII.1.5) 안에서 요청마다 base revision·precondition 검사 → validate → invert →
  apply를 순서대로 수행합니다. 뒤 command는 앞 command가 바꾼 문서로 검증됩니다. 어느 단계에서든 실패하면 guard가
  앞 command 효과까지 모두 되돌리고 실패 요청 index와 원인(`TransactionError`)을 반환합니다. revision은 유지됩니다.
- **revision**: 모든 요청의 base는 시작 revision과 같아야 하며, 성공 시 transaction당 한 번만 올립니다. 빈 transaction은
  `EmptyTransaction`으로 거부합니다.
- **역 transaction**: 각 command의 역을 역순으로 담고 base revision은 실행 후 revision, id·출처는 원 transaction과
  같습니다. `rebased`는 history 재실행(undo/redo)용으로 base revision만 바꾼 사본을 만듭니다.
- **직렬화**: id 16 + origin + 요청 수 `u32` + 요청*(VII.1.3 형식).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 26 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 두 번째 command가 검증 실패·적용 도중 실패하는 두 경우 모두 index 1 오류, 첫 command 효과 없음(snapshot·이름·revision 동일)(`second_command_failure_leaves_no_effect_of_first`) | 통과 |
| 뒤 요청의 낡은 base revision → 롤백, 빈 transaction 거부(`stale_base_in_later_request_rolls_back`) | 통과 |
| 성공 시 revision 1회 증가, change set 병합, 역 transaction 역순·base·id 검증 후 적용하면 원복(`successful_transaction_bumps_revision_once_and_inverts`) | 통과 |
| 4가지 origin round-trip, 잘못된 origin 거부, rebase(`transaction_round_trips_and_rebases`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 문서 command가 아직 없어 원자성 검증은 테스트 전용 probe로 수행했습니다(VII.1.2 보고서 참조).
- 프로덕션 registry가 비어 있는 동안 `with_command`에 `cfg_attr(not(test), allow(unreachable_code))`를 임시로 두었습니다
  (VII.2.1에서 정리).

## 남은 위험 / 한계

- undo/redo가 역 transaction을 어느 revision으로 rebase해 실행할지는 history(VII.5) 정책입니다.
