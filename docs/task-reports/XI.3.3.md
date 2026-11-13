# 작업 보고서: XI.3.3 broad-phase hit test

- WBS: XI.3.3
- 작업명: z-order 역순, clip, visibility, pointer-events semantic을 적용한 broad-phase hit test
- 상태: 완료 (`[x]`)
- 선행: XI.3.1 (이 lane에서 완료: `d09b95a`), XI.1.2 (이 lane에서 완료: `363b5ad`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/hit_test.rs`(신규) — `hit_test_broad`, `hit_test_view`, `HitResult`(`topmost`,
  `z_orders`), `HitCandidate`, `HitStats`, `clip_contains`, `clip_stack_contains`, `rounded_rect_contains`.
- `crates/scene/src/lib.rs` — `hit_test` 모듈 등록.
- `docs/task-reports/XI.3.3.md`(신규), 계획서(XI.3.3 `[ ]`→`[x]`).

## 구현 요약

- **후보**: 공간 인덱스(XI.3.1)의 점 질의(world 그리기 영역 기준 상위 집합).
- **visibility**: 실효 Hidden 제외. **pointer-events**: 실효 None은 건너뛰어 아래 node가 대상이 됩니다
  (잠금 통과). 실효 값은 builder가 조상 상태를 반영해 둡니다(XI.2.2).
- **node bounds**: 점을 world transform 역변환으로 node local 좌표에 옮겨 local hit bounds(geometry +
  stroke)에 포함되는지 봅니다. 회전·기울인 node의 방향을 고려하며, 역변환 불가 node는 맞지 않습니다.
  결과에 local hit 지점을 함께 줍니다(narrow-phase 입력).
- **clip**: clip stack 전부를 통과해야 합니다. rect clip은 모서리 radius까지 정확히, path clip은
  broad-phase에서 control point 껍질로 판정합니다(보수적, 정밀 판정은 XI.3.4).
- **순서**: z-order 내림차순 정렬, 첫 항목이 topmost입니다. view 좌표 입력은 viewport 역변환으로 world에
  옮깁니다(`hit_test_view`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 70 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| overlap fixture (80,80): 겹친 5개 node가 z 역순 `[5,4,3,2,1]`, topmost D·local 지점 (10,10); (45,45)는 `[3,1]`(`overlap_fixture_returns_topmost`) | 통과 |
| D 숨김 + C 잠금 → topmost B, 통계 hidden 1·pass_through 1(`pointer_none_and_hidden_pass_through_to_lower_nodes`) | 통과 |
| 조상 clip 밖 자식 제외, 둥근 모서리 clip 밖 자식 제외·안쪽은 자식 topmost(`clip_excludes_points_outside_ancestor_clip`) | 통과 |
| 45° 회전 node: 안쪽 점 hit, AABB 모서리 점 miss(`rotated_nodes_use_oriented_bounds`) | 통과 |
| view 좌표 → world 변환 hit(`view_space_hit_uses_viewport_inverse`) | 통과 |
| path clip control 껍질 판정, 축척 0 clip 불통과, 모서리별 radius 판정(`path_clip_uses_control_hull_in_broad_phase`) | 통과 |
| 455 node fixture 무작위 점 300개에서 결과 = brute force 판정(z 역순)(`hits_match_brute_force_predicate_on_random_fixture`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- node content(path 내부·glyph·image) 정밀 판정과 path clip 정밀 판정은 narrow-phase XI.3.4 범위입니다.
  XI.3.4는 선행 X.10.3(text caret·hit-test cluster, `[ ]`)이 미완료라 이 lane에서 진행하지 못했습니다.
- 역변환은 float 반올림이 있어 경계 위 점의 판정이 1 ulp 수준에서 흔들릴 수 있습니다(경계 포함 규칙).
- hit test 지연(p99) 측정은 XI.3.6 benchmark 범위입니다(XI.3.4에 의존해 차단).

## 후속 수정 (리뷰 minor: brute-force oracle이 구현과 같은 판정 함수를 공유)

- **지적**: `hits_match_brute_force_predicate_on_random_fixture`가 구현과 같은 `to_local`·`clip_stack_contains`를
  써서, 공간 인덱스 누락과 z 역순만 증명하고 clip·모서리 radius 수식은 독립 검증하지 못했습니다.
- **추가 테스트**(test-only, 구현 변경 없음):
  - `rounded_rect_and_rotated_clip_match_independent_sdf_oracle`: 사분면별 radius rounded-box signed distance(구현의
    모서리 영역 판정과 다른 수식)로 radius 4종 × 0.5 간격 격자 38,801점에서 `rounded_rect_contains`를, 같은 점을 30°
    회전·평행이동 clip의 world로 보내 `clip_contains`를 비교합니다. local 좌표는 `Transform2D::inverse` 대신 회전 성분으로
    직접 되돌립니다. 경계 0.01 이내 점은 부동소수 오차로 빼고, clip stack 교집합 4점은 손으로 확인했습니다.
  - `random_fixture_clip_semantics_match_independent_oracle`: 780 node 무작위 fixture에서 local 좌표(평행이동 직접 계산)와
    clip 판정(SDF, path clip은 command 점을 직접 훑은 껍질)을 독립 oracle로 계산해 `hit_test_broad` 결과와 비교합니다.
    node bounds(XI.3.1, 자체 테스트 보유)만 공유합니다.
- **변이 확인**: `rounded_rect_contains`의 top-right 모서리 radius를 bottom-left로 바꾸면 SDF oracle 테스트가, clip stack
  판정을 가장 안쪽 clip만 보게 바꾸면 두 oracle 테스트가 실패했습니다(기존 brute-force 테스트는 후자에서도 통과).
  변이는 확인 후 되돌렸습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 77, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
