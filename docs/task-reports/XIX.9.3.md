# 작업 보고서: XIX.9.3 승인 프로젝트의 headless MCP 읽기 연결

- WBS: XIX.9.3
- 작업명: 기존 신뢰 프로젝트 하나의 snapshot 읽기를 정상 stdio CLI에 연결
- 상태: 부분 완료 (`[~]`) — 이번 수직 경로는 구현·검증했으며, WBS 전체의 검증·dry-run tool과 agent-protocol 카탈로그 이동은 남았습니다.
- 선행: XIX.9.2 미완료, XXIV.4.3의 ops 조회와 XVI.7.2 신뢰 저장소를 재사용. XIX.3.3의 headless 예외는 ADR-0005 §3 및 아키텍처 §31.3의 기존 결정을 적용합니다.

## 필수 실행 환경

- `DEV-MAC`: 전용 브랜치 `stellar-takeover-read-mcp`의 Rust 빌드와 정식 Cargo 테스트.
- 모든 새 프로세스 테스트는 `TempDir`에 생성한 프로젝트·HOME·신뢰 등록을 사용합니다. 실제 저장소나 사용자 프로젝트·신뢰 파일·계정·자격 증명은 테스트 입력으로 사용하지 않았습니다.
- 앱 UI, 실제 외부 클라이언트, 공급자 CLI, CI·LAB·REF 환경은 검증하지 않았습니다.

## 계획과 완료 범위

- [x] AGENTS, ADR-0005 §3·§8, 아키텍처 §31·§37, XIX.2·XIX.9, 실제 main → server 흐름과 trust/scope/paging 검사 확인.
- [x] 기존 허용 의존 방향으로 L2 library를 분리하고 ops의 읽기 snapshot 연결.
- [x] 임시 fixture로 정상 프로세스 initialize → initialized → list/read, 읽기 범위와 크기 제한·거부·무변경 검증.
- [x] 프로젝트 없는 기본 실행, 명시적 attach, connection-info의 기존 동작 유지.
- [x] 이번 연결 범위와 남은 WBS 기능을 한국어로 기록.
- [x] 구현 Task 커밋 `e7d46f02` 뒤 `cargo xtask generate-ledger` 실행. 원장 1,152개 작업 중 XIX.9.1·XIX.9.3 상태/보고서만 갱신했고 상태·계획 동기화 검증을 통과했습니다.

## 변경 파일 목록

- `crates/mcp-core/`: 기존 `tools/stellar-mcp/src/`의 library 모듈 이동. `headless.rs`와 server의 고정 backend 분기 추가. JSON, IPC 인증, attach 수명, paging·scope 판정과 catalog URI·schema의 의미는 유지합니다.
- `tools/stellar-mcp/src/main.rs`, `src/lib.rs`, `Cargo.toml`: ID 선택 인자, core 재노출, 기존 실행 파일·library 사용 경로 유지.
- `crates/ops/src/trusted.rs`, `lib.rs`: 기존 신뢰 등록에서 ID 하나로 고정한 프로젝트를 읽기 전용으로 선택. `test_support.rs`: 임시 신뢰 fixture용 기존 타입 재노출.
- `crates/project/src/trust_store.rs`: 동일 ID에 대응하는 target이 정확히 하나인 경우만 찾는 읽기 accessor. grant/revoke·저장 형식·승인 정책은 변경하지 않습니다.
- `tools/stellar-mcp/tests/headless_read.rs`: 실제 실행 파일의 새 읽기 경로 테스트. 기존 소스 inventory 테스트 4개 파일은 이동 위치와 ops의 기존 승인 의존을 반영합니다.
- `Cargo.toml`, `Cargo.lock`, `tools/xtask/src/crate_graph.rs`, `crate_graph/reach.rs`, `docs/architecture/crate-dependency-graph.md`: ADR-0005 §8이 이미 허용한 `stellar-mcp → stellar-mcp-core → stellar-ops` 등록. mcp-core에서 agent-host/runtime/ffi 전이 도달 금지를 추가합니다. 새 외부 package·버전 변경은 없습니다.
- 계획서 XIX.9.1·XIX.9.3 상태와 이 보고서, XIX.9.1·XIX.2.5 보고서. 원장은 Task 커밋 뒤 생성합니다.

## 실행과 승인 범위

사용자가 기존에 신뢰한 프로젝트의 ID를 실행 인자로 명시합니다.

```text
stellar-mcp --headless --project <project-id>
```

- 파일 경로를 받지 않습니다. 기존 `FileTrustStore`에서 해당 ID에 등록된 유일한 canonical target만 선택합니다. 저장소 위치는 기존 ops 앱 데이터 이름(`xyz.taeyoon.stellar`)을 OS 사용자 데이터 base에 붙인 디렉터리의 `trust/trusted-projects.yaml`입니다. 경로 override, 신뢰 부여, 자동 승인 기능은 없습니다.
- 등록 없음, 중복 ID, 저장소 손상, OS 데이터 위치 불명, canonical target 또는 실제 매니페스트 ID 불일치는 시작 실패(77)입니다. 경로 문자열·잘못된 인자 조합은 사용법 오류(64)입니다.
- 세션 시작 때 backend와 프로젝트를 고정하고, 요청과 응답 시점에 기존 trust/identity를 다시 확인합니다. 신뢰 철회·ID 변경 시 내용을 내보내지 않습니다. 요청 URI와 응답 payload는 기존 `ApprovedScope` 검사를 거칩니다. scope 자체를 승인으로 취급하지 않습니다.
- headless 읽기는 기존 ADR의 승인 예외이며 attached의 첫 연결 승인을 대체하지 않습니다. `--attach`는 기존 인증·handshake를 유지하고 headless로 자동 전환하지 않습니다. 인자 없는 실행은 기존 빈 capability·ping 동작 그대로입니다.
- 프로젝트 내용은 resource의 `application/json` text content 안에 데이터로만 들어갑니다. instructions·로그에 프로젝트 문자열을 넣지 않습니다.

## 향후 Claude가 이 경로로 읽을 범위

| 호출 | 제공 범위 |
|---|---|
| `initialize` | 기존 protocol 협상, `resources: {}` capability만 광고 |
| `resources/list` | 기존 `project-structure`, `stellar://project/structure` 한 항목 |
| `resources/templates/list` | 기존 `document`, `stellar://documents/{document}{?cursor,limit}` 한 항목 |
| `resources/read` 구조 | 저장된 문서 ID·이름, page·View 개요, chunk 종류별 개수, revision |
| `resources/read` 문서 | 승인 프로젝트의 저장된 문서 트리 page: node ID·부모·View·깊이·종류·이름·표시/잠금 여부·자식 수, revision·cursor |
| `tools/list` | 빈 목록. 구현하지 않은 읽기·검증·dry-run·쓰기 tool은 모두 광고하지 않음 |
| `tools/call` | 제공하지 않음(`-32601`) |

- revision은 ops의 `token`, `generation`, `manifest_hash` 그대로입니다. 매 읽기는 하나의 디스크 generation을 사용합니다. 다음 요청에서 정상 저장이 관찰되면 이전 revision의 cursor는 기존 `PageError::StaleCursor`로 거부합니다(`-32602`).
- 문서 page는 기존 XIX.2.2의 기본 500개, 최대 2,000개, 내용 JSON 최대 512 KiB를 적용합니다. byte 한도에 맞춰 page를 줄이고, 한 item이 한도를 넘으면 오류입니다. 인코딩된 resource envelope에도 stdio 1 MiB 한도를 검사합니다.
- project structure는 기존 catalog에서 비페이지 resource입니다. URI·결과 schema에 임의 pagination 필드를 추가하지 않고, 내용이 512 KiB를 넘으면 오류로 닫습니다.
- 다른 프로젝트 ID/문서, URI의 프로젝트 선택, 선언 밖 인자, 다른 문서 cursor, 잘못된 limit은 `-32602`; 세션 중 신뢰를 확인할 수 없으면 내용 없는 `-32000`입니다. 미초기화 요청은 기존 `-32002`입니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo check -p stellar-mcp --all-targets` | 0, 통과 |
| `cargo test -p stellar-mcp --test headless_read` | 0, 새 프로세스 테스트 4개 통과 |
| `cargo test -p stellar-mcp -p stellar-mcp-core -p stellar-ops --no-fail-fast` | 0, MCP 프로세스·통합 34개, core 56개, ops 37개 통과 |
| `cargo test -p stellar-project trust_store` | 0, 15개 통과 |
| `cargo test -p xtask crate_graph` | 0, 41개 통과 |
| `cargo clippy -p stellar-mcp -p stellar-mcp-core -p stellar-ops -p stellar-project --all-targets -- -D warnings` | 0, 경고 없음 |
| `cargo xtask validate-crate-graph` | 0, 허용 직접 의존과 전이 금지 통과 |
| `cargo xtask validate-dependencies` | 0, 기존 승인 외부 의존 정책 통과 |
| `cargo fmt -p stellar-mcp -p stellar-mcp-core -p stellar-ops -p stellar-project -p xtask -- --check` | 0, 통과 |
| `cargo xtask validate-task-report docs/task-reports/XIX.9.3.md` (XIX.9.1·XIX.2.5도 각각 실행) | 0, 통과. 최초 표 헤더 형식 오류(65)는 수정 후 재검증 |
| `cargo xtask validate-adr <stage 변경 경로 전체>` | 0, 기존 허용 경로이며 보호 경로 변경 없음 |
| `cargo xtask validate-versions` | 0 |
| `cargo xtask generate-ledger` (Task 커밋 뒤) | 0, 1,152 tasks |
| `cargo xtask validate-task-status`, `cargo xtask sync-plan` | 각각 0 |
| `git diff --check` | 0, 통과 |

새 프로세스 테스트는 2,101개 node의 개수 제한 pagination, 긴 정상 이름의 byte 제한 pagination, 단일 item 초과 오류, 정상 저장 뒤 stale cursor, 다른 프로젝트 ID·선언 밖 인자, tool 호출 거부, 신뢰 철회·불명확·중복 ID·매니페스트 ID 변경을 확인합니다. 각 단계 전후 임시 프로젝트·신뢰 저장소·HOME 파일 목록과 content hash가 같습니다(테스트가 의도적으로 한 저장·grant/revoke 전후는 별도 기준). 기존 catalog의 20만 node pagination 테스트도 유지합니다. 성능 목표 달성을 주장하지 않으며 성능 측정은 하지 않았습니다.

## 차이(작업 지시와 다른 점)

- 사용자 요청은 좁은 첫 읽기 경로입니다. WBS XIX.9.3 전체의 검증·dry-run tool은 구현하지 않았으며 상태를 `[x]`로 올리지 않습니다.
- 임의 경로를 받지 않으라는 요청을 적용해 실행 인자는 루트 경로 대신 기존 신뢰 등록의 ID입니다. 해석한 루트 하나를 세션에 고정하므로 ADR-0005의 고정 프로젝트 경계를 유지합니다.
- 직접 `tools/stellar-mcp → ops` edge를 새로 허용하지 않았습니다. 필요한 library 이동은 기존 ADR-0005 §8·XIX.9.1 경로를 따른 것으로 ADR·서명·승인 정책 변경은 없습니다.

## 남은 위험 / 한계

- XIX.9.1의 agent-protocol 등록·공용 정의 이동, XIX.9.2 카탈로그 생성은 후속입니다. 현재 catalog는 이동한 mcp-core 안에 있고 기존 golden을 유지합니다.
- 앱의 미저장 문서·현재 선택·current document는 읽지 않습니다. token/component/target-profile/Git 상태·semantic diff·검증·dry-run·모든 쓰기·attached resource 전달은 제공하지 않습니다.
- 신뢰 등록 UI와 앱의 grant 호출 연결은 이번 범위가 아닙니다. 기존 등록이 없거나 위치/ID가 불명확하면 비활성으로 남습니다. 동일 ID의 신뢰된 복사본이 둘 이상이면 하나를 임의 선택하지 않습니다.
- 문서 page는 기존 ops `DocumentTree`의 문서 단위 로딩을 재사용합니다. 출력은 bounded지만 각 요청의 메모리와 계산량은 문서 크기에 비례하며, lazy node paging·캐시는 후속입니다.
- 신뢰 확인과 snapshot 응답은 OS 파일 시스템의 원자적 승인 transaction이 아닙니다. 요청 전후 재확인과 ops의 고정 generation 일관성을 사용하며 watcher 기반 즉시 종료·강한 원자성은 구현하지 않았습니다.
- Phase XIX 전체 완료나 앱 번들 배포·외부 클라이언트 설정 설치를 의미하지 않습니다. 다른 worktree·작업자·예약 실행·push·계정·서명 작업은 수행하지 않았습니다.
