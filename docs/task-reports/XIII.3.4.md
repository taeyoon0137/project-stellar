# 작업 보고서: XIII.3.4 세 snapshot reconciliation 연결

- WBS: XIII.3.4
- 작업명: Phase XIV Semantic Merge 완료 후 세 snapshot reconciliation을 연결하고 겹치지 않는 변경만 자동 적용
- 상태: 완료 (`[x]`)
- 선행: XIII.3.2, XIV.10.9
- 결정: [ADR-0110](../adr/ADR-0110-window-reconciliation-and-merge-result-apply.md) 결정 1·2·3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 fixture. 셸 UI·FFI 없음).

## 변경 파일 목록

- `crates/semantic-merge/src/apply.rs`(신규), `apply/tests.rs`(신규), `lib.rs`(mod 등록) — 병합 결과 적용 판정
  `reconcile`·`Reconciliation`(KeepOurs·TakeTheirs·Merged·Conflicted·Blocked).
- `crates/ops/src/merge_apply.rs`(신규), `merge_apply/document.rs`(신규), `merge_apply/tests.rs`·`tests/document.rs`(신규),
  `lib.rs`(mod 등록) — 문서 하나의 3-way `merge_document`(무손실 왕복 조건, `ConflictEntry` 목록).
- `crates/runtime/src/reconcile.rs`(신규), `reconcile/tests.rs`(신규), `lib.rs`(mod 등록) — LoadedBase 읽기
  `base_records`, 창 병합 `merge_window`, 공개 `ExternalConflict`·`ReconcileBlock`.
- `crates/runtime/src/window_session.rs`(소유 밖, 최소 연결) — `sync_external`의 외부 writer + dirty 분기에서
  `reconcile_external` 호출, clean 반영 끝부분을 `adopt_external`로 나눠 두 경로가 공유, 병합 불가 기록
  `external_conflict` 필드와 공개 조회 `external_conflict()`, `stale_judgement`가 그 관찰값에 한해 두 선택지.
- `crates/runtime/src/app/autosave.rs`(소유 밖, 최소 연결) — 자동 저장 writer가 외부 writer stale을 만나면 바로 반영을
  미뤄 두어 병합, 병합 반영 뒤 stale 해제와 자동 저장 일정 재시작.
- `crates/runtime/src/window_session/stale_tests.rs`, `source_tests.rs`, `app/autosave/pipeline_tests.rs`(소유 밖) —
  "외부 writer + dirty → 대기"를 고정했던 시험 세 개를 XIII.3.4 동작으로 바꾸고 겹침 시험 하나를 더함.

## 구현 요약

- 판정: theirs = base → ours 유지, ours = base 또는 ours = theirs → theirs, 그 밖은 `CandidateSnapshot::build` 병합.
  conflict가 있으면 자동 적용하지 않고, conflict 0이면 `CandidateSnapshot::validate`를 통과해야 결과로 씁니다.
- 세 입력: LoadedBase는 창의 loaded generation에서 그 문서 chunk만 다시 읽고(current와 무관), CurrentMemory는 창 문서,
  ExternalDisk는 열기와 같은 `read_disk_state`. 세 입력 모두 snapshot 무손실 왕복을 확인합니다.
- 쓸 수 있는 결과: `ApplyExternalSnapshot` transaction 하나로 history에 기록하고 base를 디스크 상태로 바꿉니다.
  결과가 디스크와 같으면 clean, 다르면 미저장 편집으로 남아 자동 저장이 씁니다. reconciliation 자체는 쓰지 않습니다.
- 병합 불가(겹침·결과 위반·base 없음·문서 삭제·무손실 실패): 문서·history 그대로, 이유를 디스크 관찰값과 함께 기록하고
  그 관찰값에 한해 두 선택지(`StaleResolution::Choose`)입니다. 디스크가 다시 바뀌면 다시 병합합니다.
- runtime → semantic-merge 직접 간선을 두지 않고 기존 runtime → ops 간선의 `merge_apply::document`를 씁니다(ADR-0110 결정 3).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 통과(apply 6 포함) |
| `cargo test -p stellar-ops --lib merge_apply` | 통과(document 4) |
| `cargo test -p stellar-runtime` | lib 276 통과·4 ignored, 통합 시험·doctest 통과 |
| `cargo clippy -p stellar-semantic-merge -p stellar-ops -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |

| 검사(완료 기준) | 결과 |
|---|---|
| non-overlap fixture 자동: CLI가 node 11, 창이 node 12를 바꿈 → `Applied`, 두 변경 모두 창에 있음, 쓰기 0, stale 해제, 미저장 편집 | 통과(`stale_tests`) |
| 병합 결과 저장 → 디스크에 두 변경, Undo 한 번이 병합 반영 전체를 되돌림 | 통과(`reconcile::tests`) |
| 텍스트 원본을 외부가 바꾸고 창이 다른 node를 편집 → 병합, 저장한 원본에 두 변경 | 통과(`source_tests`) |
| 앱 runtime: 감시 반영으로 병합, 두 선택지 alert 0, 자동 저장 재개로 디스크에 두 변경 | 통과(`pipeline_tests`) |
| overlap conflict: 같은 node 이름을 다르게 바꿈 → 문서·history 그대로, `ExternalConflict::Conflicts` 1건(node 12), 두 선택지 | 통과 |
| conflict 뒤 다시 외부 변경 → 다시 reconciliation 대기 → 다시 판정 / UseLatestDisk·UseCurrentWindow 실행 | 통과 |
| 앱 runtime overlap: 두 선택지 alert 한 번, 자동 저장 쓰기 0, UseCurrentWindow 저장 | 통과 |
| 양쪽 같은 변경 → 반영 뒤 clean | 통과 |
| base generation chunk가 없음 → 병합하지 않음(`BaseUnavailable`), 문서 그대로 | 통과 |

## 차이(작업 지시와 다른 점)

- lane 정의의 "runtime → semantic-merge 의존 간선 추가"는 하지 않았습니다. 기존 runtime → ops 간선으로 같은 판정·투영을 써서
  두 번째 경로를 만들지 않았습니다(ADR-0110 결정 3). crate graph는 그대로입니다.
- 소유 범위 밖인 `window_session.rs`·`app/autosave.rs`와 시험 세 파일을 최소로 고쳤습니다. 병합 결과를 창 history에 넣고
  자동 저장을 다시 돌리는 일은 그 파일의 비공개 상태를 거쳐야 해서입니다.

## 남은 위험 / 한계

- 셸 표시(두 선택지 시트의 conflict 목록)는 UI 재설계 때 `ProjectWindowSession::external_conflict`를 읽어 만듭니다. `AppRuntime`
  수준 조회 API는 더하지 않았습니다(`external_status`의 `stale`이 `Choose`로 알림).
- writer 종류 구분(CLI·MCP·Git, XXIV.4.6)과 agent lease 겹침(XIX.7.2)은 이 경로 위에서 할 일입니다.
- 문서 밖 의미(컴포넌트·token)는 기존대로 library 변경 경로가 받고, 이 병합 대상이 아닙니다.
- 대형 문서에서 reconciliation 시간은 재지 않았습니다.
