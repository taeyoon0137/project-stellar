# 작업 보고서: XVIII.2.5 대화형 터미널 child 환경을 ADR-0023에 맞춤

## WBS

- WBS: XVIII.2.5
- 작업명: 대화형 terminal session의 child 환경(앱 환경 상속 + 식별 변수·`SHELL`·profile 명시 지정)
- 상태: 완료 (`[x]`) 후보 — 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XVIII.1.4, XVIII.2.2
- 결정: [ADR-0023](../adr/ADR-0023-integrated-terminal-user-shell.md), [ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 3
- lane: `recovery-git-terminal`

## 변경 파일

- `crates/terminal/src/env.rs`: 상속 허용 목록 삭제, `EnvironmentPolicy`는 profile 명시 지정만, `TerminalIdentity`,
  `EnvChange`, `interactive_env`, `apply_env_changes`, `STELLAR_SET_VARIABLES`, `TERM_PROGRAM_NAME`
- `crates/terminal/src/launch.rs`: `LaunchRequest`가 상속 환경에 얹을 변경 목록(`env_changes`)을 담음
- `crates/terminal/src/registry.rs`: `LaunchEnvironment.identity`(spawn 입력), 변경 목록 구성, `process_env`·`set_env` 삭제
- `crates/terminal/src/pty/mod.rs`: `env_clear` 삭제, 변경(설정·삭제)만 적용
- `crates/terminal/src/profile.rs`: 기본 profile은 명시 지정 없음
- `crates/terminal/src/program.rs`, `crates/terminal/tests/worktree_type_inventory.rs`: 새 입력으로 시험 갱신
- `crates/terminal/src/lib.rs`: 모듈 설명
- `crates/terminal/tests/interactive_env.rs`(신규)

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 0 — lib 115, `interactive_env` 3, `no_cli_specific_code` 2, `scrollback_heap_measurement` 1, `worktree_type_inventory` 5, doctest 2 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-terminal -- --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬, 실제 POSIX PTY와 `/bin/bash` login 셸(계정 로그인 셸은 zsh).

## 테스트

| 검증 열 항목 | 테스트 | 결과 |
|---|---|---|
| 임시 HOME login 셸의 rc·profile이 더한 `PATH` 항목과 marker 변수가 child에 보임 | `interactive_env::login_shell_reads_user_profile_and_inherits_app_environment` | 통과 — `.bash_profile`의 `$HOME/fixture-bin`과 `STELLAR_PROFILE_MARKER`, `.bashrc`의 `STELLAR_RC_MARKER` |
| 앱 환경에만 있는 옛 허용 목록 밖 변수가 child에 보임 | 같은 시험, `pty::tests::child_inherits_app_environment_with_requested_changes_and_cwd` | 통과 — `CARGO_MANIFEST_DIR`(cargo가 시험 프로세스에 넣음)이 같은 값, 삭제 변경은 빠짐 |
| 식별 변수 4개가 spawn 입력 값과 일치 | 같은 login 시험 | 통과 — `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=Stellar`, `TERM_PROGRAM_VERSION=9.8.7-fixture` |
| 계정 로그인 셸과 다른 셸을 고른 기본 셸 profile에서 `SHELL` = 실행한 셸 | 같은 login 시험 | 통과 — `/bin/bash` |
| argv profile에서 `SHELL` = 상속값 | `interactive_env::argv_profile_keeps_inherited_shell_and_stellar_adds_only_identity_variables` | 통과 |
| Stellar가 더한 변수는 식별 변수 4개와 `SHELL` 외 0 | 같은 argv 시험(실제 child 환경과 시험 프로세스 환경 비교), `env::tests::stellar_adds_only_identity_and_shell_variables` | 통과 — 바뀐·더한 이름이 허용 집합 안, 빠진 앱 변수 0 |
| profile 명시 지정 우선 | 같은 argv 시험, `env::tests::default_shell_sets_shell_and_profile_assignments_win_last` | 통과 — `TERM_PROGRAM=profile-wins`, `SHELL=/custom/shell` |
| 세션 전후 rc·profile 파일 hash 동일 | login 시험 | 통과 — `.bash_profile`·`.bashrc` bytes 동일 |
| 대화형 spawn 경로의 `env_clear`·상속 허용 목록 사용 inventory 0 | `interactive_env::interactive_spawn_path_has_no_env_clear_or_inherited_allowlist` | 통과 — env·launch·profile·registry·pty 소스(주석 제외)에 `env_clear(`·`DEFAULT_INHERITED_ENV`·`with_defaults(`·`resolve_from_process(`·`.inherit(` 0 |
| 24-bit 색 미지원 identity는 `COLORTERM` 상속값을 지움 | `env::tests::colorterm_is_removed_without_truecolor_support` | 통과 |

## 성능

- 해당 없음. 환경을 지우고 다시 만드는 단계가 없어 spawn 비용이 늘지 않습니다.

## 차이

- 식별 변수 값은 spawn 입력(`LaunchEnvironment.identity`)으로 받습니다. 앱 버전은 셸이 넘길 값이라 시험에서는 고정
  문자열을 썼습니다.
- macOS SIP는 보호된 시스템 실행 파일을 exec할 때 `DYLD_*` 변수를 지웁니다. argv 시험의 "빠진 앱 변수 0" 비교에서 이 OS
  동작만 제외했습니다.

## 남은 위험

- `stellar-terminal`을 쓰는 앱 runtime 연결(터미널 패널)은 아직 없습니다. 연결할 때 `TerminalIdentity::stellar`에 앱 버전을
  넘겨야 합니다.
- 사용자 rc 파일이 `TERM`을 바꾸면 emulator와 맞지 않을 수 있습니다(ADR-0023 Consequences).
