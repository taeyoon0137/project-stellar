# 작업 보고서: XXII.4.3 Context를 CompositionLocal로, 상태 머신·MotionSpec을 Compose 상태·animation API로

## WBS

- WBS: XXII.4.3
- 작업명: Context를 CompositionLocal로, 상태 머신과 `MotionSpec`을 Compose 상태·animation API로 내보냄. 값 변경 빈도에 따라
  추적형과 비추적형 local을 고르는 규칙을 고정
- 상태: 부분 완료 (`[~]`) — local 선택 규칙과 그 fixture, CompositionLocal 출력 형태, 상태 변수의 상태 holder
  (`mutableStateOf`)는 끝났습니다. CodegenIR에 Context·상태 머신·`MotionSpec`이 아직 없어(XXII.1.5·XXII.1.6 `[ ]`, XXX.7.4
  `[ ]`) IR을 읽는 lowering과 "Context slot fixture 결과가 XXX.7.4와 같음" 검증은 하지 못했습니다.
- 결정: ADR-0107 결정 7, ADR-0025 결정 3.11·4.3·4.4
- 선행: XXII.4.1(이 브랜치), XXII.1.5·XXII.1.6·XXII.1.7(`[ ]`)

## 변경 파일

- `crates/codegen/src/android/context.rs`(신규) — `local_kind`(provision 값이 모두 Fixed면 `staticCompositionLocalOf`, 하나라도
  Changing이면 `compositionLocalOf`), `Local<접두어><이름>` 선언, `CompositionLocalProvider(… provides …)`, `.current`.
- `crates/codegen/src/android/mod.rs` — module 등록.
- 상태 변수 → `class <prefix>State { var … by mutableStateOf(…) }`는 XXII.4.1 커밋에 있습니다.
- `docs/task-reports/XXII.4.3.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC. Kotlin 컴파일러는 없습니다.

## 테스트

- `android::context::tests::local_selection_rule_follows_value_change`(검증 기준 local 선택 규칙 fixture): provision 없음·
  모두 Fixed → 비추적형, Changing 하나 이상 → 추적형.
- `android::context::tests::context_keys_lower_to_composition_locals`: local 이름, 문자열·색 default 선언, provider, 읽기,
  Double literal.

## 성능

- 해당 없음.

## 차이

- 규칙의 근거: 바뀌지 않는 값에 추적형을 쓰면 읽기 추적 비용만 들고, 자주 바뀌는 값에 비추적형을 쓰면 공급 subtree 전체를
  다시 구성합니다. 그래서 key 단위로 provision 값의 변경 여부만 봅니다(ADR-0107 결정 7).
- `MotionSpec` lowering 코드는 IR 타입이 없어 만들지 않았습니다. 규칙(cubic-bezier → `tween(…, CubicBezierEasing(…))`,
  spring → `spring(dampingRatio, stiffness)` 환산, steps → 계단 `Easing`, reduced motion → 근사)은 ADR-0107 결정 7에 적었습니다.

## 남은 위험

- provision 값의 Fixed·Changing 판정은 IR(XXII.1.6)이 값 식의 의존(상태·환경)을 보존해야 할 수 있습니다. 그 행이 의존 정보를
  싣지 않으면 이 규칙의 입력을 만들 수 없습니다.
- Kotlin 컴파일 확인이 없습니다.
