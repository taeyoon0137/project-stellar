# 작업 보고서: XVIII.2.2 사용자 기본 셸과 login/non-login 설정

- WBS: XVIII.2.2
- 작업명: 사용자 기본 shell과 login/non-login profile을 설정에서 선택
- 상태: 완료 (`[x]`)
- 선행: XVIII.2.1

## 필수 실행 환경

- `DEV-MAC` (macOS 계정 데이터베이스·실제 PTY, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/shell.rs`(신규) — `ShellSettings`, `ShellChoice`, `ShellMode`, `ResolvedShell`, `resolve_shell`, `UserShellSource`, `SystemShellSource`(macOS `getpwuid_r`), `ShellError`, `ShellPathProblem`.
- `crates/terminal/src/registry.rs` — `LaunchEnvironment`가 셸 설정·후보 공급자를 받고, `DefaultShell` profile을 시작할 때 셸을 해석(`StartError::Shell`), `argv[0]`·`SHELL` 설정.
- `crates/terminal/src/lib.rs` — `shell` 모듈.
- `docs/task-reports/XVIII.2.2.md`(신규), 계획서(XVIII.2.2 `[ ]`→`[x]`).

## 구현 요약

- **설정 값**: `ShellSettings { choice, mode }`는 앱 사용자 설정 값입니다(프로젝트 파일에 넣지 않음).
  기본값은 사용자 기본 셸 + login(macOS 터미널 관례).
- **사용자 기본 셸**: 계정 데이터베이스의 로그인 셸(`getpwuid_r(getuid())`의 `pw_shell`, buffer는
  `ERANGE`면 4배씩 최대 64 KiB)을 먼저 쓰고, 없거나 쓸 수 없으면 앱 환경의 `SHELL`을 씁니다. 둘 다
  쓸 수 없으면 `ShellError::NoDefaultShell`. 후보 공급자는 `UserShellSource` trait로 분리해 테스트에서
  바꿉니다.
- **경로 검증(shell path 없음 오류 처리)**: 비어 있음·NUL·길이(`Malformed`), 상대 경로
  (`NotAbsolute`), 없음(`NotFound`), 일반 파일 아님(`NotAFile`), 실행 권한 없음(`NotExecutable`)을
  `ShellError::InvalidShell { path, problem }`으로 돌려줍니다. registry는 이 경우 실행기를 호출하지
  않고 세션을 `NotStarted`로 둡니다.
- **login/non-login**: login이면 `argv[0]` = `-` + 파일 이름(`-zsh`), non-login이면 파일 이름. 셸
  문자열·`-c`·`login(1)`을 쓰지 않습니다. profile이 `SHELL`을 명시하지 않았으면 실제 실행 셸 경로를
  `SHELL`로 넣습니다. argv profile(`ProfileCommand::Argv`)은 셸 설정과 무관합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 44 passed, 0 failed |
| `cargo test -p stellar-terminal` 15회 반복 | 실패 0 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 없는 경로·상대·NUL·디렉터리·실행 권한 없음 → 각 `InvalidShell` 이유(`missing_or_unusable_custom_shell_is_reported`) | 통과 |
| 계정 셸 우선, 실패 시 `SHELL`, 둘 다 실패 `NoDefaultShell`(`user_default_prefers_account_shell_then_env_shell`) | 통과 |
| registry: 없는 셸·기본 셸 없음 → `StartError::Shell`, 실행기 호출 0·상태 `NotStarted`; argv profile 시작, 명시 `SHELL` 유지(`missing_shell_is_reported_without_spawning`) | 통과 |
| `argv[0]` `-sh`/`sh`, 기본 login(`login_mode_prefixes_argv0_with_dash`) | 통과 |
| 실제 PTY: login `$0`=`-sh`, non-login `$0`=`sh`, `SHELL`=`/bin/sh`, exit 7(`login_and_non_login_shells_run_in_pty`) | 통과 |
| 이 계정의 로그인 셸을 `getpwuid_r`로 읽음(`system_source_reads_account_login_shell`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 설정 UI와 설정 저장 위치는 앱 셸 범위입니다(이 crate는 설정 값 타입과 해석만 제공).

## 남은 위험 / 한계

- `/etc/shells` 등록 여부는 검사하지 않습니다(사용자가 지정한 실행 파일을 그대로 존중).
- 검증과 exec 사이에 셸 파일이 사라지면 `StartFailed{Spawn}`으로 보고됩니다.
