# 작업 보고서: IX.11.23 flex item 자동 최소 크기

- WBS: IX.11.23
- 작업명: flex item `min-width: auto`·`min-height: auto`를 CSS Flexbox 규칙(콘텐츠 기반 최소 크기)으로 구현
- 상태: 완료 (`[x]`)
- 결정: ADR-0076 결정 6
- 선행: IX.11.4(이 lane 커밋 `4861c510`), X.7.2, X.7.4

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, release benchmark). 기대값 생성만 로컬 Google Chrome 157.0.8103.79(headless)를 썼습니다.

## 변경 파일 목록

- `crates/layout/src/flex.rs` — `automatic_min_main`(콘텐츠 기반 최소 크기), flex base size 단계에서 사용.
- `crates/layout/tests/css/{definitions.py,expected.json}` — 자동 최소 크기 fixture 8개(`flex/auto-min-*`).
- `crates/layout/tests/fixtures/layout_corpus_v2.golden` — flex 요약 digest.
- `docs/task-reports/IX.11.23.md`(신규).

## 구현 요약

- scroll container가 아닌 item(엔진의 모든 box, `overflow: visible`)의 `min-*: auto`(main 축)는 콘텐츠 기반 최소
  크기입니다.
  - content size suggestion: main 축 min-content 크기. 측정은 같은 배치 요청 cache(X.7.2)를 씁니다(`content_only`
    요청, 가로 main이면 너비 질의라 자식 높이를 계산하지 않음). column이면 item cross 크기에서의 콘텐츠 높이입니다.
  - 비율 item: definite cross 크기를 비율로 바꾼 값이 하한이고, definite `min-*`·`max-*` cross 크기를 비율로 바꿔
    clamp합니다(Chromium 157 fixture `flex/auto-min-aspect-ratio-main-axis`로 확인).
  - specified size suggestion: main 축 크기 속성이 definite면 그 값. 결과는 둘 중 작은 값이고 definite max main으로
    제한합니다.
- 명시한 `min-*` 값은 그대로 씁니다(`flex/min-width-explicit-zero`, `flex/auto-min-clamped-by-max`의 `min-width: 0px`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `python3 crates/layout/tests/css/generate.py` | fixture 187개(flex 90, grid 56, positioning 33, block 8) 생성 |
| `cargo test -p stellar-layout` | unit 101, `component_layout_corpus` 4, `css_conformance` 3, `layout_corpus` 2, doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-layout -- --check`, `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 종료 코드 0, 경고 0 |
| `cargo run --release -q -p stellar-layout --example incremental_leaf_bench`(적용 전 3회, 적용 후 3회, 이전 엔진 2회) | 아래 표 |

| 검사 | 결과 |
|---|---|
| 자동 최소 크기 fixture 8개(콘텐츠가 shrink를 막음, 지정 크기 suggestion, max 제한, container item, column, block item, 비율 main 축, grow + basis 0) | Chromium과 일치 |
| 기존 fixture 179개 | 계속 일치 |
| 같은 입력 결정론(`css_fixture_inputs_are_deterministic`, corpus 재생성) | 통과 |

## X.11.1 corpus 갱신

적용 전(`f512d4fe`)과 적용 후 corpus를 같은 node id로 비교했습니다. flex 50개 중 5개만 바뀌었고, 모두 `min-*: auto`인
shrink 가능 item(`flex-shrink` > 0)에 콘텐츠(intrinsic leaf) 또는 비율이 있어 콘텐츠 최소 크기 아래로 줄지 않게 된
것입니다. 다른 배치 방식은 바뀌지 않았습니다(grid의 자동 최소 크기는 IX.11.5에서 이미 track 최소 기여로 구현).

| fixture | 바뀐 box 수 | 이유 |
|---|---|---|
| `flex-000` | 5 | 콘텐츠 46 px item(`flex-shrink: 0.5`, basis 13.75 px)이 콘텐츠 최소 크기 아래로 줄지 않음 |
| `flex-011` | 2 | 콘텐츠 76.5 px item(`flex-basis: 1.5px`)·`fit-content` item의 콘텐츠 최소 크기 |
| `flex-014` | 5 | 비율 item(`aspect-ratio: 4 / 1.25`, definite cross)의 비율 변환 최소 크기 |
| `flex-024` | 5 | 콘텐츠 79 px column item과 비율 item의 최소 크기 |
| `flex-034` | 4 | 콘텐츠 49 px item(`flex-shrink: 2`)의 콘텐츠 최소 크기 |

## X.7.4 benchmark(200,201 node, 단일 leaf 너비 edit 64회, release)

| 지표 | 적용 전(3회) | 적용 후(3회) | 이전 엔진 `ce60bb92`(같은 시간대 2회) |
|---|---|---|---|
| edit당 visited | 3 | 3 | 3 |
| reused 합 | 6,848 | 6,848 | 6,848 |
| cache hit 합 | 28,672 | 35,520 | — |
| edit duration p50 | 243–254 µs | 261–266 µs | 209 µs |
| edit duration p95 | 263–302 µs | 286–315 µs | 224–255 µs |
| edit duration max | 380–414 µs | 418–440 µs | 321–326 µs |
| 초기 전체 배치 | 1.21–1.26 s | 1.35–1.36 s | 0.97 s |
| 전체 계산 | 0.64–0.67 s | 0.75–0.78 s | 0.33–0.34 s |

- 방문·재사용 수는 그대로이고, 자동 최소 크기 측정으로 edit당 cache hit가 444 → 555로 늘었습니다(측정은 모두 cache
  hit, 새 측정기 호출 없음). edit p50은 약 4%, 전체 계산은 약 17% 늘었습니다.
- 이전 엔진보다 edit p50은 약 25% 느리고 전체 계산은 약 2.3배 느립니다. CSS intrinsic 크기 계산(min/max-content 질의,
  자동 최소 크기)을 하기 때문입니다. 측정 때 load average가 9–28이었습니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `overflow`(scroll container)를 모델링하지 않아 자동 최소 크기가 항상 적용됩니다. scroll container를 도입하면 그
  item은 0이어야 합니다.
- 전체 계산 시간이 이전 엔진보다 깁니다(위 표). 증분 edit는 방문 수가 같고 지연이 수백 µs 범위입니다.
