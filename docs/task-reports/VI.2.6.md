# 작업 보고서: VI.2.6 count/depth incremental 훅

- WBS: VI.2.6
- 작업명: node count·depth를 O(1)/incremental metadata로 유지하는 훅
- 상태: 완료 (`[x]`)
- 선행: VI.2.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/tree_stats.rs`(신규) — `TreeStats`(on_insert/on_remove/
  on_reparent 훅), `compute_stats`(검증 기준 전체 재계산).
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.2.6.md`(신규), 계획서(VI.2.6 `[ ]`→`[x]`).

## 구현 요약

- **훅 계약**: insert 직후 `on_insert`(count O(1) 증가, depth = 부모+1),
  leaf remove 직전 `on_remove`(O(1) 감소), reparent 직후 `on_reparent`(이동한
  subtree만 depth 재계산 — incremental).
- **metadata**: node별 depth map + depth 히스토그램. `max_depth`는 히스토그램
  끝 조회라 삭제로 최대 depth가 줄어도 정확합니다.
- **정확성 기준**: 전체 재계산 `compute_stats`와의 일치를 모든 테스트에서 교차
  검증합니다. 실제 성능 최적화(자식 인덱스 등)는 계획대로 후속이며, 훅 계약을
  유지한 채 내부 구조를 교체할 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 49 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| insert/move/delete 후 count·depth 정확(`insert_move_delete_keep_count_and_depth_accurate`) | 통과 |
| 500회 무작위 연산에서 재계산 기준과 일치(`randomized_operations_match_recompute`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `on_remove`는 leaf 전제입니다(subtree 삭제는 자식부터 훅 호출). command
  계층(Phase VII)의 삭제 구현이 이 계약을 따라야 합니다.
