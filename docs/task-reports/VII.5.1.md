# 작업 보고서: VII.5.1 WindowHistory

- WBS: VII.5.1
- 작업명: `WindowHistory`를 WindowSessionId별로 생성하고 다른 창 entry를 포함하지 않음
- 상태: 완료 (`[x]`)
- 선행: VII.4.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/window.rs`(신규) — `WindowHistory`(창 id + history + view 상태 + 선택 정책), `WindowHistories`(창별 모음), 테스트.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.5.1.md`(신규), 계획서(VII.5.1 `[ ]`→`[x]`).

## 구현 요약

- **`WindowHistory`**: 창 하나(`WindowSessionId`)의 `History`와 그 창의 `WindowViewState`(선택)를 함께 소유합니다. 기록·preview·선택·undo는
  모두 이 창의 stack과 선택에만 작용하고, 문서 기록 뒤 삭제된 node는 이 창 선택에서 정리합니다(VII.3.4 system event).
- **`WindowHistories`**: 열린 창마다 `WindowHistory`를 하나 만듭니다(`open`은 이미 있으면 기존 것). 창마다 sequence 공간이 독립입니다.
- 문서 상태는 창 세션(XII.2.1)이 소유하며 각 호출에 그 창의 `DocumentState`를 넘깁니다(§22.2: 창마다 독립 문서 상태).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 17 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 창 A/B 교차 편집 6회 + A 선택: 각 stack의 정방향 id 집합 = 자기 창 transaction만, A 7·B 6 항목, 창별 sequence 1..n, 선택 창별, A undo가 B stack·문서 불변, 재오픈은 기존 history(`interleaved_edits_keep_window_stacks_separate`) | 통과 |

## 차이(작업 지시와 다른 점)

- 창 닫기(VII.5.4)·redo(VII.5.3)는 해당 작업에서 추가합니다.

## 남은 위험 / 한계

- `DocumentState`에는 창 id가 없어, 다른 창의 문서를 넘기는 호출 실수는 창 세션 통합(XII.2.1)에서 소유 관계로 막아야 합니다.
