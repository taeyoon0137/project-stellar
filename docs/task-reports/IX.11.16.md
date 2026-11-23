# 작업 보고서: IX.11.16 토큰·ContextKey CSS custom property 이름 파생

- WBS: IX.11.16
- 작업명: TokenName·ContextKey 이름에서 CSS custom property 이름을 결정론적으로 파생, `@property`·`:root` 선언 텍스트, parser `var()` 연결
- 상태: 완료 (`[x]`)
- 결정: ADR-0075 결정 7, 속성 모델 10장

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/css/custom_property.rs`(신규) — `CustomPropertyKind`(`--`·`--context-`), `NameInput`,
  `derive_names`(이름·종류·StableId만 받는 순수 함수, 토큰·ContextKey 합집합 충돌 판정, `name-collision` 진단,
  StableId 오름차순 첫 항목 뒤 32자리 hex 접미사), `escape_identifier`(CSSOM identifier 직렬화), `base_name`,
  `DerivedNames`(`TokenScope` 구현: parser `var()`가 토큰 StableId로 풀림), `registered_syntax`, `literal_text`,
  `property_rule`(`inherits: true`, literal `initial-value`), `RootValue`·`root_block`(alias는 `:root`의 `var()`만).
  `crates/document/src/css/mod.rs` 등록.

## 구현 요약

- 입력 순서와 무관하게 같은 결과를 냅니다(충돌 그룹을 StableId로 정렬).
- `@property`는 Length·Percentage·Angle·Duration·Number·Color 타입만 등록하고, literal 타입이 선언 타입과 다르면
  만들지 않습니다(`None`). default expression의 기본 환경 평가는 호출자(bindings 평가기)가 하고 이 함수는 결과
  literal만 받습니다. 그래서 `initial-value`에는 `var()`가 들어갈 수 없습니다.
- ContextKey는 `var()` 해석 대상이 아닙니다(문서 값 문법에서 Context를 참조하지 않음). ContextKey resource
  (XXX.7.1)가 생기기 전이라 합성 입력으로 검증했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::custom` | 6 passed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 이름 파생 golden(구분자 치환, escape, 대소문자 보존, 비ASCII, 제어 문자, `--context-`) | 통과 |
| 토큰끼리·토큰과 ContextKey 사이 충돌 진단 fixture, 입력 순서 무관 | 통과 |
| 65초 안에 만든 StableId 두 개(앞 8자리 같음)의 접미사가 다름 | 통과 |
| `@property` 선언 golden(`inherits: true`) | 통과 |
| alias 토큰 golden(`initial-value`는 literal, `var()`는 `:root` 선언에만) | 통과 |
| rename 후 StableId 참조 유지, parser `var()`가 새 이름으로 같은 StableId를 냄 | 통과 |

## 남은 위험 / 한계

- 접미사를 붙인 이름이 다른 항목의 기본 이름과 다시 겹치는 경우(이름이 `x-<32hex>` 꼴)는 따로 검사하지 않습니다.
  32자리 hex 이름의 토큰을 의도적으로 만들 때만 생깁니다.
- 실제 토큰 집합(`crates/bindings`)과 Web export(XXII.2.x)에 연결하는 일은 codegen-css lane이 이 API로 합니다.
