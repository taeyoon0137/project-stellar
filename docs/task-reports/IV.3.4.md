# 작업 보고서: IV.3.4 GitCommandResult

- WBS: IV.3.4
- 작업명: exit code·stdout·stderr를 보존한 `GitCommandResult`(비영 종료 자동 성공 위장 금지)
- 상태: 완료 (`[x]`)
- 선행: IV.3.3(GitRunner)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/command.rs` (신규: `GitCommandResult`, `GitCommandFailure`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod command;`)
- `docs/task-reports/IV.3.4.md`(신규), 계획서(IV.3.4 `[ ]`→`[x]`)

## 구현 요약

- `GitCommandResult`는 IV.3.3의 `GitOutput`(원시 캡처)을 감싸 **exit code·stdout·stderr를 모두
  보존**하고 의미 계층을 더합니다. 접근자(`exit_code`/`stdout`/`stderr`/`*_lossy`)와
  `succeeded()`(코드 0만 성공)를 제공합니다.
- **비영 종료를 자동 성공으로 바꾸지 않습니다**: 성공이 필요하면 `require_success()`로 명시적
  처리해야 하며, 실패 시 보존된 코드·stdout·stderr를 담은 `GitCommandFailure`(에러 타입)를
  반환합니다. 시그널 종료(code `None`)도 성공이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (22 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 모든 필드(코드·stdout·stderr·lossy) 보존 | 통과 |
| 비영 종료가 자동 성공으로 바뀌지 않음(`require_success` → 실패) | 통과 |
| 코드 0만 성공, 시그널 종료(None)는 실패 | 통과 |
| **known failing git command**(비-repo `git rev-parse --verify HEAD`) 실패로 보존 | 통과 |

## 차이(작업 지시와 다른 점)

- `GitOutput`(IV.3.3 원시 캡처)을 폐기하지 않고 `GitCommandResult`로 감싸 의미 계층을
  분리했습니다(원시 vs 명령 결과). 중복 필드 없이 위임합니다.

## 남은 위험 / 한계

- `GitInstallationState`(Available/Missing/UnsupportedVersion/…)와 상태 전이는 IV.3.5에서
  추가합니다.
- known failing 테스트는 git 설치 환경을 가정하며, 미설치 시 건너뜁니다(데이터 기반 단위
  테스트는 항상 실행).
