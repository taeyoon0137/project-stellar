# 작업 보고서: XIX.9.3 승인 프로젝트의 headless MCP 읽기 연결

- WBS: XIX.9.3
- 작업명: 기존 신뢰 프로젝트 하나의 snapshot 읽기를 정상 stdio CLI에 연결
- 상태: 부분 완료 (`[~]`) — 이번 수직 경로는 구현·검증했으며, 같은 프로젝트의 Git HEAD 비교까지 연결했습니다. 다른 Git revision, dry-run tool과 agent-protocol 카탈로그 이동은 남았습니다.
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
| `initialize` | 기존 protocol 협상, `resources: {}`와 `tools: {}` capability 광고 |
| `resources/list` | `stellar://project/structure`, `stellar://components` |
| `resources/templates/list` | 기존 `document`, `stellar://documents/{document}{?cursor,limit}` 한 항목 |
| `resources/read` 구조 | 저장된 문서 ID·이름, page·View 개요, chunk 종류별 개수, revision |
| `resources/read` 컴포넌트 | 프로젝트 안의 개별 Git 원본에서 component ID·이름, 루트 ID·크기, Frame 수, 파일 hash를 읽음 |
| `resources/read` 문서 | 승인 프로젝트의 저장된 문서 트리 page: node ID·부모·View·깊이·종류·이름·표시/잠금 여부·자식 수, revision·cursor |
| `tools/list` | `document.semantic_diff`, `document.validate`(읽기 전용) |
| `tools/call` | 저장 문서의 Git HEAD 대비 변경 목록 또는 전체·선택 문서의 검증 진단. 미등록 tool 이름은 `-32602` |

- revision은 ops의 `token`, `generation`, `manifest_hash` 그대로입니다. 문서 읽기는 하나의 디스크 generation을 사용합니다. 다음 요청에서 정상 저장이 관찰되면 이전 revision의 cursor는 기존 `PageError::StaleCursor`로 거부합니다(`-32602`).
- 문서 page는 기존 XIX.2.2의 기본 500개, 최대 2,000개, 내용 JSON 최대 512 KiB를 적용합니다. byte 한도에 맞춰 page를 줄이고, 한 item이 한도를 넘으면 오류입니다. 인코딩된 resource envelope에도 stdio 1 MiB 한도를 검사합니다.
- project structure와 components는 기존 catalog에서 비페이지 resource입니다. URI·결과 schema에 임의 pagination 필드를 추가하지 않고, 내용이 512 KiB를 넘으면 오류로 닫습니다.
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

새 프로세스 테스트는 2,101개 node의 개수 제한 pagination, 긴 정상 이름의 byte 제한 pagination, 단일 item 초과 오류, 정상 저장 뒤 stale cursor, 다른 프로젝트 ID·선언 밖 인자, tool 호출 거부, 신뢰 철회·불명확·중복 ID·매니페스트 ID 변경을 확인합니다. 각 단계 전후 임시 프로젝트·신뢰 저장소·HOME 파일 목록과 content hash가 같습니다(테스트가 의도적으로 한 저장·grant/revoke 전후는 별도 기준). 기존 catalog의 20만 node pagination 테스트도 유지합니다. 최초 연결 구현 시점에는 성능 측정을 하지 않았습니다. 아래 후속 측정은 `c7d5308b`와 페이지 캐시 적용 결과를 비교합니다.

## 후속: 승인 headless 페이지 캐시와 release RTT (2026-11-14)

- 기준: `c7d5308b697348d173be3ce1ec97ab24f2ce2b3b`, 전용 checkout/브랜치 `stellar-takeover-page-cache`. 추가 에이전트·원격·push·서명/계정 변경 없이 진행했습니다. 모든 읽기·상태 이동은 임시 프로젝트와 임시 신뢰 저장소로 검증했습니다.
- 변경 파일: `crates/mcp-core/src/headless.rs`, `catalog/paging.rs`, `crates/ops/src/trusted.rs`, `query.rs`, `test_support.rs`, `tools/stellar-mcp/tests/headless_read.rs`, `connection_info.rs`, 이 보고서. 기본 실행·attach·소켓 경로, 승인 정책, 저장 포맷, resource schema와 cursor 형식은 바꾸지 않았습니다. WBS 상태는 부분 완료로 유지합니다.

### 재사용 조건과 로딩 비용

- `Headless` 인스턴스가 검증된 snapshot 하나와 최근 문서 tree 하나만 소유합니다. 프로젝트 ID·canonical target·승인 scope·session은 생성 시 고정되고 다른 연결과 cache를 공유하지 않습니다. 문서를 전환하면 기존 tree를 먼저 버립니다.
- 매 read의 시작과 결과 반환 직전에 기존 trust store, 유일한 등록 target, canonical 경로, 실제 `project.id`를 확인합니다. 재사용에는 snapshot root·project ID·`.git` 링크 종류·파싱된 `stellar.yaml` **전체 값**·`design/current` hash 일치가 필요합니다. 파일 크기/mtime만 비교하지 않습니다. 프로젝트 이름·target profile·dependency 설정은 generation hash와 독립이므로 설정 값이 바뀌면 snapshot과 tree를 모두 다시 엽니다.
- current pointer 부재는 기존 unsaved 의미(`g0`)를 유지하며, 읽기/형식 오류는 실패합니다. source가 응답 준비 중 달라지거나 신뢰/identity 확인이 실패하면 준비한 내용을 반환하지 않습니다. 실패한 read의 snapshot/tree는 보관하지 않으며, 목록 조회에서 신뢰 확인이 실패해도 cache를 폐기합니다. 다음 성공 요청은 새로 엽니다.
- cursor의 limit·문서 key·revision 검사를 기존 paging 구현에서 분리해 `build_page`와 headless가 공유합니다. 다른 generation의 cursor는 snapshot만 확인하고 큰 문서 tree를 읽기 전에 거부합니다. offset 상한·개수·byte 한도·응답 envelope 검사는 기존 계약을 유지합니다.
- `DocumentTree`는 기존 `document_model` loader의 hash/참조/문서 불변식 검증을 그대로 통과한 모델에서 공통 필드만 추출합니다. 전체 payload의 `to_records` 복제·canonical 재정렬을 생략하고 응답용 행만 보관합니다. 기존 View 정의 순서와 형제 `(OrderKey, StableId)` 전위 순회를 유지합니다. `node(index)`는 같은 행을 빌려 주므로 MCP에서 node마다 `TreeCursor`와 `page(1)` Vec를 만들지 않습니다.
- 테스트 전용 loading counters는 실제 `TrustedProject::snapshot`/`DocumentTree::load` 호출 직전에 증가합니다(시스템 호출 또는 byte 계측은 아님). 첫 문서·후속 페이지·첫 페이지 재조회·구조 조회는 `(snapshot=1, tree=1)`, A→B→A 전환은 `(1,3)`입니다. 같은 generation의 정상 설정 변경은 `(2,4)`로 재로드하고 열린 snapshot의 새 이름을 확인합니다. 저장 뒤 stale cursor에서는 snapshot만 1회 증가하며 tree는 증가하지 않습니다. chunk 읽기 실패·신뢰 철회/재승인·ID 변경/복구·프로젝트 경로 일시 부재·current 부재/복구·새 연결에서도 이전 cache의 잔류가 없음을 검사합니다.

### 측정 방법과 결과

환경은 Apple M3 Max arm64, macOS 27.2 (26B123), rustc 1.96.0, Cargo release입니다. 정식 `headless_read::release_page_rtt` ignored 테스트를 추가했습니다. 임시 fixture에 50,000개 node를 실제 저장하고 기존 신뢰 API로 승인한 뒤, 정식 stdio `initialize → initialized → resources/read`를 호출합니다. 기준 실행 파일은 제품 코드를 수정하기 전에 release로 빌드해 임시 위치에 보존했습니다.

최종 비교는 **같은 fixture에서 이전/이후 실행 순서를 번갈아** 각 10개 새 프로세스를 실행했습니다. 페이지 크기는 모두 500개입니다. 새 연결의 첫 read는 구조 조회 없이 문서 첫 페이지를 요청하며, 각 연결에서 offset 500·1000·1500·2000·2500의 후속 페이지 5개와 첫 페이지 재조회를 측정합니다. 전체 node 수, 각 페이지 500개, offset, frame/내용 byte 상한과 fixture 전체 파일 hash 무변경을 함께 확인합니다.

RTT는 요청 직렬화·stdio 송신부터 응답 줄 수신 및 JSON-RPC envelope 파싱까지입니다. 프로세스 시작·initialize·fixture 생성/저장, `contents.text` 내부 JSON의 두 번째 파싱은 제외합니다. p50/p95는 nearest-rank이며 첫 페이지/재조회는 각 n=10, 후속은 n=50입니다.

| 경로 | 이전 p50 / p95 (ms) | 이후 p50 / p95 (ms) | p50 감소 |
|---|---|---|---|
| 새 연결 첫 문서 페이지 | 90.518 / 121.423 | 79.719 / 85.192 | 11.93% |
| 같은 문서 후속 페이지 | 83.567 / 94.373 | 2.296 / 2.473 | 97.25% (36.40배) |
| 같은 연결 첫 페이지 재조회 | 82.525 / 91.875 | 2.255 / 2.398 | 97.27% (36.60배) |

재현 명령(환경 변수는 테스트 harness의 기준 **실행 파일**만 고르며 제품 backend의 프로젝트 경로 override가 아닙니다):

```sh
# 제품 코드 변경 전 c7d5308b release 실행 파일 보존
cargo build --release -p stellar-mcp
cp target/release/stellar-mcp /tmp/stellar-page-cache-baseline-bin
# 변경 적용 후 동일 harness로 교차 측정
STELLAR_MCP_BENCH_BASELINE=/tmp/stellar-page-cache-baseline-bin cargo test --release -p stellar-mcp --test headless_read release_page_rtt -- --ignored --exact --nocapture
```

환경 변수를 생략하면 현재 빌드만 측정합니다. 각 개별 sample과 p50/p95가 stdout에 출력됩니다. 최종 교차 실행 전의 독립 실행에서는 기준 첫 페이지 p50이 178.079ms, 다음 순차 비교에서는 100.584→78.717ms로 변했습니다. 환경 변동을 숨기지 않기 위해 최종 표는 순서를 번갈아 측정한 결과만 사용합니다. OS 파일 cache를 비우지 않았으므로 첫 페이지는 **연결 cache가 비어 있는 상태**이며 cold-disk 측정이 아닙니다. 작은 표본·로컬 한 기기이며 CI/LAB 성능 게이트나 전체 성능 목표 달성을 주장하지 않습니다. 최초 로드는 전체 문서 검증과 공통 필드 추출을 계속 수행하고, 유지 메모리/peak RSS는 측정하지 않았습니다.

### 후속 검증

| 명령 | 결과 |
|---|---|
| `cargo test --release -p stellar-mcp -p stellar-mcp-core -p stellar-ops --no-fail-fast` | 0, MCP 프로세스·통합 35개, core 58개, ops 41개 통과. 수동 RTT 1개 ignored |
| `cargo test --release -p stellar-cli --lib --test output_fixtures --test headless_help` | 0, lib 24개·도움말 6개·출력 fixture 24개 통과(20만 node 포함) |
| 위 release RTT 교차 측정 | 0, 기준/이후 각각 70회 read, byte 제한·파일 hash 검증 통과 |
| `cargo clippy -p stellar-mcp -p stellar-mcp-core -p stellar-ops --all-targets -- -D warnings` | 0, 경고 없음. 수동 RTT의 상수 assert 경고를 release 조건 분기로 바꾼 뒤 재검증 |
| `cargo fmt -p stellar-mcp -p stellar-mcp-core -p stellar-ops -- --check` | 0 |
| `target/debug/xtask validate-crate-graph`, `validate-dependencies`, `validate-versions` | 각각 0 |
| `target/debug/xtask validate-task-status`, `sync-plan` | 각각 0 |
| `target/debug/xtask validate-task-report docs/task-reports/XIX.9.3.md`, `validate-adr <변경 8개 경로>` | 각각 0 |
| `git diff --check` | 0 |

첫 검증에서 테스트 파일을 제품 코드로 읽은 mutation inventory 오류는 테스트를 파일 끝 모듈로 옮기고, 계측용 `cfg(test)` field 뒤 제품 코드도 검사하도록 범위를 보완한 뒤 통과했습니다. chunk 실패 테스트의 처음 seed와 기존 순회 테스트의 seed 77은 빈 문서였습니다. node 존재를 assert하는 고정 seed 91로 바꿔 실제 읽기 실패/복구와 행 필드/형제 순서 비교가 빈 반복문으로 통과하지 않게 했습니다.

추가로 시도한 `cargo test --release -p stellar-mcp -p stellar-mcp-core -p stellar-ops -p stellar-cli --no-fail-fast`는 CLI의 기존 `tests/json_copy_sync.rs:24`가 삭제된 `tools/stellar-mcp/src/json.rs`를 `include_str!`하여 컴파일 실패했습니다. 해당 경로가 `c7d5308b`에도 남아 있음을 `git show`로 확인했습니다. 이번 변경과 무관한 파일은 수정하지 않았고, 영향 있는 CLI query 출력은 위 별도 fixture 실행으로 검증했습니다.

검증된 content-addressed generation/chunk는 불변이라는 기존 storage 계약을 사용합니다. 동일 hash 경로의 객체를 매 cache hit마다 다시 읽거나 존재/권한을 전수 검사하지 않습니다. 정상 저장의 pointer 전환과 설정 변경은 확인하지만, hash를 유지한 객체 직접 수정·삭제를 즉시 감시하는 기능은 아닙니다. 신뢰·source 검사와 반환은 파일시스템 전체의 원자적 transaction이 아니며 최종 검사 직후의 경합까지 없애지는 못합니다.

## 저장 문서 검증 tool

`document.validate`는 XIX.2.3 카탈로그의 기존 Read tool입니다. 선택한 문서 또는 전체 프로젝트의 진단을 ops의 CLI validator에서 읽고, 같은 대상·revision의 후속 페이지에는 다시 검증하지 않습니다. 캐시는 이 연결의 최근 검증 대상 하나만 보관합니다. 각 페이지에서 resource 조회와 같은 프로젝트 신뢰·identity·현재 snapshot 재확인 절차를 거칩니다. 새 generation 또는 대상 변경 시 재검사합니다. 진단이 없으면 `items: []`, `next_cursor: null`입니다. 경고와 문서 오류는 tool이 성공적으로 돌려준 검사 결과에 포함됩니다.

- 실제 stdio 통합 검증은 저장된 문서 2개, 서로 다른 미해결 참조 5개의 단일 문서를 대상으로 전체 검사와 문서 필터, 정상·손상 문서의 valid/errors, 2·2·1개 페이지, 범위 밖 문서, 과다 limit, 저장 후 cursor, 신뢰 철회, 파일 목록·hash 보존을 확인합니다. 재현 명령은 `cargo test -p stellar-mcp --test headless_read`입니다.
- 저장된 프로젝트 간 비교는 CLI의 `semantic-diff <base 경로> <target 경로>`가 지원합니다. MCP 카탈로그의 `document.semantic_diff`는 같은 신뢰 프로젝트의 Git revision을 기준으로 삼습니다. HEAD의 저장 세대를 여는 경로는 아래 후속 구현에서 연결했습니다.
- 첫 검증은 대상 문서들의 모든 레코드를 읽습니다. 페이지 크기는 출력량을 제한합니다. 서버가 최근 검사에서 기억하는 진단 수의 합과 처음 검증하는 비용은 대상에 비례합니다.

### 기존 문서 트리 RTT 재검증

Apple M3 Max에서 기준 커밋 `47cba3e6`의 release 바이너리와 변경 후 바이너리를 같은 5만 node 저장 문서에 번갈아 연결했습니다. `STELLAR_MCP_BENCH_BASELINE=<기준 바이너리> cargo test --release -p stellar-mcp --test headless_read release_page_rtt -- --ignored --nocapture`를 두 번 실행했습니다. 각 회차·바이너리마다 새 프로세스 10개에서 다음 페이지 50개(500 node/페이지)를 읽었습니다.

| 다음 페이지 왕복 | 기준 p50 / p95 | 변경 후 p50 / p95 |
|---|---:|---:|
| 1회차 | 2.567 / 4.119 ms | 2.783 / 7.189 ms |
| 2회차 | 3.794 / 18.326 ms | 3.252 / 6.094 ms |

같은 장비에서 회차 간 p95 편차가 있어 고정 지연 상한이나 개선율을 확정하지 않았습니다. 모델 로딩 횟수 검사는 캐시된 문서 조회 사이에 다른 문서의 검증을 호출해도 원래 문서의 snapshot과 tree를 각각 한 번만 로드함을 확인합니다.

## 차이(작업 지시와 다른 점)

- 사용자 요청은 좁은 첫 읽기 경로입니다. HEAD 비교·검증 연결 후에도 다른 Git revision 비교와 dry-run이 남아 XIX.9.3은 부분 완료입니다.
- 임의 경로를 받지 않으라는 요청을 적용해 실행 인자는 루트 경로 대신 기존 신뢰 등록의 ID입니다. 해석한 루트 하나를 세션에 고정하므로 ADR-0005의 고정 프로젝트 경계를 유지합니다.
- 직접 `tools/stellar-mcp → ops` edge를 새로 허용하지 않았습니다. 필요한 library 이동은 기존 ADR-0005 §8·XIX.9.1 경로를 따른 것으로 ADR·서명·승인 정책 변경은 없습니다.

## 남은 위험 / 한계

- XIX.9.1의 agent-protocol 등록·공용 정의 이동, XIX.9.2 카탈로그 생성은 후속입니다. 현재 catalog는 이동한 mcp-core 안에 있고 기존 golden을 유지합니다.
- 앱의 미저장 문서·현재 선택·current document는 읽지 않습니다. token/target-profile/Git 상태·임의 Git revision 비교·dry-run·모든 쓰기·attached resource 전달은 제공하지 않습니다.
- 신뢰 등록 UI와 앱의 grant 호출 연결은 이번 범위가 아닙니다. 기존 등록이 없거나 위치/ID가 불명확하면 비활성으로 남습니다. 동일 ID의 신뢰된 복사본이 둘 이상이면 하나를 임의 선택하지 않습니다.
- 문서 page는 기존 ops의 검증된 문서 단위 로딩을 재사용합니다. 후속 변경으로 연결별 snapshot 하나와 최근 문서의 트리 행만 유지합니다. 최초 로드·문서 전환·원천 변경 뒤 로드는 여전히 문서 크기에 비례하며, node 단위 lazy paging·RSS 측정은 후속입니다.
- 신뢰 확인과 snapshot 응답은 OS 파일 시스템의 원자적 승인 transaction이 아닙니다. 요청 전후 재확인과 ops의 고정 generation 일관성을 사용하며 watcher 기반 즉시 종료·강한 원자성은 구현하지 않았습니다.
- Phase XIX 전체 완료나 앱 번들 배포·외부 클라이언트 설정 설치를 의미하지 않습니다. 다른 worktree·작업자·예약 실행·push·계정·서명 작업은 수행하지 않았습니다.


## 같은 프로젝트의 Git HEAD 비교

`tools/list`에 `document.semantic_diff`를 추가했습니다. `project`의 기존 Git provider가 HEAD를 commit/tree로 고정하고, 저장된 매니페스트 ID를 확인한 뒤 current 세대가 참조하는 chunk만 프로젝트 외부 임시 디렉터리에 복원합니다. `ops`의 문서 semantic projection과 diff 엔진으로 작업 트리의 고정 generation을 비교합니다. cache와 페이지 커서는 commit ID와 저장 revision 모두에 종속됩니다.

실제 임시 Git 브랜치 두 개로 `tools/stellar-mcp/tests/headless_read.rs`의 `head_diff_pages_follow_real_branches_and_preserve_all_inputs`를 실행했습니다. 변경 없음, 한글 노드 이름의 이전·이후 값, 개수·512 KiB 제한에 따른 페이지와 단일 대형 항목의 오류, 작업 트리를 유지한 HEAD 이동, 후속 저장, 다른 project ID와 신뢰 철회를 확인합니다. 프로젝트 파일·index·refs·신뢰 등록 내용은 요청 전후 byte hash로 대조합니다. 커밋이 없거나 한쪽에 유효한 저장 세대가 없으면 비교할 수 없습니다. 이 검사는 문서 generation만 비교하며 프로젝트 매니페스트 설정·독립 텍스트 원본을 비교하지 않습니다.

50,000개 노드의 기존 문서 읽기 release stdio 검사에서 500개 항목의 후속 페이지 RTT는 p50 2.367 ms, p95 3.718 ms(50회)였습니다. 첫 페이지 p95는 98.429 ms(10회)입니다. 측정 대상은 문서 resource이며 Git 비교 시간은 포함하지 않습니다.

## 컴포넌트별 Git 원본 조회

`resources/read stellar://components`는 고정한 신뢰 프로젝트의 `design/components`에서 v1·v2 YAML을 읽습니다. 구성 원본만 바뀌면 문서 generation은 같으므로 각 항목의 `source_hash`를 파일 버전으로 보고합니다. `cargo test -p stellar-mcp --test headless_read`의 실제 stdio 검사는 하나의 연결에서 빈 목록, 디스크 파일 추가, Git 브랜치 전환, 커밋 전 편집, 신뢰 철회를 순서대로 확인합니다. 읽기 전후 Git 및 프로젝트·사용자 데이터의 파일 digest를 대조합니다. [원본 선택과 조회 제한](../spec/component-source-v1.md)
