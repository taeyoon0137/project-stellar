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
| trie 높이 1\~4 경계 길이(0\~40000)에서 `get`·`iter`·`len` 일치, `set` 경로 복사·원본 불변(`persistent::tests`) | 통과 |

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

## 후속 수정 (리뷰 major: SceneNode Drop이 공유 자식 목록까지 순회)

- **결함**: 깊은 tree 반복 해제 수정(`ac1827b`)의 `SceneNode::drop`이 자식 목록 chunk를 다른 version과 공유할
  때도 모든 자식 `Arc`를 복제해 꺼낸 뒤 다시 줄였습니다. 그래서 node 하나를 해제하는 비용이 자식 수에 비례(원자
  증감 + 같은 크기 할당)했고, 넓은 frame 안의 편집이 남긴 이전 조상 version을 해제할 때(예: render thread가 오래된
  snapshot을 놓을 때, XI.1.5) 형제 전부를 순회해 이 작업의 구조 공유 비용 모델이 해제 경로에서 깨졌습니다.
- **수정**:
  - `PersistentVec` chunk 표현을 `Arc<[T]>`에서 크기가 정해진 `Arc<Vec<T>>`로 바꿔 `Arc::into_inner`로 유일 소유
    chunk를 넘겨받을 수 있게 했습니다(조회·순회·`set` 동작과 공개 API는 같음).
  - `PersistentVec::release_unique(self, out)` 추가: 명시적 stack으로 trie를 내려가며 마지막 참조인 chunk만 풀고
    그 원소를 복제 없이 `out`으로 옮깁니다. 공유 chunk는 참조만 하나 줄이고 멈춥니다(`ClipStackRef`가 공유 prefix에서
    멈추는 방식과 같음). `into_inner`이므로 같은 chunk를 동시에 해제하는 thread 중 정확히 하나만 원소를 받습니다.
    반환값은 놓은 chunk 수입니다.
  - `SceneNode`의 `detach_children`가 `release_unique`를 씁니다. 테스트 build에서만 놓은 chunk 수·떼어 낸 자식 수를
    thread별로 세는 계측(`drop_cost`)을 두었고, 그 밖에서는 빈 함수입니다.
- **회귀 테스트**:
  - `node::dropping_old_versions_of_a_wide_frame_skips_shared_child_chunks`: 자식 200,000개 frame에서 서로 다른
    위치의 자식을 하나씩 바꾼 version 200개를 만든 뒤 이전 version 200개를 해제합니다. 놓은 chunk ≤ 200 × 97(root +
    유일 branch 3개의 자식), 떼어 낸 자식 ∈ [200, 200 × 32]를 확인하고, 이전 version에만 있던 자식은 해제·공유 자식은
    최신 version만 보유(strong count 1)함을 확인합니다. 마지막 version 해제 시 자식 200,000개가 모두 해제됩니다.
  - `persistent::release_unique_stops_at_chunks_shared_with_another_version`: 길이 200,000에서 `set`한 이전 version
    해제는 놓은 chunk ≤ 97, 넘어온 원소는 바뀐 leaf 하나(32개)뿐이고 남은 version은 그대로임을, root 공유 복제본은
    chunk 1개만 놓음을, 마지막 version은 모든 원소를 넘김을 확인합니다.
- **변이 확인**: `detach_children`를 이전 구현(전체 복제)으로 되돌리면 node 회귀 테스트가 `떼어 낸 자식 40000000`으로
  실패하고 실행 시간이 12.89 s(debug)였습니다. 수정 후 같은 테스트는 debug 0.11 s, `--release` 0.04 s입니다(200,000개
  자식 생성 포함). 되돌린 변이는 확인 뒤 복원했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 79, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test -p stellar-renderer-core -p stellar-export` | export unit 55·doc 0, renderer-core unit 95·doc 3 passed, 0 failed |

- **남은 한계**: chunk마다 `Vec` header 할당이 하나 늘었습니다(32칸 chunk당 약 40 B와 할당 1회). snapshot 최상위
  목록(`SceneSnapshot` roots)은 기본 drop을 쓰며, 그 원소 `SceneNode` 해제는 위 반복 Drop이 처리합니다.
