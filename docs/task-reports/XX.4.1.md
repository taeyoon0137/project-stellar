# 작업 보고서: XX.4.1 부분 래스터화 경계 planner

- WBS: XX.4.1
- 작업명: unsupported effect를 가진 최소 scene subtree와 외부 blend dependency를 분석하는 raster boundary planner
- 상태: 완료 (`[x]`)
- 선행: XI.4.4 (`[x]`), XX.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/svg/raster.rs`(신규) — `plan_raster`, `RasterPlan`, `RasterBoundary`, `RasterRegion`,
  `RasterCause`, `BlendDependency`, `BackdropItem`, `RasterNode`, `RasterScope`, `raster_scope`, `PixelGrid`,
  `PixelRect`.
- `crates/export/src/svg/raster/tests.rs`(신규) — nested blur/blend fixture와 규칙별 테스트.
- `crates/export/src/svg/raster/tests/painter.rs`(신규) — 기호 painter 모사와 무작위 scene 비교.
- `crates/export/src/svg/mod.rs` — 모듈 선언·재노출.
- `crates/export/src/svg/mapping.rs` — BlendMode row 구현 위치를 `XX.4.1` → `XX.4.2`로 갱신(경계 분석은 이
  작업에서 끝나고, 래스터 렌더·삽입이 남은 차단 요인).
- `docs/task-reports/XX.4.1.md`(신규), 계획서(XX.4.1 `[ ]`→`[x]`).

## 구현 요약

- **대상과 범위**: mapping table에서 지원 상태가 `Rasterize`인 기능만 다룹니다. `raster_scope`가 wildcard 없이
  기능마다 범위를 정합니다. drop shadow·image 색공간 override는 `Content`(node 자기 내용만, 자식 vector),
  layer blur는 `Subtree`, Normal 외 blend는 `Backdrop`입니다. 두 표의 일치는 inventory test로 고정했습니다.
- **blend backdrop 의존**: blend node `N`에서 가장 가까운 isolation group `G`(opacity < 1·blend·blur 조상,
  없으면 canvas)까지 올라가며 단계마다 앞선 내용(조상 자기 내용, 경로 자식보다 앞선 형제 subtree, canvas
  단계는 배경과 앞선 최상위 node)이 `N` footprint와 겹치는지 봅니다. 겹침이 있는 가장 높은 단계 `J`에 따라
  영역은 `N` subtree, `p_J` subtree(조상 자기 내용이 겹침), `p_J` 아래 연속 형제 범위(앞선 형제만 겹침),
  canvas 전체(배경이 겹침) 중 하나입니다. 영역은 paint 순서상 연속이고 영역 밖 앞선 내용은 `N`과 겹치지
  않으므로, 영역을 투명 바탕에서 합성한 image를 첫 위치에 Normal로 그리면 원래 합성과 같습니다. 겹치지
  않는 형제·조상은 vector로 남습니다.
- **footprint**: 자기 내용은 scene world render bounds(stroke·shadow·자기 blur 포함)를 내보내기 pixel
  grid(영역 원점, 요청 scale)에서 바깥 정수화합니다(1/256 pixel 오차 안은 정수로 붙임). subtree는 자기 내용
  ∪ 자식 합집합이고, blur node는 자식 합집합을 반경 × world 변환 최대 특이값(최소 1) × scale만큼 넓힙니다.
  blur node는 subtree가 겹치면 그 node 자체를 겹침으로 봅니다. 겹침은 canvas 안에서 양의 넓이를 가진
  교집합입니다.
- **병합**: 후보를 pre-order 구간으로 정렬해 포함되는 영역은 흡수하고, 같은 부모 아래 부분 겹치는 형제 범위는
  합칩니다. 그 밖의 부분 겹침은 tree 구조상 생기지 않으며, 생기면 canvas 전체로 물러납니다(틀린 결과 대신
  넓은 래스터). canvas 후보가 있으면 전체가 canvas 하나입니다. 이유는 node paint 순서·기능 순서로 모읍니다.
- **조회**: `boundary_at(z)`(영역 시작 node), `rasterizes(z)`, 경계별 pixel 범위·래스터 node 수, 계획 전체의
  vector node 수. XX.4.2(래스터 삽입)·XX.4.3(report)·XX.4.4(vector node 수 회귀)가 씁니다.
- 순회는 모두 명시적 stack·배열이며 재귀가 없습니다. 출력은 입력 값에만 의존합니다(`HashMap` 미사용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 74 passed, 0 failed (이 작업 신규 19) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 (의존 변경 없음: core-types, scene) |
| `git diff --check` | 출력 없음 |

| 검사 | 결과 |
|---|---|
| nested blur/blend fixture 20 node(숨김 1): 경계 6개의 영역·이유·isolation·backdrop 원인 문자열, 경계별 pixel 범위·래스터 node 수(2, 2, 3, 2, 1, 3)가 기대값, 보이는 19개 중 vector 6개(P·c·G·g3·Sc·O)(`nested_blur_blend_fixture_boundaries_are_exact`) | 통과 |
| 경계 조회: 시작 node만 `boundary_at`, content 경계는 자식 미포함, 숨김 node 미포함(`boundary_lookup_by_start_node`) | 통과 |
| 같은 부모의 부분 겹침 형제 범위 병합, 떨어진 범위 분리(`overlapping_sibling_ranges_merge_and_disjoint_ones_stay_apart`) | 통과 |
| isolation group 자기 내용이 backdrop이면 group subtree가 영역(`isolation_group_content_pulls_the_group_into_the_region`) | 통과 |
| 최상위 blend는 앞선 최상위 node에 의존, 배경을 칠하면 canvas 전체, group 안 blend는 배경 미의존, 투명 단색 배경은 투명과 같음(`top_level_blend_depends_on_earlier_roots_and_background`, `plan_from_capture_uses_export_grid_and_background`) | 통과 |
| pixel 규칙: 맞닿음은 무관, pixel 공유는 의존, scale 2에서 분리, 1/256 오차 정수화, canvas 밖 겹침 무관, 영역 원점 기준(`pixel_grid_overlap_rules`, `canvas_size_follows_bounds_and_scale`) | 통과 |
| blur 반경이 backdrop을 넓힘(pixel 12까지, 13은 무관), blur group이 자식 내용을 퍼뜨림, 변환 배율 상한(`blur_spreads_the_backdrop_footprint`, `blurred_group_spreads_child_content_into_the_backdrop`, `blur_pixels_use_the_transform_scale_bound`) | 통과 |
| 숨긴 node는 경계도 backdrop도 아님, image 색공간 override는 content 경계, blend 범위 안 shadow 흡수 | 통과 |
| 같은 입력·다른 thread에서 계획 동일, 깊이 2000 chain을 128 KiB stack thread에서 계획, View fixture는 래스터 없음 | 통과 |
| 기호 painter 모사 무작위 scene 500개: 원래 합성과 경계 image 합성이 canvas 모든 pixel에서 같음, 보이는 래스터 기능 node는 모두 경계 안. 경계 있는 scene 442, Subtrees 경계 657, backdrop으로 넓힌 범위 46, 그 scene들에서 vector로 남은 node 768(`minimal_regions_preserve_the_composite_on_random_scenes`) | 통과 |
| 검출력: 조상 자기 내용 검사 제거, 한 단계만 올라가기, 자식 blur 확장 제거, 배경 의존 제거, blur node 겹침 판정 제거를 각각 임시로 넣으면 무작위 비교가 실패. 형제 범위 병합 제거는 canvas로 물러나 정확성은 유지되고 병합 테스트가 실패. 변경은 모두 되돌림 | 확인 |

## 차이(작업 지시와 다른 점)

- 영역 단위를 "subtree 하나"보다 작게 잡았습니다. 앞선 형제만 backdrop이면 같은 부모 아래 연속 형제 범위를,
  shadow·image 색공간 override는 node 자기 내용만 래스터화합니다. 둘 다 아키텍처 §34의 "최소 하위 트리"를
  더 작게 만족하며, 위 모사 비교로 합성 결과가 같음을 확인했습니다.
- 지시의 검증 기준은 "nested blur/blend fixture boundary 정확"입니다. fixture 기대값만으로는 정확성을
  보장할 수 없어 기호 painter 무작위 비교를 추가했습니다.
- `crates/export`는 ADR-0019에 따라 core-types·scene에만 의존하므로 renderer-core의 XI.4.4 분석을 쓰지 않고
  같은 원리(합성 효과 node는 subtree를 따로 합성, pixel 영역 겹침 판정)로 export 쪽에 따로 구현했습니다.

## 남은 위험 / 한계

- 합성 수식이 투명 source에서 backdrop을 바꾸지 않는다고 가정합니다(모든 separable blend mode의 성질). 수식
  고정은 XI.10.2(`[ ]`)이며, 이를 어기는 mode가 추가되면 그 mode는 isolation group 전체를 영역으로 잡도록
  바꿔야 합니다.
- clip으로 footprint를 줄이지 않습니다(효과와 clip의 적용 순서는 XI.10.4 미고정). clip이 큰 내용을 잘라
  실제로 겹치지 않아도 의존으로 보아 영역이 커질 수 있습니다. 결과가 틀리지는 않습니다.
- 배경을 칠하는 내보내기에서 isolation group 밖 blend는 canvas 전체를 래스터화합니다. 불투명 배경 위 영역만
  image로 덮어쓰는 최적화는 하지 않았습니다(XX.4.4 회귀 기준 검토 대상).
- 겹침 판정 grid는 내보내기 scale입니다. SVG를 다른 배율로 볼 때의 anti-aliasing 경계 pixel 차이는 판정
  범위 밖이며, 래스터 image 해상도·삽입(XX.4.2)이 같은 grid를 써야 합니다.
- 래스터 렌더와 SVG `<image>` 삽입은 XX.4.2입니다. 그 전까지 writer는 래스터 기능을 typed error로 거부합니다.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p stellar-export`는 기존 mapping·id·request·capture 등에서 같은
  이유(`pub mod` 바깥 `///` 문서와 안쪽 `//!` 문서가 합쳐져 상위 module 기준으로 링크를 풂)로 실패합니다. 이
  작업 전부터 있던 상태이며 검증 기준 밖이라 고치지 않았습니다. 이 작업의 `raster` module은 후속 수정에서
  링크를 전체 경로로 바꿔 경고를 더하지 않습니다(아래).

## 후속 수정 (리뷰 minor: 넓은 형제 목록에서 blend 영역 검색의 제곱 작업량)

- **지적**: blend node마다 isolation group까지 단계별로 앞선 형제를 처음부터 하나씩 훑어(형제마다 stack
  `Vec` 할당), 한 부모 아래 blend 형제가 많으면 작업량이 형제 수의 제곱이었습니다. 결과는 맞지만 큰 내보내기가
  오래 멈출 수 있었습니다.
- **재현**: 겹치지 않는 1×1 blend 4096개를 한 부모 아래와 최상위에 나란히 둔 테스트에서 겹침 판정 횟수를
  셌습니다(`PixelGrid::overlaps`를 test build에서만 세는 thread-local 계수기). 수정 전 8,390,656번으로 상한
  `4096 × 64`를 넘어 실패했습니다.
- **수정**: 단계 `i`의 앞선 내용이 pre-order `[p_i, p_{i-1})`에 있어 높은 단계일수록 앞선다는 점을 써서,
  isolation group 자기 내용(canvas 단계는 배경)을 먼저 보고 나머지는 pre-order 구간 하나에서 첫 겹침을 찾도록
  바꿨습니다. 첫 겹침은 pre-order entry 8개씩을 잎으로 묶은 footprint 합집합 구간 tree에서 왼쪽부터 찾고,
  합집합이 겹치지 않는 tree node는 건너뜁니다(부모를 거슬러 오른쪽 형제로 옮겨 stack·할당 없음). 찾은 내용이
  경로 위 조상이면 그 조상 subtree, 아니면 경로와 갈라지는 단계까지 올라간 형제부터 경로 자식까지의 범위입니다.
  영역·backdrop 의존 규칙은 바뀌지 않았습니다. 같은 테스트에서 겹침 판정은 132,536번(부모 아래)과 93,226번
  (최상위)입니다.
- **같음 검증**: 수정 전 단계별 순차 탐색을 테스트 기준 구현(`raster/tests/reference.rs`)으로 옮기고, 무작위
  scene 600개(node 2–11개 밀집, 2–81개, 2–401개 성긴 scene)의 모든 blend node에서 배경 유무 둘 다 영역과
  backdrop 의존이 같은지 비교합니다. blend 24,332건(isolation group 안 10,388, 앞선 형제 범위 2,391, 조상
  자기 내용 3,501, 잎 block 하나 넘게 앞선 backdrop 1,480)입니다. 무작위 scene 생성기는 node 수와 위치 범위를
  인자로 받게 했고, 기존 painter 비교는 같은 인자(10, 26)로 같은 scene을 만듭니다.
- **변이 확인**: isolation group 자기 내용 검사 제거, 검색 구간에 isolation group 포함, 잎 block에서 구간 시작
  무시, blur node를 자기 내용으로 판정, 갈라지는 단계 판정 오류를 각각 넣으면 기준 비교 테스트가 실패합니다
  (구간 시작 두 변이는 이 테스트만 잡음). 변경은 모두 되돌리고 `cmp`로 원본 복원을 확인했습니다.
- **rustdoc(리뷰 minor)**: `raster` module 문서의 `ExportCapture`·`raster_scope`·`RasterScope::*`·`PixelGrid`
  링크를 전체 경로로 바꿨습니다. `cargo doc -p stellar-export --no-deps` 경고는 lane 커밋 32개에서 26개로
  줄었고, 분기점(24728e3)의 26개와 목록이 같습니다.
- 변경 파일: `crates/export/src/svg/raster.rs`, `crates/export/src/svg/raster/tests.rs`,
  `crates/export/src/svg/raster/tests/painter.rs`, `crates/export/src/svg/raster/tests/reference.rs`(신규), 이
  보고서.
- 검증: `cargo test -p stellar-export` 76 passed / 0 failed(신규 2), `cargo fmt -p stellar-export -- --check`
  통과, `cargo clippy -p stellar-export --all-targets -- -D warnings` 경고 0.
- 남은 한계: 구간 tree는 합집합 상자로만 건너뛰므로 멀리 떨어진 두 곳을 번갈아 오가는 배치처럼 합집합이 넓게
  겹치는 경우에는 한 검색이 여러 tree node를 볼 수 있습니다(결과는 같음). 내보내기 crate에는 아직 취소·작업
  예산 개념이 없어 planner도 받지 않습니다. 구간 tree 메모리는 entry당 최대 약 20 byte입니다.
