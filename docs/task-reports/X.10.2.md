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
