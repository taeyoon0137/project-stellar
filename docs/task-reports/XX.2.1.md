# 작업 보고서: XX.2.1 offscreen target으로 지정 scale의 RGBA image 생성

- WBS: XX.2.1
- 작업명: offscreen RenderGraph target을 구현해 지정 scale의 RGBA image를 생성
- 상태: 구현 완료 (`[x]` 후보, marker는 통합 때 정합니다) — 이전 작업의 offscreen target 계약·CPU target·size·scale
  fixture에 더해, 남은 범위였던 GPU(Metal) RenderGraph target을 같은 trait으로 구현했습니다. 경로는 pass compiler frame
  plan(효과 pass 포함) → scene frame encoder → 공통 lowering → headless Metal입니다.
- 선행: XX.1.2 (`[x]`), XI.11.1 (`[x]`).
- 결정: ADR-0103 결정 1·2·3·4, ADR-0116 결정 3·4.

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, headless Metal).

## 변경 파일 목록

이전 작업(CPU target): `crates/export/src/raster.rs`, `crates/export/src/raster/{software,coverage,path,image,tests}.rs`,
`crates/export/src/png/{mod,zlib,decode,icc}.rs`, `docs/adr/ADR-0103-png-export-offscreen-target.md`.

이 lane(GPU target):

- `crates/renderer-core/src/pass_compiler/effects.rs`(신규), `crates/renderer-core/src/pass_compiler.rs`: 효과 pass와 합성
  program(ADR-0116 결정 3).
- `crates/renderer-core/src/effects/scene.rs`(신규, `encode_plan`, `FrameResources`, `GlyphShape`, `rounded_rect_path`,
  `transform_path`), `crates/renderer-core/src/effects/scene/tests.rs`(신규).
- `crates/renderer-core/src/effects/lower.rs`(`lower_ops`, `OpsTarget`), `crates/renderer-core/src/effects/{fixture,raster,mod}.rs`
  (shadow·layer blur 축별 σ).
- `crates/renderer-metal/src/offscreen_target.rs`(신규, `MetalOffscreenTarget`), `crates/renderer-metal/src/lib.rs`,
  `crates/renderer-metal/Cargo.toml`, `crates/renderer-metal/src/tests/{mod,offscreen_target}.rs`.
- `crates/scene/src/lib.rs`(StableId·Rect·Transform2D 재노출).
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`(renderer-metal → export·scene).
- `docs/adr/ADR-0116-render-effect-passes-and-png-export-paths.md`.

## 구현 요약

- **frame encoder**: frame plan의 main draw를 painter 순서로, layer 합성 draw를 layer pass draw를 담은 `LayerOp`로 낮춥니다.
  draw 효과(shadow device offset·σ, layer blur σ, 합성 mode, 접힌 불투명도)는 frame plan 값을 쓰고, clip은 scissor + 복합
  clip마다 stencil layer입니다. 내용은 Rect(둥근 사각형 path)·Path·Image(축 정렬 quad)·Text(glyph 모양) → paint layer
  순서입니다. stroke `Inside`·`Outside`는 geometry 안·밖 stencil로 자릅니다.
- **Metal target**: capture snapshot을 tile(128 pixel, 효과 여백)마다 축 8배 supersample viewport로 compile → encode → lowering
  → headless Metal `Rgba16Float` → readback → 상자 평균 → 비연관 → transfer encode → 8-bit. 효과 근사 진단·selection은 실패로
  돌려줍니다(근사하지 않음). 이미지는 PNG(색 정보 판정 또는 override), text는 고정 font revision glyph outline입니다.
- **supersampling 배수(검토 보완)**: 효과 σ는 supersample device 단위라 8배면 출력 pixel σ 8(box-shadow blur 16)만 넘어도
  σ 상한(`MAX_SIGMA` 64) 진단으로 실패했습니다. 이제 출력 pixel 단위 최대 σ × 배수가 상한을 넘지 않는 가장 큰 배수(1\~8)를
  씁니다(`supersample_for`). 배수 1에서도 넘으면 전처럼 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib` | 224 passed |
| `cargo test -p stellar-renderer-metal` | 65 passed(golden 대조 2개는 XX.2.4 커밋) |
| `cargo clippy -p stellar-renderer-metal -p stellar-renderer-core -p xtask --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 통과 |

| 검사(테스트) | 결과 |
|---|---|
| encoder → CPU 기준: painter 순서·pixel 정렬 사각형 정확, 겹친 opacity 그룹 한 번 합성(알파 128), multiply layer가 backdrop을 읽음, shadow가 내용 아래 device offset, 둥근 clip(stencil·mask plan 모두) 밖 투명(`effects::scene::tests`) | 통과 |
| Metal target이 CPU target이 그리지 않는 효과(shadow·layer blur·multiply)를 같은 그리기 목록의 CPU 기준과 연관 값 ±3 안에서 그림, CPU target은 같은 요청을 `Unsupported`로 거부(`metal_target_draws_effects_like_the_cpu_reference`) | 통과 |
| selection 내보내기는 실패, 취소는 `Cancelled`(`selection_and_cancel_are_reported`) | 통과 |
| blur 40 그림자·반경 12 layer blur 장면은 배수 3으로 그리고 같은 배수의 CPU 기준과 연관 값 ±4 안, 그림자가 카드 밖으로 번짐, 배율 4로 출력 σ 80이면 실패(`large_blur_lowers_supersampling_instead_of_failing`) | 통과 |
| 결과 확대 이미지 눈 검사(`target/render-conformance/png-export/`): 둥근 clip·테두리·opacity 그룹·path·이미지·text, shadow·blur·multiply | 의도대로 보임 |

## 차이(작업 지시와 다른 점)

- frame plan graph를 backend command로 그대로 encode하지 않고, frame plan의 draw·효과 값을 공통 그리기 목록으로 낮춰
  fixture lowering이 자기 graph로 실행합니다. 같은 효과 pass 구조(내용 → blur 가로·세로 → 합성)를 쓰며, 제품 frame plan graph는
  surface import(창 drawable)라 offscreen texture에 묶을 수 없기 때문입니다.
- 가장자리 coverage는 MSAA 대신 축 8배 supersampling입니다(CPU target과 비슷한 정밀도).

## 남은 위험 / 한계

- selection 내보내기는 Metal target이 아직 그리지 않습니다(pass compiler가 subtree 시작과 조상 clip 제외를 받아야 함).
- 축 정렬이 아닌 이미지 변환은 미지원 오류입니다(GPU 이미지 quad가 축 정렬 사각형만 받음).
- (해결, ADR-0130 결정 3) shadow offset은 supersample device pixel 정수로 반올림합니다(배수 8이면 출력 1/16 pixel 이하, 큰 blur로 배수를 낮추면
  최대 1/2 pixel 오차).
- (해결, ADR-0130 결정 2·4) 큰 blur로 배수를 낮추면 그 내보내기 전체의 가장자리 coverage 단계가 거칠어집니다(배수 3이면 9단계). 효과만 낮은 해상도로
  그리는 분리는 후속입니다.
- 둥근 사각형 path 작성(`rounded_rect_path`)이 export CPU target의 비공개 함수와 같은 식으로 따로 있습니다. scene crate 공용 함수로
  합치는 정리가 후속입니다.
- 앱 화면 frame encoder(창 drawable)는 같은 encoder로 붙일 수 있지만 이번에 연결하지 않았습니다(UI 재설계 때).

## 후속: supersampling 배수를 blur σ와 뗌 (lane render-shadow-quality, ADR-0130 결정 2·3·4)

- 위 "남은 위험"의 두 항목을 해결했습니다. shadow offset은 반올림하지 않고 소수 device pixel로 합성하고, 큰 blur는 효과만
  낮은 해상도로 그리므로 supersampling 배수는 8입니다. 배수는 효과 여백을 더한 tile texture 한 변이 4096
  (`MAX_TILE_TEXTURE`)을 넘을 때만 낮춥니다(`supersample_for(margin, tile, max_dimension)`, `tile_margin`). 출력 pixel σ가 64를
  넘어도 그립니다.
- Frame 그림자는 CSS `box-shadow`처럼 상자 모양이고 상자 안을 비웁니다(XI.10.4 후속 절).
- 변경 파일: `crates/renderer-metal/src/offscreen_target.rs`, `crates/renderer-metal/src/tests/offscreen_target.rs`(renderer-core
  변경은 XI.10.4 후속 절).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 68 passed, 실패 0 |

| 검사(테스트) | 결과 |
|---|---|
| blur 40 그림자·반경 12 layer blur 장면이 배수 8(이전 3)이고, 같은 배수의 CPU 기준과 연관 값 ±4 안, 그림자가 카드 밖으로 번짐(`large_blur_keeps_supersampling_and_draws_effects_at_low_resolution`) | 통과 |
| 같은 장면 배율 4(출력 σ 80, 이전에는 실패): 배수 2\~7로 그리고 카드 오른쪽 아래 그림자가 먼 곳보다 어두움 | 통과 |
| 여백이 배수 1로도 tile texture 한 변 상한을 넘으면(blur 1300) `texture 상한` 실패 | 통과 |
| 효과 장면에 배경 없는 Frame의 상자 그림자(둥근 모서리, 소수 offset)를 더해 CPU 기준과 ±3(`metal_target_draws_effects_like_the_cpu_reference`), 확대 이미지에서 상자 안이 비어 있음 | 통과 |

- 남은 한계: 여백이 출력 128 pixel을 넘는 큰 효과는 메모리 상한 때문에 배수가 8보다 낮아집니다(효과가 아닌 tile 크기 이유).
  margin 영역까지 supersample하지 않는 분리(효과 원본만 낮은 해상도 viewport로 그리기)는 후속입니다.

## 후속: 효과 여백을 target 밖 효과 window에 그림, tile 메모리 예산 (lane render-effects-3, ADR-0136)

- 위 후속 절의 남은 한계(여백이 출력 128 pixel을 넘으면 배수가 8보다 낮아짐, 여백 약 1984 pixel을 넘으면 실패, offset이 여백에
  그대로 들어가 offset 300 그림자 하나로 배수 5)를 해결했습니다.
- tile target은 출력 영역만 덮습니다(한 변 `tile × 배수`, 최대 1024). 여백은 compile viewport에만 들어가 여백의 내용이 cull되지
  않고, 효과 원본은 `effects::window` 계획의 window에 따로 그립니다. shadow 원본 window는 offset만큼 옮긴 자리라 offset이 커도
  커지지 않고, 큰 σ 효과 원본은 blur 해상도의 절반으로 그립니다(ADR-0136 결정 1·2).
- 배수는 tile마다 8부터 그리고 device σ 상한, 효과 window 상한(4096), 잰 GPU 메모리가 예산을 넘을 때만 그 tile의 배수를 낮춥니다
  (결정 3). `supersample_for`는 없앴습니다.
- tile GPU 메모리를 잽니다: lowering이 만든 texture·buffer(backend 할당 증가분) + graph transient(alias slot마다 Metal heap 크기,
  `MetalBackend::graph_transient_bytes`). 예산(`MetalOffscreenTarget::with_memory_budget`, 기본 device
  `recommendedMaxWorkingSetSize`)의 남은 양을 넘으면 encode 전에 배수를 낮춥니다. `render_measured`가 측정을 돌려줍니다(결정 4).
- 변경 파일: `crates/renderer-metal/src/offscreen_target.rs`, `crates/renderer-metal/src/backend.rs`,
  `crates/renderer-metal/src/tests/offscreen_target.rs`(renderer-core 변경은 XI.10.4 후속 절),
  `docs/adr/ADR-0136-render-effect-windows-and-export-memory-budget.md`.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 72 passed, 실패 0 |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets --locked -- -D warnings` | 경고 0 |

| 검사(테스트) | 결과 |
|---|---|
| blur 40 그림자·반경 12 layer blur 장면 배율 1·4 모두 배수 8(ADR-0130까지 배율 4는 2\~7), 배율 1은 CPU 기준과 연관 값 ±4, 배율 1·4가 같은 자리에서 ±6(`large_blur_keeps_supersampling_and_draws_effects_at_low_resolution`) | 통과 |
| blur 1300(σ 650, ADR-0130에서 실패)은 σ 상한 때문에 배수 6으로 그리고 상자 밖 그림자 알파 50\~80, 상자 안 0. blur 16386은 배수 1에서도 σ 상한이라 `blur_too_large` 실패(`huge_blur_lowers_supersampling_only_for_the_sigma_limit`) | 통과 |
| offset (300, 3) 그림자: 여백 306이라 ADR-0130 규칙이면 배수 5, 이제 배수 8이고 CPU 기준과 연관 값 ±3, 그림자는 300 pixel 오른쪽(`large_offset_shadow_keeps_supersampling`) | 통과 |
| 메모리: 배수 8 tile 최대 측정값보다 작은 예산(60%)이면 그 tile만 배수를 낮추고(최소 5) 측정값이 예산 안, 배수 8 결과와 연관 값 ±24 안. 예산 1 KiB면 "메모리 예산" 실패(`memory_budget_lowers_supersampling_when_a_tile_does_not_fit`) | 통과 |
| 효과 장면 CPU 기준 ±3(`metal_target_draws_effects_like_the_cpu_reference`), 적합성 fixture 전체(`full_inventory_matches_cpu_reference_on_metal`) | 통과 |

- 측정(개발 장비 Apple GPU, 배수 8, 예산 = `recommendedMaxWorkingSetSize` 약 28 GiB): 큰 blur 장면 배율 1 tile 최대 약 39 MiB(2
  tile), 배율 4 약 39.4 MiB(20 tile), 큰 offset 장면 약 3.8 MiB(3 tile), blur 1300(배수 6) 약 8.2 MiB. texture 한 변 최대는 모두
  1024(target) 이하입니다. 기준 장비(M1 16GB)에서는 다시 재지 않았습니다.
- 남은 한계: 제품 frame plan(pass compiler) graph의 효과 pass는 여전히 surface 크기입니다(창 drawable 경로 연결 때, ADR-0136
  Alternatives). tile마다 transient pool을 비우고 다시 만듭니다.
