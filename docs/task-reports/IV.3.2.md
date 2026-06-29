# 작업 보고서: IV.3.2 git --version 버전 파싱

- WBS: IV.3.2
- 작업명: `git --version` 실행·semver 파싱·raw 보존(최소 버전 미강제)
- 상태: 완료 (`[x]`)
- 선행: IV.3.1(Git 탐색)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/version.rs` (신규: `GitVersion`, `VersionProbeError`, `parse_git_version_output`, `probe_git_version`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod version;`)
- `docs/task-reports/IV.3.2.md`(신규), 계획서(IV.3.2 `[ ]`→`[x]`)

## 구현 요약

- `probe_git_version(path)`는 `Command::new(path).arg("--version")`(argv, **shell 미사용**)으로
  실행하고 stdout을 파싱합니다.
- `parse_git_version_output`(순수 함수)은 `git version X.Y.Z ...`에서 major.minor.patch를
  추출합니다(patch 없으면 0, `2.42.0.windows.2` 같은 접미사 허용). 형식 불일치는 `None`.
- `GitVersion`은 파싱 결과 + **raw output을 보존**합니다.
- 실패 분류: 실행 불가 `SpawnFailed{kind}`, 비영 종료 `NonZeroExit{code,stderr}`, 형식 불일치
  `Unparseable{raw}`. **최소 버전은 강제하지 않습니다**(IV.3.5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (10 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| 버전 문자열 파싱(Apple/Windows/patch 생략/공백) | 통과 |
| 비정상 문자열 파싱 거부 | 통과 |
| **Git 정상**(발견된 실제 git probe → Ok, raw 보존) | 통과 |
| **Git 없음**(없는 경로 → `SpawnFailed`) | 통과 |
| **잘못된 executable**(`/bin/echo` → `Unparseable`, raw 보존) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 파싱·raw 보존·세 fixture 처리를 충족하며 최소 버전은 강제하지 않았습니다.

## 남은 위험 / 한계

- 정상 probe 테스트는 git이 설치된 환경(DEV-MAC/CI-MAC)을 가정합니다. git 미설치 환경에서는
  발견 실패로 해당 분기를 건너뜁니다(없음·비-git 케이스는 항상 실행).
- 모든 Git 실행을 감싸는 보안 `GitRunner`(env allowlist·timeout·byte capture)는 IV.3.3에서
  추가합니다.
