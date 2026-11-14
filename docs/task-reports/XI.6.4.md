# 작업 보고서: XI.6.4 glyph outline/bitmap atlas interface와 key

- WBS: XI.6.4
- 작업명: glyph outline/bitmap atlas interface와 key(font hash, glyph id, size, variation, rendering mode) 정의
- 상태: 완료 (`[x]`)
- 선행: X.10.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/glyph/mod.rs`(신규) — `GlyphKey`(`FontHash`, `GlyphId`, `GlyphSize`, `AxisTag`,
  `VariationCoords`, `GlyphRenderMode`), `GlyphBitmap`·`GlyphBitmapFormat`, `GlyphImage`, `GlyphRasterizer`
  trait, `check_image_matches_mode`, `AtlasPageId`·`AtlasPlacement`·`AtlasError`, `GlyphAtlas` trait, 상한 상수,
  `GlyphError`.
- `crates/renderer-core/src/glyph/tests.rs`(신규) — key collision fixture와 interface 테스트.
- `crates/renderer-core/src/lib.rs` — `glyph` 모듈 등록.
- `docs/task-reports/XI.6.4.md`(신규), 계획서(XI.6.4 `[ ]`→`[x]`).

## 구현 요약

- **key**: `GlyphKey = FontHash(파일 SHA-256 + face index) + GlyphId(u32) + GlyphSize(1/64 px 양자화 em 크기,
  device scale 포함) + VariationCoords(tag 오름차순·마지막 값 우선·-0.0 정규화) + GlyphRenderMode(Outline·
  Coverage·Color)`. text crate의 `FontFileHash`·face index·`GlyphId`·`canonical_variations`(X.8.1·X.9.6·X.10.5)와
  같은 의미이며, crate 의존 정책상 renderer-core는 text를 import할 수 없어 값 타입을 따로 둡니다. 경로·family
  이름은 key가 아닙니다.
- **canonical bytes**: version + 고정 길이 field + axis 수(u16) prefix + (tag, value bits)* 로 단사입니다.
  `digest`(FNV-1a 64)는 로그·fixture용이고 동등성은 field 비교입니다. `same_glyph_shape`는 크기만 다른 같은
  glyph를 판정합니다(XI.6.6 대체 정책 입력).
- **interface**: worker용 `GlyphRasterizer::rasterize(&GlyphKey) -> GlyphImage{Empty | Bitmap | Outline}`(결정론·무
  panic 계약, mode 일치 검사 함수), `GlyphBitmap`(Coverage8 = R8 선형 coverage, Rgba8Premultiplied = sRGB 인코딩
  premultiplied, 변 ≤ 1024, 길이 검증), `GlyphAtlas` trait(`begin_frame`·`gpu_completed`·`lookup`·`insert`·
  `is_valid`)과 page 세대를 담은 `AtlasPlacement`.
- **신뢰되지 않은 입력**: 크기는 유한한 (0, 4096] px이며 양자화 결과 0을 거부합니다. axis는 최대 64개이고
  같은 tag 반복 입력도 256개에서 멈춥니다. tag는 출력 가능한 ASCII만, variation 값은 유한값만 허용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 118 unit passed(신규 5) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 폰트 5(face·digest 1 byte 차이 포함) × glyph 45(경계값 포함) × 크기 8(1/64 px 차이 포함) × variation 6(tag 경계 교차 조합 포함) × mode 3 = 32,400 key: key·canonical bytes·FNV digest 충돌 0(`key_fixture_has_no_collisions`) | 통과 |
| variation 순서·중복·-0.0 정규화로 같은 key, 1/64 px 미만 차이 같은 크기, `same_glyph_shape`(`equivalent_inputs_share_one_key`) | 통과 |
| canonical bytes 고정 배치 golden(`canonical_bytes_layout_is_stable`) | 통과 |
| 크기·tag·variation·axis 수·bitmap 길이 거부, 무한 반복 입력 상한(`untrusted_values_are_rejected`) | 통과 |
| rasterizer 출력과 mode 일치 검사, Empty 허용(`rasterizer_output_must_match_mode`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 font outline 추출·rasterization(폰트 parser)은 이 작업 범위가 아니어서 trait만 정의하고 테스트용
  rasterizer로 검증했습니다.
- subpixel 위치(가로 1/4 px bin 등)는 key에 넣지 않았습니다. 필요하면 rendering mode 확장으로 추가합니다.

## 남은 위험 / 한계

- Outline mode도 크기를 key에 포함합니다(tessellation 허용 오차가 크기에 따라 달라짐). 크기 구간 공유는 cache
  예산 작업(XVI.5)에서 판단합니다.
- 계산은 정수와 f32 `round`만 써서 ADR-0006 판정 규칙상 플랫폼 간 동일 구성입니다.
