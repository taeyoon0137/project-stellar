# 작업 보고서: VI.4.3 Fragment visual bounds helper

- WBS: VI.4.3
- 작업명: Fragment visual bounds를 가시 자식 bounds의 union으로 계산하는 read-only helper
- 상태: 완료 (`[x]`)
- 선행: VI.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/fragment.rs` — `fragment_visual_bounds`, 내부 `union_rect`.
- `docs/task-reports/VI.4.3.md`(신규), 계획서(VI.4.3 `[ ]`→`[x]`).

## 구현 요약

- **계산**: 자식을 OrderKey 순(VI.2.4)으로 순회하며 숨김 자식을 제외하고, 가시
  자식 bounds의 합집합을 반환합니다. 중첩 Fragment는 재귀적으로 그 가시 자식의
  합집합을 쓰고, 숨김 Fragment는 자손 전체가 제외됩니다. 가시 자식이 없으면
  `None`입니다.
- **좌표계**: Fragment는 좌표계를 만들지 않으므로(VI.3.2) leaf bounds는 호출자가
  containing Frame 좌표계로 제공합니다(`leaf_bounds` 콜백). helper는 문서를
  변경하지 않는 순수 조회입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 75 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 빈 fragment → `None`(`empty_fragment_has_no_bounds`) | 통과 |
| 숨김 자식 제외(`hidden_children_are_excluded`) | 통과 |
| nested fragment 재귀 합집합 + 숨김 nested 제외(`nested_fragments_union_recursively`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- leaf bounds 계산(Shape·Path·Text 등의 실제 geometry)은 각 종류 구현(VI.5~VI.8)과
  layout(Phase X)이 제공합니다.
