# 작업 보고서: X.1.2 Fragment flatten layout tree view

- WBS: X.1.2
- 작업명: layout tree view가 Fragment를 flatten하고 ComponentInstance virtual subtree와 View Variant 적용 결과를 읽도록 구성(원본 document 비수정)
- 상태: 완료 (`[x]`)
- 선행: VI.4.1(`[x]`), IX.2.2(`[x]`), IX.7.6(`[x]`) — 명시 행 선행 모두 기준 commit에서 `[x]`, 완화 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/view.rs`(신규) — `LayoutSource` trait, `SourceNode`, `build_layout_tree`, `LayoutTreeView`,
  `ViewError`/`SourceViewError`, `virtual_layout_id`, `MAX_VIEW_NODES`, 테스트.
- `crates/layout/src/lib.rs` — `view` 모듈 등록.
- `docs/task-reports/X.1.2.md`(신규), 계획서(X.1.2 `[ ]`→`[x]`).

## 구현 요약

- **crate 경계**: `stellar-layout`은 core-types만 의존하므로 문서 모델(Fragment payload, instance expansion view,
  View Variant resolver)을 직접 읽지 않습니다. 문서 쪽 adapter가 `LayoutSource`(`&self` 읽기 전용: `root`, `node`,
  `children`, `layout_id`)를 구현하고, view는 그 응답만 읽습니다. trait에 변경 API가 없어 view가 원본을 수정할 경로가
  없습니다.
- **원천 책임**: ComponentInstance의 `children`은 expansion view의 가상 자식(정의 tree 투영)이고, `SourceNode::Node`의
  속성은 View Variant·instance override 적용이 끝난 값입니다. 가상 node의 layout id는 `virtual_layout_id(chain, source)`
  (domain 분리 hash 128비트, UUID version 8로 표시해 문서 v7 id와 겹치지 않음)로 만들 수 있습니다.
- **Fragment flatten**: Fragment의 자식을 Fragment 자리에서 깊이 우선(중첩 포함) 가장 가까운 box 조상의 자식으로
  펼치고, 펼친 순서대로 view 전용 균등 분포 `OrderKey`를 다시 매깁니다. 숨겨진 Fragment의 자식은 숨김으로 펼칩니다.
  `fragment_host(key)`가 Fragment 자식을 받은 box를 알려 줍니다(X.7.1 전파용).
- **검증·상한**: 반복(비재귀) 구성, 방문 node 수 상한 `MAX_VIEW_NODES`(큰 자식 목록은 쌓기 전에 거부), 같은 원천 key
  반복(공유·순환) 거부, layout id 중복 거부, root Fragment 거부, 결과 입력은 `LayoutInput::validate` 통과.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 147 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| flatten 전(원천 tree를 테스트가 독립 재귀로 펼친 visual order) = flatten 후(layout 입력 root 자식의 OrderKey 순서): `[2, 3, 4, 5, 6, 7]`, Fragment box 없음, 가로 Stack x 위치가 그 순서로 증가, 결정적 bytes(`flatten_preserves_visual_child_order_fixture`) | 통과 |
| 숨긴 Fragment의 자식 숨김·형제 당김, 자식 자신의 숨김 유지(`hidden_fragment_hides_its_flattened_children`) | 통과 |
| 두 인스턴스·nested 인스턴스 virtual subtree: 인스턴스별 다른 가상 id, 정의 안 Fragment flatten, 원천 상태 불변, 같은 subtree 크기(`component_instance_reads_virtual_subtree_without_duplicating_definitions`) | 통과 |
| 원천이 해석한 View Variant 값(compact gap)을 그대로 읽음, 순서 불변(`view_variant_values_are_read_as_resolved_by_the_source`) | 통과 |
| root Fragment, 공유 자식, 순환, 원천 오류, leaf 자식, layout id 중복, node 상한 거부(`invalid_sources_are_rejected`) | 통과 |
| 가상 id 결정성·chain 민감성·version 8(`virtual_ids_are_deterministic_and_chain_sensitive`) | 통과 |

## 차이(작업 지시와 다른 점)

- 문서 모델 adapter(실제 `SceneIndex`·`InstanceExpansion`·View Variant resolver를 `LayoutSource`로 감싸는 구현)는 이
  crate가 문서 crate에 의존할 수 없어 포함하지 않았습니다. 테스트는 같은 계약을 따르는 원천 fixture(scene node,
  `(chain, 정의 node)` 가상 key, variant gap)로 검증했습니다.

## 남은 위험 / 한계

- adapter가 trait 계약(같은 key 응답 일정, 해석 끝난 값 제공)을 지키는지는 adapter를 두는 상위 crate(runtime 등)의
  통합 테스트가 확인해야 합니다.
- view는 전체 tree를 한 번에 펼칩니다. 변경 node만 다시 읽는 증분 갱신은 X.7 증분 레이아웃 경로(node 단위 edit)에서
  다룹니다.
