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
| 결과 확대 이미지 눈 검사(`target/render-conformance/png-export/`): 둥근 clip·테두리·opacity 그룹·path·이미지·text, shadow·blur·multiply | 의도대로 보임 |

## 차이(작업 지시와 다른 점)

- frame plan graph를 backend command로 그대로 encode하지 않고, frame plan의 draw·효과 값을 공통 그리기 목록으로 낮춰
  fixture lowering이 자기 graph로 실행합니다. 같은 효과 pass 구조(내용 → blur 가로·세로 → 합성)를 쓰며, 제품 frame plan graph는
  surface import(창 drawable)라 offscreen texture에 묶을 수 없기 때문입니다.
- 가장자리 coverage는 MSAA 대신 축 8배 supersampling입니다(CPU target과 비슷한 정밀도).

## 남은 위험 / 한계

- selection 내보내기는 Metal target이 아직 그리지 않습니다(pass compiler가 subtree 시작과 조상 clip 제외를 받아야 함).
- 축 정렬이 아닌 이미지 변환은 미지원 오류입니다(GPU 이미지 quad가 축 정렬 사각형만 받음).
- shadow offset은 supersample device pixel 정수로 반올림합니다(출력 1/16 pixel 이하 오차).
- 둥근 사각형 path 작성(`rounded_rect_path`)이 export CPU target의 비공개 함수와 같은 식으로 따로 있습니다. scene crate 공용 함수로
  합치는 정리가 후속입니다.
- 앱 화면 frame encoder(창 drawable)는 같은 encoder로 붙일 수 있지만 이번에 연결하지 않았습니다(UI 재설계 때).
