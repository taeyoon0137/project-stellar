# 작업 보고서: XIV.12.6 conflict resolution apply fault rollback

- WBS: XIV.12.6
- 작업명: conflict resolution apply 전체에 fault rollback 구현, git add 실패 시 unresolved 상태와 복구 안내, 자동 commit 없음
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.12.4(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.12.4
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate/apply.rs` — 이전 bytes 기록, `restore`, `ApplyError::{Write, Add}`의 `unrestored`, `guidance`.
- `crates/ops/src/candidate/apply/tests.rs` — 쓰기 실패 주입 `FaultyFiles`, 두 단위(컴포넌트 + generation) fault matrix.
- `tools/stellar-cli/src/merge_driver.rs` — 오류 메시지에 복구 안내.

## 구현 요약

- 쓰기 전에 계획한 모든 경로의 이전 bytes(없으면 없음)를 읽어 둡니다. i번째 쓰기가 실패하면 0..=i를 역순으로 이전 bytes로
  되돌리고(새 파일은 삭제) `Write` 오류를, 모든 쓰기 뒤 `git add`가 실패하면 전부 되돌리고 `Add` 오류를 돌려줍니다. index는 한
  번의 `git add`라 실패하면 바뀌지 않으므로 unmerged 상태가 그대로입니다.
- 되돌리기마저 실패한 경로는 `unrestored`에 남기고 안내에 `git checkout --merge -- <경로>`를 적습니다. 어느 경우도 commit하지
  않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib candidate::apply` | 3 passed |
| `cargo test -p stellar-cli --test merge_driver` | 6 passed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사(fault matrix) | 결과 |
|---|---|
| 컴포넌트 원본 + generation(서로 다른 node 이름 변경) 두 단위, 쓰기 n(≥4)개의 각 위치에서 실패: 작업 트리 전체 digest 동일, unmerged 동일, add 호출 0, 안내에 unresolved | 통과 |
| 모든 쓰기 뒤 `git add` 실패: digest·unmerged 동일, 안내에 "commit하지 않았습니다" | 통과 |
| 쓰기 실패 뒤 되돌리기 하나도 실패: `unrestored` 1개, 안내에 `git checkout --merge` | 통과 |
| 같은 작업 트리에서 정상 적용: 쓰기 n개, unmerged 0, 프로젝트가 generation 3으로 열림 | 통과 |

## 차이(작업 지시와 다른 점)

- 다중 파일 journal(ADR-0078 lane)이 기준 커밋에 없어 프로세스 내 되돌리기로 구현했습니다. 프로세스가 중간에 죽는 경우의 복구는
  journal 통합 때 다룹니다(ADR-0081 결정 6).

## 남은 위험 / 한계

- 쓰기 사이에 프로세스가 강제 종료되면 일부 파일만 바뀐 상태가 남을 수 있습니다. unmerged index는 그대로라 `git checkout --merge`로
  다시 시작할 수 있습니다.
