# 작업 보고서: XI.1.2 Scene node 평가 결과 field 정의

- WBS: XI.1.2
- 작업명: Scene node에 world transform, layout rect, clip stack ref, paint stack ref, z-order, hit-test metadata 정의
- 상태: 완료 (`[x]`)
- 선행: XI.1.1 (이 lane에서 완료: `3615863`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/node.rs` — `SceneNodeSpec`·`SceneNode`에 `world_transform`, `layout_rect`, `clip`,
  `paint`, `z`, `hit` 추가. `ZOrder`, `Visibility`, `PointerEvents`, `HitTestMeta`, `SceneTreeError`.
  `SceneNode::new`가 z-order를 검사하도록 `Result` 반환으로 변경.
- `crates/scene/src/stack.rs`(신규) — `ClipStackRef`/`ClipShape`(불변 linked list), `PaintStackRef`/
  `PaintStack`/`PaintLayer`/`Paint`/`StrokeLayer`/`StrokeAlign`/`StrokeCap`/`SceneBlendMode`/`Opacity`.
- `crates/scene/src/snapshot.rs` — 최상위 node 사이 z-order 검사(`SceneSnapshot::new`가 `Result`).
- `crates/scene/src/codec.rs` — 새 field의 canonical encode/decode와 오류(`InvalidColor`,
  `InvalidShadow`, `InvalidTree`).
- `crates/scene/src/lib.rs` — `stack` 모듈, 테스트 전용 `fixtures` 모듈 등록·재노출.
- `crates/scene/src/fixtures.rs`(신규, 테스트 전용) — SplitMix64 기반 결정론적 scene fixture 생성기.
- `docs/task-reports/XI.1.2.md`(신규), 계획서(XI.1.2 `[ ]`→`[x]`).

## 구현 요약

- **world transform / layout rect**: node local → world `Transform2D`(조상 합성 완료 값)와 node local
  layout 상자 `Rect`.
- **clip stack ref**: `ClipStackRef`는 조상 clip의 불변 linked list입니다. 자식 scope는 `push`로
  항목 하나만 새로 만들고 나머지를 `Arc`로 공유합니다. clip은 world 변환과 함께 저장되어
  가시성·hit test가 조상을 거슬러 올라가지 않아도 됩니다. 비교는 값 기준입니다.
- **paint stack ref**: fill·stroke layer(아래→위), 모든 layer 아래 drop shadow, subtree 합성 결과에
  적용하는 opacity·blend·layer blur. blend tag는 document `BlendMode`, stroke cap tag는 document
  `LineCap`과 같습니다. `StrokeLayer::outset`은 정렬·cap별 bounds 확장 거리를 줍니다.
- **z-order**: `ZOrder(u64)` 전역 paint 순서 key. 불변식은 "snapshot pre-order에서 엄격 증가"이며
  값이 연속일 필요는 없습니다. `SceneNode::new`는 직접 자식만, `SceneSnapshot::new`는 최상위
  node만 검사하고 node마다 subtree 마지막 z(`last_z`)를 캐시해 지역 검사만으로 전체 불변식이
  성립합니다(이후 구조 공유 갱신이 전체 재검사를 하지 않도록).
- **hit-test metadata**: 조상 상태를 반영한 실효 `Visibility`(Hidden은 구조 유지·미표시·hit 제외)와
  `PointerEvents`(None은 통과).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | 26 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test -p stellar-renderer-core` | 24 passed, 0 failed |

| 검사 | 결과 |
|---|---|
| 340 node fixture 생성: 모든 content 종류, 비항등 transform, rect·path clip, fill·stroke·shadow·blend, Hidden·pointer None 포함, pre-order z 엄격 증가, 같은 seed 같은 bytes·다른 seed 다른 bytes, canonical round-trip(`fixture_generation_fills_every_node_field`) | 통과 |
| 자식·형제·최상위 z-order 위반 거부(`children_must_follow_parent_in_z_order`, `z_order_must_increase_across_roots`, `z_order_violation_is_rejected_on_decode`) | 통과 |
| clip stack 조상 공유·값 비교(`clip_stack_shares_parent_entries`), paint 값 검증·outset(`paint_values_are_validated`) | 통과 |
| 새 field 손상 입력 거부(transform·clip tag·opacity·blend·blur·visibility·pointer·색상 알파·layer·cap) | 통과 |
| leaf node bytes 수기 확인(`header_bytes_are_hand_checked`) | 통과 |

## 차이(작업 지시와 다른 점)

- z-order를 "전역 key + 지역 검사 불변식"으로 정의했습니다. 연속 번호를 강제하지 않아 구조 편집 시
  재번호 범위를 증분 builder(XI.2.5)가 줄일 수 있게 했습니다.
- stroke 정렬(`StrokeAlign`)은 document에 아직 필드가 없어, Shape·Path는 document Line의 중심선
  규칙(Center), Frame 테두리는 크기 상자 안(Inside)에 매핑하기로 정했습니다(builder XI.2.2에서 적용).

## 남은 위험 / 한계

- stroke join·dash는 XI.6.2(renderer-core)에서 정의됩니다. 그 전까지 `outset`은 miter join 확장을
  반영하지 않습니다.
- `Paint`는 단색만 있습니다. gradient paint는 gradient 렌더링(XI.10.3) 때 variant로 추가해야 합니다.
- scene enum tag(blend·cap·fit·sampling·winding)와 document tag의 동등성은 두 crate를 보는 crate가
  없어 교차 테스트하지 못했습니다(대응은 각 enum 문서에 명시).
