# 작업 보고서: IX.11.4 레이아웃 엔진 입력을 CSS 값으로 바꾸고 Flexbox 의미를 CSS에 맞춤

- WBS: IX.11.4
- 작업명: layout 엔진 Stack 의미를 CSS Flexbox에 맞춤(`flex-shrink` 기본 1, `align-items`·`align-content` 생략값
  `normal`, 자식별 `align-self`, 축 고정 `row-gap`·`column-gap`, stretch 대상 제한)
- 상태: 완료 (`[x]`)
- 결정: ADR-0074 결정 6, ADR-0076
- 선행: IX.11.1, X.3.5, X.6.4. 이 lane 결정(ADR-0076 결정 1)대로 엔진 입력 타입은 `crates/layout` 안에 두고 document
  registry에 의존하지 않으므로 IX.11.1을 기다리지 않고 착수했습니다.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 기대값 생성만 로컬 Google Chrome 157.0.8103.79(headless)를 썼습니다.

## 변경 파일 목록

- `crates/layout/src/style.rs`(신규) — CSS 값 타입과 `LayoutStyle`(CSS initial value 생략값), CSS 텍스트 표기.
- `crates/layout/src/style_codec.rs`(신규) — style canonical bytes.
- `crates/layout/src/input.rs` — `LayoutNode { id, children, style, content, transform }`, 값·부분집합 검증, canonical
  bytes v2.
- `crates/layout/src/engine.rs`, `crates/layout/src/engine/tests.rs` — 요청(`known`·`parent`·`available`·`content_only`)
  기반 계산, memo·cache, 요청 정규화, 너비 질의, leaf 배치, 결과 조립.
- `crates/layout/src/block.rs`(신규) — `display: block` 배치.
- `crates/layout/src/flex.rs` — CSS Flexbox §9 배치(전면 교체).
- `crates/layout/src/position.rs`(신규) — `position: absolute`·`relative`.
- `crates/layout/src/fast_hash.rs`(신규) — memo·cache map hasher.
- `crates/layout/src/property.rs`, `dirty.rs` — CSS longhand 이름의 속성 registry·dirty class.
- `crates/layout/src/measure.rs` — subtree hash(transform 제외)와 배치 cache.
- `crates/layout/src/incremental.rs`, `crates/layout/src/incremental/tests.rs` — 배치 경계 기반 증분 레이아웃.
- `crates/layout/src/output.rs`, `error.rs` — `CyclicPercentage` 진단, `InvalidValue`·`Unsupported` 오류, 결과 bytes v2.
- `crates/layout/src/corpus.rs`, `bench.rs`, `view.rs`, `intrinsic.rs`, `transform.rs`, `guard.rs`, `rounding.rs`,
  `test_support.rs`, `lib.rs` — CSS 입력으로 이전.
- 삭제: `crates/layout/src/{sizing,stack,free,grid,grid_tracks,overlay,constraint}.rs`.
- `crates/layout/tests/css/{generate.py,definitions.py,expected.json}`(신규), `crates/layout/tests/css_conformance.rs`(신규),
  `crates/layout/tests/support/{mod,json,css}.rs`(신규) — Chromium 기대값 fixture와 runner.
- `crates/layout/tests/layout_corpus.rs`, `tests/fixtures/layout_corpus_v2.golden`(v1에서 이름 변경),
  `tests/component_layout_corpus.rs`, `tests/fixtures/component_layout_v2.golden`(v1에서 이름 변경).
- `crates/codegen/src/ir/{mod,snapshot,tests}.rs`, `crates/codegen/src/fixture.rs`,
  `crates/codegen/tests/fixtures/ir_{site,ios,android}.snapshot` — layout API 변경에 따른 호출부 적응(IR은 `LayoutStyle`).
- `docs/adr/ADR-0076-layout-engine-css-input.md`(신규), `docs/task-reports/IX.11.4.md`(신규).

## 구현 요약

- **입력 모델 교체(ADR-0074 결정 6)**: `ContainerLayout`·`SizingMode`·`FlexItem`·`StackLayout`·`FreePosition`·`Constraint`를
  없애고 CSS longhand(`display`, `position`, inset, `width`·`height`·`min-*`·`max-*`, `flex-*`, `justify-*`·`align-*`,
  gap, `grid-*`)를 받습니다. 표시 여부는 `display: none`입니다.
- **Flexbox(CSS Flexbox §9)**: flex base size(`flex-basis` → main 크기 속성 → 비율 → 콘텐츠), hypothetical main 크기
  clamp, line 나누기, flexible length 해석(freeze 반복, factor 합 < 1 규칙, 잔여 subpixel은 visual order 앞부터),
  hypothetical cross 크기, line cross 크기, `align-content`(normal = stretch), stretch, `justify-content`, absolute
  자식 static position.
- **다섯 변경**
  1. `flex-shrink` 생략값 1(`LayoutStyle::default`).
  2. `align-items`·`align-content` 생략값 `normal`(flex에서 stretch, `justify-content: normal`은 `flex-start`).
  3. 자식별 `align-self`(`auto`는 부모 `align-items`).
  4. 축 고정 `row-gap`·`column-gap`(row container의 main gap = `column-gap`).
  5. stretch는 cross 크기 속성이 `auto`인 item만 늘림. `fit-content`·`<length>`·`<percentage>` item은 `flex-start`처럼
     놓습니다.
- **분배 정렬 fallback**: `space-around`·`space-evenly`는 `safe center`(음수 남는 공간이면 시작 쪽)로 Chromium 157과
  같습니다(fixture `flex/negative-free-space-space-around`·`-space-evenly`). 이전 엔진은 가운데였습니다.
- block flow와 positioning(absolute = padding box containing block, relative offset)은 Free·Constraint 방식을 없애면서
  같은 입력 모델에 들어가야 해 이 커밋에 함께 넣었습니다. Grid(`display: grid`)는 IX.11.5 커밋에서 받고, 이 커밋에서는
  `Unsupported`로 거부합니다. `align-*: baseline`도 X.3.6 전에는 거부합니다. flex item `min-*: auto`는 IX.11.23 전에는 0입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout -p stellar-codegen -- --check` | 종료 코드 0 |
| `cargo clippy -p stellar-layout -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-layout` | unit 98, `component_layout_corpus` 4, `css_conformance` 3, `layout_corpus` 2, doctest 2 passed, 0 failed |
| `cargo test -p stellar-codegen` | 39 passed, 0 failed |
| `python3 crates/layout/tests/css/generate.py` | Chrome 157.0.8103.79로 fixture 120개(flex 82, positioning 30, block 8) 기대값 생성 |
| `cargo xtask validate-adr --base ce60bb92` | 종료 코드 0 |
| `cargo xtask generate-ffi-header --check` | drift 없음 |
| `cargo run --release -p stellar-layout --example incremental_leaf_bench` | 아래 성능 표 |

| 검사 | 결과 |
|---|---|
| Chromium 기대값 fixture 120개 rect 일치(`css_fixtures_match_browser_border_boxes`, basic 1 LayoutUnit, distribution 줄 item 수) | 120/120 일치 |
| 기본 flex(`align-items` 생략) 안 `fit-content` 자식이 늘어나지 않고 시작 쪽(`flex/default-align-fit-content-child-not-stretched`, `flex/align-items-normal`의 `fit` node) | 일치 |
| 같은 입력 결정론(`css_fixture_inputs_are_deterministic`, corpus 재생성 비교, 계산 반복 bytes 비교) | 통과 |
| 증분 = 전체 계산(무작위 edit, 적용 batch 500개 초과·거부 batch 50개 초과 조건, `random_edits_match_full_compute_and_reject_invalid_batches`) | 통과 |
| 공개 API·출력에 크기 방식 이름 없음(`every_layout_property_has_a_class_and_css_name`의 속성 이름 검사, `grep -rnE 'SizingMode\|HugContent\|\bFill\b\|\bHug\b\|Fixed\(\|StackLayout\|OverlayAlignment\|ContainerLayout' crates/layout/src crates/layout/tests/*.rs crates/layout/examples crates/codegen/src`) | 이름 검사 통과, 검색 0건 |

## X.11.1 corpus 갱신(v1 → v2)

생성기는 이전 corpus와 같은 seed와 같은 난수 호출 순서를 쓰고 각 선택을 속성 모델 4.3절 표의 longhand로 냅니다
(이전 Fixed·Percentage·HugContent는 main 축에서 `flex: 0 0 auto`, Fill은 main 축 `flex: 1 1 0px; min-*: 0px`, cross
축 `align-self: stretch`, 이전 정렬 값은 같은 CSS keyword로 명시). 그래서 node id·tree·box 수가 같고(Stack 262 box =
flex 262 box) rect 차이는 CSS 의미 차이에서만 옵니다. 이전 엔진(`ce60bb92`)과 이 커밋의 corpus를 같은 node id로 비교했습니다.

**다섯 변경별 영향**: 생성기가 이전 값을 명시하므로 변경 1(`flex-shrink` 생략값), 2(`normal` 생략값), 3(`align-self`, Fill
cross 축 변환에서만 쓰며 결과 같음), 4(gap 축 이름)는 corpus 결과를 바꾸지 않습니다(0 fixture). 변경 5(stretch 대상
제한)가 8 fixture를 바꿉니다. 그 밖의 변경은 엔진을 CSS Flexbox 알고리즘으로 바꾼 데서 옵니다.

stack(flex) 50개 중 18개는 같고 32개가 바뀌었습니다. 이유 분류(바뀐 node와 그 부모의 선언으로 판정, 한 fixture가 여러
이유를 가질 수 있음):

| 기호 | 이유 | fixture 수 |
|---|---|---|
| F1 | 변경 5: stretch는 cross 크기 `auto` item만 늘림(`fit-content` 제외) | 8 |
| F3 | 콘텐츠 크기 container 안 백분율: CSS는 container 크기 계산에서 백분율을 `auto`로 보고(순환 백분율), indefinite 높이 기준 백분율은 `auto` | 17 |
| F4 | `aspect-ratio`: 비율 축 `min-*: auto` = 콘텐츠 하한, 비율 전이 규칙 | 10 |
| F5 | 콘텐츠 크기 container의 intrinsic 크기 = item min/max-content 기여 합(`flex-basis`를 쓰지 않음) | 11 |
| F6 | wrap container의 line 나눔과 콘텐츠 크기 item | 8 |
| F7 | `fit-content` main 크기와 `flex-basis`의 flex base size 규칙 | 10 |
| F8 | `min-*`·`max-*` clamp 순서(CSS: max 뒤 min)와 hypothetical main 크기 | 29 |
| F9 | 연쇄 변화(형제·부모 크기 변화를 따름) | 1 |

| fixture | 바뀐 box 수 | 이유 |
|---|---|---|
| `flex-000` | 7 | F3; F5; F6; F7; F8 |
| `flex-001` | 7 | F1; F3; F8 |
| `flex-002` | 5 | F7; F8 |
| `flex-003` | 1 | F1; F6; F8 |
| `flex-004` | 5 | F3; F5; F7; F8 |
| `flex-005` | 2 | F4; F6 |
| `flex-006` | 1 | F3; F8 |
| `flex-007` | 1 | F1; F8 |
| `flex-008` | 8 | F1; F4; F8 |
| `flex-010` | 4 | F8 |
| `flex-011` | 5 | F3 |
| `flex-012` | 7 | F3; F7; F8 |
| `flex-013` | 6 | F3; F5; F8 |
| `flex-016` | 6 | F3; F4; F5; F8 |
| `flex-017` | 2 | F6; F8 |
| `flex-018` | 1 | F9 |
| `flex-023` | 6 | F4; F6; F8 |
| `flex-024` | 7 | F3; F4; F6; F7; F8 |
| `flex-025` | 4 | F1; F3; F4; F5; F8 |
| `flex-027` | 7 | F3; F4; F5; F8 |
| `flex-030` | 2 | F5; F8 |
| `flex-033` | 5 | F1; F3; F4; F8 |
| `flex-034` | 4 | F3; F5; F6; F7; F8 |
| `flex-035` | 6 | F4; F6; F7; F8 |
| `flex-036` | 4 | F3; F4; F5; F8 |
| `flex-037` | 1 | F1; F8 |
| `flex-038` | 6 | F3; F5; F8 |
| `flex-040` | 1 | F7; F8 |
| `flex-041` | 4 | F3; F7; F8 |
| `flex-044` | 3 | F1; F7; F8 |
| `flex-047` | 3 | F3; F5; F8 |
| `flex-048` | 5 | F8 |

absolute(이전 free)·inset(이전 constraint) 갱신은 positioning 의미 변경이라 IX.11.5 보고서에 적습니다. grid·overlay
fixture는 IX.11.5 커밋에서 다시 들어옵니다.

## X.11.2 component corpus 갱신(v1 → v2)

grid root scene(dashboard)을 뺀 16 fixture의 모든 box rect가 v1과 같습니다. 바뀐 것은 CSS 첫 baseline 규칙(block은
첫 in-flow 자식, flex는 첫 item)으로 container box에 baseline이 생긴 것뿐입니다. dashboard 4 fixture는 IX.11.5 커밋에서
`display: grid`로 다시 넣습니다.

## 성능 (X.7.4 benchmark, 200,201 node, 단일 leaf 너비 edit 64회, release)

같은 기계(Apple M3 Max)에서 이전 엔진(`ce60bb92`, 임시 사본)과 이 커밋을 번갈아 실행했습니다. 다른 작업이 함께 돌아 load
average가 25\~37이었으므로 절대값보다 같은 조건의 비교로 봅니다.

| 지표 | 이전 엔진 | 이 커밋 |
|---|---|---|
| edit당 visited / reused | 3 / 107 | 3 / 107 |
| edit duration p50 / p95 | 0.233 ms / 0.240\~0.247 ms | 0.253\~0.44 ms / 0.27\~0.67 ms |
| 초기 전체 배치(증분 상태 구성 포함) | 2.04\~2.46 s | 1.69\~2.40 s |
| 전체 계산(`full_compute_ns`) | 0.77\~0.88 s | 0.70\~0.81 s |

방문·재사용 수는 같고, 전체 계산은 같거나 빠르며, edit p50은 부하 변동 안에서 비슷하거나 느립니다. 최종 측정은
IX.11.23 보고서(자동 최소 크기 적용 전후)에 다시 기록합니다.

## 차이(작업 지시와 다른 점)

- block flow와 positioning을 이 커밋에 함께 넣었습니다(위 구현 요약). IX.11.5는 Grid·Overlay 대체와 positioning 검증을
  맡습니다.
- 속성 모델 4.2절의 `space-around`·`space-evenly` 확인 결과 기록은 속성 모델 문서가 이 lane 소유 경로 밖이라 ADR-0076
  결정 6과 이 보고서에 적었습니다.

## 남은 위험 / 한계

- `config/dependencies.yaml`에 기대값 생성 도구(브라우저·Python)를 등록하지 않았습니다(이 lane 소유 경로 밖, ADR-0076
  결정 9).
- corpus 변경 이유 분류는 바뀐 node의 선언으로 판정한 것이라 한 fixture에 여러 이유가 붙고, 이유별로 따로 끄고 켠 결과는
  아닙니다(변경 5만 F1로 분리).
