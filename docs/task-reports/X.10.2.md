# 작업 보고서: X.10.2 줄 높이·paragraph 간격·정렬

- WBS: X.10.2
- 작업명: line height, paragraph spacing, alignment 적용(text box 세로 정렬은 분리)
- 상태: 완료 (`[x]`)
- 선행: X.10.1(이 lane `[x]`), VI.7.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/text_layout.rs`(신규) — `lay_out_paragraph`(줄 나누기 + BiDi 줄 재배치 + 줄 높이·baseline +
  visual glyph 배치), `compose_text`(paragraph 쌓기·간격·가로 정렬·양쪽 정렬·text box 세로 정렬),
  `TextAlign`, `LineHeight`, `LineHeightRatio`, `ParagraphLayoutStyle`, `VerticalAlign`, `TextBox`,
  `ParagraphLines`, `ParagraphLine`, `LineGlyph`, `TextLayout`, `PositionedLine`, `PlacedGlyph`.
- `crates/text/src/paragraph.rs` — `ShapedParagraph::default_font_size`(빈 줄 높이), 테스트 fixture에 히브리어
  폰트 추가.
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.2.md`(신규), 계획서(X.10.2 `[ ]`→`[x]`).

## 구현 요약

- **2단계**: `lay_out_paragraph`는 paragraph 단위(캐시 가능) 결과로 줄마다 BiDi visual 순서 glyph(줄 시작 기준
  x)와 줄 높이·baseline을 정하고, `compose_text`는 text box 단위로 paragraph를 쌓고 정렬합니다. 내용 너비가
  다른 paragraph에 따라 정해지는 auto 폭 box에서도 정렬이 맞게 정렬은 2단계에서 합니다.
- **줄 높이**(VI.7.3 `LineHeight`): Auto = 줄 안 run의 최대 ascent + 최대 descent + 최대 line gap(gap은
  half-leading으로 위아래 반씩), Multiplier = 줄 최대 글꼴 크기 × 1/1024 정수 배율(반올림), Fixed = 고정값.
  Multiplier·Fixed는 남거나 모자라는 공간을 위아래로 반씩(floor) 나눕니다. 빈 줄은 paragraph 기본 metric.
- **paragraph 간격**: paragraph 사이에만 넣습니다.
- **가로 정렬**: paragraph 기준 방향 기준 Start/Center/End, Justify는 마지막 줄·강제 줄바꿈 줄 제외, 줄 안 공백
  segment(끝 hanging 공백 제외)에 남는 폭을 정수로 나누고 나머지 subpixel은 visual 앞쪽 공백부터 1씩.
- **text box 세로 정렬**(가로 정렬과 별개): Top/Middle/Bottom. 내용이 box보다 크면 Middle·Bottom은 위로 넘칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (91 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **줄 높이 fixture**: Auto 1024/baseline 819, Multiplier 1.5 → 1536/baseline 1075, Fixed 10px → 640/baseline 627(음수 leading), Latin+CJK 혼합 줄 최대 metric 1208/baseline 952, 빈 paragraph 한 줄 | 통과 |
| **가로 정렬 fixture**(100px box, 내용 16px): LTR Start 0·End 5376·Center 2688, RTL Start 5376·End 0, 마지막 줄 Justify = Start | 통과 |
| **양쪽 정렬**: "aa bb cc dd" 40px → 첫 줄 내부 공백이 256 subpx를 받아 폭 40px, 끝 공백·마지막 줄은 받지 않음 | 통과 |
| **paragraph 간격·쌓기**: 두 paragraph top 0 / 1664(1024 + 간격 640), 첫·마지막 baseline, 단일 paragraph에는 간격 없음, auto 폭 | 통과 |
| **text box 세로 정렬 fixture**(가로 정렬과 분리): 100px box Top 0·Middle 2688·Bottom 5376, 10px box Middle -192·Bottom -384 | 통과 |
| visual glyph 배치: LTR, LTR 안 히브리어(ג ב א 순, x 1792/2253/2714), RTL 문단 안 Latin | 통과 |

## 차이(작업 지시와 다른 점)

- 문서 `LineHeight::Multiplier(FiniteF32)`는 이 crate에서 1/1024 정수 배율(`LineHeightRatio`)로 받습니다(float
  연산 없이 결정론 유지). float → 배율 변환은 문서·레이아웃 연결 계층이 합니다.
- 자간(letter spacing)은 아직 적용하지 않습니다(X.10.1 보고서와 같음).

## 남은 위험 / 한계

- 줄 안 run별 baseline 정렬은 같은 baseline에 맞춥니다(위첨자·세로 정렬 속성은 범위 밖).
- 양쪽 정렬은 공백 segment에만 공간을 줍니다(CJK 글자 사이 inter-character justification 없음).

## 후속 수정 (posthoc 리뷰 TS-2: 줄 배치의 줄마다 전체 순회)

- **결함**: `lay_out_paragraph`가 줄마다 paragraph 전체를 훑었습니다. `place_glyphs`는 visual run마다 모든 run을,
  겹친 run마다 그 run의 모든 glyph를 걸렀고, `trailing_len`은 segment 전체를 뒤에서부터, `line_metrics`는 run
  전체를 훑었습니다. 비용이 (줄 수 × paragraph 크기)라 리뷰 측정(release, 40px 폭)에서 40k자가 866ms였습니다.
  BiDi 줄 재배치의 `line_levels`(X.9.3)도 줄마다 모든 BiDi paragraph를 훑어, U+2029가 많은 paragraph에서 같은
  문제가 있었습니다.
- **수정**: run·segment·BiDi paragraph는 논리 순서로 정렬되어 있으므로 줄과 겹치는 첫 항목을 이분 탐색하고
  겹치는 동안만 훑습니다(`overlapping_runs`). run glyph는 paragraph마다 한 번 만드는 (cluster, glyph index) 정렬
  표(`ClusterIndex`)에서 조각 범위를 이분 탐색해 원래 glyph 순서로 꺼냅니다. paragraph 전체 비용은
  O(n log n)이고 줄·glyph 배치 결과는 같습니다(golden 불변).
- **회귀 테스트**: `text_layout::tests::paragraph_layout_work_is_linear_in_text_length`는 40px 폭에서 줄이 많이
  생기는 세 입력(단일 run Latin `"abcd "`, 글자마다 run이 바뀌는 `"a日"`, 줄마다 BiDi paragraph가 바뀌는
  `"a"`+U+2029)을 500회·2,000회 반복해 `lay_out_paragraph` 방문 수(`op_count`)가 segment 수의 8배 이하이고
  입력 4배에 5배 이하인지 봅니다. 수정 전 알고리즘에 같은 계수를 넣으면 2,000회에서 각각 30,005,000회,
  13,343,335회, 16,004,000회로 실패합니다. text_layout만 고치고 `line_levels`를 그대로 두면 U+2029 입력이
  4,010,000회로 실패합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 0 (110 unit passed, golden 포함) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 |
| `cargo clippy -p stellar-text --release -- -D warnings` | 0 |
| `cargo fmt -p stellar-text -- --check` | 0 |

## 후속 수정 (posthoc 수정 리뷰: 이분 탐색 정렬 불변식 미확인)

- **결함**: 위 수정의 이분 탐색(`overlapping_runs`, `trailing_len`, `BidiParagraph::line_levels`)은 glyph run·segment·BiDi
  paragraph가 텍스트를 순서대로 빈틈없이 나눈다는 불변식에 기대지만, 이 불변식을 문서 주석으로만 적고 코드로
  확인하지 않았습니다. `shape_paragraph`가 순서를 어기는 결과를 내면 줄 배치가 panic 없이 잘못 나올 수 있었습니다.
- **수정**: `crates/text/src/tiling.rs`(신규)의 `debug_assert_tiles`로 목록을 만드는 곳(`shape_paragraph`,
  `BidiParagraph::analyze`)과 쓰는 곳(`lay_out_paragraph` 진입, paragraph당 한 번)에서 debug 빌드일 때 확인합니다.
  release 빌드에서는 확인하지 않으며 배치 결과는 같습니다(golden 불변).
- **회귀 테스트**: `text_layout::tests::unordered_runs_panic_in_debug_build`,
  `unordered_segments_panic_in_debug_build`, `unordered_bidi_paragraphs_panic_in_debug_build`는 shaping 결과의 run·
  segment·BiDi paragraph 순서를 바꿔 `lay_out_paragraph`를 부르고 debug 빌드에서 panic하는지 봅니다. 수정 전에는
  셋 다 panic하지 않아 실패했습니다. `paragraph::tests::run_segment_and_bidi_ranges_tile_the_text`는 혼합 script
  입력에서 세 목록이 텍스트를 나누는지 release 빌드에서도 고정합니다.
- **한계**: 작업량 테스트(`op_count`)는 계측한 루프 안의 퇴행만 잡습니다. 계측하지 않은 새 루프는 잡지 못합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 0 (120 unit passed, golden 포함) |
| `cargo test --release -p stellar-text` | 0 (117 unit passed, debug 전용 panic 테스트 3개 제외) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 |
| `cargo clippy -p stellar-text --release --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-text -- --check` | 0 |
