# 작업 보고서: IX.11.20 SVG export 매핑표와 registry CSS 매핑 연결

- WBS: IX.11.20
- 작업명: SVG 매핑표 각 행에 registry CSS 매핑 연결, SVG 값 표기를 CSS 값 formatter와 맞춤
- 상태: 부분 완료 (`[~]`) — 행 연결과 대조 test는 끝났습니다. formatter에 SVG profile을 더해 export가 그 함수를
  부르게 하는 정리는 하지 못했습니다.
  - crate 그래프가 `stellar-export → stellar-document` 의존을 허용하지 않습니다.
  - `css/value.rs`와 `svg/format.rs`가 이 lane 소유가 아닙니다.
- 결정: 속성 모델 8.2절
- 선행: IX.11.2, IX.11.13, XX.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/svg/mapping.rs`: `SvgFeature::css_longhands`.
- `tests/integration/tests/svg_registry_mapping.rs`(신규), `tests/integration/Cargo.toml`(dev 의존 한 줄).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 통과(기존 SVG golden bytes 불변). |
| `cargo test -p stellar-integration-tests --test svg_registry_mapping` | 통과. 매핑표 이름은 모두 registry CSS 매핑에 있습니다. 칠하기·합성 longhand 누락은 0입니다. SVG 수·sRGB 백분율·matrix 표기는 `css::value` formatter와 같습니다. |

## 차이(작업 지시와 다른 점)

- formatter를 한 곳으로 모으지 않았습니다. 대신 같은 결과를 test로 고정합니다(위 상태 설명).

## 남은 위험 / 한계

- 두 formatter가 따로 있어, 한쪽만 바뀌면 대조 test가 실패로 알려 줍니다. 한 곳으로 모으려면 crate 그래프 결정
  (ADR)이 필요합니다.
