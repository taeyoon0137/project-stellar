# 셸 프레임(메인 창 구성) UI 명세

- 상태: 초안(2026-11-14). 앱 창으로 통합됨(14장). 프로젝트 열기·닫기는 코어 FFI, 영역 내용은 Stub data source
- 영역: `shell_frame` — 코드 위치 `apps/macos/Stellar/UI/Shell/`
- 기준 문서: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), Apple HIG(Windows, Toolbars, Sidebars, Split views, Keyboard, Accessibility, Materials, Writing)
- WBS 대응은 통합 단계에서 정합니다. 관련 행은 XII.9.1(toolbar·sidebar·inspector 골격)과 가이드 10.4의 "창 상태 저장·복원"(전용 WBS 없음)입니다.

## 1. 이 영역이 보여 주는 것

메인 프로젝트 창의 틀입니다. 내용(레이어 트리, 속성, Git, 캔버스 렌더링)은 다른 영역이 슬롯으로 끼웁니다.

```text
┌ NSToolbar(통합, 스타일 automatic) ─────────────────────────────────────────────┐
│ [사이드바 토글] │ [‹ ›] 제목/부제목                     [검색] [⋯] │ [인스펙터 토글] │
├──────────────┬──────────────────────────────────────────┬────────────────────┤
│ 사이드바 슬롯  │ 콘텐츠 슬롯(캔버스 호스트, safe area 안)  │ 인스펙터 슬롯        │
│ (sidebar     │  └ 상태 화면: 빈·로딩·오류               │ (inspector         │
│  item)       │        [도구 막대(하단 가운데)]           │  item)            │
│              ├──────────────────────────────────────────┤                    │
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
| `ShellFrameStrings.swift` | UI 문자열(`String(localized:)`, 상태 제목은 형식 문자열 하나) |
| `ShellFrameStubs.swift` | `ShellFrameDataSourceStub`, `ShellPanePlaceholderStub`, `ShellFrameSlots.stub(...)` |
| `ShellFrameUICheck.swift` | offscreen 검사 진입점 `ShellFrameUICheck.run() -> Bool` |
| `ShellAppCoordinator.swift` | 앱 수준 구성(14장): 메인 메뉴 설치, 시작 창, 프로젝트 열기·복원·닫기, 설정 창, 앱 수준 메뉴 모델, 시작 창 data source |
| `ShellProjectWindow.swift` | 프로젝트 창 하나: 코어 프로젝트 session handle, 프로젝트 frame data source, 창별 도구 모델 Stub, 구성 묶음 |
| `ShellAreaSlots.swift` | 사이드바·인스펙터·Git·에이전트 영역을 슬롯에 끼우는 어댑터와 영역 data source Stub |
| `ShellRuntimeEventPump.swift` | 코어 완료 event pull 펌프(요청별 완료 handler) |
| `ShellIntegrationCheck.swift` | 통합 검사 진입점 `ShellIntegrationCheck.run() -> Bool`(`--shell-integration-check`) |
| `ShellProjectOpenFailure.swift` | 프로젝트 열기 실패 원인과 안내 문구(14.8, XII.6.2) |
| `ShellProjectOpenCheck.swift` | Open Project 검사 진입점 `ShellProjectOpenCheck.run() -> Bool`(`--project-open-check`) |

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

- `ShellFrameSnapshot`(값 타입, `Equatable`·`Sendable`): `generation`, `title`, `subtitle`, `status`, `navigation(canGoBack, canGoForward)`, `tools[ShellToolDescriptor]`, `activeToolID`, `moreActions[ShellActionDescriptor]`, `searchPlaceholder`, `isSearchAvailable`(기본 true. 코어 검색 경로가 없으면 false로 보내 ready 상태에서도 검색 필드와 "Search Project…"를 비활성화합니다).
- data source 하나는 창 하나에 대응합니다. `snapshotHandler`는 구독자 하나만 받고 창 controller가 만들어질 때 덮어쓰므로, 창마다 별도 data source를 만듭니다.
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
| 뒤쪽 | 프로젝트 검색 | `NSSearchToolbarItem`(AppKit 검색 필드) |
| 뒤쪽 | 더 보기 | `NSMenuToolbarItem`, 표시 화살표 없음 |
| 뒤쪽 끝 | 인스펙터 경계 추적, 인스펙터 토글 | 시스템 `.inspectorTrackingSeparator`, `.toggleInspector` |

- 항목 위치는 시스템 식별자로만 정하고 직접 계산하지 않습니다. [가이드 6.1]
- 상단 바에는 구분선을 두지 않고(`titlebarSeparatorStyle = .none`, 창과 모든 분할 항목) 블러 배경만 둡니다. 콘텐츠 영역의 상단 바 뒤에는 `NSVisualEffectView`(`.titlebar`, within-window)를 깝니다(2026-11-14 사용자 결정).
- 도구 영역은 툴바가 아니라 콘텐츠 슬롯 하단 가운데에 떠 있는 도구 막대(`ShellToolBarView`)에 둡니다(2026-11-14 사용자 결정). 모양은 상단 툴바에 있던 도구 그룹과 같게 유지합니다: Liquid Glass 캡슐(`NSGlassEffectView`, 높이 36pt, 반경 18pt), 도구 칸 38.5pt, 선택한 도구 뒤 32.5 × 28pt 둥근 회색 선택 표시, 구분선 없음. 캔버스 아래 가장자리와 16pt 떨어지고, 사이드바·인스펙터가 열리고 닫혀도 콘텐츠 영역 기준 가운데에 있습니다.
- 아이콘만 있는 항목에는 레이블, 도움말 태그, 이미지 접근성 설명을 모두 둡니다. 같은 묶음에서 텍스트 항목과 아이콘 항목을 섞지 않습니다. [가이드 5.4, 8.1]
- 사용자 구성은 `allowsUserCustomization`과 `autosavesConfiguration`으로 AppKit이 저장합니다.
- **사용자 구성 palette 사본**: AppKit은 Customize Toolbar palette에 보일 항목을 `toolbar(_:itemForItemIdentifier:willBeInsertedIntoToolbar:)`에 `false`를 넘겨 따로 만들고 palette를 닫으면 해제합니다. controller의 항목 참조(내비게이션·검색·더 보기)는 `true`로 만든 실제 항목에만 묶고, palette 사본에는 target·action을 달지 않습니다. 사본에 묶으면 palette를 닫은 뒤 weak 참조가 nil이 되어 snapshot 반영·활성 상태·"Search Project…" 검증이 실제 항목에 닿지 않습니다(리뷰에서 재현). 도구 막대는 툴바 항목이 아니라 controller가 직접 가지므로 palette 사본과 관계없습니다.
- 툴바 아이콘(뒤로·앞으로)은 테두리 없이 둡니다(`isBordered = false`). 눌림·강조 표현은 시스템이 그립니다. [가이드 5.4]
- 도구 막대는 placeholder입니다. 받은 도구 목록을 그리고 선택을 `selectTool`로 보낼 뿐이며, 도구 상태 기계는 XII.3.6, 생성 도구는 WBS 없음(가이드 10.4)입니다.
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
| Back | `navigateBack:` | 없음(⌘[는 Object ▸ Send Backward, tools_menus 6.1) |
| Forward | `navigateForward:` | 없음(⌘]는 Object ▸ Bring Forward) |
| Search Project… | `focusProjectSearch:` | 없음(XII.6.5에서 정함) |
| 더 보기 동작(예: Show in Finder) | `performFrameAction:`(`representedObject` = 명령 ID) | 없음. 활성 창 snapshot의 더 보기 동작에 그 ID가 없으면 비활성 |

- 더 보기 메뉴와 도구 영역의 동작도 메뉴 막대에 있어야 합니다. command ID가 코어에서 오므로 XII.6.5에서 같은 ID로 메뉴를 만듭니다. [가이드 6.1]
- 시스템 키보드 탐색 단축키를 가로채지 않습니다. [가이드 8.1]

## 7. 접근성과 외형

- VoiceOver: pane 영역 이름(Sidebar, Inspector, 보조 패널 이름), 캔버스 뷰(역할 layout area, 레이블 Canvas), 툴바 항목 레이블, 도구별 레이블, 검색 필드 레이블(Search Project), 상태 화면(그룹 레이블 = 제목, 진행 표시기 레이블 = 메시지). 장식 아이콘은 접근성 트리에서 숨깁니다. [가이드 8.3] [HIG Accessibility]
- 같은 이름을 두 번 읽지 않게 합니다: 캔버스 호스트 루트는 배치용 컨테이너라 접근성 요소가 아니고 "Canvas"는 캔버스 뷰에만 둡니다. 상태 화면 제목은 그룹 레이블로 전달하므로 제목 레이블 자체는 접근성 트리에서 뺍니다.
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

- 창 크롬: full-size content, 표준 스타일 마스크, 툴바 스타일 automatic, 기본 툴바 항목 전체, 툴바에 도구 영역 없음, 상단 바 구분선 없음, 도구 막대가 콘텐츠 하단 가운데에 있고 치수가 도구 그룹과 같음, 툴바 구성 autosave, 내비게이션 그룹 navigational
- 분할: 항목 3개, sidebar·inspector behavior, autosave 이름, 보조 패널 처음 접힘, pane 폭이 허용 범위 안, 창 폭 변경 시 캔버스만 변함, 캔버스가 툴바·사이드바·인스펙터와 겹치지 않음, drawable 크기 > 0
- Auto Layout: ready, 각 pane 전환 후, 로딩·오류·빈 상태에서 모호한 레이아웃 없음(숨긴 뷰 제외)
- 접근성: pane·캔버스 레이블(캔버스 컨테이너는 요소 아님), 툴바·도구·검색·더 보기 레이블과 도움말, 내비게이션 아이콘 테두리 없음, 상태 화면 레이블(제목 레이블 중복 없음), SF Symbol 이름 유효
- 사용자 구성 palette: `willBeInsertedIntoToolbar: false` 사본을 만들고 해제한 뒤에도 항목 참조(내비게이션·검색·더 보기)가 실제 툴바 항목이고, 도구 선택 snapshot·로딩 비활성이 도구 막대에, "Search Project…" 검증이 실제 검색 항목에 닿음. 사본은 명령을 보내지 않음
- O2 설정 값: 가운데 항목 `automaticallyAdjustsSafeAreaInsets == false`(겹침 검사만으로는 잡히지 않음)
- 검색 미제공(`isSearchAvailable == false`): ready에서도 검색 필드·메뉴 비활성
- 동작 메서드 guard: 로딩 상태에서 `navigateBack(_:)`·`navigateForward(_:)`를 직접 불러도 명령을 보내지 않음
- 명령: 창이 first responder일 때 응답자 체인으로 사이드바·인스펙터·보조 패널 전환과 되돌리기, 메뉴 제목 반영, 애니메이션 경로 전환
- snapshot: 다음 런 루프 병합, 연속 snapshot 1회 반영, 오래된 세대 폐기, 내비게이션 활성 상태, 도구·검색·더 보기·상태 화면 버튼 명령 전달, 대표 버튼 1개
- 상태 저장: frame·split autosave 기록과 새 창에서의 복원(크기, 인스펙터 접힘), 창 복원 설정, provider 미등록 시 복원 실패 보고

검사용 autosave 이름은 `StellarShellFrameUICheck` 접두어를 쓰고, 시작과 끝에 이 접두어의 user defaults 기록을 지웁니다(중간에 비정상 종료한 이전 실행 기록 포함).

## 13. 코어 연결과 통합에 남은 일

- 끝난 일(14장): `AppDelegate`가 `CanvasWindowController` 대신 셸 프레임 창을 쓰고, `ShellWindowRestoration.windowProvider` 등록과 secure restorable state를 지원합니다. 메인 메뉴에 `ShellFrameMenuItems` 항목을 설치했습니다. 사이드바·인스펙터·보조 패널 슬롯에 실제 영역 view controller를 끼웠습니다(내용은 영역 data source Stub).
- 프로젝트 창의 `ShellFrameDataSource`(`ShellProjectFrameDataSource`)는 코어 프로젝트 열기 결과만 씁니다. 코어 내비게이션·검색·페이지 이름 snapshot은 아직 없어 뒤로·앞으로·검색은 비활성입니다. [아키텍처 §8.2] [ADR-0009]
- 끝난 일(XII.2.2): 창 controller(`ShellWindowController.windowSession`)가 코어 `ProjectWindowSession` handle(`ShellWindowSessionHandle`) 하나를 소유합니다. 셸은 프로젝트 session을 연 뒤 창 session까지 열린 다음에 창을 ready로 두고, 창 session을 열지 못하면 프로젝트 session을 닫고 "The Project’s Design Couldn’t Be Loaded"로 안내합니다. 창을 닫으면 창 session → 프로젝트 session 순서로 한 번씩 닫습니다. 프로젝트 session handle(`ShellProjectSessionHandle`)은 그대로 data source가 소유합니다. [가이드 2.5] [계획 XII.2.1, XII.2.2] [ADR-0028]
- `NativeFieldsPanel`(텍스트 입력 검증 fixture)은 `--text-input-check` 전용 창(`CanvasWindowController`)에 남아 있습니다. 셸 창의 텍스트 편집 중 Undo 경로는 `--tools-menus-ui-check`가 검사합니다. [가이드 10.1]
- 선택 속성과 에이전트 세션을 연결하면 해당 영역의 `ShellAreaDataStubs`를 교체합니다. 사이드바의 활성 문서 트리와 Git 변경 목록은 창별 코어 데이터를 사용합니다. 보조 패널 노출 여부·위치는 가이드 2.4·O4·ADR-0005에 따릅니다.
- 다중 창(XII.2.4)에서 창별 `ShellWindowConfiguration` 이름 규칙을 정합니다. 지금은 모든 프로젝트 창이 `.standard` 이름을 써서 둘째 창부터 frame autosave가 걸리지 않고 split autosave는 서로 덮어씁니다.
- 모든 프로젝트 창의 캔버스(`CanvasView`)가 앱의 코어 runtime handle 하나를 함께 씁니다. 코어에 창별 session·view state가 없어 창마다 보내는 drawable 크기(`stellar_runtime_set_drawable_size`)와 입력(`stellar_runtime_submit_mac_event`)이 같은 runtime으로 가므로, 프로젝트 창이 둘 이상이면 마지막으로 보낸 창의 drawable 크기가 남습니다. 코어 `ProjectWindowSession`·창별 view state가 생기면 캔버스 호스트가 창 session으로 보내도록 바꿉니다. [계획 XII.2.1, XII.2.5]
- 툴바 스타일(O1), 캔버스 확장(O2), 배치 수치 확정(가이드 12장)을 XII.9.1에서 정합니다.
- UI 문자열 현지화 표(String Catalog)는 아직 없습니다. 셸 프레임 문자열은 `ShellFrameStrings`에서 `String(localized:)`로 만들고, "Show/Hide \<pane>" 같은 상태 제목은 이어 붙이지 않고 형식 문자열 하나로 만듭니다. Stub 데이터 문자열은 코어가 줄 값의 표본이라 현지화 대상이 아닙니다.
- 실제 창에서 Liquid Glass 재질, 키 창 강조, 대비 높이기·투명도 줄이기 조합을 눈으로 확인하는 작업은 XII.9.4 적합성 보고서에서 합니다.

## 14. 앱 통합

### 14.1 구성

```text
AppDelegate ─ RuntimeHost(코어 runtime 1개)
            └ ShellAppCoordinator
                ├ MainMenuBuilder 메뉴 + ShellMainMenuInstaller(셸 프레임 항목)
                ├ 앱 수준 router(ShellApplicationCommandModel: Open Project…, Settings…)
                ├ ShellRuntimeEventPump(코어 완료 event pull)
                ├ WelcomeWindowController(ShellWelcomeDataSource) — 열린 프로젝트가 없을 때
                └ ShellProjectWindow × n
                    ├ ShellWindowController(ShellProjectFrameDataSource ─ ShellProjectSessionHandle)
                    ├ 슬롯: 사이드바(SidebarViewController + 필터 accessory), 캔버스(ShellCanvasHostViewController),
                    │       인스펙터(InspectorViewController), 보조 패널(탭: GitPanelViewController, AgentPanelViewController)
                    ├ ShellWindowToolsModelStub → ShellCommandRouter(응답자 체인) → ToolsToolbarController(툴바 provider: 줌)
                    ├ ShellWindowToolsModelStub → ShellProjectFrameDataSource 도구 목록 → 하단 도구 막대(ShellToolBarView)
                    └ 캔버스 keyDownHandler(단일 키 도구)
```

- 시작: `applicationWillFinishLaunching`에서 메뉴 설치와 복원 provider 등록(AppKit은 `applicationDidFinishLaunching` 전에 창을 복원), `applicationDidFinishLaunching`에서 복원된 프로젝트 창이 없으면 시작 창. `applicationSupportsSecureRestorableState`는 true.
- 열기: 시작 창 Open Project…, 메뉴 막대 Open Project…(⌘O, 폴더 선택 패널), Finder·`open` 명령으로 받은 폴더(`application(_:open:)`)가 모두 `ShellAppCoordinator.openProject(atPath:origin:)`로 갑니다. 코어 `stellar_project_open`이 열기 분류와 안전 probe를 마치고 `PROJECT_OPENED`를 보내면 그때 창을 만들고 시작 창을 닫습니다. 같은 폴더(표준화·심볼릭 링크 해소 경로)를 연 창이 있으면 새 창·새 session 없이 그 창을 앞으로 가져오고, 같은 폴더 열기가 진행 중이면 다시 요청하지 않습니다(가이드 2.5). 실패 창의 Locate Folder…로 고른 폴더를 다른 열기가 여는 중이면 그 실패 창을 닫고 여는 중인 열기가 만드는 창 하나만 남기며, 여는 중인 폴더의 창 복원도 새 창을 만들지 않습니다.
- 창 제목은 매니페스트의 프로젝트 이름, 부제목은 "Restricted Mode"입니다. 코어가 신뢰 저장소 연결 전이라 모든 프로젝트를 신뢰되지 않음으로 열기 때문입니다(가이드 7.2).
- 닫기: 정상 닫기 요청은 변경 없는 창에서 즉시 허용하고, 변경 또는 미완료 명령이 있으면 창을 유지한 채 비동기 저장 결과를 확인합니다. 실패(stale 포함)·응답 timeout에는 원래 창과 편집을 유지하고 다시 시도·저장하지 않고 닫기·취소를 고르는 시트를 띄웁니다([ADR-0028 닫기 실패 선택 시트](../../adr/ADR-0028-window-session-shell-surface.md)). 앱 종료(⌘Q)도 저장이 필요한 창을 같은 경로로 저장한 뒤에만 종료합니다. 실제로 창이 닫히면 data source의 session handle이 `stellar_project_close`를 한 번 요청하고, 마지막 프로젝트 창이면 닫히기 전에 시작 창을 띄웁니다(앱 종료 아님). 복원·Try Again·Locate Folder… 열기가 끝나기 전에 창을 닫으면 창이 기다리던 요청 id를 지우고, 늦게 온 `PROJECT_OPENED`는 창에 붙이지 않고 그 session을 바로 닫습니다(창 닫기 = 프로젝트 닫기). 시작 창만 남은 상태에서 그 창을 닫으면 앱이 종료됩니다(`applicationShouldTerminateAfterLastWindowClosed`, II.5.3 lifecycle 유지). 활성 에이전트 세션이 있으면 창 닫기·앱 종료 전에 확인합니다(ADR-0005 결정 4.5, Return은 Cancel).
- 복원: 창 상태에는 프로젝트 폴더 경로 문자열 하나만 secure coding으로 넣습니다(`StellarProjectRootPath`). 복원 시 열기 중 창을 바로 만들고 같은 경로를 다시 엽니다. 실패하면 창에 실패 상태와 복구 동작(Locate Folder…[대표], Try Again, Close Window)을 보여 줍니다.
- 이벤트 펌프: 코어는 callback 없이 완료 event를 큐에 쌓으므로 셸이 꺼냅니다. 기다리는 요청이 있을 때만 1/120초 주기 timer(common mode)를 켜고, 한 번에 최대 64개를 꺼내 요청 id별 완료 handler를 부릅니다. 기다리는 요청이 없으면 timer를 끕니다. UI 스레드는 코어 작업을 기다리지 않습니다. [아키텍처 §6.3, §9.2] [ADR-0009]
- FFI 호출은 `CoreRuntime`(FFI 헬퍼)에만 둡니다. `UI/Shell`은 `stellar_` 함수를 직접 부르지 않고, `UI/Tools`는 FFI와 runtime handle을 모릅니다(smoke 구조 검사).

### 14.2 툴바 조합

`ShellToolbarController`는 `ShellToolbarItemProviding` provider(`ToolsToolbarController`)를 받으면 provider의 뒤쪽 항목(줌)을 검색 앞에 넣어 기본 항목을 다음 순서로 바꿉니다.

`[사이드바 토글] [사이드바 경계] [뒤로·앞으로] [유연] [줌] [검색] [더 보기] [인스펙터 경계] [유연] [인스펙터 토글]`

- 캔버스 도구는 상단 툴바에 넣지 않습니다. 도구 영역은 캔버스 하단 가운데 도구 막대(`ShellToolBarView`, 6.1)에 있으므로(2026-11-14 사용자 결정) provider는 가운데 항목을 내주지 않고, 상단 툴바에는 가운데 고정 항목(`centeredItemIdentifiers`)이 없습니다. provider 항목(줌)의 생성·snapshot 반영·palette 사본은 provider가 맡습니다.
- 하단 도구 막대에는 창 도구 모델(`ShellWindowToolsModelStub`)의 캔버스 도구 10개를 그립니다. `ShellProjectWindow`가 도구 목록과 현재 도구를 `ShellProjectFrameDataSource.setTools`로 셸 프레임 snapshot(`tools`, `activeToolID`, 명령 ID는 `tool.move` 같은 셸 명령 ID)에 보내고, 막대에서 고른 도구(`selectTool`)는 router로 창 도구 모델에 보냅니다. 선택 표시는 창 도구 모델 snapshot이 돌아올 때 바뀌고, 막대 활성 상태는 셸 프레임 상태(ready)로 정해집니다.
- 이전 구조(캔버스 도구 그룹을 상단 툴바 가운데에 둠)에서 1200 × 760pt 창은 도구 그룹이 넓어 줌·더 보기·뒤로·앞으로가 overflow(»)로 넘어갔습니다(14.7). 도구 막대를 옮긴 뒤의 상단 툴바 배치는 화면에서 다시 확인하지 않았습니다. 같은 명령은 모두 메뉴 막대에도 있습니다. 배치 확정은 XII.9.1입니다.

### 14.3 응답자 체인

창에 `contentViewController`(split controller)가 있으므로 router는 content view가 아니라 그 controller와 창 사이에 끼웁니다: content view → split controller → router → 창. AppKit은 view controller가 소유한 view의 `nextResponder` 설정을 controller의 `nextResponder`로 넘기므로 content view에 직접 끼우면 체인이 순환합니다(통합 중 AppKit 경고 "infinite loop responder chain"으로 확인, `ShellCommandRouter.install(in:)`에서 수정). pane 전환(`toggleSidebar:` 등)은 split controller가 router보다 먼저 받습니다.

### 14.4 메뉴 막대에 더한 항목(`ShellMainMenuInstaller`)

- View: Show/Hide Sidebar·Inspector 묶음에 Show/Hide Panels(보조 패널), 다음 묶음에 Back·Forward(단축키 없음)
- Edit ▸ Find: Search Project…(코어 검색이 없어 비활성)
- File: Show in Finder(더 보기 동작과 같은 명령 ID)
- 메뉴 전체의 key equivalent는 명령 하나에만 있습니다(통합 검사).

### 14.5 영역 data source(Stub)

사이드바는 창의 활성 Page·View와 레이어를 코어에서 읽습니다. Git 패널은 프로젝트 신뢰와 실제 Git 이름 변경을 사용합니다. `ShellAreaDataStubs`는 인스펙터 선택 없음과 에이전트 세션 없음 상태를 제공합니다. 창별 도구 상태는 `ShellWindowToolsModelStub`이 보관합니다. 메뉴는 창 세션이 제공하는 Undo·Redo·Save Now와 줌 명령의 활성 상태를 반영합니다.

### 14.6 통합 검사(`--shell-integration-check`)

실제 코어 runtime과 임시 fixture 프로젝트로 `ShellAppCoordinator`를 화면 밖에서 구동합니다. 프로젝트 창은 검사 전용 autosave 이름(`ShellCheckAutosave`, 접두어 `StellarShellIntegrationCheck`·`StellarProjectOpenCheck`)으로 만들고 시작·끝에 그 기록을 지웁니다. 앱의 실제 창 기록(`StellarMainWindow`, split, 툴바 구성)을 읽거나 바꾸지 않는지도 검사 전후로 비교합니다. 실제 기록을 쓰면 사용자가 펼친 보조 패널 같은 저장 상태에 따라 검사 결과가 달라지기 때문입니다.

- 메뉴: Open Project…·Settings… 활성(앱 수준 router target), New·Clone Project 비활성, 셸 프레임 항목 설치, key equivalent 중복 없음
- 시작 창 표시 → 시작 창 Open Project → 진행 표시 → 같은 폴더 중복 요청 없음 → 펌프 timer 켜짐·완료 후 꺼짐 → 창 1개, 시작 창 닫힘, 코어 프로젝트 session 1개
- 창 구성: 제목·제한 모드 부제목·ready, 복원 설정, 가운데 캔버스 호스트(runtime handle·drawable > 0), 사이드바·인스펙터·Git·에이전트 view controller, 보조 패널 접힘, 사이드바 필터 accessory, 툴바 조합(상단 툴바에 캔버스 도구 그룹·셸 도구 영역·가운데 고정 항목 없음, 줌이 검색 앞), 하단 도구 막대(콘텐츠 영역, 캔버스 도구 10개, 현재 도구 선택), 검색 비활성, 응답자 체인(순환 없음)
- 도구: 하단 도구 막대 → 창 모델 → 메뉴 체크·막대 선택 표시, 캔버스 R 키 → Rectangle(막대 선택 표시 반영), 도구 아닌 키·⌘R은 코어 입력으로, 코어 경로 없는 명령 비활성, Close Project·Show in Finder 활성, Search Project… 비활성
- 같은 폴더 재열기 시 새 창·요청 없음, 복원 상태 secure coding 왕복(열린 폴더는 기존 창), 창 닫기 → session 닫기 → 시작 창 → 코어 session 0
- 복원: 열기 중 → ready, 없어진 폴더 → 실패 상태·복구 동작 3개(대표 1개)·도구 명령과 하단 도구 막대 비활성 → Locate Folder…로 같은 창에서 열기
- 열기 중 닫기: 복원 열기 대기 중 창 닫기 → 목록에서 빠짐·시작 창 → 늦게 온 열기 결과가 닫힌 창에 붙지 않음 → 그 session 닫힘(코어 session 0)
- 여는 중인 폴더: 앱 수준 열기 대기 중 같은 폴더 복원은 새 창 없음, 실패 창의 Locate Folder…로 같은 폴더를 고르면 실패 창이 닫히고 창 1개·코어 session 1개
- 실패 안내: 앱 수준 열기 실패 → alert, 시작 창 열기 실패(stellar.yaml 없음) → 시작 창 안내, New Project → 이름 선택·취소, 폴더 생성과 최초 문서 열기, 같은 이름 보호, 재열기, 설정 창. 원인별 문구 대응은 14.8의 `--project-open-check`가 검사합니다.

### 14.7 화면 확인(2026-11-14, macOS 27.2, Debug 앱)

아래 프로젝트 창 기록은 도구 막대를 캔버스 하단으로 옮기기 전 구조(캔버스 도구 그룹이 상단 툴바 가운데)에서 본 것입니다.

- 프로젝트 창(`open -n -a StellarMac.app <fixture>`, 1200 × 760pt): 왼쪽 사이드바(필터 필드, Pages·Layers 머리글, "No View Selected" 빈 상태), 툴바 제목 "Sample Design Project"와 부제목 "Restricted Mode", 가운데 캔버스 도구 그룹 11개(Move 선택), 검색 버튼과 overflow 버튼(»; 줌·더 보기·뒤로·앞으로는 보이지 않음), 오른쪽 인스펙터 "No Selection", 인스펙터 토글. 캔버스는 빈 영역(렌더러 미연결, XII.4.1).
- 같은 프로젝트 창을 저장된 프레임 1440 × 900pt로 다시 띄운 화면(키 창): overflow 없이 뒤로·앞으로(비활성), 제목·부제목, 캔버스 도구 그룹, 줌 "100%"(비활성, camera 미연결), 검색, 더 보기(…), 인스펙터 토글이 모두 보였습니다.
- 시작 창(인자 없이 실행): 앱 아이콘, "Stellar", Version, New Project…·Open Project…·Clone Project… 버튼, "No Recent Projects".
- 실행 기록: 셸(백그라운드 프로세스)에서 띄운 실행 일부에서 창이 만들어졌는데도 창 목록에 현재 화면 창으로 나오지 않았습니다(시작 창 1회, 프로젝트 창 1회. 프로젝트 창은 창 번호로는 캡처됐음). 다시 실행하면 표시된 경우도 있어 실행 환경(Space·활성화 양보)에 따른 것으로 보이나, 원인은 확인하지 못했습니다. Finder에서 실행하는 경로는 확인하지 않았습니다.

### 14.8 프로젝트 열기 실패 안내(XII.6.2)

Open Project는 폴더를 고른 뒤 코어 `stellar_project_open`에 넘기고, 코어가 열기 분류(IV.2.4)와 신뢰되지 않은 프로젝트용 안전 probe(IV.5.3)를 마친 뒤에만 창을 만듭니다. 실패는 코어 고정 코드(`STELLAR_FAILURE_*`)나 FFI 호출 상태를 `ShellProjectOpenFailure`로 바꿔 알립니다. 코어 진단 메시지(한국어, 경로 포함 가능)는 화면에 쓰지 않고, 안내에는 전체 경로 대신 폴더 이름만 넣습니다.

| 원인(코드) | 제목 | 해결 방법(요지) |
|---|---|---|
| `NOT_A_REPOSITORY`(10), 경로에 폴더·일반 파일 있음 | This Folder Isn’t a Git Repository | 프로젝트 최상위 폴더(Git 저장소 + stellar.yaml)를 고름 |
| `NOT_A_REPOSITORY`(10), 경로에 항목 없음 | Project Folder Not Found | 폴더 찾기 또는 다른 프로젝트 열기 |
| `NOT_A_STELLAR_PROJECT`(11) | This Folder Isn’t a Stellar Project | 저장소를 자동 변환하지 않음(가이드 7.2) |
| `MANIFEST_UNREADABLE`(12) | The Project File Couldn’t Be Read | 폴더 읽기 권한 확인 후 다시 시도 |
| `MANIFEST_CORRUPT`(13) | The Project File Is Damaged | YAML 손상·1 MiB 초과. Git 이력에서 복원 |
| `MANIFEST_INVALID`(14) | The Project File Isn’t Valid | 필수 설정 누락·지원하지 않는 값 |
| `INVALID_PROJECT_ID`(15) | The Project ID Isn’t Valid | project.id는 UUID. Git 이력에서 복원 |
| `UNTRUSTED_SYMLINK`(16) | The Project File Is a Symbolic Link | 신뢰 전에는 링크를 따르지 않음. 링크를 파일로 바꿈 |
| `PATH_UNRESOLVED`(17) | The Project Folder Couldn’t Be Located | 열기 중 옮겨지거나 지워짐 |
| `STELLAR_INVALID_ARGUMENT`·`INVALID_UTF8`(빈·상대 경로) | The Folder Path Isn’t Valid | 이 Mac의 폴더를 고름 |
| `STELLAR_QUEUE_FULL` | Stellar Is Busy | 잠시 뒤 다시 시도 |
| runtime 없음·그 밖의 호출 실패 | Stellar Can’t Open Projects Right Now | 앱을 다시 열기 |
| 모르는 코드·예상 밖 event | The Project Couldn’t Be Opened | 다시 시도 또는 다른 프로젝트 |

- "폴더 없음"은 코어가 `NOT_A_REPOSITORY`를 보낸 경우에만 셸이 경로에 항목이 있는지 한 번 확인해 구분합니다(실패 경로의 로컬 stat 한 번). 옮겨진 최근·복원 프로젝트를 "Git 저장소 아님"으로 잘못 안내하지 않기 위해서입니다. Finder·`open` 명령으로 받은 일반 파일은 폴더는 아니지만 있으므로 "폴더 없음"이 아니라 "Git 저장소 아님"(최상위 폴더를 고름)으로 안내합니다.
- 안내 위치: 메뉴 막대·Finder에서 연 실패는 alert(열린 창이 있으면 sheet), 시작 창에서 연 실패는 시작 창 안내(`stellar.yaml` 없음은 `.notStellarProject`, 폴더 없음은 `.projectMissing`, 나머지는 `.operationFailed(.projectOpenFailed)`이며 문구는 원인 제목이 앞에 붙음), 창 복원·Locate Folder… 실패는 그 창의 실패 상태(제목·해결 방법·복구 동작)입니다.
- 실패한 열기는 창과 코어 session을 만들지 않습니다.
- 검사: `--project-open-check`(`ShellProjectOpenCheck`). 대응표 전체, 원인별 제목이 서로 다름, 폴더 이름 표시·전체 경로 비노출, 메뉴 ⌘O → 폴더 선택(취소 시 요청 없음) → 코어 열기 → 창, 실제 코어로 연 잘못된 경로 fixture 10종(폴더 없음, 일반 폴더, 폴더가 아닌 일반 파일, Git만, YAML 손상, 1 MiB 초과, 스키마 위반, 잘못된 project.id, 심볼릭 링크 매니페스트, 읽기 권한 없음)과 상대·빈 경로 2종의 코드 → 원인 대응, alert·시작 창·복원 안내 위치, 실패 후 창·session 0.
