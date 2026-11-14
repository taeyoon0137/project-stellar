# 작업 보고서: XXIV.4.2 headless 조회·편집 공용 library `stellar-ops`

- WBS: XXIV.4.2
- 작업명: `crates/ops`(`stellar-ops`)를 추가하고 crate graph 정책에 등록, 앱 없이 프로젝트를 열어 revision을 포함한 읽기 전용 snapshot 반환
- 상태: 부분 완료 (`[~]`) — crate·정책 등록·snapshot·검증은 끝났으나, 작업 지시의 "crate graph **문서**에 등록"(`docs/architecture/crate-dependency-graph.md`)은 이 lane의 수정 허용 경로 밖이라 반영하지 못했습니다. 반영할 문안은 아래 "남은 위험 / 한계"에 있습니다.
- 선행: XXIV.1.2(`[x]`), VII.1.6(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/Cargo.toml`(신규) — package `stellar-ops`. 직접 의존 project·document·storage·semantic-diff(ADR-0005 §8 허용 목록의 부분집합), `test-support` feature(document test-support 전달만).
- `crates/ops/src/lib.rs`(신규) — crate 문서(L1, 금지 도달 대상, 읽기 경로 무변경), `core_types` 재노출(semantic-diff 경유, 직접 edge 없음).
- `crates/ops/src/revision.rs`(신규) — `ProjectRevision`(Unsaved / Saved{generation, manifest_hash}), 표기 `g0`·`g<n>:<content hash>`와 엄격한 parser.
- `crates/ops/src/error.rs`(신규) — `OpenError`, `DocumentReadError`.
- `crates/ops/src/snapshot.rs`(신규) — `open_snapshot`, `ProjectSnapshot`(manifest, git link, project id, revision, 문서 정의, chunk kind별 개수, 문서 레코드 lazy 읽기).
- `crates/ops/src/test_support.rs`(신규) — 테스트 전용 fixture(scaffold+`.git`, 실제 저장 파이프라인 저장, 파일 digest). `#![cfg(any(test, feature = "test-support"))]`.
- `Cargo.toml` — workspace members에 `crates/ops` 추가.
- `Cargo.lock` — `stellar-ops` package 항목.
- `tools/xtask/src/crate_graph.rs` — `allowed_edges()`에 `stellar-ops` 항목(ADR-0005 §8 허용 8개) 추가, `ops_edges_follow_adr_0005` test.
- `docs/task-reports/XXIV.4.2.md`(신규), 계획서(XXIV.4.2 `[ ]`→`[~]`).

## 구현 요약

- **열기 순서(`open_snapshot`)**: 경로 metadata(디렉터리 확인) → `probe_project`(IV.5.1: root 검증, `stellar.yaml` loader 상한까지만 읽기, 스키마 검증) → `parse_project_id`(IV.2.3) → `LazyGeneration::open`(V.4.4 검증, V.6.6 DocumentManifest chunk만 읽음).
  - current pointer가 **없을 때만** 저장된 generation이 없는 프로젝트(`ProjectRevision::Unsaved`, 표기 `g0`)로 봅니다. 형식이 틀린 pointer·누락 manifest·hash 불일치 등은 `OpenError::Storage`로 거부합니다(Unsaved로 숨기지 않음).
- **revision**: headless 경로에는 앱 창의 인메모리 `DocumentRevision`이 없으므로, 파일이 원본이라는 ADR-0005 §1에 맞춰 current pointer가 가리키는 generation 번호와 그 manifest content hash(ADR-0004)를 함께 revision으로 씁니다. hash는 열린 manifest를 canonical encode해 다시 계산하며, fixture에서 current pointer 값과 일치함을 확인했습니다. 같은 번호라도 다른 writer의 generation은 hash가 달라 구분됩니다.
- **snapshot 내용**: manifest, `.git` 링크 종류, project id, revision, 문서 정의(DocumentManifest chunk 순서), chunk kind별 개수(V.6.1 이름), `document_records(id)`로 그 문서의 chunk만 읽어 canonical 정렬한 영속 레코드. 모든 결과는 열기 시점에 고정한 generation 하나에서 나옵니다.
- **읽기 전용**: 이 경로는 쓰기 API를 호출하지 않습니다. fixture에서 실행 전후 프로젝트 모든 파일 hash가 같음을 확인했습니다.
- **crate graph**: `stellar-ops` 허용 직접 의존을 ADR-0005 §8 목록(project, document, commands, history, storage, semantic-diff, semantic-merge, diagnostics)으로 등록했습니다. 전이 도달 금지 규칙은 XXIX.1.1이 이미 `reach::FORBIDDEN_REACH`에 `stellar-ops` 항목으로 넣어 두었고, 이번에 crate가 생기면서 적용이 시작됐습니다.
- core-types는 ADR 허용 목록에 없어 직접 의존하지 않고, semantic-diff의 `core_types` 재노출을 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 7 passed, 0 failed |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-ops --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-ops -p xtask -- --check` | 통과 |
| `cargo test -p xtask --lib crate_graph` | 26 passed(`ops_edges_follow_adr_0005` 포함) |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-dependencies` | 종료 코드 0 |
| `cargo xtask check-core` | 종료 코드 0(첫 실행은 `stellar-document`의 `workspace_production_code_does_not_use_test_support`가 `crates/ops/src/test_support.rs`를 프로덕션 코드로 판정해 실패 → 파일 첫 줄에 `#![cfg(any(test, feature = "test-support"))]`를 두어 해결 후 재실행 통과) |

| 검사 | 결과 |
|---|---|
| 앱 미실행 snapshot revision이 current pointer·generation 번호와 일치, 두 번째 저장 뒤 revision 변경(`revision_matches_current_pointer_and_generation_without_app`) | 통과 |
| 저장 전 프로젝트 `g0`·문서 0(`unsaved_project_has_g0_revision_and_no_documents`) | 통과 |
| 문서 레코드 왕복·없는 문서 거부(`document_records_round_trip_and_unknown_document_is_rejected`) | 통과 |
| 실행 전후 프로젝트 파일 hash 동일(`open_snapshot_never_writes_project_files`) | 통과 |
| 경로 없음·파일·root 아님·손상 pointer 오류 분류(`open_errors_are_typed`) | 통과 |
| revision 표기 왕복·잘못된 표기 거부(`token_round_trips`, `malformed_tokens_are_rejected`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- crate 의존 그래프 문서(`docs/architecture/crate-dependency-graph.md`) 등록을 하지 않았습니다. 이 실행의 수정 허용 경로에 포함되지 않기 때문이며, 그래서 상태를 `[~]`로 두었습니다.
- 허용 의존 8개 중 이 작업이 실제로 쓰는 4개(project, document, storage, semantic-diff)만 manifest에 넣었습니다. 나머지(commands, history, semantic-merge, diagnostics)는 후속 작업(XXIV.4.4, XXIV.1.3)이 쓰기 시작할 때 추가합니다.

## 남은 위험 / 한계

- **후속(이 행을 `[x]`로 올리기 위한 남은 일)**: `docs/architecture/crate-dependency-graph.md` 허용 의존 표에 `| \`stellar-ops\` | project, document, commands, history, storage, semantic-diff, semantic-merge, diagnostics |` 행과 Mermaid edge(`ops --> project` 등 8개)를 추가하고, 규칙 절에 "ops는 local-ipc·agent-protocol·mcp-core·agent-host·ffi에 전이적으로도 도달하지 않음(ADR-0005 §8, `reach::FORBIDDEN_REACH`)"을 적어야 합니다.
- `reach::FORBIDDEN_REACH`의 `stellar-ops` 금지 목록에는 ADR의 `mcp-core` 대신 현재 package 이름 `stellar-mcp`가 있습니다. `stellar-mcp-core`가 생기는 XIX.9.1에서 이름을 함께 맞춰야 합니다(`reach.rs`는 이 lane 허용 경로 밖).
- token·component 정의는 아직 저장 포맷(Phase VIII·IX)이 없어 snapshot은 chunk kind 개수만 보고합니다.
