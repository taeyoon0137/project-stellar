# 작업 보고서: XIV.1.2 rev-parse 기반 worktree root·Git dir 판별

- WBS: XIV.1.2
- 작업명: `git rev-parse --show-toplevel`, `--git-dir`, `--git-common-dir`, `--is-inside-work-tree`로 worktree root와 gitfile 구조 판별(경로 추측 금지)
- 상태: 완료 (`[x]`)
- 선행: IV.3.4(Git 명령 결과)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54.0, 로컬 검증; 실제 git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/location.rs` (신규: `locate_repository`, `RepositoryLocation`, `WorktreeKind`, `DotGitEntry`, `LocationError`, 출력 parser, 테스트)
- `crates/git-porcelain/src/test_support.rs` (테스트 헬퍼 `commit_file`, `canonical` 추가)
- `crates/git-porcelain/src/lib.rs` (`pub mod location;`)
- `docs/task-reports/XIV.1.2.md`(신규), 계획서(XIV.1.2 `[ ]`→`[x]`)

## 구현 요약

- `locate_repository(gate, cwd)`는 trust gate(XIV.1.1)를 거쳐 `git rev-parse`를 질의별로
  실행합니다: `--is-inside-work-tree` → `--path-format=absolute --show-toplevel` →
  `--path-format=absolute --git-dir` → `--path-format=absolute --git-common-dir`.
- 질의마다 별도 실행하므로 stdout 전체(끝 개행 1개 제외)가 한 값입니다. 개행이 포함된 경로도 줄
  단위로 쪼개지 않고 보존합니다. unix는 원시 바이트를 그대로 경로로, Windows는 UTF-8로 변환합니다.
- 경로 출력은 비어 있지 않은 절대 경로여야 합니다. `--path-format`을 모르는 구버전 Git이 플래그를
  그대로 출력하는 경우도 이 검사에서 `UnexpectedOutput`으로 걸러집니다.
- `WorktreeKind`: Git dir == common Git dir면 `Main`, 다르면 `Linked`. `DotGitEntry`: worktree root의
  `.git`이 디렉터리·gitfile·symlink·없음 중 무엇인지 관찰만 하며, Git dir 위치 계산에는 쓰지
  않습니다(gitfile 내용 직접 해석 없음).
- 오류: `--is-inside-work-tree` 비영 종료는 `NotARepository`(stderr 보존), `false`는
  `NotInsideWorkTree`, untrusted는 `Gate(ProjectNotTrusted)`(spawn 0).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 56 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| **일반 repo**: root·하위 디렉터리 cwd 모두 root·`.git` Git dir·common dir 일치, `Main`, `.git` 디렉터리 | 통과 |
| **linked worktree**(`git worktree add`): root = worktree 경로, common dir = 주 repo `.git`, Git dir ≠ common dir이고 `<common>/worktrees/` 아래, `Linked`, `.git` gitfile; 주 worktree는 `Main` | 통과 |
| 비-repo cwd → `NotARepository`(비영 종료 코드·stderr 보존) | 통과 |
| `.git` 디렉터리 내부 cwd → `NotInsideWorkTree` | 통과 |
| 출력 parser: `true`/`false` 외 거부, 상대 경로·빈 출력·구버전 플래그 echo 거부, 개행 포함 경로 보존 | 통과 |
| untrusted gate → `ProjectNotTrusted`, spawn 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 경로 질의에 `--path-format=absolute`(Git 2.31+)를 함께 씁니다. Git이 절대·canonical 경로를 직접
  출력하게 해 상대 경로 해석을 Stellar가 하지 않기 위함입니다. 구버전 Git은 오류(`UnexpectedOutput`)로
  처리하며 대체 해석을 추측하지 않습니다.

## 남은 위험 / 한계

- 최소 지원 Git 버전 정책이 아직 확정되지 않았습니다. 2.31 미만 Git에서는 위치 판별이 실패합니다.
  capability probe(XIV.1.3)와 함께 최소 버전 정책을 정할 때 반영이 필요합니다.
- Windows linked worktree fixture는 이 환경에서 실행하지 않았습니다(필수 환경은 `DEV-MAC`).
