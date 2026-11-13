# 작업 보고서: VIII.2.1 AST node StableId·kind·typed child refs

- WBS: VIII.2.1
- 작업명: 모든 AST node에 StableId, node kind, typed child refs 정의
- 상태: 완료 (`[x]`)
- 선행: VIII.1.1(`[x]`), III.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/ast.rs`(신규) — `ExprNodeId`, `ChildSlot`(안정 tag·canonical 순서), `ChildRef`,
  `ExprNode`, `ExpressionAst`, `AstError`, kind별 slot 구성 검증, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.2.1.md`(신규), 계획서(VIII.2.1 `[ ]`→`[x]`).

## 구현 요약

- **node 공통 구조**: `ExprNode`는 `ExprNodeId`(`StableId` typed ID), `NodeKind`(VIII.1.5의 닫힌 7개),
  `Vec<ChildRef>`(slot + 대상 node ID)를 가집니다. child는 ID 참조라서 한 node를 여러 부모가 공유하는 DAG를
  표현합니다. child 목록은 생성 시 slot 순서로 정렬해 연결 순서와 무관하게 같은 node가 됩니다.
- **slot**: `Operand`, `Lhs`, `Rhs`, `Argument(i)`, `Condition`, `Then`, `Else`, `Scrutinee`, `Arm(i)`,
  `Default`. 선언 순서가 canonical slot 순서이고, `tag`/`index`/`from_parts`로 안정 직렬화 tag를 제공합니다.
- **검증(`ExpressionAst::new`)**: ID 유일성 → root 존재 → kind별 slot 구성(필수 slot 누락, 허용되지 않는
  slot, 중복 slot, 순번 slot 빈틈, Match arm ≥ 1) → child 참조 해석 → root 도달 가능성. 도달성 검사는 명시적
  stack DFS라 공유·순환 참조에서도 종료합니다. AST 동등성은 삽입 순서와 무관합니다.
- cycle 금지 검사는 VIII.2.3, kind별 payload(리터럴 값·operator·함수 ID·pattern)는 VIII.2.2에서 추가합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (42 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **ID uniqueness**(`id_uniqueness_is_enforced`): 중복 node ID·root와 child ID 충돌을 `DuplicateNodeId`로 거부 | 통과 |
| **tree validation**(`tree_validates`): 5 node tree 통과 | 통과 |
| **DAG validation**(`shared_child_dag_validates`): 한 leaf를 3개 slot이 공유하는 DAG 통과, 부모 수 정확 | 통과 |
| 구조 오류(`structural_errors_are_reported`): root 없음, dangling child, 도달 불가 node, 필수 slot 누락, Literal의 child, 중복 slot | 통과 |
| 순번 slot 연속성·Match 구성(`sequence_slots_must_be_contiguous`) | 통과 |
| child 연결 순서·node 삽입 순서와 무관한 동등성(`child_order_and_insertion_order_do_not_matter`) | 통과 |
| slot tag round-trip·canonical 순서(`child_slot_tags_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 도달 불가 node를 구조 오류로 거부합니다. 하나의 AST가 root에서 닿는 node만 담게 해 canonical 직렬화
  (VIII.2.5)와 GUI projection(VIII.3.1)의 원본을 단일하게 유지하기 위해서입니다.

## 남은 위험 / 한계

- 이 단계의 검증은 cycle을 거부하지 않습니다(VIII.2.3에서 추가).
- node 수·깊이 상한은 두지 않았습니다. 큰 AST의 평가 제한은 VIII.4.5 소관입니다.
