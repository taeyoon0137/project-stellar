# 작업 보고서: XX.2.4 decoded pixel golden test

- WBS: XX.2.4
- 작업명: 동일 snapshot/options의 decoded pixel output이 양 플랫폼 허용 오차 내 일치하는 golden test
- 상태: 부분 완료 (`[~]`) — macOS golden 비교는 끝났습니다. Windows는 보류(ADR-0006)라 양 플랫폼 비교는 하지 않았고, 선행
  XX.2.3이 `[~]`입니다(계획 1.2 규칙 4).
- 선행: XX.2.3 (`[~]`).
- 결정: ADR-0103 결정 7.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/png/tests/golden.rs`(신규): golden 비교와 갱신(`STELLAR_PNG_BLESS=1`).
- `crates/export/src/png/tests.rs`: golden fixture scene(`showcase_roots`)과 target.
- `crates/export/tests/fixtures/png/{showcase_srgb_1x,showcase_p3_2x,basic_srgb_1_5x_background}.png`(신규). 바이너리 golden은 `src` 밖에 둡니다(MCP no_network test가 의존 crate `src`를 UTF-8 text로 읽음).

## 구현 요약

- fixture 셋: 둥근 clip Frame 안 opacity 그룹·테두리·곡선 path stroke·Linear 이미지·text glyph(sRGB 1배, Display P3 2배),
  basic fixture(배경색, 1.5배).
- 비교: 출력 PNG와 golden PNG를 같은 decoder로 해석해 크기·chunk 구성·색 정보는 정확히, pixel은 채널마다 ±1(8-bit) 안이어야
  합니다. 초과 pixel 수와 처음 위치 16개를 실패 메시지에 냅니다. 같은 입력은 bytes도 같아야 합니다.
- 허용 오차 ±1은 transfer 함수 `powf`의 플랫폼 libm 차이를 덮는 값이고, CPU target 계산은 그 밖에 플랫폼 독립입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `STELLAR_PNG_BLESS=1 cargo test -p stellar-export --offline --lib png`(golden 생성) | 통과, 파일 3개 |
| `cargo test -p stellar-export --offline --lib png`(비교) | 통과 |
| `sips -g pixelWidth -g pixelHeight -g space -g profile`(macOS ImageIO 독립 해석) | 48×32 sRGB, 96×64 Display P3, 150×120 sRGB |
| golden 확대 이미지 눈 검사(scratchpad `png/big_*.png`) | clip·opacity·stroke·image·text가 의도대로 보임 |

| 검사(테스트) | 결과 |
|---|---|
| 세 fixture의 decoded pixel·chunk·색 정보가 golden과 일치, 같은 입력 bytes 같음(`decoded_pixels_match_golden_within_tolerance`) | 통과 |
| fixture가 두 profile과 세 배율을 덮음(`golden_cases_cover_both_profiles_and_scales`) | 통과 |
| 비교 함수가 ±1은 통과시키고 9 차이는 위치 (1, 1)로 찾음(`tolerance_comparison_finds_changed_pixels`) | 통과 |

## 차이(작업 지시와 다른 점)

- "양 플랫폼": Windows가 보류(ADR-0006)라 macOS만 실행했습니다.

## 남은 위험 / 한계

- GPU(Metal) target이 생기면 같은 golden으로 CPU·GPU 차이를 비교해야 합니다(허용 오차를 그때 다시 정함).
- golden은 CPU target 결과로 만들었으므로 렌더 공식(XI.10.x)이 바뀌면 golden을 다시 만들고 차이를 검토합니다.
