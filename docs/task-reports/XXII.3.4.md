# 작업 보고서: XXII.3.4 Context를 Apple Environment로, 상태 머신·MotionSpec을 상태·animation API로

## WBS

- WBS: XXII.3.4
- 작업명: Context를 Apple dialect의 Environment 값으로, 상태 머신과 `MotionSpec`을 dialect의 상태·animation API로 내보냄
- 상태: 부분 완료 (`[~]`) — 상태 변수의 상태 API(`@State`·`@Binding`·AppKit 속성)와 Context의 Environment 출력 형태는
  끝났습니다. CodegenIR에 Context·상태 머신·`MotionSpec`이 아직 없어(XXII.1.5·XXII.1.6 `[ ]`, XXX.7.4 `[ ]`) IR을 읽는
  lowering과 "Context shadowing·slot 결과가 XXX.7.4와 같음" 검증은 하지 못했습니다.
- 결정: ADR-0107 결정 7, ADR-0025 결정 3.11·4.3·4.4
- 선행: XXII.3.1(이 브랜치), XXII.1.5·XXII.1.6·XXII.1.7(`[ ]`, 다른 작업이 진행 중)

## 변경 파일

- `crates/codegen/src/apple/context.rs`(신규) — ContextKey → `EnvironmentKey` + `EnvironmentValues` 속성(접두어로 SwiftUI 기본
  속성과 겹치지 않음), provision·slot 다시 공급 `.environment(\.속성, 값)`, 소비 `@Environment(\.속성)`. AppKit은
  Environment가 없어 오류.
- `crates/codegen/src/apple/mod.rs` — module 등록.
- 상태 변수 → 상태 API는 XXII.3.1 커밋에 있습니다(`swiftui.rs`의 `@State private var`·modifier `@Binding`, `appkit.rs`의
  `didSet` 속성과 `setState`).
- `docs/task-reports/XXII.3.4.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC(Xcode Swift 6.4.2, macOS SDK 27.2)

## 테스트

- `apple::context::tests::context_keys_lower_to_environment_values`: key 타입·속성 이름, P3 색 default, 공급·읽기 형태, AppKit 오류.
- `apple::context::tests::environment_source_typechecks`: key 선언, 바깥·안쪽 공급(안쪽이 가리는 구조), 읽기를 macOS 13 SDK로
  typecheck.

## 성능

- 해당 없음.

## 차이

- `MotionSpec`·상태 전환 lowering 코드는 만들지 않았습니다. IR에 timing function 타입이 없어 adapter가 따로 타입을 만들면
  XXII.1.5 IR과 두 벌이 됩니다. 낮추는 규칙(cubic-bezier → `.timingCurve`, spring → `.interpolatingSpring(mass:stiffness:
  damping:initialVelocity:)`, steps → 근사, reduced motion → `accessibilityReduceMotion`)은 ADR-0107 결정 7에 적었습니다.
- AppKit의 Context는 렌더 트리 전달 API가 없어 생성자 주입으로 낼 계획이며(결정 7) 아직 코드가 없습니다.

## 남은 위험

- XXII.1.5·XXII.1.6·XXII.1.7과 XXX.7.4가 들어온 뒤 IR의 provision·소비 출처를 읽어 이 출력 형태로 내고, shadowing·slot fixture를
  XXX.7.4 결과와 맞춰야 합니다.
