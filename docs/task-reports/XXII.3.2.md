# 작업 보고서: XXII.3.2 common layout을 Apple layout 코드로 변환하고 unsupported constraint 진단

## WBS

- WBS: XXII.3.2
- 작업명: common layout을 platform layout code로 변환하고 unsupported constraint를 diagnostic
- 상태: 부분 완료 (`[~]`) — 변환·진단과 compile fixture는 끝났습니다. 규칙 표는 XXII.1.4 matrix의 Apple 열(CSS layout
  longhand 부분)로 이 module에 두었습니다. XXII.1.4 행 자체(registry 등록·Web 항목 대조)는 아직 `[ ]`이라 통합 때 그 행이
  이 표를 등록해야 합니다(ADR-0107 결정 3).
- 결정: ADR-0107 결정 2·3·4, ADR-0025 결정 1.3·4.2, ADR-0074, ADR-0076
- 선행: Phase X, XXII.3.1(이 브랜치), XXII.1.4(`[ ]`)

## 변경 파일

- 변환 코드는 XXII.3.1 커밋에 있습니다: `crates/codegen/src/apple/native.rs`(채움·확정 크기·grid 칸),
  `crates/codegen/src/apple/swiftui.rs`, `crates/codegen/src/apple/appkit.rs`, `crates/codegen/src/apple/layout.rs`
  (`SWIFTUI_RULES`·`APPKIT_RULES`, `LayoutFinding`).
- `crates/codegen/src/apple/layout_tests.rs`(신규) — layout 진단 fixture와 검사.
- `crates/codegen/src/apple/golden/layout_ios_swiftui.swift`, `layout_mac_appkit.swift`(신규 golden).
- `crates/codegen/src/apple/mod.rs` — test module 등록.
- `docs/task-reports/XXII.3.2.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (65 통과) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo fmt -p stellar-codegen -- --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC(Xcode Swift 6.4.2, macOS SDK 27.2, iPhoneSimulator SDK 27.2)

## 테스트

- `apple::layout_tests::layout_sources_typecheck_against_apple_sdks`(검증 기준 compile fixture): layout fixture 출력을 iOS 26
  SwiftUI, Grid가 없는 iOS 15 fallback, macOS 26 SwiftUI, macOS 26 AppKit으로 `xcrun swiftc -typecheck`.
- `apple::layout_tests::layout_rules_cover_every_longhand`: `stellar_layout::declared::LONGHANDS` 40개 모두 SwiftUI·AppKit
  규칙이 하나씩 있음.
- `apple::layout_tests::layout_findings_follow_swiftui_rules`·`layout_findings_follow_appkit_rules`: 규칙별 입력·출력·진단.
  wrap(Unsupported), `%` 크기(Unsupported), min-content 최소 크기(Unsupported), reverse 읽기 순서(Approximated), 비율이 다른
  grow(Approximated), 섞인 grid 배치(Unsupported → 문서 순서 자동 배치), 비율이 다른 fr·최소 auto fr(Approximated),
  aspect-ratio(SwiftUI Approximated, AppKit 제약 Native), static position(Approximated), AppKit relative inset·transform
  (Unsupported). content-box 크기 순서, space-between Spacer, 회전 `transformEffect`, absolute trailing overlay, display none
  제외, grid 고정 열·행, baseline row, shrink 0 `fixedSize` 출력도 확인합니다.
- `apple::layout_tests::grid_falls_back_to_stacks_below_minimum_os`: Grid가 없는 최소 OS는 stack fallback과 진단.
- `apple::layout_tests::layout_sources_match_golden_static_fixtures`: 두 golden과 결정론.

## 성능

- 해당 없음.

## 차이

- 변환 원칙(ADR-0107 결정 4): CSS가 box를 부모 공간까지 늘리는 경우(block 폭, stretch, 확정 main 축의 grow, root 폭)를
  SwiftUI `.frame(max…: .infinity)`로, 확정이 아닌 축에서 자식을 늘리는 container는 `.fixedSize`로 이상 크기를 씁니다(CSS line
  cross 크기·shrink-to-fit과 같은 결과). border-box는 `.padding` 뒤 `.frame`, content-box는 그 반대입니다.
- `flex-shrink: 1`·`min-*: auto`의 넘칠 때 압축 순서는 플랫폼과 다를 수 있어 root에 한 번 Approximated(`platform-compression`)로
  냅니다. node마다 내면 모든 flex item에 같은 진단이 붙기 때문입니다.
- grid 배치는 모든 item이 명시 line이거나 모두 자동 배치일 때만 칸을 정합니다. 섞이면 배치 규칙을 다시 구현하지 않고
  Unsupported로 진단합니다(layout 엔진의 배치 함수는 crate 비공개).

## 남은 위험

- 결과 화면을 CSS 계산 결과와 픽셀·좌표로 비교하지 않았습니다. 정확도는 규칙 단위 출력 검사와 typecheck까지입니다.
- XXII.1.4 행이 Apple·Android 열을 registry에 등록할 때 이 표를 옮기고 Web 항목과 함께 완전성 검사를 해야 합니다.
