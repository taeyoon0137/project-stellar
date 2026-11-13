# 작업 보고서: XIV.1.3 Git capability probe

- WBS: XIV.1.3
- 작업명: porcelain v2, pathspec-from-file, `update-index --index-info`, worktree, LFS availability를 capability로 기록
- 상태: 완료 (`[x]`)
- 선행: IV.3.2(Git 버전 probe)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 가짜 executable fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/capability.rs` (신규: `GitCapability`, `GitCapabilities`, `probe_git_capabilities`, `lfs_available_from_output`, `CapabilityProbeError`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod capability;`)
- `docs/task-reports/XIV.1.3.md`(신규), 계획서(XIV.1.3 `[ ]`→`[x]`)

## 구현 요약

- `GitCapability`는 `StatusPorcelainV2`, `PathspecFromFile`, `UpdateIndexIndexInfo`, `Worktree`,
  `Lfs` 다섯 가지이며, 진단·저장용 안정 이름(`as_str`)을 가집니다. 초기 UI가 쓰지 않는 기능도
  기록만 하고, 사용 여부는 호출자가 결정합니다.
- Git 내장 기능은 `git --version`(IV.3.2 parser)의 버전과 기능 도입 버전표(`min_git_version`)로
  판정합니다: porcelain v2 2.11.0, pathspec-from-file 2.26.0(2.25 일부 명령, 2.26 rm·stash 확장 →
  보수적 기준), `update-index --index-info` 2.0.0(1.x부터 존재 → 보수적 기준), worktree 2.7.0
  (`list --porcelain`). 도움말 문구처럼 locale에 따라 바뀌는 출력은 해석하지 않습니다.
- LFS는 별도 도구이므로 `git lfs version`이 종료 코드 0이고 stdout이 `git-lfs/`로 시작할 때만
  사용 가능으로 기록합니다.
- `probe_git_capabilities(runner)`는 `GitRunner`로 시스템 임시 디렉터리(중립 cwd)에서 두 명령을
  실행합니다. 저장소 안에서 실행하면 LFS 미설치 시 저장소 config의 alias가 실행될 수 있기 때문입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 63 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| **서로 다른 `git --version` fixture 9종**(1.8.3.1, 2.6.7, 2.9.5, 2.11.0, 2.25.4, 2.26.0, Apple 2.39.3, Windows 2.42.0, Apple 2.54.0) → 기대 capability 목록과 정확히 일치 | 통과 |
| 버전 경계(2.10.9/2.11.0, 2.25.9/2.26.0, 2.6.9/2.7.0) 포함 판정 | 통과 |
| **LFS fixture 출력**: macOS·Windows 설치 출력 → 사용 가능, `'lfs' is not a git command` 비영 종료 → 불가, 종료 0이지만 형식 불일치 → 불가; Git 버전과 독립 기록 | 통과 |
| 가짜 Git executable 2종(2.9.5 + LFS 없음, Windows 2.42.0 + LFS 있음)을 실제 실행해 probe → 기대 capability, spawn 2회 | 통과 |
| 실제 시스템 Git probe 결과가 같은 실행기로 직접 실행한 `--version`·`lfs version` 출력 판정과 일치 | 통과 |
| `--version` 비영 종료 → `VersionFailed`, 파싱 불가 → `VersionUnparseable` | 통과 |

## 차이(작업 지시와 다른 점)

- capability는 실제 명령 실행 시험이 아니라 버전표로 판정합니다(LFS만 실행 결과). 도움말 출력처럼
  안정 형식이 아닌 출력을 해석하지 않기 위함입니다.

## 남은 위험 / 한계

- 기능 도입 버전은 공개 릴리스 기준 보수적 값입니다. 버전 문자열을 바꾼 비표준 빌드에서는 실제
  지원 여부와 다를 수 있습니다.
- LFS 버전 상세(설치 상태·버전 parse)는 XIII.7.1에서 다룹니다. 최소 지원 Git 버전 정책은 아직
  강제하지 않습니다.
