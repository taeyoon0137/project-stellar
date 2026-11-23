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

## 후속 결정 (2026-11-23, `lane/svg-export`, ADR-0091 결정 6)

- export → document 의존은 열지 않습니다(ADR-0019, Scene Snapshot만 읽는 경계). SVG profile 표기는
  `crates/export/src/svg/format.rs` 한 곳에 두고, export의 모든 SVG 속성 값(새로 생긴 Outside stroke mask 색, 래스터
  `<image>` 상자 포함)이 이 모듈을 거칩니다. CSS profile과 같은 수·백분율·행렬 표기는 기존 종단 test가 지킵니다.
- 두 profile을 한 함수로 합치는 일은 수 표기를 core-types로 옮기는 core-types·document 소유 작업이 이 결정을 근거로
  합니다. 이 행은 그 일 전까지 부분 완료로 둡니다.
- 매핑표 행 변경: OutsideStroke row가 vector가 됐고(XX.3.4), BlendMode row 설명을 래스터 이유로 고쳤습니다. 두 row의 CSS
  longhand 연결(`stroke`·`stroke-width`, `mix-blend-mode`)은 그대로입니다.
