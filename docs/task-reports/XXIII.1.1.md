# 작업 보고서: XXIII.1.1 Merge Composer SourceRef

- WBS: XXIII.1.1
- 작업명: SourceRef에 branch, commit, tag, worktree, HEAD, Index, WorkingTree를 정의
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 수준 선행, 완화 적용 — Phase XIV에는 미완료 행이 남아 있으나 이 작업에 필요한 코드
  (snapshot 원천 XIV.2.3, `rev-parse --verify` 해석 XIV.2.4, worktree 목록 XXVI.3.1, Git fixture
  harness XIV.14.1)는 존재합니다)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/composer/mod.rs`(신규) — Merge Composer 모듈.
- `crates/git-porcelain/src/composer/source.rs`(신규) — `SourceRef`, `WorktreeState`,
  `ResolvedSource`, `SourceError`, `resolve_source`.
- `crates/git-porcelain/src/snapshot/tree.rs` — `verify_rev`를 crate 내부 공유(`pub(crate)`).
- `crates/git-porcelain/src/lib.rs` — `composer` 모듈 등록.
- `docs/task-reports/XXIII.1.1.md`(신규), 계획서(XXIII.1.1 `[ ]`→`[x]`).

## 구현 요약

- **SourceRef** 일곱 종류: `Branch(name)`, `Commit(oid)`, `Tag(name)`, `Worktree { path, state }`
  (`state` = `Head`·`Index`·`WorkingTree`), `Head`, `Index`, `WorkingTree`(뒤의 셋은 현재 worktree).
  `Display`로 `branch:main`·`tag:v1`·`worktree:<path>:index` 같은 미리보기 이름을 줍니다.
- **해석**(`resolve_source`)은 읽는 시점의 고정 값 `ResolvedSource { source, worktree, snapshot, commit }`
  을 만듭니다.
  - branch·tag는 완전한 ref(`refs/heads/<name>^{commit}`, `refs/tags/<name>^{commit}`)로만 찾아 같은
    이름의 branch·tag가 섞이지 않습니다. annotated tag는 commit까지 peel하고, tree를 가리키는 tag는
    `NotACommit`입니다. 이름은 Git 실행 전에 ref 안전 규칙으로 검증합니다(`InvalidName`, spawn 0).
  - commit은 `<oid>^{commit}`가 같은 ID여야 합니다(없는 ID·tree ID는 `NotFound`).
  - HEAD·branch·tag는 해석 시점 commit으로 고정된 `SnapshotSource::Commit`입니다. unborn HEAD는
    `UnbornHead`입니다.
  - Index·WorkingTree는 `SnapshotSource::Index`·`WorkingTree`이며 이력 기준 `commit`은 그 worktree HEAD
    (unborn이면 `None`)입니다.
  - Worktree source는 `git worktree list`에 있는(bare·prunable 제외) 경로만 받고 실경로로 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 272 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **source resolution fixture**(XIV.14.1 표준 scenario): branch main·feature, annotated tag v1(→ main), lightweight tag, branch와 같은 이름의 tag(`feature`)가 있어도 branch는 refs/heads로 해석, commit, HEAD → 고정 commit / Index·WorkingTree → 현재 worktree·HEAD commit / linked worktree의 Head·Index·WorkingTree → 그 worktree·feature commit / 현재 worktree를 Worktree source로 지정 | 통과 |
| 해석 결과 고정: 해석 후 main이 움직여도 이전 결과의 commit 불변, 다시 해석하면 새 commit | 통과 |
| 실패: 없는 branch·tag `NotFound`, tree tag `NotACommit`, 없는 commit·tree ID `NotFound`, worktree가 아닌 경로·없는 경로 `NotAWorktree` | 통과 |
| unborn: Head `UnbornHead`, Index·WorkingTree는 `commit: None` | 통과 |
| 잘못된 ref 이름 7종 × branch·tag `InvalidName`, spawn 0 | 통과 |
| `Display` 이름 7종 | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase XIV`는 phase 수준 선행이라 lane 정책에 따라 완화했습니다(필요 코드 존재 확인).
- worktree source는 경로와 읽을 상태(HEAD·index·작업 트리)를 함께 받습니다. 아키텍처 27.4의
  "worktree"가 그 worktree의 어떤 상태인지 정해져 있지 않아, 현재 worktree의 HEAD·Index·WorkingTree와
  같은 세 상태를 모두 고를 수 있게 했습니다.

## 남은 위험 / 한계

- 원격 추적 branch(`refs/remotes/...`)는 source 종류에 없습니다(계획 행 범위 밖). 필요하면 commit
  source로 지정합니다.
- 텍스트 spec 해석(`branch:<name>` 등 → `SourceRef`)은 제공하지 않습니다. CLI·agent tool이 필요로 하면
  후속 작업에서 추가합니다.
