# 작업 보고서: XI.3.1 world bounds 기반 공간 인덱스

- WBS: XI.3.1
- 작업명: world bounds 기반 공간 인덱스 인터페이스 정의와 insert/update/remove/query 구현
- 상태: 완료 (`[x]`)
- 선행: XI.1.2 (이 lane에서 완료: `363b5ad`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/bounds.rs`(신규) — 닫힌 축 정렬 사각형 `Aabb`(교차·포함·합집합·교집합·확장·이동·변환),
  `path_control_bounds`, `SceneNode::{local_content_bounds, local_hit_bounds, local_render_bounds,
  world_hit_bounds, world_render_bounds}`.
- `crates/scene/src/spatial.rs`(신규) — `SpatialIndex` trait(insert/update/remove/query_rect/query_point/
  bounds_of/len), 계층형 loose grid `GridIndex`, snapshot 어댑터 `SceneSpatialIndex`, `SpatialError`.
- `crates/scene/src/lib.rs` — `bounds`, `spatial` 모듈 등록.
- `docs/task-reports/XI.3.1.md`(신규), 계획서(XI.3.1 `[ ]`→`[x]`).

## 구현 요약

- **world bounds**: hit bounds = (layout 상자 ∪ content geometry) + 최대 stroke outset, render bounds =
  hit bounds ∪ drop shadow(offset + blur) + layer blur. world bounds는 local bounds 네 꼭짓점을 world
  transform으로 옮긴 AABB입니다. path는 control point 볼록 껍질(곡선을 항상 포함, 보수적)을 씁니다.
- **인터페이스**: `SpatialIndex<K>`는 key당 world `Aabb` 하나, 중복 insert·없는 key update는 구조화 오류,
  질의는 경계 포함 교차·순서 미정입니다(순서는 소비자가 정렬).
- **GridIndex**: level `L` cell 크기 `base·2^L`(2의 거듭제곱, 24 level), 항목은 최소·최대 꼭짓점 cell이
  축마다 1칸 이내인 가장 작은 level의 최소 꼭짓점 cell 하나에 저장합니다. 질의는 level마다
  `cell(min)-1 ..= cell(max)`를 보고, 범위가 사용 중 cell 수보다 크면 사용 중 cell을 훑습니다. 가장 큰
  cell보다 큰 항목은 별도 목록입니다. 무한 canvas, O(1) 평균 갱신(hash + swap-remove).
- **level 선택**: 너비의 float 반올림에 기대지 않고 질의와 같은 단조 함수(`floor(v / size)`)로 정해
  교차 항목을 놓치지 않습니다(반올림 경계 회귀 테스트 포함).
- **SceneSpatialIndex**: snapshot node를 world render bounds로 색인하고 key는 snapshot 안에서 유일한
  `ZOrder`입니다. 비유한 world bounds node는 색인하지 않고 개수만 셉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 59 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 20만 node(작은 80%, 중간 15%, 점 3%, 최대 level 초과 2%) insert → update 2만 → remove 2만 → 재insert 1만 후, 사각형 질의 150(작은·화면·축소 보기 크기)·점 질의 150·경계 위 점 200을 brute force와 비교해 모두 일치(`random_200k_nodes_query_correctness`) | 통과 |
| 중복 insert·없는 key update 오류, 같은 cell·다른 level·거대 항목 전환 update, swap-remove 뒤 slot 정합(`insert_update_remove_contracts`) | 통과 |
| 너비 반올림(64.0000038 → 64.0) 경계 항목 점 질의(`rounding_of_width_does_not_misplace_items`) | 통과 |
| fixture snapshot 색인 결과가 node world render bounds brute force와 일치(`scene_index_matches_brute_force_world_bounds`) | 통과 |
| `Aabb` 닫힌 구간·회전 변환·오버플로 처리, path control 껍질, stroke·shadow·blur bounds(`bounds::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `SceneSpatialIndex::build`는 snapshot 전체를 색인합니다(O(node 수)). 구조 공유를 이용한 증분 갱신
  (바뀐 subtree만 update)은 증분 빌더(XI.2.5) 범위입니다.
- path bounds가 control point 껍질이라 곡선이 많은 path는 실제보다 큰 후보 영역을 가집니다. 정확한
  판정은 narrow-phase(XI.3.4)가 담당합니다.
- stroke miter join 확장은 join 정의(XI.6.2) 전이라 bounds에 반영하지 않습니다.
- 성능 수치(질의 시간)는 이 작업 범위가 아니며 XI.3.6 benchmark에서 측정합니다.
