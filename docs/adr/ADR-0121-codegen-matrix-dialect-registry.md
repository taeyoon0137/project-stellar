# ADR-0121: 코드 생성 matrix에 Apple·Android dialect 열 등록과 adapter 출력 정리

## Status

Accepted (2026-11-23).

- 계획 행 XXII.1.4, XXII.1.7, XXII.5.1의 남은 범위와 IX.11.18·IX.11.20의 남은 범위를 다룹니다.
- ADR-0025 결정 1.3·4.2, ADR-0074, ADR-0076, ADR-0082, ADR-0094, ADR-0106 결정 3·8, ADR-0107을 따릅니다.
- ADR-0107 결정 3(adapter 규칙 표의 위치)과 결정 7 `MotionSpec` 표의 steps·spring 상태를 이 ADR이 고칩니다.

## Context

- XXII.1.4 matrix(`crates/codegen/src/matrix/css.rs`)와 Apple·Android adapter 규칙 표(`apple::layout::SWIFTUI_RULES`·
  `APPKIT_RULES`, `android::layout::COMPOSE_RULES`)가 같은 layout longhand 규칙을 따로 정의했습니다.
  - 두 표의 상태가 서로 달랐습니다. 예: SwiftUI `flex-wrap`은 matrix가 Approximated, adapter가 Unsupported였습니다.
    Compose `grid-template-rows`는 matrix가 Unsupported, adapter는 실제로 출력했습니다.
  - adapter는 값별 진단의 상태(`approximated(…)`·`unsupported(…)`)를 호출 지점마다 직접 정했습니다.
  - matrix의 Apple 열은 SwiftUI 기준이고 AppKit 차이는 어디에도 등록되지 않았습니다.
- XXII.1.7 Context·`MotionSpec` matrix(`matrix/behavior.rs`)도 adapter와 어긋났습니다.
  - Android 추적형·비추적형 CompositionLocal 선택이 "스타일 값이냐"로 적혀 있었습니다. adapter 규칙
    (`android::context::local_kind`, ADR-0107 결정 7)은 값 변경 빈도로 고릅니다.
  - ADR-0107 결정 7 표와 steps·spring 상태가 달랐습니다.
  - AppKit에는 Environment가 없는데 Apple 열 하나만 있었습니다.
- 네이티브 출력 입력(`apple::native_input`)은 CSS 속성 사용·상태 머신·motion·Context를 빈 값으로 넘겼습니다. 그래서
  IR mapping에 matrix 판정이 없었습니다.
- XXII.5.1 생성 파일 규칙(`output::GeneratedFiles`)은 Web 출력만 썼습니다.
- DEV-MAC에는 Kotlin 컴파일러가 없습니다. 지금 Compose 검사는 괄호 짝과 import 사용 검사뿐입니다.
- IX.11.18 계획 함수(`modifier_plan`)를 읽는 출력기가 없었습니다. 그래서 overlay containing block을 실제 DOM에서 확인하지
  못했습니다.
- IX.11.20 SVG 매핑표는 CSS longhand 이름으로만 이어졌습니다. Stellar 전용 원천 행(Text 본문, image crop 등)은 registry
  연결이 없었습니다.
- crate graph 제약:
  - export는 document·codegen에 의존하지 않습니다(ADR-0019, ADR-0091 결정 6).
  - codegen은 document에 의존하지 않습니다.

## Decision

1. **matrix 행 하나가 dialect 열을 모두 가집니다.**
   - `CssLowering`에 `appkit: Option<PlatformLowering>` 열을 더했습니다.
     - SwiftUI는 Apple 열(Apple profile kind 기준 dialect)이고 Compose는 Android 열입니다.
     - AppKit 열은 AppKit adapter가 낮추는 항목에만 있습니다. layout longhand 40개와 local transform(`frame/transform`)
       입니다. 그 밖의 항목은 AppKit dialect에 등록되지 않습니다(`None`).
   - adapter 규칙 문장(옛 `SWIFTUI_RULES`·`APPKIT_RULES`·`COMPOSE_RULES`)은 `matrix::dialect::DIALECT_TEXTS`로 옮기고 adapter
     표는 지웠습니다. `matrix::dialect::lowering(scope, name, dialect)`가 세 dialect의 상태·규칙·template을 돌려줍니다.
   - layout 행의 상태·template을 adapter 실제 출력에 맞췄습니다. 상태가 바뀐 항목은 다음과 같습니다.
     - `node/grid-row-start`: Apple·Android Unsupported → Approximated(명시 행 배치를 출력함).
     - `frame/flex-wrap`: Apple Approximated → Unsupported(한 줄로 냄), Android Approximated → Native(`FlowRow`·`FlowColumn`).
     - `frame/align-content`: Apple Unsupported → Approximated(grid 정렬).
     - `frame/grid-template-rows`·`grid-auto-rows`: Android Unsupported → Approximated.
     - AppKit은 SwiftUI와 다르게 `aspect-ratio`가 Native(제약), local transform이 Unsupported입니다.
   - Web 열(IX.11.19)은 바꾸지 않았습니다.
2. **값별 진단의 상태는 matrix가 정합니다.**
   - `matrix::dialect::FINDINGS`가 dialect마다 (항목, 진단 code, 상태)를 등록합니다.
   - adapter는 `matrix::dialect::finding(dialect, longhand, code, reason)`으로 `LayoutFinding`을 만들고 상태를 이 표에서
     받습니다. `LayoutFinding` 타입도 matrix로 옮겼습니다(`apple::LayoutFinding`은 재수출).
   - 표에 없는 진단은 debug build에서 panic하고 release build에서는 Unsupported입니다. 표현 가능성을 과장하지 않습니다.
   - adapter 소스의 호출이 모두 등록되어 있는지, 상태를 직접 정하는 옛 helper가 남지 않았는지 test가 소스를 읽어 확인합니다.
3. **네이티브 출력 IR 입력을 채웁니다.**
   - `build_native_view_ir(nodes, behaviors, profile)`을 더했습니다. `build_native_ir`는 빈 `NativeBehaviors`로 이 함수를
     부릅니다.
   - CSS 속성 사용(`css_properties`)은 node마다 다음을 넣습니다. 매핑 상태는 `exact`입니다(layout longhand는 CSS
     longhand 그대로).
     - 엔진 입력 필드가 엔진 생략값(`LayoutStyle::default()`)과 다른 layout longhand. 비교는 필드끼리 합니다. 값 텍스트를
       다시 읽거나 초기값 표를 따로 두지 않습니다.
     - container longhand는 Frame·ComponentInstance에만 넣습니다.
     - 0이 아닌 시각 이동이 있는 Frame은 `frame/transform`을 넣습니다.
   - 상태·상태 머신·`MotionSpec`·Context는 호출자가 IR 타입으로 넘긴 값을 그대로 씁니다. 문서 값 변환은 문서 계층(ops)의
     기존 함수가 합니다(`ops::codegen::motion` 등). 이 함수는 다시 해석하지 않습니다.
4. **Context·`MotionSpec` matrix에 AppKit 열과 값 변경 빈도 항목을 더합니다.**
   - `BehaviorRow`에 `appkit` 열을 더했고, `dialect_entry(subject, dialect)`로 읽습니다.
   - Context 항목 `context-value-fixed`·`context-value-changing`을 더했습니다.
     - Compose는 각각 `staticCompositionLocalOf`·`compositionLocalOf`입니다. `android::context::local_kind`가 이 규칙을
       따르는지 test가 대조합니다.
     - 스타일 값·그 밖의 값 항목의 Compose 열은 `composition-local-by-change`로 바꿨습니다(값 변경 빈도가 고름).
   - AppKit Context 열은 `appkit-resolved-value`입니다. IR이 해석한 값(XXX.7.4 결과)을 생성자로 주입합니다.
     - 그래서 재읽기·깊이 차이·slot 재공급도 IR 값 그대로이고 Native입니다.
     - 값이 바뀌는 Context는 렌더 트리 전파가 없어 Approximated(`context-manual-propagation`)입니다.
   - timing function 상태:
     - steps: SwiftUI는 `CustomAnimation` 계단 함수이고 Approximated입니다(iOS 17·macOS 14 미만 profile은 linear로 냄).
       AppKit은 discrete keyframe으로 Native, Compose는 계단 `Easing`으로 Native입니다(ADR-0107 결정 7 Compose와 같음).
     - spring: 모든 dialect가 Approximated입니다. Stellar는 1ms 고정 step semi-implicit Euler로 적분하고 지속 시간 끝에서
       끝값으로 맞춥니다(ADR-0094). 플랫폼 spring은 닫힌 식이거나 다른 적분이라 곡선이 다를 수 있습니다. ADR-0107 결정 7
       표의 SwiftUI·Compose spring Native는 이 결정이 대체합니다.
5. **Apple·Android 출력도 XXII.5.1 생성 파일 집합으로 냅니다.**
   - `output::FileKind`에 Swift(`.swift`)·Kotlin(`.kt`)을 더했습니다. marker 주석은 `//`이고, LF·끝 줄바꿈 하나·경로 규칙은
     Web과 같습니다.
   - `apple::export_apple_files`는 `<타입 접두어>View.swift` 하나를 냅니다.
   - `android::export_compose_files`는 `<package 조각 디렉터리>/<타입 접두어>View.kt` 하나를 냅니다.
   - 파일 이름은 adapter가 이미 검사한 ASCII 식별자(타입 접두어, Kotlin package 조각)로만 만듭니다. 그래서 별도 escape가
     없고 상대 경로 규칙을 어길 수 없습니다. 본문 식별자·문자열 escape는 ADR-0107 Security 규칙 그대로입니다.
6. **Compose compile fixture는 문법 검사기와 import 표 검사로 대신합니다(Kotlin 컴파일러가 없을 때).**
   - `android::kotlin_check`(test 전용)는 생성기가 쓰는 Kotlin 부분집합의 lexer와 재귀 하강 parser입니다. 읽지 못하는
     토큰이 있으면 실패합니다.
   - import 표 검사:
     - import가 정렬·중복 없음이어야 합니다.
     - 모든 import가 생성기 import 표(`compose::IMPORTS`)의 정규 경로여야 합니다.
     - 표의 모든 경로는 Compose 1.7 공개 package 표(`PACKAGE_ARTIFACTS`)에 있어야 합니다.
   - 실제 출력에 문법 오류를 넣으면 잡는지 mutation test로 확인합니다.
   - Kotlin 컴파일러가 있는 환경에서는 compile fixture를 더해야 합니다(Migration).
7. **SVG 매핑표 행을 registry 항목에 잇습니다(IX.11.20).**
   - `SvgFeature::registry_items()`는 행마다 원천 registry 항목(범위, 이름)을 하나 이상 가리킵니다. Stellar 전용 원천도
     registry 항목이 있어 모든 행이 이어집니다.
   - export는 document에 의존하지 않습니다. 그래서 registry 전체 항목을 담은 codegen matrix golden(`matrix_css.txt`, ops
     test가 registry 전체와 대조)을 export test가 읽어 대조합니다.
   - SVG·CSS 수 표기 formatter를 한 함수로 모으는 일은 수 표기를 core-types로 옮기는 결정이 필요해 이 ADR 범위 밖입니다
     (ADR-0091 결정 6).
8. **modifier 계획 출력기와 DOM 배치 측정 fixture(IX.11.18).**
   - `modifier_plan::render_plan`이 계획을 `web::html::HtmlDocument` 요소로 냅니다. 순서는 wrapper(바깥부터) → node → overlay
     (node 첫 자식) → clip wrapper(내용 부모)입니다. 자리 표시 값이 없거나 래스터화가 필요하면 오류입니다.
   - 측정 fixture가 확인하는 것:
     - flex item이자 flex container이고 border·Fill·Stroke가 있는 Frame의 overlay containing block과 상자·radius.
     - absolute Frame과 뒤 형제의 stacking.
   - 측정 방법:
     - 로컬 Chrome 157 headless, DPR 1로 잽니다. 측정기는 `crates/codegen/tests/fixtures/modifier_plan_dom_measure.py`입니다.
     - 결과(`modifier_plan_dom.expected.txt`)와 페이지 hash를 커밋합니다.
     - test는 브라우저를 실행하지 않고, 페이지 bytes가 잰 페이지와 같은지 hash로 확인합니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| adapter 표를 그대로 두고 matrix와 대조 test만 추가 | 채택하지 않음. 같은 규칙이 두 곳에 남습니다 |
| AppKit을 별도 profile kind로 추가 | 채택하지 않음. `TargetProfileKind`는 저장·profile 계약이고 dialect는 adapter 옵션입니다 |
| AppKit 열이 없는 항목은 Apple 열로 대체 | 채택하지 않음. SwiftUI template을 AppKit 규칙처럼 보이게 합니다 |
| 값별 진단 상태를 호출 지점에 두고 표와 비교 | 채택하지 않음. 상태를 두 곳에서 정합니다 |
| native_input이 40개 longhand를 모두 사용으로 넣음 | 채택하지 않음. 생략값 항목(예: `grid-row-end: auto`)이 모든 node에 미지원 판정을 붙입니다 |
| native_input에 CSS 초기값 텍스트 표를 둠 | 채택하지 않음. 엔진 생략값과 두 벌이 됩니다 |
| Kotlin 컴파일러를 내려받아 compile fixture 실행 | 채택하지 않음. 저장소 밖 도구 설치가 필요하고 DEV-MAC 기준 환경이 아닙니다 |
| export에 document dev-dependency 추가 | 채택하지 않음. crate graph 정책(ADR-0019)이 export → document를 막습니다 |
| DOM 배치를 layout 엔진으로 확인 | 채택하지 않음. 엔진은 border를 모델링하지 않아 overlay 음수 inset을 확인할 수 없습니다 |

## Consequences

- adapter가 따로 정의하던 규칙이 matrix 한 곳에 있습니다. 상태 표기가 바뀌면 golden(`matrix_css.txt`·
  `matrix_dialect.txt`·`matrix_behavior.txt`)이 알립니다.
- IR mapping에 네이티브 출력 node의 CSS 속성 판정이 붙습니다. 생성 소스 bytes는 바뀌지 않습니다.
- Swift·Kotlin 출력이 Web과 같은 생성 파일 규칙(경로 순서, marker, formatting)을 따릅니다.
- 한계가 있습니다.
  - ops `export_native`가 `build_native_view_ir`에 문서의 상태·motion·Context를 넘기는 연결과, CLI·MCP가
    `export_apple_files`·`export_compose_files`를 쓰는 연결은 ops·CLI 소유 작업입니다. 지금 CLI는 소스 텍스트를 냅니다.
  - Kotlin은 compile fixture가 아니라 문법·import 검사입니다.
  - SVG 매핑 행의 registry 대조는 matrix golden을 거칩니다. 종단 대조는 `tests/integration/tests/svg_registry_mapping.rs`에
    `registry_items` 검사를 더해야 합니다.
  - DOM 측정은 기준 browser 한 버전입니다.

## Compatibility

- 저장 형식·FFI 변경은 없습니다.
- `stellar-codegen` 공개 API가 바뀝니다.
  - 늘어난 것: `matrix::dialect`, `CssLowering::appkit`, `BehaviorRow::appkit`, `behavior::dialect_entry`, `ContextFeature`
    두 항목, `apple::export_apple_files`·`apple_file_path`, `android::export_compose_files`·`compose_file_path`,
    `output::FileKind::{Swift, Kotlin}`, `native_input::{NativeBehaviors, build_native_view_ir}`, `modifier_plan::render_plan`.
  - 빠진 것: `apple::layout::{SWIFTUI_RULES, APPKIT_RULES}`, `android::layout::COMPOSE_RULES`.
- `stellar-export`에 `SvgFeature::registry_items`가 늘었습니다. SVG 출력 bytes는 같습니다.

## Performance

- 값별 진단 상태 조회는 작은 정적 표 선형 탐색입니다(진단이 날 때만).
- native_input의 사용 판정은 node마다 40개 필드 비교입니다.
- 핵심 입력·렌더 경로에 들어가지 않습니다.

## Security

- 생성 코드를 실행하지 않습니다. Swift typecheck와 Chrome 측정은 test·개발 도구에서만 씁니다.
- 생성 파일 경로는 검사한 ASCII 식별자로만 만들고 상대 경로 규칙(`..`·절대 경로 거부)을 지킵니다.

## Migration

- 데이터 migration은 없습니다.
- ops·CLI·MCP 연결은 다음 작업이 합니다.
  - ops `export_native`가 문서 상태·`MotionSpec`·Context를 IR 타입으로 바꿔 `build_native_view_ir`에 넘깁니다.
  - 출력 파일 쓰기(XXII.5.2)는 `export_*_files` 집합을 씁니다.
- Kotlin 컴파일러가 있는 환경을 쓸 수 있게 되면 `android_compose.kt`·`layout_compose.kt` golden을 Compose 1.7 classpath로
  compile하는 fixture를 더합니다.
- 계획 출력기 페이지가 바뀌면 측정기를 다시 실행해 `modifier_plan_dom.expected.txt`를 갱신합니다.
