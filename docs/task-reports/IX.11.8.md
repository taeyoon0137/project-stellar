# 작업 보고서: IX.11.8 CSS 레이아웃 적합성 fixture

- WBS: IX.11.8
- 작업명: CSS 레이아웃 적합성 fixture(Flexbox 60, Grid 40, positioning 30 이상, CSS 선언과 기대 border box rect)
- 상태: 부분 완료 (`[~]`) — fixture·runner·판정은 끝났고, 완료 기준 중 `space-around`·`space-evenly` 확인 결과의
  속성 모델 4.2절 기록과 `config/dependencies.yaml` 승인 절차가 이 lane 소유 경로 밖이라 남았습니다(아래 "차이").
  상태 표기는 통합 때 ledger가 정합니다.
- 결정: ADR-0076 결정 9
- 선행: IX.11.4(`4861c510`), IX.11.5(`f512d4fe`), IX.11.23(`83c5ab7b`), X.11.1

## 필수 실행 환경

- `DEV-MAC`. 기대값 생성은 로컬 Google Chrome 157.0.8103.79(macOS, `--headless=new`)와 Python 3 표준 라이브러리,
  테스트는 순수 Rust입니다.

## 변경 파일 목록

- `crates/layout/tests/css/definitions.py` — fixture 정의(정본). 이 커밋에서 두 축 indefinite `aspect-ratio` fixture 4개
  추가(누적 201개). 검토 단계에서 56개를 더했습니다(누적 257개, 아래 "검토 보강").
- `crates/layout/tests/css/generate.py` — HTML 생성·headless 측정·`expected.json` 기록(이 lane 첫 커밋부터, 재시도 포함).
- `crates/layout/tests/css/expected.json` — 기대값(브라우저 버전·user agent·LayoutUnit 배율 기록).
- `crates/layout/tests/css/README.md` — 생성 절차, 기준 브라우저 버전, HTML 변환 규칙, 구성·판정.
- `crates/layout/tests/css_conformance.rs` — runner(basic 1 LayoutUnit, distribution 줄 item 수), 예외 표시 처리,
  필수 범주·개수 검사, 결정론 검사.
- `crates/layout/tests/support/{json,css}.rs` — 테스트용 JSON reader와 CSS 선언 parser(외부 의존성 없음).
- `docs/task-reports/IX.11.8.md`(신규).

## 구현 요약

- 각 fixture는 node tree, node별 CSS 선언(`style`), 고정 intrinsic 콘텐츠(`content`: 너비·높이·baseline px), 기대
  border box rect(부모 border box 원점, 1/64 px 정수)를 가집니다. 같은 선언 문자열이 브라우저 HTML style과 엔진 입력
  parser에 그대로 들어갑니다.
- 텍스트는 고정 intrinsic 크기 대체 요소로 둡니다(README "HTML로 옮기는 규칙").
- `basic` 묶음은 남는 공간·분배 몫이 1/64 px 격자에서 나누어떨어지는 입력, `distribution` 묶음(5개)은 나누어떨어지지
  않는 입력입니다.
- 예외 표시(`exception`)는 속성 모델 14장 확장용입니다. 지금 예외는 0개입니다. 예외 표시 fixture가 브라우저와 일치하면
  테스트가 실패해 예외를 지우게 합니다.
- 브라우저와 생성기는 개발 도구이고 테스트는 브라우저를 실행하지 않습니다. 도입 승인 기록은 ADR-0076 결정 9입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `python3 crates/layout/tests/css/generate.py` | fixture 201개(flex 102, grid 56, positioning 33, block 10) 생성, Chrome 157.0.8103.79 |
| `cargo test -p stellar-layout --test css_conformance` | 4 passed(일치, 개수, 필수 범주, 결정론) |
| `cargo test -p stellar-layout` | 전부 passed, 0 failed |
| (검토) `python3 crates/layout/tests/css/generate.py` 재실행 | 보강 전 201개의 기대값이 커밋된 `expected.json`과 byte 단위로 같음(브라우저 재측정으로 기대값 출처 확인). 보강 뒤 257개(flex 124, grid 75, positioning 43, block 15) |
| (검토) `cargo test -p stellar-layout --test css_conformance` | 4 passed, 257/257 일치 |

| 검사 | 결과 |
|---|---|
| basic 묶음 rect 1 LayoutUnit 안 일치 | 196/196 |
| distribution 묶음 같은 줄 item 수 이하 LayoutUnit 차이(`flex/grow-distribution-thirds`, `flex/center-odd-free-space`, `flex/space-between-uneven`, `grid/fr-thirds-distribution`, `grid/center-odd-free`) | 5/5 |
| 속성 모델 14장 확장 예외 표시 | 0개(아래) |
| fixture count(flex ≥ 60, grid ≥ 40, positioning ≥ 30) | 124 / 75 / 43 |
| 필수 범주(두 축 indefinite `aspect-ratio`, 한 축 definite `aspect-ratio` 안 넘치는 콘텐츠와 `min-height: 0px` 명시, 음수 남는 공간 `space-around`·`space-evenly`, bare `fr`, fr 합 < 1, 기본 flex 안 `fit-content` 자식, 자동 최소 크기) | 모두 있음, 모두 일치 |

## 확인 결과 기록

- **`space-around`·`space-evenly`의 음수 남는 공간**: Chromium 157은 시작 쪽에 놓습니다(`safe center` fallback). 엔진도
  같습니다(`flex/negative-free-space-space-around`·`-space-evenly`, 이전 엔진은 가운데). 속성 모델 4.2절은 이 lane 소유
  경로 밖이라 여기와 ADR-0076 결정 6에 기록했습니다.
- **14장 E9·E10**: 엔진이 CSS 의미(비율 축 `min-*: auto` = 콘텐츠 하한, 두 축 indefinite면 콘텐츠 너비 뒤 비율)로 바뀌어
  `block/aspect-ratio-content-overflow-min-auto`(E10), `block/aspect-ratio-both-indefinite-*`·
  `flex/aspect-ratio-both-indefinite-*`(E9)가 브라우저와 일치합니다. 예외 표시가 필요 없습니다. 속성 모델 14장의 E9·E10
  삭제는 속성 모델 소유 lane이 합니다.

## 검토 보강

구현과 따로 정한 입력으로 엔진을 시험하려고 56개를 더했습니다. 기대값은 같은 브라우저로 생성했고 엔진 코드는 바꾸지 않았습니다.
범주는 다음과 같습니다.

- 백분율 해석: column flex 안 백분율 높이, padding이 있는 container의 백분율 `flex-basis`, stretch된 item·flex로 정해진
  item 안 백분율 자식, `auto` 높이 부모 안 백분율 높이, `aspect-ratio`와 백분율 너비, 백분율 inset.
- positioning: flex container 안 absolute 자식의 static position(`justify-content`·`align-items`·padding 반영), 양쪽 inset과
  `auto`·`<length>`·`fit-content` 너비, `right`·`bottom`만 준 자식, indefinite 높이 기준 백분율 `top`, flex item의
  `position: relative`, grid container 안 absolute 자식.
- grid: px·fr 혼합, span 자동 배치와 빈 칸, 음수 line, 명시 track 밖 암시 track, `grid-auto-flow: column`,
  `fit-content()` track, item 백분율 크기와 `justify-self`·`align-self`, `justify-content`·`align-content` 분배,
  콘텐츠가 넘치는 `1fr`, span item의 intrinsic track 기여, indefinite 너비의 fr, definite 높이의 fr 행과
  `minmax(0px, 1fr)`, `justify-items`, 겹치는 명시 배치.
- flex: `row-reverse`·`wrap-reverse`·`column-reverse` wrap, `max-width` freeze 반복, root `min-content` 너비 wrap,
  `display: none` 형제와 gap, column `align-items: center` 축소 맞춤, 음수 남는 공간의 `align-content: space-around`,
  `flex-basis: content`, `max-content` 너비, column wrap, column item 자동 최소 크기, 가중 shrink와 `min-width` clamp,
  stretch와 `max-height`·`min-height`, wrap 줄 stretch.

## 차이(작업 지시와 다른 점)

- `config/dependencies.yaml` 승인 절차(계획 1장 14번): 등록 파일은 cargo ecosystem 형식이고 이 lane 소유 경로 밖이라
  등록하지 않았습니다. 승인 근거와 범위(로컬 설치 브라우저, Python 표준 라이브러리, 빌드·테스트 의존성 아님)는
  ADR-0076 결정 9에 적었습니다.

## 남은 위험 / 한계

- 완료 기준 두 항목이 남았습니다. 속성 모델 4.2절에 `space-around`·`space-evenly` 확인 결과(음수 남는 공간이면 시작 쪽)를
  적는 일과 `config/dependencies.yaml` 승인 절차는 속성 모델·설정 소유 쪽에서 통합 때 처리해야 합니다.

- 기준 브라우저는 macOS Chrome 한 버전입니다. 다른 버전에서 다시 생성하면 결과가 다를 수 있어 README 절차대로 차이를
  검토해야 합니다.
- 텍스트 줄 나눔·glyph 위치는 판정에 넣지 않습니다(속성 모델 1.3절).
