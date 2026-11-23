# 작업 보고서: XX.2.2 sRGB·Display P3 출력 profile과 metadata 기록

- WBS: XX.2.2
- 작업명: sRGB와 Display P3 output profile/metadata를 기록하고 unsupported encoder는 오류로 반환
- 상태: 완료 (`[x]`)
- 선행: XI.10.5 (`[x]`).
- 결정: ADR-0103 결정 4·5.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/png/encode.rs`(신규): RGBA image → PNG bytes, 색 정보 chunk, `pHYs`.
- `crates/export/src/png/mod.rs`: encoder 지원 표, `PngOptions`·`PngBitDepth`·`PngOptionsError`, 재노출.
- `crates/export/src/png/tests.rs`, `crates/export/src/png/tests/codec.rs`(신규): 아래 검사.

## 구현 요약

- **sRGB**: `sRGB`(perceptual) + `gAMA` 45455 + `cHRM`(sRGB primaries).
- **Display P3**: `cICP`(12, 13, 0, 1) + `iCCP`("Display P3", 생성 ICC v4.3 profile). profile colorant는 CIE xy
  primaries에서 계산하고 Bradford로 D50에 맞추며 transfer는 sRGB parametric curve입니다(core-types transfer와 같은 곡선).
  `iCCP`와 `sRGB` chunk는 함께 쓰지 않습니다(PNG 명세).
- **`pHYs`**: 배율 1 = 72 dpi. 배율 2는 5669 pixel/m(144 dpi)입니다.
- **encoder 지원 표**: {sRGB, Display P3} × 8-bit만 지원합니다. 16-bit, 다른 bit 수, 다른 profile 이름(`rec2020` 등)은
  `PngOptionsError`이며 조용히 sRGB·8-bit로 바꾸지 않습니다.
- encode는 결정론적입니다(`tIME`·텍스트 chunk 없음, 고정 filter 선택 규칙, 결정론적 압축).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --offline --lib png` | 통과 |
| `sips -g space -g profile -g dpiWidth`(macOS ImageIO로 golden PNG 읽기) | sRGB 파일은 `sRGB IEC61966-2.1`, P3 파일은 `Display P3`, dpi 72·108·144 |

| 검사(테스트) | 결과 |
|---|---|
| sRGB·P3 encode → decode 무손실, bytes 결정론, `pHYs`, `tIME` 없음(`encode_decode_round_trip_is_lossless_and_deterministic`) | 통과 |
| sRGB chunk 구성(IHDR·sRGB·gAMA·cHRM·pHYs·IDAT·IEND), P3의 `cICP` 값과 `iCCP` profile이 P3로 판정, ICC만 있는 경우와 BT.2020 `cICP` 거부(`srgb_and_display_p3_metadata_are_recorded`) | 통과 |
| `rec2020`·16-bit·12-bit 요청은 오류(`unsupported_encoder_options_are_errors`) | 통과 |
| 참조 구현이 만든 16-bit gray+tRNS·4-bit palette+tRNS·Sub/Up filter RGB 해석(`decodes_reference_png_variants`) | 통과 |
| 손상 CRC·잘림·크기 상한·interlace 거부, 모든 byte 변형 panic 없음(`decoder_rejects_corrupt_and_unsupported_input`) | 통과 |
| ICC: 생성 profile 구조(`acsp`, v4.3, `mntr`, 크기 field, TRC 공유)와 colorant가 Apple Display P3 공개 값과 0.001 이내(`icc` 단위 test) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 16-bit·HDR·wide gamut(Rec. 2020) 출력은 지원 표 밖입니다(아키텍처 §10.3 초기 범위).
- 생성 ICC profile의 profile ID(MD5)는 0입니다(ICC 명세상 선택). ColorSync는 이 profile을 Display P3로 읽습니다.
