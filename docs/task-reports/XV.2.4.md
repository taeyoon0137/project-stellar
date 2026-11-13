# 작업 보고서: XV.2.4 시스템 Git 선택자 해석

- WBS: XV.2.4
- 작업명: version selector resolution을 시스템 Git `ls-remote`, fetch된 refs, commit verification으로 수행(Git 객체 직접 파싱 금지)
- 상태: 완료 (`[x]`)
- 선행: XIV.1.3(`[x]`), XV.1.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 Git 2.54.0, 로컬 검증. 네트워크 미사용 — 로컬 경로 원격).

## 변경 파일 목록

- `crates/dependencies/src/git.rs`(신규) — `GitCommandRunner` trait, `GitCommandOutput`(stderr를
  가리는 `Debug`), `GitInvocationError`, `git_args`.
- `crates/dependencies/src/git_refs.rs`(신규) — `RemoteRef`, `parse_ls_remote`, `select_object`,
  `FetchedRefs` trait, `GitSelectorResolver`(`SelectorResolver` 구현).
- `crates/dependencies/src/test_support.rs`(신규, 테스트 전용) — `TempDir`, hermetic `SystemGit`
  (하위 명령별 호출 기록), `fixture_repo`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.2.4.md`(신규), 계획서(XV.2.4 `[ ]`→`[x]`).

## 구현 요약

- **tag·branch**: `git ls-remote -- <url> refs/heads/* refs/tags/*`(shell 미경유, pattern은 Git이
  해석) 출력을 `<oid>\t<ref>` 줄로 엄격 파싱하고 이름을 정확히 비교합니다(꼬리 일치 금지).
  annotated tag는 peeled 줄 `refs/tags/<t>^{}`의 commit을 우선합니다. 결과는 repository 비교 키별로
  세션 memo합니다.
- **fetch된 refs**: `FetchedRefs::fetched_git_dir`가 해당 repository를 fetch한 bare git dir을
  제공합니다(XV.3.2 mirror cache가 구현).
- **commit 검증**: `git --git-dir <mirror> cat-file -t <oid>` 출력이 `commit`이어야 합니다. 실패
  종료는 `CommitNotFound`, 다른 object 종류는 `NotACommit`입니다. commit 선택자는 ls-remote 없이
  검증만 합니다. oid는 hex 검증을 통과한 값이라 인자 주입이 불가능합니다.
- 오류는 작업 이름·종료 코드만 담고 stderr(URL 포함 가능)는 담지 않습니다. 실제 실행기는 project가
  git-porcelain `GitRunner`로 이 trait을 구현합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 26 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-dependencies git_refs -- --nocapture` | 4 passed(시스템 Git 건너뜀 메시지 없음 — 실제 실행) |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 실제 Git fixture: lightweight tag `v1`→첫 commit, annotated tag `v2`→peeled 두 번째 commit, branch `main`·`feature`, exact commit 해석(`tag_branch_commit_fixture_resolve_with_system_git`) | 통과 |
| 없는 tag·branch → `RefNotFound`, mirror에 없는 commit → `CommitNotFound`, tree를 가리키는 tag → `NotACommit` | 통과 |
| ls-remote는 repository당 1회, commit 선택자는 ls-remote 없이 cat-file 검증만(호출 횟수) | 통과 |
| ls-remote 출력 파서: 정상·peeled·대문자 hex 정규화, 축약 oid·공백 구분·빈 ref·비 UTF-8 거부(`parse_ls_remote_output`) | 통과 |
| peeled 우선·정확한 이름 비교(`select_prefers_peeled_tag_and_exact_names`) | 통과 |
| 없는 저장소 ls-remote 비영 종료 → `GitFailed`, 메시지에 경로 없음, fetch 미호출(`ls_remote_failure_is_reported_without_stderr`) | 통과 |

## 차이(작업 지시와 다른 점)

- 시스템 Git 실행은 이 crate의 `GitCommandRunner` trait을 통합니다(의존 방향상 git-porcelain을
  직접 쓸 수 없음). 테스트는 테스트 전용 hermetic `std::process` 실행기로 실제 Git을 실행했고, 운영
  연결(`GitRunner` adapter)은 project crate 작업입니다.

## 남은 위험 / 한계

- `ls-remote`와 fetch 사이에 원격 ref가 바뀌면 ls-remote가 준 commit이 mirror에 없을 수 있습니다
  (`CommitNotFound`로 실패, 재시도는 호출자 정책).
- 같은 이름의 tag와 branch가 둘 다 있을 때 선택은 선언 종류(tag/branch)를 따르며 모호성 경고는
  없습니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.
