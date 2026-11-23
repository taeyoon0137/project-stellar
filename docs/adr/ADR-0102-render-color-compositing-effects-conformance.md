# ADR-0102: 렌더 색·합성·효과 수식과 적합성 fixture

## Status

Accepted (2026-11-23, lane render-conformance 구현 기록)

## Context

아키텍처 §10.2·§10.3은 내부 색 연산과 alpha 합성을 선형 색공간에서 하고, gradient·blend·blur·shadow 계산 규칙을 두
backend가 공통으로 정의하며, 셰이더·블렌딩·필터별 허용 오차를 렌더 적합성 테스트에서 고정하라고 합니다. 계획서 XI.10.1\~XI.10.4,
XI.11.1\~XI.11.4가 이 일을 맡습니다. 이 lane을 시작할 때 상태는 다음과 같았습니다.

- shader ABI 문서(`renderer-metal` `shaders`)는 "색은 premultiplied 선형 값, encode 규칙은 XI.10.1에서 고정"이라고만 적었습니다.
  Metal backend는 `Bgra8Unorm` drawable에 sRGB 인코딩 colorspace를 붙였고 shader는 선형 값을 냈습니다. renderer-core HDR
  fallback(XI.10.6)이 `Rgba16Float`를 `Bgra8Unorm`으로 바꾸면 화면이 어둡게 표시되는 미결 문제가 있었습니다.
- `ShaderProgram::BlendComposite`·`GaussianBlur`는 Metal에서 `Unsupported`였습니다. scene paint에는 gradient가 없고, core-types
  `Gradient::visual_order`(III.4.3)가 stop 순서 규칙의 유일한 구현입니다.
- scene을 GPU draw command(정점 데이터)로 바꾸는 제품 frame encoder가 없습니다. pass compiler(XI.4.2\~XI.4.5)는 draw 계획까지
  만듭니다. 제품 fill tessellator도 없습니다(XI.6.1은 interface만, stroke만 기준 구현이 있음).
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·FFI는 바꾸지 않습니다. D3D12는 Windows 재개 때 합니다(ADR-0006).
- crate 의존 정책상 renderer-core는 scene에만, renderer-metal은 renderer-core에만 의존합니다.

## Decision

### 1. working space (XI.10.1)

- working 값은 surface 색공간(협상 결과 `OutputColorPlan::surface`) primaries의 선형광 RGB에 alpha를 곱한 premultiplied RGBA
  `f32`입니다(`stellar_renderer_core::color::WorkingColor`).
- 문서·scene 색과 decoded 이미지 texel은 `OutputColorPlan::convert`(XI.10.5) 하나로 working 값이 됩니다(`color::to_working`,
  `color::image_texel`). transfer 곡선은 core-types `ColorSpace::decode_channel`·`encode_channel`(ADR-0002)입니다. 같은 변환을
  다른 곳에 두지 않습니다.
- shader의 vertex color·tint·sampling 결과·blend 입력은 모두 working 값입니다. fragment는 encode하지 않습니다.

### 2. 출력 encode (XI.10.1)

- attachment format이 저장 방식을 정합니다(`color::output_encoding`). `*UnormSrgb`는 premultiplied 선형 채널마다 하드웨어 transfer
  encode(alpha는 선형), `Rgba8Unorm`·`Bgra8Unorm`은 선형 값 그대로 8-bit, `Rgba16Float`는 선형 half float, `R8Unorm`은 coverage입니다.
- 8-bit 양자화 기준은 `round(v × 255)`입니다. GPU 반올림 방향 차이(1 LSB)는 적합성 허용 오차가 흡수합니다.
- **미결이던 fallback drawable**: `Bgra8Unorm`은 선형 값을 저장하는 format으로 정하고, Metal backend는 그 layer에 선형 colorspace
  (`kCGColorSpaceLinearSRGB`·`kCGColorSpaceLinearDisplayP3`)를 붙입니다. shader가 format마다 encode 변형을 두지 않으므로 blend는 항상
  선형 값으로 계산됩니다. 선형 8-bit는 어두운 영역 계조가 거칠어 표시 기본값은 `Bgra8UnormSrgb`로 둡니다.

### 3. alpha 합성과 blend mode (XI.10.2)

- 기본 합성은 premultiplied source-over `Co = Cs + Cb·(1 − αs)`, `αo = αs + αb·(1 − αs)`입니다.
- blend mode는 W3C Compositing and Blending Level 1의 separable 식을 working 값(선형)에 씁니다.
  `Co = Cs·(1 − αb) + Cb·(1 − αs) + αs·αb·B(Cb/αb, Cs/αs)`, `αo`는 source-over와 같습니다. B는 multiply `cb·cs`, screen
  `cb + cs − cb·cs`, overlay `hard-light(cs, cb)`, darken `min`, lighten `max`, normal `cs`입니다(`stellar_renderer_core::blend`).
- 초기 지원 목록은 document·scene과 같은 6개(normal, multiply, screen, overlay, darken, lighten)입니다. 목록 밖 tag는
  `render.blend.unsupported_mode` 진단(`blend::BlendDiagnostic`)을 내고 Normal로 그립니다. 조용히 바꾸지 않습니다.
- Metal `BlendComposite(mode)`는 Apple GPU framebuffer fetch(`[[color(0)]]`)로 backdrop을 읽어 shader가 위 식으로 합성합니다. 그래서
  pipeline blend state는 `Replace`여야 하고, `PremultipliedOver`를 주면 `InvalidUsage`입니다.

### 4. gradient (XI.10.3)

- renderer는 stop을 **시각 순서로 받은 그대로** 씁니다. 순서는 core-types `Gradient::visual_order`(`(position, 안정 ID)`)가 정하며
  renderer는 다시 정렬하지 않습니다(같은 규칙의 두 번째 구현을 만들지 않음). renderer는 position이 줄지 않는지만 확인합니다.
- 보간은 premultiplied working 값(선형)으로 합니다(§10.3). CSS 기본(인코딩 sRGB 보간)과 결과가 다르므로 export는 blend와 같이
  근사·래스터 경계를 따로 판단합니다.
- 값 규칙: `t < 첫 position`은 첫 stop, `t ≥ 마지막 position`은 마지막 stop, 그 사이는 `p_i ≤ t < p_(i+1)`인 마지막 `i`와 다음 stop의
  선형 보간입니다. 같은 position의 stop들은 `t = p`에서 **뒤 stop**이 이깁니다(hard edge). 길이 0 gradient는 마지막 stop 단색입니다.
- 선형 gradient `t = dot(p − start, end − start) / |end − start|²`, 원형 gradient `t = |p − center| / radius`입니다.
- GPU 구현: stop을 1024 texel `Rgba16Float` ramp(texel 중심의 정확한 값)로 만들고, 선형 gradient는 `t`가 위치의 affine 함수이므로
  정점 uv.x = `t`로 `Texture` program이 linear filter·clamp로 읽습니다. 원형 gradient는 정점 보간으로 정확히 표현되지 않아 CPU
  수식만 두고, GPU 경로는 gradient 전용 shader program을 추가할 때 넣습니다.

### 5. 효과 순서와 수식 (XI.10.4)

node 합성 결과에 효과를 아래 순서로 적용합니다(CSS `filter → clip-path → mask → opacity → mix-blend-mode` 순서와 같음).

1. 내용: 자식과 자기 paint를 painter 순서로 layer에 그립니다.
2. shadow: 내용 alpha를 blur(σ = blur 반경 / 2, CSS `box-shadow`·`drop-shadow` 정의)하고 shadow 색(working, premultiplied)을 곱해
   offset만큼 옮긴 뒤, 목록 순서(앞이 아래)로 쌓고 그 위에 내용을 놓습니다.
3. layer blur: 위 결과 전체를 σ = blur 반경(CSS `filter: blur()` 정의)으로 blur합니다.
4. clip: scissor(사각형, pixel 정확), stencil(path, pixel 중심 판정), mask clip(coverage 곱).
5. mask: coverage(0\~1)를 곱합니다.
6. opacity: 값을 곱합니다.
7. blend: 결정 3의 식으로 backdrop에 합성합니다.

- gaussian 이산화: 반경 `R = ceil(3σ)`, 가중치 `w_i = exp(−i² / (2σ²))`를 `[−R, R]`에서 정규화, 가로 → 세로 분리 적용, texture 밖은
  투명입니다. σ = 0이면 blur하지 않습니다(`stellar_renderer_core::effects::blur`).
- shader ABI: `DrawUniforms { float4 tint; float depth; float mode; float2 param; }`(32 byte)입니다. `LayerComposite`·`BlendComposite`는
  `mode = 1`이면 texture slot 1 coverage mask(`.r`)를 곱합니다. `GaussianBlur`는 48 byte `BlurUniforms { tint; depth; sigma; direction;
  source_alpha; }`이고 `source_alpha = 1`이면 texel alpha를 모든 채널로 퍼뜨립니다(shadow 1단계). byte 배치는 renderer-core
  `effects::abi`가 소유하고 backend shader가 따릅니다.

### 6. 적합성 harness와 비교 (XI.11.1, XI.11.2)

- **common fixture**: backend 중립 `effects::fixture::Fixture`(크기, surface 색공간, target format, clear, 그리기 목록: mesh fill·
  gradient fill·이미지·glyph coverage·효과 layer)입니다. 같은 fixture를 CPU 기준 rasterizer(`effects::raster`)와 backend lowering
  (`effects::lower`, `RendererBackend` generic)이 씁니다. D3D12 harness는 같은 lowering을 씁니다.
- CPU 기준 rasterizer: pixel 중심 표본, top-left 규칙, 결정 1\~5의 수식, `f32`입니다.
- Metal harness는 headless backend로 offscreen render한 뒤 readback합니다. reference는 raw RGBA(`STRGBA1` header + 크기 + byte)와
  PNG(무압축 deflate) 두 가지로 씁니다. 제품 export 기능이 아닙니다.
- 허용 오차 config(`effects/tolerances.conf`, 줄 단위 `class key=value`)는 class마다 채널 허용 차이(`channel`), structural 반경
  (`radius`: 기대 image의 이웃 pixel이 허용 차이 안이면 위치 어긋남으로 인정), 허용 초과 pixel 수(`max_failures`), structural 인정
  수(`max_shifted`)를 둡니다.
- 판정 결과의 diff artifact는 초과 pixel 위치·기대·실제 값 목록(`failures.txt`)과 expected·actual·diff PNG입니다.
- fixture inventory는 Frame·Shape·Path·Text·Image·Modifier·clip·P3 분류마다 최소 개수와 전체 100개 이상을 검사합니다.
- fixture fill tessellation: 제품 fill tessellator가 없어 `effects::fill::SweepFillTessellator`(`FillTessellator` 구현, 가로 band
  사다리꼴 분해로 nonzero·evenodd 모두 정확)를 둡니다. 겹치는 stroke mesh는 stencil union(`IncrementWhereEqual` 후
  `DecrementWhereEqual`)으로 pixel마다 한 번만 칠합니다.

### 7. 2만 node 구간 benchmark (XI.11.4)

- `benchmarks/fixtures`의 `render_stage_benchmark`가 2만 가시 node scene에서 scene build(증분 build 전체 재빌드), render graph
  compile·batching, Metal encode, submit을 frame마다 재고 단계별 nearest-rank p50/p95/p99와 present queue 지표(XI.11.5)를 JSON 한
  줄로 냅니다.
- 제품 frame encoder가 없으므로 batch → draw command 변환은 benchmark 안 대리 구현(사각형 bounds 단색 quad)입니다. 제품 frame
  encoder가 생기면 그것으로 바꿉니다.

## Alternatives

- **fallback 대상 `Bgra8UnormSrgb`로 변경**: `OutputColorPlan::surface_config`(이 lane 소유 밖)를 바꿔야 하고, 셸이 `Bgra8Unorm`을
  직접 고르면 같은 문제가 남습니다. format별 colorspace 규칙이 모든 경우를 덮습니다.
- **shader가 `Bgra8Unorm`에 encode**: 하드웨어 blend가 인코딩 값으로 계산되어 선형 합성 원칙(§10.3)이 깨집니다.
- **blend용 backdrop 복사**: copy pass와 texture가 더 들고 Metal 계약(그리는 attachment sampling 금지) 때문에 graph가 복잡해집니다.
  Apple Silicon만 대상이므로(ADR-0006) framebuffer fetch가 더 단순합니다.
- **renderer에서 stop 정렬**: core-types 규칙의 사본이 생깁니다.
- **CSS 보간 공간(sRGB 인코딩) 사용**: §10.3과 blend 선형 합성(ADR-0091 export 경계)과 어긋납니다.
- **gradient 정점 색 분할**: shape mesh를 stop 경계로 잘라야 하고 원형에도 맞지 않습니다. ramp texture가 더 단순합니다.
- **fixture를 SceneSnapshot으로 정의**: renderer-core는 core-types 타입을 만들 수 없어 codec byte로만 만들 수 있고, scene → draw
  command 제품 경로가 없어 lowering을 따로 만들어야 합니다. scene lowering은 제품 frame encoder가 맡습니다.

## Consequences

- 모든 backend가 같은 색·합성·효과 수식과 fixture를 씁니다. D3D12 harness는 lowering을 그대로 쓰고 backend shader만 추가합니다.
- `output_color` 모듈 문서의 "미결(XI.10.1)" 문단은 이 결정으로 풀렸지만 이 lane 소유 밖이라 고치지 않았습니다. 다음 정리에서
  결정 2를 가리키게 고칩니다.
- `effects::fill`은 stroke tessellator의 내부 평탄화와 별도로 cubic을 평탄화합니다. 두 평탄화를 하나로 합치는 정리(stroke 평탄화
  공개)가 후속입니다.
- Text fixture는 실제 font glyph가 아니라 합성 coverage bitmap으로 glyph atlas 합성 경로를 검사합니다. 실제 glyph rasterization
  비교는 text rasterizer가 들어온 뒤 더합니다.
- benchmark의 frame encoder 대리 구현은 제품 경로가 아닙니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. renderer-core에 `color`, `blend`, `effects` 공개 모듈이 생기고, renderer-metal shader ABI(`DrawUniforms`
  뒤쪽 필드, `GaussianBlur` uniform, slot 1 mask)가 늘었습니다. 기존 program의 동작은 같습니다.

## Performance

- 제품 경로 변화는 `Bgra8Unorm` layer colorspace뿐입니다(compositor 변환 비용은 OS 몫).
- blur shader는 `2R + 1` texel을 읽습니다(σ = 8이면 49). 효과 layer는 기존과 같이 offscreen boundary(XI.4.4)에서만 생깁니다.
- benchmark 결과는 아키텍처 §6.2 Scene·Render Graph 예산과 비교할 수 있는 단계별 수치를 줍니다.

## Security

- harness artifact는 fixture 이름·크기·pixel만 기록합니다. 사용자 문서 내용을 읽지 않습니다.

## Migration

- 없음. 기존 test의 `Rgba8Unorm` target은 선형 저장 그대로입니다.
