# 작업 보고서: VII.2.1 InsertNode

- WBS: VII.2.1
- 작업명: caller가 StableId를 제공하는 `InsertNode` — duplicate, invalid parent, invalid OrderKey 검증
- 상태: 완료 (`[x]`)
- 선행: VII.1.6, VI.2.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/node_ops.rs`(신규) — `InsertNode`(validate/invert/apply/codec), 역 command `DeleteSubtree`
  (현재 자식 없는 node만), `existing_stacks`, 테스트.
- `crates/commands/src/tree.rs`(신규) — 부모·깊이·종류·OrderKey 조회, `ChildIndex`, 문서 영속 id 공간(`id_owners`).
- `crates/commands/src/command.rs` — `Command::InsertNode`/`DeleteSubtree`, 검증 오류 variant(`NilId`, `DuplicateId`,
  `InvalidParent`, `DuplicateOrderKey`, `DepthExceeded`, `ReferenceTypeMismatch`, `Modifier`, `ModifierCycle`,
  `ViewRoot`, `InvalidRequest`).
- `crates/commands/src/kind.rs` — 등록 `InsertNode = 0x0001`, `DeleteSubtree = 0x0002`(golden 테스트).
- `crates/commands/src/codec.rs`, `crates/commands/src/transaction.rs`, `crates/commands/src/lib.rs` — variant
  직렬화 연결, VII.1.2·VII.1.6의 임시 `allow(unreachable_code)`/`allow(dead_code)` 제거.
- `docs/task-reports/VII.2.1.md`(신규), 계획서(VII.2.1 `[ ]`→`[x]`).

## 구현 요약

- **입력**: caller가 정한 StableId의 node 하나(`InsertNode::single`) 또는 그 node를 root로 하는 pre-order subtree와
  그 modifier stack. 삭제 undo(VII.2.2)가 같은 command로 subtree를 복원합니다.
- **검증**(적용 전, 문서 무변경):
  - duplicate: node·path point/handle·modifier id가 NIL이 아니고 요청 안에서 유일하며 문서의 영속 id 공간(문서·
    페이지·View·node·modifier·path element, VI.10.1과 같은 공간)에서 미사용.
  - invalid parent: root 부모는 문서에 있는 container(Frame·Fragment)여야 하며 부모 없는 root 삽입은 거부(View root는
    View 정의 소관). 자손의 부모는 앞선 레코드인 container. 깊이 ≤ 64.
  - invalid OrderKey: root key가 기존 형제 key와 충돌하거나 요청 안 형제끼리 충돌하면 거부(`OrderKey` 정규형은 타입이
    보장).
  - 참조 종류: Frame clip 대상은 Path, modifier 참조 대상은 node(대상 부재는 미해결 진단이라 허용).
  - modifier: 소유 node가 요청 안 node, Fragment 금지(VI.9.6), 소유자당 stack 하나, 문서 전체 참조 cycle 금지(VI.9.5).
- **inverse**: `DeleteSubtree { root }`. 이번 작업의 `DeleteSubtree`는 InsertNode의 역으로 쓰는 범위(자식 없는 node,
  레코드와 stack을 InsertNode로 복원)만 구현했습니다.
- **change set**: 삽입 node마다 inserted, 부모는 구조 무효화.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 33 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| subtree+stack 삽입 → inverse(DeleteSubtree) 적용 시 snapshot 원복, 삭제의 역(InsertNode)이 레코드·stack 복원(`insert_apply_inverse_round_trip`) | 통과 |
| 기존 node·문서 정의·요청 안 중복 id, NIL, path point id 충돌 거부(`duplicate_ids_are_rejected`) | 통과 |
| 부모 없음·없는 부모·Text 부모·subtree 밖 부모·빈 요청 거부(`invalid_parent_is_rejected`) | 통과 |
| 형제 OrderKey 충돌(문서·요청 안), 깊이 65 거부(`invalid_order_key_and_depth_are_rejected`) | 통과 |
| clip 종류 불일치, Fragment modifier, 정의 id 참조, 자기 mask cycle 거부(`references_and_modifier_rules_are_checked`) | 통과 |
| View root·없는 node 삭제 거부, InsertNode(0x0001)·DeleteSubtree(0x0002) codec round-trip(`delete_rules_and_codec_round_trip`) | 통과 |
| registry golden(`registry_numbers_are_golden`) | 통과 |

## 차이(작업 지시와 다른 점)

- inverse로 쓸 삭제 command가 필요해 `DeleteSubtree` 종류를 이번 작업에서 등록하되, 자식 없는 node 삭제만 지원합니다.
  subtree 보존·내부 참조·외부 inbound ref 정책은 VII.2.2에서 완성합니다.
- `InsertNode`는 단일 node 외에 pre-order subtree도 받습니다(삭제 undo가 같은 경로를 쓰도록).

## 남은 위험 / 한계

- id 중복·참조·cycle 검사는 문서 전체를 한 번 훑습니다(O(N)). 대형 문서의 반복 삽입 비용은 증분 id/참조 index가 생기면
  줄일 수 있습니다.
