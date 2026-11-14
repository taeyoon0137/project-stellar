# 작업 보고서: XIII.1.2 완료된 document transaction만 autosave dirty 생성

- WBS: XIII.1.2
- 작업명: 완료된 document CommandTransaction만 autosave dirty state를 만들고 Selection·viewport·preview
  event는 만들지 않게 함
- 상태: 완료 (`[x]`)
- 선행: VII.3.2(`[x]`), V.5.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/autosave/dirty.rs`(신규): `DirtyTrackedHistory`, `AutosavePlan`, `AutosavePlanError`,
  테스트.
- `crates/project/src/autosave.rs`: `pub mod dirty;`와 모듈 문서 한 줄.
- `crates/project/Cargo.toml`: `stellar-commands`·`stellar-history` 의존 추가(crate graph 허용 edge,
  외부 crate 아님). 이에 따라 `Cargo.lock`의 `stellar-project` 의존 목록이 2줄 늘었습니다(자동 생성).
- `docs/task-reports/XIII.1.2.md`(신규), 계획서(XIII.1.2 `[ ]`→`[x]`).

## 구현 요약

- **분류를 경로로 강제**: `DirtyTrackedHistory`가 창 history(`WindowHistory`)를 소유하고 문서·선택을 바꾸는
  모든 경로를 자기 method로만 엽니다. 호출자가 change set 종류를 알려 주지 않으므로 preview change set을
  완료 편집으로 넘기거나 완료 편집을 빠뜨릴 수 없습니다.
  - dirty: `record_document`·`record_grouped`(완료 transaction, 효과가 있을 때), `end_preview`가 항목을
    기록(drag·scrub 확정), `undo`·`redo`의 문서 항목.
  - dirty 없음: `record_selection`, `undo`·`redo`의 선택 항목, `begin_pointer_gesture`·`begin_scrub`·
    `update_preview`(진행 중), `cancel_preview`. viewport는 창 view 상태라 history·문서 상태 밖이며 이 type에
    들어오는 경로 자체가 없습니다.
- **dirty 값**: 저장되지 않은 마지막 완료 revision(`DocumentRevision`). `mark_saved(revision)`은 dirty가 그
  이하일 때만 clean으로 만들어 저장 중에 이어진 편집을 잃지 않습니다. `reset_clean`은 외부 generation 재로드
  (XIII.3.1)용입니다.
- **저장 계획**: `plan_save`는 clean이면 문서를 직렬화하지 않고 `AutosavePlan::Clean`, preview 진행 중이면
  `PreviewInProgress`(미완료 변경이 메모리에 섞여 있음), 그 밖에는 문서를 직렬화해 V.5.1 `build_save_plan`
  결과를 `Ready { revision, plan }`으로 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 347 passed, 1 ignored(수동 system font scan); 통합 테스트 전부 통과) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **선택 1,000회 후 save plan 없음**: 선택 1,000회가 history 항목 1,000개로 기록되지만 dirty 없음, `plan_save` = `Clean`, 문서 revision 0(`a_thousand_selection_changes_create_no_save_plan`) | 통과 |
| 선택 항목 undo·redo 뒤에도 `Clean`(`undo_and_redo_of_selection_entries_create_no_save_plan`) | 통과 |
| preview 갱신 100회·취소는 `Clean`, 확정(`end_preview`)은 확정 revision으로 `Ready`(`preview_updates_and_cancel_create_no_save_plan_but_commit_does`) | 통과 |
| 완료 transaction은 dirty chunk가 있는 `Ready`, 저장 중 이어진 편집은 dirty 유지, 저장 후 `Clean`, 문서 항목 undo는 dirty(`completed_transactions_create_a_save_plan_and_saving_clears_it`) | 통과 |
| 완료 편집이 있어도 preview 중이면 `PreviewInProgress`, 취소 후 `Ready`(`preview_in_progress_defers_capture_of_earlier_completed_edits`) | 통과 |
| 실패한 transaction은 dirty 없음(`failed_transactions_create_no_dirty_state`) | 통과 |

## 차이(작업 지시와 다른 점)

- viewport event를 받는 API를 따로 두지 않았습니다. viewport는 `WindowHistory`·`DocumentState`에 없는 창 view
  상태라, dirty를 만들 수 있는 경로가 type에 없음을 문서로 명시했습니다.
- runtime의 창 session(현재 `WindowHistory`를 직접 씀)을 이 type으로 바꾸는 연결은 runtime crate 범위라 이
  작업에서 하지 않았습니다.

## 남은 위험 / 한계

- `DirtyTrackedHistory`를 거치지 않고 `WindowHistory`를 직접 쓰는 코드는 dirty를 만들지 못합니다. runtime
  연결 시 창 session이 이 type을 소유해야 합니다.
- 효과가 없는 완료 transaction(같은 값 설정)은 dirty를 만들지 않습니다. undo로 디스크와 같은 내용으로
  돌아가도 dirty입니다(저장 계획이 새 chunk를 만들지 않을 뿐).
- 자동 저장 scheduler(XIII.1.3)는 XIII.1.1이 `[~]`(config 수치 미정)라 이 lane에서 진행하지 않았습니다.
