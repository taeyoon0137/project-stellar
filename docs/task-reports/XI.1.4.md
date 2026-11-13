# 작업 보고서: XI.1.4 구조 공유 persistent collection

- WBS: XI.1.4
- 작업명: 이전 snapshot과 구조를 공유하는 persistent collection 도입, 바뀐 경로만 새로 할당
- 상태: 완료 (`[x]`)
- 선행: XI.1.3 (이 lane에서 완료: `a4347ad`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/persistent.rs`(신규) — 32갈래 trie `PersistentVec`(`from_vec`, `get`, `set`, `iter`,
  `last`, `shared_leaf_chunks`), 값 기준 `PartialEq`.
- `crates/scene/src/node.rs` — 자식 목록을 `PersistentVec<Arc<SceneNode>>`로 교체, `with_spec`(자식 공유
  property 갱신), `with_child_replaced`(지역 z-order 검사), `spec()`, `child_list()`, `SceneEditError`.
- `crates/scene/src/snapshot.rs` — 최상위 목록을 `PersistentVec`으로 교체, `replace_node`(path copying),
  `sharing_with`/`SharingStats`, `root_list()`.
- `crates/scene/src/revision.rs` — `SceneRevisionMap::derive`(구조 공유 새 세대), `SceneGenerateError::Edit`.
- `crates/scene/src/lib.rs` — `persistent` 모듈 등록.
- `docs/task-reports/XI.1.4.md`(신규), 계획서(XI.1.4 `[ ]`→`[x]`).

## 구현 요약

- **persistent vector**: 원소를 32개 이하 leaf chunk에, chunk를 32개 이하 branch에 `Arc`로 담는
  bit-partitioned trie입니다. `set`은 root→leaf 경로의 chunk(높이 h면 h개, 각 32칸 이하)만 새로 만들고
  나머지 chunk를 공유합니다. 이전 vector는 바뀌지 않습니다. 외부 crate 없이 safe Rust로 구현했습니다.
- **path copying**: `SceneSnapshot::replace_node(path, node)`는 대상 node와 그 조상만 새로 할당합니다.
  형제 subtree, 자식 목록 chunk, clip·paint 참조는 공유합니다. 자식을 건드리지 않는 단일 property
  갱신은 `with_spec`으로 자식 목록을 O(1) 공유합니다.
- **z-order 검사**: 교체 시 경로 단계마다 앞 형제(또는 부모)·뒤 형제와의 순서만 검사하고, 최상위
  단계에서 앞뒤 root를 검사합니다. 전체 tree를 다시 순회하지 않습니다.
- **공유 통계**: `sharing_with(previous)`는 이전 snapshot의 node 주소 집합을 만들고, 새 snapshot에서
  공유 subtree는 내려가지 않고 `subtree_len`만 더합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | 38 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test -p stellar-renderer-core` | 24 passed, 0 failed |

| 검사 | 결과 |
|---|---|
| 4680 node(8×8×8×8) fixture에서 깊이 3 leaf 하나 교체: `total 4680 / shared 4676 / new 4`(leaf + 조상 3), 형제·다른 root `Arc` 동일, clip 공유, 이전 snapshot·hash 불변(`single_leaf_edit_shares_every_other_node`) | 통과 |
| frame property만 바꾸면 frame + root 2개만 새 node, 자식 subtree 공유(`frame_property_edit_keeps_children_shared`) | 통과 |
| 자식 5000개 frame에서 자식 하나 교체: node 2개만 새로, 자식 목록 leaf chunk 157개 중 156개 공유(`wide_frame_child_edit_copies_one_chunk_path`) | 통과 |
| z-order 범위 밖 교체·경로 범위 밖(단계 보고)·빈 경로·다음 root와 겹침 거부(`replacement_must_fit_z_range_and_path`) | 통과 |
| trie 높이 1~4 경계 길이(0~40000)에서 `get`·`iter`·`len` 일치, `set` 경로 복사·원본 불변(`persistent::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- "바뀐 subtree만 새 allocation"을 path copying으로 구현해, 바뀐 node의 **조상 node 기록**도 새로
  할당됩니다(자식 pointer가 바뀌므로 불가피). 조상의 다른 자식과 자식 목록 chunk는 공유합니다.
  검증 기준 수치는 이를 반영해 `new = 경로 길이`로 고정했습니다.

## 남은 위험 / 한계

- 자식 삽입·삭제는 아직 `SceneNode::new`로 해당 node 자식 목록을 다시 만듭니다(그 node에서 O(자식 수)).
  구조 편집 증분화는 XI.2.5 범위입니다.
- 구조 편집으로 z-order 간격이 부족하면 뒤쪽 node 재번호가 필요합니다. 간격 정책은 증분 builder
  (XI.2.5)에서 정해야 합니다.
- `sharing_with`는 이전 snapshot node 수만큼 주소 집합을 만듭니다(진단·테스트용, 매 frame 용도 아님).
