# 작업 보고서: XVIII.4.6 창 종료 시 터미널 미복원과 recovery snapshot의 출력·프로세스 제외 검사

## WBS

- WBS: XVIII.4.6
- 작업명: 창 종료 시 terminal을 복원하지 않고 종료하며 recovery snapshot에 output/process를 넣지 않는 검사
- 상태: 부분 완료 (`[~]`) — 엔진 검사는 모두 통과하지만 선행 XVIII.4.3(macOS terminal view, 셸 UI)이 `[ ]`입니다(계획 1.2
  규칙 4). terminal view는 사용자 방향(2026-11-23)에 따라 UI 재설계 때 만듭니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.8.2, XVIII.4.3
- 결정: [ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 4, [ADR-0023](../adr/ADR-0023-integrated-terminal-user-shell.md) 결정 10
- lane: `recovery-git-terminal`

## 변경 파일

- `crates/terminal/tests/no_restore.rs`(신규): 창 종료·crash/relaunch fixture
- `crates/project/src/recovery_payload.rs`: payload 필드 고정·터미널 crate 비의존 시험

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal --test no_restore` | 0 — 3 passed(5회 반복 모두 통과, 뒤에 남은 `sleep 300` 프로세스 없음) |
| `cargo test -p stellar-project --lib recovery_payload` | 0 — 7 passed |
| `cargo clippy -p stellar-terminal -p stellar-project --all-targets -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬, 실제 POSIX PTY. crash는 시험 binary를 앱 역할 helper로 다시 실행해 `SIGKILL`로 끝냅니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `no_restore::crash_and_relaunch_leave_no_terminal_process_to_reconnect` | 통과 — helper가 PTY 세션(`sleep 300`)을 연 채 `SIGKILL` → PTY hangup으로 세션 프로세스 종료 → 새 registry 비어 있음, 이전 pid 없음(process 재연결 없음) |
| `no_restore::window_close_ends_terminal_processes_and_reopen_starts_empty` | 통과 — registry drop(창 종료) 뒤 프로세스 종료, 같은 창 새 registry 빈 상태 |
| `recovery_payload::tests::payload_has_no_terminal_output_or_process_fields` | 통과 — payload·탭 필드 목록을 구조 분해로 고정, `stellar-project`·`stellar-runtime`이 `stellar-terminal`에 의존하지 않음 |
| 기존 `lib::tests::terminal_state_has_no_persistence_dependency` | 통과 — 터미널 crate는 저장·프로젝트·문서·직렬화 crate에 의존하지 않음 |

## 성능

- 해당 없음.

## 차이

- 검증 열의 "crash/relaunch"는 앱 UI 대신 registry를 쥔 helper 프로세스의 강제 종료와 새 registry로 대신했습니다.
- recovery snapshot 쪽은 실행 시점 검사 대신 형식·의존성 고정 검사입니다. 복구 문서 chunk는 runtime이 문서 직렬화 결과와
  같은지 다시 확인하므로(`load_model`) 다른 내용이 들어갈 경로가 없습니다.

## 남은 위험

- 앱 runtime이 터미널 패널을 소유하게 되면 창 session 닫기 경로에서 registry를 drop하는지 그 연결에서 다시 확인해야 합니다.
