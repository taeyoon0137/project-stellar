# 작업 보고서: VIII.3.1 ExpressionGraphView

- WBS: VIII.3.1
- 작업명: AST를 GUI block 목록과 edge 목록으로 읽는 `ExpressionGraphView` — 별도 원본 상태 없음
- 상태: 완료 (`[x]`)
- 선행: VIII.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/graph_view.rs`(신규) — `BlockId`, `SocketId`, `BlockView`, `EdgeView`, `ExpressionGraphView`
  (`new`, `blocks`, `edges`, `block`, `source_of`, `consumers_of`, `to_ast`), 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.3.1.md`(신규), 계획서(VIII.3.1 `[ ]`→`[x]`).

## 구현 요약

- **block** = AST node(ID = node StableId), payload는 AST에서 빌린 참조(`&NodePayload`)라 view가 별도 원본 값을 갖지
  않습니다. 입력 socket = child slot(`SocketId { block, slot }`), 결과 block 표시(`is_root`), GUI용 `label`(연산 기호·함수 이름).
- **edge** = child block 출력 → 부모 block 입력 socket. 공유(DAG) node는 block 하나에 출력 edge 여러 개입니다.
- **순서**: block은 canonical 순서(VIII.2.5), edge는 (부모 block 순서, slot 순서)라 같은 AST는 삽입 순서와 무관하게 같은 view입니다.
- **역투영**: `to_ast`는 block payload와 edge만으로 AST를 재구성하고 `ExpressionAst::new` 검증을 거칩니다. view에는
  편집 API가 없으며, 편집은 AST edit plan(VIII.3.3) 적용 후 view를 다시 만드는 방식입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (73 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **AST→view→AST 의미 동일**(`ast_to_view_to_ast_is_semantically_identical`): match·call·unary·공유 node AST의 재구성 결과가 원본과 동등, canonical bytes 동일, 재투영 view 동일 | 통과 |
| literal 단독·binary AST round-trip(`single_literal_and_binary_round_trip`) | 통과 |
| block 수 = node 수, edge 수 = child 참조 수, canonical block 순서, root 1개, 공유 block 출력 edge 3개, socket 조회, payload가 AST 참조(포인터 동일)(`blocks_and_edges_mirror_nodes_and_child_refs`) | 통과 |
| 삽입 순서와 무관한 view(`view_is_independent_of_insertion_order`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- block 좌표 등 GUI 배치 정보는 projection에 포함하지 않습니다. 배치 저장 여부와 위치는 Phase XII UI 연결에서 정해야 합니다.
