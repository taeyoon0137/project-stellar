# 작업 보고서: XXI.5.1 외부 process 플러그인 IPC contract

- WBS: XXI.5.1
- 작업명: 장기 실행·네이티브 플러그인을 별도 process로 실행하는 IPC contract 정의. app process에 native library를 dlopen하지 않음
- 상태: 완료 (`[x]`)
- 선행: XXI.4.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 11

## 필수 실행 환경

- `DEV-MAC` (Rust, `/bin/sh`, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/process.rs`(신규) — `ProcessPlugin`(`launch`, `pump`, `revoke`), `ProcessConfig`, `ProcessState`, `KillReason`.
- `crates/plugin-host/src/lib.rs` — module 등록.
- `crates/plugin-host/tests/fixtures/process/plugin.sh`(신규) — 동작별 fixture process.
- `crates/plugin-host/tests/support/process.rs`(신규), `tests/support/mod.rs` — fixture 공용 함수.
- `crates/plugin-host/tests/process_plugin.rs`(신규).

## 구현 요약

- 플러그인은 `std::process::Command`로 띄운 별도 process입니다. 앱 process에 플러그인 코드를 적재하지 않습니다.
- IPC: stdin·stdout의 UTF-8 줄 단위 JSON. 플러그인 → Host 요청은 XXI.4.3 RPC envelope 그대로이고 같은 `HostSession`(권한·상한·
  권한 철회)을 지납니다. Host 응답(조각 포함)도 같은 줄 형식입니다. stderr는 버립니다.
- 수명 메시지: Host `{"stellar_process":1,"type":"heartbeat","seq":n}` → 플러그인 `{"stellar_process":1,"type":"heartbeat-ack","seq":n}`.
- 줄 길이 상한(기본 1 MiB)을 넘거나 JSON이 아니면 protocol 위반으로 끝냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test process_plugin` | 2 passed |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| fixture process가 `host.info`를 요청해 응답을 받고 heartbeat 3회에 답함 | 통과 |
| process가 code 3으로 죽어도 Host process는 계속 동작하고 같은 프로젝트로 새 dispatcher가 읽기 성공(process crash isolation) | 통과 |
| 철회 뒤 process id가 더는 살아 있지 않음 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 실행 파일을 manifest에 선언하는 필드는 아직 없습니다. 호출자가 package 안 경로(`PluginPackage::resolve`)로 정합니다.
- process sandbox(macOS sandbox profile 등)는 적용하지 않았습니다. 권한은 Host RPC에서만 제한됩니다.
