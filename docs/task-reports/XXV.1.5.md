# 작업 보고서: XXV.1.5 history journal retention과 compaction

- WBS: XXV.1.5
- 작업명: retention size/age policy와 compaction 구현
- 상태: 완료 (`[x]`)
- 선행: XXV.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/journal_retention.rs`(신규) — `JournalRetention`(기본값·검증), `RetentionReport`, `JournalStore::apply_retention`,
  봉인 직후 정리 `prune_after_seal`, 테스트 3건.
- `crates/history/src/journal.rs` — `HistoryJournal::{encode_bounded, compact}`, `JournalCompaction`, `JOURNAL_EMPTY_LEN`, 공통 header
  기록 함수.
- `crates/history/src/journal_store.rs` — `JournalStore`의 retention 정책(`with_retention`, `retention`), 기록 시 compaction
  (`write`, 결과를 함께 주는 `write_reporting`), `JournalStoreError::InvalidRetention`, 임시 파일 표지 상수.
- `crates/history/src/journal_restore.rs` — 봉인 시 수정 시각 갱신(나이 기준)과 봉인 session 경계 즉시 적용.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XXV.1.5.md`(신규), 계획서(XXV.1.5 `[ ]`→`[x]`).

## 구현 요약

- **journal 하나 compaction**(기록할 때마다): 항목 수(`max_journal_entries`, 기본 10,000)나 bytes(`max_journal_bytes`, 기본 32 MiB)를
  넘으면 현재 위치에서 가장 먼 항목부터 버립니다. 남은 undo 수가 redo 수 이상이면 가장 오래된 undo, 아니면 가장 먼 redo를
  버립니다. 남은 항목은 현재 위치 앞뒤로 연속이라 복원 후 그대로 undo·redo되고 sequence 무결성도 유지됩니다. 항목은 한 번만
  인코딩합니다. 메모리 history는 줄이지 않습니다.
- **나이**: 봉인 session은 봉인 시각부터 `max_sealed_age`(기본 30일)가 지나면 지웁니다. 봉인 때 수정 시각을 갱신하고 내용은
  보존합니다. 열려 있지 않은 창의 active journal은 마지막 기록부터 `max_active_age`(기본 90일)가 지나면 지우고, 크래시로 남은
  임시 파일은 `temp_grace`(기본 1시간)가 지나면 지웁니다. 미래 시각은 나이 0으로 봅니다. 상한을 넘는 기존 active journal은
  다시 기록해 줄이며, 이때 수정 시각은 원래 값으로 되돌려 나이가 늘지 않게 합니다.
- **개수·전체 bytes**: 봉인 session 수(`max_sealed_sessions`, 기본 64)나 active + 봉인 bytes 합(`max_total_bytes`, 기본 256 MiB)을
  넘으면 가장 오래된 봉인 session부터 지우고, 그다음 열려 있지 않은 창의 가장 오래된 active journal을 지웁니다. 열린 창의 journal은
  지우지 않으므로 정리 뒤 상한은 `max(max_total_bytes, 열린 창 수 × max_journal_bytes)`입니다. 열린 창만으로 넘으면
  `within_budget = false`로 보고합니다.
- **정리 시점**: `apply_retention(now, open_windows)`는 프로젝트를 열 때 부릅니다. 봉인 경로(XXV.1.3 복원 봉인, XXV.1.4 기준점 변경)는
  봉인할 때마다 봉인 session 경계를 바로 적용합니다. 방금 봉인한 session은 유지하며, best-effort라 실패는 다음 정리에서 다시
  처리합니다. guard `.gitignore`와 사용자 파일은 세지도 지우지도 않습니다.
- 정책 검증: journal 하나 상한 ≥ 빈 journal 길이(`JOURNAL_EMPTY_LEN`), 전체 상한 ≥ journal 하나 상한.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 82 passed·1 ignored·doctest 3 passed, history unit 41 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-document workspace_production_code_does_not_use_test_support`(VI.10.4 검사) | 통과 |

| 검사 | 결과 |
|---|---|
| 편집 40·undo 10(undo 30, redo 10)을 항목 상한 12로 기록: undo 24·redo 4를 버리고 현재 위치 앞뒤 6개씩 유지, in-place `compact`와 bytes 동일, bytes 상한 1/3 이하·계산 길이 = 실제 길이, 빈 journal 상한이면 header만, 상한 안이면 원본 bytes. 저장소 기록 후 복원하면 `Restored{6,6}`, 전체 undo = 편집 24 뒤 상태, 전체 redo = 편집 36 뒤 상태, 메모리 history는 30·10 그대로(`compaction_keeps_entries_nearest_the_current_position`) | 통과 |
| active 4개(열린 창 200일, 닫힌 창 100일·1일·2일 대형)·봉인 5개(1·2·3·4·40일)·임시 파일 2개(2시간·방금)·무관한 파일: 오래된 임시 1개만 삭제, 닫힌 100일 journal 삭제·열린 창 유지, 대형 journal compaction(상한 이하, 수정 시각 유지), 40일 봉인(나이)과 4일 봉인(개수 3) 삭제, 보고 bytes = 디스크 합, 재적용 변화 0. 전체 상한을 줄이면 봉인 3개 → 닫힌 창 중 더 오래된 journal 순으로 삭제. 열린 창 journal만으로 넘으면 삭제 없이 compaction 후 `within_budget=false`(`retention_removes_expired_and_over_budget_files_but_keeps_open_windows`) | 통과 |
| 30 round 기록·checkout 봉인 반복(상한 항목 20·journal 16 KiB·전체 48 KiB·봉인 4): 매 round journal 항목 ≤ 20·크기 ≤ 16 KiB, 봉인 직후 봉인 수 ≤ 4·전체 bytes ≤ 48 KiB, 방금 봉인한 session 유지·수정 시각 = 봉인 시각. 31일 뒤 정리하면 봉인 0·전체 0. 모순 정책 거부, 기본 정책 확인(`repeated_sessions_stay_within_bounded_storage`) | 통과 |
| 검사 민감도(커밋하지 않은 임시 변경): 봉인 직후 정리 제거 → bounded test round 4 실패. 기록 시 compaction 제거 → 세 test 모두 실패 | 확인 |

## 차이(작업 지시와 다른 점)

- 기본 상한(32 MiB·10,000 항목·256 MiB·64 session·30/90일·1시간)은 잠정치입니다. 계획서의 다른 수치처럼 실제 사용량 측정으로
  조정해야 하며 `config/` 예산 파일에는 넣지 않았습니다(이 lane의 허용 경로 밖).
- 나이 기준은 파일 수정 시각(wall clock)입니다. 백업·복사 도구가 수정 시각을 바꾸면 나이가 달라질 수 있습니다.

## 남은 위험 / 한계

- 프로젝트 열기 시점에 `apply_retention`을 부르는 배선(project·runtime)은 이 작업 범위 밖입니다.
- 같은 journal 디렉터리를 쓰는 다른 프로세스가 동시에 기록 중이면 1시간 유예보다 오래 걸린 임시 파일이 지워질 수 있습니다. 이때 그
  기록은 rename 실패 오류가 되며 기존 journal은 손상되지 않습니다.
- compaction은 기록 시점 비용으로 항목을 모두 한 번 인코딩합니다(O(journal 크기), XXV.1.2와 같음).

## 후속 수정(리뷰 반영): 봉인 묶음 정리·항목 상한 compaction·봉인 목록 파일 종류

- **SEAL-BATCH-PRUNE**: `seal_for_base_change`가 창마다 봉인하고 그때마다 정리해서, 한 호출이 봉인 session 상한보다
  많은 창을 봉인하면 뒤 창의 봉인이 같은 호출에서 앞서 봉인한 session을 지웠습니다. 그러면 `BaseChangeOutcome.sealed`에
  없는 경로가 남았습니다. 이제 정리하지 않는 내부 봉인(`seal_unpruned`)으로 모두 봉인한 뒤, 그 호출에서 봉인한 session을
  모두 지키며 한 번만 정리합니다. 중간에 실패해도 이미 봉인한 것까지 정리합니다. 공개 `seal`(XXV.1.3 복원 경로 포함)은
  한 호출에 하나를 봉인하므로 기존처럼 그 session을 지키며 정리합니다. 묶음이 상한보다 크면 그 묶음은 남기고 다음
  `apply_retention`이 상한을 맞춥니다.
- **COMPACT-ACTIVE-ENTRIES**: `apply_retention`은 bytes 상한을 넘을 때만 기존 active journal을 줄였습니다. 이제 항목 상한을
  넘어도 줄입니다. 항목 수는 항목을 디코딩하지 않고 길이 프리픽스로 셉니다(`HistoryJournal::read_entry_counts`). 길이로 보아
  상한을 넘을 수 없는 파일(항목마다 프리픽스 4 bytes)은 열지 않습니다.
- **SEALED-LIST-NO-FILETYPE**: `list_sealed`가 파일 종류를 보지 않아, `sealed/`에 봉인 이름 형식인 디렉터리가 있으면 정리가
  `remove_file` 오류로 중단되었습니다. symlink는 대상 파일 길이로 세었습니다. 이제 일반 파일만 봉인 session으로 세고
  지웁니다(`list_active`와 같은 규칙, symlink는 따라가지 않음). 다만 봉인 순번은 형식이 같은 모든 이름을 피해 정합니다.
  그래서 rename이 기존 디렉터리·symlink와 충돌하지 않습니다.

| 검사 | 결과 |
|---|---|
| 이전 봉인 1개(1일) + active 3개, 봉인 상한 1로 checkout 봉인: 결과 3개 경로가 모두 남고 `list_sealed` = 결과, 이전 봉인만 삭제. 다음 정리 뒤 봉인 1개·`within_budget`(`one_base_change_keeps_every_session_it_sealed`). 수정 전에는 첫 창의 봉인 경로가 없어 실패 | 통과 |
| 기본 정책으로 기록한 항목 40개(undo 30·redo 10) journal을 항목 상한 12로 정리: bytes 상한 안이어도 compaction(undo 24·redo 4 버림, 6·6 유지), 수정 시각 유지, 상한 안 journal은 그대로, 재적용 변화 0. helper는 정상 (30, 10), undo 구간 잘림·형식 아님·다른 format version은 `None`(`retention_compacts_existing_journal_over_entry_cap`). 수정 전에는 `compacted`가 비어 실패 | 통과 |
| `sealed/`에 봉인 이름 형식 디렉터리(순번 1)와 외부 64 KiB 파일을 가리키는 symlink(순번 2): 새 봉인은 순번 3, `list_sealed`는 그 파일만, 봉인 상한 0으로 정리해도 오류 없이 그 파일만 삭제·`total_bytes` 0, 디렉터리·symlink·대상 유지(`sealed_listing_counts_only_regular_files`). 수정 전에는 `list_sealed`가 3개를 돌려 실패 | 통과 |
| 검사 민감도(커밋하지 않은 임시 변경): 봉인 순번을 일반 파일만으로 계산 → 위 test 실패 | 확인 |
