# 작업 보고서: XVIII.2.1 macOS POSIX PTY 생성과 child spawn wrapper

- WBS: XVIII.2.1
- 작업명: POSIX PTY create와 child process spawn wrapper 구현
- 상태: 완료 (`[x]`)
- 선행: XVIII.1.2

## 필수 실행 환경

- `DEV-MAC` (macOS 실제 PTY·child process, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/pty/mod.rs`(신규, macOS 전용) — `PtyLauncher`, `PtyProcess`, PTY 생성·spawn·출력 drain·종료.
- `crates/terminal/src/pty/sys.rs`(신규) — libSystem 최소 FFI(`posix_openpt`·`grantpt`·`unlockpt`·`ptsname_r`·`open`·`ioctl`·`setsid`·`kill`)와 상수.
- `crates/terminal/src/size.rs`(신규) — `TerminalSize`(1~4096행·열), `SizeError`.
- `crates/terminal/src/launch.rs` — `LaunchRequest`에 초기 크기, `TerminalProcess`에 `write_input`·`output_reader`.
- `crates/terminal/src/registry.rs` — `LaunchEnvironment::size`, 가짜 프로세스의 새 trait 메서드.
- `crates/terminal/src/test_support.rs` — `OutputCollector`(별도 thread 출력 수집, timeout).
- `crates/terminal/src/lib.rs` — `pty`(macOS)·`size` 모듈.
- `docs/task-reports/XVIII.2.1.md`(신규), 계획서(XVIII.2.1 `[ ]`→`[x]`).

## 구현 요약

- **외부 crate 없음**: 새 의존성 없이 libSystem C 함수만 선언했습니다(project watcher의 FSEvents,
  git-porcelain의 `kill` FFI와 같은 방식). 가변 인자 함수(`open`, `ioctl`)는 Apple arm64 호출
  규약 때문에 `...`로 선언했고, `TIOCSCTTY`·`TIOCSWINSZ`·`TIOCOUTQ`·`O_*`·신호 상수를 SDK 헤더 값과
  테스트로 고정했습니다.
- **PTY 생성**: `posix_openpt(O_RDWR|O_NOCTTY|O_CLOEXEC)` → `grantpt` → `unlockpt` → `ptsname_r`
  (NUL 확인은 `CStr::from_bytes_until_nul`) → slave `open(O_RDWR|O_NOCTTY|O_CLOEXEC)` → master에
  초기 `TIOCSWINSZ`. 두 fd 모두 생성 시점부터 close-on-exec입니다(실측으로 `FD_CLOEXEC` 확인).
- **spawn wrapper**: `std::process::Command`에 argv 배열·`arg0`(선택)·검증된 cwd·`env_clear` 후 요청
  환경만, stdin/stdout/stderr = slave 복제. `pre_exec`에서 async-signal-safe 호출만으로 `setsid()`
  (새 세션·process group leader)와 `ioctl(0, TIOCSCTTY)`(controlling terminal)를 합니다.
- **출력 유실 방지(실측 근거)**: child가 slave의 마지막 참조를 닫으면 macOS가 master가 아직 읽지
  않은 출력을 버리는 경우가 있었습니다(`/bin/echo` 400회 실측: 부모가 slave를 닫는 방식 9~12회 유실).
  부모가 slave 하나를 보관하고, leader가 끝나면 `TIOCOUTQ`가 0이 될 때까지(상한 1초) 기다린 뒤 닫는
  방식은 같은 실측(직렬·동시 fork 부하·ctty 유무)에서 유실 0이었습니다. 그래서 `poll_exit`은
  출력 drain까지 끝난 뒤 결과를 돌려주고, master 읽기는 그 뒤 EOF가 됩니다. 사용자가 닫는
  `terminate`는 기다리지 않습니다.
- **종료(이 단계)**: leader process group에 `SIGHUP`+`SIGCONT` → 유예(기본 1.5초) → `SIGKILL` →
  회수. group 신호는 회수 전에만 보냅니다. 세션 전체 정리(다른 process group의 job, leader가 먼저
  끝난 뒤 남은 프로세스)는 XVIII.2.4에서 확장합니다.
- 입력은 `write_input`(master write), 출력은 `output_reader`(master 복제 fd, 별도 thread용)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 38 passed, 0 failed |
| `cargo test -p stellar-terminal` 40회 반복 | 실패 0 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| `/bin/echo` output fixture 20회 반복: 출력 `hello stellar terminal 'quoted' $HOME\r\n` 정확·exit 0(`echo_output_fixture`) | 통과 |
| 요청 환경만 전달(`/usr/bin/env`), cwd(`/bin/pwd`)(`child_gets_only_requested_environment_and_cwd`) | 통과 |
| stdin이 `/dev/ttys*`, `/dev/tty` 열기 성공 = controlling terminal(`child_is_session_leader_with_pty_as_controlling_terminal`) | 통과 |
| 입력 쓰기·echo·종료 `SIGHUP`·재호출 동일·EOF(`interactive_input_and_terminate`) | 통과 |
| 없는 프로그램 `Spawn(NotFound)`(`missing_program_is_a_spawn_failure`) | 통과 |
| SDK 상수 고정(`constants_match_macos_sdk_headers`), 크기 범위(`sizes_are_bounded`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 외부 PTY crate 대신 최소 FFI를 택해 `config/dependencies.yaml`·ADR 변경이 필요 없습니다.

## 남은 위험 / 한계

- `std::process::Command`는 `pre_exec`가 있으면 fork를 씁니다. fork와 exec 사이의 짧은 구간에는 다른
  세션의 fd 사본이 child에 있지만 close-on-exec라 exec 시 닫힙니다(exec가 실패한 child는 곧 종료).
- drain 상한(1초) 안에 reader가 읽지 않으면 남은 출력은 버려집니다(reader가 없는 경우).
- 출력 유실 원인에 대한 커널 내부 설명은 실측 기반 추정이며, Apple 공식 문서 근거는 찾지 못했습니다.
