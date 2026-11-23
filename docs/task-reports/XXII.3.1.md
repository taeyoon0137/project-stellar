# 작업 보고서: XXII.3.1 Apple TargetProfile의 SwiftUI·AppKit dialect로 Control contract 매핑

## WBS

- WBS: XXII.3.1
- 작업명: Apple TargetProfile에 맞춰 SwiftUI/AppKit 중 지정된 output dialect로 Control contract를 매핑
- 상태: 부분 완료 (`[~]`) — adapter와 검증 기준(golden source)은 끝났습니다. 선행 XXII.1.1이 `[!]`이라 실제 프로젝트 View에서
  IR을 채우는 문서 계층 연결이 없고, adapter는 IR fixture로만 검증했습니다.
- 결정: ADR-0107 결정 1·2·5, ADR-0025 결정 4, ADR-0049
- 선행: XXII.1.1(`[!]`, IR 모델·`build_ir`는 있음)

## 변경 파일

- `crates/codegen/src/apple/mod.rs` — `export_apple_ui`(색·접근성 출력 뒤에 view tree), `AppleNoteKind::Control`·`Layout`,
  `ControlNote`. 기존 `export_apple`(XXII.3.3) 출력은 bytes가 같습니다.
- `crates/codegen/src/apple/accessibility.rs` — 상태 이름 표(`state_names`)와 node별 hook을 view 코드와 공유하도록 노출.
  생성 소스는 바뀌지 않습니다(기존 golden 그대로).
- `crates/codegen/src/apple/native.rs`(신규) — Apple·Android 공용 구조 분석: 출력 view 종류, contract 선택(우선순위·
  Unsupported contract 처리), Fragment 펼침, `display: none` 제외, absolute 분리, 채움·확정 크기, grid 칸 배치.
- `crates/codegen/src/apple/swiftui.rs`(신규) — `struct <prefix>View: View`.
- `crates/codegen/src/apple/appkit.rs`(신규) — `final class <prefix>View: NSView`.
- `crates/codegen/src/apple/layout.rs`(신규) — layout 규칙 표와 진단 타입(XXII.3.2에서 검증).
- `crates/codegen/src/apple/ui_tests.rs`(신규), `crates/codegen/src/apple/golden/*_view.swift`(신규 golden 3개).
- `docs/task-reports/XXII.3.1.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (65 통과) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-codegen -- --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC(Xcode Swift 6.4.2, macOS SDK 27.2, iPhoneSimulator SDK 27.2)

## 테스트

- `apple::ui_tests::view_sources_match_golden_static_fixtures`: iOS SwiftUI·macOS SwiftUI·macOS AppKit view tree가 golden과
  byte 단위로 같고 두 번 생성해도 같음(검증 기준 golden source).
- `apple::ui_tests::generated_views_typecheck_against_apple_sdks`: 같은 소스를 `xcrun swiftc -typecheck`로 iOS 26·16.4,
  macOS 26·13.0(AppKit) deployment target에서 검사.
- `apple::ui_tests::controls_map_to_swiftui_and_appkit`: Button(activate → `onAction`, radio 선택 → 선택 상태 쓰기,
  `.buttonStyle(.plain)`), TextInput(상태 binding·placeholder), SF Symbol Image, 접근성 modifier에 같은 상태 binding,
  AppKit 비활성 guard·입력 확정 상태 쓰기·`setState`, Shape·Modal 노트, profile이 제외한 node 없음.
- `apple::ui_tests::unsupported_contract_falls_back_to_container`: Unsupported contract는 container + `ContractUnsupported` 노트.
- `apple::ui_tests::ui_source_extends_accessibility_export`: view 출력이 XXII.3.3 출력을 그대로 앞에 둠.

## 성능

- 해당 없음. 분석은 node 수에 선형이고(형제 목록 한 번), 재귀를 쓰지 않습니다.

## 차이

- Control contract 대응(ADR-0107 결정 5):
  - Button은 SwiftUI `Button { … } label: { 자식 layout }` + `.buttonStyle(.plain)`, AppKit은 자식 layout view +
    `NSClickGestureRecognizer`입니다. 디자인한 모양(CSS layout)을 그대로 그리기 위해서입니다.
  - List·SingleSelectionGroup은 SwiftUI `List`·`Picker`가 아니라 layout container입니다. 플랫폼 행 모양과 scroll이 디자인과
    달라지기 때문입니다. 의미는 XXII.3.3 접근성 modifier가 냅니다.
  - Modal은 표시 상태 변수가 IR에 없어 그 자리에 내용 container로 내고 `ModalInPlace` 노트를 남깁니다.
- IR에 없는 값은 만들지 않습니다: 이미지 원본(SF Symbol 속성만 읽음), 도형 기하, 펼치지 않은 instance는 빈 view와 노트.
  Typography record(IX.11.11)가 IR에 없어 Text 스타일은 내지 않습니다.

## 남은 위험

- 실제 문서에서 IR을 만드는 연결이 없습니다(XXII.1.1 `[!]`). CLI 연결 범위는 ADR-0107 결정 8에 적었습니다.
- compile fixture는 typecheck이며 link·실행은 하지 않습니다. 화면 결과를 CSS와 픽셀 비교하지 않았습니다.
