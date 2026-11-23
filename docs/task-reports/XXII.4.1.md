# 작업 보고서: XXII.4.1 Android TargetProfile에 맞춰 Compose output 생성

## WBS

- WBS: XXII.4.1
- 작업명: Android TargetProfile에 맞춰 Compose output을 생성
- 상태: 부분 완료 (`[~]`) — Compose 생성기와 golden source는 끝났습니다. DEV-MAC에 Kotlin 컴파일러가 없어 compile fixture
  대신 정적 구조 검사를 했고, 선행 XXII.1.1이 `[!]`이라 IR fixture로만 검증했습니다.
- 결정: ADR-0107 결정 2·6, ADR-0025 결정 4, ADR-0049
- 선행: XXII.1.1(`[!]`, IR 모델·`build_ir`는 있음)

## 변경 파일

- `crates/codegen/src/android/mod.rs`(신규) — `export_compose`, `ComposeOptions`(package·접두어), `ComposeExport`,
  `AndroidNote`·`AndroidNoteKind`, `ComposeError`.
- `crates/codegen/src/android/compose.rs`(신규) — 생성기: `<prefix>Colors`, 상태 holder `<prefix>State`, root
  `<prefix>View(state, onAction)`, node마다 부모 scope receiver를 받는 `private fun <Scope>.Node<id>()`, element·layout
  container, 사용한 이름만 import.
- `crates/codegen/src/android/kotlin.rs`(신규) — 문자열 literal(`$` 포함 escape), `Float`·`Dp` literal, 예약어 회피 식별자.
- `crates/codegen/src/android/color.rs`(신규) — 색공간 → `Color(…, colorSpace = ColorSpaces.*)`.
- `crates/codegen/src/android/layout.rs`(신규) — `COMPOSE_RULES`(XXII.4.2에서 검증).
- `crates/codegen/src/android/tests.rs`(신규), `crates/codegen/src/android/golden/android_compose.kt`(신규 golden).
- `crates/codegen/src/lib.rs` — `android` module 등록.
- `crates/codegen/src/apple/native.rs`·`swiftui.rs`·`appkit.rs`·`mod.rs` — main 축 정렬 함수(`justify_edge`)를 공용 분석으로
  옮기고 test module을 crate 안에서 쓸 수 있게 함(출력 변화 없음).
- `docs/task-reports/XXII.4.1.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (71 통과) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-codegen -- --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC. Kotlin·Android Gradle 도구는 없습니다(`kotlinc` 없음).

## 테스트

- `android::tests::compose_source_matches_golden_static_fixture`(검증 기준 golden source): Android profile IR의 Compose 소스가
  golden과 byte 단위로 같고 두 번 생성해도 같음. 같은 소스에 정적 검사(괄호 짝, import 이름 사용, 사용한 Compose 이름의
  import)를 적용.
- `android::tests::compose_output_hoists_state_and_maps_elements`: 상태 holder(`mutableStateOf`), root composable 기본값,
  부모 scope receiver, `BasicText`, `BasicTextField`의 상태 읽기·쓰기와 placeholder, 색공간(P3·확장 sRGB), 이미지·도형·Modal 노트.
- `android::tests::invalid_targets_and_options_are_rejected`, `static_check_detects_broken_sources`, Kotlin literal·색 단위 test.

## 성능

- 해당 없음. node마다 함수 하나를 두어 재귀 없이 씁니다.

## 차이

- compile fixture는 실행하지 못했습니다. 정적 검사는 Kotlin 문법·타입을 검사하지 않습니다. Kotlin 컴파일러를 받으려면 외부
  다운로드가 필요해 이 작업에서 하지 않았습니다.
- Material 컴포넌트 대신 foundation(`BasicText`, `BasicTextField`, `Row`·`Column`·`Box`)만 씁니다. 디자인한 모양을 Material 기본
  모양이 덮지 않게 하기 위해서입니다(ADR-0107 결정 6).
- SF Symbol 같은 Apple 전용 이미지 원본은 Android에서 쓰지 않습니다(빈 `Box`와 `ImageSourceUnavailable` 노트).

## 남은 위험

- Kotlin 컴파일 확인이 없어 Compose API 이름·overload 오류가 남아 있을 수 있습니다. Android 빌드 환경에서 compile fixture를
  돌려야 합니다.
- 실제 문서에서 IR을 만드는 연결이 없습니다(XXII.1.1 `[!]`).
