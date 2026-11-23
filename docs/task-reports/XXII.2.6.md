# 작업 보고서: XXII.2.6 상태 머신과 `MotionSpec`을 Web 출력으로 낮추기

## WBS

- WBS: XXII.2.6
- 작업명: CSS transition으로 표현 가능한 motion은 CSS, 상태 전환은 생성 코드, 표현할 수 없는 항목은 근사·미지원 진단,
  `interaction_state` 조건부 토큰은 Control 상태 selector
- 상태: 완료 (`[x]`) — 선행 XXII.1.7이 보고 상태라 원장 표기는 통합 때 정합니다.
- 결정: ADR-0106 결정 3·7, ADR-0025 결정 4.4, 속성 모델 10장
- 선행: XXII.1.5, XXII.1.7, XXII.2.1, XXII.2.3

## 필수 실행 환경

- `DEV-MAC`. 기대값 생성은 Google Chrome 157.0.8103.79(`--headless=new`, DevTools `CSS.forcePseudoState`)와 Python 3
  표준 라이브러리(XXII.2.3과 같은 기준 브라우저와 DevTools client), test는 순수 Rust입니다.

## 변경 파일

- `crates/codegen/src/web/motion.rs`(신규): `lower_motions`(transition longhand, `Becomes` 상태 selector, reduced motion
  media 블록, matrix 진단, spring `linear()` 표본), `EasingSampler`, `lower_interaction_tokens`(Control 상태 selector),
  `supported_states`, `state_selector`, `state_attr_value`, `css_string`, `rules_text`.
- `crates/codegen/src/web/state.rs`(신규): `lower_state_machines` → `state.js`.
- `crates/codegen/src/web/motion/tests.rs`(신규), `crates/codegen/src/web/mod.rs`.
- `crates/codegen/tests/fixtures/web_motion.css.txt`, `web_state.js`(신규).
- `crates/ops/src/export_web_motion.rs`(신규): 문서 registry CSS longhand 이름(`css_property_name`)과 문서 formatter로
  `export_behavior`.
- `crates/ops/src/export_web_motion/tests.rs`(신규), `fixture/{control.css,expected.txt,generate.py}`(신규).
- `crates/ops/src/lib.rs`: module 등록.

## 구현 요약

- motion: 대상 경로의 CSS longhand마다 `transition-property`·`-duration`·`-delay`·`-timing-function` 목록을 냅니다.
  `Becomes`는 export root의 `[data-s-<상태 hex>="<값>"]` 아래 규칙이라 그 상태로 들어갈 때만 재생되고, `Changes`는
  element 규칙입니다. reduced motion `instant`는 `@media (prefers-reduced-motion: reduce)` 안에서 그 motion을 뺀
  목록(없으면 `none`)입니다.
- timing: linear·cubic-bezier·steps는 그대로, spring은 호출자 `EasingSampler`(엔진 evaluator)의 33개 표본 `linear()`
  (`motion-spring-approximated`), sampler가 없으면 `motion-spring-unsupported`로 그 motion을 뺍니다. Color 대상은
  `motion-color-space-approximated`입니다. 진단 code는 XXII.1.7 matrix 표에서 읽습니다.
- 상태 전환: `state.js`가 상태 객체와 root `data-s-*` 속성을 두고, (element, DOM 이벤트)마다 listener 하나에서
  transition을 목록 순서로 보고 처음 참인 것 하나를 원자 commit합니다. emit·nested action은 `stellar:<action hex>`
  CustomEvent입니다. DOM 이벤트가 없는 종류와 JavaScript로 낼 수 없는 식은 진단합니다.
- 상호작용 상태 토큰: XXII.2.3 계획의 `interaction` 토큰을 쓰는 속성을 element 규칙(기본 값)과 상태 selector
  (`:hover`·`:focus-visible`·`:active`·`:disabled`)로 냅니다. Button은 네 상태, TextInput은 hover·focused·disabled,
  그 밖의 contract는 hover·focused이고, 지원하지 않는 상태와 contract 없는 element는 진단합니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `python3 crates/ops/src/export_web_motion/fixture/generate.py` | Chrome 157.0.8103.79, element 3 × 상태 5 + `:root` 기록 |
| `cargo test -p stellar-codegen` | 통과(90 + 2) |
| `cargo test -p stellar-ops --lib export_web_motion` | 통과(3) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- fixture output: `web::motion::tests::motions_become_transitions_with_state_selectors_and_reduced_motion`
  (`web_motion.css.txt`), `state_machines_become_event_listeners_with_atomic_commit`(`web_state.js`).
- 근사·미지원 진단 fixture 정확: `approximated_and_unsupported_motion_diagnostics_are_exact`.
- Control 상태별 계산값(브라우저 기대값 커밋): `ops::export_web_motion::tests::control_state_computed_values_in_the_browser_follow_the_contract_states`
  — Button 다섯 상태, TextInput(pressed는 기본 값), contract 없는 Frame(모두 기본 값)이 IR 토큰 값과 같음.
- 그 토큰이 `:root` 선언에 없음: 같은 fixture의 `:root` 값 빈 문자열과
  `interaction_rules_match_the_committed_browser_input_and_diagnose_unsupported_states`.

## 차이(작업 지시와 다른 점)

- spring은 호출자 sampler가 있을 때만 근사 출력이고, 지금 제품 경로에는 sampler 연결이 없습니다.

## 남은 위험

- 상태에 따라 바뀌는 속성 binding의 Web 출력은 아직 없어, transition 선언은 있지만 값 변화는 다른 경로가 만들어야
  합니다(ADR-0106 Consequences).
- 엔진 spring evaluator(`stellar_prototype::timing`)를 `EasingSampler`로 잇는 호출자는 아직 없습니다(ops가 prototype에
  닿지 못함). 지금 제품 경로에서는 spring이 미지원 진단입니다.
- `state.js`의 `read`는 상태 값만 제공합니다. 토큰·속성·환경·Context·Component Property를 읽는 식의 transition은
  `state-reference-unsupported` 진단과 함께 빼고, transition이 모두 빠진 listener는 만들지 않습니다
  (`state_transitions_with_unavailable_references_or_events_are_diagnosed_and_skipped`). 생성 JavaScript 실행 test는
  없습니다.
