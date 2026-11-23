# 작업 보고서: IX.11.12 Text paragraph 속성의 CSS 의미

## WBS

- WBS: IX.11.12
- 작업명: text layout의 `line-height`를 inline box마다 half-leading으로 해석, root inline box strut, run `letter-spacing` 연결
- 상태: 부분 완료 (`[~]`) — text 엔진(`stellar_text`)의 줄 상자·strut·letter-spacing과 fixture는 들어왔습니다. 앱 그리기 경로(runtime)가 문단 typography strut와 run letter-spacing을 엔진에 넘기는 연결은 남았습니다(아래 "차이").
- 결정: [ADR-0075](../adr/ADR-0075-css-property-registry.md) 개정 결정 12.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/text/src/text_layout.rs`: inline box마다 자기 글꼴 metric과 자기 line-height로 half-leading을 적용하는 `line_box`, root inline box `Strut`, `lay_out_paragraph_with_strut`(기존 `lay_out_paragraph`는 기본 strut로 위임), fixture 4개.
- `crates/text/src/paragraph.rs`: `ShapedParagraph::with_letter_spacing`(run style별 CSS letter-spacing, grapheme 단위, glyph·segment advance 동시 갱신).
- `crates/text/tests/golden/logical_layout_v1.txt`: 글꼴이 섞인 줄의 X.10 golden 갱신(이유는 아래).
- `docs/adr/ADR-0075-css-property-registry.md`: 개정 결정 12와 개정 Consequences·호환성.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 113 passed(lib), 실패 0 |
| `cargo clippy -p stellar-text --all-targets` | 경고 0 |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| 배율·고정 길이 혼합 크기 run line box fixture(`line-height: 20px`에 10px·40px run → 20px보다 큼) | `mixed_size_runs_use_their_own_half_leading_css_line_box`: 고정 20px → 1856 subpixel(29px), 배율 1.5 Latin·CJK → 1618 > 1536, 배율 2 10px·40px |
| 문단 글꼴보다 작은 run만 있는 줄의 strut 높이 fixture | `strut_from_paragraph_typography_sets_the_minimum_line_box`: 40px strut에 10px run만 있는 줄 = 2560 |
| run별 letter-spacing fixture | `run_letter_spacing_adds_after_each_character_unit`: run 0 px, run 1 em, ligature cluster는 grapheme 수만큼 |
| 단일 크기 문단 결과 불변 test | `single_size_paragraphs_keep_previous_line_boxes`(10·16·40px × 세 모드), 기존 `line_height_modes_fixture`의 단일 크기 기대값 그대로 |
| 바뀐 X.10 golden은 이유와 함께 갱신 | 아래 |

## golden 갱신 이유

- 바뀐 것은 글꼴이 섞인 줄뿐입니다(Latin 기본 face의 strut와 CJK·emoji fallback run). 이전 규칙은 줄의 최대 ascent·최소 descent·최대 line gap을 따로 모아 더했고, CSS는 inline box마다 자기 gap의 half-leading을 더한 뒤 baseline 위·아래 최댓값을 더합니다.
  - Latin 16px + CJK 16px `normal` 줄: 1208 → 1157(위 901 + 51, 아래 Latin 205).
  - CJK만 있는 줄: 1126 → 1157. root inline box strut(문단 첫 사용 가능 글꼴 = Latin)의 depth 205가 항상 들어갑니다.
  - emoji 혼합 한 줄: 1331 → 1229.
- 크기와 글꼴이 하나인 case의 digest는 바뀌지 않았습니다.

## 차이

- runtime(`crates/runtime/src/canvas/text/document.rs`)이 `ResolvedTextStyle::letter_spacing`을 `with_letter_spacing`에, 문단 typography의 strut를 `lay_out_paragraph_with_strut`에 넘기는 연결은 이 lane 소유 밖입니다. 그 전까지 앱 그리기는 첫 span 기본 strut를 쓰고 letter-spacing을 적용하지 않습니다.
- 작업 정의의 소유 경로 `crates/layout/src/text*`에 해당하는 파일이 없어, 행 정의가 가리키는 `crates/text/src/text_layout.rs`와 letter-spacing을 넣을 `crates/text/src/paragraph.rs`(메서드 하나)를 고쳤습니다. 이 crate를 소유한 다른 lane은 없습니다.
- 브라우저 실측 비교는 하지 않았습니다. 값은 CSS 2 §10.8 산술로 고정했습니다.

## 남은 위험

- strut를 항상 넣어 fallback 글꼴만 있는 줄도 기본 글꼴의 ascent·depth를 최소로 가집니다(CSS 의미). 앱 화면에서 해당 줄 높이가 커집니다.
- letter-spacing에서 ligature를 끄지 않아 ligature glyph 모양은 그대로이고 폭만 CSS와 같습니다.
