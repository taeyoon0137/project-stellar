# 작업 보고서: VII.5.5 undo·redo 중 선택 복원 reentrancy guard

- WBS: VII.5.5
- 작업명: Undo/Redo 중 생성되는 selection restoration이 새 history entry를 만들지 않게 reentrancy guard 추가
- 상태: 완료 (`[x]`)
- 선행: VII.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/history.rs` — `replaying` guard, undo/redo를 공통 `replay`로 통합, `undo_with`/`redo_with`(선택 복원 직후 hook을
  guard 안에서 호출), `is_replaying`, `note_selection`, 재생 중 `record_selection`은 무기록·문서 기록은 `Replaying` 거부.
- `crates/history/src/entry.rs` — `SelectionRestore { before, after }`, `HistoryEntry::selection`, `HistoryError::Replaying`.
- `crates/history/src/window.rs` — 문서 기록·grouping 병합·preview 확정 시 편집 직전/직후 선택 기록, `undo_with`/`redo_with`, 테스트 2건.
- `docs/task-reports/VII.5.5.md`(신규), 계획서(VII.5.5 `[ ]`→`[x]`).

## 구현 요약

- **선택 복원**: 창 history는 문서 항목마다 편집 직전·직후 선택(`SelectionRestore`, 삭제 정리 반영)을 기록합니다(병합 시 직전은 처음 값 유지).
  undo는 직전 선택을, redo는 직후 선택을 필터를 거쳐 복원하며 이 복원은 history에 기록되지 않습니다.
- **reentrancy guard**: 재생(undo·redo) 동안 `replaying`을 켜고, 선택 복원 직후 UI 선택 변경 알림을 본뜬 hook(`undo_with`/`redo_with`)을 guard
  안에서 부릅니다. 그 안에서 `record_selection`은 항목을 만들지 않고(`Ok(None)`), 문서 기록·grouping·재귀 재생은 `Replaying`으로 거부합니다.
  hook이 끝나면 guard를 끕니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 23 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 선택·편집 혼합 100항목 → undo 100회(매회 hook이 복원 선택 echo + 다른 선택 기록 시도 200회 모두 무기록, 문서 기록 `Replaying`): 매회 undo = 100−k·redo = k, guard 해제, 최종 빈 선택 → redo 100회 대칭(`selection_restoration_during_undo_creates_no_entries`) | 통과 |
| 선택 [13,12] 후 13 포함 subtree 삭제: 선택 정리(항목은 삭제 1건), 복원 정보 before/after 정확, undo 시 [13,12] 복원·항목 증가 없음, redo 시 [12](`undo_restores_selection_from_before_the_edit`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선택 복원 정보를 남기는 것은 창 history(`WindowHistory`) 경로입니다. `History`를 직접 쓰는 호출은 복원 정보 없이 기록됩니다.

## 남은 위험 / 한계

- 실제 UI 알림 경로(네이티브 셸 → core)와 hook 연결은 창 세션 통합(XII.6.4)에서 합니다.
