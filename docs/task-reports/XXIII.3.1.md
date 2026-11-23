# 작업 보고서: XXIII.3.1 결과 target 정의와 검증

- WBS: XXIII.3.1
- 작업명: 결과 target을 current worktree, existing worktree, new branch+worktree로 정의
- 상태: 완료 (`[x]`)
- 선행: XXIII.2.4
- 결정: [ADR-0110](../adr/ADR-0110-window-reconciliation-and-merge-result-apply.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git fixture).

## 변경 파일 목록

- `crates/ops/src/merge_apply/target.rs`(신규) — `ResultTarget`(`CurrentWorktree`·`ExistingWorktree`·`NewBranchWorktree`),
  `ValidatedTarget`, `TargetError`, `validate_target`.
- `crates/ops/src/merge_apply.rs`(mod 등록), `merge_apply/tests.rs`, `tests/fixture.rs`(신규, 시스템 Git fixture),
  `tests/target.rs`(신규).

## 구현 요약

- Merge Composer 프로젝트의 worktree root(`git rev-parse --show-toplevel`)와 그 안의 프로젝트 상대 경로를 구해, target
  프로젝트 root = target worktree + 같은 상대 경로로 정합니다.
- `CurrentWorktree`·`ExistingWorktree`: 프로젝트가 열리고 project id가 같아야 하며, 진행 중 merge·rebase·cherry-pick·revert와
  unmerged 경로가 없어야 합니다. `ExistingWorktree`는 `git worktree list`에 있고 bare·prunable이 아니어야 합니다. 검증 시점의
  프로젝트 revision을 기록합니다(쓰기 때 다시 비교).
- `NewBranchWorktree`: branch 이름 형식(`is_valid_ref_body`)·미존재, 경로는 절대 경로·`..`·`.` 없음·미존재·부모 있음·기존 worktree
  밖, 시작 commit이 있고 그 commit tree에 프로젝트 `stellar.yaml`·`design/current`가 있어야 합니다.
- 검증은 읽기 명령(`rev-parse`·`worktree list`·`ls-tree`·`ls-files -u`)만 실행하고 아무것도 만들지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib merge_apply` | 통과(target 1, document 4) |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사(target validation) | 결과 |
|---|---|
| 잘못된 branch 이름(`a..b`) → `InvalidBranch`, 있는 branch(`a`) → `BranchExists` | 통과 |
| 부모 없는 경로·상대 경로 → `InvalidPath`, 있는 경로 → `PathExists`, worktree 안 경로 → `InsideWorktree` | 통과 |
| 없는 시작 commit → `StartNotFound`, 프로젝트가 없는 commit → `StartWithoutProject` | 통과 |
| worktree가 아닌 디렉터리 → `NotAWorktree` | 통과 |
| 모든 실패 뒤 새 경로·branch 없음 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 생성은 XXIII.3.2, 쓰기는 XXIII.3.3입니다.

## 남은 위험 / 한계

- target 검증 뒤 쓰기 전 target이 바뀌는 경우는 쓰기 단계(XXIII.3.3)의 잠금 안 revision 비교가 막습니다.
- CLI·MCP 명령과 UI는 아직 없습니다(엔진 API만).
