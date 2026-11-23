# 작업 보고서: XIV.12.5 merge·rebase·cherry-pick 자동 continue·commit 없음

- WBS: XIV.12.5
- 작업명: merge/rebase/cherry-pick 진행 상태를 자동 continue하거나 commit하지 않고 Git 상태만 반환
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.12.4(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.12.4
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 6·7

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate/apply.rs` — `ApplyReport::{operation, remaining}`, add 뒤 상태만 다시 읽기, 모듈 문서.
- `crates/ops/src/candidate/apply/tests.rs`, `session/tests.rs` — Git 호출 순서 검사.
- `tools/stellar-cli/src/merge_driver.rs` — 결과 `git` 항목(진행 작업, 남은 unmerged, 다음 단계 안내).
- `tools/stellar-cli/tests/merge_driver.rs` — HEAD 불변 검사.

## 구현 요약

- 해결 적용은 add 뒤 진행 중 작업(`merge`·`rebase`·`cherry-pick`·`revert`·`none`)과 남은 unmerged 경로만 돌려줍니다.
- `MergeGit` 포트에는 continue·commit·push 동작이 없어 해결 경로가 그 명령을 부를 수 없습니다(구조적 보장). CLI adapter의 Git
  명령은 `ls-files`, `cat-file`, `rev-parse --git-path`, `add -A --`, `config --local`뿐입니다(코드 검토).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib candidate::` | 12 passed |
| `cargo test -p stellar-cli --test merge_driver` | 6 passed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `grep -n 'continue\|"commit"' tools/stellar-cli/src/merge_driver/system_git.rs crates/ops/src/candidate/git_port.rs`(주석 제외) | 일치 0 |

| 검사 | 결과 |
|---|---|
| ops: 적용 중 Git 호출이 정확히 unmerged·operation·unmerged·add·operation·unmerged이고 continue·commit 0 | 통과 |
| 시스템 Git: `--apply` 결과 operation = merge, 남은 unmerged 0, HEAD가 적용 전 commit과 같고 `MERGE_HEAD` 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- "UI에는 Git 상태만 반환"은 UI 재설계 전이므로 CLI 결과 JSON과 ops `ApplyReport`로 제공합니다(셸 FFI 추가 없음).

## 남은 위험 / 한계

- add 뒤 상태 읽기가 실패하면 `null`로 보고하며 쓰기·add 결과는 그대로입니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.12.5)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `96bb142c` Feat: 해결 적용 뒤 Git 상태만 반환하고 continue·commit 없음
