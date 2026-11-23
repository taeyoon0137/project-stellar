# CSS 레이아웃 적합성 fixture

엔진이 받는 CSS 선언의 배치 결과가 브라우저와 같은지 확인하는 fixture입니다(ADR-0076 결정 9, 계획 IX.11.8).

## 파일

| 파일 | 역할 |
|---|---|
| `definitions.py` | fixture 정의(node tree와 CSS 선언). 정본입니다 |
| `generate.py` | 정의를 HTML로 만들어 브라우저로 border box rect를 재고 `expected.json`을 씁니다 |
| `expected.json` | 생성 결과(커밋). 테스트가 읽는 기대값입니다 |
| `expected_ua_box_sizing.json` | `--ua-box-sizing`으로 만든 기대값(커밋). 요소의 `box-sizing`을 정하지 않아 UA 기본값(`content-box`)으로 잽니다. `css` 기본값 세트가 브라우저 기본값과 같은지 보는 종단 test(`tests/integration/tests/style_defaults_layout.rs`)가 읽습니다 |
| `../css_conformance.rs` | 기대값과 엔진 결과를 비교하는 테스트 |

## 기준 브라우저

- Google Chrome 157.0.8103.79(macOS, headless `--headless=new`). 버전과 user agent는 `expected.json` 머리에 기록됩니다.
- Chromium 계열은 레이아웃 좌표를 1/64 px 고정소수(LayoutUnit)로 계산해 엔진 `LayoutUnit`과 해상도가 같습니다
  (속성 모델 1.3절). 생성기는 `getBoundingClientRect()` 값에 64를 곱한 정수를 기록합니다.

## 다시 생성하기

```sh
python3 crates/layout/tests/css/generate.py            # macOS 기본 설치 경로의 Chrome
python3 crates/layout/tests/css/generate.py --chrome /path/to/chrome
python3 crates/layout/tests/css/generate.py --ua-box-sizing    # expected_ua_box_sizing.json
STELLAR_CHROME=/path/to/chrome python3 crates/layout/tests/css/generate.py
```

- Python 표준 라이브러리만 씁니다. 브라우저와 생성기는 빌드·테스트 의존성이 아닌 개발 도구입니다. 테스트
  (`cargo test -p stellar-layout --test css_conformance`)는 브라우저를 실행하지 않습니다.
- 기준 브라우저 버전을 바꾸면 결과 차이를 검토한 뒤 이 README와 ADR-0076의 버전 표기를 함께 바꿉니다.

## HTML로 옮기는 규칙

- 모든 요소에 `box-sizing: border-box; margin: 0; border: 0; padding: 0; font-size: 0; line-height: 0`을 줍니다(엔진의
  고정 값). fixture style에 `box-sizing: content-box`가 있으면 inline style이 이겨 그 요소만 content-box가 됩니다
  (`css` 기본값 세트의 생략값, ADR-0086).
- 엔진은 모든 box를 absolute 자식의 containing block으로 씁니다. 그래서 `position: static` 요소는 inset 없이
  `position: relative`로 냅니다(속성 모델 3장 stacking 규칙과 같은 결과).
- root는 너비 1,000,000 px containing block 안 `left: 0; top: 0` absolute 요소입니다(엔진 `ROOT_AVAILABLE_PX`).
- 콘텐츠 leaf(`content`)는 고정 intrinsic 크기 대체 요소로 둡니다. baseline이 없으면 `width × height` block 하나,
  있으면 `width × height` inline-block 안에 높이 `baseline`인 inline-block을 둬 첫 baseline을 콘텐츠 top에서
  `baseline`에 만듭니다.

## 구성(2026-11-23 생성, content-box 10개 추가)

| suite | fixture 수 | 최소(IX.11.8) |
|---|---|---|
| `flex` | 127 | 60 |
| `grid` | 76 | 40 |
| `positioning` | 45 | 30 |
| `block` | 19 | — |

필수 범주는 `css_conformance.rs`의 `required_fixture_categories_exist`가 이름으로 검사합니다: 두 축 indefinite
`aspect-ratio`, 한 축만 definite인 `aspect-ratio` 안 넘치는 콘텐츠(`min-height: 0px` 명시 포함), 음수 남는 공간의
`space-around`·`space-evenly`, bare `fr`, fr 합 < 1, 기본 flex 안 `fit-content` 자식, flex item 자동 최소 크기, baseline
없는 node의 아래 변 fallback, `distribution` 묶음 5개 이상.

## 판정

- `basic` 묶음: 남는 공간과 분배 몫이 1/64 px 격자에서 나누어떨어지는 입력. rect가 1 LayoutUnit 안에서 같아야 합니다.
- `distribution` 묶음: 나누어떨어지지 않는 입력. 같은 줄(부모의 표시되는 자식) item 수 이하 LayoutUnit 차이를 허용합니다.
- `exception`: 속성 모델 14장 확장으로 CSS와 다르게 정한 항목. 지금은 없습니다.
