# 작업 보고서: VII.5.4 창 닫기 시 history·선택 폐기

- WBS: VII.5.4
- 작업명: 창 닫기 시 WindowHistory와 selection을 폐기하고 project 파일에는 기록하지 않음
- 상태: 완료 (`[x]`)
- 선행: VII.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/window.rs` — `WindowHistories::close`, 테스트.
- `docs/task-reports/VII.5.4.md`(신규), 계획서(VII.5.4 `[ ]`→`[x]`).

## 구현 요약

- `close(window)`는 그 창의 `WindowHistory`(undo·redo stack, view 상태의 선택)를 버립니다. history는 세션 메모리 전용이고 저장 경로가 없어
  프로젝트 파일에 아무것도 기록하지 않습니다(§23.1). 같은 창 id로 다시 열면 빈 history·빈 선택입니다. 다른 창은 영향이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 21 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 편집 3·undo 1·선택 1 후 닫기 → 조회 불가·중복 닫기 `false`, 문서 레코드 불변, 다른 창 유지, 재오픈 시 undo·redo·선택 비어 있음·undo `None`(`closing_a_window_discards_history_and_selection`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 창 종료 시 문서 flush(§22.1)는 창 세션·저장 계층(XII·XIII)의 몫입니다.
