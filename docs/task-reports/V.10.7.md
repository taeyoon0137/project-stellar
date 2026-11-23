# 작업 보고서: V.10.7 앱 기본 원본의 텍스트 전환과 chunk cache

## WBS

- WBS: V.10.7
- 작업명: 검증된 변환 경로에서 기본 원본을 텍스트로 전환하고 chunk 저장을 로컬 cache로 내림
- 상태: 부분 완료 (`[~]`) — 앱 창 session의 열기·저장·자동 저장·stale·외부 변경이 텍스트 원본을 기본으로 쓰고, chunk generation은 stamp 짝으로 관리하는 파생 cache가 되었습니다. 2026-11-24 `text-source-everywhere` lane이 CLI·MCP의 읽기·쓰기, semantic Git, 새 프로젝트의 첫 저장을 같은 원본 session으로 옮겼습니다(아래 "후속: CLI·MCP·Git과 첫 저장"). 같은 날 `git-cache-out` lane이 chunk cache를 Git 밖 `.stellar/cache/chunks/`로 옮기고 옛 배치 migration, 원본에서 다시 만들기, cache GC를 연결했습니다(아래 "후속: chunk cache를 Git 밖으로"). 기준 장비(REF-MAC) 실측·예산 판정은 하지 않았습니다(아래 차이). 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.10.6(같은 lane에서 앱 연결을 함께 마침, 보고서 V.10.6)
- 결정: [ADR-0088](../adr/ADR-0088-text-source-default-and-chunk-cache.md), [ADR-0113](../adr/ADR-0113-headless-text-source-everywhere.md), [ADR-0122](../adr/ADR-0122-chunk-cache-out-of-git.md)
- lane: `text-source-default`, `text-source-everywhere`, `git-cache-out`

## 후속: CLI·MCP·Git과 첫 저장 (2026-11-24, ADR-0113)

### 변경 파일

- `crates/project/src/document_io/session.rs`: 열기 계획 관찰 `observe`·`DiskSource`, 원본 revision hash `source_revision`, 검증된 변환 `convert_saved`(앱 창 session의 변환을 옮겨 한 곳으로 둠)
- `crates/runtime/src/window_session/source.rs`: 변환을 `convert_saved` 호출로 바꿈(같은 판정 한 곳)
- `crates/ops/src/snapshot.rs`: `open_snapshot`이 같은 열기 계획을 쓰고 FromText면 원본에서 문서를 읽음, 원본 revision, `disk_unchanged`, `source_semantic`
- `crates/ops/src/snapshot/save.rs`(신규): 원본 → cache → stamp 저장 `save_documents`, 명시적 원본 전환 `adopt`
- `crates/ops/src/snapshot/text_tests.rs`(신규): ops 시험
- `crates/ops/src/apply.rs`, `crates/ops/src/merge_apply/composer.rs`: 쓰기를 `save_documents` 하나로
- `crates/ops/src/trusted.rs`: MCP snapshot 재사용 판정이 원본 bytes도 비교(소유 밖 한 함수, ADR-0113 결정 2)
- `crates/project/src/create/initial_document.rs`: 새 프로젝트 첫 저장이 원본·cache·stamp를 함께 씀
- `tools/stellar-cli/src/{text_source,cli}.rs`, `tools/stellar-cli/tests/fixtures/help.txt`: `stellar text-source adopt`
- `crates/runtime/src/window_session.rs`: dirty 창 reconciliation이 원본 event 뒤 따라온 cache를 병합 base로 씀, 시험 helper `cache_only_rename`(소유 밖, ADR-0113 Consequences)
- `crates/runtime/src/window_session/source_tests.rs`, `crates/runtime/src/app/autosave/pipeline_tests.rs`, `tests/integration/tests/e2e_new_project_reopen.rs`: "CLI는 generation만 씀"·"최초 commit에 원본 없음" 전제의 기대값 조정(소유 밖)

### 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --locked --lib snapshot::text_tests` | 0 — 6 passed |
| `cargo test -p stellar-runtime --locked --lib`(샌드박스 해제) | 0 — 361 passed, 7 ignored |
| `cargo test -p stellar-integration-tests --locked --test e2e_new_project_reopen`(샌드박스 해제) | 0 — 1 passed |
| `cargo test -p stellar-project --locked --lib create` | 0 — 23 passed |

### 시험

| 시험 | 확인 |
|---|---|
| `a_paired_source_opens_from_the_cache_and_apply_writes_source_cache_and_stamp` | 짝인 원본은 cache에서 열고, `apply`가 View 원본 파일 하나·cache·stamp를 쓰고, 다시 연 revision이 쓰기 결과와 같음 |
| `a_source_edit_is_read_from_text_changes_the_revision_and_the_next_write_rebuilds_the_cache` | 원본만 바뀌면(Git pull) FromText로 원본에서 읽고 revision이 바뀜, 읽기는 파일을 쓰지 않음, MCP 재사용 판정이 변경을 봄, 옛 revision 쓰기 거부·아무것도 쓰지 않음, 새 revision 쓰기는 cache를 다시 맞춤 |
| `a_cache_only_write_is_read_forward_and_the_next_write_moves_it_into_the_source` | cache만 쓴 옛 writer 뒤 Forward로 읽고 다음 쓰기가 원본으로 옮김 |
| `a_broken_source_opens_but_refuses_document_reads_and_writes_with_the_file_name` | 깨진 원본: 열기는 되고 문서 읽기·쓰기는 파일 이름이 든 진단으로 거부, 아무것도 쓰지 않음 |
| `a_text_only_clone_without_cache_is_read_from_text_and_the_first_write_creates_the_cache` | cache·stamp 없는 clone은 원본에서 읽고 첫 쓰기가 generation 1과 stamp를 만듦 |
| `a_chunk_project_without_source_stays_chunk_only_on_write` | 원본 없는 chunk 프로젝트의 쓰기는 generation만 쓰고 원본을 만들지 않음 |
| `editable_project_is_openable_and_original_files_survive_every_outcome`(기존) | 원본을 더한 첫 저장도 생성·commit·열기 통과 |
| `new_project_edit_save_close_reopen_keeps_semantic_snapshot`(E2E) | 최초 commit이 `design/document.stellar.yaml`을 담음 |
| `cli_edits_write_the_text_source_and_a_pulled_source_is_read_from_text`(CLI) | `text-source adopt`(revision 그대로, 두 번째는 `text-source-present`), CLI 쓰기가 원본 한 줄 diff, 원본만 바뀌면 `query`가 원본 값과 새 revision |

### 차이(후속)

- cache GC와 chunk의 Git 밖 이동은 하지 않았습니다. storage 경로(보호 경로)·git-porcelain snapshot·앱 창 session·감시·fixture 전체가 바뀌어야 하고 대부분 이 lane 소유 밖입니다. 대신 semantic Git·merge가 원본을 기준으로 보게 해 cache가 commit에 들어가도 원본이 이깁니다(ADR-0113 결정 5·6·7).
- `[REF-MAC]` 실측은 하지 않았습니다.

## 후속: chunk cache를 Git 밖으로 (2026-11-24, ADR-0122)

### 변경 파일

- `crates/storage/src/location.rs`(신규): cache 위치 `.stellar/cache/chunks/` 함수·상수, 경로 판정, `.stellar/cache/.gitignore` 보장, 옛 배치 이동 `move_legacy_layout`
- `crates/storage/src/{generation/current.rs,generation/loader.rs,generation/scanner.rs,chunk/path.rs,chunk/store.rs,gc/sweep.rs}`: 경로를 위치 모듈로, 쓰기 때 Git 제외 규칙 보장, symlink 시험을 새 구성 요소로
- `crates/storage/src/journal/path.rs`: cache 안 파일만 `.stellar/` 아래 대상으로 받음
- `crates/storage/tests/{migration_git_fixture.rs,chunk_canonical_fuzz.rs}`: migration이 Git 작업 트리를 바꾸지 않음, 경로 구성 요소 수
- `crates/project/src/cache.rs`(신규): `migrate_layout`·`rebuild_from_text`·`prepare_snapshot`·`text_snapshot`·`collect_garbage`·`untrack_legacy`
- `crates/project/src/document_io/session.rs`: 관찰 전에 옛 배치 이동, stamp의 Git 제외 규칙을 저장소 함수로
- `crates/ops/src/git.rs`: Git snapshot을 원본으로 만든 cache로 엶, 후보 index에서 cache 경로 제외·원본 mode 유지, 원본 없는 index의 변경 단위 stage 거부, generation 묶음이 문서 원본 파일을 덮음, `GitSession::migrate_cache`(stage·unstage·commit 전), `GitError::Cache`
- `crates/ops/src/git_cache.rs`(신규): `stellar cache status|rebuild|gc|migrate`
- 소유 밖 최소 수정(위치 함수 사용·Git snapshot cache 준비): `crates/project/src/{conflict_open.rs,saved_project/head.rs,watcher/rescan.rs}`, `crates/ops/src/{candidate.rs,candidate/session.rs,merge_apply/composer.rs,merge_apply/target.rs}`, `tools/stellar-cli/src/cli.rs`·`tests/fixtures/help.txt`(명령 등록)
- 시험 fixture·기대값(소유 밖): `crates/ops/src/test_support.rs`(`save_text_document`), project·runtime·ops·mcp-core·CLI·MCP·integration 시험의 `design/current` 경로와 "generation이 Git에 있음" 전제
- `tests/integration/tests/git_cache_out.rs`(신규), `docs/adr/ADR-0122-chunk-cache-out-of-git.md`(신규)

### 시험

| 시험 | 확인 |
|---|---|
| `two_branch_saves_merge_with_only_text_sources_as_conflict_candidates` | 두 branch 저장 뒤 병합의 unmerged가 텍스트 원본 파일뿐, Git이 cache를 추적하지 않음, conflict 세션 해결 뒤 원본에서 읽고 `rebuild`로 cache 짝 |
| `clone_without_cache_reads_the_text_source_and_rebuilds_the_cache` | clone에 cache 없음, 읽기는 원본에서(쓰지 않음), `rebuild`가 1세대 cache를 만들고 작업 트리는 깨끗 |
| `legacy_tracked_cache_is_moved_on_open_and_untracked_before_commit` | 옛 배치를 추적하던 저장소: 관찰이 옮기고 `migrate_cache`가 index에서 빼며 commit 뒤 Git에 cache 없음 |
| `cache_gc_removes_old_generations_without_touching_git` | GC가 보존 정책 밖 세대를 지우고 Git 상태는 그대로 |
| `stellar_storage::location::tests::*` | 경로 판정, 옛 배치 이동·중복 제거·pointer 교체·다른 bytes 보존, symlink 미추적, Git 제외 규칙 |
| `stellar_ops::git_cache::tests::status_rebuild_and_gc_work_on_a_text_project_without_cache` | 명령 함수의 계획·재생성·GC |
| `selected_property_round_trips_without_changing_working_bytes_or_real_index`(runtime) | 변경 ID stage 후보 delta가 View 원본 파일 하나, 원본 실행 bit 유지 |

### 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage --locked` | 0 — lib 297 포함 전부 통과 |
| `cargo test -p stellar-integration-tests --locked --test git_cache_out`(샌드박스 해제) | 0 — 4 passed |
| `cargo test -p stellar-project --locked` | lib·통합 통과. `restricted_yaml_fuzz`의 시간 기준(3초) stress 한 건은 다른 lane 부하에서 3.19초로 실패(이 변경과 무관한 시간 의존 시험) |
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-mcp-core -p stellar-mcp -p stellar-cli --locked`(샌드박스 해제) | 0 — 통과 |
| `cargo test --workspace --locked --no-fail-fast`(샌드박스 해제) | 실패 시험을 고친 뒤 실패 묶음은 단독으로 다시 돌려 통과. `stellar-ffi`의 `create_via_abi_reopens_in_an_editable_window_and_preserves_existing_folder`는 전체 실행 부하에서만 실패하고 단독 통과 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 |
| `cargo xtask validate-adr --base ef01e66d` | 0 |
| `cargo xtask generate-ffi-header --check` | 0 — drift 없음 |
| `STELLAR_CHECK_BASE=ef01e66d cargo xtask check-core`(샌드박스 해제) | 1회차 test 단계에서 `two_app_windows_resolve_a_source_stale_with_the_two_choices`가 저장 잠금 Busy로 실패, 단독 3회 통과. 재실행 결과는 lane 보고에 적음 |

### 차이(후속)

- 앱 runtime의 유휴 cache GC 일정(GcScheduler)은 연결하지 않았습니다. GC는 `stellar cache gc`·`migrate`로 실행합니다.
- semantic Git stage는 Git snapshot마다 원본을 해석해 cache를 만듭니다. 원본만으로 semantic snapshot을 만드는 경로로 바꾸는 개선이 필요합니다(ADR-0122 Performance).
- Git 줄 병합이 같은 문서 파일의 View 목록을 깨끗이 병합해도 결과 원본이 읽히지 않을 수 있습니다(시험 `independent_page_and_view_insertions_merge_cleanly`에서 관찰). merge driver를 등록하면 의미 병합되고, 등록하지 않은 저장소는 열기·검증이 원본 진단을 냅니다.
- 원본이 없는 옛 chunk 프로젝트(root 문서 둘 이상 등)는 Git에 원본이 없어 변경 ID stage·의미 병합을 거부합니다.
- 기술 아키텍처 §19 파일 구조 그림의 `design/current` 등은 문서 소유 밖이라 고치지 않았습니다(ADR-0122가 기준).

## 필수 실행 환경

- `DEV-MAC`: 순수 Rust 시험(아래).
- `REF-MAC`: 실제 통합 T0→T5와 열기·저장 예산 실측. 이 lane에서는 실행하지 않았습니다.

## 변경 파일

- `crates/project/src/document_io/session.rs`(신규): 원본 파일 집합·base·digest, stamp(`.stellar/cache/text-source.stamp`), 열기 계획 `OpenPlan`, 바뀐 파일만 journal로 쓰는 `commit`, 활성 기본값 세트 읽기
- `crates/project/src/document_io/mod.rs`: 모듈 등록 한 줄
- `crates/project/src/external_change/source.rs`: 엔진 `TextSourceSession`이 같은 원본 읽기·저장 함수를 씀(기본값 세트 파일 제외 포함)
- `crates/runtime/src/window_session.rs`: 원본 상태(`SourceMode`), 저장 순서(generation 확인 → 원본 journal → cache → stamp), 맞춤 쓰기(`reconcile`·`reconcile_pending`·`needs_write`), stale 판정에 원본 포함, UseCurrentWindow·UseLatestDisk·외부 변경 반영의 원본 경로, 활성 기본값 세트 거울
- `crates/runtime/src/window_session/source.rs`(신규): 디스크 읽기 `read_disk_state`(계획별 읽기, 검증된 변환 `convert`)
- `crates/runtime/src/window_session/source_tests.rs`(신규), `source_bench.rs`(신규, 실측 `#[ignore]`)
- `crates/runtime/src/app/autosave.rs`, `crates/runtime/src/app.rs`: stale 판정을 `is_stale`로, 자동 저장·flush가 `needs_write`를 봄, 원본만 쓴 저장도 저장으로 보고
- `crates/runtime/src/app/autosave/pipeline_tests.rs`, `crates/runtime/src/app/canvas_gesture.rs`, `crates/runtime/src/app_window_tests.rs`, `crates/runtime/src/window_session/stale_tests.rs`: 시험 조정(아래)
- `docs/adr/ADR-0088-text-source-default-and-chunk-cache.md`(신규), 이 보고서

## 구현 요약

- 열기: stamp 짝과 디스크로 `Cache`(generation 부분 로딩)·`Forward`(CLI·MCP의 generation을 원본으로)·`FromText`(원본 읽고 cache 다시 만듦)·`Migrate`(기존 chunk 프로젝트를 원본으로)·`Empty`를 고릅니다. 변환은 generation 전체 chunk 소비와 다시 읽은 의미 일치를 확인했을 때만 하고, 아니면 chunk 전용으로 남습니다.
- 저장: 원본을 먼저 씁니다(바뀐 파일만, 기대 hash = 원본 base). cache 실패는 원본 저장 성공 + `cache_stale`이고 다음 열기가 고칩니다.
- 외부 변경: 원본 파일 변경(Git pull·편집기·엔진 session)과 generation 변경(CLI·MCP)을 모두 반영합니다. clean 창은 `ApplyExternalSnapshot` 하나(Undo 한 번)로 받고, 뒤처진 쪽 맞춤 쓰기는 writer가 합니다. dirty 창은 편집을 보존하고 외부 writer면 reconciliation 대기입니다.
- 활성 기본값 세트: 열 때 `stellar.yaml` 값을 모델 거울에 넣고, 그 창에서 바꾼 경우에만 저장 때 되씁니다.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked --no-fail-fast`(샌드박스 해제) | 0 — lib 262 passed·2 ignored, 통합 테스트 전부 통과. 두 번 연속 실행 |
| `cargo test -p stellar-runtime --locked --lib source_tests`(샌드박스 해제) | 0 — 11 passed |
| `cargo test -p stellar-project --locked --test text_session` | 0 — 8 passed |
| `cargo test -p stellar-project --locked --lib document_io::session` | 0 — 3 passed |
| `cargo clippy -p stellar-runtime -p stellar-project --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 33ba9b2f` | 0 |
| `cargo test -p stellar-runtime --release --lib window_session::source_bench -- --ignored --nocapture` | 0 — 실측(아래 성능) |

## 테스트

| 테스트 | 확인 |
|---|---|
| `opening_a_chunk_project_migrates_it_to_the_text_source_once` | 기존 chunk 프로젝트 열기 → 원본·stamp·`.stellar/cache/.gitignore` 생성, generation 그대로, 다시 열면 `Cache`, 원본만 읽은 의미가 같음 |
| `save_writes_the_changed_source_file_then_the_cache_and_the_stamp` | 이름 하나 저장 → 원본 파일 하나만 바뀜, generation 2, stamp 짝, 자기 저장은 stale 아님, 바뀐 것 없으면 쓰기 0 |
| `a_source_edit_while_closed_reopens_from_the_text_and_rebuilds_the_cache` | 닫힌 동안 원본 변경 → `FromText`, cache(generation)·CLI 읽기도 같은 의미 |
| `a_cache_only_write_moves_forward_into_the_source` | `stellar apply`(generation) 뒤 열기 → `Forward`, 원본이 CLI 편집을 담음 |
| `a_clean_window_applies_an_external_source_edit_as_one_undo_and_rebuilds_the_cache` | 열린 clean 창이 원본 변경을 Undo 한 번 항목으로 받고, 맞춤 쓰기는 다음 저장에서, 되돌린 결과 저장이 원본에 반영 |
| `a_clean_window_applies_a_cache_only_write_and_moves_it_into_the_source` | 열린 clean 창이 CLI generation을 받고 다음 저장이 원본만 씀 |
| `a_dirty_window_keeps_its_edits_and_does_not_overwrite_an_external_source_edit` | dirty 창 + 외부 원본 변경 → 편집 보존, reconciliation 대기, 저장 stale·원본 불변, 두 선택지 거부 |
| `two_app_windows_resolve_a_source_stale_with_the_two_choices` | 두 앱 창: 앱 창 출처 판정, UseCurrentWindow가 원본·cache를 함께 씀, UseLatestDisk가 원본 base까지 맞춤 |
| `a_broken_source_is_not_opened_and_names_the_file_and_line` | 깨진 원본 → 열기 거부, 파일·줄 진단 |
| `a_generation_the_text_source_cannot_hold_stays_chunk_only` | root 문서 둘 → chunk 전용, 원본을 만들지 않고 generation 저장 |
| `the_active_style_defaults_set_is_mirrored_on_open_and_written_back_on_save` | 열 때 거울, 바꾼 창만 `stellar.yaml` 되씀, 바꾸지 않은 창은 외부 변경을 되돌리지 않음 |
| `a_library_only_source_change_is_taken_even_by_a_dirty_window` | token 파일만 외부에서 더해지면 문서 stale이 아니고, dirty 창도 그 의미를 받고 저장을 계속하며 token 파일을 덮어쓰지 않음 |
| `a_save_that_fails_on_the_source_does_not_leave_the_style_defaults_choice_written` | 기본값 세트를 바꾼 창의 저장이 원본 journal에서 stale로 실패하면 먼저 되쓴 `stellar.yaml`을 앞 bytes로 되돌리고 편집은 dirty로 남김 |
| `opening_recovers_an_interrupted_source_save_before_reading_the_source` | commit marker 전에 멈춘 원본 저장 거래가 있으면 열기가 원본을 읽기 전에 앞 세대로 되돌리고, 반쯤 쓴 파일을 원본으로 읽지 않음 |
| `a_cache_opened_window_saving_over_a_library_only_change_keeps_its_own_save` | cache로 연 창이 저장하는 사이 token 파일만 바뀌어도 그 저장을 자기 저장으로 반영하고(clean, base = 디스크), 다음 저장이 stale이 아님 |

시험 조정: 자동 저장·창 닫기 실패 시험은 cache(`design/objects`) 대신 원본 journal(`.stellar/journal`)을 막습니다(cache 실패는 저장 실패가 아님). 열기가 원본을 쓰므로 "pan·zoom·overlay는 bytes를 바꾸지 않음" 시험은 연 뒤의 bytes를 기준으로 잡습니다. 창 열기·zoom 시험은 요청 없는 글꼴 완료 event를 건너뜁니다. 잠금을 얻을 수 없는 fixture mode(`busy`·`missing-home`)는 열기 전환을 못 하므로 되돌린 편집의 쓰기 0 확인을 `write` mode에서만 합니다.

## 성능

DEV-MAC release, 다른 lane이 동시에 돌던 부하 상태입니다. 자세한 표와 해석은 ADR-0088 Performance입니다.

| 문서 node | Migrate 열기 | Cache 열기 p50 | FromText 열기 | 이름 하나 저장 p50 |
|---:|---:|---:|---:|---:|
| 1,011 | 625 ms | 6.9 ms | 190 ms | 367 ms |
| 10,101 | 5,987 ms | 54 ms | 1,715 ms | 1,244 ms |
| 50,201 | 19,895 ms | 266 ms | 12,474 ms | 7,982 ms |

텍스트 원본 encode(5만 node 5.8초)·decode(1.2초)가 저장·FromText·Migrate를 지배합니다. 저장은 writer thread라 인터랙션을 막지 않지만, codec 증분 인코딩 없이는 대형 문서 예산을 맞추기 어렵습니다.

## 차이

- `[REF-MAC]` 실제 통합 T0→T5와 기준 장비 열기·저장 예산 실측을 하지 않았습니다.
- (2026-11-24 해소, ADR-0122) cache GC: chunk cache가 Git 밖이라 GC가 Git 추적 파일을 지우지 않습니다. `stellar cache gc`·`stellar cache migrate`가 연결되었고, 앱 runtime의 유휴 GC 일정은 아직 연결하지 않았습니다.
- (2026-11-24 해소, ADR-0113 결정 4) 새 프로젝트 생성의 최초 commit이 원본을 담습니다.

## 남은 위험

- (2026-11-24 해소, ADR-0113) CLI·MCP도 원본과 cache를 함께 씁니다. 원본 기준 revision이라 Git pull 전 revision으로 쓴 CLI 편집은 거부됩니다.
- 다른 앱 창의 journal 거래 도중 읽은 clean 창은 일부 파일만 바뀐 상태를 받을 수 있습니다(다음 event에서 맞춰짐).
- 원본 stale 판정은 판정마다 원본 bytes를 읽고 hash합니다(감시 event·선택 실행 때). 외부 변경 반영은 document thread에서 돌고, 자기 저장 event에도 원본 전체를 한 번 이상 읽고 hash합니다(문서 밖 의미 확인과 stale 판정). 대형 원본에서는 metadata 지문으로 먼저 거르는 개선이 필요합니다.

## 검토 보완

- 열기가 남은 저장 journal 거래를 복구하지 않고 원본을 읽던 결함을 고쳤습니다. 이제 원본을 읽기 전에 `journal::recover`를 부릅니다(ADR-0088 결정 4).
- 기본값 세트를 바꾼 저장에서 `stellar.yaml`을 되쓴 뒤 원본 journal이나 chunk 전용 generation 쓰기가 실패하면, 저장은 실패("아무것도 쓰지 않음")로 알리면서 `stellar.yaml`은 바뀐 채 남던 결함을 고쳤습니다. 실패하면 앞 값으로 되돌립니다(ADR-0088 결정 7).
- 열 때 맞춤 쓰기가 실패하면 `reconcile_pending`을 세워 다음 자동 저장이 다시 맞추게 했습니다. 전에는 다음 편집 전까지 맞추지 않아 ADR의 "다음 저장이 다시 맞춤"과 달랐습니다.
- cache로 연 창의 writer가 문서 밖 의미 파일만 바뀐 디스크 base 위에 저장하면, 창이 그 저장을 반영하지 않아(캡처 base 비교가 어긋남) dirty와 옛 base가 남고 다음 저장·판정이 자기 저장을 stale로 보던 결함을 고쳤습니다. 반영 비교는 캡처한 session base로 합니다.
