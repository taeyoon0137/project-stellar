# 작업 보고서: XVIII.5.1 외부 CLI를 일반 terminal profile 명령으로 실행

- WBS: XVIII.5.1
- 작업명: Claude Code, Codex 등을 특별 파서 없이 일반 terminal profile command로 실행
- 상태: 완료 (`[x]`)
- 선행: XVIII.4.5 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + macOS PTY 통합 테스트, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/program.rs`(신규) — `find_program`, `ProgramError`, `MAX_PATH_ENTRIES`, 임의 CLI smoke.
- `crates/terminal/src/profile.rs` — `TerminalProfile::command`, `ProfileCommand::Argv` 문서.
- `crates/terminal/src/header.rs` — `HeaderCommand`(셸 세션·명령 세션 구분), `TerminalHeader::command`, `build` 인자.
- `crates/terminal/src/registry.rs` — header에 profile 명령 이름 전달.
- `crates/terminal/src/shell.rs` — 실행 파일 검사(`check_shell_path`)를 crate 안에서 함께 쓰도록 공개 범위 확대.
- `crates/terminal/src/lib.rs` — `program` 모듈.
- `crates/terminal/tests/no_cli_specific_code.rs`(신규) — CLI 전용 이름 inventory 검사.
- `docs/task-reports/XVIII.5.1.md`(신규), 계획서(XVIII.5.1 `[ ]`→`[x]`).

## 구현 요약

- **일반 profile 명령**: 외부 CLI 에이전트도 셸이나 다른 명령과 같은 `ProfileCommand::Argv` profile
  (`TerminalProfile::command`)로 실행합니다. 시작 경로는 신뢰 gate·cwd·환경 허용 목록·`TERM`
  기본값·PTY 실행까지 다른 명령과 같고, 출력은 emulator 화면으로만 해석됩니다. CLI를 알아보는 분기,
  전용 실행 경로, 출력 parser가 없습니다.
- **실행 파일 찾기**(`find_program`): 사용자가 입력한 명령 이름을 child가 받을 `PATH` 값에서 찾아
  절대 경로로 바꿉니다. profile에는 그 절대 경로를 저장하므로 실행할 때 `PATH` 검색으로 다른 파일이
  선택되지 않습니다(XVIII.1.1 `ArgvCommand` 규칙 유지). 빈 `PATH` 항목·상대 경로 항목(현재 디렉터리
  검색)은 건너뛰고, 실행 권한이 없는 같은 이름은 건너뛰며, `/`가 있는 상대 경로·빈 이름·NUL은 받지
  않습니다. 인자는 argv 배열 그대로입니다.
- **header 구분**(XVIII.4.5 연결): `HeaderCommand::Shell`과 `HeaderCommand::Program{name}`(실행 파일
  이름)으로 일반 셸 세션과 명령 세션을 나눠 보여 줄 수 있게 했습니다(기술 아키텍처 32 "일반 세션과 외부
  에이전트 세션 구분 가능"). 이름만 보여 주고 종류를 해석하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | unit 114 passed, 0 failed / integration `no_cli_specific_code` 2 passed / doctest 2 passed |
| `cargo test -p stellar-terminal` 10회 반복 | 실패 0(장비 부하 load average 약 100에서 8.6~17.6 s), 반복 뒤 `pgrep -fl "sleep 300"` 결과 없음 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |
| `./scripts/macos/smoke.sh` | 실행하지 않음(앱 셸 변경 없음, 터미널 view는 XVIII.4.3) |

| 검사 | 결과 |
|---|---|
| **arbitrary CLI command smoke**: 가짜 외부 CLI(`agent-cli`: raw mode 입력, 대체 화면, 색, `tput`)를 `PATH`에서 찾아 일반 profile로 실행 — argv 원문(`two words`) 전달, child `TERM=xterm-256color`, raw 한 글자 입력, 종료 코드 4·`ChildExited`·남은 프로세스 0, emulator가 대체 화면을 나온 주 화면 결과(`key=y cols=100`)를 보임, header `Program{agent-cli}`; 시스템 명령 `echo`·`env`·`tput lines`·`ls`도 같은 경로로 실행·종료 0·화면 확인(`arbitrary_cli_commands_run_as_plain_profiles`) | 통과 |
| 이름→절대 경로, 실행 권한 없는 같은 이름 건너뜀, 상대 `PATH` 항목 무시, 절대 경로 검사, 못 찾음, 잘못된 이름(`names_are_resolved_through_path_to_absolute_programs`) | 통과 |
| `TerminalProfile::command`는 일반 argv profile(`command_profile_is_a_plain_argv_profile`), header 명령 이름(`header_command_names_the_program_without_interpreting_it`) | 통과 |
| terminal crate 소스·생성기·manifest에 CLI 전용 이름 0건, 위반 fixture는 검출(`terminal_crate_has_no_cli_specific_code`, `detector_finds_cli_specific_names`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 공급자 CLI(설치·로그인이 필요한 외부 프로그램)는 테스트에서 실행하지 않았습니다. 같은 실행
  경로를 쓰는 가짜 CLI와 시스템 명령으로 "arbitrary CLI command smoke"를 확인했습니다. 실제 CLI는
  설치 여부와 사용자 자격 증명에 의존하고, Stellar는 자격 증명을 다루지 않기 때문입니다(기술 아키텍처
  31.8, 37.3).
- 작업 행에 없던 header 명령 구분(`HeaderCommand`)을 넣었습니다. 선행 XVIII.4.5 header data model에
  명령 세션 구분을 더한 것입니다.

## 남은 위험 / 한계

- inventory 검사는 terminal crate 안만 봅니다. workspace 전체 출력 표시 격리(셸·FFI 포함)는
  XXIX.6.6에서 검사합니다.
- profile 생성 UI(명령 이름 입력, 찾은 경로 표시)와 터미널 view는 XVIII.4.3·앱 셸 작업입니다.
- CLI가 직접 파일을 수정하는 경로의 UI 경고는 XIX.8.2입니다.
