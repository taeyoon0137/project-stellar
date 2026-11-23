# 작업 보고서: XXIX.2.4 macOS 실행기

- WBS: XXIX.2.4
- 작업명: ADR-0005 §4.3 불변식과 ADR-0023 결정 9 허용 목록 환경을 지키는 macOS 구독 CLI 실행기
- 상태: `[x]` 완료
- 선행: XXIX.2.2, XVIII.2.4, XVI.7.5
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/process.rs`(신규): `ScratchDir`, `run_capture`, `SessionProcess`, `OutputSink`, MCP 설정 쓰기, process group signal
- `crates/agent-host/src/env.rs`(신규): `ChildEnv` 허용 목록 환경
- `crates/agent-host/src/output.rs`(신규): 수신 출력 처리(원문·디코드)
- `crates/agent-host/tests/executor.rs`

## 구현 요약

- **파이프 비대화형 실행**: stdin·stdout·stderr 파이프, PTY 없음.
- **환경**: `env_clear` 뒤 profile 환경 변수 이름 목록에 있는 이름만 앱 환경에서 그대로 전달합니다. Stellar가 더하는 변수 0. 결합 token(XIX.1.8)은 CLI 환경이 아니라 MCP 설정 파일의 서버 `env`에만 둡니다.
- **불변식 2(흔적 없음)**: 세션마다 OS 사용자 임시 디렉터리(`confstr(_CS_DARWIN_USER_TEMP_DIR)`, Apple `confstr(3)`) 아래 mode 0700 scratch를 만들고 `work/`를 cwd로 씁니다. 프로젝트 working tree를 cwd로 주지 않습니다. MCP 설정은 scratch에 mode 0600(`O_CREAT|O_EXCL`)으로 씁니다. 세션이 끝나면 scratch를 통째로 지웁니다.
- **불변식 1(문서 변경은 Stellar MCP tool로만)**: 실행기는 profile의 공식 설정(XXIX.2.6: 내장 tool 끔, Stellar MCP만)을 그대로 넘기고, 프로젝트 경로를 CLI에 주지 않습니다.
- **process group**: `CommandExt::process_group(0)`(POSIX `setpgid`)으로 CLI를 새 group leader로 띄우고, CLI가 끝나면 group 전체에 SIGKILL을 보내 하위 프로세스를 남기지 않습니다.
- **timeout·취소**: 취소 또는 시작 timeout(첫 stdout byte까지 `startup_ms`)이면 group에 SIGTERM, `terminate_grace_ms` 뒤 SIGKILL. 확인 명령(`run_capture`)은 전체 timeout에 SIGKILL.
  - 값: version 5 s·status 10 s(로컬 명령, 대역 실측 수 ms), startup 60 s(Claude Code CLI reference: `-p`와 `--mcp-config`이면 MCP 서버 연결을 `MCP_TIMEOUT` 기본 30 s까지 기다림 → 두 배), grace 3 s.
- **수신 정지 없음(ADR-0005 §6)**: stdout·stderr마다 reader 스레드가 64 KiB씩 읽어 바로 `OutputSink`에 넘깁니다. `SessionOutput`은 lock을 조각 처리 동안만 잡고 UI는 증분만 가져갑니다.
- **stdin**: 전용 writer 스레드. 파이프 fd에 macOS `F_SETNOSIGPIPE`(`fcntl(2)`)를 켜 앱이 SIGPIPE로 죽지 않게 합니다. Claude Code는 `--input-format stream-json` user message 줄(여러 turn), Codex는 첫 메시지를 쓰고 닫음.
- 세션 실행은 `PreflightPass::launch`로만 부를 수 있습니다(XXIX.2.7). 실행 직전 실행 파일 지문을 다시 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| 테스트 대역 세션에서 Stellar MCP tool 외 프로젝트 변경 0: 프로젝트 디렉터리 전후 내용 동일 | 통과 |
| 세션 전후 프로젝트·사용자 설정(가짜 HOME의 CLI 설정) 동일, CLI 실행 파일 bytes 동일 | 통과 |
| 세션 종료 후 실행 흔적 0: scratch(cwd, MCP 설정, CLI가 cwd에 만든 파일) 없음, 하위 프로세스(`spawn-child`) 5 s 안에 소멸 | 통과 |
| cwd가 scratch `work/`이고 프로젝트가 아님 | 통과 |
| 허용 목록 밖 환경 변수 전달 0: child 환경 이름 ⊆ profile 목록, 가짜 비밀·API key·`DYLD_INSERT_LIBRARIES` 미전달 | 통과 |
| timeout fixture: startup 300 ms 대기 세션 → `StartupTimeout`, 5 s 안 종료, scratch 정리 | 통과 |
| 취소 fixture: 출력 뒤 hang 세션 cancel → `Cancelled`, 3 s 안 종료 | 통과 |
| UI 지연 fixture: UI가 증분을 전혀 가져가지 않는 동안 32 MiB 출력 전부 수신(종료 코드 0, 받은 byte 33,554,432), 원문 상한 유지 | 통과 |
| stdin: stream JSON 첫 줄 정확, 후속 메시지 전달, 한 turn profile은 후속 거부 | 통과 |
| MCP 설정 mode 0600, `mcpServers.stellar` stdio, 결합 token은 서버 env에만, 종료 뒤 파일 없음 | 통과 |

- 성능: 테스트 대역 32 MiB 출력 수신 0.08\~1.0 s(debug, 동시 작업 부하에 따라 다름). 성능 예산 파일 변경 없음.

## 차이(작업 지시와 다른 점)

- 실행 세부(timeout 값, cwd, process group, SIGPIPE)는 ADR-0092 결정 6과 이 보고서에 근거와 함께 기록했습니다.

## 남은 위험 / 한계

- 같은 사용자 권한의 다른 프로세스는 scratch를 읽을 수 있습니다(신뢰 경계는 OS 사용자).
- CLI가 자기 설정 디렉터리에 쓰는 상태 파일은 CLI 동작이라 대역으로 재현할 수 없습니다. 릴리스 전 수동 검증(XXIX.6.8)에서 profile별로 확인해야 합니다.
- 앱 runtime이 이 실행기를 Agent Host 스레드에서 부르는 연결은 XXIX.3.1입니다.
