# 작업 보고서: XX.2.4 decoded pixel golden test

- WBS: XX.2.4
- 작업명: 동일 snapshot/options의 decoded pixel output이 양 플랫폼 허용 오차 내 일치하는 golden test
- 상태: 구현 완료 (`[x]` 후보, marker는 통합 때 정합니다) — CPU target golden(이전 작업)에 더해, 남은 범위였던 GPU(Metal)
  target과 CPU target의 decoded pixel 대조를 더했습니다. Windows는 보류(ADR-0006)라 macOS의 두 target을 비교합니다.
- 선행: XX.2.3 (이 lane에서 구현, marker는 통합 때 판정).
- 결정: ADR-0103 결정 7, ADR-0116 결정 4.

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, headless Metal).

## 변경 파일 목록

이전 작업(CPU golden): `crates/export/src/png/tests/golden.rs`, `crates/export/src/png/tests.rs`,
`crates/export/tests/fixtures/png/{showcase_srgb_1x,showcase_p3_2x,basic_srgb_1_5x_background}.png`.

이 lane(CPU·Metal 대조):

- `crates/renderer-metal/src/tests/png_golden.rs`(신규), `crates/renderer-metal/src/tests/mod.rs`.
- 공용 fixture(scene·asset·glyph·확인한 내보내기)는 `crates/renderer-metal/src/tests/offscreen_target.rs`(XX.2.1 커밋)에 있습니다.

## 구현 요약

- 같은 snapshot·옵션으로 CPU target(`SoftwareTarget`)과 Metal target(`MetalOffscreenTarget`)이 `render_png`로 PNG를 만들고, 같은
  decoder로 해석한 pixel을 연관 알파 값(encode 값 × 알파)으로 비교합니다. 색 정보 chunk는 정확히 같아야 합니다.
- 허용 오차: 내부 pixel ±2. 가장자리 pixel은 경계를 공유하지 않는 scene 40(축 8 sample과 CPU 16 sub-scanline의 coverage 차이),
  경계를 공유하는 scene(Frame 배경과 안쪽 테두리, fill과 stroke) 128(CPU coverage 합성의 공유 경계 번짐: 알파와 아래 색 비침 각각
  1/4 이하). 내부 허용치를 넘는 pixel은 12% 이하.
- 경우: 두 scene(경계 공유 없음·있음) × {sRGB 1배, Display P3 2배, sRGB 1.5배 배경}, 그리고 배율 12(tile 경계 여럿) 결정론.
- 결과 PNG(CPU·Metal)는 `target/render-conformance/png-export/`에 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal --lib png_golden` | 2 passed |
| `cargo test -p stellar-renderer-metal` | 67 passed |
| `cargo test -p stellar-export --lib png` | 통과(이전 CPU golden 그대로) |

| 검사(테스트) | 결과 |
|---|---|
| 6개 경우(2 scene × 3 옵션) CPU·Metal decoded pixel이 허용 오차 안, 색 정보 같음(`metal_target_matches_cpu_target_on_shared_scenes`) | 통과 |
| 배율 12에서 Metal 두 번 같은 pixel, CPU와 허용 오차 안(`metal_target_is_deterministic_and_tiles_without_seams`) | 통과 |
| 분석: 둥근 모서리(배율 12, x = 68)에서 정확한 coverage 0.889(알파 227)에 CPU 223, 4 sample이던 첫 구현은 255라 sample을 축 8배로 올림 | 확인 |

## 차이(작업 지시와 다른 점)

- "양 플랫폼": Windows가 보류(ADR-0006)라 같은 macOS의 CPU target과 GPU target을 대조했습니다.
- 경계를 공유하는 그리기의 가장자리 허용치(128)가 큽니다. CPU target의 coverage 합성 성질(공유 경계 번짐) 때문이고, 경계를
  공유하지 않는 scene은 40으로 따로 검사합니다.

## 남은 위험 / 한계

- CPU target의 공유 경계 번짐을 없애려면 CPU target이 sample 합성이나 합집합 coverage를 써야 합니다(export raster 소유 후속). 그
  뒤 공유 경계 허용치를 줄입니다.
- shadow·layer blur·blend는 CPU target이 그리지 않아 두 target 대조가 없습니다. 대신 Metal target을 같은 그리기 목록의 CPU 기준과
  ±3으로 비교합니다(XX.2.1 `metal_target_draws_effects_like_the_cpu_reference`).
- 저장 golden PNG는 CPU target 결과뿐입니다. GPU 결과는 매번 CPU 결과와 비교하므로 따로 저장하지 않았습니다.

## 후속: 효과 대조 갱신 (lane render-shadow-quality, ADR-0130)

- CPU target은 여전히 shadow·blur를 그리지 않으므로 효과 대조는 같은 그리기 목록의 CPU 기준과 합니다. 그 기준 그리기는
  fixture 크기 상한(512)을 넘을 수 있어 canvas에 직접 그립니다(`cpu_reference_of`). 큰 blur 장면은 이제 배수 8로 그려
  CPU 기준과 연관 값 ±4 안입니다(XX.2.1 후속 절). 공유 scene golden(`metal_target_matches_cpu_target_on_shared_scenes`)과 결정론
  test는 바뀌지 않았고 통과합니다.
