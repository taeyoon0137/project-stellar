# ADR-0116: 렌더 효과 pass, 문서 scene 변환 위치, PNG 내보내기 명령과 Metal offscreen target

## Status

Accepted (2026-11-23).

- ADR-0103 결정 8(CLI·MCP 연결 보류)을 결정 1·5로 풉니다.
- ADR-0103 결정 2의 "GPU target은 같은 trait을 구현" 후속을 결정 4로 채웁니다.
- ADR-0019·ADR-0028의 crate 의존 표에 `stellar-scene → stellar-document`, `stellar-scene → stellar-commands`(feature
  한정), `stellar-ops → stellar-assets`, `stellar-renderer-metal → stellar-export, stellar-scene`을 더합니다.

## Context

- 저장 문서 → scene 변환(canvas scene source, ADR-0084)은 앱 runtime(`crates/runtime/src/canvas/scene_source.rs`)에
  있었습니다. headless 도구(ops·CLI·MCP)는 runtime에 닿지 않으므로(runtime → ops 방향, ADR-0005 §8) PNG 내보내기
  엔진(ADR-0103)을 명령으로 연결하지 못했습니다.
- 같은 변환을 ops에 다시 만들면 이중 경로입니다(ADR-0103 Alternatives).
- 문서 Shadow·Blur·Blend·Opacity modifier는 scene에 닿지 않았습니다(도형 modifier 규칙 밖이라 node가 skip됨). scene paint는
  효과 field를 가졌지만 pass compiler는 layer 경계까지만 만들고 shadow·blur·mask pass와 blend program을 만들지 않았습니다
  (XI.10.4 보고서 남은 일). `blend::resolve_or_normal`을 부르는 제품 경로가 없었습니다.
- PNG 내보내기 offscreen target은 CPU 구현뿐이었고, scene → draw command 제품 frame encoder가 없었습니다(ADR-0102 결정 6).

## Decision

1. **문서 → scene 변환은 scene crate의 `document` 모듈에 둡니다.** `stellar_scene::document`(feature `document`)가
   `document_scene_source`·`scene_edits`·`scene_invalidations`와 node 입력 함수(`inputs::frame_input`·`shape_source`·
   `shape_style`·`image_input`·`scene_command`·`scene_fit`)를 소유합니다. 앱 runtime은 이 모듈을 다시 노출하고
   (`stellar_runtime::canvas::scene_source`), 캔버스 행도 같은 입력 함수를 씁니다. ops는 같은 모듈로 저장 문서의
   scene을 만듭니다.
   - feature 한정 의존: `stellar-scene → stellar-document`, `stellar-scene → stellar-commands`(change set 입력). renderer-core·
     export는 feature를 켜지 않으므로 renderer는 문서 계층을 빌드하지 않습니다. 순환은 없습니다(document·commands는 scene에
     의존하지 않음).
   - 변환 규칙(숨김 유지, layout Frame·Path clip skip, instance 펼침, Boolean host, View Variant 표시 patch)은 ADR-0084 그대로입니다.
2. **문서 효과 modifier를 scene 합성 효과로 평가합니다(`stellar_scene::document::effects`).** Shadow·Blur·Blend·Opacity
   modifier는 node 합성 결과에 걸리는 CSS 효과(`box-shadow`·`filter: drop-shadow()`, `filter: blur()`, `mix-blend-mode`,
   `opacity`)라 내용 모양(transform·fill·stroke)과 따로 평가합니다. 변환은 이 종류를 stack에서 떼어 scene
   `ModifierPaint`의 `shadows`·`opacity`·`blend`·`layer_blur`에 넣고, 나머지 stack은 기존 입력 함수가 그대로 평가합니다.
   도형 modifier 규칙(`evaluate_shape_modifiers`)과 캔버스 행의 동작은 바뀌지 않습니다.
   - Opacity는 활성 항목의 곱(`ModifierStack::effective_opacity`, 문서 규칙)입니다. Shadow는 활성 항목 전부이고 OrderKey
     순서가 scene 목록 순서(앞이 아래)입니다. Blur·Blend는 CSS 속성 하나에 대응하므로 활성 항목이 둘 이상이면 평가하지
     않고 `SkipReason::UnsupportedModifiers`로 남깁니다(근사하지 않음). blend tag는 문서·scene이 같습니다.
   - Frame은 효과 modifier만 가진 stack을 평가합니다. 다른 종류가 있으면 전처럼 skip합니다. Boolean operand의 효과는
     결과에 흡수되어 쓰지 않습니다. 효과 적용 순서는 renderer가 정합니다(ADR-0102 결정 5).
3. **pass compiler가 효과 pass와 합성 program을 만듭니다(`pass_compiler::effects`).** draw 순서·layer 경계·clip 결정
   (XI.4.2\~XI.4.4) 뒤, 효과가 필요한 draw와 layer에 graph pass를 더하고 backend가 실행할 값을 `FramePlan`에 붙입니다
   (`effect_passes`·`draw_effects`·`layer_effects`). 수식과 순서는 ADR-0102 결정 5입니다.
   - **합성 program**: 모든 draw의 blend mode를 `blend::resolve_or_normal`로 풀고(목록 밖 tag는
     `render.blend.unsupported_mode` 진단을 `blend_diagnostics`에 남기고 Normal), `composite_program`·`composite_blend_state`로
     program과 blend state를 정합니다. layer 합성 draw는 경계 node 자기 blend가 Normal이 아니면 그것, 아니면 접힌 blend입니다.
     layer 합성 draw에는 경계 node 불투명도를 함께 둡니다(접힌 불투명도와 곱함).
   - **layer blur**: layer pass 뒤 `GaussianBlur` 가로·세로 pass, 부모는 세로 결과를 읽습니다. σ = 반경.
   - **drop shadow**: shadow가 있는 내용 draw마다 node 자기 내용(shadow 제외)을 그리는 원본 pass 하나, shadow마다 가로(alpha를
     퍼뜨림)·세로(shadow 색을 곱함) blur pass입니다. target은 내용 draw 바로 아래에 offset만큼 옮긴 결과를 목록 순서(앞이 아래)로
     합성합니다. σ = blur 반경 / 2. scene의 shadow는 node 자기 내용의 shadow입니다(자식 제외, Frame은 CSS `box-shadow`와 같음).
   - **clip mask**: clip 방식이 `ClipPlan::Mask`인 draw는 복합 clip coverage(나열한 clip의 곱)를 `R8Unorm`에 그리는 pass를
     두고 draw가 그것을 곱합니다. 같은 clip stack·같은 복합 clip 목록의 draw는 mask 하나를 같이 씁니다(형제 draw마다 화면 크기
     texture를 만들지 않음, 2만 node 구간 benchmark가 GPU 메모리 예산 안에 듦).
   - **device 값**: σ·offset은 node local 단위입니다. world → device 선형 부분 `M`으로 offset은 `M·offset`, σ는 축마다
     `σ·√(a² + c²)`, `σ·√(b² + d²)`입니다. `M·Mᵀ`가 대각(축 정렬, 회전 + 균일 배율)이면 정확하고, 기울임·비균일 회전은 대각
     성분만 쓰는 근사이며 `render.effect.anisotropic_blur` 진단을 남깁니다. σ가 `MAX_SIGMA`(64)를 넘으면 상한으로 그리고
     `render.effect.blur_too_large` 진단을 남깁니다. 진단이 있는 plan을 정확해야 하는 곳(내보내기)은 거부합니다.
4. **scene frame encoder와 Metal offscreen target(XX.2.1).**
   - `effects::scene::encode_plan`은 frame plan을 적합성 fixture와 같은 그리기 목록(`effects::fixture::Op`)으로 낮춥니다. 그래서
     scene → pixel 경로가 fixture로 검증한 수식·효과 순서(ADR-0102)를 그대로 쓰고, 같은 목록을 CPU 기준
     (`effects::raster`)과 GPU lowering(`effects::lower::lower_ops`, 임의 크기·format target)이 실행합니다. main target draw는
     painter 순서(opaque·transparent 분리는 결과가 같은 GPU 최적화라 합침), layer 합성은 layer pass draw를 담은 `LayerOp`,
     draw 효과는 `LayerOp` 하나의 순서(내용 → shadow → blur → clip → opacity → blend)입니다. 복합 clip이 여럿이면 layer를 겹쳐
     교집합으로 만들고 blend는 바깥 layer에서만 합성합니다. shadow offset은 device pixel 정수로 반올림합니다. stroke `Inside`는
     geometry 안, `Outside`는 경계 상자에서 geometry를 even-odd로 뺀 영역으로 자릅니다. 축 정렬이 아닌 이미지 변환은 미지원
     오류입니다. shadow·layer blur의 축별 σ를 표현하려고 fixture 모델에 축별 배율을 더했습니다(기본 등방, 기존 fixture 불변).
   - `stellar_renderer_metal::offscreen_target::MetalOffscreenTarget`이 export `OffscreenTarget`을 구현합니다. capture snapshot
     → frame plan → encoder → lowering → headless Metal(`Rgba16Float`) → readback입니다. 가장자리 coverage는 축마다 8배
     supersampling이고 output pixel마다 연관 알파 선형광 값을 상자 평균한 뒤 CPU target과 같은 마지막 단계(비연관 → transfer
     encode → 8-bit 반올림, 알파 0이면 RGB 0)를 거칩니다. 출력을 128 pixel tile로 나누고 효과가 번지는 거리(shadow offset + 3σ,
     layer blur 3σ의 경로 합) 이상 여백을 두고 그려 tile 경계가 이어집니다. 효과 근사 진단이 있으면 그리지 않고 실패합니다.
     selection 내보내기(조상 clip·효과 제외)는 pass compiler가 subtree 시작을 받지 않아 아직 실패입니다.
   - crate 의존: `stellar-renderer-metal → stellar-export, stellar-scene`(test는 core-types). export·ops·CLI는 GPU backend에
     의존하지 않으므로 headless 도구는 계속 CPU target을 씁니다(ADR-0005 §8).
   - golden 대조(XX.2.4): 같은 snapshot·옵션의 두 target PNG를 해석해 연관 알파 값으로 비교합니다. 내부 pixel ±2, 가장자리
     pixel은 경계를 공유하지 않는 모양 40, 공유하는 모양(배경과 안쪽 테두리, fill과 stroke) 128, 내부 허용치를 넘는 pixel은
     12% 이하입니다. 공유 경계 차이는 CPU target이 그리기마다 coverage를 곱해 합성해 아래 그리기가 비치는 coverage 합성의
     성질이고 GPU sample 합성에는 없습니다. CPU target이 그리지 않는 효과는 같은 그리기 목록의 CPU 기준과 ±3으로 비교합니다.
5. **`stellar export png`와 MCP `document.export_png`(ADR-0103 결정 8을 풂).** 두 경로 모두
   `stellar_ops::export_png::view`의 한 함수(`prepare_view_png`)를 씁니다. 저장 generation의 문서 View → 결정 1의 scene 변환 →
   `IncrementalScene` snapshot → `ExportCapture` → `ExportPreflight` → `ConfirmedExport` → CPU target(`SoftwareTarget`)입니다.
   - 입력은 저장 generation뿐입니다(미저장 편집·활성 Variant 제외). asset·font revision은 generation 번호로 고정합니다.
   - **빠진 node**: 변환이 넣지 못한 node(`SkipReason`, 예: headless에는 Text layout hook이 없음, layout Frame)는 기본
     거부입니다. CLI `--omit-skipped`·MCP `omit_skipped`로 빼고 그리는 것을 확인합니다. 결과에 node와 이유 code를 남깁니다.
   - **사전 진단**: 확인이 필요한 진단(없는 이미지, 검사하지 못한 target profile)은 기본 거부이고 `--accept-issues`·
     `accept_issues`로 확인합니다. headless scene은 target profile을 평가하지 않으므로 profile key는 `base`이고,
     `stellar.yaml`에 target profile이 있으면 검사하지 못한 profile로 진단합니다(조용히 통과시키지 않음).
   - **이미지**: generation의 asset metadata chunk와 asset content store(symlink를 따르지 않는 읽기)에서 PNG 내용을 읽습니다.
     이를 위해 `stellar-ops → stellar-assets`를 허용합니다. PNG가 아닌 asset은 CPU target이 해석하지 못하므로 렌더 전에
     거부합니다.
   - **CLI**는 temp + atomic rename으로 저장합니다(`PngExport`, ADR-0103 결정 6). **MCP**는 파일을 쓰지 않는 읽기 tool이며
     PNG를 base64(SVG data URI와 같은 encoder)로 돌려줍니다. 요청 계약상 필요한 출력 경로는 쓰지 않는 이름입니다. 응답이
     JSON-RPC 메시지 상한 안에 들도록 PNG는 512 KiB까지이고, 넘으면 `too-large`입니다.
   - 배경 색은 문서 CSS 색 parser(`css::value::parse_color`)로 읽습니다.

## Alternatives

- ops에 별도 변환: 이중 경로라 탈락했습니다.
- MCP tool이 파일을 씀: headless backend는 프로젝트 파일을 바꾸지 않는 읽기·dry-run만 제공하므로(ADR-0005 §3) 탈락했습니다.
- 빠진 node를 조용히 생략: 결과가 화면과 달라지는데 알리지 않게 되어 탈락했습니다.
- 새 crate(`stellar-scene-source`): crate 하나와 정책 행이 더 생기지만 얻는 경계가 feature 한정 의존과 같아 탈락했습니다.
- scene이 무조건 document에 의존: renderer-core와 GPU backend가 문서 계층을 빌드하게 되어 탈락했습니다.

## Consequences

- 앱 runtime, xtask replay, headless 도구가 같은 변환 함수를 씁니다.
- 앱이 같은 scene에서 CPU·GPU 어느 target으로도 PNG를 만들 수 있고, 두 결과의 차이가 golden test로 드러납니다.
- CPU target의 공유 경계 번짐(coverage 합성)은 그대로입니다. 고치려면 CPU target이 sample 합성이나 analytic 합집합 coverage를
  써야 합니다(export raster 소유 후속).
- frame plan이 효과 pass를 graph에 포함하므로 transient 수명·메모리 계산이 효과 texture를 셉니다. shadow가 있는 draw마다 원본
  layer 하나와 shadow마다 blur texture 둘이 생깁니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. 공개 경로 `stellar_runtime::canvas::scene_source`는 re-export로 유지됩니다.
- 새 공개 API: `stellar_scene::document`, `pass_compiler::effects`, `FramePlan`의 효과 field, `effects::scene`,
  `effects::lower::{lower_ops, OpsTarget}`, `stellar_renderer_metal::offscreen_target`, `stellar_ops::export_png::view`, MCP
  `document.export_png`. fixture `ShadowSpec`·`LayerOp`에 축별 배율 field가 생겼습니다(builder는 그대로).
- crate 의존 표(`tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`)가 바뀌었습니다.

## Performance

- 변환 코드는 그대로 옮겼습니다.
- Metal target은 tile마다 supersample target과 효과 layer(`Rgba16Float`, (128 + 2·여백)·8 한 변)를 씁니다. draw마다 효과·clip이
  있으면 전체 tile 크기 layer를 만듭니다. 내보내기 전용이라 frame 예산(§6.2) 밖이며 실제 시간 측정은 XX.5.3 benchmark 몫입니다.
- pass compiler는 draw마다 합성 program 하나를 풀고(`DrawEffects`, 할당 없음), 효과가 있는 draw·layer에만 pass를 더합니다.
  효과가 없는 scene의 graph는 이전과 같습니다.

## Security

- 변화 없음.

## Migration

- 없음.
