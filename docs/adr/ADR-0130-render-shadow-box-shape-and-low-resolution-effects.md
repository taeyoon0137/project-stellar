# ADR-0130: Frame shadow의 상자 모양 원본, 효과만 낮은 해상도로 그리기, 소수 shadow offset

## Status

Accepted (2026-11-24).

- ADR-0116 결정 3의 "shadow 원본을 상자 모양으로 바꾸는 후속"을 결정 1로 채웁니다.
- ADR-0116 결정 3의 σ 상한(`MAX_SIGMA` 64를 넘으면 상한으로 그리고 진단)을 결정 2로 바꿉니다.
- ADR-0116 결정 4의 "shadow offset은 device pixel 정수로 반올림"과 "출력 pixel 최대 σ × 배수가 `MAX_SIGMA`를 넘으면
  supersampling 배수를 낮춤"을 결정 3·4로 바꿉니다.

## Context

- scene shadow는 node 자기 내용의 alpha에서 만든 drop shadow였습니다(ADR-0116 결정 3). Frame은 CSS로 `box-shadow`인데
  (codegen `modifier_plan`: Frame Shadow는 앞에 filter 계열이 없으면 `box-shadow`), `box-shadow`는 배경과 관계없이 테두리
  상자 모양으로 그리고 상자 안을 비웁니다. 그래서 배경이 없거나 반투명한 Frame의 그림자가 실제 구현(브라우저)과 달랐습니다.
  "실제 구현에 가까운 디자인"이라는 핵심 가치와 맞지 않습니다.
- 한 blur pass의 σ 상한은 64 pixel(kernel 탭 수 상한)입니다. Metal 내보내기는 가장자리 coverage를 축 8배 supersampling으로
  만들어 효과 σ도 8배가 됩니다. 그래서 출력 pixel σ가 8(box-shadow blur 16)만 넘어도 supersampling 배수를 그 내보내기
  전체에서 낮췄고(blur 40이면 배수 3, 가장자리 9단계), 출력 pixel σ가 64를 넘으면 배수 1로도 그리지 못해 실패했습니다.
- shadow offset은 device pixel 정수로 반올림했습니다. 배수를 낮추면 출력 pixel 기준 최대 1/2 pixel 어긋났습니다.

## Decision

1. **shadow 원본 모양은 CSS 대응을 따릅니다.** pass compiler가 node 내용으로 [`ShadowShape`]을 정합니다.
   - `SceneContent::Rect`(Frame)는 `ShadowShape::BorderBox`(CSS `box-shadow`)입니다. 원본은 테두리 상자(모서리 반경
     포함)를 불투명하게 칠한 모양이고, 합성할 때 상자 안(offset 전 자리, pixel 중심 판정)은 비웁니다. 배경이 없거나
     반투명해도 상자 모양이고 상자 안에는 비치지 않습니다. 배경이 불투명하면 이전과 결과가 같습니다.
   - 그 밖의 내용(Path·Text·Image)은 `ShadowShape::ContentAlpha`(CSS `filter: drop-shadow()`)로 이전과 같습니다. codegen도
     Frame 밖은 `drop-shadow()`입니다.
   - pass compiler: `EffectStep::ShadowSource { node, shape }`, `ShadowComposite::shape`. encoder: 상자 모양은 Frame 사각형
     path의 device fill mesh입니다. fixture 모델: `ShadowSpec::source`(`ShadowSource::Content` | `ShadowSource::Box(mesh)`).
   - CPU 기준(`effects::raster`)은 mesh를 불투명하게 칠한 canvas를 원본으로 쓰고 mesh 안 pixel(중심 판정)을 건너뜁니다.
     GPU lowering은 상자 원본 pass(`SolidFill`)를 그리고, 쌓는 pass에서 상자를 stencil로 표시한 뒤
     (`ClipStencil` + `IncrementWhereEqual`) stencil 0인 곳(`TestEqual` reference 0)에만 shadow를 그리고 되돌립니다.
2. **σ가 크면 효과만 낮은 해상도로 그립니다.** device σ가 `MAX_SIGMA`(64)를 넘으면 σ / d ≤ 64인 가장 작은 2의 거듭제곱
   d(`effects::blur::effect_downsample`, 상한 `MAX_DOWNSAMPLE` 64)로 blur 입력을 줄이고, 줄인 texture에서 σ / d로 분리
   blur한 뒤 bilinear로 키웁니다. 내용의 가장자리 coverage 해상도는 그대로입니다.
   - 줄이기: 한 단계마다 크기 `ceil(n / 2)`, `LayerComposite` Linear 표본 하나(2×2 상자 평균, 가장자리 clamp). 새 shader
     program은 없습니다. 키우기: shadow는 쌓는 pass의 offset quad가, layer blur는 surface 크기 키우기 pass가 Linear로 읽습니다.
     CPU 기준은 같은 단계(`sample` Linear clamp)를 그대로 따릅니다.
   - 정밀도: d ≥ 2이면 d < σ / 32라 상자 평균·bilinear가 더하는 분산이 σ²의 1/4096 미만입니다. CPU 기준에서 σ 80을 device
     해상도로 직접 blur한 값과 낮은 해상도 값의 차이는 8-bit 한 단계보다 작습니다(test).
   - 진단: device σ가 `MAX_DEVICE_SIGMA`(64 × 64 = 4096)를 넘을 때만 `render.effect.blur_too_large`이고 그 상한으로 그립니다.
     `DeviceBlur::downsample`이 배수를 싣고, graph에는 `EffectStep::Downsample`·`EffectStep::Upsample` pass와 줄인 고정 크기
     texture가 들어갑니다.
   - fixture 계약: σ는 유한·비음수이고 `effect_downsample`이 배수를 정할 수 있어야 합니다(이전: σ ≤ 64). 배수는 σ의 순수
     함수라 fixture에 따로 적지 않습니다(CPU 기준·GPU lowering·pass compiler가 같은 함수).
3. **shadow offset은 소수 device pixel 그대로입니다.** fixture `ShadowSpec::offset`은 `[f32; 2]`입니다. 합성 quad를 offset
   자리에 두고 Linear로 읽습니다. 정수 offset이고 줄이지 않았으면 texel 중심을 읽어 이전(Nearest)과 같은 값입니다. builder
   `with_shadow([i32; 2], …)`는 그대로입니다.
4. **Metal 내보내기 supersampling 배수는 σ가 아니라 tile texture 크기로만 정합니다.** 배수는 8이고, 효과 여백을 더한 tile
   texture 한 변(`(tile + 2·여백)·배수`)이 `MAX_TILE_TEXTURE`(4096, `Rgba16Float` 한 장 128 MiB)와 device 상한을 넘을 때만 넘지
   않는 가장 큰 배수로 낮춥니다(`supersample_for(margin, tile, max_dimension)`). 배수 1로도 넘으면 실패합니다(근사하지 않음).
   - 여백은 shadow `|offset| + REACH_PER_SIGMA·σ`와 layer blur `REACH_PER_SIGMA·σ`의 경로 합입니다(`tile_margin`).
     `REACH_PER_SIGMA` = 3 + 1/16은 kernel 3σ에 줄이기·키우기가 더하는 거리(2d < σ / 16)를 더한 상한입니다.
   - 결과: blur 40(출력 σ 20) 그림자는 배수 3 대신 8로 그립니다. 출력 pixel σ가 64를 넘어도(예: 배율 4의 blur 40, σ 80)
     그리고, 여백 때문에 배수만 6으로 낮아집니다. 여백이 대략 출력 1984 pixel을 넘으면 실패합니다.

## Alternatives

- 내용 alpha 원본 유지(근사로 문서화): 배경 없는 카드·반투명 패널 그림자가 브라우저와 달라 탈락했습니다.
- 상자 원본을 scene에 별도 primitive로 두기: scene crate 변경과 문서 → scene 변환 변경이 필요하지만 renderer가 내용 종류로
  같은 판정을 할 수 있어 탈락했습니다(같은 규칙 한 곳, pass compiler `ShadowShape::of`).
- 상자 안 비우기를 shadow texture에 따로 pass로 적용: 소수 offset·낮은 해상도 좌표에 mesh를 옮겨야 하고 pass가 늘어 탈락했습니다.
  stencil 판정은 clip과 같은 방식입니다.
- 효과를 analytic(상자 shadow의 erf 닫힌 식)으로 그리기: 상자에는 정확하지만 둥근 모서리·drop-shadow에는 맞지 않아 두
  경로가 됩니다. 탈락했습니다.
- σ 상한을 크게 올리기: kernel 탭 수가 σ에 비례해 GPU 비용 상한(신뢰되지 않은 입력)이 무너져 탈락했습니다.
- supersampling 배수를 σ로 낮추기(이전 결정): 흔한 그림자에서 가장자리 품질이 내보내기 전체에서 떨어져 탈락했습니다.

## Consequences

- Frame 그림자가 CSS `box-shadow`와 같은 모양입니다. 배경이 불투명한 Frame은 결과가 이전과 같습니다.
- 큰 blur·출력 pixel σ 64 초과 그림자를 supersampling 배수 8(메모리 상한 안)로 그립니다.
- 남은 차이: codegen은 Frame에서 Blur가 Shadow보다 앞이면 `filter: blur() drop-shadow()`로 내지만, scene은 효과 순서를 싣지
  않아(ADR-0116 결정 2) renderer는 Frame 그림자를 늘 상자 모양으로 그립니다. 순서를 scene에 싣는 일은 scene 변환 소유 후속입니다.
- CPU 내보내기 target(`SoftwareTarget`)은 여전히 효과를 그리지 않습니다(ADR-0103 결정 2). CPU·Metal 대조는 같은 그리기
  목록의 CPU 기준과 비교합니다.

## Compatibility

- 파일 포맷·FFI 변경 없음.
- 공개 API 변경(renderer-core): `ShadowSpec::offset`이 `[i32; 2]` → `[f32; 2]`, `ShadowSpec::source`·`ShadowSource` 추가,
  `LayerOp::with_shadow_sigmas`가 소수 offset과 원본을 받음, `LayerOp::with_box_shadow`·`blur_downsample`,
  `ShadowSpec::downsample`, `pass_compiler::effects::{ShadowShape, EffectStep::{Downsample, Upsample}}`,
  `EffectStep::ShadowSource::shape`, `ShadowComposite::shape`, `DeviceBlur::downsample`, `effects::blur::{MAX_DOWNSAMPLE,
  MAX_DEVICE_SIGMA, REACH_PER_SIGMA, effect_downsample, downsampled_extent}`. renderer-metal: `supersample_for` 시그니처 변경,
  `tile_margin`·`MAX_TILE_TEXTURE` 추가. 사용처는 renderer crate 안뿐입니다.
- 기존 적합성 fixture(정수 offset, σ ≤ 64, 내용 alpha shadow)의 CPU 기준 결과는 그대로입니다.

## Performance

- 효과 pass는 줄인 크기에서 돌아 σ가 큰 blur의 GPU 비용(탭 수 × pixel 수)이 약 d³배 줄어듭니다. 줄이기 pass가 단계마다
  하나 더 생깁니다.
- 상자 shadow는 원본 pass가 내용 대신 상자 하나를 칠하고, 쌓는 pass에 stencil attachment와 stencil draw 두 개가 생깁니다.
- 내보내기 tile texture는 배수 8에서 한 변 `(128 + 2·여백)·8`입니다. 여백이 출력 128 pixel 이하면 배수 8입니다. 상한 4096은
  `Rgba16Float` 한 장 128 MiB이고, GPU 메모리 예산(transient pool, XI.8.7)을 넘으면 할당 전에 실패합니다. 내보내기 전용이라
  frame 예산(§6.2) 밖입니다.

## Security

- σ 입력이 커도 한 pass의 kernel 탭 수는 `MAX_SIGMA`로, 줄이기 배수는 `MAX_DOWNSAMPLE`로 묶입니다. tile texture 크기는
  `MAX_TILE_TEXTURE`로 묶입니다.

## Migration

- 없음. 저장 문서·fixture byte는 바뀌지 않습니다.
