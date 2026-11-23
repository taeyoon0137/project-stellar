# ADR-0107: Apple·Android 코드 생성 adapter의 Control·layout·Context 낮춤과 CLI 연결

## Status

Accepted (2026-11-23).

- 계획 행 XXII.3.1, XXII.3.2, XXII.3.4(일부), XXII.4.1, XXII.4.2, XXII.4.3(일부)을 구현하는 결정입니다.
- ADR-0025 결정 4, ADR-0049, ADR-0074, ADR-0076, ADR-0082, ADR-0089를 따릅니다.

## Context

- CodegenIR(XXII.1.1)과 Apple 접근성·색 출력(XXII.3.3)은 있습니다. Control contract, layout, 상태를 SwiftUI·AppKit·Compose
  코드로 내는 adapter는 없었습니다.
- layout 입력은 CSS longhand 그대로입니다(`stellar_layout::style::LayoutStyle`, ADR-0076). Auto Layout 같은 비CSS 개념을
  만들지 않습니다(ADR-0074).
- ADR-0025 결정 4.2는 Apple·Android lowering 규칙을 XXII.1.4 capability matrix에 두라고 합니다. XXII.1.4·XXII.1.5·XXII.1.6·
  XXII.1.7은 아직 `[ ]`이고 같은 crate의 다른 작업이 진행 중입니다. 이 작업은 `crates/codegen/src/apple/**`·
  `crates/codegen/src/android/**`만 씁니다.
- Context 해석기(XXX.7.4)와 IR의 Context·`MotionSpec`(XXII.1.5·XXII.1.6)이 없습니다.
- DEV-MAC에는 Xcode(`swiftc`)가 있고 Kotlin 컴파일러는 없습니다.
- CLI는 `stellar-ops`에만 의존합니다(ADR-0005 §8). crate graph는 `ops → layout` edge를 허용하지 않습니다.

## Decision

1. **출력 진입점.**
   - `apple::export_apple_ui`: XXII.3.3 `export_apple` 출력(색 상수·접근성 modifier·함수)을 그대로 앞에 두고 view tree를
     더합니다. SwiftUI는 `struct <prefix>View: View`, AppKit은 `@MainActor final class <prefix>View: NSView`입니다.
     - SwiftUI는 node마다 `private var node<id>: some View` 속성을 둡니다. 식 하나가 커져 type-check가 느려지는 것을 막습니다.
     - 상태 변수는 SwiftUI `@State private var`(접근성 modifier에는 `$상태` binding), AppKit은 `didSet`에서 접근성을 다시
       적용하는 속성과 `setState`입니다. semantic action은 생성자가 받는 `onAction(<action StableId>)`입니다.
   - `android::export_compose`: Kotlin 파일 하나입니다. `object <prefix>Colors`, 상태 holder `class <prefix>State`
     (`mutableStateOf`), root `@Composable fun <prefix>View(state, onAction)`, node마다
     `@Composable private fun <부모 scope>.Node<id>(state, onAction)`입니다.
     - 부모 container의 scope(`RowScope`·`ColumnScope`·`BoxScope`·Flow scope)를 receiver로 받아 `weight`·`align`을 씁니다.
     - node마다 함수를 두어 재귀 없이 씁니다. import는 실제로 쓴 이름만 이름 순으로 냅니다.
   - 같은 IR과 옵션은 항상 같은 bytes입니다.
2. **공용 구조 분석(`apple::native`).** SwiftUI·AppKit·Compose가 같은 판정을 씁니다.
   - 출력 view 종류와 Control contract 선택을 정합니다. 한 node에 contract가 여럿이면 TextInput > Button > Modal >
     SingleSelectionGroup > List 순입니다. 고른 contract가 그 profile에서 Unsupported이면 contract 없는 container와 노트입니다.
   - Fragment는 펼치고, `display: none`은 빼고, `position: absolute`는 흐름에서 분리합니다.
   - 채움(block 폭, flex cross stretch, 확정 main 축의 grow, grid stretch, root 폭)과 확정 크기를 정합니다.
   - 작업 소유 경계 때문에 `apple` module 아래에 둡니다. Android도 이 module과 노트 타입(`ControlNote`, `LayoutFinding`)을
     씁니다. XXII.5에서 중립 위치로 옮길 수 있습니다.
3. **layout 규칙 표는 XXII.1.4 matrix의 Apple·Android 열(CSS layout longhand 부분)입니다.**
   - `apple::layout::SWIFTUI_RULES`·`APPKIT_RULES`, `android::layout::COMPOSE_RULES`는 layout longhand 40개
     (`stellar_layout::declared::LONGHANDS`)마다 규칙 하나를 둡니다. 완전성은 test가 검사합니다.
   - 생성기는 이 규칙대로만 낮추고, 1:1이 아닌 값은 `LayoutFinding { longhand, status, code, reason }`으로 냅니다. 상태
     어휘는 IX.10.5의 Native·Approximated·Unsupported입니다. Native는 진단하지 않습니다.
   - Apple·Android 상태는 CSS 매핑 상태에서 유도하지 않습니다(ADR-0025 결정 1.3). Web 열(IX.11.19)은 바꾸지 않습니다.
   - XXII.1.4 행은 이 표를 registry에 등록하고 Web 항목과 함께 검사합니다. 같은 규칙을 다시 정의하지 않습니다.
4. **layout 낮춤 원칙.**
   - 단위: CSS `px` 1 = Apple point 1 = `1.dp`.
   - 채움은 SwiftUI `.frame(max…: .infinity)`, Compose `fillMax*()`, AppKit 부모 크기 제약입니다.
   - 확정이 아닌 축에서 자식을 늘리는 container는 이상 크기를 씁니다(SwiftUI `.fixedSize`, Compose `IntrinsicSize.Max`).
     CSS line cross 크기·shrink-to-fit과 같은 결과입니다. AppKit은 Auto Layout이 같은 결과를 냅니다.
   - box-sizing: border-box는 padding 뒤 크기, content-box는 크기 뒤 padding입니다. min·max는 크기를 자르도록 놓습니다.
   - grow:
     - SwiftUI는 남은 공간을 콘텐츠와 무관하게 균등 분배합니다. 성장 형제가 하나이거나, 모두 `flex-basis: 0`·main 크기 auto·
       같은 grow일 때만 Native입니다.
     - Compose `weight`는 비율을 그대로 받습니다. 성장 형제가 둘 이상이면 basis 0일 때 Native입니다.
     - 부모 main 크기가 확정이 아니면 CSS에서도 효과가 없어 내지 않습니다.
   - absolute는 부모 padding box 위 층입니다(SwiftUI `.overlay(alignment:)` + offset, Compose 바깥 `Box` + `align` + offset,
     AppKit `addSubview` + inset 제약). relative inset은 SwiftUI·Compose offset이고, AppKit은 Unsupported입니다.
   - local transform(layout 뒤 적용, 원점 좌상단): SwiftUI `.offset`·`.transformEffect`(Native)입니다. Compose 이동은
     `offset`(Native), 선형 부분은 `drawWithContent` 그리기 변환입니다. hit test가 바뀌지 않아 Approximated입니다. AppKit은
     Unsupported입니다.
   - grid는 모든 item이 명시 line이거나 모두 자동 배치일 때만 칸을 정합니다. 섞이거나 음수 line·row span이면 배치 규칙을
     다시 구현하지 않고 Unsupported로 진단한 뒤 문서 순서 자동 배치로 냅니다. layout 엔진의 배치 함수는 crate 비공개입니다.
     - SwiftUI는 `Grid`(iOS·tvOS 16, macOS 13, watchOS 9 이상, 아니면 행·열 stack으로 Approximated)입니다.
     - Compose는 행 `Column` 안 칸 `Row`입니다.
     - AppKit은 `NSGridView`입니다.
   - `flex-shrink: 1`과 `min-*: auto`의 넘칠 때 압축 차이는 root에 한 번만 Approximated(`platform-compression`)로 냅니다.
     node마다 내면 모든 flex item에 같은 진단이 붙기 때문입니다.
5. **Control contract 대응.** 디자인한 모양(CSS layout)을 플랫폼 기본 모양이 덮지 않게 합니다.

   | contract | SwiftUI | AppKit | Compose |
   |---|---|---|---|
   | Button | `Button { … } label: { 자식 layout }` + `.buttonStyle(.plain)` | 자식 layout view + `NSClickGestureRecognizer`(비활성이면 동작 안 함) | `clickable(indication = null, enabled, role = Role.Button)`, radio는 `selectable` |
   | TextInput | `TextField(text: $상태, prompt:)` + `.textFieldStyle(.plain)` | `NSTextField(string:)` + `placeholderString` | `BasicTextField` + placeholder `decorationBox` |
   | List·SingleSelectionGroup | layout container(의미는 접근성) | layout container | layout container + `collectionInfo`·`selectableGroup` |
   | Modal | 그 자리에 내용(표시 상태가 IR에 없음, 노트) | 같음 | 같음 + `paneTitle` |

   - SwiftUI `List`·`Picker`와 Material 컴포넌트를 쓰지 않습니다. 플랫폼 행 모양·scroll·Material 모양이 디자인과 달라지기
     때문입니다.
   - IR에 없는 값은 만들지 않습니다. 이미지 원본(SF Symbol 속성만 읽음), 도형 기하, 펼치지 않은 instance는 빈 view와
     노트입니다. Typography record(IX.11.11)가 없어 Text 스타일을 내지 않습니다.
6. **Compose 접근성·검증.**
   - semantics: 숨김 subtree `clearAndSetSemantics {}`, button·radio·listItem `semantics(mergeDescendants = true)`, 이름
     `contentDescription`, 값 `stateDescription`, 상태 `disabled()`·`selected`·`toggleableState`·`focused`, dismiss는
     `dismiss {}`입니다. 읽기 순서는 `isTraversalGroup`·`traversalIndex`이며 Compose UI 1.6 미만이면 노트입니다.
   - wrap은 `FlowRow`·`FlowColumn`(Compose UI 1.4 이상)입니다.
   - Kotlin 컴파일러가 없어 compile fixture 대신 static fixture(golden + 괄호 짝·import 검사)를 씁니다. Android 빌드
     환경에서 compile fixture를 더해야 합니다.
7. **Context·`MotionSpec`(일부).** IR에 Context·`MotionSpec`이 생기기 전에는 출력 형태와 규칙만 둡니다. adapter가 timing
   function·Context 타입을 따로 만들면 XXII.1.5·XXII.1.6 IR과 두 벌이 되기 때문입니다.
   - SwiftUI: ContextKey는 `EnvironmentKey` + `EnvironmentValues` 속성(`<소문자 접두어><이름>`)입니다. provision과 slot 다시
     공급(ADR-0025 결정 3.11)은 `.environment(\.속성, 값)`, 소비는 `@Environment(\.속성)`입니다.
   - AppKit에는 렌더 트리 전달 값이 없습니다. IR이 정한 값을 생성자로 주입합니다(아직 코드 없음).
   - Compose:
     - ContextKey는 `Local<접두어><이름>`이고, provision과 slot 다시 공급은 `CompositionLocalProvider`입니다.
     - key의 provision 값이 모두 고정(literal, 해석한 token, key default)이면 `staticCompositionLocalOf`입니다.
     - 하나라도 상태·실행 중 환경 값을 읽으면 `compositionLocalOf`입니다.
     - 근거: 바뀌지 않는 값의 추적 비용과, 자주 바뀌는 값의 subtree 전체 재구성을 함께 피합니다.
   - `MotionSpec` 낮춤 규칙(XXII.1.7 Apple·Android 열의 입력). 코드는 IR 타입이 생긴 뒤 둡니다.

     | timing function | SwiftUI | Compose |
     |---|---|---|
     | linear | `.linear(duration:)` Native | `tween(d, delay, LinearEasing)` Native |
     | cubic-bezier | `.timingCurve(x1, y1, x2, y2, duration:)` Native | `tween(…, CubicBezierEasing(x1, y1, x2, y2))` Native |
     | steps | 계단 곡선이 없어 Approximated | 계단 `Easing` lambda로 Native |
     | spring | `.interpolatingSpring(mass:stiffness:damping:initialVelocity:)` Native | `spring(dampingRatio = c / (2·√(k·m)), stiffness = k / m)`, 초기 속도는 `Animatable.animateTo(initialVelocity =)`. `animate*AsState`면 Approximated |

     - 지연은 SwiftUI `.delay`, Compose `delayMillis`입니다.
     - reduced motion 대체는 SwiftUI `accessibilityReduceMotion`(Native)입니다. Compose는 시스템 animation scale을 읽어야 해
       Approximated입니다.
8. **CLI `stellar export swiftui|appkit|compose`.**
   - `ops::export_native`는 `export-web`과 같은 Web 투영(`web_frame::export_web_frame_node`)의 구조·배치(ADR-0089 결정 4)·
     풀린 40개 longhand(`ResolvedLayout::declarations`)를 그대로 씁니다. 그래서 받는 node 범위와 실패 진단이 같습니다.
     - 배치가 정한 값만 바꿉니다: free → `position: absolute` + `left`·`top`(`css::value::format_length`), inset →
       `position: absolute`, 숨김 → `display: none`.
   - `codegen::apple::native_input::build_native_ir`가 그 선언을 `stellar_layout::declared::style_from_declarations`로 엔진
     입력으로 바꾸고, 접근성을 `stellar_accessibility::generate`로 만들어 `build_ir`를 부릅니다. ops가 layout crate에 직접
     의존하지 않으므로 crate graph를 바꾸지 않습니다.
   - Target Profile은 대상마다 내장 기본 profile입니다(iOS·macOS 26.0, Android API 35·Compose UI 1.7.0). 저장 profile을 IR
     registry pin과 아직 잇지 못하기 때문입니다(ADR-0049).
   - 문서가 복원하지 못하는 control contract·상태·token·Variant·Context·Motion·paint는 넣지 않고 `native-scope` 정보 진단을
     냅니다. adapter 노트는 경고 `native-note`입니다. 파일을 쓰지 않습니다.
   - MCP 도구는 이 작업에서 더하지 않았습니다. 같은 ops 함수를 쓰면 됩니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| SwiftUI `List`·`Picker`, Material 컴포넌트로 contract 출력 | 채택하지 않음. 플랫폼 모양이 디자인한 CSS layout을 덮어 실제 구현과 멀어집니다 |
| Compose를 nested lambda 하나로 출력 | 채택하지 않음. 재귀 없이 쓰기 어렵고 scope receiver를 잃습니다 |
| grid 배치 규칙을 adapter에서 다시 구현 | 채택하지 않음. layout 엔진과 두 벌의 배치 판정이 됩니다 |
| 압축 차이를 node마다 진단 | 채택하지 않음. 모든 flex item에 같은 진단이 붙습니다 |
| adapter 안에 timing function·Context 타입 정의 | 채택하지 않음. XXII.1.5·XXII.1.6 IR 타입과 두 벌이 됩니다 |
| `ops → layout` crate edge 추가 | 채택하지 않음. crate graph 정책 변경이 필요하고, codegen 쪽 builder로 충분합니다 |
| CLI가 저장 문서를 따로 걸어 IR 생성 | 채택하지 않음. Web 투영과 배치·값 해석이 두 벌이 됩니다 |

## Consequences

- 같은 IR에서 SwiftUI·AppKit·Compose 소스가 나오고, CSS layout과 다른 부분은 규칙 표의 상태로 진단됩니다.
- Swift 출력은 iOS·macOS SDK typecheck로 검증됩니다. Kotlin은 static fixture까지입니다.
- 한계가 있습니다.
  - 실제 문서 경로는 Web 투영 범위(Frame·Fragment·Rectangle)만 받습니다. Text·Image·component instance는 실패합니다.
  - Context·`MotionSpec`·상태 전환 lowering은 IR 행(XXII.1.5·XXII.1.6·XXII.1.7)과 XXX.7.4 뒤에 연결해야 합니다.
  - 공용 분석이 `apple` module 아래에 있습니다.

## Compatibility

- 저장 형식·FFI 변경은 없습니다.
- `stellar-codegen`에 `apple::export_apple_ui`, `apple::native`·`layout`·`context`·`native_input`, `android` module이
  늘었습니다. `export_apple` 출력 bytes는 같습니다.
- `stellar-ops`에 `export_native`, CLI에 `export` 명령과 help 한 줄이 늘었습니다.

## Performance

- 분석·생성은 node 수에 선형입니다(형제·접근성 자식 목록 한 번). 재귀를 쓰지 않습니다.
- 핵심 입력·렌더 경로에 들어가지 않습니다.

## Security

- 생성 코드를 실행하지 않습니다. typecheck는 test에서만 합니다.
- 문자열은 대상 언어 규칙으로 escape합니다. Swift는 `\`·`"`·제어 문자, Kotlin은 그에 더해 `$`(문자열 template)입니다.
- 식별자는 ASCII로 정리하고 예약어·생성 코드 이름과 겹치면 접미사를 붙입니다.

## Migration

- 데이터 migration은 없습니다.
- XXII.1.4가 들어오면 결정 3의 표를 registry에 등록합니다.
- XXII.1.5·XXII.1.6·XXII.1.7과 XXX.7.4가 들어오면 결정 7의 출력 형태로 IR의 Context·`MotionSpec`을 낮춥니다. 그 뒤
  XXII.3.4·XXII.4.3의 Context shadowing·slot fixture를 XXX.7.4 결과와 맞춥니다.
- XXII.1.1의 문서 계층 연결이 생기면 결정 8의 CLI 입력을 그 연결로 바꿉니다.
