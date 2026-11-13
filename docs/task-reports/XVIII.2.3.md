# 작업 보고서: XVIII.2.3 winsize ioctl resize

- WBS: XVIII.2.3
- 작업명: resize를 winsize ioctl로 전달
- 상태: 완료 (`[x]`)
- 선행: XVIII.2.1

## 필수 실행 환경

- `DEV-MAC` (macOS 실제 PTY, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/pty/mod.rs` — `resize`(`TIOCSWINSZ`), `size`(`TIOCGWINSZ`), 크기 fixture 테스트.
- `crates/terminal/src/pty/sys.rs` — `TIOCGWINSZ` 상수와 고정 테스트.
- `crates/terminal/src/launch.rs` — `TerminalProcess::resize`·`size`.
- `crates/terminal/src/registry.rs` — `TerminalRegistry::resize`, `RegistryError::NotRunning`·`Process`, 가짜 프로세스 크기 계측.
- `docs/task-reports/XVIII.2.3.md`(신규), 계획서(XVIII.2.3 `[ ]`→`[x]`).

## 구현 요약

- **전달 경로**: `TerminalRegistry::resize(id, size)` → 실행 중 세션의 `TerminalProcess::resize` →
  master fd에 `ioctl(TIOCSWINSZ, struct winsize)`. 커널이 새 크기를 저장하고 foreground process
  group에 `SIGWINCH`를 보냅니다. 크기는 `TerminalSize`(1~4096행·열)로 검증된 값만 받습니다.
- **조회**: `size()`는 master의 `TIOCGWINSZ`로 커널에 저장된 값을 읽습니다.
- 실행 중이 아닌 세션은 `NotRunning`, 없는 세션은 `UnknownSession`, ioctl 실패는 `Process(err)`입니다.
- 초기 크기는 XVIII.2.1의 PTY 생성 시 `LaunchEnvironment::size`로 설정됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 48 passed, 0 failed |
| `cargo test -p stellar-terminal` 15회 반복 | 실패 0 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| columns/rows fixture | 결과 |
|---|---|
| 초기 크기 24×80·33×101·1×1·200×500을 child `stty size`가 그대로 읽음(`initial_size_fixture`) | 통과 |
| 실행 중 50×132로 resize → `TIOCGWINSZ` 일치, child `stty size` = `50 132`(`resize_fixture_columns_and_rows`) | 통과 |
| resize가 foreground job에 `SIGWINCH` 전달(trap에서 `40 100` 출력, `resize_delivers_sigwinch_to_foreground_job`) | 통과 |
| registry: 시작 전 `NotRunning`, 초기 크기 전달, resize 전달, 닫은 뒤 `UnknownSession`(`resize_is_forwarded_to_running_sessions_only`) | 통과 |
| `TIOCGWINSZ` = `0x40087468` 고정(`constants_match_macos_sdk_headers`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. pixel 크기(`ws_xpixel`·`ws_ypixel`)는 0으로 둡니다(terminal view 연결 XVIII.4.3에서 필요하면
  확장).

## 남은 위험 / 한계

- view가 resize를 연속으로 보낼 때의 coalescing(throttle)은 앱 셸 view 연결 범위입니다.
