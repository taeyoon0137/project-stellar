# 작업 보고서: XIX.4.7 user interaction preview 중인 node는 gesture 종료 후에만 agent lease 획득

- WBS: XIX.4.7
- 작업명: user interaction preview 중인 node는 gesture 종료 후에만 agent lease 획득
- 상태: 부분 완료 (`[~]`)
- 선행: XII.3.7, XIX.4.2
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/coordinator.rs` — `Gesture`, `begin_gesture`, `HeldGesture::end`, `Conflict::Gesture`.
- `crates/mcp-core/src/lease.rs` — `LeaseState::WaitingForGesture`, `AgentEditLeases::retry_waiting`.

## 구현 요약

- 사용자 조작 미리보기는 조정자에 gesture 항목으로 표시합니다. gesture 대상과 겹치는 lease 요청은 `Conflict::Gesture`이고, 요청 관리자는 승인된 요청을 대기로 두었다가 gesture가 끝난 뒤 `retry_waiting`에서 획득합니다.
- lease 안 node에서 gesture를 시작하는 것은 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | 185 passed, 5 ignored(자식 프로세스 전용 1건 포함) |
| `cargo test -p stellar-mcp-core --locked` | 모든 test target 통과(lib 82 passed) |
| `cargo clippy -p stellar-commands -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base edfea9db` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |

| 검사 | 결과 |
|---|---|
| drag 중 lock 대기 fixture: drag 중 승인한 요청이 `WaitingForGesture`이고 잠그지 않으며, 종료 전 재시도는 계속 대기, 종료 뒤 재시도는 `Granted`(`approved_request_waits_for_gesture_end`) | 통과 |
| gesture와 같은 node·조상·root 요청은 대기, 겹치지 않는 subtree는 gesture 중에도 획득, lease 안 gesture 시작 거부(`lease_waits_for_gesture_end`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XII.3.7이 `[!]`이고, 앱 canvas gesture 경로(runtime)가 `begin_gesture`를 부르는 연결은 하지 않았습니다(소유 밖). 그래서 `[~]`입니다.

## 남은 위험 / 한계

- runtime canvas gesture 시작·종료에 `begin_gesture`·`end`와 `retry_waiting` 호출을 연결해야 합니다.

## 근거 커밋

- `286d8c4c`
- `52aa2c2a`
