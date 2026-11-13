# 작업 보고서: VII.3.3 영역 선택 transient preview와 grouping API

- WBS: VII.3.3
- 작업명: 영역 선택 중간 상태는 transient preview로 유지하고 pointer up 때 최종 SetSelection 한 건만 생성
- 상태: 완료 (`[x]`)
- 선행: VII.3.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/selection.rs` — `WindowViewState`의 영역 선택 API(`begin/update/finish/cancel_area_selection`,
  `displayed_selection`, `area_selection_active`), `AreaSelectionMode`, `AreaSelectionError`, 테스트 2건.
- `docs/task-reports/VII.3.3.md`(신규), 계획서(VII.3.3 `[ ]`→`[x]`).

## 구현 요약

- **pointer down** `begin_area_selection(mode)`: transient preview를 엽니다. 확정 선택은 바뀌지 않습니다.
- **pointer move** `update_area_selection(hits)`: preview만 교체합니다(Replace: 영역 hit, Add: 시작 전 선택 + hit). history event를
  만들지 않으며 화면은 `displayed_selection`(preview)을 그립니다.
- **pointer up** `finish_area_selection()`: preview를 닫고 기록할 **최종 `SetSelection` 한 건**을 돌려줍니다. 확정 선택과 같으면 `None`.
  event의 적용(필터 포함)과 기록은 history가 맡습니다.
- **취소** `cancel_area_selection()`: preview를 버리고 event를 만들지 않습니다(§23.3: 취소된 조작은 기록하지 않음).
- preview는 `WindowViewState` 안에만 있어 저장·history 대상이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 61 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 1,000 move 동안 확정 선택·기록 불변, preview만 표시 → pointer up에서 event 1건 기록, 마지막 preview가 최종 선택(`area_selection_produces_one_event_for_many_moves`) | 통과 |
| Add 모드 합집합·primary 유지, 중복 begin 거부, 취소는 event 없음, 비활성 상태 update/cancel 거부, 변화 없는 영역 선택은 `None`(`area_selection_add_cancel_and_no_change`) | 통과 |

## 차이(작업 지시와 다른 점)

- history crate(VII.4)가 아직 없어 "history 1건"은 테스트의 기록 목록으로 확인했습니다. VII.4.6/VII.5.x에서 실제 history와 함께
  다시 검증합니다.

## 남은 위험 / 한계

- 영역 hit 계산(어떤 node가 영역에 걸리는지)은 hit test(XI.3) 소관이며 이 API는 결과 id 목록을 받습니다.
