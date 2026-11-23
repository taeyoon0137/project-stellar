# 작업 보고서: XXII.2.5 Context를 Web 출력으로 낮추기

## WBS

- WBS: XXII.2.5
- 작업명: Context를 등록 custom property 상속·생성 코드 context 구조로 낮춤(같은 key 재읽기 깊이 이름, 깊이가 다른
  provision의 구조 대체, slot 재공급, 이름 충돌 진단)
- 상태: 부분 완료 (`[~]`) — XXX.7.4 해석기 대조 기준이 남았습니다.
- 결정: ADR-0106 결정 6, ADR-0025 결정 3.5·3.11·4.3, 속성 모델 10장
- 선행: XXII.2.3, XXII.1.6, XXII.1.7, IX.11.16, XXX.7.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

## 변경 파일

- `crates/codegen/src/web/expr.rs`(신규): `IrExpr` → CSS 식(literal·`var()`·`calc()`·`min()`/`max()`)과 JavaScript 식
  (닫힌 연산·함수 표). XXII.2.6도 씁니다.
- `crates/codegen/src/web/context.rs`(신규): `lower_contexts` — key 방식 결정, element별 선언, 깊이 이름, slot 재공급,
  이름 충돌, 소비 지점 `var()`, `context.js` 정적 표.
- `crates/codegen/src/web/context/tests.rs`(신규): custom property 상속 해석기(test 전용)로 출력을 해석해 비교.
- `crates/codegen/src/web/mod.rs`: module 등록.
- `crates/codegen/tests/fixtures/web_context.{css.txt,js}`(신규).
- `crates/ops/src/export_web_css.rs`(신규): 문서 `derive_names`·`literal_text`를 codegen `CssNames`로 넘기고, CSS key의
  `@property`를 토큰과 같은 함수로 냅니다.
- `crates/ops/src/export_web_tokens.rs`: test helper 공유(`pub(crate) mod tests`). `crates/ops/src/lib.rs`: 등록.

## 구현 요약

- CSS key: default와 모든 provision 값이 CSS 식이고 깊이 집합 원소가 하나인 key. default는 export root element,
  provision은 provider element에 선언합니다. 등록은 `inherits: true`·literal `initial-value`(토큰과 같은 함수)입니다.
- 같은 key 재읽기: 그 key는 깊이 이름을 씁니다. 깊이 d provision은 `<key>-<d>`에 쓰고 `<key>: var(<key>-<d>)`로 다시
  공급하며, 식 안의 같은 key 읽기는 `<key>-<d-1>`, default는 root의 `<key>-0`입니다.
- 깊이 집합 원소가 둘 이상이거나 CSS로 낼 수 없는 식이면 key 전체를 `context.js`로 냅니다(`context-depth-varies` 또는
  `context-structure` 정보 진단). 구조는 provision별 값 함수(`(read, outer) => …`)와 소비 지점별 출처(IR 결과)를 담고
  실행 시 다시 해석하지 않습니다.
- slot 재공급: Slot 공개 목록에 없고 instance–host 구간(instance 제외)에 provision이 있는 CSS key는 instance에서
  `-outer`로 보존하고 host에서 다시 공급합니다. 깊이 이름 key는 그 구간의 깊이 이름도 같이 옮깁니다.
- 깊이·보존 이름은 토큰·key 이름 공간과 충돌을 검사합니다(`name-collision`).

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(86 + 2) |
| `cargo test -p stellar-ops --lib export_web` | 통과(5) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- deterministic output: `web::context::tests::output_is_deterministic_and_golden`(`web_context.css.txt`).
- 중첩·shadowing·slot 해석값: `css_inheritance_of_the_output_matches_the_ir_resolution` — 출력 선언을 custom property
  상속 규칙으로 해석한 소비 값 4곳(shadowing 3 → 4, slot 너머 2 → 8, default → 11, 2단 재읽기 → 4)이 IR 출처로 계산한
  값과 같음.
- 같은 key 재읽기 2단 중첩: `same_key_reads_use_depth_names_without_self_referencing_var` — 자기 참조 `var()` 0건, 해석값
  `calc(calc(1 * 2) * 2)`.
- 깊이가 다른 fixture: `varying_depth_key_goes_to_context_structure_with_one_info_diagnostic` — 진단 1건, `context.js`
  golden.
- 이름 충돌: `depth_and_outer_names_colliding_with_tokens_are_diagnosed`.
- 문서 formatter·등록: `ops::export_web_css::tests::context_keys_use_document_names_values_and_registration`.

## 차이(작업 지시와 다른 점)

- 해석값 대조를 해석기 대신 IR이 보존한 해석 결과(합성 입력)로 했습니다.

## 남은 위험

- XXX.7.4 해석기 결과와 대조하는 test는 해석기가 생긴 뒤 더해야 합니다.
- 다른 key를 읽는 식은 그 key가 CSS key일 때만 CSS로 냅니다. 상속된 값 기준이라 해석기의 "provision 위치 기준" 의미와
  다를 수 있는 경우(다른 key를 provision보다 안쪽에서 바꾸는 구조)는 해석기 대조 test에서 다시 확인해야 합니다.
- 브라우저 실측은 하지 않았습니다(이 행 기준에 없음). 해석은 test 전용 상속 해석기로 했습니다.
