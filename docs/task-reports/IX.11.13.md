# 작업 보고서: IX.11.13 registry 값 문법별 CSS 값 formatter

- WBS: IX.11.13
- 작업명: registry 값 문법별 CSS 값 formatter(속성 모델 8.2·8.3절 정규 형식, 토큰 `var(--name)`)
- 상태: 완료 (`[x]`)
- 결정: ADR-0075 결정 5

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/css/value.rs`(신규, `crates/commands/src/css_value.rs`의 구현을 옮김) — `CssValue`에
  `Percentage`·`Duration`·`Number`·`Transform`·`Var`(토큰 참조 `CssVar`) 추가, `format_percentage`·
  `format_duration`·`format_transform`(`none` 또는 `matrix(a, b, c, d, e, f)`)·`format_var`, golden·byte 동일성·
  f32 왕복 property test. `crates/document/src/css/mod.rs` 등록.
- `crates/commands/src/css_value.rs` — 구현을 지우고 `stellar_document::css::value`를 다시 내보냄(속성 패널 API
  그대로, 이중 구현 없음).

## 구현 요약

- 정규형: 수는 f32를 왕복하는 가장 짧은 10진(Rust `Display`, 지수 없음, `-0`은 `0`), 길이 `px`·백분율 `%`·각도
  `deg`·시간 `ms`(0에도 단위), keyword 소문자, 색은 sRGB이고 네 채널이 [0,1]이며 8비트로 정확하면 `#rrggbb[aa]`,
  그 밖은 `color(srgb …)`·`color(display-p3 …)`, transform은 단위 행렬이면 `none` 아니면 `matrix()`(분해하지 않음,
  O13), 토큰은 `var(--name)`.
- 이름 파생(`--name`)은 IX.11.16, parser 확장은 IX.11.14입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::value` | 9 passed |
| `cargo test -p stellar-commands --lib` | 122 passed, 4 ignored(옮긴 css_value test 6개는 document에서 실행) |
| `cargo clippy -p stellar-document -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo build --workspace --all-targets` | 성공 |

| 검사 | 결과 |
|---|---|
| 값 문법별 golden 출력(길이·백분율·각도·시간·수·개수·색 4형·keyword·bool·문자열·transform·border·corners·var) | 통과 |
| 같은 값 byte-identical 출력(다른 입력 표기 → 같은 정규 텍스트) | 통과 |
| f32 왕복 property test(임의 bit 패턴 약 20만 개, 고정점 확인) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음. SVG profile 표기(속성 모델 8.2절 SVG)는 IX.11.20 범위입니다.
