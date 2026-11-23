# 작업 보고서: XX.3.2 Text glyph run의 outline Path 변환

- WBS: XX.3.2
- 작업명: 모든 Text glyph run을 outline Path로 변환 — `<text>` 요소와 font embedding을 만들지 않음
- 상태: 완료 (`[x]`)
- 선행: X.9.6 (`[x]`), XI.6.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. 외부 확인에 macOS Quick Look(WebKit) 렌더 사용).

## 변경 파일 목록

- `crates/export/src/text.rs`(신규) — glyph 입력 값 타입(`ExportFontFace`, `GlyphSize`, `AxisTag`, `GlyphVariations`,
  `GlyphOutlineKey`, `PlacedGlyph`, `GlyphRun`, `TextGlyphLayout`, `GlyphOutline`, `GlyphOutlineError`), host 조회 trait
  `ExportTextSource`, text 없는 source `NoText`.
- `crates/export/src/svg/text.rs`(신규) — glyph run → outline path 변환(`text_outlines`, `TextOutlines`, `RunOutline`,
  `TextOutlineError`).
- `crates/export/src/capture.rs` — 고정 font revision 조회(`ExportCapture::glyph_layout`, `glyph_outline`).
- `crates/export/src/svg/writer.rs` — `write_svg`에 text source 인자, Text content를 outline으로 기록, glyph outline
  cache, 오류 `MissingTextLayout`·`Text`.
- `crates/export/src/svg/mapping.rs` — `TextContent` row를 vector로 바꾸고 대응 SVG 표기 갱신, 관련 테스트 갱신.
- `crates/export/src/svg/element.rs`(문서 주석), `crates/export/src/svg/mod.rs`, `crates/export/src/lib.rs`(모듈·재노출).
- `crates/export/src/svg/writer/tests.rs` — 새 인자 반영, Text를 미지원 사례에서 제외, text 테스트 모듈 연결.
- `crates/export/src/svg/writer/tests/text.rs`(신규) — 합성 폰트 fixture, golden·외형 테스트, 테스트 rasterizer.
- `crates/export/tests/fixtures/text_view_v1.svg`(신규) — Text View golden SVG.
- `crates/export/tests/fixtures/text_view_v1.coverage.txt`(신규) — 외형 golden(pixel 중심 coverage 격자).
- `docs/task-reports/XX.3.2.md`(신규), 계획서(XX.3.2 `[ ]`→`[x]`).

## 구현 요약

- **입력**: scene Text의 `TextLayoutKey`를 capture의 **고정 font revision**으로 host(`ExportTextSource`)에 물어 glyph run을
  얻습니다. 고정 revision에 없으면 다른 revision으로 대체하지 않고 `MissingTextLayout`입니다.
- **glyph outline key**: 폰트 face(파일 SHA-256 + face index) + glyph id + 크기(1/64 px 양자화, ≤ 4096 px) + variation
  좌표(tag 오름차순·마지막 값 우선·`-0`→`0`). renderer glyph key(XI.6.4)의 Outline mode와 같은 의미의 값이라 host가 renderer
  outline rasterizer에 위임할 수 있습니다. variation은 shaping에 쓴 run 좌표(X.9.6)를 그대로 써서 variable font outline이
  shaping instance와 같습니다.
- **변환**: glyph outline(pen 원점 기준 px, y 아래)을 pen 원점(text local 1/64 px)만큼 옮겨 run마다 path 하나로 잇습니다.
  run 고유 색(document text style fill)이 있으면 `<path d fill>`(non-zero)로 쓰고, 모든 run을 이은 path를 paint layer
  (Fill·Stroke modifier) 기하로 그 위에 칠합니다(scene builder paint 순서). 공백 glyph는 건너뜁니다.
- **거부**: outline이 없는 glyph(bitmap·color 전용)는 `TextOutlineError::NoOutline`, 폰트 없음·손상은 `Outline`, 좌표
  비유한은 `NonFinite`로 멈춥니다(조용히 빼지 않음). 남은 출력 예산의 절반을 path command 상한으로 써서 거대한 text가
  메모리를 먼저 쓰지 않고 `OutputTooLarge`가 됩니다.
- **결정론**: 같은 key outline은 한 번만 조회합니다(cache는 조회 전용, 출력 순서는 run·glyph 순서).
- `<text>`·`<tspan>`·SVG font·`@font-face`·font data URI·`<style>`을 만드는 코드가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 83 passed, 0 failed (이 작업 신규 13) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |
| golden SVG에서 paint layer path를 뺀 문서를 `qlmanage -t`(Quick Look, WebKit)로 렌더해 coverage golden과 pixel 중심 비교 | 경계 인접 cell을 뺀 3,284 cell 모두 일치(불일치 0) |

| 검사 | 결과 |
|---|---|
| **`<text` 문자열 0**: run 5개(폰트 2, 크기 20·16 px, `wght` 700·300, P3 색, 색 없는 run)와 반투명 Fill·Center Stroke modifier를 가진 Text의 SVG가 golden과 byte 동일, `<text`·`<tspan`·`<font`·`<glyph`·`@font-face`·`font-family`·`data:font`·`font/`·`<style` 0, path 6개(run 색 4 + layer 2), layer 기하에 색 없는 run 포함(`text_view_matches_golden_without_text_elements`) | 통과 |
| **외형 golden**: paint layer 없는 같은 Text의 SVG를 테스트 rasterizer(pixel 중심 non-zero point sampling)로 칠한 결과가 writer를 거치지 않고 glyph 모양 정의에서 직접 칠한 reference와 같고 coverage golden과 같음, 고리 glyph 구멍 비칠함, `wght` 700 막대가 300보다 넓음(`text_outlines_render_like_the_glyph_shapes`) | 통과 |
| outline key가 폰트·glyph·크기(1280/1024 = 20/16 px)·variation을 담고 같은 key는 한 번만 조회(`outline_keys_carry_font_size_and_variations`) | 통과 |
| font revision 1·2에 다른 layout이 있을 때 고정 revision 결과만 사용, 고정 revision에 없으면 `MissingTextLayout`(`text_reads_the_pinned_font_revision`) | 통과 |
| 같은 snapshot 재기록·새 source 모두 byte 동일(`same_text_snapshot_is_byte_identical`) | 통과 |
| 없는 폰트 → `Text { Outline { FontUnavailable } }`, color 전용 glyph → `Text { NoOutline }`, text source 없음 → `MissingTextLayout`(`unavailable_fonts_and_color_glyphs_are_errors`) | 통과 |
| 출력 상한(64·400·전체−1 byte 거부, 전체 길이 허용)(`text_output_respects_the_size_limit`) | 통과 |
| 변환 단위: pen 원점 이동 정확, combined = run path 연결, 폰트 없음·NoOutline·command 상한·먼 좌표, key의 run instance(`svg/text.rs` 3건) | 통과 |
| 값 타입: 크기 양자화·범위, variation 정규형·상한·tag 규칙(`text.rs` 3건) | 통과 |
| mapping inventory(TextContent vector), 기존 writer 테스트(basic golden 포함) | 통과 |

## 차이(작업 지시와 다른 점)

- glyph run·outline은 text crate(X.10)와 renderer(XI.6.4)가 갖지만 export는 crate 의존 정책(ADR-0019)상 그 crate에 의존할 수
  없어, 같은 의미의 값 타입과 host 조회 trait(`ExportTextSource`)을 export에 두었습니다. 실제 text layout cache·font registry를
  이 trait에 연결하는 일은 export command(XX.5.1)의 범위입니다.
- 저장소에 외곽선 있는 폰트 파일이 없어(text 테스트 폰트에 `glyf` 없음) fixture는 코드로 정의한 합성 glyph 모양입니다.
- "외형 golden"은 테스트 rasterizer와 coverage golden으로 고정했고, 외부 렌더러 확인은 Quick Look(WebKit) 수동 비교 1회입니다.
  browser render fixture 자동화는 XX.3.4의 범위입니다.
- `write_svg`에 text source 인자를 추가했습니다(XX.3.3 API 변경). text가 없으면 `NoText`를 넘깁니다.

## 남은 위험 / 한계

- outline이 없는 color glyph(emoji bitmap)는 지금 내보낼 수 없습니다. 부분 래스터화(XX.4.2)가 이 glyph를 래스터 경계로
  다뤄야 합니다.
- 내보내기 영역(capture bounds)은 scene bounds(layout 상자 + stroke)라 layout 상자 밖으로 넘친 glyph는 viewBox에서 잘립니다.
  glyph ink bounds를 영역에 넣으려면 scene bounds 계약(XI.3.1)과 함께 정해야 합니다.
- outline key 크기 상한은 renderer key와 같은 4096 px이고, 그보다 큰 run(text engine 상한 10,000 px까지)은 상한 크기
  outline을 배율로 키웁니다(후속 수정 1). host outline이 em 크기에 선형(hinting 없음)이라는 가정에 기댑니다.
- glyph outline 좌표는 f32입니다. pen 원점 |값| ≥ 2²⁴ subpixel(262,144 px)이면 1/64 px 정밀도를 잃습니다.

## 후속 수정 (리뷰 반영)

### 1. 4096 px를 넘는 Text도 outline으로 기록

`Fix: 4096 px를 넘는 glyph run을 상한 크기 outline 배율로 SVG에 기록` 커밋에서 고쳤습니다.

- **문제**: run 크기를 outline key 크기 타입(`GlyphSize`, renderer key 상한 4096 px)으로 받아, text engine이 허용하는
  (4096, 10000] px Text는 glyph run을 만들 수 없었습니다. 행 지시 "모든 Text glyph run"과 맞지 않았습니다.
- **변경**: run 크기 타입 `RunSize`(`(0, MAX_RUN_PX = 10000]` px, 1/64 px)를 두고 `GlyphRun.size`를 이 타입으로 바꿨습니다.
  outline key 크기는 `RunSize::outline_size`(4096 px에서 자름)이고, 자른 만큼은 `RunSize::outline_scale`(run ÷ 4096)을
  outline 좌표에 곱합니다. Outline mode outline은 bitmap atlas를 쓰지 않고 em 크기에 선형이라 배율 결과가 run 크기 outline과
  같습니다. 4096 px 이하 run의 배율은 정확히 1이라 기존 출력이 bit 단위로 같습니다(golden 그대로).
- **검증**: `runs_above_the_key_limit_scale_the_capped_outline`(8192 px와 10,000 px run이 4096 px key 하나로 조회하고, 결과가
  run 크기로 직접 만든 사각형과 같음), `run_sizes_reach_the_text_engine_limit`(범위·양자화·배율). 수정 전에는
  `GlyphSize::from_px(8192.0)`가 `InvalidSize`라 run을 만들 수 없었습니다(`size_quantizes_like_the_renderer_key`가 확인하는
  기존 동작).

| 명령(후속 수정 1) | 결과 |
|---|---|
| `cargo test -p stellar-export` | 93 passed, 0 failed |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-export -- --check` | 통과 |

### 2. glyph pen 원점에 layout 상자 원점 반영

`Fix: Text glyph를 layout 상자 원점 기준으로 배치` 커밋에서 고쳤습니다.

- **문제**: glyph pen 원점은 layout 상자 좌상단 기준(`crate::text` 문서 규약)인데 writer는 node world transform만 적용하고
  `TextPrimitive.bounds`(node local layout 상자)의 원점을 더하지 않았습니다. scene builder는 원점을 항상 0으로 만들어 겉으로
  드러나지 않았지만, codec이나 직접 만든 primitive는 원점이 0이 아닐 수 있습니다. Image는 `bounds`를 그대로 써서 두 content의
  규약도 달랐습니다.
- **변경**: `text_outlines`가 layout 상자 원점(`origin`)을 받아 glyph 이동량에 더합니다. writer는 `primitive.bounds.origin()`을
  넘깁니다. 원점 0이면 `0 + pen`이 pen과 정확히 같아 출력이 바뀌지 않습니다.
- **검증**: `glyphs_start_at_the_layout_box_origin`을 먼저 추가해 수정 전 실패(glyph가 원점 0 위치 "M87 30 …"에 남음)를
  확인했습니다. 수정 후 상자 원점 (5, 3)이면 glyph가 그만큼 옮겨지고("M92 33 …"), 원점 0 상자는 golden과 byte 단위로
  같습니다. 변환 단위 테스트(`glyphs_move_to_their_pen_origin`)에도 원점 이동 case를 더했습니다.

| 명령(후속 수정 2) | 결과 |
|---|---|
| `cargo test -p stellar-export glyphs_start_at`(수정 전) | 1 failed |
| `cargo test -p stellar-export` | 94 passed, 0 failed |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-export -- --check` | 통과 |
