# 작업 보고서: XXII.2.3 토큰을 CSS custom property·생성 상수로 내보내기

## WBS

- WBS: XXII.2.3
- 작업명: token을 CSS custom property 또는 generated constants로 내보냄(IX.11.16 이름, 속성 모델 10장 등록 정책)
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 IX.11.8이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 선행 IX.11.8이 부분 완료라 원장 표기는 통합 때 정합니다.
- 결정: ADR-0106 결정 1.5·5, ADR-0075 결정 7, 속성 모델 10장
- 선행: Phase VIII, IX.11.8, IX.11.16

## 필수 실행 환경

- `DEV-MAC`. 기대값 생성은 로컬 Google Chrome 157.0.8103.79(`--headless=new`, DevTools `Emulation.setEmulatedMedia`)와
  Python 3 표준 라이브러리, test는 순수 Rust입니다(IX.11.8과 같은 기준 브라우저).

## 변경 파일

- `crates/codegen/src/ir/token_env.rs`(신규): 환경 조건 지역 거울(`IrEnvironmentValue` 등), `IrTokenVariant`, 조건 정규화.
- `crates/codegen/src/ir/{mod,build,snapshot,tests}.rs`: `TokenDecl`·`IrToken`의 alias·조건부 값, 검사(alias 대상,
  조건 key 중복·조합 중복·빈 조건), snapshot `when` 줄, `IrTree::from_nodes`(adapter 입력용 tree 생성).
- `crates/codegen/src/web/tokens.rs`(신규): 선언 위치 계획 `plan_tokens`(CSS 토큰·media 블록·생성 상수·
  interaction 토큰·진단), media cascade 계산.
- `crates/codegen/src/web/js.rs`(신규): JavaScript 문자열·값·식별자 표기(XXII.5.1에서도 씀).
- `crates/codegen/src/web/mod.rs`, `crates/codegen/src/fixture.rs`, `crates/codegen/tests/fixtures/ir_*.snapshot`.
- `crates/ops/src/export_web_tokens.rs`(신규): 문서 `derive_names`·`literal_text`·`root_block`로 `tokens.css`·`tokens.js` 조립.
- `crates/ops/src/export_web_tokens/tests.rs`(신규), `crates/ops/src/export_web_tokens/fixture/{tokens.css,expected.txt,
  generate.py,cdp.py}`(신규).
- `crates/ops/src/lib.rs`: module 등록. `docs/adr/ADR-0106-…md`: 결정 5의 media 조합·상수 규칙을 구현과 맞춤.

## 구현 요약

- 이름: 토큰과 ContextKey를 한 이름 공간으로 문서 `derive_names`가 파생합니다. 충돌은 `name-collision` 진단입니다.
- 값: CSS 값 텍스트가 있는 토큰(문서 `literal_text`)은 `@property`(`inherits: true`, 기본 환경 literal
  `initial-value`)와 `:root` 선언, 조건부 값은 `:root`를 감싼 `@media` 블록입니다. alias는 `:root`의 `var()`입니다.
- media 조합: `color_scheme` dark, `contrast` high, `reduced_motion` true의 부분집합을 (크기, key 순)으로 보고,
  cascade 결과와 다른 값만 냅니다. 조건에 나온 key 밖 key는 값을 바꾸지 않고, 값이 없는 조합은 기본 값입니다.
  `viewport_*`·`density`·`locale` 등은 `token-environment-unsupported` 진단을 내고 뺍니다.
- CSS 값 텍스트가 없는 토큰은 `tokens.js` 생성 상수(`Object.freeze`, key는 CSS 이름)입니다.
- `interaction_state` 조건부 토큰은 토큰 선언에서 빼고 계획의 `interaction`으로 넘깁니다(XXII.2.6).
- `@property` 문법: ops는 `ExpressionType`을 쓸 수 없어 literal variant로 문법을 정하고, 문서 `registered_syntax`·
  `property_rule`과 같은 표·template인지 문서 source와 대조하는 test를 둡니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `python3 crates/ops/src/export_web_tokens/fixture/generate.py` | Chrome 157.0.8103.79, media 조합 8개 × probe 6개 기록 |
| `cargo test -p stellar-ops --lib export_web_tokens` | 통과(4) |
| `cargo test -p stellar-codegen` | 통과(80 + 2) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- deterministic CSS: `token_css_is_deterministic_and_matches_the_committed_browser_input`(같은 입력·순서 바꾼 입력 동일,
  브라우저가 읽은 `tokens.css`와 byte 동일, `inherits: true`만, 값은 `:root`·`@media :root`에만).
- 출력 이름이 IX.11.16 golden과 같음: `output_names_follow_the_ix_11_16_derivation`(`--radius-lg`, `--Space-2`, 토큰
  `context/accent`와 ContextKey `accent` 충돌 진단).
- 브라우저 계산값: `registered_tokens_compute_to_root_and_media_values_in_the_browser` — `var()`를 쓰는 자손 요소의
  계산값이 조합 8개 모두에서 IR 값(기본·dark·high contrast·dark+high·reduced motion)과 같고, alias도 따라가며,
  `interaction_state` 토큰은 `:root`에 없음.
- 문서 함수 일치: `property_rule_template_matches_the_document_function`.

## 차이(작업 지시와 다른 점)

- `@property` 문법을 문서 `property_rule` 대신 ops 표로 냈습니다(crate 그래프). 문서 source 대조 test로 같음을 고정했습니다.

## 남은 위험

- 토큰 저장(bindings)이 문서 열기 경로에 없어 `TokenDecl`의 조건부 값을 채우는 평가기 연결이 없습니다. fixture 입력으로만
  검증했습니다.
- `@property` 문법 표가 ops에 한 번 더 있습니다(소스 대조 test로 고정). ops가 expressions 타입을 쓸 수 있게 되면 문서
  `property_rule` 호출로 바꿉니다.
