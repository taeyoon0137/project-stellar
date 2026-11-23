# 작업 보고서: XX.4.2 래스터 경계를 offscreen PNG로 렌더해 SVG image로 삽입

- WBS: XX.4.2
- 작업명: boundary를 offscreen PNG data로 렌더하고 SVG image element로 삽입
- 상태: 완료 (export 쪽). 실제 pixel 렌더는 host rasterizer가 맡고, renderer offscreen target(XX.2.1)이 아직 없어
  제품 host는 아직 `NoRasterizer`입니다(아래 한계).
- 선행: XX.2.1 (`[ ]`), XX.4.1 (`[x]`).
- 결정: ADR-0091 결정 2·3.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/svg/rasterize.rs`(신규): 렌더 요청 `RasterRequest`·`RasterRoot`, host 연결 지점 `ExportRasterizer`,
  기본값 `NoRasterizer`, 오류 `RasterizeError`, 출력 계약 검사.
- `crates/export/src/svg/writer.rs`: `write_svg_with`(rasterizer 인자), 래스터 경계를 `<image>`로 쓰는 경로, clip chain
  공용화(`clip_chain`), `SvgExportError::Raster`, `SvgDocument::raster_images`. `write_svg`는 `NoRasterizer`로 위임합니다.
- `crates/export/src/svg/element.rs`: `write_raster_image`.
- `crates/export/src/svg/raster.rs`: `PixelGrid::world_rect`(pixel 상자 → world 상자).
- `crates/export/src/preflight.rs`: `PreflightInput::rasterizer`. rasterizer가 있으면 래스터 기능을 형식 차단에서 빼고,
  래스터 경계 안 Text는 glyph outline 진단을 하지 않습니다.
- `crates/export/src/svg/mod.rs`: 모듈 등록과 재노출.
- 테스트: `crates/export/src/svg/writer/tests/raster.rs`(신규), `crates/export/src/preflight/tests.rs`.

## 구현 요약

- **연결 지점**: export는 renderer에 의존하지 않으므로(ADR-0019) host가 `ExportRasterizer`를 구현합니다. 요청은 경계,
  출력 pixel 상자(canvas로 자름), device pixel grid, 그릴 root 목록(paint 순서, 자기 내용만 여부, 건너뛸 clip 깊이),
  canvas 배경입니다. 출력은 요청 크기의 PNG이며 writer가 형식과 크기를 검사합니다.
- **배치**: image는 경계 pixel 상자의 world 상자(`원점 + pixel / scale`)에 `preserveAspectRatio="none"`로 놓습니다. PNG
  pixel 하나가 내보내기 device pixel 하나입니다.
- **영역별 구조**: Content 영역은 node 그룹(clip·opacity)을 그대로 두고 자기 내용 자리에 image를 넣습니다. Subtrees
  영역은 첫 root id의 그룹에 영역 root 공통 clip을 걸고 image 하나를 넣으며, 영역 안 node는 vector로 쓰지 않습니다.
  Canvas 영역은 배경까지 image에 넣고 vector 배경 사각형을 쓰지 않습니다. 영역 밖 조상의 opacity·clip은 SVG 그룹이
  합성합니다.
- **rasterizer 없음**: 사전 진단이 래스터 기능을 형식 차단으로 거부하고, writer도 그 기능에서
  `SvgExportError::Unsupported`로 멈춥니다(기존 동작 유지).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --locked` | 통과 |
| `cargo clippy -p stellar-export --all-targets --locked -- -D warnings` | 경고 0 |

| 검사(테스트) | 결과 |
|---|---|
| 위치·scale: shadow node의 image 상자가 경계 pixel 상자의 world 상자(19, -1, 14×14)이고 PNG 크기가 배율 1·2에서 world 크기 × 배율(`content_regions_replace_only_the_node_content`) | 통과 |
| alpha: rasterizer PNG bytes가 그대로 들어가고, blur 가지 image 위에 부모 opacity 0.5 그룹과 frame clip이 vector로 남음(`subtree_regions_keep_ancestor_effects_vector`) | 통과 |
| blend backdrop 범위(앞 형제까지)와 canvas 배경(배경은 rasterizer가 칠하고 vector 배경 없음)(`blend_backdrops_and_canvas_backgrounds_are_rasterized_together`) | 통과 |
| 계약 위반(크기·형식·실패)은 `Raster` 오류, rasterizer 없으면 `Unsupported`(`raster_contract_violations_are_errors`) | 통과 |
| 같은 snapshot byte-identical(`raster_output_is_byte_identical`) | 통과 |
| 사전 진단: rasterizer가 있으면 shadow 차단 해제, 래스터 경계 안 color glyph Text는 glyph 차단 없음(`rasterizers_lift_raster_blockers_and_skip_rasterized_glyphs`) | 통과 |

## 남은 위험 / 한계

- 제품 host rasterizer가 아직 없습니다. renderer offscreen target(XX.2.1, metal 작업)이 생기면 그 target으로
  `ExportRasterizer`를 구현해 연결합니다. 그 전까지 래스터 기능이 있는 SVG 내보내기는 사전 진단에서 형식 차단입니다.
- 테스트 rasterizer는 단색 PNG로 계약(위치·크기·bytes 전달·요청 내용)만 검사합니다. 실제 합성 pixel 비교는 renderer
  연결 뒤 golden으로 고정해야 합니다.
- 효과와 clip의 적용 순서(XI.10.4)가 고정되면 rasterizer가 건너뛸 clip 깊이 규약을 다시 확인해야 합니다.
