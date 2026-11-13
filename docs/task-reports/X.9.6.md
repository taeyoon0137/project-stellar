# 작업 보고서: X.9.6 OpenType feature와 variation axes를 shaping input에 적용

- WBS: X.9.6
- 작업명: OpenType feature와 variation axes를 shaping input에 적용
- 상태: 완료 (`[x]`)
- 선행: X.9.1(이 lane `[x]`), VI.7.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/shaping.rs` — `FontFeature`, `canonical_features`, `canonical_variations`,
  `ShapeRequest::features`, `FontFace::with_variations`·`variations`, feature 전달, fixture 테스트.
- `crates/text/src/font_runs.rs` — `ShapingStyle`, `shape_font_runs`가 style feature와 chain 항목·style
  variation을 적용(같은 (face, axes) instance 재사용), 통합 테스트.
- `docs/task-reports/X.9.6.md`(신규), 계획서(X.9.6 `[ ]`→`[x]`).

## 구현 요약

- **feature**: `ShapeRequest::features`(run 전체 적용)를 엔진 feature로 넘깁니다. 목록은 canonical(tag 오름차순,
  같은 tag는 마지막 값 우선 — VI.7.2 style run 규칙과 같음)로 정리합니다. 명시하지 않은 폰트 기본 feature
  (`liga` 등)는 켜진 상태입니다.
- **variation**: `FontFace::with_variations(axes)`가 같은 폰트 bytes를 공유하는 instance를 만듭니다. 값은
  canonical로 정리하고, 폰트에 없는 axis는 무시, 범위 밖 값은 axis 범위로 제한(OpenType 정규화)합니다. 줄
  metric은 instance 위치에서 다시 읽습니다.
- **style 연결**: `ShapingStyle { features, variations }`(문서 style run의 feature·axes를 옮긴 값). run의
  최종 axes = 선택된 chain 항목(`FontFaceRef`)의 axes 위에 style axes를 덮은 값(같은 tag는 style 우선)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (75 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **liga on/off fixture**(`liga_on_off_fixture`): "office fit" 기본·`liga=1` → `ffi`(cluster 1)·`fi`(cluster 7) ligature 포함 7 glyph, 폭 3400; `liga=0` → 문자별 10 glyph, 폭 3550; 같은 tag 중복은 마지막 값 우선 | 통과(예상 차이 일치) |
| **variable axis fixture**(`variable_axis_fixture`): wght 400 → 500, 900 → 600(HVAR +100), 650 → 550, 300 → 500(기본 아래 region 없음), 1000 → 600(범위 제한), 없는 axis 무시, 마지막 값 우선, canonical 기록 | 통과(예상 차이 일치) |
| chain 항목 axes(wght 900 → 600)와 style 덮어쓰기(wght 650 → 550), style `liga=0`(550 → 300·300), style 표 부족 시 typed error | 통과 |
| canonical feature·variation 목록 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (feature는 run 전체 범위에만 적용합니다. 문서 style run 경계가 shaping run을 이미 나누므로 run 안 부분
  범위 feature는 필요하지 않습니다.)

## 남은 위험 / 한계

- 줄 metric은 `MVAR`가 있는 폰트에서 instance 위치 값을 쓰지만, 합성 fixture 폰트에는 `MVAR`가 없어 그 경로는
  실제 variable font로 아직 검증하지 않았습니다.
- `gvar` phantom point로만 advance가 바뀌는 폰트(HVAR 없음)는 fixture가 없습니다.
