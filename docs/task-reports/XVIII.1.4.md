# 작업 보고서: XVIII.1.4 신뢰된 프로젝트에서만 터미널 시작

- WBS: XVIII.1.4
- 작업명: terminal은 trusted project에서만 시작되게 capability check 연결
- 상태: 완료 (`[x]`)
- 선행: XVI.7.1, XVIII.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/capability.rs`(신규) — `TerminalTrust`, `TerminalCapability`, `TerminalNotTrusted`.
- `crates/terminal/src/launch.rs`(신규) — `LaunchRequest`, `TerminalLauncher`·`TerminalProcess` trait, `ExitReport`, `TerminalIoError`, `LaunchError`.
- `crates/terminal/src/registry.rs` — `start`(시작 gate), `StartError`, `LaunchEnvironment`, `process_env`, 세션의 cwd·process 보관, `close`의 child 종료, `RegistryError::Terminate`.
- `crates/terminal/src/lib.rs` — 모듈 추가.
- `docs/task-reports/XVIII.1.4.md`(신규), 계획서(XVIII.1.4 `[ ]`→`[x]`).

## 구현 요약

- **capability token**: 신뢰 상태의 원본은 project crate의 capability matrix(XVI.7.1,
  `ProjectCapability::Terminal`: 비신뢰 `denied`·신뢰 `allowed`)입니다. terminal crate는 project에
  의존할 수 없으므로(ADR-0019) 호출자가 matrix 판정을 `TerminalTrust`로 바꿔 공급하고,
  `TerminalCapability::grant(window, trust)`는 `Trusted`일 때만 token을 만듭니다. token은
  프로젝트 창(`WindowSessionId`) 하나에 묶이고, `Clone`·`Copy`가 아니며 `start` 한 번에 **소비**되므로
  신뢰 철회 뒤 예전 판정을 재사용할 수 없습니다.
- **시작 gate**: `TerminalRegistry::start(id, capability, environment, launcher)`의 검사 순서는
  capability 창 일치 → 세션 존재·상태(`NotStarted`/`StartFailed`만) → cwd 검증(XVIII.1.3) → 명령
  (`DefaultShell`은 창의 셸 명령, `Argv`는 profile argv)·환경(허용 목록 적용) 구성 → 실행기 호출입니다.
  capability가 없거나(비신뢰) 다른 창의 것이면 cwd·세션도 보지 않고 실행기를 호출하지 않습니다.
- **실행기 경계**: 실행은 `TerminalLauncher` trait 뒤에 둡니다(macOS PTY 실행기는 XVIII.2.1).
  실행 결과 `TerminalProcess`(pid, 비차단 종료 확인, 종료 요청, 종료)는 세션에 붙고, 실행 실패는
  `StartFailed{Pty|Spawn}`으로 기록되어 다시 시작할 수 있습니다. `close`는 실행 중 child를 종료한 뒤
  `Finished{Closed}`를 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 31 passed, 0 failed |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 비신뢰 시작 100회 모두 `NotTrusted`, 다른 창 token은 `CapabilityWindowMismatch`(세션 존재 검사보다 먼저), 실행기 호출 0·요청 0·상태 `NotStarted`(`untrusted_projects_never_spawn`) — **untrusted spawn 0** | 통과 |
| 신뢰 시작의 요청 구성(argv 원문·worktree cwd·허용 목록 환경, 비허용 변수 제외), 중복 시작 거부, close 시 종료·drop(`trusted_start_builds_request_and_close_terminates_child`) | 통과 |
| cwd 실패 시 실행기 미호출, 실행 실패 `StartFailed` 후 재시작(`start_failures_do_not_leave_processes`) | 통과 |
| 20 session start/close: 실행 20·종료 20·drop 20, registry 비움(`twenty_started_sessions_close_without_leaking_processes`) | 통과 |
| token은 신뢰일 때만 발급(`capability_is_granted_only_for_trusted_projects`) | 통과 |

## 차이(작업 지시와 다른 점)

- project crate의 matrix 판정을 `TerminalTrust`로 바꾸는 연결 함수(git의 `git_trust`에 해당)는 project
  crate 쪽 변경이라 이 작업 범위(terminal crate)에서 만들지 않았습니다. 앱 셸이
  `check_capability(ProjectCapability::Terminal, trust)` 결과로 `TerminalTrust`를 정하면 됩니다.

## 남은 위험 / 한계

- 이미 실행 중인 세션은 신뢰를 철회해도 자동으로 끝나지 않습니다. 철회 시 앱 셸이
  `close_all`(XVIII.2.4)로 정리해야 합니다.
