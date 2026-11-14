# 작업 보고서: X.10.1 shaped glyph run 줄 나누기(cluster 보존)

- WBS: X.10.1
- 작업명: shaped glyph runs를 width constraint에 맞춰 line으로 나누고 cluster를 쪼개지 않음
- 상태: 완료 (`[x]`)
- 선행: X.9.4(이 lane `[x]`), X.9.5(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/units.rs`(신규) — `TextUnit`(1/64 px, 레이아웃 `LayoutUnit`과 같은 격자), `FontSize`, 정수
  반올림 변환, `UnitError`.
- `crates/text/src/paragraph.rs`(신규) — `shape_paragraph`(itemize → font fallback → shaping → subpixel 변환),
  `ShapedParagraph`, `GlyphRun`, `PositionedGlyph`, `Segment`, `ScaledLineMetrics`, `TextStyleSpec`,
  `ParagraphInput`, `TextLayoutError`, `is_hanging_whitespace`.
- `crates/text/src/line_layout.rs`(신규) — `break_lines`, `LineBreak`, `WrapMode`, prefix 합 측정.
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.1.md`(신규), 계획서(X.10.1 `[ ]`→`[x]`).

## 구현 요약

- **pipeline**: paragraph(`'\n'` 없음)를 itemization(X.9.2) → grapheme fallback(X.9.5) → shaping(X.9.1·X.9.6)
  하고, font unit을 style 글꼴 크기로 `i128` 정수 변환(중간값은 0에서 먼 쪽)한 subpixel glyph run으로 만듭니다.
- **원자 segment**: shaping cluster(`MonotoneGraphemes`라 grapheme을 쪼개지 않음, ligature는 한 cluster)를
  segment로 삼습니다. glyph가 없는 run은 grapheme segment(폭 0)입니다. 줄 경계는 segment 경계에서만 생기므로
  cluster가 두 줄로 나뉘지 않습니다.
- **줄 나누기**: X.9.4 기회 중 segment 경계에 있는 것만 후보로 씁니다. greedy로 들어가는 마지막 기회에서
  끊고, 줄 끝 hanging 공백은 폭에서 뺍니다. Mandatory는 항상 줄을 끝냅니다. 기회 사이가 너비보다 넓으면
  segment 경계 비상 줄바꿈(최소 segment 하나)을 합니다. `NoWrap`·너비 없음은 강제 줄바꿈만 씁니다. 폭은
  prefix 합으로 O(1) 측정합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (85 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **narrow width fixture**: "the quick brown fox" 48px → `the `/`quick `/`brown `/`fox`, 90px → 2줄, 끝 공백 hanging(폭 정확히 맞는 경우 같은 줄, trailing 256 subpx), 너비 0도 줄마다 segment 1개 이상 | 통과 |
| **long word fixture**: "officeoffice" 20px → `o`/`ffi`/`ce`/…(ffi ligature 미분할), 넉넉하면 한 줄, 긴 단어 뒤 단어 | 통과 |
| **CJK fixture**: "日本語の文章。です" 40px → `日本`/`語の`/`文`/`章。`/`です`(。 앞 금지), 한글 음절 사이 줄바꿈 | 통과 |
| segment가 shaping cluster를 따르고 subpixel 변환(16px: 500fu → 512, ffi 800fu → 819, 줄 metric 819/-205) | 통과 |
| U+2028 강제 줄바꿈·`NoWrap`·`hard_break`·음수 너비 오류·빈 paragraph 한 줄·`'\n'` 거부 | 통과 |
| 결정론·줄 연결이 원문과 같음(5개 너비) | 통과 |

## 차이(작업 지시와 다른 점)

- 줄 끝에서 `unsafe_to_break` cluster를 다시 shaping하지 않습니다(run 전체 shaping 결과를 그대로 나눔). 줄
  경계 문맥에 따라 glyph가 바뀌는 script(아랍어 연결형 등)는 줄 끝 glyph가 줄 중간 형태로 남을 수 있습니다.
- 자간(letter spacing)은 아직 적용하지 않습니다.

## 남은 위험 / 한계

- 위 줄 끝 재shaping 부재는 아랍어·인도계 script 품질 문제입니다. `ShapedGlyph::unsafe_to_break`가 보존되어 있어
  후속 작업에서 줄 끝 cluster만 다시 shaping할 수 있습니다.
- hyphenation(자동 하이픈)은 범위 밖입니다.

## 후속 수정 (posthoc 리뷰 TS-1: segment advance 계산의 제곱 비용)

- **결함**: `push_segments`가 shaping cluster segment마다 run의 모든 glyph를 다시 훑어 advance를 더했습니다.
  단일 run paragraph에서 비용이 glyph 수의 제곱이라, 리뷰 측정(release)에서 40k자 `shape_paragraph`가 3.13s
  걸렸습니다(엔진 `shaping::shape`만은 0.55ms). 텍스트가 바뀌면 paragraph 전체를 다시 shaping하므로 긴
  paragraph에서는 입력마다 이 비용이 듭니다.
- **수정**: glyph를 (cluster, advance)로 한 번 정렬하고 segment 순서로 한 번만 훑어 더합니다(O(n log n)).
  segment 경계·advance 합·공백 판정은 같습니다(golden 불변).
- **작업량 계수기**: `crates/text/src/op_count.rs`(신규)를 추가했습니다. 내부 루프가 방문 수를 `tick`으로
  알리고, 테스트 빌드에서만 thread-local로 셉니다(일반 빌드는 빈 함수). wall-clock이 아니라 결정론적 횟수로
  선형 상한과 크기 비례를 확인합니다.
- **회귀 테스트**: `paragraph::tests::segment_work_is_linear_in_glyph_count`는 `"abcd "` 반복 2,000자·8,000자
  paragraph에서 방문 수가 글자 수의 4배 이하이고, 입력을 4배로 늘려도 방문 수가 5배 이하인지 봅니다.
  수정 전 알고리즘에 같은 계수를 넣으면 8,000자에서 64,000,000회로 실패합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 0 (109 unit passed, golden 포함) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 |
| `cargo clippy -p stellar-text --release -- -D warnings` | 0 |
| `cargo fmt -p stellar-text -- --check` | 0 |
