# 작업 보고서: XIV.3.5 move/reorder 분류

- WBS: XIV.3.5
- 작업명: move/reorder는 parent ID와 OrderKey의 의미 순서를 비교해 분류하고 저장 chunk 이동은 semantic change로 표시하지 않음
- 상태: 완료 (`[x]`)
- 선행: III.5.2 `[x]`, V.6.5 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/diff.rs` — 같은 정체성 객체의 위치 비교(MoveObject·ReorderObject).
- `docs/task-reports/XIV.3.5.md`(신규), 계획서(XIV.3.5 `[ ]`→`[x]`).

## 구현 요약

- **분류**: parent가 다르면 MoveObject(from/to에 parent와 새 OrderKey), parent가 같고 OrderKey가
  다르면 ReorderObject입니다. 순서 비교는 OrderKey의 의미 순서(byte 사전순)이며 같은 key면
  변경이 없습니다. node·modifier·View(parent = page) 모두 같은 규칙입니다.
- **저장 배치 무관**: snapshot에는 레코드 배열 순서·chunk·파일 위치가 없습니다(XIV.3.1). 따라서
  레코드를 다른 chunk 묶음·다른 순서로 다시 직렬화해도 변경이 0입니다.
- **순서**: 같은 객체 안에서 위치 변경을 속성 변경보다 앞에 둡니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 30 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| rechunk-only: 50 seed random document를 seed별 임의 chunk 묶음·역순으로 node record canonical bytes 왕복(배치가 실제로 다름을 확인), semantic diff 0(`rechunk_only_change_has_zero_semantic_diff`) | 통과 |
| 같은 parent OrderKey 변경 → ReorderObject만, 자식 순서 반영(`reorder_within_parent_is_not_move`) | 통과 |
| parent 변경 → MoveObject(새 key 포함)만, 이동+속성 변경 시 위치가 먼저(`parent_change_is_move_with_new_key`) | 통과 |
| modifier 항목 OrderKey 변경 → ReorderObject(`modifier_reorder_is_reorder`) | 통과 |

## 차이(작업 지시와 다른 점)

- V.6.5 merge planner는 project/storage crate에 있어 직접 호출할 수 없습니다(II.2.8 그래프).
  rechunk fixture는 같은 레코드를 다른 chunk 묶음으로 canonical 직렬화·복원하는 방식으로
  "chunk 이동만 있는 저장"을 모사했습니다.

## 남은 위험 / 한계

- OrderKey rebalance처럼 형제 상대 순서는 같고 key bytes만 바뀐 경우도 저장 레코드가 실제로
  바뀌므로 ReorderObject로 보고합니다(정확한 재현을 위해 필요). UI가 묶어 보여줄지는
  Status UI(XIV.13) 결정입니다.
