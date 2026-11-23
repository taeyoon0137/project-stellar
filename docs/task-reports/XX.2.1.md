# 작업 보고서: XX.2.1 offscreen target으로 지정 scale의 RGBA image 생성

- WBS: XX.2.1
- 작업명: offscreen RenderGraph target을 구현해 지정 scale의 RGBA image를 생성
- 상태: 부분 완료 (`[~]`) — export 쪽 offscreen target 계약과 CPU target, size·scale fixture는 끝났습니다. 선행 XI.11.1
  (Metal offscreen harness)이 `[ ]`이고(계획 1.2 규칙 4), GPU RenderGraph target은 그 harness가 생긴 뒤 같은 trait으로
  붙입니다.
- 선행: XX.1.2 (`[x]`), XI.11.1 (`[ ]`).
- 결정: ADR-0103 결정 1·2·3·4.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/raster.rs`(신규): `OffscreenTarget`, `OffscreenRequest`, `RgbaImage`, `OutputProfile`, `CancelToken`,
  `OffscreenError`, `UnsupportedFeature`, `check_image`.
- `crates/export/src/raster/software.rs`(신규): CPU target `SoftwareTarget`, `unsupported_features`.
- `crates/export/src/raster/coverage.rs`(신규): polygon coverage 래스터화(non-zero·even-odd, 16 sub-scanline).
- `crates/export/src/raster/path.rs`(신규): f64 affine, path 평탄화, 모서리 둥근 사각형 path.
- `crates/export/src/raster/image.rs`(신규): 이미지 asset texel 해석(출력 색공간 선형광)과 sampling.
- `crates/export/src/raster/tests.rs`(신규): 아래 검사.
- `crates/export/src/png/{mod.rs,zlib.rs,decode.rs,icc.rs}`(신규): 이미지 asset 해석에 쓰는 zlib·PNG decoder와 ICC
  colorant 판정.
- `crates/export/src/lib.rs`, `crates/export/Cargo.toml`, `Cargo.lock`: 모듈 등록, renderer-core 의존.
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`: export → renderer-core, ops → export·scene.
- `docs/adr/ADR-0103-png-export-offscreen-target.md`(신규).

## 구현 요약

- **크기·좌표**: SVG 래스터 경계와 같은 `PixelGrid`(capture 영역 × 배율, 각 변 올림·최소 1)의 canvas 전체를 그립니다.
  world → device는 grid 자신의 pixel → world 정의에서 원점·배율을 읽어 만듭니다(같은 공식을 따로 두지 않음).
- **그리기**: pre-order painter 순서, node 자기 내용(image pixel·text run 색 → paint layer) 뒤 자식입니다. 합성 불투명도
  < 1 node는 행 단위로 필요할 때 만드는 layer에 subtree를 그려 불투명도와 경계 node 바깥 clip으로 합성합니다(renderer pass
  compiler XI.4.2\~XI.4.4 규칙). selection은 조상 clip·합성 효과를 그리지 않습니다.
- **재사용**: stroke는 renderer `tessellate_stroke`(Inside는 geometry coverage를 곱하고 Outside는 여집합을 곱함), 색은
  renderer `OutputColorPlan::convert`, 이미지 배치는 scene `ImageFit::place`(crop 영역이 원본 크기), Text는 SVG 내보내기와
  같은 `text_outlines`입니다.
- **미지원**: shadow·layer blur·Normal이 아닌 blend는 그리기 전에 `Unsupported`(근사 없음). PNG가 아닌 asset, interlace
  PNG, sRGB·P3가 아닌 내장 profile, outline 없는 glyph도 `Unsupported`입니다. 확인하지 않은 missing image·text layout은
  오류, 확인한 것은 생략합니다.
- 합성은 선형광 연관 알파 f32이고 마지막에 비연관으로 되돌려 transfer encode 후 8-bit 반올림합니다. 알파 0 pixel의 RGB는 0입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --offline --lib raster` | 통과 |
| `cargo clippy -p stellar-export -p stellar-ops -p xtask --all-targets --offline -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 통과 |

| 검사(테스트) | 결과 |
|---|---|
| size/scale: basic fixture(영역 100×80)가 배율 1·2·0.5·0.25에서 100×80·200×160·50×40·25×20, 3×1 영역 배율 1.5는 5×2(`size_follows_export_bounds_and_scale`) | 통과 |
| pixel 경계에 맞은 불투명 사각형은 모든 pixel이 정확히 (255,0,0,255)(`opaque_pixel_aligned_rect_is_exact`) | 통과 |
| 0.5 pixel 가장자리는 알파 128, 흰 배경 위는 선형광 반 섞임 188(`fractional_edges_are_antialiased_and_background_is_painted`) | 통과 |
| opacity 0.5 그룹의 겹친 두 사각형도 알파 128(layer 합성)(`group_opacity_composites_subtree_once`) | 통과 |
| 둥근 clip Frame, Inside 테두리(`frame_clip_and_inside_border`), Inside·Outside stroke 정렬(`stroke_alignment_inside_and_outside`) | 통과 |
| even-odd·non-zero 차이와 중심 stroke(`path_fill_rules_and_center_stroke`) | 통과 |
| 이미지 Fill·Contain 배치, Nearest·Linear sampling, JPEG 미지원 오류(`images_decode_place_and_sample`) | 통과 |
| 확인 없는 missing image는 오류, 사전 진단 확인 뒤에는 생략(`missing_image_requires_confirmation_to_omit`) | 통과 |
| text glyph outline·run 색, outline 없는 glyph·layout 없음 오류(`text_glyph_outlines_are_filled_with_run_color`) | 통과 |
| shadow·blur·blend는 `Unsupported`(`unrendered_effects_are_errors_not_approximations`) | 통과 |
| selection은 조상 clip·opacity 무시(`selection_ignores_ancestor_clip_and_opacity`) | 통과 |
| Display P3 출력 색 변환, sRGB 출력의 P3 색 자르기(`display_p3_output_converts_colors`) | 통과 |
| 결정론·취소(`rendering_is_deterministic_and_cancellable`), 크기 상한(`oversized_output_is_rejected_before_rendering`), 계약 검사(`check_image_rejects_contract_violations`) | 통과 |
| zlib(참조 zlib 동적 Huffman stream 해제 포함)·ICC colorant(Apple Display P3 공개 값과 0.001 이내) 단위 test | 통과 |

## 차이(작업 지시와 다른 점)

- 지시는 "offscreen RenderGraph target"입니다. renderer에 frame plan을 draw command로 낮추는 경로와 Metal offscreen
  harness(XI.11.1)가 아직 없고, headless 도구는 GPU backend에 닿을 수 없어(ADR-0005 §8) 계약(trait)과 CPU target을
  먼저 만들었습니다(ADR-0103 결정 2). GPU target은 같은 trait으로 붙입니다.

## 남은 위험 / 한계

- shadow·layer blur·blend 화면은 PNG 내보내기가 미지원 오류입니다(XI.10.2·XI.10.4 공식 고정 뒤 확장).
- GPU target과의 pixel 일치는 아직 비교하지 못했습니다(XI.11.1 뒤 XX.2.4 golden으로 비교).
- SVG 래스터 경계 host(`ExportRasterizer`)로 CPU target을 연결하지 않았습니다. 그 경계 기능이 바로 CPU target 미지원
  효과라 연결해도 풀리는 기능이 거의 없습니다(이미지 색공간 override만 해당).
