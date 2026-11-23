# 작업 보고서: XXIX.2.1 stellar-agent-host crate 추가와 crate graph 등록

- WBS: XXIX.2.1
- 작업명: `stellar-agent-host` crate를 추가하고 crate graph에 등록, 금지 타입 inventory 고정
- 상태: `[x]` 완료
- 선행: XIX.9.1, XXIX.1.1
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `Cargo.toml`(workspace member `crates/agent-host`), `Cargo.lock`
- `crates/agent-host/Cargo.toml`, `crates/agent-host/src/lib.rs`, `crates/agent-host/src/json.rs`(mcp-core 사본)
- `crates/agent-host/tests/inventory.rs`
- `tools/xtask/src/crate_graph.rs`(`allowed_edges()` 행, `agent_host_edges_follow_adr_0005` test)
- `docs/architecture/crate-dependency-graph.md`(허용 의존 표 행, Mermaid edge)
- `docs/adr/ADR-0092-agent-host-subscription-cli-core.md`(신규)

## 구현 요약

- crate 허용 직접 의존: core-types, agent-protocol, local-ipc, diagnostics. 실제 의존은 agent-protocol(권한 어휘)과 local-ipc(Team ID·code requirement, 사용자 임시 디렉터리, 결합 token)뿐입니다.
- 전이 도달 금지(document, commands, history, storage, ops)는 기존 `FORBIDDEN_REACH` 규칙이 crate가 생긴 순간부터 적용됩니다.
- 외부 crate가 없어 JSON reader·writer는 `crates/mcp-core/src/json.rs`의 의도적 사본입니다(ops 타입 변환 두 개만 뺌, 본문 일치 test).
- inventory(`tests/inventory.rs`, 주석과 단위 테스트를 뺀 코드 줄 기준): API key·network API·자격 증명 저장·모델 라우팅·사용량 계량 낱말 0.

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
| `cargo xtask validate-crate-graph`(agent-host 포함 workspace) | 위반 0 |
| `agent_host_edges_follow_adr_0005`: 실제 manifest edge 허용, document·commands·history·storage·ops·mcp-core·runtime·ffi edge는 `forbidden-edge` | 통과 |
| `no_provider_api_client_key_credential_routing_or_metering` | 금지 타입 inventory 0 |
| `json_body_matches_mcp_core_copy` | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 없음. ADR-0092 결정 1에 기록했습니다.

## 남은 위험 / 한계

- JSON 사본이 셋(stellar-cli, mcp-core, agent-host)이 됩니다. 공용화는 XXIX.1.2(외부 JSON crate 도입) 결정에 맡깁니다.
