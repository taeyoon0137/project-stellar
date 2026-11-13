# 작업 보고서: XIII.2.6 감시 넘침 시 전체 rescan

- WBS: XIII.2.6
- 작업명: watcher overflow 시 전체 project current pointer와 tracked files rescan 요청, 임의로 상태를
  clean 처리하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIII.2.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 외부 symlink fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/watcher/rescan.rs` (신규: `OverflowRecovery`, `RescanRequest`, `WatchState`,
  `TrackedFiles`, `FileFingerprint`, `RescanOutcome`, 테스트)
- `crates/project/src/watcher/coalesce.rs` (`ChangeCoalescer::rescan_pointer` 추가 — 조용한 구간 없이
  current pointer 즉시 재확인)
- `crates/project/src/watcher/mod.rs` (`rescan` 모듈 추가)
- `docs/task-reports/XIII.2.6.md`(신규), 계획서(XIII.2.6 `[ ]`→`[x]`).

## 구현 요약

- **요청**: `OverflowRecovery::on_events`는 event 중 `Overflow`가 있으면 상태를
  `RescanPending(reason)`으로 바꾸고 `RescanRequest { current_pointer: true, tracked_files: true }`를
  돌려줍니다. 기다리는 동안의 추가 넘침은 첫 사유로 합칩니다. 일반 event는 요청을 만들지 않고
  상태도 바꾸지 않습니다.
- **clean 가정 금지**: `RescanPending`은 `run_rescan`이 **성공**해야만 `Synced`로 돌아갑니다. 그 사이
  일반 event나 coalescer 결과가 비어도 상태는 바뀌지 않고, rescan이 실패하면(예: 프로젝트 폴더
  사라짐) pending을 유지합니다.
- **rescan 실행**: (1) `ChangeCoalescer::rescan_pointer`로 current pointer를 바로 확인해 외부
  generation을 XIII.2.5와 같은 결과(`Generation`·`InvalidGeneration`·`PointerUnavailable`)로 보고하고,
  (2) `TrackedFiles::capture`로 저장소 영역(`design/`)·`.git` 밖 프로젝트 파일 전체를 resolver(XIII.4)로
  훑어 내용 hash를 직전 기준과 비교해 추가·삭제·변경 경로를 돌려줍니다. 비신뢰 프로젝트의 외부
  symlink target은 내용을 읽지 않고 target 경로만 비교합니다(XIII.4.5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 228 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **overflow fixture에서 rescan 실행**: 넘침 동안 잃어버린 외부 generation·`stellar.yaml` 수정·에셋 추가·삭제가 rescan 1회로 모두 보고되고 `Synced`, 다음 rescan은 새 기준 대비 변화 0(`overflow_requests_and_runs_a_full_rescan`) | 통과 |
| 넘침 뒤 일반 event·빈 coalescer 결과로 clean 처리 안 됨, 추가 넘침 합침, rescan 실패 시 pending 유지·성공 시 `Synced`(`state_is_not_marked_clean_without_a_rescan`) | 통과 |
| 비신뢰 프로젝트 rescan은 외부 target 내용을 읽지 않음(`untrusted_rescan_compares_external_targets_without_reading_them`) | 통과 |

## 차이(작업 지시와 다른 점)

- "tracked files"는 Git index가 아닌 "저장소 영역과 `.git` 밖의 프로젝트 파일 전체"로 정의했습니다.
  Git 상태 조회는 신뢰된 프로젝트에서만 가능하고(XVI.7.1) 넘침 복구는 신뢰 상태와 무관해야 하기
  때문입니다. Git index 기준 비교는 Git status(XIV.2)에서 다룹니다.
- 실제 커널 drop은 테스트에서 일으킬 수 없어 넘침 event를 직접 넣었습니다. FSEvents drop flag →
  `Overflow` 변환은 XIII.2.2 normalize 테스트가 검증합니다.

## 남은 위험 / 한계

- tracked files rescan은 모든 파일 내용을 다시 읽어 hash하므로 큰 에셋이 많은 프로젝트에서는 비용이
  큽니다. 크기·수정 시각 선검사는 성능 작업에서 추가할 수 있습니다.
