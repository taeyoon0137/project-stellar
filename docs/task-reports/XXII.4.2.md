# 작업 보고서: XXII.4.2 layout, control semantics, accessibility를 Compose API로 매핑

## WBS

- WBS: XXII.4.2
- 작업명: layout, control semantics, accessibility를 Compose API로 매핑. layout 변환과 진단은 XXII.1.4 matrix 규칙만 씀
- 상태: 부분 완료 (`[~]`) — 매핑·진단과 static fixture는 끝났습니다. 규칙 표는 XXII.1.4 matrix의 Android 열(CSS layout
  longhand 부분)로 이 module에 두었고, XXII.1.4 행(`[ ]`)이 registry에 등록해야 합니다. Kotlin 컴파일은 하지 못했습니다.
- 결정: ADR-0107 결정 3·4·6, ADR-0025 결정 1.3·4.2
- 선행: Phase X, IX.8.6(`[x]`), XXII.1.4(`[ ]`)

## 변경 파일

- `crates/codegen/src/android/compose/semantics.rs`(신규) — Control 의미(`clickable`·`selectable`·`selectableGroup`)와 접근성
  (`semantics`·`clearAndSetSemantics`, 역할·이름·값·상태·동작, `isTraversalGroup`·`traversalIndex`, `collectionInfo`).
- `crates/codegen/src/android/compose.rs` — 상호작용·접근성 modifier를 크기 modifier 바깥에 연결, import 표 추가.
- `crates/codegen/src/android/semantics_tests.rs`(신규), `crates/codegen/src/android/golden/layout_compose.kt`(신규 golden),
  `crates/codegen/src/android/golden/android_compose.kt`(semantics 반영 갱신), `crates/codegen/src/android/tests.rs`,
  `crates/codegen/src/android/mod.rs`.
- `docs/task-reports/XXII.4.2.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (75 통과) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-codegen -- --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC. Kotlin·Android Gradle 도구는 없습니다.

## 테스트

- `android::semantics_tests::controls_and_accessibility_map_to_compose_semantics`(static fixture): Button `clickable`(비활성
  binding → `enabled`, 눌림 표시 없음), radio `selectable`(선택 식, 선택 상태 쓰기), 그룹 `selectableGroup`, image 역할·이름,
  병합 semantics, dialog `paneTitle`·`dismiss`, 숨김 `clearAndSetSemantics`, 읽기 순서 `traversalIndex`.
- `android::semantics_tests::reading_order_needs_compose_ui_1_6`: Compose UI 1.5 profile은 traversal API를 쓰지 않고 노트.
- `android::semantics_tests::compose_rules_cover_every_longhand`: layout longhand 40개 모두 Compose 규칙이 하나씩 있음.
- `android::semantics_tests::layout_findings_follow_compose_rules`: Apple과 같은 layout fixture의 규칙별 진단과 출력. wrap은
  `FlowRow`(Native), 비율이 다른 fr·grow는 `weight` 그대로(Native), 최소 auto fr(Approximated), `%` 크기(Approximated
  `fillMaxWidth(비율)`), min-content 최소 크기(Unsupported), space-between + gap(Approximated), 회전은 그리기 변환
  (Approximated), content-box 순서, absolute `BottomEnd` 정렬, relative offset, grid 고정 열·행, display none 제외, baseline.
  golden과 정적 구조 검사.

## 성능

- 해당 없음. 숨김·읽기 순서·목록 위치는 IR 전체에서 한 번 계산합니다.

## 차이

- Compose는 SwiftUI와 규칙이 다른 곳이 있어 상태가 다릅니다. `weight`는 비율을 그대로 받아 basis 0 grow·fr 비율이 Native입니다.
  `Arrangement.Space*`는 gap과 함께 쓸 수 없어 Approximated입니다.
- dialog·modal은 Compose에 대응 역할이 없어 `paneTitle`로 근사합니다(노트).
- compile fixture 대신 static fixture(golden + 정적 구조 검사)입니다. 검증 기준이 compile/static fixture라 static으로 충족했지만
  Kotlin 컴파일 확인은 남았습니다.
- 검토 보완: semantics·`clickable`·`selectable`·`Role`·`MutableInteractionSource` 등 24개 이름이 import 표에 없어
  생성 Kotlin 파일에 import 줄이 빠졌습니다(컴파일되지 않는 출력). 정적 검사도 표에 있는 이름만 보아 놓쳤습니다. 표에
  이름을 더하고 표에 없는 이름을 쓰면 debug assert로 실패하게 했습니다.

## 남은 위험

- `role`·`selected`는 `clickable`·`selectable`의 이름 있는 인자와 철자가 같아 정적 검사의 "쓴 이름 → import" 방향에서 뺐습니다.
- Compose API overload 오류는 Kotlin 컴파일 없이는 잡히지 않습니다.
