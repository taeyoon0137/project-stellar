# 작업 보고서: V.10.7 앱 기본 원본의 텍스트 전환과 chunk cache

## WBS

- WBS: V.10.7
- 작업명: 검증된 변환 경로에서 기본 원본을 텍스트로 전환하고 chunk 저장을 로컬 cache로 내림
- 상태: 부분 완료 (`[~]`) — 앱 창 session의 열기·저장·자동 저장·stale·외부 변경이 텍스트 원본을 기본으로 쓰고, chunk generation은 stamp 짝으로 관리하는 파생 cache가 되었습니다. 기준 장비(REF-MAC) 실측·예산 판정, cache GC, 새 프로젝트 생성 경로의 첫 commit 원본, chunk의 Git 밖 이동은 하지 않았습니다(아래 차이). 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.10.6(같은 lane에서 앱 연결을 함께 마침, 보고서 V.10.6)
- 결정: [ADR-0088](../adr/ADR-0088-text-source-default-and-chunk-cache.md)
- lane: `text-source-default`

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
- cache GC: generation이 아직 `design/`에 있어 Git이 추적하므로, 옛 generation을 지우면 추적 파일 삭제가 됩니다. chunk를 Git 밖으로 옮기는 일(ops·CLI·MCP·semantic Git의 원본 전환 뒤)과 함께 연결해야 합니다.
- 새 프로젝트 생성(`stellar_project::create`)의 최초 commit은 아직 generation만 담습니다. 앱이 처음 열 때 원본으로 옮깁니다. create 경로는 소유 밖입니다.

## 남은 위험

- CLI·MCP는 아직 generation만 씁니다. 앱이 닫힌 동안 원본(Git pull)과 generation(CLI)이 함께 바뀌면 원본이 이기고 CLI 편집은 옛 generation에만 남습니다.
- 다른 앱 창의 journal 거래 도중 읽은 clean 창은 일부 파일만 바뀐 상태를 받을 수 있습니다(다음 event에서 맞춰짐).
- 원본 stale 판정은 판정마다 원본 bytes를 읽고 hash합니다(감시 event·선택 실행 때).
