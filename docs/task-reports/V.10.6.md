# 작업 보고서: V.10.6 텍스트 원본의 감시·외부 변경·저장·재열기 연결

## WBS

- WBS: V.10.6
- 작업명: 텍스트 원본의 감시·외부 변경·저장·재열기 연결
- 상태: 부분 완료 (`[~]`) — 2026-11-24 통합 판정: 이 행의 구현은 끝났지만 선행 V.10.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 2026-11-24 `text-source-everywhere` lane: 앱 창 session의 감시·외부 변경·명령 저장·재열기(앞선 lane)에 더해, CLI·MCP의 명령 저장과 재열기가 같은 텍스트 원본 session을 쓰고 검증 열의 CLI dry-run→저장→재열기→Git diff를 실제 `stellar` 명령으로 확인했습니다. dirty 창 병합은 XIII.3.4(ADR-0110)로 들어와 있고, CLI가 원본도 쓰면서 생긴 원본 event·cache event 순서도 병합합니다. 선행 V.10.5의 판정과 원장 상태는 통합 때 `generate-ledger`로 정합니다. 처음 보고 당시: 부분 완료 (`[~]`).
- 선행: V.10.5, XIII.3.1
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md), [ADR-0088](../adr/ADR-0088-text-source-default-and-chunk-cache.md)
- lane: `window-session-stale`, `text-source-default`

## 변경 파일

- `crates/project/src/external_change/source.rs`(신규): `TextSourceSession`(열 때 journal recover, journal 저장, 외부 변경 반영), `SourceDiagnostic`
- `crates/project/src/watcher/event.rs`: `affects_text_source`·`TEXT_SOURCE_DIR`
- `crates/project/src/external_change.rs`: 모듈 등록
- `crates/project/tests/text_session.rs`(신규)
- (`text-source-default`) `crates/runtime/src/app/autosave/external.rs`: 감시 thread가 원본 파일 event(`affects_text_source`)를 조용한 구간 뒤 반영 job으로 보냄, 감시기가 없으면 원본 metadata 지문 비교
- (`text-source-default`) `crates/runtime/src/window_session.rs`·`window_session/source.rs`: 앱 창 session의 외부 변경 반영·재열기·저장이 원본을 씀(V.10.7 보고서·ADR-0088)
- (`text-source-default`) `crates/runtime/src/app/autosave/pipeline_tests.rs`, `crates/runtime/src/window_session/source_tests.rs`: 시험 추가

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --locked` | 0 — lib 418 passed·1 ignored, 통합 테스트(text_session 8 포함) 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |
| (`text-source-default`) `cargo test -p stellar-runtime --locked --no-fail-fast`(샌드박스 해제) | 0 — 아래 runtime 시험 포함 전부 통과 |
| (`text-source-default`) `cargo xtask validate-adr --base 33ba9b2f` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `text_session::save_writes_only_changed_files_through_the_journal_and_reopens_identically` | 통과 — 저장→재열기 의미 동일, 바뀐 파일 하나, 시스템 Git diff에 그 파일·이름만 |
| `text_session::clean_session_applies_an_external_edit_as_one_undoable_command_and_ignores_its_own_save` | 통과 — clean 외부 변경, cache 무효화 node, Undo 한 번 |
| `text_session::dirty_session_keeps_its_edits_and_its_next_save_is_stale` | 통과 — dirty 편집 보존, 다음 저장 stale·쓰기 0 |
| `text_session::broken_external_yaml_is_rejected_with_path_and_line_and_applies_nothing` | 통과 |
| `text_session::open_recovers_a_leftover_save_transaction_first` | 통과 |
| `text_session::watcher_events_drive_the_external_check` | 통과(macOS FSEvents) |
| `pipeline_tests::watcher_applies_an_external_text_source_edit_and_rebuilds_the_cache` | 통과 — 앱 runtime 감시 thread가 외부 원본 저장을 clean 창에 Undo 한 번 항목으로 반영, 자동 저장 writer가 cache를 맞춤 |
| `source_tests::cli_dry_run_then_window_save_and_reopen_show_one_view_file_in_git_diff` | 통과 — CLI dry-run은 쓰기 0(Git status 깨끗), 창 저장→재열기 의미 동일, 시스템 Git diff의 원본 파일은 바뀐 View 하나, `.stellar/`는 Git 밖 |
| `source_tests::a_clean_window_applies_an_external_source_edit_as_one_undo_and_rebuilds_the_cache` 외 V.10.7 보고서의 runtime 시험 | 통과 — clean/dirty 외부 변경, cache 무효화 입력(변경 집합), 자기 저장 무시 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- (2026-11-24 해소) 검증 열의 CLI dry-run 단계를 실제 `stellar` 명령으로 확인했습니다. CLI·MCP도 원본을 읽고 씁니다(ADR-0113).
- (2026-11-24 기준) dirty 창 병합은 XIII.3.4(ADR-0110)가 연결했습니다.

## 남은 위험

- 다른 앱 창의 journal 거래 도중 읽은 clean 창은 일부 파일만 바뀐 상태를 받을 수 있습니다(다음 event에서 맞춰짐).
- 문서 밖 의미(컴포넌트·token·catalog)의 외부 변경은 Undo로 되돌리지 않습니다.

## 후속: CLI 명령 저장·재열기 (2026-11-24, ADR-0113)

- CLI·MCP 쓰기(`stellar apply`·`stellar css paste`·Merge Composer)가 원본 → cache → stamp 순서로 씁니다(`stellar_ops::snapshot::save`). 재열기(`open_snapshot`)는 앱 창과 같은 열기 계획을 씁니다.
- 열린 dirty 앱 창은 CLI 저장의 원본 event를 먼저 받아 FromText 상태가 된 뒤 cache event를 받습니다. 그 cache가 창 base 원본과 짝이면 병합 base로 씁니다(`crates/runtime/src/window_session.rs` `reconcile_external`, 소유 밖 한 판정).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --locked --test text_source_git`(샌드박스 해제) | 0 — 3 passed |
| `cargo test -p stellar-runtime --locked --lib`(샌드박스 해제) | 0 — 361 passed, 7 ignored |

| 시험 | 확인 |
|---|---|
| `cli_edits_write_the_text_source_and_a_pulled_source_is_read_from_text` | `stellar apply --dry-run`은 작업 트리를 바꾸지 않음 → `stellar apply`가 View 원본에 쓰고 `git diff -U0`이 이름 줄 하나(삭제 1·추가 1) → 새 프로세스 `stellar query`가 같은 값 → 원본만 바뀌면(Git pull) `query`가 원본 값과 새 revision |
| `external_change_over_a_dirty_window_is_reconciled_and_autosave_resumes`(runtime) | CLI가 원본·cache를 모두 써도 dirty 창이 두 선택지 없이 병합하고 자동 저장이 재개 |
| `a_cache_only_write_moves_forward_into_the_source`·`a_clean_window_applies_a_cache_only_write_and_moves_it_into_the_source`(runtime) | cache만 쓰는 옛 writer는 `cache_only_rename` helper로 계속 확인 |
