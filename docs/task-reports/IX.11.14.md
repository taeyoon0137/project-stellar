# 작업 보고서: IX.11.14 CSS 값 parser와 shorthand 확장

- WBS: IX.11.14
- 작업명: registry 값 문법별 CSS 값 parser와 shorthand 확장(구조화 오류, 입력 길이 상한, 재귀 없는 문법)
- 상태: 완료 (`[x]`)
- 결정: ADR-0075 결정 5

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/css/value.rs` — `Percentage`(`%`)·`Duration`(`ms`·`s`→ms)·`Number`·`UnitInterval`(수 또는
  `%`, 0..1)·`Transform`(`none`, `matrix()`, `translate[X|Y]()`, `scale[X|Y]()`, 직각 배수 `rotate()`; 그 밖 회전·
  skew·3D는 `unsupported-value`) 해석, `TokenScope`·`NoTokens`·`parse_with`(`var(--name)`을 문서 토큰으로 풀고
  이름은 대소문자 구분, fallback 거부), `is_custom_property_name`, 테스트 전용 fuzz target `fuzz_one`, test.
- `crates/document/src/css/shorthand.rs`(신규) — 속성 모델 11장 shorthand 전부(`padding`, `inset`, `gap`, `flex`,
  `flex-flow`, `place-*`, `border`, `border-width|style|color`, `border-radius`, `grid-row|column|area`,
  `grid-template`, `overflow`, `background`, `font`, `white-space`, `text-wrap`)의 longhand 확장. 생략한 longhand는
  CSS initial value. `crates/document/src/css/mod.rs` 등록.

## 구현 요약

- 거부 코드: `unsupported-unit`(받지 않는 단위), `unsupported-value`(계산식·전역 keyword·fallback·타원 반경·
  areas·이미지 배경 등), `unknown-token`(scope에 없는 `var()`), `forbidden-value`(`url()`·`@import`·`image()`),
  형식 오류는 `invalid-value`.
- 문법은 재귀가 없습니다. 값 하나는 256 byte 상한, transform 함수는 16개 상한, shorthand 괄호 깊이는 8 상한입니다.
- shorthand 확장은 이름 대응만 하고, 펼친 값이 Stellar에서 받을 수 있는지는 longhand parser가 정합니다(예:
  `border: 1px`의 style `none`, color `currentcolor`).
- fuzz가 실제 결함 하나를 찾아 고쳤습니다: 다중 byte 문자로 시작하는 입력에서 `var(` 접두 검사가 문자 경계가
  아닌 곳을 잘라 panic했습니다(`get(..4)`로 수정).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::` | 24 passed |
| `cargo test -p stellar-commands --lib` | 122 passed, 4 ignored |
| `cargo clippy -p stellar-document -p stellar-commands --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| parse(format(v)) == v: fuzz target이 받아들인 모든 값에서 정규 텍스트 재해석이 같은 정규 텍스트(고정점) | 통과 |
| 11장 shorthand 확장 fixture(전 shorthand) | 통과 |
| 거부 입력 fixture(코드 4종 + 형식 오류, shorthand 거부) | 통과 |
| parser fuzz target panic 0(무작위 bytes와 유효 입력 변조 6,000 round, 모든 문법 × 모든 shorthand) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- fuzz는 결정론적 내장 harness입니다. 외부 fuzz 엔진(cargo-fuzz) 연결과 장기 실행은 XVI.6.7 범위입니다.
- `<length-percentage>` 혼합과 `em` 기준값은 속성 모델 15장 O3·O5가 정하기 전까지 거부합니다.
