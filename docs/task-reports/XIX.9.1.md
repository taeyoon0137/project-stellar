# 작업 보고서: XIX.9.1 headless 읽기 연결에 필요한 L2 library 경계

- WBS: XIX.9.1
- 작업명: 기존 MCP library를 ADR-0005의 mcp-core 경계로 이동
- 상태: 부분 완료 (`[~]`) — mcp-core와 기존 local-ipc 경로는 연결했고 agent-protocol 등록·정의 이동은 남았습니다.
- 선행: XIX.1.3, XXIV.4.2, XXIX.1.1. 이번 구현은 XIX.9.3의 한 읽기 경로에 필요한 범위입니다.

## 필수 실행 환경

`DEV-MAC`. 임시 fixture의 실행 파일 테스트와 기존 golden을 사용했습니다. 상세 증거는 [XIX.9.3](XIX.9.3.md)에 있습니다.

## 변경 파일 목록

- `crates/mcp-core/`: 기존 MCP library 모듈을 이동하고 ops를 in-process로 연결.
- `tools/stellar-mcp/src/lib.rs`: 기존 Rust 사용 경로를 보존하는 재노출. `main.rs`는 인자·입출력·프로세스 수명 진입점을 유지.
- workspace Cargo manifest·lock, `tools/xtask/src/crate_graph.rs`, `crate_graph/reach.rs`, 의존 그래프 문서: ADR-0005 §8에 이미 정의된 edge와 전이 금지 등록.
- 기존 소스 inventory 테스트는 이동한 library까지 계속 검사합니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo check -p stellar-mcp --all-targets` | 0 |
| `cargo test -p stellar-mcp -p stellar-mcp-core --no-fail-fast` | 0, 기존 protocol·catalog·attach·수명 테스트 통과 |
| `cargo xtask validate-crate-graph` | 0 |
| `cargo xtask validate-dependencies` | 0 |

새 외부 의존은 없고 ops의 기존 YAML·텍스트 승인 의존만 전이적으로 포함됩니다. 성능 측정은 하지 않았습니다.

## 차이(작업 지시와 다른 점)

이번 사용자 요청은 한 읽기 경로이므로 agent-protocol의 공용 정의·생성 schema까지 이동하지 않았습니다. 승인·서명·runtime·FFI 경계는 변경하지 않았습니다.

## 남은 위험 / 한계

agent-protocol 등록과 catalog 이동(XIX.9.2), attached host trait, 나머지 backend 작업이 남습니다. XIX.9.1 전체 완료를 주장하지 않습니다.
