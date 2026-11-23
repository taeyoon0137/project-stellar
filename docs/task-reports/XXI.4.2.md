# 작업 보고서: XXI.4.2 Host API namespace 분리

- WBS: XXI.4.2
- 작업명: read snapshot, query projection, apply CommandTransaction, Git status, file/network 요청을 별도 RPC namespace로 분리
- 상태: 완료 (`[x]`)
- 선행: XXI.4.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 1·5·6

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/host.rs`(신규) — `Dispatcher::call`, `check_params`, `HostConfig`, `NetworkPort`.
- `crates/plugin-host/src/plugin_data.rs`(신규) — `file.*`가 쓰는 `plugins/<plugin-id>/` 디렉터리.
- `crates/plugin-host/src/rpc.rs`(신규) — `RpcError`, `RpcLimits`.
- `crates/plugin-host/src/lib.rs`, `Cargo.toml`(`stellar-git-porcelain` 의존), IDL `GitHead.commit` field와 재생성 TypeScript.
- `crates/plugin-host/tests/host_api.rs`, `tests/support/mod.rs`(신규).
- 등록: `tools/xtask/src/crate_graph.rs` 허용 edge에 git-porcelain, `docs/architecture/crate-dependency-graph.md`.

## 구현 요약

- namespace 7개: `host`(권한 없음), `document`·`query`(`document.read`), `command`(`document.apply`), `git`(`git.read`),
  `file`(`file`), `network`(`network`). namespace마다 권한 하나에 묶입니다.
- `Dispatcher::call` 순서: 생성 표로 method 찾기 → 생성 매개변수 표로 검사 → 유효 권한(manifest 선언 ∩ 승인) 확인 → 신뢰 재확인
  → handler. handler는 ops 공개 함수(`query::project_structure`, `query::document_tree_page`, `query::node_detail`,
  `query::tokens`, `query::components`, `apply::apply` + `mcp_command_input::command_input`, `git::GitSession::status`)만 부릅니다.
- `git.status`는 session이 확인한 신뢰로 `GatedGitRunner`를 열어 `GitSession::open`을 부릅니다(OS 신뢰 저장소를 따로 다시 읽지 않음).
- `network.fetch`는 https URL만 받고 Host가 준 `NetworkPort`로만 보냅니다. port가 없으면 `unavailable`입니다.
- 오류 문구는 고정 문구이고 code로 구분합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test host_api` | 4 passed |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 통과 |

| 검사(API inventory) | 결과 |
|---|---|
| namespace별 권한이 하나이고 기대 표와 같음 | 통과 |
| 모든 method 결과가 IDL record 형태와 같음(모르는 field 없음, 필수 field 있음) | 통과(실제 `git init` 저장소 포함) |
| 모르는 method(`pointer.move` 포함), 모르는 매개변수, 형식 오류, 권한 없음 거부 | 통과 |
| 프로젝트 신뢰 철회 뒤 프로젝트 method `not-trusted`, `host.info`는 동작 | 통과 |

## 남은 위험 / 한계

- terminal·clipboard 권한은 정의만 있고 method는 아직 없습니다(WebView·앱 연결 때 추가).
- JSON 결과 변환(ops 결과 → JSON)은 mcp-core와 별도 구현입니다. 판정·해석은 ops 하나이고 변환 형태만 다릅니다.
