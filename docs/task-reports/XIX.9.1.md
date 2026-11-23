# 작업 보고서: XIX.9.1 agent-protocol·mcp-core 경계 등록과 실행 파일 진입점 정리

- WBS: XIX.9.1
- 작업명: `stellar-local-ipc`·`stellar-agent-protocol`·`stellar-mcp-core`를 crate graph 정책과 문서에 등록하고 `tools/stellar-mcp`가 library 진입점만 갖게 함
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(agent-protocol-mcp lane). `[~]`의 이유였던 선행 XXIX.1.1은 `Task(XXIX.1.1)` 커밋으로 `[x]`가 되었습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XXIX.1.1이 `[~]`(crate 의존 그래프 문서의 검사 범위·전이 도달 금지 절, 이 lane 소유 밖)라서 계획 1.2 규칙 4에 따라 `[x]`로 올리지 않습니다. 이 행 자체의 범위는 모두 구현했습니다.
- 선행: XIX.1.3 `[x]`, XXIV.4.2(ledger-verification 재판정 `[x]`, 원장 미반영), XXIX.1.1 `[~]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/agent-protocol/`(신규) — package `stellar-agent-protocol`. 허용 직접 의존은 `stellar-core-types`뿐입니다.
- `Cargo.toml` — workspace member에 `crates/agent-protocol` 한 항목 추가.
- `crates/mcp-core/Cargo.toml` — `stellar-agent-protocol` 의존 한 줄.
- `crates/mcp-core/src/entry.rs`(신규) — 이전 `tools/stellar-mcp/src/main.rs`의 인자 해석, backend 선택(`--headless`·`--attach`·`--connection-info`), 부모 감시, 서버 실행을 `entry::run(args, input, output)`으로 옮겼습니다.
- `crates/mcp-core/src/lib.rs` — `entry` 모듈, `agent_protocol` 재노출.
- `tools/stellar-mcp/src/main.rs` — 표준 입출력 handle을 열고 `entry::run`을 부르는 함수 하나만 남겼습니다.
- `tools/stellar-mcp/Cargo.toml` — 일반 의존은 `stellar-mcp-core` 하나. `stellar-local-ipc`는 앱 측 IPC로 세션 수명을 검사하는 테스트 전용 dev 의존으로 옮겼습니다.
- `tools/stellar-mcp/tests/entry_point.rs`(신규) — 실행 파일 package가 진입점만 갖는지 검사.
- `tools/xtask/src/crate_graph.rs` — 정책 표에 `stellar-agent-protocol → core-types`, agent-protocol·mcp-core 실제 manifest 검사 test.
- `docs/architecture/crate-dependency-graph.md` — 허용 표와 Mermaid에 `stellar-agent-protocol` 한 줄씩(행 지시의 "crate 의존 그래프 문서에 등록").

## 구현 요약

- crate 배치는 ADR-0005 §8 표 그대로입니다: agent-protocol·local-ipc는 core-types만, mcp-core는 ops·agent-protocol·local-ipc·diagnostics만, 실행 파일 `stellar-mcp`는 mcp-core에 의존합니다. 전이 도달 금지(`FORBIDDEN_REACH`)의 agent-protocol 규칙(commands·document·ops·agent-host)은 crate가 생긴 순간부터 적용됩니다.
- 실행 파일의 stdout handle은 지금처럼 `main.rs`에서 한 번만 엽니다(`protocol_smoke`의 stdout 검사 유지). library는 받은 handle에만 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo xtask validate-crate-graph` | 종료 0 |
| `cargo test -p xtask --lib crate_graph` | 42 passed |
| `cargo test -p stellar-mcp` | 전부 통과(새 `entry_point` 1개 포함) |

| 검사 | 결과 |
|---|---|
| tools 진입점에 protocol 로직 0: `src`에 `main.rs`·`lib.rs`만, `main.rs` 함수 하나·분기 없음·`entry::run` 호출(`executable_package_has_only_the_library_entry_point`) | 통과 |
| agent-protocol 실제 manifest의 내부 의존이 core-types 하나, 금지 edge 거부(`agent_protocol_and_mcp_core_follow_adr_0005`) | 통과 |

## 차이(작업 지시와 다른 점)

- XIX.1.1~XIX.1.3 구현(IPC·attach·headless)은 이전 통합에서 이미 `crates/mcp-core`·`crates/local-ipc`로 옮겨져 있었습니다. 이번에는 남은 실행 파일 로직(인자 해석·backend 선택)을 옮겼습니다.

## 남은 위험 / 한계

- `docs/architecture/crate-dependency-graph.md`의 첫 문단·검사 범위·전이 도달 금지 절(XXIX.1.1 남은 일)은 고치지 않았습니다. XXIX.1.1이 `[x]`가 되면 이 행도 `[x]`로 판정할 수 있습니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIX.9.1)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `923c3090` Feat: agent-protocol crate를 등록하고 stellar-mcp를 library 진입점만 갖게 함 (XIX.9.1)
