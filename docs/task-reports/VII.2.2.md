# 작업 보고서: VII.2.2 DeleteSubtree

- WBS: VII.2.2
- 작업명: `DeleteSubtree` — 삭제 전 subtree와 내부 참조를 inverse payload에 보존, 외부 inbound ref는 정책 오류
- 상태: 완료 (`[x]`)
- 선행: VII.2.1, VI.2.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/node_ops.rs` — `DeleteSubtree`를 subtree 전체로 확장(validate/invert/apply), 테스트 2건 추가.
- `crates/commands/src/tree.rs` — `ChildIndex::subtree`(pre-order), `reference_index`(VI.2.5 `ReferenceIndex` 구성).
- `crates/commands/src/command.rs` — `CommandError::ExternalReference { target, from }`.
- `docs/task-reports/VII.2.2.md`(신규), 계획서(VII.2.2 `[ ]`→`[x]`).

## 구현 요약

- **대상**: root와 모든 자손(pre-order, 형제는 OrderKey 순). View root·없는 node는 거부합니다.
- **inbound ref 정책**: 문서의 node 간 참조(Frame clip 대상, modifier 참조 — 비활성 포함)를 document crate의
  `ReferenceIndex`(VI.2.5)로 구성하고, subtree 안 node를 subtree **밖** node가 참조하면 삭제하지 않고
  `ExternalReference { target, from }`를 반환합니다(index는 사실만 보고하고 정책은 command 단계, VI.2.5 지침).
- **inverse payload**: 삭제 전 subtree 레코드(pre-order)와 그 node들의 modifier stack을 담은 `InsertNode`입니다. subtree
  안 참조(clip·modifier)는 레코드·stack에 그대로 있어 undo가 복원합니다.
- **apply**: 역 pre-order(자손 먼저)로 leaf 삭제 primitive를 반복합니다. change set은 subtree 전체를 removed, 부모를 구조
  무효화로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 35 passed, doctest 3 passed, 0 failed(2.28s) |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| clip·(비활성) mask로 서로 참조하는 subtree: 일부만 삭제 → `ExternalReference`·문서 불변, 전체 삭제 → removed 3건·pre-order inverse, undo 후 semantic snapshot 동일(`delete_subtree_preserves_internal_references_and_rejects_external`) | 통과 |
| random valid document 150 seed의 비root node 1/3: 삭제 성공 시 removed = subtree, undo 후 레코드 동일(seed마다 semantic snapshot 동일), 거부 시 참조자가 subtree 밖·문서 불변, 삭제 > 300·거부 > 20(`delete_undo_round_trips_on_random_documents`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (VII.2.1에서 역 command용으로 등록한 `DeleteSubtree`를 subtree 전체로 완성했습니다.)

## 남은 위험 / 한계

- 참조 index를 실행마다 문서 전체에서 다시 만들고, `DocumentModel::remove_leaf`가 자식 확인에 전체 scan을 써 삭제 비용은
  O(subtree × N)입니다. 증분 참조 index와 자식 index가 생기면 줄일 수 있습니다.
- 컴포넌트 인스턴스 등 Phase IX 참조는 아직 `DocumentModel` 레코드에 없어 검사 대상이 아닙니다.
