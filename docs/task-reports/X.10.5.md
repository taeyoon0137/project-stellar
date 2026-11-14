# 작업 보고서: X.10.5 shaping·줄 레이아웃 cache key

- WBS: X.10.5
- 작업명: shaping·line layout cache key에 text hash, style hash, font face hash, width, locale, target profile 포함
- 상태: 완료 (`[x]`)
- 선행: X.10.1(이 lane `[x]`), X.8.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/cache.rs`(신규) — `ShapingCacheKey`, `LineLayoutCacheKey`, `StableHasher`(FNV-1a 128),
  `Hash128`, `TargetProfileKey`, `text_hash`, `style_hash`, `font_face_hash`, `LayoutCache`(hit·miss 집계, 검증 값
  비교).
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.5.md`(신규), 계획서(X.10.5 `[ ]`→`[x]`).

## 구현 요약

- **shaping key**: text hash(paragraph 텍스트), style hash(기준 방향, span 범위·style index·언어, style별 글꼴
  크기·feature·variation), font face hash(style별 chain 항목의 출처·file hash·face index·axes와 항목별 사용 가능
  여부), locale, target profile(호출자 제공 `TargetProfileKey`).
- **줄 레이아웃 key**: shaping key + 너비 제약 + paragraph 레이아웃 속성(정렬·줄 높이·wrap)과 줄바꿈 옵션
  (word-break·엄격도·locale) hash.
- **hash**: 길이 접두 필드를 쓰는 FNV-1a 128-bit(정수 연산만, 실행·플랫폼 독립). key는 저장하지 않는 런타임 값입니다.
- **cache**: `LayoutCache`는 `BTreeMap` 기반이고 key와 함께 원본 검증 값을 보관해, hash가 같아도 원본이 다르면
  miss로 처리합니다(충돌 방어). 사용 가능 여부가 key에 있어 누락 폰트가 등록·로드되면 다시 shaping합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (101 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **한 입력 변경마다 예상 cache miss**(`each_input_change_causes_expected_cache_miss`): 같은 입력은 둘 다 hit. text·글꼴 크기·feature·span 언어·기준 방향·font face(chain 항목 추가)·locale·target profile 변경은 shaping·줄 key 모두 miss, width·정렬·word-break 변경은 shaping hit·줄 miss. hit 집계(shaping 3, 줄 1) | 통과 |
| font 사용 가능 여부: 등록만 된 face는 hash 불변, 로드되어 사용 가능해지면 hash 변경 | 통과 |
| 줄 높이 모드(Auto·Multiplier·Fixed)가 줄 key에 반영 | 통과 |
| FNV-1a 128 golden(빈 입력 = offset basis, "a" 공개 test vector), 길이 접두 구분, 검증 값 불일치 miss | 통과 |

## 차이(작업 지시와 다른 점)

- target profile은 `target-profile` crate 타입(IX.10.1, 미구현)에 의존하지 않고 호출자가 profile id·고정 version
  으로 만든 `TargetProfileKey`(128-bit)를 받습니다(`text`는 core-types만 의존).

## 남은 위험 / 한계

- 128-bit 비암호 hash라 의도적 충돌을 만들 수 있지만, cache가 원본 검증 값을 비교하므로 잘못된 결과를 쓰지
  않습니다(X.10.6 엔진이 검증 값으로 원본 입력을 보관).
- font face hash는 폰트 bytes가 아니라 file hash로 정체성을 봅니다(X.8.1). 같은 hash에 다른 bytes를 연결하는
  잘못된 registry 사용은 막지 못합니다.

## 후속 수정 (posthoc 리뷰 TS-4: style·font face hash 범위)

- **결함**: `style_hash`·`font_face_hash`가 요청의 style 표 전체를 담았습니다. 둘째 paragraph만 쓰는 style을 바꿔도
  모든 paragraph의 shaping key가 바뀌어 전부 다시 shaping했습니다. paragraph마다 style 표 전체를 hash해 key 계산
  비용도 (paragraph 수 × style 수)였습니다.
- **수정**: `paragraph_styles`(신규)가 paragraph shaping 결과가 읽는 style index를 정합니다. span이 가리키는 style과,
  빈 paragraph 기본 줄 metric용 style(첫 span style이 없거나 표 밖이면 첫 style, `shape_paragraph` 규칙)입니다. 두
  hash는 이 style만 index와 함께 담습니다. `font_face_hash`는 paragraph 입력을 받도록 signature가 바뀌었습니다
  (crate 밖 사용처 없음). key는 저장하지 않는 런타임 값이라 hash 값 변경은 호환성 문제가 아닙니다. 레이아웃
  결과는 같습니다(golden 불변).
- **회귀 테스트**: `incremental::tests::style_change_reshapes_only_paragraphs_using_that_style`는 style 0·1·span 없는
  빈 paragraph로 된 문서에서 style 1만 바꾸면 둘째 paragraph만, style 0을 바꾸면 첫째와 빈 paragraph만 다시
  shaping하는지와 결과가 새 엔진 결과와 같은지 봅니다. 수정 전 `cache.rs`로는 `[0, 1, 2]`를 다시 shaping해
  실패합니다. `cache::tests::paragraph_styles_are_span_styles_plus_default_metric_style`는 style index 규칙을 고정합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 0 (114 unit passed, golden 포함) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 |
| `cargo clippy -p stellar-text --release -- -D warnings` | 0 |
| `cargo fmt -p stellar-text -- --check` | 0 |

## 후속 수정 (posthoc 리뷰 TS-4: 충돌 방어 범위)

- 위 "남은 위험"의 원본 검증 값 비교는 원래 텍스트·span·기준 방향만 비교했습니다. 이제 style 값·fallback chain·
  사용 가능 handle·paragraph 속성·줄바꿈 옵션까지 원본으로 비교합니다. 결함·수정·회귀 테스트는 X.10.6 보고서의
  "후속 수정 (posthoc 리뷰 TS-4: cache 검증 값이 hash로만 비교하던 입력)" 절에 있습니다.
