# 작업 보고서: XXII.4.2 layout·control semantics·접근성 → Compose API 매핑

## WBS

- WBS: XXII.4.2
- 작업명: layout, control semantics, accessibility를 Compose API로 매핑
- 상태: 완료 (`[x]`)
- 선행: Phase X(phase 수준 선행 — 완화, 아래 "차이" 참조), IX.8.6(기준 브랜치 `[x]`)

## 변경 파일

- `crates/codegen/src/android/mod.rs` (신규) — `export_compose`, `ComposeOptions`, `ComposeExport`, `ComposeNote`·
  `ComposeNoteKind`·`ComposeFacet`·`LayoutFacet`, `ComposeError`.
- `crates/codegen/src/android/compose.rs` (신규) — IR → `@Composable` 함수 본문 매핑(layout·control semantics·접근성),
  재귀 없는 writer.
- `crates/codegen/src/android/api.rs` (신규) — 생성 코드가 쓰는 Compose API 표(import·사용 형태·최소 Compose UI 버전).
- `crates/codegen/src/android/check.rs` (신규) — Kotlin 정적 검사 `check_kotlin`.
- `crates/codegen/src/android/kotlin.rs` (신규) — Kotlin 문자열 literal(`$` template escape), `Dp`·`Float`·`Double`
  literal, 예약어 회피 식별자.
- `crates/codegen/src/android/tests.rs` (신규) — 검증 테스트.
- `crates/codegen/src/lib.rs` (수정) — `android` module.
- `crates/codegen/tests/fixtures/compose_android.kt` (신규) — golden.
- `docs/guides/crate-map.md` (수정) — codegen module 목록.
- `docs/task-reports/XXII.4.2.md`(신규), 계획서(XXII.4.2 `[ ]`→`[x]`)

## 구현 요약

- **layout**: Free → `Box` + 자식 `align`(끝 anchor)·`offset`, Stack → `Column`/`Row`(wrap은 `FlowColumn`/`FlowRow` +
  `@OptIn(ExperimentalLayoutApi::class)`), main 정렬 + gap → `Arrangement.spacedBy(gap, Alignment.…)`/`Arrangement.…`,
  cross 정렬 → `Alignment.…`(Stretch는 자식 `fillMax*()`), Grid → `LazyVerticalGrid(GridCells.Fixed|FixedSize)` +
  `item(span = { GridItemSpan(k) })`, Overlay → `Box(contentAlignment)`, sizing → `size`/`width`/`height`/`weight`/
  `fillMax*`, limits → `widthIn`/`heightIn`, aspect → `aspectRatio`, padding → `padding`/`absolutePadding`, 이동 transform
  → `offset`. modifier 순서는 부모 scope → 크기 → 상호작용·semantics → padding입니다. 보이지 않는 node는 만들지 않습니다.
- **control semantics**: button → `clickable(enabled, role = Role.Button, onClick)`, radio → `selectable(selected, role =
  Role.RadioButton, onClick)`, radioGroup·선택 list → `selectableGroup()`, list·listItem → `collectionInfo`·
  `collectionItemInfo`, textField → `BasicTextField(value = 상태, onValueChange = { 상태 = it }, enabled, readOnly)`,
  dialog → `Dialog(onDismissRequest) { … }`.
- **접근성**: label → `contentDescription`(dialog는 `paneTitle`), value → `stateDescription`, image → `role = Role.Image`,
  disabled → control `enabled`/`disabled()`, selected·focused → `selected`·`focused`, 숨김 subtree →
  `clearAndSetSemantics { }`, 읽기 순서가 시각 순서와 다르면 부모 `isTraversalGroup = true` + 자식 `traversalIndex`(Compose
  UI 1.5 이상, 미만이면 노트).
- 상태 변수는 `var 이름 by remember { mutableStateOf(초기값) }`, semantic action은 `onAction("<StableId>")`입니다.
  import는 API 표 symbol 중 실제 쓴 것만 정렬해 씁니다. 근사·대응 없음은 노트로 남깁니다(Constraint, Space\*와 gap, flex
  basis, 이미지 리소스, Free 양끝 anchor·백분율 offset, 회전·확대 transform 등).
- **정적 검사**: package 선언, 문자열 밖 괄호 짝, 닫히지 않은 문자열, 표에 없는·중복·미정렬·미사용 import, 사용 형태별
  import 누락, profile Compose UI 버전보다 새 API, Flow layout opt-in 누락.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (40 passed) |
| `cargo fmt -p stellar-codegen -- --check` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo xtask check-core` | 65 (fmt·clippy 통과, test 단계에서 `stellar-project` `restricted_yaml_fuzz`의 시간 상한 3s 초과로 실패 — 아래 "남은 위험") |
| `cargo test --workspace --locked --no-fail-fast -- --skip restricted_yaml_fuzz_campaign_has_no_findings` | 0 (test binary 123개 모두 통과) |
| `cargo test --workspace --doc --locked` | 0 |
| `cargo test -p stellar-project --locked --test restricted_yaml_fuzz` | 101 (단독 재실행도 4.95s·3.14s로 상한 초과, load average 약 50) |
| `cargo xtask validate-task-report docs/task-reports/XXII.4.2.md` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음

## 테스트

- `android_profile_maps_to_golden_compose_and_passes_static_check`: Android profile IR의 Compose 소스가 golden
  `compose_android.kt`와 같고(두 번 생성해도 같음) 정적 검사 issue 0건(검증 기준 static fixture).
- `layout_control_and_accessibility_use_compose_api`: Column·Row·Grid·크기·padding·offset, radio `selectable`, 버튼
  `clickable(enabled = !(isSaving))`, `selectableGroup`, `BasicTextField`, `Dialog`·`paneTitle`, `contentDescription`,
  `Role.Image`, `clearAndSetSemantics`, `isTraversalGroup`·`traversalIndex`, 상태 선언, flex·이미지 노트.
- `compose_ui_version_gates_traversal_semantics`: Compose UI 1.4 profile은 traversal API를 쓰지 않고 노트를 남기며, 1.7
  소스를 1.4 기준으로 검사하면 `RequiresNewerCompose` 2건.
- `wrapping_stack_uses_flow_layout_with_opt_in`: wrap Stack → `FlowRow` + opt-in, opt-in을 지우면 `MissingOptIn`.
- `static_check_detects_each_violation`: package 없음·import 누락·표에 없는 import·중복·미사용·괄호·문자열·정렬 위반을
  각각 검출.
- `invalid_targets_and_options_are_rejected`, Overlay 정렬 상수 9종, API 표 일관성, Kotlin literal, symbol 사용 형태 판정.
  남은 경고 0.

## 성능

- 해당 없음.

## 차이

- 검증 열 "compile/static fixture" 중 static fixture로 충족했습니다. 이 환경에는 Compose runtime·foundation 라이브러리
  (Android AAR)와 Compose 컴파일러를 함께 쓰는 빌드가 없어 Kotlin 컴파일은 하지 않았습니다. 대신 생성 symbol을 API 표로
  제한하고 import·사용 형태·버전·opt-in·괄호를 정적으로 검사합니다.
- 선행 `Phase X`는 행 ID가 아닌 phase 수준 선행이라 기존 관례대로 완화했습니다. 기준 브랜치 Phase X는 X.3.6·X.11.3
  `[ ]`, X.10.4 `[!]`, X.10.7·X.11.2 `[~]`가 남아 있으며, 이 매핑은 기준 브랜치 `stellar-layout` 입력 모델이 가진 layout
  기능(Free·Stack·Grid·Overlay·Constraint, sizing·limits·aspect·flex)만 다룹니다. baseline 정렬(X.3.6)은 입력 모델에
  없어 매핑하지 않았습니다.
- 파일 단위 Compose 출력(Android TargetProfile에 맞춘 파일 구성, XXII.4.1)과 결정론·출력 정책(XXII.5)은 별도 행입니다.
  이 행은 매핑 검증용으로 `@Composable` 함수 하나를 씁니다.

## 남은 위험

- `check-core` test 단계의 `restricted_yaml_fuzz_campaign_has_no_findings`(stellar-project, 기준 브랜치 코드)가 실행 시간
  상한 3s를 넘어 실패합니다. 이 lane은 project crate와 그 의존 crate를 바꾸지 않았고(codegen은 project에 의존하지 않고
  project도 codegen에 의존하지 않음), 여러 lane이 동시에 빌드·테스트하는 부하에서 단독 재실행도 실패했습니다. 그 테스트만
  건너뛴 workspace 전체 테스트와 doc test는 통과했습니다. 부하가 없는 환경에서 다시 확인해야 합니다.
- Kotlin 컴파일 검증이 없어 API 표의 시그니처 해석이 실제 Compose 버전과 다르면 컴파일 단계에서야 드러납니다. Compose
  의존성을 받을 수 있는 환경에서 컴파일 fixture를 추가해야 합니다.
- `LazyVerticalGrid`는 스크롤 가능한 부모 안에서 높이 제약이 필요합니다. 실행 시 layout 검증은 XXII.4.1·XXII.5 범위입니다.
- 이미지 리소스·도형 그리기·색은 이 행 범위 밖이라 `Box`로 둡니다(노트).
