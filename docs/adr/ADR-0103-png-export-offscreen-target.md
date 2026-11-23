# ADR-0103: PNG 내보내기의 offscreen target 계약, CPU target, PNG codec, 저장, golden

## Status

Accepted (2026-11-23).

- PNG 내보내기 행(XX.2.1\~XX.2.4)의 구현 결정을 기록합니다.
- ADR-0019(export crate 의존은 core-types·scene)와 ADR-0091 결정 2의 "export는 renderer에 의존하지 않음"을 결정 3의
  범위로 고칩니다. GPU backend(renderer-metal·d3d12)에는 여전히 닿지 않습니다.

## Context

- 계획서 XX.2.1은 "offscreen RenderGraph target으로 지정 scale의 RGBA image"를 요구합니다. renderer-core에는 scene을
  frame plan(pass·layer·clip)으로 바꾸는 compiler(XI.4.2\~XI.4.4)와 stroke tessellation(XI.6.2), 출력 색 변환
  (XI.10.5)이 있지만, frame plan을 실제 draw command(vertex buffer·shader)로 낮추는 경로와 Metal offscreen 기준 harness
  (XI.11.1)는 아직 없습니다. 같은 시기에 다른 작업이 Metal offscreen harness와 색·합성·효과 공식(XI.10.1\~XI.10.4)을
  만들고 있습니다.
- headless 도구(CLI·MCP)는 crate 의존 정책상 GPU backend에 닿을 수 없습니다(ADR-0005 §8). PNG 내보내기를 AI 도구로도
  쓰려면 GPU 없이 그리는 경로가 필요합니다.
- export crate는 문서 모델에 의존하지 않고 고정 입력(`ExportCapture`·`ConfirmedExport`)만 읽습니다(XX.1.2, XX.1.3).
- 저장 문서에서 scene을 만드는 경로(canvas scene source)는 앱 runtime(`crates/runtime/src/canvas`)에 있습니다. ops와
  CLI는 runtime에 닿지 않습니다(runtime → ops 방향).
- 외부 crate 없이 PNG를 쓰는 기존 경로는 없습니다. SVG 래스터 경계(XX.4.2)는 host가 주는 PNG를 넣기만 합니다.

## Decision

1. **offscreen target 계약(XX.2.1).** `stellar_export::raster`에 `OffscreenTarget` trait과 `OffscreenRequest`를 둡니다.
   - 요청은 진행 허가 하나와 출력 profile입니다. 출력 크기와 좌표는 SVG 래스터 경계와 같은 `PixelGrid`(capture 영역 ×
     배율, 각 변 올림·최소 1)의 canvas 전체입니다. 한 변 16384, 전체 4096² pixel을 넘으면 `TooLarge`입니다.
   - 출력 `RgbaImage`는 행 우선 RGBA8이고, RGB는 출력 profile 색공간의 transfer 인코딩 값, 알파는 비연관입니다. 알파 0
     pixel의 RGB는 0입니다. `check_image`가 크기·profile을 검사합니다.
   - 합성은 출력 색공간 선형광, 연관 알파입니다(아키텍처 §10.3). scene 색과 이미지 pixel은 renderer
     `OutputColorPlan::convert`(XI.10.5)로 옮깁니다. 파일 출력은 두 색공간을 모두 표현하므로 요청 profile 그대로 협상합니다.
   - 그리기 의미는 renderer pass compiler와 같습니다. paint 순서(pre-order), node 자기 내용(image pixel·text run 색 →
     paint layer) 뒤 자식, 합성 불투명도 < 1이면 subtree layer를 불투명도와 경계 node 바깥 clip으로 합성하고 layer 안은
     안쪽 clip만 적용합니다. selection root는 조상 clip(`base_clip_depth`)과 조상 합성 효과를 그리지 않습니다.
2. **CPU target(`SoftwareTarget`).** 계약의 첫 구현은 CPU입니다. headless 도구와 앱이 같은 target을 씁니다.
   - 재사용: stroke는 renderer `tessellate_stroke`(join·cap·dash와 Inside·Outside 규약), 이미지 배치는 scene
     `ImageFit::place`, Text는 SVG 내보내기와 같은 `text_outlines`(고정 font revision outline), 색 변환은
     `OutputColorPlan`입니다. 같은 값을 따로 해석하는 공식을 두지 않습니다.
   - coverage: 닫힌 polygon을 pixel 행당 16 sub-scanline으로 훑고 가로는 정확한 넓이를 더합니다(non-zero·even-odd).
     곡선은 device 0.05 pixel 허용 오차로 평탄화합니다.
   - 렌더러가 공식을 아직 고정하지 않은 효과(drop shadow, layer blur, Normal이 아닌 blend)는 근사하지 않고 그리기 전에
     `Unsupported`로 멈춥니다(ADR-0091 결정 2와 같은 원칙). PNG가 아닌 이미지 asset, interlace PNG, sRGB·Display P3가
     아닌 내장 색 profile, outline 없는 glyph도 `Unsupported`입니다. XI.10.2·XI.10.4 공식이 고정되면 같은 trait 안에서
     넓힙니다.
   - GPU(Metal RenderGraph offscreen) target은 XI.11.1 harness가 생기면 같은 trait을 구현합니다. 두 target의 일치는
     결정 6의 decoded pixel golden으로 확인합니다.
3. **crate 의존.** `stellar-export → stellar-renderer-core`를 허용합니다. renderer-core는 backend 중립(GPU 객체 없음)이고
   export는 출력 색 변환·stroke tessellation만 씁니다. `stellar-ops → stellar-export`(PNG 실행·저장)와
   `stellar-ops → stellar-scene`(저장 테스트 fixture)을 허용합니다. CLI의 renderer-core 전이 도달은 GPU backend가 아니므로
   금지 목록에 넣지 않습니다.
4. **PNG codec.** export crate가 외부 crate 없이 zlib·PNG를 구현합니다(`stellar_export::png`).
   - 압축: LZ77(32 KiB 창, hash chain 64) + 고정 Huffman, 결정론적입니다. 행 filter는 다섯 종류 중 부호 있는 byte 절댓값
     합이 가장 작은 것입니다. `tIME`·텍스트 chunk를 쓰지 않아 같은 image·배율이면 bytes가 같습니다.
   - 해제·해석: stored·고정·동적 Huffman 전체, 비interlace의 모든 색 형식·bit depth, `tRNS`, chunk CRC 검사, 크기·해제
     상한. 신뢰하지 않는 입력에 panic하지 않습니다. 이미지 asset과 golden 비교가 같은 decoder를 씁니다.
5. **색 metadata와 encoder 지원 표(XX.2.2).**
   - sRGB: `sRGB`(perceptual) + `gAMA` 45455 + `cHRM`(sRGB primaries).
   - Display P3: `cICP`(primaries 12, transfer 13, matrix 0, full range) + `iCCP`("Display P3", 생성 ICC v4.3 profile:
     CIE xy primaries에서 계산하고 Bradford로 D50에 맞춘 colorant, sRGB parametric curve, 고정 날짜).
   - `pHYs`: 배율 1 = 72 dpi(배율 × 2834.6 pixel/m).
   - 지원 조합은 {sRGB, Display P3} × 8-bit입니다. 16-bit와 그 밖의 profile 이름은 `PngOptionsError`로 거부하고 조용히 바꾸지
     않습니다.
   - 이미지 asset 색공간 판정: `cICP`가 우선(1 → sRGB, 12 → P3, transfer 13), 없으면 `iCCP` colorant 비교(허용 오차 0.002),
     없으면 sRGB입니다.
   - 내보내기 결과는 출력 profile로 옮길 때 잘린 색과 확장 범위 색(XI.10.6)을 `GamutSummary`로 돌려줍니다.
6. **worker와 저장(XX.2.3).** 렌더·encode는 `PngJob`이 이름 있는 worker thread에서 실행하고 draw·행·압축 block 사이에서
   취소를 확인합니다. 저장은 `stellar_ops::export_png`가 storage `write_file_atomic_observed`(같은 디렉터리 temp →
   flush → fsync → rename → 부모 sync)로 합니다. 쓰기 단계마다 취소를 확인하고, 취소·실패면 storage가 이 호출의 temp를
   지우며 최종 경로는 이전 상태입니다. 출력 디렉터리는 만들지 않습니다.
7. **golden(XX.2.4).** 같은 snapshot·옵션의 PNG를 해석한 pixel을 저장 golden과 채널마다 ±1(8-bit) 안에서 비교하고,
   chunk 구성과 색 정보는 정확히 비교합니다. 같은 입력은 bytes도 같아야 합니다. 갱신은 `STELLAR_PNG_BLESS=1`입니다.
   Windows는 보류(ADR-0006)라 macOS에서만 실행합니다. CPU 계산은 플랫폼 독립이고 허용 오차는 transfer 함수 `powf`의 libm
   차이를 덮습니다.
8. **CLI·MCP 연결 보류.** `stellar export png`와 MCP 도구는 이번에 추가하지 않습니다. 저장 문서 → scene 변환이 runtime에만
   있어 ops·CLI가 scene을 만들 수 없기 때문입니다. 동작하지 않는 명령을 등록하지 않습니다. canvas scene source를 ops가 쓸
   수 있는 crate로 옮기는 작업(runtime canvas 소유)이 끝나면, CLI·MCP는 그 scene으로 capture를 만들고 사전 진단 →
   `stellar_ops::export_png::PngExport`를 부릅니다.

## Alternatives

- export가 GPU backend(renderer-metal)로 직접 렌더: headless 도구가 GPU crate에 닿게 되고(ADR-0005 §8), Metal offscreen
  harness와 draw 낮추기가 아직 없어 탈락했습니다.
- 외부 PNG crate(`png`, `miniz_oxide`) 사용: 승인된 외부 crate 정책상 가능하지만 export crate 의존을 늘리고, 압축 출력이
  crate 버전에 따라 바뀌어 bytes 결정론을 crate 밖에 맡기게 됩니다. 필요한 기능(쓰기, 이미지 asset 해석)이 작아 탈락했습니다.
- 효과를 CPU에서 근사(Gaussian blur, W3C blend 공식): 렌더러 공식이 정해지기 전에 두 번째 공식을 만들게 되고 결과가 앱
  캔버스와 달라질 수 있어 탈락했습니다.
- ops가 저장 문서에서 scene을 직접 만듦: runtime canvas scene source와 같은 변환을 따로 두는 이중 경로라 탈락했습니다.
- 동작하지 않는 `stellar export png` 명령을 먼저 등록: 사용자·AI가 실패만 받는 명령이라 탈락했습니다.

## Consequences

- PNG 내보내기 엔진(렌더·encode·색 metadata·취소·atomic 저장·golden)은 GPU 없이 동작합니다. 앱 runtime은 scene snapshot이
  있으므로 바로 `PngExport`를 부를 수 있습니다.
- shadow·blur·blend가 있는 화면은 PNG 내보내기가 미지원 오류입니다. 렌더 공식 행이 끝나면 풀어야 합니다.
- CLI·MCP의 PNG 내보내기는 scene source 이동 뒤에 연결합니다.
- GPU target이 생기면 CPU target과의 차이를 golden으로 드러냅니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. 새 공개 API: `stellar_export::raster`, `stellar_export::png`, `stellar_ops::export_png`.
- crate 의존 표(`tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`)에 edge 셋이 늘었습니다.
- 기존 SVG golden과 SVG 경로는 바뀌지 않습니다.

## Performance

- CPU 렌더는 draw마다 coverage mask(덮는 상자 크기)를 만듭니다. 비용은 대략 덮는 pixel 수 × 16 sub-scanline × 활성 변
  수입니다. 합성 buffer는 pixel당 16 byte이고 opacity layer는 쓴 행만 만듭니다.
- 내보내기는 worker thread에서 돌아 UI·document thread를 막지 않습니다. 실제 시간 예산 측정은 XX.5.3 benchmark 몫입니다.

## Security

- PNG·zlib 해석은 신뢰하지 않는 입력을 가정합니다(CRC, 크기·해제 상한, panic 없음, 임의 변형 test).
- 출력 경로는 `OutputPath` 검증(절대 경로, `..` 없음)을 거치고, temp는 storage 규칙(`O_EXCL`, symlink 따라가지 않음)으로
  만듭니다.

## Migration

- 없음. 저장 데이터와 문서 형식을 바꾸지 않습니다.
