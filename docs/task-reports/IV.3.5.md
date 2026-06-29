# 작업 보고서: IV.3.5 GitInstallationState

- WBS: IV.3.5
- 작업명: Git 설치 상태(5종)와 이벤트 기반 전이
- 상태: 완료 (`[x]`)
- 선행: IV.3.2(버전 probe)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/install_state.rs` (신규: `GitInstallationState`, `GitInstallEvent`, `TransitionError`, 전이·fixture)
- `crates/git-porcelain/src/lib.rs` (`pub mod install_state;`)
- `docs/task-reports/IV.3.5.md`(신규), 계획서(IV.3.5 `[ ]`→`[x]`)

## 구현 요약

- `GitInstallationState`: `Available{version}`, `Missing`, `UnsupportedVersion{version}`,
  `InstallInProgress`, `ReprobeRequired`.
- `apply(event)` 전이:
  - probe(발견 `ProbeFound{version, supported}` / 미발견 `ProbeMissing`)는 **어느 상태에서나**
    현실을 반영(Available/UnsupportedVersion/Missing).
  - `RequestInstall`은 `Missing`·`UnsupportedVersion`에서만 `InstallInProgress`로(그 외
    `CannotInstallFrom`).
  - `InstallCompleted`는 `InstallInProgress`에서만 `ReprobeRequired`로(그 외
    `NoInstallInProgress`).
  - 허용되지 않는 전이는 `Err`이며 상태는 보존됩니다.
- 지원 버전 여부(`supported`)는 호출자가 결정합니다(최소 버전 정책은 미강제, IV.3.2와 일관).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (전체) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| state transition fixture | 결과 |
|---|---|
| 설치 happy path(Missing→요청→진행→완료→재probe→Available) | 통과 |
| probe가 어느 상태에서나 현실 반영(Available/Unsupported/Missing) | 통과 |
| UnsupportedVersion에서 설치 요청 허용 | 통과 |
| Available·InstallInProgress에서 설치 요청 거부 | 통과 |
| InstallCompleted는 InstallInProgress에서만 허용 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 5개 상태와 전이를 정의하고 fixture로 검증했습니다.

## 남은 위험 / 한계

- 플랫폼별 설치 어댑터(macOS `xcode-select`, Windows `winget`)와 설치 후 재탐지는
  IV.3.6~IV.3.8에서 추가합니다. 본 작업은 상태·전이 정의에 한정합니다.
