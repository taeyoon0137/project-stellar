# 작업 보고서: VI.2.4 OrderKey 정렬 자식 조회

- WBS: VI.2.4
- 작업명: 자식 조회가 OrderKey 정렬 결과를 반환하고 arena 삽입 순서에 비의존
- 상태: 완료 (`[x]`)
- 선행: VI.2.2, III.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/tree_ops.rs` — `children_in_order` 추가.
- `crates/document/src/node_columns.rs` — slot별 live 핸들 추적
  (`handle_by_slot` column)과 `iter_live` 순회 API 추가(column invariant 검사
  포함).
- `docs/task-reports/VI.2.4.md`(신규), 계획서(VI.2.4 `[ ]`→`[x]`).

## 구현 요약

- **`children_in_order(columns, parent)`**: 부모의 자식들을 `(OrderKey,
  StableId)` 순으로 정렬해 반환합니다. StableId tie-break로 OrderKey가 겹쳐도
  결정론적입니다(유일성 강제는 VI.10.1).
- **arena 순서 비의존**: `iter_live`의 slot 순서는 구현 세부이며 결과 정렬이
  이를 지웁니다 — 같은 tree를 6가지 삽입 순열로 만들어도 자식 순서가 동일함을
  테스트로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 42 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 6개 삽입 순열의 child order 동일 + 의미 순서 일치(`child_order_is_independent_of_insertion_order`) | 통과 |
| 다른 부모·삭제 node 제외, leaf 빈 결과 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 조회가 전체 live 순회(O(n))입니다. 자식 인덱스 최적화는 VI.2.6 훅과 후속 성능
  작업에서 다룹니다.
