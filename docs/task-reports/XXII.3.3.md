# 작업 보고서: XXII.3.3 접근성 의미와 색공간을 Apple API 표현으로 내보내기

## WBS

- WBS: XXII.3.3
- 작업명: accessibility semantics와 color space를 Apple API 표현으로 내보냄
- 상태: 완료 (`[x]`)
- 선행: IX.8.6, III.4.2(둘 다 기준 브랜치 `[x]`)

## 변경 파일

- `crates/codegen/src/apple/mod.rs` (신규) — `export_apple`, `AppleDialect`(SwiftUI·AppKit), `AppleOptions`, `AppleExport`,
  `AppleNote`, `AppleError`.
- `crates/codegen/src/apple/color.rs` (신규) — 색공간 → SwiftUI `Color`·AppKit `NSColor`·UIKit `UIColor` 생성자, 확장 범위
  노트, `<prefix>Colors` 상수 enum.
- `crates/codegen/src/apple/accessibility.rs` (신규) — 접근성 의미 → SwiftUI `ViewModifier`·AppKit `NSView` 함수.
- `crates/codegen/src/apple/swift.rs` (신규) — Swift 문자열 literal escape, 예약어 회피 식별자.
- `crates/codegen/src/apple/tests.rs` (신규) — static·compile fixture 테스트.
- `crates/codegen/src/lib.rs` (수정) — `apple` module. `crates/codegen/src/fixture.rs` (수정) — macOS profile.
- `crates/codegen/tests/fixtures/apple_ios_swiftui.swift`, `apple_mac_swiftui.swift`, `apple_mac_appkit.swift` (신규) — golden.
- `docs/guides/crate-map.md` (수정) — codegen module 목록.
- `docs/task-reports/XXII.3.3.md`(신규), 계획서(XXII.3.3 `[ ]`→`[x]`)

## 구현 요약

- **색공간**: Stellar 색(색공간 + 비연관 알파, encoded 채널)을 같은 encoded 값으로 생성자에 넘깁니다. SwiftUI
  `Color(.sRGB|.displayP3, red:green:blue:opacity:)`, AppKit `NSColor(srgbRed:…)`/`NSColor(displayP3Red:…)`(확장 범위
  sRGB는 `NSColor(colorSpace: .extendedSRGB, components:count:)`), UIKit `UIColor(red:…)`/`UIColor(displayP3Red:…)`.
  채널이 `[0, 1]` 밖이면 값을 유지하고 `ExtendedRange` 노트, 확장 범위를 자를 수 있는 생성자는 `MayClamp` 노트를 남깁니다.
  Color token 전체와 Color 값 node 속성을 상수로 내보내고, token에 묶인 속성은 token 상수를 가리킵니다.
- **접근성(SwiftUI)**: node마다 `ViewModifier` — container(`.contain`)·control(`.combine`), trait(`.isButton`,
  `.isStaticText`, `.isImage`, `.isModal`, `.isSelected`, `.isToggle`), `.accessibilityLabel(Text(verbatim:))`,
  `.accessibilityValue`, `.disabled`, 숨김 subtree `.accessibilityHidden(true)`, 읽기 순서 `.accessibilitySortPriority`,
  activate+select를 합친 기본 `.accessibilityAction`, dismiss `.accessibilityAction(.escape)`. 상태는 `@Binding`, semantic
  action은 `onAction(<StableId>)`, 선택 일치는 `상태 == "<item StableId>"`. `.isToggle`은 profile 최소 OS가 지원할 때만
  씁니다(iOS·tvOS 17, macOS 14, watchOS 10, visionOS 1).
- **접근성(AppKit, macOS 전용)**: node마다 `apply<id>(to: NSView, …)` — `setAccessibilityElement`·`Role`·`Subrole`·`Label`·
  `Value`·`Enabled`·`Selected`·`Focused`·`Modal`·`CustomActions`, 읽기 순서는 `setAccessibilityChildren`(id → view 표).
- 표현할 수 없거나 근사한 의미는 노트(`NativeControlRequired`, `Approximated`, `NoEquivalent`, `UnresolvedBinding`,
  `ComponentProperty`, `Unavailable`)로 남깁니다. Apple이 아닌 IR, iOS 등에서 AppKit dialect, 식별자가 아닌 접두어는 오류입니다.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (30 passed) |
| `cargo fmt -p stellar-codegen -- --check` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `xcrun --sdk macosx swiftc -typecheck -target arm64-apple-macos26.0 crates/codegen/tests/fixtures/apple_mac_appkit.swift` | 0 |
| `xcrun --sdk macosx swiftc -typecheck …`(일부러 없는 modifier를 쓴 대조 파일) | 1 (`no member` 오류 확인) |
| `cargo xtask validate-task-report docs/task-reports/XXII.3.3.md` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음(Xcode Swift 6.4.2, macOS SDK 27.2, iPhoneSimulator SDK 27.2)

## 테스트

- `swift_sources_match_golden_static_fixtures`: iOS SwiftUI·macOS SwiftUI·macOS AppKit 소스가 golden 3개와 byte 단위로
  같고 두 번 생성해도 같음(static fixture).
- `generated_swift_typechecks_against_apple_sdks`: 같은 소스를 `xcrun swiftc -typecheck`로 iOS 시뮬레이터 SDK(arm64 iOS
  26.0)와 macOS SDK(arm64 macOS 26.0)에서 검사해 통과(compile fixture, macOS에서만 실행).
- `color_spaces_and_semantics_use_apple_api`: token 색공간(P3·sRGB), token 상수 참조, radio 선택 식, 비활성 binding, modal·
  escape, 읽기 순서 priority, 숨김, 노트(textField native, radio group 근사), macOS 확장 범위 sRGB 노트와 `extendedSRGB`.
- `pressed_toggle_respects_minimum_os`, `invalid_targets_and_options_are_rejected`, 색 생성자 2건, Swift literal 1건.
  남은 경고 0.

## 성능

- 해당 없음.

## 차이

- 컨트롤 자체(SwiftUI `Button`·`TextField` 등)와 layout 생성은 XXII.3.1·XXII.3.2 범위라 만들지 않았습니다. 이 행은 그
  view에 붙일 접근성 `ViewModifier`·AppKit 함수와 색 상수를 내보냅니다. 그래서 textField 역할과 focus·편집·값 쓰기는
  native control 몫으로 노트만 남깁니다.
- AppKit의 press 동작은 subclass override(`accessibilityPerformPress`)가 필요해 `NSAccessibilityCustomAction`으로 근사했고,
  dialog·listItem role도 `.group`으로 근사했습니다(노트).
- compile fixture는 Swift 소스 typecheck(`swiftc -typecheck`)이며 실행 파일 link·실행은 하지 않습니다.

## 남은 위험

- compile fixture 테스트는 macOS에서 Xcode(`xcrun swiftc`, iOS 시뮬레이터 SDK)가 있어야 통과합니다. 다른 환경에서는
  컴파일되지 않습니다(`cfg(target_os = "macos")`).
- 상태 변수 이름의 Swift 식별자 변환은 ASCII 정리·예약어 접미사 수준입니다(한글 이름은 `state`, `state_2` …). escape
  규칙 고정은 XXII.5.1 범위입니다.
