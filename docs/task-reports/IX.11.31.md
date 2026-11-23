# 작업 보고서: IX.11.31 `css`·`react-native` 세트 conformance fixture

- WBS: IX.11.31
- 작업명: `css`·`react-native` 세트 conformance fixture(ADR-0082 Consequences)
- 상태: 부분 완료 (`[~]`) — fixture·측정기·test는 끝났고 완료 기준 세 가지를 확인했습니다. 선행 IX.11.25가 `[ ]`(엔진
  연결이 이 작업 소유 밖)이고, RN 기대값은 Yoga를 실행해 얻지 않았습니다(아래 "남은 위험").
- 결정: ADR-0115 결정 2(신규)
- 선행: IX.11.24, IX.11.25, IX.11.8

## 필수 실행 환경

- `DEV-MAC` (순수 Rust test. 관측값을 다시 확인할 때만 로컬 Chrome).

## 변경 파일 목록

- `crates/document/src/css/style_defaults_conformance.txt`(신규): fixture 386줄. 두 세트 × 구획 4개 × layout longhand
  40개, 두 세트 × payload longhand 33개. 한 줄이 (세트, 구획, longhand, 기대값, 관측값, 출처)입니다.
- `crates/document/src/css/style_defaults_conformance.py`(신규): Chrome headless 측정기(표준 라이브러리만, 빌드·테스트
  의존성 아님).
- `crates/document/src/css/style_defaults.rs`: `CONFORMANCE_FIXTURE`, `conformance_mismatches`, test 2개.
- `docs/spec/style-defaults.md` 2.3절: fixture 설명.

## 구현 요약

- `css` 줄의 관측값은 Google Chrome 157.0.8103.79 headless에서 저자 스타일 없는 요소의 `computedStyleMap` 계산값입니다.
  구획 대응은 `frame`=`div`, `text`=`span`, `image`=`img`, `shape`=`svg rect`(`fill-rule`은 `svg path`)입니다.
  - 관측값과 기대값이 같은 줄은 출처 `chrome:`입니다.
  - 다른 줄은 9개이고, 출처가 이유를 적습니다.
    - `stellar-mapping:` Text·Image·Shape `display`(관측 `inline`, 기대 `block`).
    - `css-computed:` `border-*-width` 4개(관측 Typed OM `3px`, 기대 `0px`), `letter-spacing`(관측 `normal`, 기대 `0px`).
    - `user-agent:` `font-family`(관측 측정 환경 시스템 글꼴, 기대 `serif`).
- `react-native` 줄은 관측값 `-`이고 출처가 `rn-docs:`(RN 공식 문서), `rn-source:`(RN 구현 기본값), `yoga:`(Yoga 동작),
  `inherits:`(css 값을 물려받음과 그 이유) 중 하나입니다. `inherits:` 줄의 기대값은 `css` 값과 같아야 합니다.
- test는 기대값과 registry 표 값, 줄의 빠짐·겹침, 출처 접두사와 설명, 관측값과 출처의 일관성을 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `python3 crates/document/src/css/style_defaults_conformance.py` | Chrome 157.0.8103.79 측정 JSON. fixture 관측값과 같음 |
| `cargo test -p stellar-document --lib css::style_defaults` | 통과(13). `conformance_fixture_matches_tables`, `conformance_fixture_fails_when_a_table_value_changes` 포함 |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| 세트별 fixture가 registry 표 값과 일치 | `conformance_fixture_matches_tables`: 386줄 모두 일치, 빠진 줄 0 |
| 표 값을 바꾸면 fixture가 실패 | `conformance_fixture_fails_when_a_table_value_changes`: RN frame `flex-direction`을 바꾸면 그 줄 하나, 두 세트를 바꾸면 payload 줄, 출처를 지우거나 줄을 빼도 실패 |
| 기대값 출처 기록 누락 0 | 모든 줄이 세트별 출처 접두사와 설명을 가짐(test가 확인) |

## 차이(작업 지시와 다른 점)

- `react-native` 기대값은 RN 공식 문서와 RN 구현 기본값에서 얻었습니다. Yoga를 실행한 측정은 하지 않았습니다. Yoga를
  테스트 의존성으로 넣지 않는다는 결정은 ADR-0115 결정 2에 있습니다.
- layout 결과(rect) 수준 conformance는 기존 IX.11.8 corpus(`css` 세트 content-box fixture)가 맡습니다. 이 fixture는
  계산값(세트 표) 수준입니다.

## 남은 위험 / 한계

- `react-native` 표가 실제 RN 동작과 어긋나도 이 fixture는 잡지 못합니다. Yoga 실행 fixture가 후속입니다.
- 관측값은 한 브라우저 버전·한 OS 언어 설정에서 잰 값입니다. `font-family`처럼 환경에 따라 다른 값은 기대값을 환경과
  무관한 값으로 두고 이유를 적었습니다.
- 선행 IX.11.25(엔진이 이 표로 푼 값만 받음)가 끝나야 행 전체가 `[x]`입니다.
