# 작업 보고서: XXV.1.4 기준점 변경 시 journal 봉인

- WBS: XXV.1.4
- 작업명: checkout/merge/rebase 등 기준점 변경 시 journal을 별도 sealed session으로 이동
- 상태: 완료 (`[x]`)
- 선행: XIV.2.2(`[x]`), XXV.1.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/journal_base_change.rs`(신규) — `BaseChangeKind`, `BaseChangeOutcome`,
  `JournalStore::{seal_for_base_change, list_active}`, 테스트 3건.
- `crates/history/src/history.rs` — journal 하한 `journal_floor`, `History::{journal_floor, journal_stacks}`, crate 내부
  `begin_journal_session`·`check_journal_session`.
- `crates/history/src/journal.rs` — `HistoryJournal::capture`가 `journal_stacks`(하한 이후 항목)만 담음.
- `crates/history/src/journal_restore.rs` — `SealReason::BaseChanged(BaseChangeKind)`와 token, `SealReason::ALL` 12개, 봉인 이름
  parser crate 공개.
- `crates/history/src/journal_store.rs` — `JournalStoreError::Replaying`.
- `crates/history/src/window.rs` — crate 내부 `WindowHistory::history_mut`, `WindowHistories::iter_mut`.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XXV.1.4.md`(신규), 계획서(XXV.1.4 `[ ]`→`[x]`).

## 구현 요약

- **진입점** `JournalStore::seal_for_base_change(kind, windows)`: caller(project·runtime)가 Git 작업 뒤 창 문서를 새 기준점
  snapshot으로 바꾼 직후 부릅니다. 무엇이 기준점 변경인지(HEAD commit 변경, `RepositoryStatus` 진행 중 작업 XIV.2.2)는
  git-porcelain을 쓰는 caller가 판정해 `BaseChangeKind`(checkout·merge·rebase·cherry-pick·revert·reset·apply-mailbox·other)로
  넘깁니다. history는 crate graph상 git-porcelain에 의존할 수 없습니다.
- **1단계, 메모리 session 분리**: 열린 창마다 journal 하한을 지금의 다음 sequence로 올리고 열린 grouping을 닫습니다. 이후
  `capture`는 하한 이상 항목만 담습니다. undo는 뒤쪽 연속 구간이고, redo는 다음 redo가 하한 이상일 때만 전부 담습니다. 맨 위가
  하한 미만이면 현재 위치가 기준점 변경 이전이라 하한 이상 redo도 앞 항목 없이는 적용할 수 없으므로 담지 않습니다. 메모리
  undo·redo stack은 그대로라 같은 실행 세션에서는 기준점 변경을 적용한 synthetic command까지 undo할 수 있습니다(§22.5).
- **2단계, 봉인**: 디렉터리의 active journal을 열려 있지 않은 창 것까지 모두 `sealed/<window>.<순번>.<작업 token>.stlhjnl`로
  옮깁니다(XXV.1.3 봉인 경로 재사용, bytes 보존, 자동 복원 없음). 같은 base로 돌아와도(A→B→A) 봉인 session은 복원되지
  않습니다.
- **거부·재시도**: preview 또는 undo 재생 중인 창이 하나라도 있으면 아무 창도 바꾸지 않고 봉인도 하지 않습니다
  (`PreviewActive`/`Replaying`). 순서가 1→2라 봉인 중 I/O 실패에도 이후 기록은 섞이지 않고, 창 없이 다시 부르면 남은 active
  journal만 봉인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 82 passed·1 ignored·doctest 3 passed, history unit 38 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo check -p stellar-project -p stellar-runtime`(history 의존 crate) | 통과 |
| `cargo test -p stellar-document workspace_production_code_does_not_use_test_support`(VI.10.4 검사) | 통과 |

| 검사 | 결과 |
|---|---|
| 기준점 A에서 편집 2개 기록 + 닫힌 창 journal → checkout synthetic command 기록 후 봉인: 두 journal 모두 `.000001.checkout.stlhjnl`, bytes 동일, active 0, 무관한 파일 보존. B에서 새 편집 저장 → journal undo 1개(하한 이상만), 재실행 복원 후 undo가 checkout 직후 상태에서 멈춤(A 항목 없음), redo로 B 상태. B→A 재checkout 후 base 5(A)로 복원해도 `Fresh`(봉인 session 미복원). 메모리 전체 undo는 최초 문서(`checkout_seals_journals_and_later_journals_do_not_mix`) | 통과 |
| text grouping이 열린 채 rebase → 같은 key 입력이 새 항목, 하한 이전까지 undo하면 journal 비어 있음, 하한 위치로 redo하면 journal redo 1개, 복원 후 redo·undo가 하한 범위 안에서만 동작(`groups_close_and_redo_before_floor_is_not_journaled`) | 통과 |
| preview 창이 섞인 호출 → `PreviewActive`, 두 창 하한 0·봉인 0·active 유지. undo hook 안 `Replaying`. 취소 뒤 진행, 빈 창 목록 재시도. 12개 token 유일·봉인 이름 parser 왕복, active 이름 parser는 정확한 형식만(`preview_or_replay_refuses_without_side_effects_and_retry_seals_rest`) | 통과 |
| 검사 민감도(커밋하지 않은 임시 변경): `journal_stacks`의 하한 필터 제거 → 위 첫 두 test 실패. `begin_journal_session`의 grouping 닫기 제거 → grouping test 실패 | 확인 |

## 차이(작업 지시와 다른 점)

- 기준점 변경 판정은 caller 몫으로 두고, history는 판정 결과(`BaseChangeKind`)를 받아 처리합니다(crate graph 제약).
- 메모리 history는 폐기하지 않습니다. §22.5가 synthetic command의 현재 세션 undo를 요구하므로, 섞임 방지는 journal 쪽(봉인 +
  하한)에서 합니다.

## 남은 위험 / 한계

- 마지막 저장 시점 뒤의 미저장 메모리 항목은 journal에 기록되지 않았으므로 봉인 session에도 없습니다.
- 호출 시점 계약(새 snapshot 전환 직후)을 caller가 지켜야 합니다. 전환 전에 부르면 synthetic command가 하한 이후 항목이 되어,
  새 기준점 journal이 기준점 변경 자체를 undo할 수 있게 됩니다.
- 실제 Git 작업과 연결하는 project·runtime 배선은 이 작업 범위 밖입니다.
