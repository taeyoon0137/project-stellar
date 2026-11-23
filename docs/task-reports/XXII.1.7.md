# 작업 보고서: XXII.1.7 matrix에 Context·`MotionSpec` 항목 추가

## WBS

- WBS: XXII.1.7
- 작업명: XXII.1.4 matrix에 Context·`MotionSpec` 항목의 Web·Apple·Android profile kind별 상태와 lowering 규칙 추가
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXII.1.4, XXII.1.5, XXII.1.6이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 선행 XXII.1.6이 부분 완료라 원장 표기는 통합 때 정합니다.
- 결정: ADR-0106 결정 3, ADR-0025 결정 1.2·4.2·4.3·4.4
- 선행: XXII.1.4, XXII.1.5, XXII.1.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

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

## 실행한 명령 / 검증

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

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험

- Apple·Android template은 lowering 규칙의 대상 표기입니다. 실제 source와 compile 검사는 XXII.3.4·XXII.4.3 몫입니다.
- Android 추적형·비추적형 선택 규칙의 값 변경 빈도 판정은 XXII.4.3에서 고정합니다.

## 후속 작업 (2026-11-23, `lane/codegen-registry`, ADR-0121 결정 4)

- `BehaviorRow`에 AppKit dialect 열을 더했습니다(`behavior::dialect_entry`).
  - AppKit에는 렌더 트리 전달 값이 없습니다. 그래서 Context는 IR이 해석한 값의 생성자 주입(`appkit-resolved-value`)입니다.
  - 바뀌는 값은 Approximated(`context-manual-propagation`)입니다.
- Context 항목 `context-value-fixed`·`context-value-changing`을 더했습니다.
  - Compose 추적형·비추적형 CompositionLocal은 값 변경 빈도로 고릅니다. `android::context::local_kind`와 test로 대조합니다.
  - 스타일 값·그 밖의 값 항목의 Compose 열은 `composition-local-by-change`로 바꿨습니다.
- timing function 상태를 adapter 결정과 맞췄습니다.
  - steps: SwiftUI Approximated(`CustomAnimation`), AppKit Native(discrete keyframe), Compose Native(계단 `Easing`).
  - spring: 모든 dialect Approximated(ADR-0094 고정 step 적분과 다름). ADR-0107 결정 7의 spring Native를 대체합니다.
- CSS 기반 속성 항목은 이 커밋에서 바꾸지 않았습니다(`matrix_css.txt` 불변 test 통과).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen matrix` | 통과(16) |

- 검증 대응:
  - 완전성: `every_context_and_motion_item_has_a_rule_for_every_profile_kind`. profile kind와 dialect 열 모두 봅니다.
  - adapter 대조: `adapters_follow_the_context_rules_of_the_matrix`.
  - fixture: `matrix_behavior.txt`(17항목 × 4열).
