# 작업 보고서: XXI.5.2 process lifecycle, heartbeat, timeout, permission revocation

- WBS: XXI.5.2
- 작업명: process lifecycle, heartbeat, timeout, permission revocation 구현
- 상태: 완료 (`[x]`)
- 선행: XXI.5.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 11

## 필수 실행 환경

- `DEV-MAC` (Rust, `/bin/sh`, `/bin/kill`, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/tests/process_lifecycle.rs`(신규).
- 구현은 XXI.5.1 커밋의 `crates/plugin-host/src/process.rs`(`ProcessPlugin::pump`·`revoke`·`Drop`)입니다.

## 구현 요약

- `pump`가 Host 응답을 child에 넘기고, `heartbeat_interval`마다 heartbeat를 보내고, 응답이 `heartbeat_timeout` 안에 없으면 kill합니다.
- child가 스스로 끝나면 `Exited(code)`, Host가 끝내면 `Killed(이유)`입니다. 어느 쪽이든 child를 `wait`로 거두고 stdin을 닫고 session
  worker를 멈춥니다.
- 권한 철회(`revoke`)와 Host 쪽 drop도 kill과 같은 정리를 합니다. Host가 끝낸 경우 session queue에 남은 요청은 실행하지 않습니다(`HostSession::abort`, 검토 보완). session 단위 권한 철회만 할 때는 `HostSession::set_permissions`(XXI.4.3)를 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test process_lifecycle` | 2 passed |
| `cargo test -p stellar-plugin-host --test process_plugin` | 2 passed(철회 정리 포함) |

| 검사(kill/hang fixture cleanup) | 결과 |
|---|---|
| stdin을 읽지 않는 process: heartbeat 제한 400ms 뒤 `HeartbeatTimeout`으로 kill, 5초 안, pid 사라짐 | 통과 |
| JSON이 아닌 줄: `ProtocolViolation`으로 kill, pid 사라짐 | 통과 |
| Host가 `ProcessPlugin`을 놓으면(drop) process를 끝내고 pid 사라짐 | 통과 |
| `revoke`가 `Killed(Revoked)`, pid 사라짐 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `sh`가 띄운 손자 process는 이 계약이 추적하지 않습니다. 실제 플러그인 실행 파일은 직접 실행하는 단일 process를 전제로 합니다.
  process group 단위 정리는 앱 supervisor를 붙일 때 정합니다.
- 기본 heartbeat 간격 5초·제한 10초는 측정 근거 없이 정한 값입니다.
