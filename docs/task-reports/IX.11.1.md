# 작업 보고서: IX.11.1 registry CSS 매핑 metadata 타입과 단위 계약

- WBS: IX.11.1
- 작업명: registry 항목별 CSS 매핑 metadata 타입, CSS longhand 이름 inventory, `core-types` 단위 계약
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 선행 IX.11.22(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0075 결정 2·4, ADR-0008 개정(단위 계약)

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/css/mapping.rs`(신규) — `MappingStatus`(`exact`·`converted`·`approximated`·`stellar-only`),
  `CssMapping`(longhand 이름 목록, 값 문법, 상태, `initial-differs`·`subset` 표시, 차이 설명, 변형별 변환 규칙
  `ConversionRule`), const 생성자, `CSS_LONGHAND_NAMES` inventory, `CSS_SHORTHANDS`(속성 모델 11장 펼침),
  이름 형식 검사, test.
- `crates/document/src/css/grammar.rs`(신규) — 값 문법 종류 `CssGrammar`를 registry 쪽으로 옮기고 `Percentage`·
  `Duration`·`Number`·`UnitInterval`·`Transform`을 더함. `crates/document/src/css/mod.rs` 등록.
- `crates/commands/src/css_value.rs` — `CssGrammar`를 document에서 다시 내보냄(이중 정의 제거). 새 문법의
  입력은 IX.11.13·IX.11.14 전까지 `unsupported-value`.
- `crates/core-types/src/value.rs`·`typed_value.rs` — 단위 계약 문서 주석(`px`·`deg`·`ms`·`%`).
- `docs/adr/ADR-0008-core-types-values.md` — 단위 계약 개정 문단.

## 구현 요약

- 매핑 metadata는 저장 bytes와 무관한 const 값입니다. registry 항목이 실제로 갖게 하는 일은 IX.11.2(Frame·
  Modifier)와 IX.11.3(Shape·Path·Text·Image)이 합니다.
- inventory에는 registry가 쓰는 이름과, 영속 record가 아직 없지만 이름이 정해진 레이아웃·Typography longhand를
  함께 둡니다. shorthand는 inventory에 넣지 않고 펼침 표로 둡니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css` | 5 passed |
| `cargo test -p stellar-core-types` | 131 passed + doctest 8 passed |
| `cargo build -p stellar-commands` | 성공 |

| 검사 | 결과 |
|---|---|
| 타입 golden(상태 이름 4개 고정, 이름↔상태 왕복, builder 필드) | 통과 |
| inventory 중복 0·형식 검사, shorthand 펼침이 모두 inventory longhand | 통과 |
| storage golden 불변 | 저장 encoder·decoder 변경 없음. `crates/storage` 미변경(lane 최종 check-core에서 storage test 포함 실행) |
| 같은 diff 범위의 ADR | ADR-0008 개정 + ADR-0075. `cargo xtask validate-adr --base ce60bb92`는 lane 최종 검증에서 실행 |

## 차이(작업 지시와 다른 점)

- 값 문법 종류(`CssGrammar`)를 이 행에서 registry 쪽 crate로 옮겼습니다. 매핑 metadata의 "값 문법 종류" 필드가 parser와 같은 타입을 쓰게 하려는 것입니다(이중 정의 없음).

## 남은 위험 / 한계

- 없음. 새 문법 종류의 값 해석은 IX.11.13·IX.11.14에서 구현합니다.
