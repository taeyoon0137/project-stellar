# 작업 보고서: XXV.1.3 앱 재실행 후 같은 base일 때만 history 복원

- WBS: XXV.1.3
- 작업명: 앱 재실행 후 current generation과 journal base가 같을 때만 history 복원, 불일치 base 봉인
- 상태: 완료 (`[x]`)
- 선행: XXV.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/journal_restore.rs`(신규) — `JournalStore::{restore_window, seal, list_sealed, read_sealed, sealed_dir}`, `SealReason`,
  `SealedJournal`, `RestoreOutcome`, `RestoredWindow`, 테스트 3건.
- `crates/history/src/history.rs` — journal stack으로 history를 만드는 `History::from_parts`(crate 내부).
- `crates/history/src/window.rs` — 복원한 history로 창을 여는 `WindowHistory::from_history`(crate 내부).
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XXV.1.3.md`(신규), 계획서(XXV.1.3 `[ ]`→`[x]`).

## 구현 요약

- **복원 조건**: `restore_window(window, current, policy)`는 active journal을 읽어 `journal.header.base == current`이고(generation 번호와 manifest
  hash 모두 같음) 이 writer가 편집 가능(V.8.1)일 때만 undo·redo stack과 다음 sequence를 복원합니다. 복원한 history는 열린 grouping·preview가 없고 새
  항목은 기록된 next sequence부터 받습니다(단조성 유지). 현재 창 선택은 journal에 없으므로 비어 있습니다(선택 복구는 복구 스냅샷 V.9.2의 몫).
- **봉인**: 복원하지 못하는 journal은 지우지 않고 `sealed/<window>.<순번 6자리>.<사유>.stlhjnl`로 rename합니다(같은 디렉터리 안이라 bytes 보존,
  프로젝트 내부 정책에서는 guard `*`가 하위 디렉터리도 제외). 사유: `base-mismatch`, `read-only`(최소 writer가 더 높아 이어 기록 불가),
  `unreadable`(형식·버전·손상 — 새 reader를 위해 보존), `window-mismatch`. 봉인 뒤 history는 비어 있고, 봉인 session은 자동으로 다시 복원하지 않아
  이후 기록과 섞이지 않습니다. `list_sealed`·`read_sealed`로 확인할 수 있습니다.
- journal이 없으면 빈 history(`Fresh`)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands -p stellar-history` | 완료 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 82 passed·1 ignored·doctest 3 passed, history unit 35 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0(`unnecessary_sort_by` 1건 수정 후) |
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 편집 3·선택 1 기록 후 1회 undo한 창을 저장 시점에 기록 → history 폐기 → 같은 세대를 레코드 왕복으로 다시 연 문서에서 같은 base로 복원: `Restored{undo 3, redo 1}`, next sequence 유지, redo가 종료 전 최종 상태, 전체 undo가 최초 문서, 전체 redo가 최종 상태, 새 편집 sequence = 기록된 next, journal 없는 창은 `Fresh`(`same_base_restores_undo_redo_after_relaunch`) | 통과 |
| generation 번호가 다른 base, 번호는 같고 manifest hash만 다른 base 각각 봉인(순번 1·2): 봉인 파일 bytes = 원래 journal, active 없음, 빈 history의 undo는 문서 불변, 새로 기록한 journal에 봉인 항목 없음, 봉인 session은 같은 base 복원에서도 다시 쓰이지 않음(`mismatched_base_is_sealed_and_not_mixed`, 앱 데이터 정책) | 통과 |
| 손상 journal `unreadable`, 미래 writer journal `read-only`(같은 base여도), 다른 창 journal `window-mismatch` 봉인, 봉인 파일 이름 형식·bytes 보존, active 없는 봉인 `None`, token 왕복(`unreadable_read_only_and_foreign_journals_are_sealed`, 프로젝트 내부 정책) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시의 "current generation"은 caller가 storage current pointer에서 얻어 `JournalBase`(generation 번호 + manifest hash)로 넘깁니다. history는
  storage에 의존할 수 없어 current pointer를 직접 읽지 않습니다.
- base 불일치 외에 읽기 전용·읽을 수 없음·창 불일치도 같은 봉인 경로로 처리했습니다(삭제하지 않고 보존).

## 남은 위험 / 한계

- 같은 base라도 journal이 그 세대 내용과 맞는지는 기록 시점 계약(XXV.1.2: 저장 직후 기록)에 기댑니다. 복원 시 역 transaction을 미리 실행해 보는
  검증은 하지 않으며, 맞지 않으면 해당 undo가 transaction 오류로 실패하고 항목은 stack에 남습니다(VII.4.6 동작).
- 봉인 session 정리(나이·크기 상한)는 XXV.1.5입니다. 봉인 순번은 창당 999,999까지입니다.
- 창 id(WindowSessionId)가 재실행 후 같은 값으로 주어지는지는 셸의 창 복원 정책(V.9 복구 키와 같음)에 달려 있습니다.
