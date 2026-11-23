# ADR-0091: SVG 내보내기의 vector 범위, 래스터 경계 host 렌더, report, 값 표기 위치

## Status

Accepted (2026-11-23).

- SVG 내보내기 남은 행(XX.1.3, XX.3.2, XX.3.4, XX.4.2, XX.4.3, XX.4.4, IX.11.20)의 구현 결정을 기록합니다.
- ADR-0019(export crate 의존은 core-types·scene뿐)는 바꾸지 않습니다.

## Context

- 아키텍처 §34 SVG 정책: 텍스트는 Path로, 정확히 표현할 수 없는 최소 subtree만 래스터화하고 나머지는 vector로 둡니다.
- 래스터 경계 planner(XX.4.1)는 완료됐지만, 경계를 렌더할 renderer offscreen target(XX.2.1)이 아직 없습니다. 같은 시기에
  다른 작업이 Metal offscreen 렌더를 만들고 있습니다.
- export crate는 문서 모델(`stellar-document`)과 renderer에 의존하지 않습니다(ADR-0019, `tools/xtask/src/crate_graph.rs`).
  그래서 CSS 값 formatter(`crates/document/src/css/value.rs`, IX.11.13)를 export에서 직접 부를 수 없습니다.
- scene `Paint`에는 단색만 있고 gradient 기하가 없습니다. core-types `Gradient`는 stop 목록만 가집니다.
- 효과와 clip의 적용 순서·blur 반경 수식·합성 색공간은 렌더 행(XI.10.2, XI.10.4)이 고정합니다. SVG 렌더러는 sRGB에서,
  Stellar 렌더러는 선형 공간에서 합성합니다(아키텍처 §10.3).

## Decision

1. **vector 범위(XX.3.4).** scene 값만으로 SVG 표기가 정확히 정해지는 기능만 vector로 씁니다.
   - clip(직각·둥근·path·중첩 chain, Inside stroke 자기 clip), group opacity, 색 알파, Outside stroke입니다.
   - Outside stroke는 두 배 폭 stroke에 `<mask maskUnits="userSpaceOnUse">`를 겁니다. mask는 흰 영역 위에 geometry를
     자기 채우기 규칙으로 검게 칠합니다. 흰색·검은색 luminance는 색공간과 무관하게 1·0이라 정확합니다. 영역은 geometry
     상자를 가장 넓은 Outside stroke 폭 × 4(SVG 기본 miter limit 4의 도달 거리)만큼 넓힙니다.
   - blur·drop shadow·Normal이 아닌 blend·image 색공간 override는 래스터 대상으로 둡니다. blur·shadow는 반경 의미가
     렌더러 수식과 다르고, blend는 합성 색공간이 달라 결과가 다릅니다. opacity·알파도 색공간 차이는 있지만 구조가 같고
     차이가 렌더 허용 오차 문제라 vector로 둡니다(XX.3.1 결정 유지).
   - gradient는 scene 값이 없어 mapping row를 `NotInScene`으로 둡니다. scene `Paint`에 gradient 기하가 생기면 row와
     writer를 함께 바꿉니다.
2. **래스터 경계 렌더는 host가 맡습니다(XX.4.2).** export는 `ExportRasterizer` trait과 `RasterRequest`(경계, 출력 pixel
   상자, device pixel grid, 그릴 root·자기 내용만 여부·건너뛸 clip 깊이, canvas 배경)를 둡니다. host(앱·CLI)가 renderer
   offscreen target으로 구현합니다.
   - 출력 계약: 요청 pixel 크기의 PNG, sRGB, 비연관 알파, 투명 바탕입니다. writer가 형식과 크기를 검사하고, 어기면
     `SvgExportError::Raster`입니다.
   - 배치: image는 경계 pixel 상자의 world 상자에 `preserveAspectRatio="none"`으로 놓습니다. PNG pixel 하나가 내보내기
     device pixel 하나입니다.
   - 구조: Content 영역은 node 그룹(clip·opacity)을 남기고 자기 내용 자리에 image를 넣습니다. Subtrees 영역은 첫 root
     id 그룹에 root 공통 clip을 걸고 image 하나를 넣습니다. Canvas 영역은 배경까지 image에 넣습니다. rasterizer는
     SVG 그룹이 이미 건 clip 앞부분(건너뛸 깊이)을 적용하지 않고, 영역 밖 조상 효과도 그리지 않습니다.
   - rasterizer가 없으면(`NoRasterizer`, 지금 제품 기본값) 사전 진단이 래스터 기능을 형식 차단으로 거부합니다. writer도
     그 기능에서 `Unsupported`로 멈춥니다. 근사하거나 조용히 빼지 않습니다.
   - 래스터 경계 안 Text는 rasterizer가 그리므로 사전 진단의 glyph outline 검사에서 뺍니다. 경계 밖 Text는 모든 glyph
     key를 조회해 outline 없는 glyph·손상 glyph를 형식 차단(`GlyphBlocker`)으로 알립니다.
3. **사전 진단과 확인(XX.1.3).** writer는 `ConfirmedExport`만 받습니다. 확인한 missing resource만 생략하고, 형식 차단은
   확인해도 진행하지 않습니다.
4. **report(XX.4.3).** `SvgDocument::report`가 보이는·vector·래스터 node 수와 래스터 영역 목록(종류, 최상위 node, pixel
   상자, node 수, 이유)을 paint 순서로 담습니다. 이유는 기능, registry CSS longhand(IX.11.20), mapping table의 SVG 한계
   설명, blend backdrop 의존입니다. `to_text`는 고정 표기입니다.
5. **text glyph 입력(XX.3.2).** export는 text·renderer crate에 의존하지 않으므로 host가 `ExportTextSource`로 고정 font
   revision의 glyph run과 outline을 줍니다. 폰트 face 식별과 variation tag는 core-types `FontFileHash`·`OpenTypeTag`를
   씁니다(같은 규칙의 export 전용 타입을 따로 두지 않음).
6. **SVG 값 표기 위치(IX.11.20).** export → document 의존을 열지 않습니다. SVG profile 표기(수, 범위 안 sRGB
   `rgb(R%,G%,B%)`, 그 밖 `color()`, 별도 `*-opacity`, `matrix(a b c d e f)`)는 `crates/export/src/svg/format.rs` 한
   곳에 둡니다. export의 모든 SVG 속성 값은 이 모듈을 거칩니다. CSS profile(`css::value`)과 같은 수·백분율·행렬 표기는
   종단 test(`tests/integration/tests/svg_registry_mapping.rs`)가 고정합니다.
   - 두 profile을 한 함수로 합치려면 수 표기를 두 crate가 함께 쓰는 하위 crate(core-types)로 옮겨야 합니다. 그 일은
     core-types·document 소유 작업이 이 결정을 근거로 합니다.

## Alternatives

- export가 renderer-core에 직접 의존해 경계를 렌더: ADR-0019·crate 그래프를 바꾸고, GPU 자원을 export crate로 끌어옵니다.
  host 연결 지점이면 Metal 작업이 끝나는 대로 같은 trait을 구현해 붙일 수 있어 탈락했습니다.
- rasterizer가 없을 때 래스터 기능을 SVG filter(`feGaussianBlur`·`feDropShadow`)로 근사: 반경 의미와 합성 색공간이 달라
  결과가 다르고, 조용한 근사는 §34 정책에 어긋나 탈락했습니다.
- Outside stroke를 even-odd clip(큰 사각형 + geometry)으로 자르기: geometry가 자기 겹침이 있는 non-zero path면 안쪽
  판정이 틀려 탈락했습니다.
- export → document 의존 허용: export가 문서 모델에 닿게 되어 Scene Snapshot만 읽는다는 경계(§8, §34)가 무너져 탈락했습니다.

## Consequences

- 지금 제품 경로에서는 래스터 기능이 있는 SVG 내보내기가 사전 진단에서 형식 차단입니다. renderer offscreen target이
  `ExportRasterizer`를 구현하면 코드 변경 없이 풀립니다.
- clip·mask·opacity·Text outline은 renderer 없이도 vector로 내보냅니다.
- 값 표기는 두 곳(CSS profile, SVG profile)에 있고 종단 test가 같음을 지킵니다. 한쪽만 바뀌면 test가 실패합니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. `write_svg` 인자는 그대로이고 `write_svg_with`가 rasterizer를 받습니다. `PreflightInput`에
  `rasterizer` field가 늘었습니다(공개 Rust API, 외부 사용처 없음).
- 기존 SVG golden(`basic_view_v1.svg`, `text_view_v1.svg`) bytes는 그대로입니다.

## Performance

- writer는 내보내기마다 래스터 계획을 한 번 세웁니다(XX.4.1, node 수에 대해 거의 선형). 래스터 기능이 없으면 경계가
  없고 출력은 이전과 같습니다.
- 사전 진단은 SVG에서 vector로 쓸 Text의 glyph key를 모두 조회합니다(같은 key는 한 번). writer도 같은 조회를 하므로
  조회 수는 최대 두 배입니다.

## Security

- rasterizer PNG는 크기를 검사한 뒤 base64로 넣고, 출력 상한(`max_output_bytes`)을 넣기 전에 검사합니다.
- host 문자열(rasterizer 오류 설명)은 오류 값에만 담고 SVG에 쓰지 않습니다.

## Migration

- 없음. 저장 데이터와 문서 형식을 바꾸지 않습니다.
