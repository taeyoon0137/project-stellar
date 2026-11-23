# 작업 보고서: IX.11.5 Grid·Free·Overlay 의미를 CSS에 맞춤

- WBS: IX.11.5
- 작업명: grid item 생략 정렬 `normal`(stretch)과 `justify-self`·`align-self`, track 최솟값 `auto`(bare `fr` =
  `minmax(auto, <n>fr)`), CSS fr 크기 규칙, Free containing block = padding box와 백분율 기준, anchor 없는 축,
  absolute 자식의 부모 크기 비기여, Overlay 정렬의 CSS 이름과 stretch, `grid-auto-rows` 생략값 결정
- 상태: 완료 (`[x]`)
- 결정: ADR-0074 결정 5·6, ADR-0076
- 선행: IX.11.1, X.2.3, X.4.5, X.5.1, X.6.5. 엔진 입력 타입은 `crates/layout` 안에 있어 IX.11.1을 기다리지 않습니다
  (ADR-0076 결정 1).

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 기대값 생성만 로컬 Google Chrome 157.0.8103.79(headless)를 썼습니다.

## 변경 파일 목록

- `crates/layout/src/grid.rs`(신규) — `display: grid` 배치(track 정의·암시 track, sparse 자동 배치, track sizing
  §11, track·item 정렬, absolute 자식, 첫 baseline).
- `crates/layout/src/engine.rs` — grid 값 변환(`Prepared::grid`)과 dispatch.
- `crates/layout/src/input.rs` — `display: grid` 수락, grid 부분집합 검증(absolute 자식의 명시 line, grid item baseline).
- `crates/layout/src/error.rs`, `lib.rs` — grid 상한 문서, `grid` 모듈.
- `crates/layout/src/corpus.rs` — `grid`(이전 Grid)·`grid-area`(이전 Overlay) 배치 방식.
- `crates/layout/src/engine/tests.rs`, `crates/layout/src/incremental/tests.rs` — grid 부분집합·상한·순환 백분율 진단,
  무작위 edit의 `display: grid`.
- `crates/layout/tests/css/{definitions.py,expected.json}` — grid fixture 56개, grid 안 positioning fixture 3개.
- `crates/layout/tests/css_conformance.rs` — grid fixture 수 검사(40 이상).
- `crates/layout/tests/component_layout_corpus.rs`, `tests/fixtures/component_layout_v2.golden` — dashboard scene을
  `display: grid`로 다시 넣음(fixture 20).
- `crates/layout/tests/fixtures/layout_corpus_v2.golden` — grid·grid-area 요약 줄.
- `crates/codegen/src/fixture.rs`, `crates/codegen/tests/fixtures/ir_{site,ios,android}.snapshot` — 이전 Overlay·Grid
  node를 `display: grid`로 되돌림(호출부 적응).
- `docs/task-reports/IX.11.5.md`(신규).

## 구현 요약

- **Grid(CSS Grid §8·§11)**: 명시·암시 track(앞쪽 암시 track은 pattern을 끝에서부터 반복), 음수 line, span, sparse
  자동 배치(`grid-auto-flow: row | column`, 행 고정 item 먼저), 열 → 행 순서 track sizing, intrinsic track(span 1 →
  span 크기 순 extra space 분배 → flexible track은 fr 비율), maximize, fr 확장, auto track stretch, track 묶음 정렬
  (`justify-content`·`align-content`), item 정렬.
- **track 최솟값 `auto`**: `TrackSize::fr(n)`은 CSS bare `<n>fr` = `minmax(auto, <n>fr)`이고
  `TrackSize::fr_from_zero(n)`이 이전 `Fraction`과 같은 `minmax(0px, <n>fr)`입니다. item 최소 기여는 자동 최소 크기
  (지정 크기와 min-content 중 작은 값, max로 제한)입니다(fixture `grid/bare-fr-grows-to-content-min`,
  `grid/minmax-zero-fr-ignores-content-min`, `grid/min-width-zero-in-bare-fr`).
- **fr 크기**: factor 합이 1보다 작으면 1로 나눕니다. indefinite면 factor가 1 이하인 track은 base를 factor로 나누지
  않습니다(fixture `grid/fr-sum-below-one-single`·`-pair`(definite), `grid/fr-sum-below-one-indefinite`,
  `grid/fr-indefinite-factor-above-one`).
- **item 정렬**: `normal`은 크기 `auto` 축에서 stretch(비율이 그 축을 정하면 start), `stretch`는 크기 `auto` 축만
  영역 크기로 늘림, `<length>`·`<percentage>`·`fit-content` item은 시작 쪽(fixture `grid/items-*`,
  `grid/aspect-ratio-item-normal`, `grid/self-overrides`). 비율 item의 `normal`은 Chromium 157이 가로는 늘리고 세로는
  비율로 정함을 fixture로 확인해 맞췄습니다.
- **Overlay 대체**: 한 칸 grid(모든 자식 `grid-area: 1 / 1`)이고 정렬 이름은 `justify-items`·`align-items`입니다.
  생략값 `normal`은 크기 `auto` 자식을 영역에 채우고 `<length>`·`fit-content` 자식은 시작 쪽에 둡니다(fixture
  `grid/overlay-default-stretch`, `grid/overlay-single-cell-center`, `grid/overlay-content-sized`).
- **positioning**(입력 모델 교체와 함께 IX.11.4 커밋에서 구현, 이 행에서 grid까지 검증): containing block = padding
  box, 백분율 inset·크기 기준 = padding box, inset 없는 축 = static position(block 흐름 위치, flex 유일 item 위치,
  grid padding box 안 self 정렬), absolute 자식은 부모 콘텐츠 기반 크기에 기여하지 않음. positioning fixture 33개
  (`position/*`, grid 안 3개 포함)가 Chromium과 같습니다.
- **`grid-auto-rows` 결정**: track 어휘에 `auto`(`TrackSize::AUTO`)를 더했습니다. 그래서 `grid-auto-rows`·
  `grid-auto-columns` 생략값은 CSS initial value `auto`이고, 영속 record(IX.11.6)는 `grid-auto-rows`를 생략할 수 있으며
  생략값은 `auto`입니다. 새 node 생성 preset은 명시 값입니다(fixture `grid/auto-rows-default-content`,
  `grid/auto-rows-stretch-definite-height`, `grid/auto-rows-explicit`).
- **순환 백분율 진단 유지**: 이전 HugContent 순환 진단은 CSS 의미의 `CyclicPercentage`로 바뀌었고, block·grid 모두
  테스트가 있습니다(`engine::tests::cyclic_percentage_is_resolved_like_css_and_reported`,
  `grid_cyclic_percentage_is_reported`).
- **받지 않는 조합**: grid 안 absolute 자식의 명시 line 배치, grid item `baseline` 정렬(`Unsupported`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `python3 crates/layout/tests/css/generate.py` | Chrome 157.0.8103.79로 fixture 179개(flex 82, grid 56, positioning 33, block 8) 생성 |
| `cargo fmt -p stellar-layout -p stellar-codegen -- --check` | 종료 코드 0 |
| `cargo clippy -p stellar-layout -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-layout` | unit 101, `component_layout_corpus` 4, `css_conformance` 3, `layout_corpus` 2, doctest 2 passed, 0 failed |
| `cargo test -p stellar-codegen` | 39 passed, 0 failed |

| 검사 | 결과 |
|---|---|
| Chromium 기대값 fixture 179개 rect 일치(grid 56·positioning 33 포함) | 179/179 |
| Overlay·Grid 생략 정렬에서 크기 `auto` 자식이 영역을 채우고 `<length>`·`fit-content` 자식은 시작 쪽 | `grid/items-normal`, `grid/overlay-default-stretch` 일치 |
| fr 합 < 1(`0.5fr` 단독, `0.25fr 0.25fr`) definite·indefinite | 일치 |
| `grid-auto-rows` 생략·명시가 고른 규칙(`auto` 생략값)과 일치 | 일치 |
| bare `fr`과 `minmax(0, <n>fr)`의 콘텐츠 최소 크기 차이 | 일치 |
| padding 있는 부모의 백분율 위치·크기(`position/abs-percent-insets`, `abs-percent-size-padding-box`) | 일치 |
| anchor 없는 자식(`position/abs-no-insets-static-position`, `abs-static-after-flow-siblings`) | 일치 |
| 순환 백분율 진단(block·grid) | 통과 |
| 증분 = 전체 계산(grid 포함 무작위 edit) | 통과 |

## X.11.1 corpus 갱신(v1 → v2)

IX.11.4와 같은 방법(같은 seed·난수 순서, 4.3절 변환, 같은 node id로 rect 비교)입니다. box 수는 모든 배치 방식에서
v1과 같습니다(absolute 235, grid 260, grid-area 190, inset 224). 이전 `Fraction` track은 export와 같은
`minmax(0px, <n>fr)`, 이전 `implicit_row`는 `grid-auto-rows`로 냈습니다.

| 배치 방식 | 같음 | 바뀜 |
|---|---|---|
| absolute(이전 free) | 0 | 50 |
| inset(이전 constraint) | 0 | 50 |
| grid | 24 | 26 |
| grid-area(이전 overlay) | 43 | 7 |

이유 기호(바뀐 node와 그 부모의 선언으로 판정, 한 fixture에 여러 이유):

| 기호 | 이유 |
|---|---|
| P1 | containing block = padding box(이전 content box). 생성기 root padding이 0–12 px라 inset이 있는 모든 fixture가 바뀝니다 |
| P2 | 백분율 inset·크기 기준 = padding box |
| P3 | inset 없는 축 = static position |
| P4 | absolute `auto`·`fit-content` 크기 = 가용 공간 안 fit-content(이전 콘텐츠 크기는 가용 공간과 무관) |
| P5 | absolute 자식은 부모 콘텐츠 기반 크기에 기여하지 않음 |
| P6 | 형제·가운데 기준 constraint는 CSS 값이 없어 `auto` inset(ADR-0074 결정 5) |
| G1 | fr 합 < 1: 남는 공간 × 합만 씀(§11.7.1) |
| G2 | 콘텐츠 크기 grid container: track sizing을 min/max-content 제약으로 계산 |
| G3 | 백분율 item 크기 기준 = grid 영역(행 sizing 중에는 indefinite) |
| G4 | 여러 track에 걸친 item의 extra space 분배(§11.5.3) |
| G5 | `aspect-ratio` item: `normal` 정렬과 비율 축 |
| G6 | item `min-*`·`max-*`와 최소 기여(자동 최소 크기) |
| G7 | intrinsic track의 item min/max-content 기여(§11.5.2) |
| F9 | 연쇄 변화(형제·부모 크기 변화를 따름) |

### absolute·inset

| 이유 | fixture 수 | fixture |
|---|---|---|
| P1; P2; P4 | 22 | `absolute-001`, `absolute-002`, `absolute-003`, `absolute-005`, `absolute-006`, `absolute-008`, `absolute-009`, `absolute-014`, `absolute-015`, `absolute-017`, `absolute-019`, `absolute-023`, `absolute-025`, `absolute-028`, `absolute-034`, `absolute-035`, `absolute-037`, `absolute-042`, `absolute-043`, `absolute-044`, `absolute-047`, `absolute-049` |
| P1; P2; P4; P5 | 11 | `absolute-018`, `absolute-020`, `absolute-021`, `absolute-022`, `absolute-029`, `absolute-030`, `absolute-031`, `absolute-033`, `absolute-045`, `absolute-046`, `absolute-048` |
| P1; P4 | 6 | `absolute-004`, `absolute-011`, `absolute-012`, `absolute-016`, `absolute-036`, `absolute-041` |
| P1; P2 | 5 | `absolute-000`, `absolute-007`, `absolute-010`, `absolute-024`, `absolute-039` |
| P1; P2; P5 | 2 | `absolute-013`, `absolute-038` |
| P1 | 1 | `absolute-032` |
| P1; P2; P3; P4 | 1 | `absolute-026` |
| P1; P2; P3; P4; P5 | 1 | `absolute-027` |
| P1; P5 | 1 | `absolute-040` |
| P1; P3; P4; P6 | 12 | `inset-000`, `inset-004`, `inset-005`, `inset-011`, `inset-012`, `inset-017`, `inset-027`, `inset-036`, `inset-040`, `inset-041`, `inset-046`, `inset-049` |
| P1; P4; P6 | 12 | `inset-006`, `inset-009`, `inset-010`, `inset-013`, `inset-015`, `inset-018`, `inset-019`, `inset-020`, `inset-025`, `inset-042`, `inset-043`, `inset-044` |
| P1; P3; P4; P5; P6 | 9 | `inset-007`, `inset-016`, `inset-021`, `inset-022`, `inset-026`, `inset-031`, `inset-035`, `inset-038`, `inset-047` |
| P1; P4; P5; P6 | 8 | `inset-002`, `inset-003`, `inset-008`, `inset-014`, `inset-023`, `inset-028`, `inset-033`, `inset-034` |
| P1; P6 | 7 | `inset-001`, `inset-024`, `inset-029`, `inset-030`, `inset-032`, `inset-045`, `inset-048` |
| P1; P5; P6 | 2 | `inset-037`, `inset-039` |

### grid·grid-area

| 이유 | fixture 수 | fixture |
|---|---|---|
| G2; G3; G4; G5; G6; G7 | 2 | `grid-030`, `grid-048` |
| G3; G4; G6; G7 | 2 | `grid-029`, `grid-040` |
| F9 | 1 | `grid-046` |
| G1 | 1 | `grid-037` |
| G1; G2; G3; G4; G6; G7 | 1 | `grid-006` |
| G1; G2; G3; G4; G7 | 1 | `grid-022` |
| G1; G2; G4; G6 | 1 | `grid-033` |
| G1; G2; G6 | 1 | `grid-042` |
| G1; G2; G7 | 1 | `grid-024` |
| G1; G3; G4; G5 | 1 | `grid-013` |
| G1; G3; G5; G6 | 1 | `grid-047` |
| G1; G3; G6; G7 | 1 | `grid-041` |
| G2; G3; G4 | 1 | `grid-045` |
| G2; G3; G4; G6; G7 | 1 | `grid-003` |
| G2; G3; G6 | 1 | `grid-025` |
| G2; G4; G6 | 1 | `grid-016` |
| G2; G5; G7 | 1 | `grid-007` |
| G2; G6 | 1 | `grid-000` |
| G2; G6; G7 | 1 | `grid-017` |
| G3; G4; G6 | 1 | `grid-021` |
| G3; G6 | 1 | `grid-038` |
| G4 | 1 | `grid-010` |
| G4; G7 | 1 | `grid-008` |
| G7 | 1 | `grid-031` |
| G3; G6 | 2 | `grid-area-007`, `grid-area-031` |
| G6; G7 | 2 | `grid-area-024`, `grid-area-035` |
| G2; G3; G5 | 1 | `grid-area-048` |
| G3; G5; G7 | 1 | `grid-area-019` |
| G5 | 1 | `grid-area-028` |

## X.11.2 component corpus

dashboard scene(grid root, 열 `minmax(0px, 1fr)` 2개, 행 `max-content` 2개)을 다시 넣어 fixture 20개가 되었고, 20개
모두 v1과 rect가 같습니다(차이는 IX.11.4 보고서의 container baseline뿐).

## 차이(작업 지시와 다른 점)

- positioning 구현은 Free·Constraint 방식 제거와 함께 IX.11.4 커밋에 들어갔습니다. 이 행은 grid·Overlay 대체와
  positioning 검증(fixture·corpus 비교)을 맡았습니다.
- 속성 모델 4.4절(`grid-auto-rows`)·4.5절 갱신은 속성 모델 문서가 이 lane 소유 경로 밖이라 ADR-0076 결정 6과 이
  보고서에 적었습니다.

## 남은 위험 / 한계

- grid track sizing의 열 재해석(행 크기로 열 min-content 기여가 바뀌는 경우, §12.1 3단계)은 하지 않습니다.
- indefinite 축 fr 크기에서 container `min-*`·`max-*`로 다시 계산하는 단계(§11.7 마지막 문단)는 하지 않습니다.
- 여러 track에 걸친 item 분배의 잔여 subpixel은 앞 track부터 1씩이라 Chromium(float 분배)과 나누어떨어지지 않는
  입력에서 차이가 날 수 있습니다(`distribution` 묶음 범위).
- `grid-template-areas`, named line, `repeat(auto-fill | auto-fit)`, `dense`는 받지 않습니다.
