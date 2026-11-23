# 작업 보고서: XI.10.1 선형 working space 변환과 target별 출력 encode 공통 수식

- WBS: XI.10.1
- 작업명: shader 입력 색을 linear working space로 변환하고 output target에 맞춰 encode하는 공통 수식을 문서화·구현
- 상태: 완료 (`[x]`)
- 선행: III.4.2 (`[x]`), XI.8.4 (`[x]`)
- 결정 기록: ADR-0102 결정 1·2

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/color.rs`(신규) — `WorkingColor`, `to_working`, `image_texel`, `OutputEncoding`, `output_encoding`,
  `encode_stored`, `encode_pixel`, `decode_pixel`, `quantize_unorm8`, half float 변환, `ColorSpace` 재노출.
- `crates/renderer-core/src/color/tests.rs`(신규) — 수식 단위 테스트.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `crates/renderer-metal/src/surface.rs`, `sys.rs` — layer colorspace를 format 저장 방식으로 고름(`Bgra8Unorm` → 선형 colorspace).
- `crates/renderer-metal/src/shaders.rs` — shader ABI 색 규칙 문서.
- `crates/renderer-metal/src/tests/color.rs`(신규), `tests/mod.rs` — GPU pixel 비교 테스트.
- `docs/adr/ADR-0102-render-color-compositing-effects-conformance.md`(신규).

## 구현 요약

- working 값 = surface primaries 선형광 premultiplied RGBA. 문서·scene 색과 이미지 texel은 `OutputColorPlan::convert`(XI.10.5) 한 곳을
  거쳐 working 값이 됩니다. transfer 곡선은 core-types `ColorSpace`의 함수(III.4.2)를 그대로 씁니다.
- 출력 encode는 format이 정합니다. `*UnormSrgb`는 premultiplied 선형 채널마다 하드웨어 encode, `*Unorm`은 선형 그대로(자름), `Rgba16Float`는
  선형 half(자르지 않음), `R8Unorm`은 coverage입니다. 8-bit 양자화 기준은 `round(v × 255)`입니다.
- `Bgra8Unorm` fallback drawable 미결(XI.10.6 문서): 선형 저장 format으로 정하고 Metal layer에 `kCGColorSpaceLinearSRGB`·
  `kCGColorSpaceLinearDisplayP3`를 붙입니다. shader는 format마다 encode하지 않으므로 blend는 언제나 선형입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 172 passed(신규 8), doctest 3 passed |
| `cargo test -p stellar-renderer-metal` | 57 passed(신규 3) |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| sRGB·P3 surface × `Rgba8UnormSrgb`·`Bgra8UnormSrgb`·`Rgba8Unorm`·`Rgba16Float`에 sRGB·P3·반투명·어두운 값·gamut 밖 P3 색 8개를 그린 GPU pixel이 CPU `encode_pixel`과 8-bit 1 LSB, half 2e-3 안(`solid_colors_match_cpu_reference_for_every_output_encoding`) | 통과 |
| clear 값과 sRGB·선형·half target의 premultiplied over blend가 선형 CPU 식과 일치(`clear_values_and_linear_blending_match_cpu_reference`) | 통과 |
| format·색공간별 layer colorspace 대응(`layer_colorspace_follows_format_storage`) | 통과 |
| premultiplied 변환, 이미지 texel과 단색의 같은 경로, format 표, premultiplied 채널 encode, round-trip, 양자화, half float IEEE 값 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. P3 surface의 실제 창 표시(셸 layer)는 UI 재설계 전이라 offscreen texture로만 확인했습니다.

## 남은 위험 / 한계

- `crates/renderer-core/src/output_color.rs` 모듈 문서의 "미결(XI.10.1)" 문단은 이 결정으로 풀렸지만 lane 소유 밖이라 고치지 않았습니다.
- 선형 8-bit `Bgra8Unorm` 표시는 어두운 영역 계조가 거칩니다. 표시 기본값은 `Bgra8UnormSrgb`입니다.
