# 작업 보고서: IV.2.2 프로젝트 root 검증기

- WBS: IV.2.2
- 작업명: `stellar.yaml`과 `.git`(디렉터리/gitfile)이 같은 경로에 있는지 검증
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(매니페스트)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/root.rs` (신규: `validate_project_root`, `GitLink`, `RootError`, 임시 디렉터리 fixture 테스트)
- `crates/project/src/lib.rs` (`pub mod root;`)
- `docs/task-reports/IV.2.2.md`(신규), 계획서(IV.2.2 `[ ]`→`[x]`)

## 구현 요약

- `validate_project_root(path)`는 **같은 경로**에 `stellar.yaml`(파일)과 `.git`이 있는지
  확인합니다. `.git`이 디렉터리면 `GitLink::Directory`, gitfile(linked worktree)이면
  `GitLink::GitFile`을 반환합니다.
- 없으면 `MissingManifest`(매니페스트 우선)·`MissingGit`을 반환합니다. **상위로 거슬러
  올라가지 않으므로**, `.git`이 부모에만 있는 하위 폴더는 `MissingGit`입니다.
- 존재·동일 경로만 확인하고 매니페스트 내용은 파싱하지 않습니다(파싱은 IV.5, Git repo지만
  매니페스트 없음 거부는 IV.2.4).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (68 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| 정상 repo(`stellar.yaml` + `.git` 디렉터리) | `Ok(Directory)` |
| linked worktree(`stellar.yaml` + `.git` gitfile) | `Ok(GitFile)` |
| 상위 `.git`만 있는 하위 폴더(`sub/stellar.yaml`) | `Err(MissingGit)` |
| 매니페스트 없음·`.git`만 | `Err(MissingManifest)` |
| `.git` 없음·매니페스트만 | `Err(MissingGit)` |
| 빈 디렉터리 | `Err(MissingManifest)` |
| `stellar.yaml`이 디렉터리 | `Err(MissingManifest)` |

## 차이(작업 지시와 다른 점)

- 없음. 테스트는 시스템 임시 디렉터리에 fixture 구조를 만들고 드롭 시 정리하는 `TempDir`
  가드를 사용합니다(외부 의존성 없이 std만 사용).

## 남은 위험 / 한계

- gitfile 내용(`gitdir:` 경로)의 유효성은 검사하지 않습니다(존재·종류만 판정). 실제 worktree
  연결 검증은 `git-porcelain`(Phase IV.3 이후) 소관입니다.
- 매니페스트 파싱·신뢰성은 IV.2.4·IV.5에서 다룹니다.
