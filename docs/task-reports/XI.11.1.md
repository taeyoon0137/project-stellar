# 작업 보고서: XI.11.1 common fixture offscreen render와 raw RGBA reference harness (Metal)

- WBS: XI.11.1
- 작업명: common scene fixture를 offscreen render해 PNG-like raw RGBA reference를 생성하는 테스트 harness를 Metal backend에 구현
- 상태: 완료 (`[x]`)
- 선행: XI.8.5 (`[x]`)
- 결정 기록: ADR-0102 결정 6

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증). D3D12 harness는 Windows 재개 때 XI.9 행과 함께 더합니다(ADR-0006 결정 4).

## 변경 파일 목록

- `crates/renderer-core/src/effects/mod.rs`(신규) — 모듈 등록.
- `crates/renderer-core/src/effects/abi.rs`(신규) — 공통 shader ABI byte 배치(정점·`DrawUniforms`·`BlurUniforms`).
- `crates/renderer-core/src/effects/fixture.rs`(신규) — backend 중립 fixture 모델(`Fixture`, `Op`, `Mesh`, `ImageData`, `CoverageMask`,
  분류·기능, path 작성 도우미, `FillRule`·`StrokeCap` 재노출).
- `crates/renderer-core/src/effects/fill.rs`(신규) — `SweepFillTessellator`(`FillTessellator` 구현, 가로 band 사다리꼴 분해).
- `crates/renderer-core/src/effects/raster.rs`(신규) — CPU 기준 rasterizer(pixel 중심·top-left 규칙).
- `crates/renderer-core/src/effects/lower.rs`(신규) — fixture → graph + draw command(`RendererBackend` generic, resource 생성·해제).
- `crates/renderer-core/src/effects/artifact.rs`(신규) — `RgbaImage`, raw RGBA(`STRGBA1`)·PNG 기록.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `crates/renderer-metal/src/shaders.rs` — ABI 상수를 공통 `effects::abi`에서 가져옴(사본 제거), `DrawUniforms` 필드 이름.
- `crates/renderer-metal/src/tests/conformance.rs`(신규), `tests/mod.rs` — `MetalHarness`와 harness 테스트.

## 구현 요약

- fixture는 크기·surface 색공간(capability 모사)·target format(`Rgba8UnormSrgb`·`Rgba8Unorm`)·clear 색·그리기 목록입니다. 색은 문서 값으로
  적고 CPU 기준과 lowering 모두 `color::to_working`으로 working 값을 얻습니다.
- lowering은 backend에 정점·uniform buffer와 이미지·glyph texture를 만들고, external target 하나를 그리는 graph와 pass command를 만듭니다.
  겹치는 stroke mesh는 stencil union(`IncrementWhereEqual` → `ClipStencil` + `DecrementWhereEqual`)으로 pixel마다 한 번만 칠합니다.
- `MetalHarness`는 headless Metal에서 encode·submit·`read_texture`를 하고 `RgbaImage`를 돌려줍니다. `write_reference`가 `<name>.rgba`와
  `<name>.png`를 `STELLAR_RENDER_CONFORMANCE_OUT`(없으면 `target/render-conformance`) 아래에 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 통과(effects 신규 13) |
| `cargo test -p stellar-renderer-metal` | 58 passed(신규 1) |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 단색 rect·evenodd 별·반투명 stroke union·이미지 nearest·P3 glyph fixture를 두 번 그려 byte가 같고, raw 파일을 다시 읽어 같은 image이며 PNG가 생김. CPU 기준과 1 LSB를 넘는 pixel은 크기의 1/64 이하(`harness_renders_common_fixtures_to_deterministic_raw_references`) | 통과 |
| mock backend lowering: resource 생성, union mesh의 stencil attachment, draw 3개, encode·submit·해제(`lowering_creates_resources_and_valid_frame`), 잘못된 fixture는 backend 호출 전 거부 | 통과 |
| sweep tessellator 면적(사각형·오목·별 nonzero/evenodd·겹친 사각형·반대 방향 구멍·cubic 원), top-left 규칙 공유 변 한 번 칠함, union 한 번, bilinear sampling, raw round-trip·PNG CRC/Adler | 통과 |

## 차이(작업 지시와 다른 점)

- "common scene fixture"는 `SceneSnapshot`이 아니라 backend 중립 그리기 목록입니다. scene → draw command 제품 frame encoder가 없고
  renderer-core가 core-types 타입을 만들 수 없기 때문입니다(ADR-0102 Alternatives). 제품 frame encoder가 생기면 scene fixture를 이 그리기
  목록으로 lowering합니다.

## 남은 위험 / 한계

- `effects::fill`의 cubic 평탄화는 stroke tessellator 내부 평탄화와 별도입니다(ADR-0102 Consequences 후속).
- PNG는 저장 byte(premultiplied)를 그대로 담습니다. 반투명 pixel의 보이는 색은 straight alpha 해석과 다릅니다.
