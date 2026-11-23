# 작업 보고서: XXII.1.7 matrix에 Context·`MotionSpec` 항목 추가

## WBS

- WBS: XXII.1.7
- 작업명: XXII.1.4 matrix에 Context·`MotionSpec` 항목의 Web·Apple·Android profile kind별 상태와 lowering 규칙 추가
- 상태: 완료 보고(`reported_done`). 선행 XXII.1.6이 부분 완료라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0106 결정 3, ADR-0025 결정 1.2·4.2·4.3·4.4
- 선행: XXII.1.4, XXII.1.5, XXII.1.6

## 변경 파일

- `crates/codegen/src/matrix/behavior.rs`(신규): `MotionFeature`(10), `ContextFeature`(5), `BehaviorSubject`, 규칙 표
  `BEHAVIOR_LOWERINGS`(항목 × Web·Apple·Android의 상태·규칙·template·진단), `behavior_entry`, `apply`.
- `crates/codegen/src/matrix/mod.rs`: module 등록과 문서.
- `crates/codegen/src/matrix/rule_tests.rs`: 완전성·fixture·불변 test.
- `crates/codegen/tests/fixtures/matrix_behavior.txt`(신규).

## 구현 요약

- MotionSpec 항목: timing function 네 종류, reduced motion 두 종류, 시작 조건 두 종류, 지연, Color 보간.
  - spring: Web은 `linear()` 표본 곡선(Approximated, `motion-spring-approximated`), Apple은 `interpolatingSpring`
    (Approximated, 적분 방식 차이), Android는 `spring(dampingRatio, stiffness)`(Approximated, 질량 정규화).
  - steps: Apple은 대응 API가 없어 Unsupported, Android는 사용자 정의 Easing(Approximated).
  - Color 보간: 세 profile 모두 encoded premultiplied 보간 공간을 지정할 수 없어 Approximated(ADR-0025 결정 1.2).
  - Android reduced motion은 시스템 animator scale로 판정(Approximated).
- Context 항목: 스타일 값, 그 밖의 값, 같은 key 재읽기, 깊이가 위치마다 다름, slot 재공급.
  - Web: 등록 custom property 상속, context 구조, 깊이 이름 재명명, 깊이가 다르면 context 구조 대체(Approximated,
    `context-depth-varies`), slot outer 재공급.
  - Apple: Environment 값, `transformEnvironment`(바깥 값 읽기), slot 재공급.
  - Android: 추적형(`compositionLocalOf`)·비추적형(`staticCompositionLocalOf`) local, slot 재공급.
- Native가 아닌 항목은 모두 진단이 짝으로 있습니다. CSS 기반 속성 표는 바꾸지 않았습니다.

## 명령과 결과

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen matrix` | 통과(8) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- 완전성 검사: `matrix::rule_tests::every_context_and_motion_item_has_a_rule_for_every_profile_kind`.
- XXII.1.4 항목 불변: `context_and_motion_items_leave_css_items_unchanged`(`matrix_css.txt` golden 그대로, 이 커밋에서
  파일 변경 없음).
- 규칙별 입력·출력·진단 fixture: `context_and_motion_rules_have_input_output_and_diagnostic_fixture`
  (`matrix_behavior.txt`, 15항목 × 3 profile).

## 남은 위험

- Apple·Android template은 lowering 규칙의 대상 표기입니다. 실제 source와 compile 검사는 XXII.3.4·XXII.4.3 몫입니다.
- Android 추적형·비추적형 선택 규칙의 값 변경 빈도 판정은 XXII.4.3에서 고정합니다.
