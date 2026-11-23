# 작업 보고서: IX.11.21 제품 문서 migration registry

## WBS

- WBS: IX.11.21
- 작업명: 제품 문서 migration registry를 `crates/project`에 만들고 project 열기에 연결
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문). 보고 당시 기록: 부분 완료 (`[~]`) — registry·고정 decoder·열기 판정·메모리 migration은 들어왔습니다. `cargo xtask test-migrations`의 제품 registry 사용과 Semantic Diff·Merge의 옛 schema snapshot fixture는 소유 범위 밖(`tools/xtask`, `crates/semantic-diff`·`semantic-merge`)이라 하지 않았습니다(아래 "차이").
- 구현 커밋: `becdcefe`. 결정: [ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md) 결정 4.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/project/src/migration/mod.rs`(신규): `PRODUCT_SCHEMA_VERSION`(1), 제품 step 표 `PRODUCT_STEPS`(비어 있음), `product_registry`, `MigrationPolicy`, V.8.1 열기 판정 `decide`(Current·MigrateInMemory·Reject), `MigratedGeneration`(메모리 migration과 문서 로드).
- `crates/project/src/migration/decoders.rs`(신규): schema 1 Frame 고정 사본 `FrameV1`(현재 타입과 독립), kind별 고정 decoder 표 `decode_node_payload`, golden bytes 테스트.
- `crates/project/src/saved_project.rs`: `SavedProjectLoader::open`이 schema를 판정합니다. `SavedProject.schema`·`migrated`, `SavedProjectError::Schema`·`Migration`, `with_migrations`.
- `crates/project/src/lib.rs`: 모듈 등록 한 줄.
- `crates/ops/src/migrate.rs`: `CURRENT_SCHEMA_VERSION`·`product_registry`를 project migration에서 가져옵니다.
- `crates/project/tests/migration_open.rs`(신규), `crates/runtime/src/staging/tests.rs`(새 schema 거부 위치가 열기 단계로 옮겨진 것 반영).

## 구현 요약

- 제품 step은 표 하나만 registry에 들어가며 1부터 빈틈 없이 이어집니다. storage 시험용 noop step(1→2)은 제품 registry에 없습니다(테스트로 고정).
- 열기 판정: 같은 schema는 편집 가능, 낮은 schema는 경로가 있으면 메모리에서만 migration한 읽기 전용, 경로가 없거나 높은 schema는 거부. 열기는 디스크를 쓰지 않고 자동 commit하지 않습니다. Git snapshot loader(`for_materialized_snapshot`)도 같은 경로입니다.
- 고정 decoder: migration step은 옛 bytes를 그 schema의 사본으로 읽습니다. Frame v1 golden bytes가 현재 encoder와 같다는 단언은 schema 1이 현재 형식과 같은 동안만 유지되고, 형식이 바뀌면 그 단언만 바뀝니다.
- IX.11.6·9·10·11의 schema 변경은 공개 포맷 전 1→2 한 번으로 묶는다고 ADR-0077에 기록했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib migration` | 5 passed |
| `cargo test -p stellar-project --test migration_open` | 3 passed: 현재 schema 편집 가능, 옛 schema 메모리 migration(디스크 digest 불변·Git snapshot loader 포함), 경로 없음·새 schema 거부, 명시 migration 결과 = 메모리 결과 |
| `cargo test -p stellar-ops --lib migrate` | 4 passed |
| `cargo test -p stellar-runtime --lib staging` | 9 passed |
| `cargo test --workspace --locked --no-fail-fast` | 4,168 passed, 1 failed(`stellar-local-ipc` `slow_authenticated_peer_does_not_block_other_handshakes` 타이밍, 단독 실행 통과, 변경과 무관) |

## 차이

- `cargo xtask test-migrations`가 제품 registry를 쓰게 하는 일: `tools/xtask/src/migration_fixtures.rs`는 이 lane 소유가 아닙니다. 제품 step이 생기면(1→2) xtask fixture를 제품 registry로 바꿔야 합니다.
- 옛 schema HEAD 대 Working Tree semantic diff 0건, 옛 schema merge base 3-way merge 동등성 fixture: 열기 경로(`SavedProjectLoader`)는 메모리 migration을 하지만 semantic-diff·merge crate 테스트는 소유 범위 밖이라 만들지 않았습니다. 현재 제품 step이 없어 옛 schema 제품 fixture도 만들 수 없습니다.

## 테스트

- 이 행의 테스트는 모두 통과했습니다. workspace 전체에서는 변경과 무관한 `stellar-local-ipc` 타이밍 테스트 1건이 병렬 실행에서 실패했습니다(단독 통과). 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다. 열기·저장 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## 남은 위험

- 제품 step이 아직 없어 메모리 migration은 시험용 정책으로만 실행됩니다. 첫 제품 step(1→2)에서 고정 decoder 사본과 xtask fixture 전환이 함께 필요합니다.
- `SavedProject.generation`을 직접 읽는 호출자는 옛 schema일 때 원본 bytes를 봅니다. 문서 읽기는 `load_document`·`document_ids`를 써야 합니다.
- (검토 보완) 앱 창 session 열기(`crates/runtime` `ProjectWindowSession::load`)와 headless 읽기(`crates/ops` `open_snapshot`)는 `SavedProjectLoader`를 거치지 않고 current generation을 직접 열어 schema 판정이 없습니다. 지금은 schema 1뿐이라 도달하지 않지만, 첫 제품 step과 함께 두 경로를 판정에 연결해야 합니다. 편집 열기는 아키텍처 §36·행 정의대로 V.8.4 경로로 디스크에 migration하고 Git commit은 하지 않습니다(ADR-0077 결정 4). 두 파일 모두 이 lane 소유 밖입니다.
