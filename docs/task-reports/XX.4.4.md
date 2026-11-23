# 작업 보고서: XX.4.4 한 unsupported effect가 View 전체를 래스터화하지 않는 회귀 테스트

- WBS: XX.4.4
- 작업명: 한 unsupported effect가 전체 View를 불필요하게 rasterize하지 않는 회귀 테스트 추가
- 상태: 부분 완료 (`[~]`) — 2026-11-23 lane 검토 판정: 회귀 test는 끝났지만 선행 XX.4.2가 `[~]`입니다(계획 1.2 규칙 4). 투명
  배경 경우만 검사하며, 불투명 배경 위 blend 하나가 canvas 전체를 래스터화하는 XX.4.1 planner 규칙은 남은 범위입니다(아래
  "검토 추가"). 보고 당시 기록: 완료 (`[x]`).
- 선행: XX.4.2 (`[~]`, 위 판정).
- 결정: ADR-0091 결정 2.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/svg/writer/tests/regression.rs`(신규): 회귀 테스트 3개.
- `crates/export/src/svg/writer/tests.rs`: 테스트 모듈 등록.

## 구현 요약

검증 기준은 "vector node count 유지"입니다. planner 계획(`RasterPlan::vector_nodes`)과 실제 SVG 출력(vector `<rect>` 수,
`SvgDocument::raster_images`) 두 쪽을 함께 셉니다.

- `one_unsupported_effect_does_not_rasterize_the_whole_view`: 사각형 60개 View에서 하나만 layer blur → 래스터 node 1개,
  vector node 59개, SVG vector 사각형 59개와 image 1개, node 그룹 60개. 같은 View에 effect가 없으면 사각형 60개 모두
  vector(기준선).
- `nested_effects_rasterize_only_the_affected_branch`: frame > group > [blur 잎, 잎], 형제 잎, 형제 root 구조에서 blur 잎만
  래스터화되고 조상 frame·group과 다른 node 5개는 vector입니다.
- `each_raster_feature_keeps_the_rest_of_the_view_vector`: 사각형 40개 View에서 drop shadow(Content), layer blur(Subtree),
  겹치는 앞 내용이 없는 blend(Backdrop)를 각각 한 node에만 걸면 vector node 39개가 유지됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --locked regression` | 3 passed |
| `cargo test -p stellar-export --locked` | 통과 |

## 차이(작업 지시와 다른 점)

- 테스트 rasterizer(단색 PNG)를 써서 pixel 대신 구조와 node 수로 검증합니다.

## 남은 위험 / 한계

- 테스트 rasterizer(단색 PNG)로 구조와 개수를 봅니다. 실제 pixel 비교는 renderer offscreen target 연결 뒤에 더합니다.

## 검토 추가(2026-11-23)

- 불투명 배경(`ExportBackground::Solid`, alpha > 0)에서 Normal이 아닌 blend node가 배경과 겹치면 XX.4.1 planner가 canvas
  전체를 영역 하나(`RasterRegion::Canvas`)로 래스터화합니다. 그 blend와 겹치지 않는 뒤 node도 image에 들어가 vector
  node 수가 0이 됩니다(`blend_backdrops_and_canvas_backgrounds_are_rasterized_together`가 이 동작을 고정). backdrop은 blend
  footprint 안 배경과 앞선 겹치는 내용뿐이라 영역을 그 범위로 줄이는 planner 변경(XX.4.1 규칙과 writer 배경 처리)이 후속으로
  필요합니다. 이 행의 회귀 test는 투명 배경만 검사합니다.
