# 작업 보고서: XI.3.2 viewport·clip 기반 가시 후보

- WBS: XI.3.2
- 작업명: viewport frustum과 clip bounds로 visible candidate 계산
- 상태: 완료 (`[x]`)
- 선행: XI.3.1 (이 lane에서 완료: `d09b95a`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/visibility.rs`(신규) — `Viewport`(화면 크기 + world→view 변환, world frustum),
  `ViewportError`, `ClipBounds`·`clip_world_bounds`·`clip_shape_world_bounds`, `visible_candidates`,
  `DrawList`·`DrawItem`·`CullStats`, `world_to_view`.
- `crates/scene/src/lib.rs` — `visibility` 모듈 등록.
- `docs/task-reports/XI.3.2.md`(신규), 계획서(XI.3.2 `[ ]`→`[x]`).

## 구현 요약

- **2D frustum**: 화면 사각형 `(0,0)~size`를 world→view 역변환으로 되돌린 AABB입니다. 역변환 불가
  (축척 0)·비유한 결과는 구조화 오류입니다.
- **후보 계산**: 공간 인덱스(XI.3.1)를 frustum으로 질의 → 숨김 제외 → node world 그리기 영역 ∩ frustum
  ∩ clip stack world bounds가 비면 제외 → z-order 오름차순(paint 순서) draw list. 항목마다 보이는 영역
  (보수적)과 통계(후보·숨김·clip·영역 밖)를 남깁니다.
- **clip bounds**: clip마다 world AABB(rect는 rect, path는 control point 껍질)를 교집합한 보수적
  값입니다. 회전·곡선 clip은 실제보다 넓어 화면 밖 조각이 남을 수는 있어도 보일 node를 빼지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 63 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 800×600 viewport: 화면 안 frame·자식은 포함, 완전히 밖 frame·숨김 frame·frame clip 밖 자식·화면 밖 clip frame의 화면 안 자식은 제외, 경계 걸친 frame은 포함되고 가시 영역이 화면으로 잘림(`offscreen_nodes_are_excluded_from_draw_list`) | 통과 |
| pan·zoom(2배) frustum 계산, 이동한 화면에서만 보이는 node 선택, 역변환 불가 오류(`pan_and_zoom_move_the_world_frustum`) | 통과 |
| 455 node fixture × 무작위 viewport 60개에서 draw list = brute force, paint 순서(`draw_list_matches_brute_force_on_random_fixture`) | 통과 |
| clip 교집합·서로소 clip은 Empty(`clip_bounds_intersect_conservatively`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- clip bounds가 AABB 교집합이라 회전된 clip·곡선 path clip에서는 culling이 덜 됩니다(정확성에는 영향
  없음). scissor/stencil/mask 계획은 XI.4.3(renderer-core)이 담당합니다.
- 불투명도 0 node도 draw list에 남습니다. 투명 node 제외 여부는 render graph compiler(XI.4.2) 정책입니다.
- 매 frame `SceneSpatialIndex::build`를 다시 하면 O(node 수)입니다. snapshot 간 증분 갱신은 XI.2.5 이후 과제입니다.
