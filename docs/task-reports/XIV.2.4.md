# 작업 보고서: XIV.2.4 HEAD/commit tree provider

- WBS: XIV.2.4
- 작업명: HEAD/commit tree의 파일 목록과 mode/blob ID를 `git ls-tree -rz`로 읽는 provider
- 상태: 완료 (`[x]`)
- 선행: XIV.2.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/snapshot/tree.rs` (신규: `resolve_tree`, `read_tree_snapshot`, `read_tree_entries`, `parse_ls_tree`, `ResolvedTree`, `TreeSnapshot`, 테스트)
- `crates/git-porcelain/src/snapshot/mod.rs` (`SnapshotEntry`, `SnapshotError`, `tree` 모듈)
- `docs/task-reports/XIV.2.4.md`(신규), 계획서(XIV.2.4 `[ ]`→`[x]`)

## 구현 요약

- 원천 해석: `Head`는 `git rev-parse -q --verify HEAD^{commit}`(종료 코드 1 = unborn → 빈 snapshot),
  `Commit(oid)`는 `<oid>^{commit}`, `Tree(oid)`는 `<oid>^{tree}`가 **같은 ID로** 해석되어야 합니다.
  commit ID를 Tree로, blob ID를 Commit으로 쓰면 `ObjectNotFound`입니다. `Index`·`WorkingTree`는
  `WrongSourceKind`(Git 실행 0).
- 목록: `git ls-tree -r -z --full-tree <tree>`. 레코드 `<mode> <type> <oid>\t<path>`를 첫 TAB에서
  나눠 경로(원시 바이트, TAB 포함 가능)를 보존하고, mode·종류·ID를 검증합니다. mode와 object 종류가
  맞지 않으면(예: `100644 commit`, `160000 blob`) 해석 오류입니다. gitlink는 commit ID로 보존합니다.
- `--full-tree`로 cwd가 하위 디렉터리여도 root 기준 전체 목록을 받습니다. 모든 실행은 trust gate를
  거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 142 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **symlink·executable·unicode path fixture**: 한글·공백·emoji 경로 일반 파일, 실행 파일(`100755`), symlink(`120000`, blob = 대상 경로 바이트), NFD 한글 경로, non-UTF8 경로 → mode·blob ID가 `hash-object` 기대값과 일치(`symlink_executable_unicode_fixture`) | 통과 |
| `Commit(hash)`·`Tree(hash)` 원천 목록 == HEAD 목록, 하위 디렉터리 cwd에서도 동일 | 통과 |
| unborn HEAD → 빈 목록, commit을 Tree로·blob을 Commit으로·없는 commit → `ObjectNotFound`(`unborn_head_is_empty_and_wrong_ids_are_rejected`) | 통과 |
| 합성 레코드(공백·TAB 경로, gitlink, non-UTF8) 해석과 형식 오류 10종 거부 | 통과 |
| untrusted gate → spawn 0, Index/WorkingTree 원천 → spawn 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- macOS Git은 `core.precomposeUnicode`로 argv의 NFD를 NFC로 바꿉니다. NFD 경로 fixture는 그
  저장소에서만 해당 설정을 꺼서 만들었습니다(provider는 Git이 기록한 바이트를 그대로 보존).
- 대형 tree는 출력 전체를 메모리에 읽습니다(streaming 미구현).
