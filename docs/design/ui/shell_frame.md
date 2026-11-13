# 셸 프레임(메인 창 구성) UI 명세

- 상태: 초안(2026-11-14), 코어 미연결(Stub 데이터)
- 영역: `shell_frame` — 코드 위치 `apps/macos/StellarMac/UI/Shell/`
- 기준 문서: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), Apple HIG(Windows, Toolbars, Sidebars, Split views, Keyboard, Accessibility, Materials, Writing)
- WBS 대응은 통합 단계에서 정합니다. 관련 행은 XII.9.1(toolbar·sidebar·inspector 골격)과 가이드 10.4의 "창 상태 저장·복원"(전용 WBS 없음)입니다.

## 1. 이 영역이 보여 주는 것

메인 프로젝트 창의 틀입니다. 내용(레이어 트리, 속성, Git, 캔버스 렌더링)은 다른 영역이 슬롯으로 끼웁니다.

```text
┌ NSToolbar(통합, 스타일 automatic) ─────────────────────────────────────────────┐
│ [사이드바 토글] │ [‹ ›] 제목/부제목   [도구 영역(가운데)]   [검색] [⋯] │ [인스펙터 토글] │
├──────────────┬──────────────────────────────────────────┬────────────────────┤
│ 사이드바 슬롯  │ 콘텐츠 슬롯(캔버스 호스트, safe area 안)  │ 인스펙터 슬롯        │
│ (sidebar     │  └ 상태 화면: 빈·로딩·오류               │ (inspector         │
│  item)       ├──────────────────────────────────────────┤  item)            │
│              │ 보조 패널 슬롯(선택, 처음엔 접힘)          │                    │
└──────────────┴──────────────────────────────────────────┴────────────────────┘
```

- 창: 표준 `NSWindow`(titled, closable, miniaturizable, resizable, full-size content). 창 크롬과 툴바 높이는 시스템이 정합니다. [가이드 2.1] [HIG Windows]
- 분할: `NSSplitViewController` 3열. 사이드바는 `NSSplitViewItem(sidebarWithViewController:)`, 인스펙터는 `NSSplitViewItem(inspectorWithViewController:)`, 가운데는 세로 `NSSplitViewController`(콘텐츠 / 보조 패널)입니다. 분할선은 thin 스타일입니다. [가이드 2.3] [HIG Split views]
- 캔버스: 기존 `CanvasView`(II.5.4)를 수정 없이 생성해 감쌉니다(`ShellCanvasHostViewController`). drawable 크기 전달은 `CanvasView`의 기존 경로가 맡습니다. [가이드 2.3, 9장]
- 캔버스는 safe area 안에만 둡니다. 툴바 glass와 사이드바·인스펙터 아래로 확장할지는 열린 결정 O2이므로, 결정 전에는 `automaticallyAdjustsSafeAreaInsets = false`로 두고 겹치지 않게 합니다. [가이드 2.3, 열린 결정 O2] [아키텍처 §6.2]
- 툴바 스타일은 `.automatic`에서 시작합니다(이 창 구성에서는 통합 툴바로 표시됨). unified와 unified compact 중 선택은 열린 결정 O1입니다. [가이드 2.1]

## 2. 파일

| 파일 | 내용 |
|---|---|
| `ShellFrameModel.swift` | 값 snapshot(`ShellFrameSnapshot`), 명령(`ShellFrameCommand`), 데이터 경계 protocol(`ShellFrameDataSource`) |
| `ShellFrameSlots.swift` | 슬롯 protocol, 슬롯 묶음, 창 구성 이름(`ShellWindowConfiguration`), 배치 수치(`ShellFrameMetrics`), SF Symbol 이름 |
| `ShellWindowController.swift` | 창 생성, snapshot 반영, 메뉴·툴바 공용 명령, 활성화 검사, 창 복원 진입점(`ShellWindowRestoration`) |
| `ShellSplitViewController.swift` | 3열 분할과 가운데 세로 분할, pane 전환, 동작 줄이기 대응 |
| `ShellToolbarController.swift` | 툴바 delegate, 항목 생성·갱신 |
| `ShellContentContainerViewController.swift` | 콘텐츠 슬롯 + 빈·로딩·오류 상태 화면 |
| `ShellCanvasHostViewController.swift` | 기존 `CanvasView`를 감싼 콘텐츠 슬롯 |
| `ShellFrameMenuItems.swift` | 메뉴 막대 항목(설치는 하지 않음) |
| `ShellFrameStubs.swift` | `ShellFrameDataSourceStub`, `ShellPanePlaceholderStub`, `ShellFrameSlots.stub(...)` |
| `ShellFrameUICheck.swift` | offscreen 검사 진입점 `ShellFrameUICheck.run() -> Bool` |

SwiftUI는 쓰지 않습니다. 모든 구성요소는 AppKit 표준 컨트롤입니다. [가이드 5.3] [아키텍처 §8.1]

## 3. 슬롯 protocol

다른 영역은 아래 protocol을 따르는 view controller를 넘겨 메인 창에 끼웁니다. 셸 프레임은 슬롯 내용을 해석하지 않습니다. [가이드 1.2 원칙 4]

```swift
@MainActor protocol ShellPaneSlot: AnyObject {
    var paneViewController: NSViewController { get }   // 창 생성 시 한 번 읽음
    var paneTitle: String { get }                       // VoiceOver 영역 이름, View 메뉴 제목
    var topAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { get }    // 기본 []
    var bottomAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { get } // 기본 []
}
@MainActor protocol ShellSidebarSlot: ShellPaneSlot {}
@MainActor protocol ShellContentSlot: ShellPaneSlot {}
@MainActor protocol ShellInspectorSlot: ShellPaneSlot {}
@MainActor protocol ShellAuxiliaryPanelSlot: ShellPaneSlot {}

struct ShellFrameSlots {
    var sidebar: any ShellSidebarSlot
    var content: any ShellContentSlot
    var inspector: any ShellInspectorSlot
    var auxiliary: (any ShellAuxiliaryPanelSlot)?   // nil이면 보조 패널 분할 항목 자체가 없음
}
```

- pane 하나에만 붙는 상단·하단 컨트롤(예: 레이어 필터 필드)은 accessory로 넘깁니다. safe area와 scroll edge effect는 시스템이 처리합니다. [가이드 2.3, 6.2]
- 슬롯이 루트 뷰에 접근성 레이블을 붙이지 않았으면 셸이 `paneTitle`을 붙입니다. 역할과 요소 여부는 슬롯이 정합니다. [가이드 8.3]
- 사이드바·인스펙터 슬롯은 배경을 직접 칠하지 않습니다. 시스템 재질을 가리기 때문입니다. [가이드 3.2]
- **보조 패널 슬롯과 가이드의 관계**: 가이드 2.4는 캔버스 아래 분할 영역을 로드맵 기능 자리로만 두고 초기 릴리스에 노출하지 않으며, Git 패널 위치(사이드바 구획 또는 인스펙터 탭)는 열린 결정 O4, 에이전트 패널은 인스펙터 탭 후보(6.5절)입니다. 그래서 이 슬롯은 구조상의 자리만 제공합니다. 선택 사항이고, 넘겨도 처음에는 접힌 상태이며 툴바 버튼을 두지 않고 View 메뉴 항목만 만들 수 있습니다. 최종 배치는 XIV.13.5와 ADR-0005 후속 WBS가 정합니다.

## 4. 데이터·명령 protocol

```swift
@MainActor protocol ShellFrameDataSource: AnyObject {
    var currentSnapshot: ShellFrameSnapshot { get }
    var snapshotHandler: ((ShellFrameSnapshot) -> Void)? { get set }
    func send(_ command: ShellFrameCommand)
}
```

- `ShellFrameSnapshot`(값 타입, `Equatable`·`Sendable`): `generation`, `title`, `subtitle`, `status`, `navigation(canGoBack, canGoForward)`, `tools[ShellToolDescriptor]`, `activeToolID`, `moreActions[ShellActionDescriptor]`, `searchPlaceholder`.
- `ShellFrameCommand`: `navigateBack`, `navigateForward`, `selectTool(id)`, `submitSearch(query:)`, `perform(id)`.
- 명령 식별자(`ShellCommandID`)는 코어와 공유하는 문자열이며 셸은 해석하지 않습니다. [가이드 6.1] [계획 XII.6.5]
- 사이드바·인스펙터·보조 패널 접기는 문서 의미가 아닌 창 상태이므로 명령으로 보내지 않습니다. [가이드 2.6] [아키텍처 §8.1]
- Rust 타입을 쓰지 않습니다. 코어 연결 시 FFI pull 이벤트(`stellar_runtime_poll_event`)로 받은 메시지를 snapshot으로 바꿔 메인 스레드에서 `snapshotHandler`로 넘기고, 명령은 command ID로 바꿔 보냅니다. [아키텍처 §8.2] [ADR-0009]

### 4.1 메인 스레드 규칙

- 창 controller는 받은 snapshot 중 이미 반영했거나 대기 중인 세대 이하를 버리고, 연속으로 받은 snapshot은 다음 런 루프에서 한 번만 반영합니다. [가이드 9장] [아키텍처 §6.3, §9.2]
- 반영은 바뀐 항목만 갱신합니다(제목, 도구 선택, 더 보기 메뉴, 활성 상태, 상태 화면).
- 검색은 Return으로 확정한 질의만 보냅니다(`sendsSearchStringImmediately = false`, `sendsWholeSearchString = true`). 질의 실행은 코어가 메인 스레드 밖에서 합니다. [가이드 9장]

## 5. 상태

| 상태 | 표시 | 툴바 |
|---|---|---|
| `ready` | 콘텐츠 슬롯(캔버스) | 도구·검색·내비게이션 활성(내비게이션은 snapshot 값) |
| `empty(title, message, actions)` | 아이콘(`square.dashed`), 제목, 안내문, 다음 할 일 버튼(예: New Project…, Open Project…, Clone Project…) | 도구·검색·내비게이션 비활성 |
| `loading(message)` | 회전 진행 표시기(접근성 레이블 = message), 제목 | 비활성 |
| `failed(title, message, recoveryActions)` | 아이콘(`exclamationmark.triangle`), 제목, 해결 방법, 복구 버튼 | 비활성 |

- 빈·오류 상태는 다음에 할 일과 그 동작 버튼을 함께 둡니다. [가이드 7.1] [HIG Writing]
- 대표 동작(`isPrimary`)은 한 상태 화면에 하나만 Return 키 등가로 지정해 시스템이 강조하게 합니다. 두 개 이상 표시되면 첫 번째만 적용합니다. [가이드 4.2, 5.1]
- 상태 화면은 배경을 칠하지 않고, 콘텐츠 계층이므로 glass를 쓰지 않습니다. 상태 화면이 보이는 동안 콘텐츠 슬롯 뷰는 숨겨 접근성 트리에서도 빠집니다. [가이드 3.1, 8.3]
- 메뉴·버튼 제목은 추가 입력이 필요하면 말줄임표로 끝냅니다. [가이드 8.1]

## 6. 상호작용

### 6.1 툴바

| 위치 | 항목 | 구현 |
|---|---|---|
| 앞쪽 끝 | 사이드바 토글 | 시스템 `.toggleSidebar`, 사용자 구성에서 옮기거나 뺄 수 없음 |
| 앞쪽 끝 | 사이드바 경계 추적 | 시스템 `.sidebarTrackingSeparator`, 고정 |
| 앞쪽(내용 영역 시작) | 뒤로·앞으로 | `NSToolbarItemGroup`(버튼 두 개), `isNavigational = true`라 창 제목 앞에 놓임 |
| 가운데 | 도구 영역 | `NSToolbarItemGroup`(selectOne), `centeredItemIdentifiers`로 가운데 고정, 좁아지면 시스템 overflow |
| 뒤쪽 | 프로젝트 검색 | `NSSearchToolbarItem`(AppKit 검색 필드) |
| 뒤쪽 | 더 보기 | `NSMenuToolbarItem`, 표시 화살표 없음 |
| 뒤쪽 끝 | 인스펙터 경계 추적, 인스펙터 토글 | 시스템 `.inspectorTrackingSeparator`, `.toggleInspector` |

- 항목 위치는 시스템 식별자와 `centeredItemIdentifiers`로만 정하고 직접 계산하지 않습니다. [가이드 6.1]
- 아이콘만 있는 항목에는 레이블, 도움말 태그, 이미지 접근성 설명을 모두 둡니다. 같은 묶음에서 텍스트 항목과 아이콘 항목을 섞지 않습니다. [가이드 5.4, 8.1]
- 사용자 구성은 `allowsUserCustomization`과 `autosavesConfiguration`으로 AppKit이 저장합니다.
- 도구 영역은 placeholder입니다. 받은 도구 목록을 그리고 선택을 `selectTool`로 보낼 뿐이며, 도구 상태 기계는 XII.3.6, 생성 도구는 WBS 없음(가이드 10.4)입니다.
- Branch·Pull·Fetch 같은 Git 동작은 툴바에 두지 않습니다. [가이드 6.1]

### 6.2 pane 전환

- 사이드바·인스펙터 전환은 툴바 버튼, View 메뉴 항목, 단축키가 모두 같은 action(`toggleSidebar:`, `toggleInspector:`)으로 연결됩니다. [가이드 2.3]
- first responder가 pane 안이면 split controller가 먼저 받고, 창 자체가 first responder이면 창 delegate(창 controller)가 split controller로 전달합니다.
- 숨긴 pane은 캔버스에 공간을 돌려줍니다. 창 폭이 바뀌면 캔버스만 늘고 줄어듭니다(가운데 항목 holding priority가 가장 낮음).
- 보조 패널은 `toggleAuxiliaryPanel:`로 전환합니다(슬롯이 있을 때만 활성).
- 동작 줄이기가 켜져 있으면 접기·펼치기를 애니메이션 없이 즉시 합니다. 꺼져 있으면 시스템 애니메이션을 씁니다. [가이드 8.4, 8.5]

### 6.3 메뉴 막대 항목(`ShellFrameMenuItems`)

메인 메뉴 구성은 XII.6.5 범위이므로 항목만 만들고 설치하지 않습니다. 모두 target 없는 action이고 툴바와 같은 메서드로 연결됩니다. 활성화 검사(`NSMenuItemValidation`)에서 숨기지 않고 비활성화하며, 표시 전환 항목 제목은 "Show …"/"Hide …"로 바뀝니다. [가이드 8.1]

| 항목 | action | 단축키 |
|---|---|---|
| Show/Hide Sidebar | `toggleSidebar:` | Control-Command-S |
| Show/Hide Inspector | `toggleInspector:` | Option-Command-I |
| Show/Hide `<보조 패널 이름>` | `toggleAuxiliaryPanel:` | 없음(O8, XII.6.5에서 정함) |
| Back | `navigateBack:` | Command-[ |
| Forward | `navigateForward:` | Command-] |
| Search Project… | `focusProjectSearch:` | 없음(XII.6.5에서 정함) |

- 더 보기 메뉴와 도구 영역의 동작도 메뉴 막대에 있어야 합니다. command ID가 코어에서 오므로 XII.6.5에서 같은 ID로 메뉴를 만듭니다. [가이드 6.1]
- 시스템 키보드 탐색 단축키를 가로채지 않습니다. [가이드 8.1]

## 7. 접근성과 외형

- VoiceOver: pane 영역 이름(Sidebar, Canvas, Inspector, 보조 패널 이름), 캔버스 뷰(역할 layout area, 레이블 Canvas), 툴바 항목 레이블, 도구별 레이블, 검색 필드 레이블(Search Project), 상태 화면(그룹 레이블 = 제목, 진행 표시기 레이블 = 메시지). 장식 아이콘은 접근성 트리에서 숨깁니다. [가이드 8.3] [HIG Accessibility]
- 캔버스의 선택·도구 상태 노출은 XII.8.1 범위입니다.
- 키보드: `autorecalculatesKeyViewLoop`로 Tab·Shift-Tab이 사이드바 → 캔버스 → 인스펙터 순서로 이동합니다. 사용자 조작 없이 포커스를 옮기지 않습니다(검색 포커스는 명령으로만). [가이드 8.2]
- 색: 시스템 의미 색(label 계층)만 씁니다. 하드코딩 색과 이미지 asset이 없습니다. [가이드 4.2, 5.4]
- Light·Dark·대비 높이기·투명도 줄이기: 셸 프레임은 직접 그리는 반투명 표면, 직접 쓰는 glass, 커스텀 배경이 없으므로 표준 구성요소가 시스템 설정을 그대로 따릅니다. 직접 대응하는 것은 동작 줄이기(pane 전환 애니메이션)뿐입니다. [가이드 8.4]
- 글꼴: 텍스트 스타일(title3, body, headline, callout)만 씁니다. [가이드 4.1]

## 8. SF Symbol

| 이름 | 쓰임 |
|---|---|
| `chevron.backward`, `chevron.forward` | 뒤로·앞으로 |
| `ellipsis` | 더 보기 메뉴 |
| `square.dashed` | 빈 상태 |
| `exclamationmark.triangle` | 오류 상태 |
| `cursorarrow`, `hand.raised`, `plus.magnifyingglass` | Stub 도구(Select, Pan, Zoom). 실제 이름은 코어 snapshot이 줌 |
| `sidebar.left`, `sidebar.right`, `rectangle.bottomthird.inset.filled` | Stub 사이드바·인스펙터·보조 패널 placeholder |

사이드바·인스펙터 토글과 검색 아이콘은 시스템 항목이 제공합니다.

## 9. 창 상태 저장과 복원

셸은 값을 직접 저장하지 않고 AppKit 기능만 씁니다. 창 상태는 프로젝트 파일과 Git에 넣지 않습니다. [가이드 2.6]

| 대상 | AppKit 기능 | 이름(`ShellWindowConfiguration.standard`) |
|---|---|---|
| 창 프레임 | `setFrameUsingName(_:)` + `setFrameAutosaveName(_:)`, 없으면 가운데, 복원 시 계단식 배치 끔 | `StellarMainWindow` |
| 3열 pane 폭·접힘 | `NSSplitView.autosaveName`(view 로드 전 지정) | `StellarMainWindow.MainSplit` |
| 보조 패널 높이·접힘 | `NSSplitView.autosaveName` | `StellarMainWindow.CenterSplit` |
| 툴바 사용자 구성 | `NSToolbar.autosavesConfiguration` | `StellarMainToolbar` |
| 창 복원 | `isRestorable`, `identifier`, `restorationClass = ShellWindowRestoration` | `StellarMainWindow` |

- `ShellWindowRestoration`은 앱이 등록하는 `windowProvider`로 창을 만듭니다. 등록 전에는 복원을 건너뛰고 AppKit에 실패를 알립니다.
- split view는 상태를 다음 런 루프에서 기록합니다.
- 같은 split autosave 이름을 쓰는 창이 동시에 둘 이상 살아 있으면 서로의 기록을 덮어씁니다(검사 중 확인). 같은 프로젝트 여러 창(XII.2.4)에서는 창마다 다른 `ShellWindowConfiguration`을 넘겨야 합니다.

## 10. 배치 수치(시안)

가이드 0.2에 따라 아래 값은 셸 프레임 시안 값이고 `ShellFrameMetrics` 한곳에 있습니다. XII.9.1에서 측정해 확정하고 가이드 12장에 기록합니다.

| 항목 | 값(pt) |
|---|---|
| 첫 창 콘텐츠 크기 | 1200 × 760 |
| 창 콘텐츠 최소 크기 | 640 × 400(더 좁으면 사이드바·인스펙터가 시스템 규칙으로 접힘) |
| 사이드바 폭 | 200–360 |
| 인스펙터 폭 | 240–400 |
| 캔버스 최소 | 320 × 200 |
| 보조 패널 높이 | 140–480 |
| 상태 안내문 최대 줄 폭 | 320 |
| 상태 화면 요소 간격 | 8 |

## 11. 구현 중 확인한 사항

- **모서리 적응 layout 영역과 pane 애니메이션**: 사이드바 placeholder를 `layoutGuide(for: .margins(cornerAdaptation: .horizontal))` 안에 배치했을 때, 애니메이션으로 사이드바를 접으면 레이아웃 중 제약 갱신이 반복돼 AppKit 예외(`_postWindowNeedsUpdateConstraints`)로 앱이 종료됐습니다(macOS 27.2, Xcode 27.2 SDK). 동작 줄이기 경로(애니메이션 없음)에서는 재현되지 않았습니다. 가운데 정렬 내용은 창 모서리에 닿지 않으므로 셸 프레임은 모서리 적응 없이 `.margins()` 영역을 씁니다. 가이드 2.1의 모서리 적응 규칙을 사이드바·인스펙터 안 모서리 근처 내용에 적용할 때는 애니메이션 접기를 함께 검사해야 합니다.
- **Auto Layout 검사 범위**: 접힌 pane의 분할선(`NSSplitDividerView`)과 stack view가 떼어 낸 숨긴 뷰는 배치되지 않아 `hasAmbiguousLayout`이 참으로 나옵니다. 검사는 숨긴 뷰(`isHiddenOrHasHiddenAncestor`)를 제외합니다.
- **화면 확인 범위**: offscreen 렌더링(`cacheDisplay`)으로 3열 배치, 툴바 항목 순서, 빈 상태 화면을 눈으로 확인했습니다. offscreen 렌더링에는 Liquid Glass 재질과 키 창 강조가 나타나지 않으므로, 실제 창에서의 재질·선택 세그먼트 표현은 확인하지 않았습니다.

## 12. 검사(`ShellFrameUICheck`)

`ShellFrameUICheck.run() -> Bool`은 Stub data source와 Stub 슬롯으로 창을 화면에 띄우지 않고 만들어 일반 크기(1200 × 760)로 layout한 뒤 다음을 확인합니다. 통합 시 `scripts/macos/smoke.sh`에 연결합니다.

- 창 크롬: full-size content, 표준 스타일 마스크, 툴바 스타일 automatic, 기본 툴바 항목 전체, 가운데 도구 영역, 툴바 구성 autosave, 내비게이션 그룹 navigational
- 분할: 항목 3개, sidebar·inspector behavior, autosave 이름, 보조 패널 처음 접힘, pane 폭이 허용 범위 안, 창 폭 변경 시 캔버스만 변함, 캔버스가 툴바·사이드바·인스펙터와 겹치지 않음, drawable 크기 > 0
- Auto Layout: ready, 각 pane 전환 후, 로딩·오류·빈 상태에서 모호한 레이아웃 없음(숨긴 뷰 제외)
- 접근성: pane·캔버스 레이블, 툴바·도구·검색·더 보기 레이블과 도움말, 상태 화면 레이블, SF Symbol 이름 유효
- 명령: 창이 first responder일 때 응답자 체인으로 사이드바·인스펙터·보조 패널 전환과 되돌리기, 메뉴 제목 반영, 애니메이션 경로 전환
- snapshot: 다음 런 루프 병합, 연속 snapshot 1회 반영, 오래된 세대 폐기, 내비게이션 활성 상태, 도구·검색·더 보기·상태 화면 버튼 명령 전달, 대표 버튼 1개
- 상태 저장: frame·split autosave 기록과 새 창에서의 복원(크기, 인스펙터 접힘), 창 복원 설정, provider 미등록 시 복원 실패 보고

검사용 autosave 이름은 `StellarShellFrameUICheck` 접두어를 쓰고, 시작과 끝에 이 접두어의 user defaults 기록을 지웁니다(중간에 비정상 종료한 이전 실행 기록 포함).

## 13. 코어 연결과 통합에 남은 일

- `ShellFrameDataSource`의 FFI 구현: `stellar_runtime_poll_event` 메시지 → `ShellFrameSnapshot`(세대 번호 포함), `ShellFrameCommand` → command ID. [아키텍처 §8.2] [ADR-0009]
- 창 controller가 `ProjectWindowSession` handle을 소유하도록 연결합니다. [가이드 2.5] [계획 XII.2.2]
- `AppDelegate`가 `CanvasWindowController` 대신 `ShellWindowController`를 쓰고, `ShellWindowRestoration.windowProvider` 등록과 secure restorable state 지원을 추가합니다.
- 메인 메뉴에 `ShellFrameMenuItems` 항목을 설치하고, 더 보기·도구 영역 command ID를 메뉴 막대에도 둡니다. [계획 XII.6.5]
- 현재 `NativeFieldsPanel`(텍스트 입력 검증 fixture)을 사이드바로 옮길 때 `--text-input-check` 경로를 함께 옮깁니다. [가이드 10.1]
- 사이드바·인스펙터·보조 패널의 실제 슬롯으로 Stub을 바꿉니다. 보조 패널 노출 여부는 가이드 2.4·O4·ADR-0005에 따릅니다.
- 다중 창(XII.2.4)에서 창별 `ShellWindowConfiguration` 이름 규칙을 정합니다.
- 툴바 스타일(O1), 캔버스 확장(O2), 배치 수치 확정(가이드 12장)을 XII.9.1에서 정합니다.
- UI 문자열 현지화 체계는 아직 없습니다(현재 영문 문자열).
- 실제 창에서 Liquid Glass 재질, 키 창 강조, 대비 높이기·투명도 줄이기 조합을 눈으로 확인하는 작업은 XII.9.4 적합성 보고서에서 합니다.
