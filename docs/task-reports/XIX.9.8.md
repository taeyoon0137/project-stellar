# 작업 보고서: XIX.9.8 MCP client 등록 정보 제공

- WBS: XIX.9.8
- 작업명: MCP 클라이언트 등록에 필요한 정보를 제공하되 다른 도구의 설정은 바꾸지 않음(ADR-0005 §3)
- 상태: 완료 (`[x]`)
- 선행: XIX.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/connection_info.rs`(신규) — 등록 정보 JSON 생성(`connection_info`), 실행 파일 절대 경로(`current_command`), 형식 식별자.
- `tools/stellar-mcp/src/main.rs` — `--connection-info` 실행 방식, 도움말·종료 코드 문서.
- `tools/stellar-mcp/src/lib.rs` — module 등록.
- `tools/stellar-mcp/tests/connection_info.rs`(신규), `tests/fixtures/connection-info.json`(신규) — golden, 실행 충분성, 설정 무변경, 파일 변경 API inventory.
- `docs/task-reports/XIX.9.8.md`(신규), 계획서(XIX.9.8 `[ ]`→`[x]`).

## 제공 방식과 근거(MCP 명세 2025-11-25)

- **방식**: `stellar-mcp --connection-info`가 MCP 서버를 실행하지 않고 등록 정보 JSON 한 줄을 stdout에 쓰고 종료 코드 0으로 끝납니다. 파일을 읽거나 쓰지 않습니다(실행 파일 경로 확인 제외). 사용자나 client가 이 값을 자기 설정 형식으로 옮깁니다.
- **필드와 근거**
  - `transport: "stdio"`, `command`, `args`, `env`: Transports "stdio" — client가 서버를 subprocess로 실행하고 stdin/stdout으로 통신하므로, 등록에 필요한 것은 실행 파일과 인자, 추가 환경 변수입니다. `command`는 `std::env::current_exe()`를 `canonicalize`한 절대 경로이고, 이 서버는 인자와 환경 변수를 요구하지 않아 `args: []`, `env: {}`입니다.
  - `serverInfo{name, version}`: Lifecycle "Initialization"의 initialize 응답 `serverInfo`와 같은 값입니다.
  - `protocolVersions`: Lifecycle "Version Negotiation"의 협상 대상 목록(`config/mcp-protocol.yaml`)을 참고용으로 줍니다. 실제 version은 initialize에서 정해집니다.
  - `schema: "stellar-mcp/connection-info"`, `schemaVersion: 1`: MCP 명세(Transports·Lifecycle)는 client 설정 파일 형식을 정의하지 않으므로, 특정 client 형식을 만들지 않고 Stellar 소유 형식으로 둡니다. key 순서는 고정입니다.
- **다른 도구 설정 무변경**: 출력만 하고 client 설정 파일을 찾거나 수정하는 코드가 없습니다. 브리지 소스(`tools/stellar-mcp/src`, 테스트 module 제외)에 파일 생성·수정·삭제 API가 없음을 inventory 테스트로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 33, connection_info 4, json_copy_sync 2, no_network 3, parent_exit 2, protocol_smoke 5, protocol_version_matrix 2 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **연결 정보 golden 일치**: 출력 = `connection-info.json`(`<COMMAND>` = 빌드된 실행 파일 절대 경로), JSON 한 줄, stderr 비어 있음, 종료 코드 0 | 통과 |
| **다른 도구 설정 변경 0**: 임시 HOME에 client 설정 파일 모양의 파일 3개를 두고 실행 → HOME·작업 디렉터리의 항목 목록·내용·수정 시각 동일, 작업 디렉터리 생성 파일 0(`connection_info_matches_golden_and_changes_no_other_tool_config`) | 통과 |
| 등록 정보의 `command`·`args`·`env`만으로 실행한 서버가 initialize에 응답, `serverInfo` 일치, 협상 version ∈ `protocolVersions`(`connection_info_is_sufficient_to_launch_the_server`) | 통과 |
| 추가 인자 거부(64, stdout 비어 있음)(`connection_info_rejects_extra_arguments`) | 통과 |
| 브리지 소스의 파일 변경 API 0건(`bridge_sources_have_no_file_mutation_api`) | 통과 |
| 고정 key·값, 경로 JSON escape, 실행 파일 경로 절대(`connection_info::tests`) | 통과 |
| 기존 stdout 사용 제한(`io::stdout()`은 `main.rs`에서 한 번, 출력 macro 없음) 계속 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 앱 번들 안 helper 경로(XXIX.5.1)와 headless 읽기 backend의 프로젝트 인자(XIX.9.3)가 생기면 `command`·`args`가 바뀔 수 있으며, 그때 형식 버전을 검토합니다.

## 남은 위험 / 한계

- 실행 파일 경로가 UTF-8이 아니면 JSON으로 표현할 수 없어 종료 코드 74로 끝납니다.
- 사용자가 실행 파일을 옮기면 등록한 `command`가 무효가 됩니다. 등록 정보는 출력 시점의 경로입니다.
- 같은 정보를 앱 UI(명령 설치 화면 등)에서 보여 주는 일은 XXIX 이후 작업입니다.
