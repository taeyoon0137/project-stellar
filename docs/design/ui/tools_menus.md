# 도구·메뉴 영역 명세 (tools_menus)

- 상태: 초안(2026-11-14)
- 코드: `apps/macos/StellarMac/UI/Tools/`
- 기준: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), 구현 계획(이하 "계획"), Apple HIG
- 근거 표기는 가이드 0.1절을 따릅니다. 이 문서는 가이드·아키텍처·계획을 바꾸지 않으며, 어긋나면 그 문서들이 우선입니다. [가이드 0.2]

## 1. 범위

이 영역은 다음을 맡습니다.

- 캔버스 도구 집합 11종의 툴바 도구 그룹과 캔버스 단일 키 해석
- 줌 컨트롤 툴바 항목
- 메인 메뉴 막대 전체(`MainMenuBuilder`)
- 메뉴·툴바 명령의 단일 식별자(`ShellCommand`)와 라우팅(`ShellCommandRouter`)

다음은 이 영역 밖입니다.

- 도구별 상호작용 상태 기계: 코어가 구현합니다. [계획 XII.3.6]
- 캔버스 뷰 `keyDown(with:)` 연결, 창·분할 구조, 사이드바·인스펙터·검색 툴바 항목: 각 영역과 통합 단계가 맡습니다. [가이드 2.3, 6.1]
- 명령의 실제 동작(문서 변경, 저장, 프로젝트 열기): 코어가 맡고 셸은 명령만 보냅니다. [가이드 1.2 원칙 4] [아키텍처 §8.1, §8.2]

## 2. 구성 파일

| 파일 | 역할 |
|---|---|
| `ShellCommand.swift` | 명령 enum(70개). 제목, 범위(앱·창), 소유(셸·코어), 표준 action, 단축키, SF Symbol |
| `CanvasTool.swift` | 도구 enum(11개), `CanvasToolShortcut` 단일 키 해석기 |
| `KeyShortcut.swift` | 단축키 값 타입, 표시 문자열, 물리 키 정규화 |
| `ShortcutPolicy.swift` | HIG 표준·시스템 단축키 표와 충돌 감사 |
| `ToolsMenuModel.swift` | snapshot·프로토콜, `ShellCommandPresentation`, `ZoomFormat`, `ShellCommandCoreIDStub`, `ToolsMenuModelStub` |
| `ShellCommandRouter.swift` | 응답자 체인 router, 메뉴 검증 |
| `MainMenuBuilder.swift` | 메뉴 막대 구성과 앱 연결 |
| `ToolsToolbarController.swift` | 도구 그룹·줌 툴바 항목, 툴바 delegate |
| `ToolsUICheck.swift` | 정적 검사 진입점 `ToolsUICheck.run()` |

Xcode 프로젝트는 file-system synchronized group(`PBXFileSystemSynchronizedRootGroup`)이므로 새 Swift 파일은 프로젝트 파일 수정 없이 빌드에 포함됩니다.

## 3. 표시 내용

### 3.1 툴바

기본 배치는 가이드 6.1의 세 구역을 따릅니다. 사이드바·인스펙터 토글과 경계 정렬은 시스템 식별자를 씁니다. [가이드 6.1] [AppKit 문서 NSToolbarItem.Identifier]

| 위치 | 항목 | 구성요소 |
|---|---|---|
| 앞쪽 끝 | 사이드바 토글, 사이드바 경계 구분선 | `.toggleSidebar`, `.sidebarTrackingSeparator`(커스터마이즈 불가) |
| (툴바 밖) | 도구 그룹(Tools) | 캔버스 하단 가운데 도구 막대 `ShellToolBarView`로 옮김(2026-11-14, [셸 프레임 명세](shell_frame.md) 툴바 절) |
| 가운데 | (유연 공간) | `.flexibleSpace` |
| 뒤쪽 끝 | 줌(Zoom) | `NSMenuToolbarItem`, 제목은 현재 배율 |
| 뒤쪽 끝 | 인스펙터 경계 구분선, 인스펙터 토글 | `.inspectorTrackingSeparator`, `.toggleInspector` |

- 프로젝트 검색 툴바 항목과 더 보기 메뉴는 검색 영역이 같은 delegate에 추가합니다. 이 영역은 `ToolsToolbarController.item(for:willBeInsertedIntoToolbar:)`로 항목만 내줄 수 있습니다. [가이드 6.1]
- 사용자 구성 palette: AppKit은 palette에 보일 항목을 `willBeInsertedIntoToolbar: false`로 따로 요청하고 palette를 닫으면 해제합니다. 이때는 `paletteItem(for:)`가 실제 항목과 다른 사본을 만들고 target·action을 달지 않습니다. 실제 항목 인스턴스를 palette에 넘기면 한 항목이 두 곳에 놓입니다.
- 도구 그룹은 한 번 클릭으로 도구를 바꿔야 하므로 펼친 표현을 고정합니다. macOS 26 SDK로 빌드해 macOS 27.2에서 실행해 보니 `.automatic`은 이 11개 그룹을 창 폭과 상관없이 pull-down 하나로 접었습니다. 창이 좁으면 시스템 overflow 메뉴가 처리하고, 줌 항목(`visibilityPriority = .low`)이 도구 그룹(`.high`)보다 먼저 overflow로 넘어갑니다. [HIG Toolbars]
- 측정값(같은 환경, 화면 밖 창, 참고용이며 확정 수치 아님): 펼친 도구 그룹 폭 423.5pt. 툴바만 있는 창(분할 뷰 없음)에서 폭 1000pt 이상이면 모든 기본 항목이 보이고, 900pt이면 줌 항목만 overflow로 넘어갑니다. 분할 뷰가 붙으면 사용 가능한 폭이 달라지므로 통합 후 다시 측정합니다. 확정 수치는 가이드 12장에 기록하는 별도 작업으로 정합니다. [가이드 0.2]

도구 그룹 순서와 표시:

| 순서 | 도구 | 레이블 | SF Symbol | 단일 키 | 도움말 태그 |
|---:|---|---|---|---|---|
| 1 | move | Move | `cursorarrow` | V | Move (V) |
| 2 | frame | Frame | `number` | F | Frame (F) |
| 3 | rectangle | Rectangle | `rectangle` | R | Rectangle (R) |
| 4 | ellipse | Ellipse | `circle` | O | Ellipse (O) |
| 5 | line | Line | `line.diagonal` | L | Line (L) |
| 6 | polygon | Polygon | `pentagon` | 없음 | Polygon |
| 7 | pen | Pen | `pencil.tip` | P | Pen (P) |
| 8 | text | Text | `textformat` | T | Text (T) |
| 9 | image | Image | `photo` | 없음(메뉴 ⇧⌘K) | Image |
| 10 | hand | Hand | `hand.raised` | H | Hand (H) |
| 11 | zoom | Zoom | `magnifyingglass` | Z | Zoom (Z) |

줌 항목의 pull-down 메뉴: Zoom In, Zoom Out, 구분선, Actual Size, Zoom to Fit, Zoom to Selection. 이 메뉴에는 key equivalent를 넣지 않습니다. 단축키는 View 메뉴에만 둡니다(같은 단축키를 두 메뉴에 두지 않음).

### 3.2 메뉴 막대

순서는 앱(Stellar), File, Edit, View, 앱 고유 메뉴(Insert, Object), Window, Help입니다. Format 메뉴는 서식 있는 텍스트 편집 명령이 아직 없으므로 두지 않습니다. 생기면 Edit와 View 사이에 넣습니다. [HIG The menu bar] [가이드 8.1] 앱 고유 메뉴의 이름과 구성은 가이드의 열린 결정 O8입니다. 이 문서는 Insert(만들기 도구)와 Object(정렬·배치)를 제안안으로 구현합니다.

`|`는 구분선입니다. `▸`는 하위 메뉴입니다.

- **Stellar**: About Stellar | Settings… | Services ▸ | Hide Stellar, Hide Others, Show All | Quit Stellar
- **File**: New Project…, Open Project…, Clone Project… | Close, Close Project | Save Now
- **Edit**: Undo, Redo | Cut, Copy, Paste, Delete, Duplicate | Select All, Deselect All | Find ▸ (Find…, Find Next, Find Previous)
- **View**: Show Toolbar, Customize Toolbar… | Show Sidebar, Show Inspector | Zoom In, Zoom Out, Actual Size, Zoom to Fit, Zoom to Selection | Move Tool, Hand Tool, Zoom Tool | Show Rulers | Enter Full Screen
- **Insert**: Frame | Rectangle, Ellipse, Line, Polygon | Pen, Text | Image
- **Object**: Group, Ungroup, Frame Selection | Bring to Front, Bring Forward, Send Backward, Send to Back | Align ▸ (Align Left, Align Center, Align Right | Align Top, Align Middle, Align Bottom), Distribute ▸ (Distribute Horizontally, Distribute Vertically) | Flip Horizontal, Flip Vertical | Lock, Hide
- **Window**: Minimize, Minimize All(Option 동적 항목), Zoom | Bring All to Front (창 목록은 시스템이 덧붙임)
- **Help**: Stellar Help

규칙:

- 모든 명령 항목은 `ShellCommand` 하나에 대응합니다. 항목 identifier는 `xyz.taeyoon.stellar.command.<rawValue>`이고, 하위 메뉴 컨테이너만 명령이 없습니다. [계획 XII.6.5]
- 툴바의 모든 명령은 메뉴 막대에도 있습니다(도구 11개는 View·Insert, 줌 5개는 View). [HIG Toolbars] [가이드 6.1]
- 추가 입력이 필요한 명령에만 말줄임표를 붙입니다(Settings…, New/Open/Clone Project…, Find…, Customize Toolbar…). 도구 선택은 체크 표시가 붙는 모드 전환이라 Image를 포함해 말줄임표를 붙이지 않습니다. [HIG The menu bar] [가이드 8.1]
- 제목 형식: View 메뉴의 Move Tool·Hand Tool·Zoom Tool은 탐색 도구라 "Tool"을 붙이고, Insert 메뉴 항목은 만들 대상 이름(Frame, Rectangle, …)만 씁니다. 메뉴 이름이 이미 "삽입"을 말하므로 대상 이름이 더 짧고 명확합니다.
- 지금 쓸 수 없는 항목은 숨기지 않고 비활성화합니다. [HIG The menu bar] [가이드 8.1]
- 표시·숨김 항목의 제목은 현재 상태를 반영합니다. Show/Hide Sidebar·Inspector·Toolbar와 Enter/Exit Full Screen은 AppKit이 바꾸고, Show/Hide Rulers, Lock/Unlock, Hide/Show는 `ShellCommandPresentation`이 snapshot으로 바꿉니다. [HIG The menu bar]
- 메뉴 아이콘: macOS 26은 표준 action 항목(undo:, copy:, performClose: 등)에 시스템 아이콘을 붙입니다. 셸 명령 항목에는 같은 의미의 SF Symbol을 붙이고, 구분선으로 나뉜 그룹에 아이콘이 없는 표준 항목(예: toggleInspector:, toggleToolbarShown:)이 섞이면 그 항목만 SF Symbol로 채웁니다. 시스템 아이콘은 바꾸지 않습니다. 결과적으로 모든 그룹이 아이콘을 모두 갖거나 모두 갖지 않습니다. [가이드 8.1] [HIG The menu bar]

## 4. 상태

| 상태 | 판정 | 메뉴 | 툴바 |
|---|---|---|---|
| 정상 | snapshot의 `enabledCommands`에 명령이 있음 | 활성. 현재 도구에 체크, 상태 반영 제목 | 현재 도구 선택, 배율 제목 |
| 빈 상태(열린 프로젝트 창 없음) | 창 router가 응답자 체인에 없음 | 앱 수준 셸 명령(Settings…, New/Open/Clone Project…)은 앱 수준 router가 받아 활성. About·Hide·Quit·Help는 AppKit 표준 동작. 창 수준 명령은 받을 대상이 없어 AppKit이 비활성화 | 창이 없으므로 툴바 없음 |
| 로딩(세션 준비 중) | 코어가 `enabledCommands`를 비우거나 줄여 보냄 | 숨기지 않고 비활성 | 도구 그룹·줌 항목 `isEnabled = false`, 마지막 배율 제목 유지 |
| 오류(명령 거부·실패) | 명령 뒤 snapshot이 바뀌지 않거나 다른 값을 보고함 | 다음 snapshot 기준으로 다시 검증 | 도구 클릭으로 바뀐 선택을 snapshot의 도구로 되돌림. 오류 설명 alert는 해당 기능이 맡음 |
| 텍스트 편집 중 | first responder가 `NSText`(field editor 포함) | 객체 명령(Duplicate, Deselect All, Object 메뉴 전체) 비활성. Cut·Copy·Paste·Delete·Select All은 field editor가, Undo·Redo는 창 undo manager가 처리("Undo Typing") | 변화 없음(도구·줌은 텍스트 편집과 무관) |
| 오래된 snapshot | `generation`이 이미 반영한 값보다 작음 | 해당 없음(메뉴는 매번 최신 snapshot을 읽음) | 무시 |

- 셸은 낙관적으로 상태를 바꾸지 않습니다. 단 도구 그룹 세그먼트는 클릭 순간 AppKit이 먼저 선택을 바꾸므로, 명령이 거부되면 snapshot의 도구로 되돌립니다. [아키텍처 §9.3]
- 줌 배율 제목은 지역화된 백분율이며(`ZoomFormat`) 유효하지 않은 값은 "–"로 표시합니다.

## 5. 상호작용

### 5.1 도구 선택

세 경로 모두 같은 `ShellCommand`(`tool.*`)를 보냅니다.

1. 툴바 도구 그룹 클릭 → `ToolsToolbarController.toolSelectionChanged` → `ShellCommandRouter.dispatch`
2. View·Insert 메뉴 → `performShellCommand(_:)` → 활성 창 router
3. 캔버스 포커스 중 단일 키 → 캔버스 `keyDown(with:)`가 `CanvasToolShortcut.tool(for:)`로 도구를 구한 뒤 `router.dispatch(tool.command)` (통합 단계에서 연결)

명령은 `ToolsMenuModel.send`로 나가고, 결과 snapshot의 `activeTool`이 툴바 선택과 메뉴 체크를 정합니다.

- Image 도구 선택은 다른 도구처럼 모드 전환이므로 메뉴 제목이 "Image"이고 체크 표시가 붙습니다. 이미지 파일 선택 패널은 도구를 쓰는 단계(캔버스 클릭)에서 열고, 취소 시 이전 도구 복귀와 함께 코어 연결 시 정합니다.
- Space를 누르는 동안 Hand로 전환, Escape로 Move 복귀 같은 일시 전환은 도구 상태 기계의 일입니다. [계획 XII.3.6] [가이드 O10]

### 5.2 줌

- 툴바 줌 pull-down, View 메뉴, 단축키가 같은 줌 명령 5개를 씁니다.
- 줌 단계, 맞춤·선택 영역 맞춤 계산은 코어 camera가 정합니다. 셸은 snapshot의 `zoomScale`만 표시합니다. [계획 XII.4.3] [가이드 9]
- Stub은 ×2 / ÷2(2%~25600%)와 맞춤 명령 시 100%만 흉내 냅니다.

### 5.3 편집 명령 라우팅

- Undo, Redo, Cut, Copy, Paste, Delete, Select All은 AppKit 표준 action을 target 없이 보냅니다. 텍스트 컨트롤 편집 중에는 field editor가 먼저 받아 네이티브 클립보드를 처리하고, 캔버스·목록 포커스에서는 `ShellCommandRouter`가 받아 활성 창 session으로 보냅니다. [가이드 8.2] [아키텍처 §11.2, §23]
- 텍스트 편집 중 Undo·Redo: field editor(`NSTextView`)는 `undo:`·`redo:`를 구현하지 않고 창 undo manager가 처리합니다. 그래서 router는 텍스트 컨트롤이 first responder이면 `responds(to:)`에서 `undo:`·`redo:`에 응답하지 않는다고 알려 응답자 체인과 메뉴 검증이 창까지 가게 합니다(메뉴 제목 "Undo Typing"). 직접 불려도 창으로 넘깁니다. 이 경로가 없으면 이름 입력 중 ⌘Z가 캔버스 "Undo Move"를 보냅니다(리뷰에서 재현). [가이드 5.2, 8.2]
- `NSWindow`가 `undo:`·`redo:`에 응답하므로 router는 `install(in:)`으로 창 바로 앞에 끼웁니다. 창에 `contentViewController`가 있으면 그 controller와 창 사이(content view → controller → router → 창), 없으면 content view와 창 사이입니다. AppKit은 view controller가 소유한 view의 `nextResponder` 설정을 controller로 넘기므로 content view에 직접 끼우면 체인이 순환합니다(셸 통합 중 확인). content view나 controller를 바꾸면 다시 호출해야 합니다.
- Undo·Redo 제목은 snapshot의 대상 이름을 붙입니다(예: "Undo Move"). [HIG The menu bar]
- Delete 키 자체(⌫)는 메뉴 key equivalent로 두지 않습니다. 텍스트 입력을 가로채지 않도록 캔버스 `keyDown`→`deleteBackward(_:)` 경로로 `.delete`를 보내는 연결은 통합 단계에서 합니다.

### 5.4 앱 수준 명령

New/Open/Clone Project와 Settings는 창이 없어도 동작해야 하므로 `MainMenuBuilder.build(applicationTarget:)`에 넘긴 앱 수준 router를 명시 target으로 씁니다. About, Hide, Quit, Help, Minimize All, Bring All to Front는 AppKit 표준 action입니다.

## 6. 단축키

### 6.1 메뉴 단축키 전체 표

분류: **표준**은 HIG Keyboards 표준 단축키 표의 원래 의미, **AppKit**은 AppKit 기본 메인 메뉴 템플릿의 관례, **사용자 정의**는 HIG 사용자 정의 규칙에 맞춘 Stellar 지정입니다.

| 메뉴 | 항목 | 단축키 | 명령 ID | 분류 |
|---|---|---|---|---|
| Stellar | About Stellar | — | `app.about` | — |
| Stellar | Settings… | ⌘, | `app.settings` | 표준 |
| Stellar | Hide Stellar | ⌘H | `app.hide` | 표준 |
| Stellar | Hide Others | ⌥⌘H | `app.hide_others` | 표준 |
| Stellar | Show All | — | `app.show_all` | — |
| Stellar | Quit Stellar | ⌘Q | `app.quit` | 표준 |
| File | New Project… | ⌘N | `file.new_project` | 표준(새 문서) |
| File | Open Project… | ⌘O | `file.open_project` | 표준(열기) |
| File | Clone Project… | — | `file.clone_project` | — |
| File | Close | ⌘W | `file.close_window` | 표준 |
| File | Close Project | ⇧⌘W | `file.close_project` | 표준(파일과 관련 창 모두 닫기) |
| File | Save Now | ⌘S | `file.save_now` | 표준(저장) |
| Edit | Undo | ⌘Z | `edit.undo` | 표준 |
| Edit | Redo | ⇧⌘Z | `edit.redo` | 표준 |
| Edit | Cut | ⌘X | `edit.cut` | 표준 |
| Edit | Copy | ⌘C | `edit.copy` | 표준 |
| Edit | Paste | ⌘V | `edit.paste` | 표준 |
| Edit | Delete | — | `edit.delete` | — |
| Edit | Duplicate | ⌘D | `edit.duplicate` | 사용자 정의 |
| Edit | Select All | ⌘A | `edit.select_all` | 표준 |
| Edit | Deselect All | ⇧⌘A | `edit.deselect_all` | 표준 |
| Edit ▸ Find | Find… | ⌘F | `edit.find` | 표준 |
| Edit ▸ Find | Find Next | ⌘G | `edit.find_next` | 표준 |
| Edit ▸ Find | Find Previous | ⇧⌘G | `edit.find_previous` | 표준 |
| View | Show/Hide Toolbar | ⌥⌘T | `view.toggle_toolbar` | 표준 |
| View | Customize Toolbar… | — | `view.customize_toolbar` | — |
| View | Show/Hide Sidebar | ⌃⌘S | `view.toggle_sidebar` | AppKit(예외, 6.3) |
| View | Show/Hide Inspector | ⌥⌘I | `view.toggle_inspector` | 표준 |
| View | Zoom In | ⌘+ (숨은 보조 ⌘=) | `view.zoom_in` | 표준(선택 크기 키우기) |
| View | Zoom Out | ⌘- | `view.zoom_out` | 표준(선택 크기 줄이기) |
| View | Actual Size | ⌘0 | `view.zoom_actual_size` | 사용자 정의 |
| View | Zoom to Fit | ⌘9 | `view.zoom_to_fit` | 사용자 정의 |
| View | Zoom to Selection | ⌘* | `view.zoom_to_selection` | 사용자 정의 |
| View | Move Tool | 캔버스 V | `tool.move` | 단일 키(6.2) |
| View | Hand Tool | 캔버스 H | `tool.hand` | 단일 키 |
| View | Zoom Tool | 캔버스 Z | `tool.zoom` | 단일 키 |
| View | Show/Hide Rulers | ⌘R | `view.toggle_rulers` | 사용자 정의(계획 XII.10.19, Stub에서 비활성) |
| View | Enter/Exit Full Screen | ⌃⌘F | `view.toggle_full_screen` | 표준 |
| Insert | Frame | 캔버스 F | `tool.frame` | 단일 키 |
| Insert | Rectangle | 캔버스 R | `tool.rectangle` | 단일 키 |
| Insert | Ellipse | 캔버스 O | `tool.ellipse` | 단일 키 |
| Insert | Line | 캔버스 L | `tool.line` | 단일 키 |
| Insert | Polygon | — | `tool.polygon` | — |
| Insert | Pen | 캔버스 P | `tool.pen` | 단일 키 |
| Insert | Text | 캔버스 T | `tool.text` | 단일 키 |
| Insert | Image | ⇧⌘K | `tool.image` | 사용자 정의 |
| Object | Group | ⌥⌘G | `object.group` | 사용자 정의(예외, 6.3) |
| Object | Ungroup | ⌥⇧⌘G | `object.ungroup` | 사용자 정의(예외, 6.3) |
| Object | Frame Selection | — | `object.frame_selection` | — |
| Object | Bring to Front | — | `object.bring_to_front` | — |
| Object | Bring Forward | ⌘] | `object.bring_forward` | 사용자 정의 |
| Object | Send Backward | ⌘[ | `object.send_backward` | 사용자 정의 |
| Object | Send to Back | — | `object.send_to_back` | — |
| Object ▸ Align | Align Left | ⌘{ | `object.align_left` | 표준(왼쪽 정렬) |
| Object ▸ Align | Align Center | ⌘\| | `object.align_center` | 표준(가운데 정렬) |
| Object ▸ Align | Align Right | ⌘} | `object.align_right` | 표준(오른쪽 정렬) |
| Object ▸ Align | Align Top, Align Middle, Align Bottom | — | `object.align_top` 등 | — |
| Object ▸ Distribute | Distribute Horizontally, Distribute Vertically | — | `object.distribute_*` | — |
| Object | Flip Horizontal, Flip Vertical | — | `object.flip_*` | — |
| Object | Lock/Unlock | — | `object.toggle_lock` | — |
| Object | Hide/Show | — | `object.toggle_visibility` | — |
| Window | Minimize | ⌘M | `window.minimize` | 표준 |
| Window | Minimize All(Option 동적) | ⌥⌘M | `window.minimize_all` | 표준 |
| Window | Zoom | — | `window.zoom` | — |
| Window | Bring All to Front | — | `window.bring_all_to_front` | — |
| Help | Stellar Help | ⌘? | `help.app_help` | 표준 |

- 표기는 HIG 순서(Control, Option, Shift, Command)를 따르고, 두 글자 키의 윗글자는 Shift 없이 그 글자로 씁니다(⌘+, ⌘{, ⌘?, ⌘*). [HIG Keyboards]
- Zoom In의 숨은 보조 항목(⌘=)은 `isHidden` + `allowsKeyEquivalentWhenHidden`이라 메뉴에 보이지 않고 Shift 없이 누른 경우도 받습니다.
- Bring to Front·Send to Back에는 단축키를 두지 않습니다. 대괄호에 Option을 더하면 모든 키보드 배열에서 입력할 수 없는 문자에 수식키를 더하는 형태가 되기 때문입니다. [HIG Keyboards]

### 6.2 캔버스 단일 키

| 키 | 도구 | 키 | 도구 |
|---|---|---|---|
| V | Move | T | Text |
| F | Frame | H | Hand |
| R | Rectangle | Z | Zoom |
| O | Ellipse | (없음) | Polygon |
| L | Line | (없음) | Image(메뉴 ⇧⌘K) |
| P | Pen | | |

- 메뉴 key equivalent로 두지 않고 캔버스 뷰 `keyDown(with:)`에서만 해석합니다. 텍스트 컨트롤이 first responder이면 캔버스가 키를 받지 않으므로 발동하지 않습니다. [가이드 8.2]
- 수식키(Caps Lock 제외)가 있거나 키 반복이면 해석하지 않습니다. Shift+글자는 향후 도구 변형용으로 비워 둡니다.
- 라틴 배열은 입력 문자로 판정해 배열 차이(AZERTY 등)를 따르고, 한글 등 비라틴 입력 소스에서는 미국 ANSI 물리 키 위치로 판정합니다(예: 두벌식 "ㄱ" 자리 = R).
- 메뉴에는 key equivalent 대신 도움말 태그("Canvas shortcut: R")로 알리고, 툴바 도움말 태그에도 "Rectangle (R)"처럼 함께 표시합니다. [가이드 8.1] [HIG Offering help]
- 단일 키와 Command 조합(예: R과 ⌘R)은 서로 다른 입력입니다. HIG의 "기존 단축키에 수식키를 더하지 않는다" 규칙은 메뉴 단축키끼리에 적용했습니다.

### 6.3 충돌 점검

`ShortcutPolicy.audit()`가 모든 메뉴 단축키(숨은 보조 포함 42개)를 미국 ANSI 물리 키로 정규화한 뒤(예: ⌘+ → ⇧⌘=, ⌘{ → ⇧⌘[) 다음을 검사합니다. `ToolsUICheck`가 실행할 때마다 확인하며 현재 결과는 **문제 0건**입니다.

1. 같은 물리 키 조합을 서로 다른 명령이 쓰지 않습니다.
2. 시스템 전역 단축키를 쓰지 않습니다. HIG 표준 표의 시스템 항목(⌘Space, ⌘Tab, ⌘\`, ⌥⌘Esc, ⇧⌘3·4, ⌥⌘8, ⌥⌘-·=, ⌥⌘D, ⌃⌘D, ⇧⌘Q, ⌃⌥⌘,·. 등)과 표 밖 시스템 단축키(⇧⌘5, ⌃⌘Q)가 대상입니다.
3. HIG 표준 단축키는 원래 의미의 명령에만 씁니다. Stellar가 쓰지 않는 표준 단축키는 비워 둡니다: ⌘B, ⌘I, ⌘U, ⌘T(Format 메뉴용), ⇧⌘C, ⌥⌘C, ⌃⌘C, ⌘E, ⌥⌘F, ⌘J, ⌘P, ⇧⌘P, ⇧⌘S, ⇧⌘V, ⌥⌘V, ⌥⇧⌘V, ⌃⌘V, ⌥⌘W, ⌘:, ⌘;, ⌘.
4. 사용자 정의 단축키는 Command를 포함하고 Control을 쓰지 않습니다(예외 아래).
5. 관련 없는 기존 단축키에 Shift·Option·Control만 더한 파생 단축키를 만들지 않습니다. Undo/Redo, Select All/Deselect All, Find Next/Previous, Close/Close Project, Minimize/Minimize All, Hide/Hide Others, Group/Ungroup은 서로 관련된 묶음으로 봅니다.

의도적 예외와 근거:

| 명령 | 단축키 | 규칙 | 근거 |
|---|---|---|---|
| Show/Hide Sidebar | ⌃⌘S | 4(Control), 5(⌘S 파생) | AppKit 기본 메인 메뉴 템플릿의 Show Sidebar 관례. 가이드 2.3의 "표준 단축키" 경로 |
| Group | ⌥⌘G | 5(⌘G Find Next 파생) | ⌘G는 표준 Find Next라 재사용하지 않음. 그룹은 자주 쓰는 명령이라 Option 변형을 씀. Apple iWork 앱의 Group 단축키와 같다고 알려져 있으나 이 작업에서 직접 확인하지 못함 |
| Ungroup | ⌥⇧⌘G | 5 | Group과 짝 |

감사기 자체의 음성 검사: 표준 재사용(⌘B), 시스템 단축키(⌘Space), 충돌, Command 없음, Control 사용, 파생(⇧⌘B), 정규화 충돌(⌘+ 대 ⇧⌘=)을 일부러 넣으면 모두 문제로 보고하는지 `ToolsUICheck`가 확인합니다.

## 7. 접근성

- **VoiceOver 레이블**: 메뉴 항목은 제목, 툴바 도구는 레이블과 SF Symbol 접근성 설명(도구 이름), 도구 그룹은 "Tools", 줌 항목은 레이블 "Zoom"과 배율 제목을 씁니다. 검사에서 화면 밖 창의 툴바 뷰 계층에 "Zoom" 접근성 레이블이 있는지 확인합니다. [HIG Accessibility] [가이드 8.3] [계획 XII.8.1]
- **현재 도구 노출**: 도구 그룹은 단일 선택 세그먼트이므로 선택 상태가 접근성으로 노출되고, 메뉴에서는 현재 도구에 체크 표시가 붙습니다. 색만으로 상태를 나타내지 않습니다. 캔버스 호스트 자체의 도구 상태 노출은 XII.8.1에서 합니다. [가이드 4.2, 8.3]
- **키보드**: 모든 명령이 메뉴 막대에 있어 Full Keyboard Access로 실행할 수 있습니다. 툴바 포커스 이동 같은 시스템 키보드 탐색 단축키는 가로채지 않습니다(감사 규칙 2). [HIG Keyboards] [가이드 8.1, 8.2]
- **도움말 태그**: 아이콘만 있는 툴바 도구마다 도움말 태그를 둡니다. [HIG Offering help] [가이드 8.1]
- **투명도 줄이기, 대비 높이기, 동작 줄이기, Light/Dark**: 이 영역은 표준 구성요소(`NSToolbarItemGroup`, `NSMenuToolbarItem`, `NSMenu`)만 쓰고 커스텀 뷰·색·애니메이션이 없으므로 시스템이 처리합니다. 하드코딩 색은 없습니다. 검사는 Light, Dark, 고대비 Light, 고대비 Dark 외형에서 레이아웃을 확인합니다. [가이드 3.2, 4.2, 8.4, 8.5]

## 8. SF Symbols

도구 SF Symbol은 3.1절 표에 있습니다. 메뉴 아이콘(셸 명령 항목과 아이콘이 빠진 표준 항목 채움용):

| 명령 | Symbol | 명령 | Symbol |
|---|---|---|---|
| Settings | `gearshape` | Zoom In / Out | `plus.magnifyingglass` / `minus.magnifyingglass` |
| New Project | `folder.badge.plus` | Actual Size | `1.magnifyingglass` |
| Open Project | `folder` | Zoom to Fit | `arrow.up.left.and.arrow.down.right` |
| Clone Project | `square.and.arrow.down.on.square` | Zoom to Selection | `viewfinder` |
| Close Project | `xmark.rectangle` | Rulers | `ruler` |
| Save Now | `square.and.arrow.down` | Full Screen | `arrow.up.left.and.arrow.down.right.rectangle` |
| Duplicate | `plus.square.on.square` | Group / Ungroup | `square.on.square.dashed` / `square.on.square` |
| Deselect All | `circle.dashed` | Frame Selection | `number.square` |
| Find / Next / Previous | `magnifyingglass` / `chevron.forward` / `chevron.backward` | Bring to Front / Forward | `square.3.layers.3d.top.filled` / `square.2.layers.3d.top.filled` |
| Show Toolbar | `menubar.rectangle` | Send Backward / to Back | `square.2.layers.3d.bottom.filled` / `square.3.layers.3d.bottom.filled` |
| Customize Toolbar | `wrench.adjustable` | Align L·C·R | `align.horizontal.left` / `.center` / `.right` |
| Show Sidebar / Inspector | `sidebar.left` / `sidebar.right` | Align T·M·B | `align.vertical.top` / `.center` / `.bottom` |
| Lock / Hide | `lock` / `eye.slash` | Distribute H·V | `distribute.horizontal.center` / `distribute.vertical.center` |
| Flip H | `arrow.left.and.right.righttriangle.left.righttriangle.right` | Flip V | `arrow.up.and.down.righttriangle.up.righttriangle.down` |
| 하위 메뉴 Find / Align / Distribute | `magnifyingglass` / `align.horizontal.left` / `distribute.horizontal.center` | | |

표준 action 항목 중 macOS 27.2 실행에서 시스템 아이콘이 붙는 것으로 관찰한 항목(About, Hide, Hide Others, Show All, Quit, Undo, Redo, Cut, Copy, Paste, Delete, Select All, Close, Show Sidebar, Customize Toolbar, Minimize, Zoom, Bring All to Front, Help)은 시스템 아이콘을 그대로 씁니다. 같은 실행에서 Show Toolbar, Show Inspector, Enter Full Screen에는 시스템 아이콘이 없었고, 앞의 둘은 같은 그룹 규칙에 따라 위 표의 값으로 채웁니다(Enter Full Screen은 혼자인 그룹이라 아이콘 없음). 검사는 모든 Symbol 이름이 실행 OS에 있는지 확인합니다. 이미지 에셋은 쓰지 않습니다. [가이드 5.4]

## 9. 데이터·명령 경계(프로토콜)

```swift
struct ToolsMenuSnapshot: Equatable {
    var generation: UInt64              // 단조 증가 세대, 오래된 값 폐기
    var activeTool: CanvasTool
    var zoomScale: Double               // 1.0 = 100%
    var enabledCommands: Set<ShellCommand>
    var undoActionName: String?
    var redoActionName: String?
    var rulersVisible: Bool
    var selectionIsLocked: Bool
    var selectionIsHidden: Bool
}

protocol ToolsMenuModel: AnyObject {
    var snapshot: ToolsMenuSnapshot { get }                          // 저장 값, 계산 없음
    func addSnapshotObserver(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation
                                                                     // 메인 스레드, 구독자 여럿, 토큰 해제 시 끝
    func send(_ command: ShellCommand)                               // 비동기 반영
}
```

- 구독은 여럿일 수 있습니다(툴바, 캔버스 호스트, 그 밖에 같은 창 session을 보는 영역). 구현체는 `ToolsMenuObservers`로 구독자 목록을 두고, 반환한 `ToolsMenuObservation` 토큰이 해제되거나 `cancel()`되면 그 구독만 끝납니다. 한 구독자가 다른 구독자를 덮어쓰지 않습니다.
- 값 snapshot이 들어오고 명령이 나가는 구조이며 Rust 타입이 없습니다. 코어 연결 시 pull 이벤트 큐(`stellar_runtime_poll_event`)로 받은 버전형 메시지를 이 구조체로 디코드하고, `send`는 command ID와 인자를 Document Thread 큐에 넣기만 합니다. [아키텍처 §8.2, §9.2] [ADR-0009]
- FFI 표현 제안: `enabledCommands`는 command ID 비트셋, `activeTool`은 작은 정수 tag, 이름 문자열은 UTF-8 `(ptr, len)`. 형식은 코어 command ID가 정해질 때 함께 정합니다.
- 메뉴 검증은 `snapshot` 읽기와 제목 비교만 하므로 메인 스레드 예산 안입니다. 측정값은 10장에 있습니다. [아키텍처 §6.3]
- `ShellCommandCoreIDStub`: 코어 command ID 체계가 아직 없어(`crates/commands`는 골격) 코어 소유 명령에 임시 문자열 `stellar.stub.<rawValue>`를 주고, 셸 전용 명령(창·메뉴·툴바·시스템 동작)은 nil을 줍니다. [계획 XII.6.5]
- `ToolsMenuModelStub`: 코어 없이 영역을 띄우는 의도적 placeholder입니다. FFI를 호출하지 않고, 도구 전환·줌·Lock/Hide만 흉내 내며, 눈금자와 Redo를 비활성으로 둡니다. 이 제한은 검사가 확인합니다. [계획 1.2 규칙 6]

## 10. 검사

`ToolsUICheck.run()`(통합 시 `scripts/macos/smoke.sh`에 연결)은 Stub 데이터로 메뉴·router·툴바를 화면 밖에 만들고 다음을 확인합니다. 실패 항목은 표준 오류로 출력합니다.

- 명령 70개: 제목, 말줄임표 규칙, SF Symbol 존재, identifier 왕복, 코어 ID Stub(셸 전용 nil, 코어 소유 고유·임시 접두사)
- 단축키 감사 0건, 감사기 음성 검사 7종
- 단일 키 해석: 11개 도구, Caps Lock, 수식키·반복 거부, 한글 입력 소스(물리 키), `NSEvent` 경로
- 메뉴: 최상위 순서, 모든 명령 항목의 `ShellCommand` 대응·action·target·key equivalent, 모든 명령이 메뉴에 정확히 한 번 보임, 동적 항목, 아이콘 그룹 균일, Services 연결
- 라우팅: 응답자 체인 위치(content view → router → window), 캔버스 포커스의 copy:·undo:·셸 명령 전달, 메뉴 제목·체크·활성, 도구 그룹 클릭과 snapshot 반영, 오래된 snapshot 폐기, 비활성 도구 거부와 선택 복귀, 로딩 상태, 텍스트 편집 중 억제와 field editor 우선, 창 없이 New Project
- 텍스트 편집 중 Undo·Redo: router가 `undo:`·`redo:`에 응답하지 않고 체인의 처리 대상이 창, 창 검증 제목이 캔버스 명령 이름이 아님, ⌘Z·⇧⌘Z가 field editor 입력을 되돌리고 다시 적용, router 직접 호출도 창으로 넘김, 코어 명령 미전달
- 구독: 두 번째 구독자가 snapshot을 받고 툴바 구독을 덮어쓰지 않음, 취소한 구독은 더 받지 않음
- palette 사본: 도구 그룹·줌 사본이 실제 항목과 다르고 router로 명령을 보내지 않음, 실제 항목 유지
- 툴바: 레이블·도움말 태그·아이콘 접근성 설명, 펼친 표현, overflow 우선순위, 커스텀 뷰 없음, 1280pt에서 overflow 없음
- 레이아웃: 4개 외형과 좁은 폭(560pt)에서 창 전체 뷰 계층의 모호한 Auto Layout 없음
- 성능: snapshot 반영과 View 메뉴 전체 검증 median이 2ms 미만

2026-11-14 로컬 실행 결과(Apple Silicon, macOS 27.2, Debug 앱 바이너리를 임시 실행 인자로 호출, 여러 번 실행): `tools-menus ok: 70 commands, 83 menu items, shortcut audit 0 issues, 11 tools`, snapshot 반영 median 약 0.09ms(최대 0.18ms 이하), View 메뉴 전체 검증 median 약 0.02ms.

## 11. 코어 연결·통합에 남은 일

- **command ID**: 코어가 command ID를 정의하면 `ShellCommandCoreIDStub`을 그 표로 바꿉니다. [계획 XII.6.5]
- **snapshot 수신**: pull 이벤트 디코더와 `ToolsMenuModel` 실제 구현(창 session별 1개, 앱 수준 1개). `ToolsMenuModelStub`은 검사·미리보기용으로만 남깁니다. `MainMenuBuilder.build(applicationTarget:)`의 target은 필수 인자입니다(빠뜨리면 창이 없을 때 New/Open/Clone Project가 비활성으로 남음).
- **도구 상태 기계**: Space 일시 Hand, Escape 복귀, Image 파일 선택 흐름. [계획 XII.3.6]
- **줌**: camera 입력과 줌 단계·맞춤 계산. [계획 XII.4.3]
- **프로젝트 명령**: New/Open/Clone Project 흐름과 패널. [계획 XII.6.1~XII.6.3]
- **Undo/Redo/Save Now(완료)**: 메뉴·단축키는 응답자 체인으로 key 창의 router가 받아 그 창의 코어 창 session(`ShellWindowSessionHandle`)에 `stellar_window_session_perform`으로 보냅니다. Undo는 그 창 undo stack, Redo는 redo stack이 비어 있지 않을 때, Save Now는 작업 문서가 있을 때 활성입니다. 저장 실패(다른 저장이 먼저 끝남, 쓰기 실패)는 alert 한 번으로 알립니다. 검사는 `--session-commands-check`입니다. [계획 XII.6.4] [ADR-0028]
- **접근성**: 캔버스 호스트의 도구 상태 노출과 VoiceOver 자동화. [계획 XII.8.1]
- **창 통합(완료, shell_frame 14장)**: `ShellAppCoordinator`가 `MainMenuBuilder.build(applicationTarget:)`·`install(on:)`을 하고, 프로젝트 창마다 `ShellCommandRouter.install(in:)`과 셸 툴바 조합(`ToolsToolbarController`가 `ShellToolbarItemProviding`으로 줌 제공)을 합니다. 캔버스 도구 그룹은 셸 툴바에 넣지 않고, 같은 도구 11개를 프로젝트 창이 셸 프레임 snapshot 도구 목록으로 캔버스 하단 가운데 도구 막대(`ShellToolBarView`)에 보냅니다(shell_frame 14.2). 창 모델은 `ShellWindowToolsModelStub`(창별 도구 상태, 코어 경로 없는 명령 비활성)입니다. [가이드 2.3, 2.5]
- **캔버스 keyDown(단일 키 완료)**: `CanvasView.keyDownHandler`가 `CanvasToolShortcut.tool(for:)`로 도구를 구해 `router.dispatch`합니다. 처리한 키는 key down·짝 key up 모두 코어 입력으로 보내지 않고, 도구가 아니거나 비활성이면 코어 입력으로 보냅니다. Delete 키(`deleteBackward(_:)` → `.delete`)는 코어 삭제 명령 경로가 없어 아직 연결하지 않았습니다.
- **앱 이름**: 메뉴 막대의 굵은 앱 이름은 번들 이름에서 옵니다. 현재 `PRODUCT_NAME`이 `StellarMac`이므로 표시 이름을 "Stellar"로 맞추는 프로젝트 설정 변경이 필요합니다(이 작업 범위 밖, 프로젝트 파일 미수정).
- **smoke 연결(완료)**: `--tools-menus-ui-check`로 `ToolsUICheck.run()`을 실행하고, `UI/Tools/`가 코어 FFI·runtime(`stellar_`, `CoreRuntime`, `RuntimeHost`, `runtimeHandle`)을 참조하지 않는다는 구조 검사를 `scripts/macos/smoke.sh`가 합니다.
- **눈금자**: 지금은 placeholder이고 동작은 계획 XII.10.19가 구현합니다.
- **WBS 없음**: Open Recent, Open in New Window(XII.2.4), Export, Format 메뉴, Help Book(현재 Stellar Help는 시스템 "도움말 없음" 동작), 메뉴 문자열 지역화.
- **실화면 확인**: 화면 밖 bitmap 캡처에서는 선택된 도구 세그먼트의 아이콘이 보이지 않았습니다. 캡처가 시스템 glass·선택 레이어를 담지 못한 것으로 보이나, 실제 키 창 화면에서는 확인하지 않았습니다.

## 12. 이 영역이 제안하는 결정(확정 아님)

| 대상 | 제안 | 정할 곳 |
|---|---|---|
| 가이드 O8 앱 고유 메뉴 | Insert(만들기 도구), Object(그룹·순서·정렬·분배·뒤집기·잠금·숨김) | XII.6.5 |
| 단일 키 배정 | 6.2절 표(Polygon·Image 없음) | XII.3.6 또는 가이드 O10과 함께 |
| 줌 단축키 | ⌘+/⌘-/⌘0/⌘9/⌘* | XII.6.5 |
| 도구 그룹 표현 | 펼친 표현 고정, 줌이 먼저 overflow | XII.9.1 |
