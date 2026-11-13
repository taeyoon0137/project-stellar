# 작업 보고서: XIII.7.1 Git LFS 설치 상태·버전 감지

- WBS: XIII.7.1
- 작업명: `git lfs version`을 시스템 Git 환경에서 실행해 설치 상태와 버전을 읽음
- 상태: 완료 (`[x]`)
- 선행: IV.3.4(Git 명령 결과)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 프로세스 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/lfs.rs` (신규: `LfsInstallation`, `LfsVersion`, `parse_lfs_version_output`, `classify_lfs_output`, `probe_lfs`, 테스트)
- `crates/git-porcelain/src/capability.rs` (`lfs_available_from_output`가 `classify_lfs_output`과 같은 기준을 쓰도록 위임; XIV.1.3 테스트 변경 없음)
- `crates/git-porcelain/src/lib.rs` (`pub mod lfs;`)
- `docs/task-reports/XIII.7.1.md`(신규), 계획서(XIII.7.1 `[ ]`→`[x]`)

## 구현 요약

- `probe_lfs(runner)`는 `GitRunner`로 시스템 임시 디렉터리(중립 cwd)에서 `git lfs version`을
  실행합니다. Git이 실제로 찾는 `git-lfs`(exec-path·PATH의 dashed external command)를 쓰므로 LFS
  executable 경로를 직접 추측하지 않습니다. 저장소 안에서 실행하지 않는 이유는 LFS 미설치 시
  저장소 config의 `alias.lfs`가 실행될 수 있기 때문입니다.
- 분류(`classify_lfs_output`): 종료 코드 0 + `git-lfs/<major>.<minor>[.<patch>]` →
  `Installed { version }`(raw 보존), 종료 코드 0이지만 형식 불일치 → `Unrecognized { raw }`,
  비영 종료 → `NotInstalled { exit_code, stderr }`. stderr 문구는 locale에 따라 바뀌므로 해석하지
  않고 진단용으로만 보존합니다. Git executable 자체를 실행하지 못하면 상태가 아니라
  `GitRunError`입니다.
- XIV.1.3 capability의 LFS 판정이 같은 분류 기준을 쓰도록 위임했습니다(판정 기준 단일화).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 96 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| **installed fixture**: 실제 시스템 Git + PATH에 fixture `git-lfs`(출력 `git-lfs/9.8.7 (fixture; test)`)만 둠 → `Installed { 9.8.7, raw 보존 }` | 통과 |
| **not installed fixture**: 실제 시스템 Git + 빈 PATH 디렉터리 → `NotInstalled`(비영 종료, stderr 보존), spawn 1 | 통과 |
| 위 두 실제 Git 테스트가 건너뛰지 않고 실행됨을 임시 출력으로 확인(확인 후 원복) | 확인 |
| 가짜 Git executable 2종(설치 3.4.1 / `'lfs' is not a git command` exit 1) → `Installed(3,4,1)` / `NotInstalled{1, stderr}` | 통과 |
| 출력 parser: macOS·Windows 형식, patch 생략(`3.5` → 3.5.0), 잘못된 형식 8종 거부 | 통과 |
| 종료 0 + 형식 불일치 → `Unrecognized`, Git executable 없음 → `GitRunError::Spawn` | 통과 |
| XIV.1.3 capability LFS fixture 테스트 그대로 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- "미설치"를 stderr 문구가 아니라 `git lfs version`의 비영 종료로 판정합니다. 설치되어 있어도
  실행에 실패하는 LFS는 사용할 수 없으므로 같은 상태로 묶고 종료 코드·stderr를 보존합니다.
- Git exec-path에 `git-lfs`가 함께 들어 있는 배포판에서는 PATH만으로 미설치·fixture 상태를 만들 수
  없으므로 실제 Git 기반 두 테스트를 건너뛰게 했습니다. 이 환경(Apple Git 2.54.0)에서는 건너뛰지
  않고 실행되었습니다.

## 남은 위험 / 한계

- LFS 권고 정책(XIII.7.2), 승인 후 `git lfs track`(XIII.7.3), history migration 거부(XIII.7.4),
  missing LFS object 진단(XIII.7.5)은 후속 작업입니다.
- Windows(Git for Windows 번들 LFS) 환경은 이 환경에서 실행하지 않았습니다.
