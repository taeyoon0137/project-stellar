# ADR-0136: 효과 window 분리와 내보내기 메모리 예산에 따른 supersampling 배수

## Status

Accepted (2026-11-24).

- ADR-0130 결정 4("Metal 내보내기 supersampling 배수는 효과 여백을 더한 tile texture 크기로 정함")를 결정 3·4로 바꿉니다.
- ADR-0130 결정 1·2·3(상자 shadow, 큰 σ 낮은 해상도, 소수 offset)은 그대로이고, 그 pass들이 그리는 영역과 해상도를 결정 1·2가
  정합니다.

## Context

- 그리기 목록(`effects::fixture::Op`)의 모든 pass(layer 내용, shadow 원본·blur, layer blur, mask)는 target과 같은 크기였습니다.
  Metal 내보내기 tile target은 효과 여백(shadow `|offset| + 3σ`, layer blur `3σ`의 경로 합)을 더한 크기라 여백이 출력 128 pixel을
  넘으면 tile texture 한 변 상한(4096)을 지키려고 supersampling 배수를 8보다 낮췄고, 여백이 출력 약 1984 pixel을 넘으면 배수 1로도
  넘어 실패했습니다.
- 여백에 `|offset|`이 그대로 들어가 offset 300인 그림자 하나로 배수가 5로 내려갔습니다(blur가 작아도). 여백 안 내용은 출력에
  보이지 않고 효과 원본으로만 쓰이는데, 출력과 같은 해상도(배수 8)로 모두 그렸습니다.
- 배수 8 tile이 GPU 메모리를 얼마나 쓰는지 재지 않았고, backend 메모리 예산(XI.8.7, `recommendedMaxWorkingSetSize`)을 넘으면
  transient 할당에서 실패했습니다.

## Decision

1. **pass마다 window.** pass 하나는 `Window`(device 원점, texel 크기, 해상도 `step` = texel 하나의 device pixel 수, 2의
   거듭제곱)에 그립니다. 그리기 목록 좌표는 device pixel 그대로이고, pass에 기록할 때 `(p − 원점) / step`으로 옮깁니다. 계획은
   pure 함수 `effects::window::plan_layer` 하나이고 CPU 기준(`effects::raster`)과 GPU lowering(`effects::lower`)이 같이 씁니다.
   - 바깥 target은 출력 영역만 덮습니다(`OpsTarget::origin`, 내보내기는 tile 출력 영역). 그리기 목록이 뜻을 갖는 device 범위는
     `EffectSpace::canvas`(fixture target, tile compile viewport)이고 효과 window는 그 범위를 넘지 않습니다.
   - 효과 없는 layer: 내용·합성 결과가 부모 window 그대로입니다.
   - layer blur: 내용과 shadow를 (부모 window ⊕ 번짐) ∩ canvas ∩ (내용·shadow 범위 ⊕ 번짐)에 쌓고, blur 해상도에서 blur한 뒤 부모
     window에 bilinear로 되돌려 그립니다. 번짐 `reach(σ) = REACH_PER_SIGMA·σ + 1`(ADR-0130 결정 4의 상한 + bilinear 한 pixel).
   - shadow: 원본 window는 (쌓는 window − offset) ⊕ 번짐 ∩ canvas ∩ (원본 범위 ⊕ 번짐)입니다. offset만큼 **옮긴** 자리라 offset이
     커도 커지지 않습니다. 원본이 내용이고 해상도가 같고 쌓는 window 안이면 내용 pass를 그대로 쓰고, 아니면 내용을 원본 window에
     따로 그립니다. 상자 원본은 원본 window에 그리고 비우기 stencil은 쌓는 window 좌표입니다.
   - 범위는 그리기 목록에서 계산합니다(`ops_bounds`, `layer_bounds`: mesh·이미지·glyph 상자, shadow 번짐, blur 번짐, clip·mask로
     자름). 부모 window에 보이는 것이 없는 layer와 shadow는 그리지 않습니다.
2. **효과 원본 해상도는 blur 해상도의 절반.** 낮은 해상도 배수 `e`(ADR-0130 결정 2, `effect_downsample`)가 2 이상이면 원본·쌓기
   window 해상도는 `max(부모 step, e / 2)`이고, 그 window에서 `max(e, step)`까지 2×2 상자 평균으로 줄여 σ / `max(e, step)`로
   blur합니다. window 원점은 `max(e, step)`의 배수(device 0 기준)라 낮은 해상도 격자가 window와 무관합니다.
   - 정밀도: `e ≥ 2`이면 device σ / e ≥ 32이고, `e / 2` 해상도 pixel 중심 표본의 가장자리 위치 오차는 `e / 4` device pixel
     이하라 blur 결과 차이가 약 `0.4 · 0.25 / 32` = 0.3%입니다. CPU 기준에서 σ 300을 device 해상도로 직접 blur한 값과 8-bit 한
     단계 미만으로 다릅니다(test).
   - window 한 변이 `EffectSpace::max_side`(내보내기 4096 = `MAX_TILE_TEXTURE`, fixture 16384)를 넘으면 해상도를 2배씩 낮추고,
     blur 해상도 `max(e, 부모 step)`보다 거칠어져야 하면 `WindowError::TooLarge`입니다(근사하지 않음).
3. **내보내기 배수는 tile마다 8부터.** Metal 내보내기는 tile마다 배수 8로 그리고, 다음 셋 중 하나면 그 tile만 배수를 하나씩
   낮춰 다시 그립니다. 배수 1로도 안 되면 실패합니다.
   - device σ가 `MAX_DEVICE_SIGMA`(4096)를 넘음(`render.effect.blur_too_large`, 배수에 비례). 기울임 blur 진단은 배수와 무관해
     바로 실패입니다.
   - 효과 window가 blur 해상도로도 `MAX_TILE_TEXTURE`를 넘음.
   - 잰 GPU 메모리가 backend 메모리 예산의 남은 양을 넘음(결정 4).
   - tile compile viewport 한 변이 device 2^20을 넘음(f32 좌표가 device 1/8 pixel보다 거칠어짐).
   - 여백은 compile viewport에만 들어갑니다(`tile_margin`, 여백 내용이 cull되지 않게). target은 한 변 `tile × 배수`(최대 1024)
     입니다. ADR-0130의 `supersample_for(margin, tile, max_dimension)`은 없앱니다.
4. **tile GPU 메모리 측정.** tile마다 남은 transient를 놓고(`purge_transients`), lowering이 만든 target·upload texture·buffer의
   backend 할당 증가분과 graph transient를 alias slot마다 Metal heap 크기(`MetalBackend::graph_transient_bytes`,
   `heapTextureSizeAndAlignWithDescriptor:`)로 더해 잽니다. 예산은 `MetalOffscreenTarget::with_memory_budget`(없으면 device
   `recommendedMaxWorkingSetSize`)이고, 잰 값이 예산 − 현재 할당을 넘으면 encode 전에 배수를 낮춥니다. encode 중
   `OutOfDeviceMemory`도 같게 다룹니다. `MetalOffscreenTarget::render_measured`가 측정(`ExportMemoryReport`: tile 수, 배수
   최소·최대, tile 최대 byte, texture 한 변 최대, 예산)을 돌려줍니다.

## Alternatives

- 여백을 target에 두고 여백 영역만 낮은 해상도로 그리기(한 texture 안 두 해상도): rasterizer가 영역마다 해상도를 바꿀 수 없어
  pass를 나눠야 하고, 효과마다 window를 두는 것과 같은 일이 됩니다. 탈락했습니다.
- 여백 내용을 출력과 같은 해상도로 그리되 shadow window만 offset만큼 옮기기: 큰 offset 퇴행은 풀지만 큰 σ의 여백(배율 4의 blur 40
  이면 출력 250 pixel)은 여전히 배수 8로 4096을 넘어 탈락했습니다.
- 효과 원본을 blur 해상도 `e`로 바로 그리기: window가 가장 작지만 pixel 중심 표본 오차가 `e / 2`로 커져 8-bit 한 단계를 넘을 수
  있어 탈락했습니다(`e / 2` + 상자 평균 한 단계).
- 내보내기 전체에 한 배수: tile 하나의 큰 효과·메모리 때문에 모든 tile의 가장자리 품질이 떨어져 탈락했습니다. tile마다 배수가
  달라도 같은 tile은 같은 결과(결정론)이고, 배수 차이는 가장자리 coverage 단계뿐입니다.
- 메모리를 lowering 전에 추정하기: Metal heap 크기·alias가 backend마다 달라, lowering 뒤 backend가 잰 값을 씁니다. lowering이 만든
  resource는 예산을 넘으면 encode 전에 놓습니다.
- 제품 frame plan(pass compiler) graph의 효과 pass를 같은 window로 줄이기: 창 drawable 경로가 아직 연결되지 않아(UI 재설계 때)
  이번에 하지 않았습니다. pass compiler는 바꾸지 않습니다.

## Consequences

- 효과 여백이 커도 내보내기 배수는 8입니다. 배율 4의 blur 40(출력 σ 80, ADR-0130에서 배수 2\~7), offset 300 그림자(ADR-0130에서
  배수 5)가 배수 8이고, blur 1300(σ 650, ADR-0130에서 실패)은 σ 상한 때문에 배수 6으로 그립니다.
- tile target은 한 변 1024 texel 이하(`Rgba16Float` 8 MiB)이고 효과 window는 내용 범위로 잘려 작습니다. 측정(개발 장비, 배수 8):
  큰 blur 장면 tile 최대 약 39 MiB, 큰 offset 장면 약 3.8 MiB, blur 1300(배수 6) 약 8.2 MiB.
- shadow 원본이 내용이고 쌓는 window 밖이 필요하면 내용을 한 번 더 그립니다(그 window에서). Frame 그림자는 상자 원본이라 내용을 다시
  그리지 않습니다.
- CPU 기준과 GPU는 같은 window 계획을 쓰므로 적합성 fixture 비교는 그대로입니다. 전체 target 하나에 그리던 이전 결과와는 `e ≥ 4`
  효과만 값이 다릅니다(원본 해상도 `e / 2`, 8-bit 한 단계 미만).

## Compatibility

- 파일 포맷·FFI 변경 없음.
- 공개 API 변경(renderer-core): `effects::window`(신규: `Bounds`, `Window`, `EffectSpace`, `WindowError`, `reach`, `ShadowPlan`,
  `BlurPlan`, `LayerPlan`, `ops_bounds`, `layer_bounds`, `plan_layer`), `effects::lower::OpsTarget::{origin, space}`,
  `effects::fixture::FixtureError::Window`, `effects::raster::draw_ops`가 `Result`를 돌려줌과 `draw_ops_in`(신규).
  renderer-metal: `supersample_for` 삭제, `MetalOffscreenTarget::{with_memory_budget, render_measured}`, `ExportMemoryReport`,
  `MetalBackend::graph_transient_bytes`(신규). 사용처는 renderer crate 안뿐입니다.

## Performance

- 내보내기 tile target이 여백만큼 작아지고(한 변 `tile × 8`), 큰 σ 효과의 원본은 `e / 2` 해상도라 GPU pixel 수가 줄어듭니다.
- shadow 원본을 따로 그리는 경우 내용 draw가 한 번 더 듭니다. window 계획은 layer마다 그리기 목록 범위를 한 번 훑습니다(CPU).
- tile마다 transient pool을 비우고 다시 만듭니다(내보내기 전용, frame 예산 §6.2 밖). 측정은 descriptor마다 Metal heap 크기 조회
  하나입니다.

## Security

- 효과 window 한 변은 `max_side`로, 내보내기 compile viewport는 device 2^20으로, GPU 메모리는 backend 예산으로 묶입니다. 신뢰되지
  않은 큰 σ·offset은 배수를 낮추거나 실패할 뿐 상한을 넘는 할당을 만들지 않습니다.

## Migration

- 없음. 저장 문서·fixture byte는 바뀌지 않습니다.
