# 작업 보고서: XXIV.4.3 `stellar inspect` 확장과 `stellar query` 추가

- WBS: XXIV.4.3
- 작업명: 프로젝트 구조·문서 트리(cursor 분할)·node 속성·token·component를 revision과 함께 JSON으로 출력하고, `stellar-cli` 직접 의존을 `stellar-ops`·diagnostics로 축소
- 상태: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 구현과 작업 행의 검증 기준은 모두 통과했으나, 선행 XXIV.4.2가 `[~]`입니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`가 아니면 완료로 두지 않음)에 따라 XXIV.4.2가 `[x]`가 될 때까지 `[~]`로 둡니다.
- 선행: XXIV.4.1(`[x]`), XXIV.4.2(이 lane에서 구현·commit, `[~]` — 남은 일은 crate graph 문서 반영뿐이며 이 작업이 쓰는 코드는 모두 있음)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/query.rs`(신규) — `DocumentTree`·`document_tree_page`(전위 순회 page, `TreeCursor`), `node_detail`(공통 필드 + semantic snapshot 속성 투영), `project_structure`, `tokens`·`components`, `OpsValue`, `property_path_label`.
- `crates/ops/src/snapshot.rs` — `probe`(문서 chunk를 읽지 않는 IV.5.1 검사)·`ProjectProbe` 분리, `open_snapshot`이 이를 사용.
- `crates/ops/src/lib.rs` — `query` 모듈, `types` 재노출(Manifest·GitLink·RootError·LoaderError 등).
- `crates/ops/src/test_support.rs` — `large_document(n)`(정확히 n node 대형 문서 fixture).
- `tools/stellar-cli/Cargo.toml` — 직접 의존을 `stellar-project`에서 `stellar-ops`로 교체, dev 의존 `stellar-ops`(test-support).
- `tools/stellar-cli/src/commands.rs` — `inspect`가 ops `probe`를 쓰고 `--structure` 추가, `query` 구현.
- `tools/stellar-cli/src/views.rs`(신규) — ops 결과 → 결과 JSON 변환, 열기·조회 오류의 종료 코드 분류.
- `tools/stellar-cli/src/cli.rs`, `src/lib.rs` — `query` 등록, `QUERY_TARGETS` 도움말, 사용법 오류 test 확장.
- `tools/stellar-cli/tests/output_fixtures.rs` — inspect 호환·`--structure`·query 성공/오류·20만 node 명령 test, `STELLAR_CLI_BLESS` golden 갱신 스위치.
- `tools/stellar-cli/tests/headless_help.rs` — 허용 직접 의존 목록을 ops·diagnostics로, dev 의존 검사 추가.
- `tools/stellar-cli/tests/fixtures/help.txt` — 새 명령·query 대상이 들어간 help snapshot.
- `tools/stellar-cli/tests/fixtures/results/`(신규 12개) — `inspect-structure`, `query-project`, `query-project-unsaved`, `query-tree-first`, `query-tree-next`, `query-node`, `query-tokens`, `query-components`, `query-unknown-document`, `query-unknown-node`, `query-invalid-cursor`, `query-stale-cursor`.
- `tools/xtask/src/crate_graph.rs` — `stellar-cli` 허용 의존을 `stellar-ops`·`stellar-diagnostics`로 갱신, test 갱신.
- `tools/xtask/src/crate_graph/manifest.rs` — `real_workspace_scan_covers_crates_and_tools`의 표본 edge 단언 1줄을 `stellar-cli -> stellar-project`에서 `stellar-cli -> stellar-ops`로 변경(아래 "차이" 참조).
- `Cargo.lock`, `docs/task-reports/XXIV.4.3.md`(신규), 계획서(XXIV.4.3 `[ ]`→`[x]`).

## 구현 요약

- **명령**
  - `stellar inspect <경로>`: 옵션이 없으면 XXIV.1.2 결과와 byte 단위로 같습니다(문서 chunk를 읽지 않음). `--structure`를 주면 같은 결과 뒤에 `revision`·`documents`(페이지·View 개요)·`chunks`(kind별 개수) key를 덧붙입니다.
  - `stellar query <경로> project | tree <문서 id> [--cursor <c>] [--limit <n>] | node <문서 id> <node id> | tokens | components`.
  - 모든 query 결과와 `--structure` 결과에 `revision`(`token`, `generation`, `manifest_hash`)이 들어갑니다. revision은 XXIV.4.2 정의(current generation 번호 + manifest content hash)입니다.
- **문서 트리 page**: 정의 순서 View별 깊이 우선 전위 순회, 형제는 (OrderKey, StableId) 순. 기본 1000, 상한 10000 node. `next_cursor`는 `t1.<offset>.<문서 id>.<revision>`이며, 다른 문서·다른 revision(다른 writer가 저장한 뒤)·범위 밖 cursor는 거부합니다. 대형 문서를 한 응답에 모두 담지 않습니다.
- **node 속성**: 공통 필드(부모, 종류, 표시 이름, visible·locked, OrderKey 16진수, 자식 순서, modifier 수)와 semantic snapshot 투영(XIV.3.1) 속성 목록(`path` = PropertyKey 4자리 16진수 `/` 연결 + tail, `type`, `value`). typed·bytes 값은 canonical bytes 16진수입니다.
- **token·component**: 아직 프로젝트 저장 포맷(Phase VIII·IX)이 없어 항목을 해석할 수 없으므로 `entries`는 비어 있고 해당 chunk kind 개수만 사실대로 보고합니다(아래 "차이").
- **종료 코드**: 기존 inspect 분류 유지. 없는 문서·node 66, cursor 형식·문서 불일치·범위 밖 65(`invalid-cursor`), 낡은 cursor 65(`stale-cursor`), 잘못된 id·`--limit` 64, 저장소 손상 65(`storage`).
- **읽기 전용·앱 불필요**: 조회는 ops snapshot 하나에서 나오고 쓰기 API를 부르지 않습니다. 실행 전후 프로젝트 파일 hash를 비교해 확인했습니다.
- **의존 축소**: `stellar-cli` `[dependencies]`는 `stellar-ops` 하나, `[dev-dependencies]`도 `stellar-ops`(test-support)뿐입니다. crate graph 정책의 `stellar-cli` 허용 의존은 ops·diagnostics입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 13 passed, 0 failed |
| `cargo test -p stellar-ops` | 13 passed, 0 failed |
| `cargo clippy -p stellar-cli --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-cli -p stellar-ops -p xtask -- --check` | 통과 |
| `cargo test -p xtask` | lib 170 passed, 통합 2+2 passed |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-dependencies` | 종료 코드 0 |
| `cargo xtask check-core` | 종료 코드 0(약 5분) |
| `git diff --stat 24728e3 -- tools/stellar-cli/tests/fixtures/results/inspect-*.json tools/stellar-cli/tests/fixtures/results/version.json tools/stellar-cli/schema` | 변경 없음(기존 fixture 그대로) |

| 검사 | 결과 |
|---|---|
| JSON golden 일치: `inspect --structure`, query 7종 성공·4종 오류 fixture(`inspect_structure_and_query_success_fixtures`, `query_error_fixtures`) | 통과 |
| 20만 node 문서 page 분할: ops에서 200,000 node 전체를 10000씩 20 page로 순회, 중복·누락 0, cursor 문자열 왕복(`two_hundred_thousand_node_document_is_split_into_pages`, 약 16초) | 통과 |
| 같은 문서를 명령으로 첫 page(10000)·마지막 page(5000, `next_cursor` null) 조회(`query_tree_splits_a_two_hundred_thousand_node_document`) | 통과 |
| 실행 전후 프로젝트 파일 hash 동일(성공 fixture 전체 실행 후 비교, ops 200k 순회 후 비교) | 통과 |
| 기존 inspect fixture 9종 통과, 저장된 generation이 있는 프로젝트의 옵션 없는 inspect가 `inspect-valid.json`과 byte 일치(`inspect_without_option_keeps_the_xxiv_1_2_result`) | 통과 |
| 낡은 cursor·다른 문서 cursor·범위 밖 cursor·잘못된 limit 거부(ops `stale_foreign_and_out_of_range_cursors_are_rejected`, 명령 `query-stale-cursor.json`) | 통과 |
| 전위 순회 순서가 부모 우선·`children_in_order`와 일치(`preorder_visits_parents_before_children_in_order_key_order`) | 통과 |
| `stellar-cli` 직접·dev 의존이 ops뿐(`manifest_depends_only_on_core_crates`), 정책상 cli→project·document·storage·core-types 직접 edge 거부(`stellar_cli_uses_only_core_crates`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- `inspect` 확장은 `--structure` 옵션으로 켭니다. 옵션 없는 결과를 바꾸면 XXIV.1.2 golden fixture(byte 비교)가 깨지므로, "기존 inspect fixture 통과"와 "기존 결과 필드 호환"을 둘 다 지키려고 opt-in으로 두었습니다. 새 key는 기존 key 뒤에 붙습니다.
- token·component 항목은 저장 포맷이 아직 없어 빈 목록과 chunk 개수만 출력합니다. 포맷이 정해지면 `stellar_ops::query::tokens`·`components`가 항목을 채웁니다.
- `tools/xtask/src/crate_graph/manifest.rs`는 이 lane의 허용 경로(`crate_graph.rs`) 밖이지만, 실제 workspace scan test가 `stellar-cli -> stellar-project` edge를 표본으로 단언하고 있어 이 작업(그 edge 제거)과 직접 충돌했습니다. test 의도(scan이 cli manifest edge를 읽음)를 유지하도록 표본만 `stellar-cli -> stellar-ops`로 바꾼 1줄 변경입니다.

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: XXIV.4.2의 `docs/architecture/crate-dependency-graph.md` 등록을 마쳐 XXIV.4.2를 `[x]`로 올린 뒤, 이 보고서의 검증 명령을 다시 실행해 `[x]`로 바꿉니다.
- `tools/xtask/src/crate_graph.rs`의 기존 `stellar-cli` 허용 의존 항목을 고친 일은 작업 행이 요구한 변경("crate graph 정책의 `stellar-cli` 허용 의존도 ops·diagnostics로 갱신")이지만, lane 규칙(새 crate 항목만 추가)보다 넓습니다. `manifest.rs` 1줄 변경과 함께 통합 시 확인이 필요합니다.
- 한 page(최대 10000 node)의 JSON은 수 MB가 될 수 있습니다(200k fixture에서 10000 node page 약 2.8 MB). MCP처럼 메시지 상한이 있는 경로는 더 작은 limit을 써야 합니다(XIX.2.2·XIX.9.10에서 정함).
- 각 `query tree` 호출은 문서 전체 chunk를 읽어 순회 index를 만듭니다. 20만 node 문서에서 debug 빌드 기준 호출당 수 초가 걸리며, release 측정과 index 재사용은 하지 않았습니다.
- 속성 `path`는 PropertyKey 숫자 표기입니다. 사람이 읽는 이름 registry는 제공하지 않습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다.
- 근거: query·inspect 확장과 의존 축소가 코드에 있고, 남은 사유는 선행 XXIV.4.2뿐이었습니다. 이번 재판정에서 XXIV.4.2를 `[x]`로 판정했으므로 함께 `[x]`입니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/ops/src/query.rs`: `document_tree_page`(전위 순회, `TreeCursor`가 문서·revision·offset을 담고 다른 문서·낡은 revision·범위 밖을 거부), `node_detail`, `project_structure`, `tokens`·`components`(generation chunk 개요). 모두 `ProjectSnapshot` 하나에서 읽고 쓰기를 하지 않습니다.
- `tools/stellar-cli/Cargo.toml`: `[dependencies]`는 `stellar-ops` 하나, `[dev-dependencies]`도 ops(test-support)뿐입니다. 정책 `allowed_edges()`의 `stellar-cli`는 ops·diagnostics입니다.
- `tools/stellar-cli/tests/output_fixtures.rs`: inspect 호환 golden, query 성공·오류 golden, 20만 node page 분할 test.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 26, component_source 10, headless_help 6, json_copy_sync 2, output_fixtures 26, text_source 6, web_frame 7 passed, 0 failed |
| `cargo test -p stellar-ops` | 51 passed(20만 node page 순회 test 포함) |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

### 남은 조건과 finding

- token 항목은 아직 generation chunk 개요만 냅니다. 토큰 모델(VIII.5)은 `[x]`지만 토큰의 프로젝트 저장 포맷이 생기면 `query tokens`가 항목을 채워야 합니다(행 범위 밖 후속).
