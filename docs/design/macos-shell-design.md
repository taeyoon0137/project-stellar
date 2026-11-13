# macOS 셸 디자인 가이드

- 상태: 초안(2026-11-14)
- 디자인 컨셉: **macOS 네이티브 최적화**(제품 책임자 결정, 2026-11-14)
- 범위: macOS 앱 셸(Apple Silicon, macOS 26 이상, ADR-0006). 캔버스 문서 렌더링 결과는 범위 밖입니다.

## 0. 문서 사용법

### 0.1 근거 표기

모든 규칙 끝에 근거를 붙입니다. 근거가 없는 규칙은 두지 않습니다.

| 표기 | 의미 |
|---|---|
| `[PO 2026-11-14]` | 제품 책임자 결정 "UI 디자인 컨셉은 macOS 네이티브 최적화" |
| `[아키텍처 §n]` | 기술 아키텍처 명세의 해당 절 |
| `[계획 ID]` | 구현 계획의 WBS 행(예: `XII.9.1`) |
| `[ADR-000n]` | `docs/adr/`의 해당 ADR |
| `[HIG <주제>]` | Apple Human Interface Guidelines의 해당 주제 |
| `[AppKit 문서 <대상>]` | Apple AppKit 공개 레퍼런스의 해당 클래스·속성 |

### 0.2 이 문서의 위치

- 이 문서는 아키텍처 명세, 구현 계획, ADR을 바꾸지 않습니다. 서로 어긋나면 그 문서들이 우선이고, 이 문서를 고칩니다. [AGENTS.md 지침 우선순위]
- 이 문서는 원칙과 배치만 정합니다. 각 화면의 세부 UX는 해당 WBS 구현 작업이 정합니다.
- 폭, 반경, 간격, 지속 시간, 색 값 같은 **수치는 이 문서에 두지 않습니다.** 필요해지면 해당 구현 작업에서 Apple HIG와 Stellar 성능 계약(§6)을 근거로 측정해 정하고, 작업 보고서와 이 문서의 12장(확정 수치 기록)에 남깁니다. 경계를 넘는 결정이면 ADR로 올립니다. [ADR-0005 결정 9와 같은 원칙]

---

## 1. 목적과 원칙

### 1.1 목적

Stellar macOS 셸이 운영체제 기본 구성요소 위에서 일관된 모습과 동작을 갖게 하고, 디자인 결정이 성능 계약과 코어·셸 경계를 깨지 않게 합니다.

### 1.2 원칙

1. **시스템 기본값이 디자인입니다.** 셸의 시각 언어는 자체 테마가 아니라 운영체제가 제공하는 창 크롬, 재질, 시스템 색, 시스템 글꼴, 표준 컨트롤입니다. [PO 2026-11-14] [아키텍처 §3.5, §8.1]
2. **커스텀 표현은 디자인 도구 고유 요소에만 둡니다.** 선택 핸들, 가이드, 측정 표시처럼 운영체제에 대응 구성요소가 없는 것만 Stellar가 직접 정합니다. 이것들도 셸 뷰가 아니라 코어 렌더러의 transient scene layer로 그립니다. [아키텍처 §10] [계획 XII.4.4(선택 overlay·핸들)] 가이드·측정 표시는 WBS 없음(10.4).
3. **승인되지 않은 커스텀 모사 컨트롤을 만들지 않습니다.** 표준 AppKit 구성요소로 할 수 있는 것을 직접 그리지 않습니다. 표준 구성요소로 할 수 없어 커스텀 컨트롤이 필요하면 ADR로 승인받고, 승인된 ADR 번호를 해당 구현 작업 보고서에 남깁니다. [계획 XII.9.1 완료 기준] [0.2절]
4. **셸은 의미를 소유하지 않습니다.** 셸은 창, 메뉴, 툴바, 사이드바, 인스펙터, 다이얼로그, 네이티브 입력·접근성만 맡고, 문서 의미·레이아웃·렌더링 의미·저장·Semantic Diff·Merge는 코어가 맡습니다. [아키텍처 §3.5, §8.1, §8.2]
5. **셸 표현과 캔버스 결과를 분리합니다.** 셸의 재질·외형·강조 색은 캔버스 문서 렌더링에 적용하지 않습니다. 캔버스 프리뷰는 실행 OS가 아니라 선택한 Target Profile을 기준으로 합니다. [아키텍처 §10.2] [계획 XII.9.1]
6. **성능 계약이 장식보다 우선합니다.** 어떤 시각 효과도 §6의 인터랙션 예산과 UI 스레드 점유 상한을 넘게 할 수 없습니다. [아키텍처 §6.3, §6.4]
7. **본체 UI에 Electron, DOM, WebView를 쓰지 않습니다.** WebView는 향후 플러그인 UI의 격리된 호스트로만 씁니다. [아키텍처 §8.1]

---

## 2. 창 구조

### 2.1 창 크롬

- 메인 창은 표준 `NSWindow`(titled, closable, miniaturizable, resizable)와 `NSToolbar`로 만듭니다. 커스텀 창 프레임과 커스텀 창 컨트롤(신호등 버튼 대체)을 만들지 않습니다. [HIG Windows] [AppKit 문서 NSWindow, NSToolbar] [아키텍처 §8.1]
- 콘텐츠는 full-size content view로 창 상단까지 확장합니다. 제목 표시줄 높이와 창 컨트롤 위치는 직접 계산하지 않고 툴바 스타일이 결정하게 합니다. [AppKit 문서 NSWindow.StyleMask, NSWindow.toolbarStyle]
- 툴바 스타일은 기본값(automatic)에서 시작하고, unified와 unified compact 중 무엇을 고를지는 시안 단계에서 정합니다(열린 결정 O1). [AppKit 문서 NSWindow.ToolbarStyle]
- 창 모서리 근처에 놓이는 콘텐츠는 macOS 26의 모서리 적응 레이아웃 영역(`NSView.LayoutRegion`, `layoutGuide(for:)`)으로 배치해 둥근 창 모서리에 잘리지 않게 합니다. [AppKit 문서 NSView.LayoutRegion]
- 전체 화면 전환에서 툴바와 창 컨트롤 처리는 시스템 동작을 따르고, `NSWindowDelegate` 전체 화면 콜백에서 시스템 동작을 덮어쓰지 않습니다. [HIG Going full screen] [AppKit 문서 NSWindowDelegate]

### 2.2 창 제목

- 창 제목은 그 창에 열린 프로젝트·문서를 식별합니다. Window 메뉴와 VoiceOver가 이 제목으로 창을 구분합니다. [HIG Windows] [아키텍처 §18 "앱 자체 UI는 네이티브 접근성 API 사용"]
- 제목에 앱 이름을 반복하지 않습니다. [HIG Windows]
- 제목에 넣는 프로젝트 유래 문자열은 화면 표시에만 쓰고, OS 알림 본문·텔레메트리·진단 기록에는 넣지 않습니다. [아키텍처 §39]

### 2.3 기본 영역

```text
┌─────────────────────────── 툴바 (NSToolbar) ───────────────────────────┐
├──────────────┬──────────────────────────────────────┬──────────────────┤
│ 사이드바      │ 캔버스                               │ 인스펙터          │
│ 문서·페이지·  │ 코어 렌더러가 그림                   │ 속성·Value        │
│ View, 레이어  │ (CAMetalLayer, Metal backend)        │ Binding·접근성    │
│ 트리          │                                      │ 의미 (탭 자리)    │
└──────────────┴──────────────────────────────────────┴──────────────────┘
```

- 메인 창은 "사이드바 | 캔버스 | 인스펙터" 3열을 기본 구조로 합니다. 영역 구성은 툴바, 왼쪽 사이드바(문서·페이지·View 목록과 레이어 트리), 가운데 캔버스, 오른쪽 인스펙터입니다. [HIG Sidebars, HIG Split views] [아키텍처 §3.5] [계획 XII.9.1(toolbar, sidebar, inspector)]
- Git 패널은 새 창 계층을 만들지 않고 이 구조 안의 전환 가능한 구획 또는 탭으로 들어갑니다. 위치는 열린 결정 O4입니다(6.4절). [HIG Windows] [계획 XIV.13.5]
- 에이전트 패널(로드맵)은 인스펙터 영역 탭을 후보 자리로 고려합니다(6.5절). [ADR-0005 Migration 5]
- 3열은 `NSSplitViewController`로 구성합니다. 사이드바는 `NSSplitViewItem(sidebarWithViewController:)`, 인스펙터는 `NSSplitViewItem(inspectorWithViewController:)`로 만들어 재질, 접기·펼치기, 폭 유지 동작을 시스템에 맡깁니다. [AppKit 문서 NSSplitViewController, NSSplitViewItem]
- 캔버스는 항상 남는 공간을 모두 차지합니다. 패널 폭 변경은 캔버스 폭만 바꾸며, 바뀐 drawable 크기는 기존 CanvasHost 경로로 코어에 전달합니다. [HIG Split views] [계획 II.5.4, XII.4.1]
- 분할선은 얇은 스타일을 우선합니다. 각 pane에는 분할선이 사라지지 않도록 최소·최대 폭을 두고, 허용 범위는 코드 한곳에서 정의합니다. 값은 구현 작업에서 정합니다. [HIG Split views]
- 사이드바와 인스펙터 표시 전환은 View 메뉴 항목(현재 상태를 반영한 제목), 표준 단축키, 툴바 버튼 세 경로를 모두 제공하고 같은 명령 하나로 연결합니다. 숨긴 pane은 캔버스에 공간을 돌려줍니다. [HIG Sidebars, HIG The menu bar]
- 캔버스 영역을 사이드바·인스펙터 아래까지 확장할지(`automaticallyAdjustsSafeAreaInsets`, 배경 확장 뷰 `NSBackgroundExtensionView`)는 제품 결정입니다(열린 결정 O2). 결정 전에는 캔버스를 다른 pane과 겹치지 않게 둡니다. 겹치면 drawable 크기, hit test 영역, 렌더 비용 계산이 달라지기 때문입니다. [AppKit 문서 NSSplitViewItem, NSBackgroundExtensionView] [아키텍처 §6.2]
- pane 하나에만 붙는 상단·하단 컨트롤(예: 레이어 필터 필드)은 `NSSplitViewItemAccessoryViewController`로 붙여 safe area와 scroll edge effect를 시스템이 처리하게 합니다. scroll edge effect 스타일 지정 API는 macOS 26.1부터이므로, 지원 하한이 macOS 26.0인 동안은 가용성 분기를 둡니다. [AppKit 문서 NSSplitViewItemAccessoryViewController, NSScrollEdgeEffectStyle] [ADR-0006]

### 2.4 하단·보조 패널

- 하단 상태 막대에는 핵심 정보나 동작을 두지 않습니다. 툴바와 메뉴로 접근할 수 없는 기능을 상태 막대에만 두지 않습니다. [HIG Windows, HIG Toolbars]
- 캔버스 아래 분할 영역은 향후 내장 터미널 같은 로드맵 기능의 자리로만 고려하며, 초기 릴리스에는 노출하지 않습니다. [아키텍처 §32] [계획 XVIII.4.3, XVI.11.8]
- 색상·폰트·에셋처럼 떠 있는 보조 도구는 `NSPanel`로 만듭니다. 앱이 비활성화되면 숨고, 최소화 버튼을 두지 않으며, 메뉴에는 "panel"이라는 단어 없이 제목으로 표기합니다. HUD 스타일은 표준 panel이 핵심 콘텐츠를 가릴 때만 씁니다. [HIG Panels] [AppKit 문서 NSPanel]

### 2.5 다중 창

- 프로젝트 창 하나는 `ProjectWindowSession` 하나에 대응하고, `NSWindowController`가 그 session handle을 소유합니다. [아키텍처 §22.2] [계획 XII.2.2]
- 같은 프로젝트를 여러 창에서 여는 기능은 File 메뉴와 문맥 메뉴의 "새 창에서 열기" 같은 명시적 명령으로 제공합니다. 기본 동작으로 새 창을 남발하지 않습니다. [HIG Windows] [계획 XII.2.4]
- 각 창은 독립된 열린 탭, active view, viewport, 선택 상태, Undo/Redo 이력을 가집니다. 메뉴 명령은 활성 창의 session에 연결합니다. [아키텍처 §22.2, §23] [계획 XII.2.5, XII.6.4]
- Window 메뉴에는 Minimize, Zoom과 열린 창 목록을 두고, `NSPanel`은 목록에서 뺍니다. [HIG The menu bar]
- 같은 프로젝트를 연 다른 창과 저장이 충돌하면 시각적 diff 없이 단순 네이티브 alert(`NSAlert`, 해당 창의 sheet)로 두 선택지만 보여 줍니다: 현재 창 버전으로 저장, 디스크 최신 버전 불러오기. 자동 저장 중 감지되면 자동 저장을 멈추고 이 alert를 한 번만 띄웁니다. [아키텍처 §22.2, §27.3] [계획 XII.7.3, XII.7.7] [HIG Alerts]
- 자동 저장이 기본이며 명시적 저장을 강요하지 않습니다. `Save Now`는 메뉴 명령으로 둡니다. [아키텍처 §22.1] [계획 XII.6.4] [HIG File management]
- 폴더·Git 저장소가 원본이므로 `NSDocument` 구조를 채택할지는 별도 결정입니다(열린 결정 O3). [아키텍처 §3.3, §19]

### 2.6 창 상태 저장과 복원

- 창 프레임은 AppKit의 frame autosave(`NSWindow.setFrameAutosaveName(_:)`)와 시스템 창 상태 복원(`NSWindow.isRestorable`, `NSWindowRestoration`, `encodeRestorableState(with:)`)으로 저장·복원합니다. 셸이 프레임을 직접 저장하거나 화면 배치를 직접 검사하지 않습니다. [AppKit 문서 NSWindow, NSWindowRestoration] [HIG Windows] [1.2절 원칙 1]
- pane 폭과 접힘 상태는 split view 자동 저장(`NSSplitView.autosaveName`)으로 유지합니다. [AppKit 문서 NSSplitView]
- 창 상태는 문서 의미가 아니므로 프로젝트 파일과 Git에 넣지 않습니다. [아키텍처 §8.1 "플랫폼 셸은 문서 의미나 파일 포맷을 소유하지 않습니다"]
- 창별 열린 탭, active view, viewport, 선택은 위 창 상태와 별개로 WindowViewState가 다룹니다. [계획 XII.2.5] 창 프레임·pane 상태 복원 자체는 WBS 없음(10.4).

---

## 3. 재질과 깊이

### 3.1 계층 구분

| 계층 | 해당 요소 | 재질 |
|---|---|---|
| 컨트롤·내비게이션 | 툴바, 사이드바·인스펙터 pane, 캔버스 위에 떠 있는 편집 컨트롤·도구 팔레트·HUD | macOS 26 Liquid Glass(표준 구성요소가 자동 적용) |
| 콘텐츠 | 캔버스, 레이어 행, 속성 값, 커밋 메시지, Diff 내용 | Liquid Glass를 쓰지 않음 |

- Liquid Glass는 컨트롤·내비게이션 계층에만 씁니다. 콘텐츠 계층에는 쓰지 않습니다. 디자인 도구에서는 캔버스와 속성 값이 곧 콘텐츠입니다. [HIG Materials] [아키텍처 §8.1]
- 예외는 슬라이더·토글처럼 조작하는 동안만 잠깐 glass 표현으로 바뀌는 시스템 컨트롤뿐이며, 이것은 시스템 동작에 맡깁니다. [HIG Materials]

### 3.2 표준 구성요소에 맡기는 재질

- 툴바와 split view의 사이드바·인스펙터 항목은 macOS 26에서 시스템이 Liquid Glass를 적용합니다. 셸은 이 위에 자체 배경을 깔거나 컨트롤을 임의로 틴트하지 않습니다. [AppKit 문서 NSToolbar, NSSplitViewItem] [HIG Materials]
- 사이드바 안에 레거시 `NSVisualEffectView` 사이드바 재질을 넣지 않습니다. 시스템 glass를 가리기 때문입니다. 지원 하한이 macOS 26이므로 이전 OS용 대체 분기는 두지 않습니다. [AppKit 문서 NSVisualEffectView, NSSplitViewItem] [ADR-0006]
- 콘텐츠 계층 안에서 구조 구분이 필요하면 표준 `NSVisualEffectView` 재질과 블렌딩 모드를 의미에 맞게 고르고, 그 위에 시스템 색을 씁니다. [HIG Materials] [AppKit 문서 NSVisualEffectView.Material]
- 인스펙터 pane의 재질은 시스템이 정하고, 그 안의 입력 필드는 표준 텍스트 컨트롤의 자체 배경을 유지합니다. 입력 영역에 반투명 배경을 직접 추가하지 않습니다. [HIG Materials] [아키텍처 §11.2]
- glass 아래로 콘텐츠가 스크롤되는 곳에는 구분선이나 불투명 배경을 직접 깔지 않고 시스템 scroll edge effect를 씁니다. 한 뷰에는 효과를 하나만 적용합니다. [AppKit 문서 NSScrollEdgeEffectStyle] [HIG Materials]

### 3.3 직접 쓰는 glass

- 캔버스 위에 떠 있는 편집 바·도구 팔레트처럼 최상위 기능 요소에만 `NSGlassEffectView`를 씁니다. 대상 뷰는 `contentView`로 넣고 형제 뷰로 뒤에 깔지 않습니다. [AppKit 문서 NSGlassEffectView]
- 인접한 glass가 여러 개면 `NSGlassEffectContainerView`로 묶어 시각적 불일치를 막고 렌더 패스를 줄입니다. glass 버튼은 `NSButton.BezelStyle.glass`를 씁니다. [AppKit 문서 NSGlassEffectContainerView, NSButton.BezelStyle]
- glass 위 텍스트와 아이콘은 시스템 label 색을 쓰고, 대비 요구는 4.2절을 따릅니다. [HIG Materials, HIG Color]

### 3.4 캔버스와 재질

- 셸의 재질·theme은 캔버스 문서 렌더링에 적용하지 않습니다. 캔버스 픽셀은 코어 렌더러만 그립니다. [아키텍처 §10.2] [계획 XII.9.1, XII.4.1]
- 캔버스 위 native glass 뷰는 Metal 레이어 위에 합성되는 셸 계층이며 문서 픽셀이 아닙니다. 합성 비용 때문에 캔버스 위에 상시 표시하는 glass 요소 수를 최소화합니다. [아키텍처 §6.2] [AppKit 문서 NSGlassEffectContainerView]
- 선택 overlay와 핸들은 native 뷰가 아니라 transient scene layer로 렌더링하며 프로젝트 데이터에 넣지 않습니다. [계획 XII.4.4]

### 3.5 모서리 형태

- 창 안쪽의 둥근 컨테이너는 macOS 26의 동심(concentric) 모서리 지침을 따릅니다. 값은 구현 작업에서 정합니다. [HIG Layout] [0.2절]

---

## 4. 타이포그래피·색·간격

### 4.1 타이포그래피

- 앱 UI 텍스트는 시스템 글꼴을 쓰고, UI용 글꼴 파일을 앱에 내장하지 않습니다. 디자인 결과 텍스트의 폰트 소스(§11.3)와는 무관합니다. [HIG Typography] [아키텍처 §11.2, §11.3]
- 크기는 pt 값이 아니라 macOS 텍스트 스타일(body, callout, caption, headline 등)과 `NSFont`의 시스템 글꼴 메서드(control content, label, menu, tool tip 등)로 지정합니다. macOS에는 Dynamic Type이 없으므로 시스템 기본 크기를 따릅니다. [HIG Typography] [AppKit 문서 NSFont]
- 위계는 크기보다 label 계층(primary, secondary, tertiary)과 굵기로 표현합니다. 아주 가는 굵기는 쓰지 않습니다. [HIG Typography]
- 커밋 해시, 경로, 표현식처럼 정렬이 필요한 값만 시스템 고정폭 글꼴을 씁니다. 수치 입력 필드에 고정폭 숫자를 쓸지는 열린 결정 O7입니다. [HIG Typography] [AppKit 문서 NSFont]

### 4.2 색

- 앱 UI 색은 `NSColor` 의미 색(label 계층, separator, control·window·text background, selected content background, unemphasized selected content background, keyboard focus indicator 등)을 용도대로만 씁니다. 의미를 바꿔 쓰지 않습니다(예: separator 색을 텍스트에 쓰지 않음). [HIG Color] [AppKit 문서 NSColor]
- 강조 색은 사용자가 시스템 설정에서 고른 강조 색(`controlAccentColor`)을 씁니다. 앱 고유 강조 색은 두지 않으며, 두려면 ADR로 정합니다. [HIG Color] [PO 2026-11-14]
- 하드코딩한 RGB 값은 앱 크롬에 쓰지 않습니다. 고정 색 값은 사용자 디자인 콘텐츠의 색(§10.3)에만 존재합니다. 불가피한 커스텀 색은 asset catalog에 light, dark, 고대비 변형을 모두 정의합니다. [HIG Color] [아키텍처 §10.3]
- 사용자 디자인 콘텐츠가 화면의 주인공이므로 툴바와 glass 위에는 색을 아껴 씁니다. [HIG Color, HIG Materials]
- 이 문서에서 "강조"는 prominent 스타일 버튼·항목(5.1절)만 뜻합니다. 한 창의 한 문맥(툴바, sheet, dialog 하나)에는 prominent 항목을 하나만 둡니다. 시스템이 강조 색으로 그리는 선택 하이라이트, 포커스 링, 사이드바 아이콘은 여기에 들어가지 않습니다. [HIG Buttons, HIG Color]
- 선택 상태를 색에만 의존해 표시하지 않고 모양이나 레이블을 함께 씁니다. [HIG Accessibility]
- 앱 UI는 모든 외형과 대비 설정에서 HIG 최소 대비를 지킵니다. 검증은 XII.9.4가 합니다. [HIG Accessibility, HIG Color] [계획 XII.9.4]

### 4.3 외형과 동적 색

- 기본은 시스템 외형(Light, Dark, 자동)을 따릅니다. 앱 전용 외형 강제 설정은 초기 범위에 두지 않습니다(열린 결정 O5). 둔다면 `NSAppearance`를 앱 또는 창 단위로 지정하는 방식만 씁니다. [HIG Dark Mode] [AppKit 문서 NSAppearance]
- 모든 커스텀 뷰는 `effectiveAppearance` 변화에 반응합니다. glass 툴바는 아래 콘텐츠 밝기에 따라 외형을 바꿀 수 있고 이 변화도 `NSAppearance`로 전달되기 때문입니다. [AppKit 문서 NSAppearance] [HIG Materials]
- 렌더러가 그리는 문서 바깥 표면(캔버스 바깥 작업 영역 배경, 선택 표시 등)의 색을 ShellTheme가 실어 나를지, 싣는다면 어떤 색인지는 열린 결정 O9입니다. 계획 XII.9.3의 ShellTheme 정의(dark/light, high contrast, reduced motion, accent change 정규화)에는 색 값 전달이 없습니다. [계획 XII.9.3] [0.2절]
  - 후보: 싣는다면 ShellTheme가 외형, 강조 색, 접근성 표시 옵션이 바뀔 때마다 이 색들을 새 값으로 다시 전달합니다. [계획 XII.9.3] [아키텍처 §9.3]
  - 후보: 캔버스 바깥 작업 영역 배경은 의미상 `underPageBackgroundColor`("문서 콘텐츠 뒤의 배경")가 맞습니다. [AppKit 문서 NSColor]
- 앱 외형 전환은 문서 색을 바꾸지 않습니다. ShellTheme 이벤트는 앱 UI와 캔버스 EvaluationEnvironment에 따로 전달하며, 문서 결과물은 Target Profile 기준으로 평가합니다. [아키텍처 §10.2, §17] [계획 XII.9.3]

### 4.4 간격과 컨트롤 크기

- 표준 컨트롤 크기(`controlSize`)를 우선 쓰고, 컨트롤 높이와 행 높이를 하드코딩하지 않고 Auto Layout으로 배치합니다. [HIG Layout] [AppKit 문서 NSControl.ControlSize]
- 밀도가 높은 인스펙터와 popover에는 small·regular 크기의 둥근 사각형 컨트롤을 씁니다. 기존 밀도 유지가 필요한 영역에 `prefersCompactControlSizeMetrics`를 쓸지는 인스펙터 구현 작업에서 측정해 정합니다. [HIG Layout] [AppKit 문서 NSView.prefersCompactControlSizeMetrics]
- 여백과 간격은 작은 수의 공통 단위로만 구성합니다. 패널 바깥 여백, 패널 안 행 간격, 컨트롤 묶음 간격을 각각 하나의 상수로 정의해 모든 패널이 공유합니다. 값은 구현 작업에서 HIG Layout 기준으로 정하고 12장에 기록합니다. [HIG Layout] [0.2절]
- 클릭 대상 크기는 HIG의 권장·최소 컨트롤 크기를 지킵니다. [HIG Accessibility]

---

## 5. 컨트롤

### 5.1 네이티브 컨트롤 우선

- 버튼, 팝업, 세그먼트, 슬라이더, 체크박스, 목록, 아웃라인은 AppKit 표준 컨트롤을 씁니다. 같은 역할의 컨트롤을 직접 그려 모사하지 않습니다. [계획 XII.9.1] [HIG Buttons, HIG Lists and tables]
- 핵심 동작 하나만 prominent 스타일로 강조합니다(4.2절 "강조"). 기본 버튼은 Return 키 등가를 지정해 시스템이 강조하게 합니다. [HIG Buttons] [AppKit 문서 NSButton]
- 파괴적 동작 버튼은 `hasDestructiveAction = true`로 표시하고, 기본(Return) 버튼으로 만들지 않습니다. [HIG Buttons] [AppKit 문서 NSButton.hasDestructiveAction]
- `tintProminence`의 secondary는 기본 동작이 아닌 대안 동작에만 씁니다. 파괴적 동작 표시 수단으로 쓰지 않습니다. [HIG Buttons] [AppKit 문서 NSButton, NSTintProminence]
- 0이나 100% 같은 기준점이 있는 값 슬라이더(불투명도, 배율 등)는 `NSSlider.neutralValue`로 채움 기준점을 지정합니다. [AppKit 문서 NSSlider]
- 다이얼로그는 `NSAlert`과 창 sheet를 우선 씁니다. [HIG Alerts, HIG Sheets]

### 5.2 텍스트 입력

- 레이어·페이지·문서 이름, 검색, 속성 입력, 토큰·브랜치·커밋 이름, 설정·다이얼로그, 커밋 메시지는 AppKit 네이티브 텍스트 컨트롤(`NSTextField`, AppKit 검색 필드, `NSTextView`)을 씁니다. IME, 맞춤법, 텍스트 서비스, 네이티브 Undo가 시스템과 같게 동작합니다. [아키텍처 §11.2] [계획 XII.5.1(완료)]
- 캔버스 Text 노드 편집은 이와 다릅니다. `NSTextInputClient` adapter가 marked text, 선택 범위, 후보창 위치만 공통 editing session에 연결하고, 표시는 코어 텍스트 엔진이 합니다. 캔버스 텍스트를 네이티브 overlay 텍스트 컨트롤로 그리지 않습니다. [아키텍처 §11.1] [계획 XII.5.3~XII.5.6]

### 5.3 AppKit과 SwiftUI의 경계

- AppKit이 소유: 창, 툴바, 분할 뷰, 메뉴, 캔버스(Metal 뷰), 텍스트 입력, 레이어 트리처럼 고빈도로 갱신되는 목록. [아키텍처 §8.1 "AppKit 중심"]
- SwiftUI 허용: 갱신 빈도가 낮은 인스펙터 섹션 일부, 설정, sheet, 빈 상태. `NSHostingController`/`NSHostingView`로 삽입합니다. [아키텍처 §8.1 "필요한 영역에 SwiftUI"] [AppKit 문서 NSHostingView] [계획 II.5.1]

### 5.4 아이콘

- 아이콘은 SF Symbols를 씁니다. 툴바 아이콘은 테두리 없이 두고, 같은 glass 묶음 안에서 텍스트 항목과 아이콘 항목을 섞지 않습니다. [HIG SF Symbols, HIG Toolbars]
- 아이콘만 있는 모든 컨트롤에 접근성 레이블을 지정합니다(8.3절). [HIG Accessibility] [계획 XII.8.1]
- 앱 아이콘과 그래픽 리소스는 이 문서가 정하지 않습니다. [AGENTS.md 변경 정책]

### 5.5 설정 창

- 설정 창은 툴바로 pane을 전환하고, 변경은 즉시 반영합니다. 앱 메뉴의 Settings… 항목과 Command-Comma로 엽니다. [HIG Settings, HIG The menu bar]
- 현재 SwiftUI 설정 placeholder(II.5.1)를 이 구조로 바꾸는 전용 WBS는 아직 없습니다(10장). [계획 II.5.1]

---

## 6. 영역별 배치 원칙

화면 영역의 위치와 경계만 정합니다. 세부 UX는 각 WBS가 정합니다. [0.2절]

### 6.1 툴바 명령

- **앞쪽 끝**: 사이드바 토글, 창 제목. 사용자 커스터마이즈 대상에서 뺍니다. [HIG Toolbars]
- **가운데**: 자주 쓰는 도구 전환과 앱 명령. 사용자가 커스터마이즈할 수 있게 하고, 창이 좁아지면 overflow 메뉴로 접히게 합니다. [HIG Toolbars]
  - Select, Pan, Zoom 도구의 상호작용 상태 기계는 XII.3.6이 구현합니다. [계획 XII.3.6]
  - Frame, Shape, Text 생성 도구는 WBS 없음(10.4).
- **뒤쪽 끝**: 인스펙터 토글, 프로젝트 전체 검색(검색 툴바 항목), 더 보기 메뉴. 대표 동작은 하나만 둡니다. [HIG Toolbars, HIG Search fields]
- 사이드바·인스펙터 토글과 두 pane 경계에 맞춘 항목 정렬은 시스템 툴바 항목 식별자 `.toggleSidebar`, `.sidebarTrackingSeparator`, `.toggleInspector`, `.inspectorTrackingSeparator`로 구성합니다. 항목 위치를 직접 계산하지 않습니다. [AppKit 문서 NSToolbarItem.Identifier] [1.2절 원칙 1]
- glass 묶음은 `NSToolbarItemGroup`과 spacer로 조절합니다. 저장 상태처럼 누를 수 없는 정보 항목은 테두리를 빼고(`isBordered = false`), 알림 표시는 `NSItemBadge`, 항목 숨김은 뷰가 아니라 `NSToolbarItem.isHidden`으로 합니다. [AppKit 문서 NSToolbarItem, NSToolbarItemGroup, NSItemBadge]
- 툴바의 모든 명령은 메뉴 막대에도 있어야 합니다. 툴바는 사용자가 숨기거나 바꿀 수 있기 때문입니다. [HIG Toolbars, HIG The menu bar]
- 툴바·메뉴 명령의 command ID는 코어와 공유하고, 현재 도구 상태는 접근성으로 노출합니다. [계획 XII.6.5, XII.8.1]
- Branch 전환, Pull, Fetch 같은 로드맵 Git 동작은 툴바에 두지 않습니다. [아키텍처 §24.4] [계획 XIV.13.7]

### 6.2 사이드바(문서·페이지·레이어)

- 내용은 문서·페이지·View 목록과 레이어 트리입니다. 레이어 트리는 Frame, Fragment, Shape, Path, Text, Image, ComponentInstance 계층을 보여 줍니다. [아키텍처 §12] [계획 VI.1.1]
- HIG Sidebars의 "두 단계 이내" 권장은 사이드바 최상위 구획(문서, 페이지, View)에만 적용합니다. [HIG Sidebars]
- 레이어 트리는 이 권장의 예외입니다. 레이어 트리는 내비게이션 계층이 아니라 문서 노드 계층 자체이고, 문서의 중첩 깊이에는 상한이 없기 때문입니다. 레이어 트리는 아웃라인 뷰(`NSOutlineView`)의 disclosure로 깊이 제한 없이 표현합니다. [HIG Outline views] [아키텍처 §12] [AppKit 문서 NSOutlineView]
- 레이어 트리는 시스템 아웃라인 뷰 동작을 따릅니다: 화살표 키 이동·펼침, Return으로 이름 변경, Escape로 취소, 다중 선택, 문맥 메뉴, 드래그 재정렬. [HIG Outline views, HIG Lists and tables]
- 인라인 이름 변경은 포커스를 잃으면 확정하고 Escape로 취소하며, 값이 바뀌지 않았으면 Command를 만들지 않습니다. 이름 변경과 재정렬은 일반 Command로 적용해 창별 Undo에 들어갑니다. [아키텍처 §23] [계획 VII.2]
- 사이드바 목록 선택과 캔버스 선택은 같은 창별 선택 상태 하나를 공유합니다. [계획 VII.3, XII.2.5]
- 레이어 필터 필드는 사이드바 상단 accessory로 둡니다(2.3절). [AppKit 문서 NSSplitViewItemAccessoryViewController] [HIG Search fields]
- 사이드바 아래쪽에는 핵심 동작을 두지 않습니다. 사이드바 아이콘은 사용자가 고른 강조 색을 따릅니다. [HIG Sidebars]
- 대량 레이어 목록은 페이지 단위로 받고 화면에 보이는 행만 그립니다. [아키텍처 §6.3, §6.4, §8.2]

### 6.3 인스펙터

- 내용은 선택 대상의 속성, Value Binding, Component Property·Slot, View Variant, 접근성 의미입니다. [아키텍처 §14.2, §15, §16, §18]
- 모든 속성은 하나의 Value Binding UI로 편집하고, 최종 값이 어떤 토큰·조건·Property·Provider를 거쳤는지 출처 경로를 보여 줄 수 있어야 합니다. [아키텍처 §14.2]
- ComponentInstance 편집은 공개 Property, 허용된 Override, 공개 Slot으로 제한하고, 자유 편집은 명시적 Detach로만 엽니다. [아키텍처 §15.1] [계획 IX.2.6]
- 디자인 문서의 접근성 의미 트리(role, label, value, state, 읽기 순서)를 읽는 preview를 둡니다. [아키텍처 §18] [계획 XII.8.3]
- 섹션은 disclosure로 접고 펼 수 있게 하고, 입력은 AppKit 텍스트 컨트롤과 표준 컨트롤을 씁니다(5장). 연속 조정의 Undo 단위는 §23.3 그룹화 규칙을 따릅니다. [HIG Disclosure controls] [아키텍처 §11.2, §23.3]
- 인스펙터 영역에는 향후 에이전트 패널을 위한 탭 자리를 미리 고려합니다. 초기 릴리스에는 탭을 노출하지 않습니다. [ADR-0005 Migration 5] [계획 XVI.11.8]
- 선택이 바뀌면 보이는 섹션만 갱신하고, 연속 상태 변경은 다음 런 루프에서 한 번 반영합니다(9장). [아키텍처 §6.3]

### 6.4 Git 패널

- 초기 범위는 Status·Semantic Diff, 객체·속성 단위 Stage·Unstage, Commit, Push, Semantic Conflict 해결뿐입니다. 강제 Push는 없고, Branch·Pull·Fetch·Merge·Rebase·Stash·Worktree·Remote 관리 UI는 초기 빌드에 넣지 않습니다. [아키텍처 §24.4, §25.3] [계획 XIV.13.5, XIV.13.7]
- 새 창 계층을 만들지 않고 메인 창 분할 구조 안의 구획으로 둡니다. 사이드바의 전환 가능한 구획으로 둘지 인스펙터 탭으로 둘지는 XIV.13.5 구현 작업에서 정합니다(열린 결정 O4). [HIG Sidebars, HIG Windows] [계획 XIV.13.5]
- Conflict 해결은 Base·Ours·Theirs와 결과를 같은 Conflict Graph로 보여 주고 캔버스와 Property 단위 선택을 지원합니다. 따라서 해결 화면은 캔버스 영역을 함께 씁니다. [아키텍처 §27.2]
- 커밋 메시지는 `NSTextView`로 입력합니다. [아키텍처 §11.2]
- 신뢰 전 제한 모드에서는 Git 동작이 비활성화되므로, 패널은 그 이유와 신뢰 부여 동작을 함께 보여 줍니다. [아키텍처 §24.3] [계획 XVI.7.2]
- Git UI를 열기 전 저장 대기 중인 변경을 flush할지는 코어가 반환한 상태로 사용자에게 묻습니다. [계획 XIV.2.8]
- 변경 목록은 페이지 단위로 받고, stage·commit·push의 진행과 결과는 이벤트 큐로 받습니다. Git 작업은 메인 스레드에서 실행하지 않습니다. [아키텍처 §6.4] [계획 XIV.13.1, XIV.13.2]

### 6.5 에이전트 패널(로드맵)

- ADR-0005는 Proposed 상태이고, 아키텍처 §31.1 본문(내장 AI 없음)과 아직 맞춰지지 않았습니다. 이 절은 자리와 정책 경계만 적으며, 초기 릴리스에는 노출하지 않습니다. [ADR-0005] [계획 XVI.11.8]
- 인스펙터 영역 탭을 후보 자리로 고려합니다. 최종 배치는 해당 WBS가 정합니다. 패널은 세션, 입력, 승인, 미리보기를 제공합니다. [ADR-0005 Context, 결정 4.5 "화면 배치는 WBS가 정합니다", Migration 5]
- 승인 화면은 네이티브 chrome(패널, 창 sheet, dialog)에만 두고 캔버스나 WebView 안에는 그리지 않습니다. 주 내용은 코어가 만든 semantic diff, validation 결과, 렌더 프리뷰, 잠금 범위이고, 에이전트가 붙인 설명은 출처를 표시한 보조 정보로만 둡니다. [ADR-0005 결정 4.5]
- 민감한 확인(GitCommit, GitPush 등)은 파괴적 동작 확인 패턴을 따르고 키보드 기본 동작만으로 승인되지 않게 합니다. 보조 기술로는 확정할 수 있어야 합니다. [ADR-0005 결정 4.5] [HIG Alerts]
- 모드 전환, 현재 세션 중지, 프로젝트의 에이전트 lease 모두 해제를 제공하고, 활성 세션이 있으면 창 닫기와 앱 종료 전에 확인합니다. [ADR-0005 결정 4.5]
- 대화 입력은 `NSTextView`를 쓰고, CLI 출력은 텍스트로만 표시합니다. [아키텍처 §11.2] [ADR-0005 결정 4.4]
- 세부 UX는 ADR-0005가 Accepted된 뒤 추가되는 WBS가 정합니다. [ADR-0005 Migration]

---

## 7. 빈 상태와 첫 실행

### 7.1 빈 상태

- 빈 상태에는 다음에 할 일과 그 동작을 실행하는 버튼을 함께 둡니다. 빈 상태는 일시적이므로 필수 정보를 두지 않습니다. 감탄사 없이 명확하게 쓰고, 오류는 문제 위치 가까이에 해결 방법과 함께 보여 줍니다. [HIG Writing]
- 대상:
  - 열린 프로젝트가 없음: New Project, Open Project, Clone Project. [아키텍처 §25] [계획 XII.6.1~XII.6.3]
  - 빈 페이지 캔버스, 빈 레이어 목록: 페이지와 노드 계층이 비어 있는 상태입니다. [아키텍처 §12] [HIG Writing]
  - 레이어 필터 결과 없음: 6.2절의 필터 필드가 결과를 찾지 못한 상태입니다. [HIG Search fields, HIG Writing]
  - Git 변경 없음: 초기 Git UI의 Status에 변경이 없는 상태입니다. [아키텍처 §24.4, §26] [계획 XIV.13.5]
- 빈 상태 화면은 SwiftUI로 만들 수 있습니다(5.3절). [아키텍처 §8.1]

### 7.2 프로젝트 흐름 안내

- `stellar.yaml`이 없는 Git 저장소는 전환하지 않고 안내만 합니다. [아키텍처 §25.2] [계획 XII.6.2]
- 시스템 Git이 없으면 설치 안내만 하고 자동 설치하지 않습니다. [아키텍처 §24.1]
- Git `user.name`·`user.email`이 없으면 최초 커밋 전에 설정을 요청합니다. [아키텍처 §25.1]
- 처음 연 프로젝트는 제한 모드로 열립니다. 제한 모드임을 창에서 알 수 있게 하고, 신뢰를 부여·철회하는 네이티브 UI를 제공합니다. [아키텍처 §24.3] [계획 XVI.7.2]
- 진단 전송은 사용자가 명시적으로 동의할 때만 하며, 동의 질문과 철회 경로는 설정에서도 접근할 수 있게 합니다. [아키텍처 §39]

### 7.3 `stellar` 명령 PATH 노출

- PATH 노출 방법은 ADR-0005 Open Questions 7이고, 같은 ADR 결정 7은 초기 릴리스에 보조 실행 파일과 alias를 넣지 않으므로, ADR-0005가 Accepted되기 전에는 이 화면을 정하지 않습니다. [ADR-0005 결정 7, Open Questions 7]

---

## 8. 키보드·접근성

### 8.1 메뉴와 단축키

- 모든 명령은 메인 메뉴 항목으로 존재합니다. 메뉴 순서는 앱 메뉴(Stellar), File, Edit, Format, View, 앱 고유 메뉴(예: 객체·배치), Window, Help입니다. 앱 고유 메뉴의 구성은 열린 결정 O8입니다. [HIG The menu bar] [계획 XII.6.5]
- 지금 쓸 수 없는 항목은 숨기지 않고 비활성화합니다. 활성화 여부는 `NSMenuItemValidation`으로 현재 first responder와 선택 상태에서 판단합니다. [HIG The menu bar] [AppKit 문서 NSMenuItemValidation]
- 추가 입력이 필요한 명령에는 말줄임표(…)를 붙이고, 메뉴 제목은 title-style 대문자 규칙을 따릅니다. macOS 26 메뉴 아이콘은 표준 selector에서 시스템이 붙이는 것을 쓰고, 한 그룹 안에서는 모두 넣거나 모두 뺍니다. [HIG The menu bar]
- 표준 단축키(실행 취소·다시 실행, 잘라내기·복사·붙여넣기·전체 선택, 저장, 새로 만들기·열기·닫기, 찾기, 인스펙터 표시, 전체 화면, 최소화)는 원래 의미를 유지합니다. Undo, Redo, Save Now는 활성 창 session에 연결합니다. [HIG Keyboard] [계획 XII.6.4, XII.6.5]
- 커스텀 단축키는 자주 쓰는 앱 고유 명령에만 두고 Command를 주 수식키, Shift를 보조로 씁니다. Option은 고급 기능에만 쓰고 Control은 피합니다. [HIG Keyboard]
- 메뉴 막대·툴바·창 순환 같은 시스템 키보드 탐색 단축키를 가로채지 않습니다. [HIG Keyboard]
- 캔버스 키 조작(화살표 키 이동 단위, Shift-화살표 이동 단위, 드래그 중 수식키 의미)은 HIG가 정하지 않는 디자인 도구 고유 규칙이므로 열린 결정 O10으로 둡니다. 화살표 키 이동과 드래그 수식키는 서로 다른 규칙으로 정합니다. [계획 XII.3.6] [0.2절]
- 아이콘만 있는 컨트롤에는 도움말 태그(`toolTip`)와 접근성 레이블을 둡니다. [HIG Offering help, HIG Accessibility] [AppKit 문서 NSView.toolTip]

### 8.2 포커스

- 키 입력은 응답자 체인을 따릅니다. 캔버스, 레이어 목록, 인스펙터 사이를 Tab·Shift-Tab과 시스템 키보드 탐색 설정에 맞춰 이동할 수 있게 하고, Full Keyboard Access로 모든 컨트롤에 도달할 수 있게 합니다. [HIG Focus and selection, HIG Keyboard] [AppKit 문서 NSResponder] [계획 XII.8.1]
- 텍스트·검색 필드에는 시스템 포커스 링을 쓰고 숨기지 않습니다. 목록은 포커스 링 대신 행 전체 하이라이트를 씁니다(키 창은 강조 색, 비활성 창은 회색 계열 의미 색). [HIG Focus and selection] [AppKit 문서 NSColor]
- 캔버스 뷰는 `acceptsFirstResponder`로 키 입력을 받습니다. 텍스트 편집 중 단축키 전달은 응답자 체인으로 정합니다. [AppKit 문서 NSResponder]
  - 표준 편집 메뉴 항목(잘라내기, 복사, 붙여넣기, 전체 선택, 실행 취소)은 target 없는(first responder) action으로 연결합니다. 텍스트 필드 편집 중에는 창의 field editor(`NSWindow.fieldEditor(_:for:)`)가 first responder이므로 이 action을 먼저 받습니다. [AppKit 문서 NSResponder, NSWindow, NSMenuItem]
  - 수식키 없는 한 글자 도구 단축키는 메뉴 key equivalent로 두지 않고 캔버스 뷰의 `keyDown(with:)`에서 처리합니다. 따라서 텍스트 컨트롤이 first responder인 동안에는 발동하지 않습니다. 캔버스 Text 노드 편집 중에는 캔버스 뷰가 키를 먼저 `interpretKeyEvents(_:)`로 텍스트 입력 경로에 넘깁니다(5.2절). [AppKit 문서 NSResponder, NSMenuItem.keyEquivalent, NSTextInputClient]
- 사용자 조작 없이 포커스를 옮기지 않습니다. [HIG Focus and selection]

### 8.3 VoiceOver

- 모든 네이티브 컨트롤에 label, role, 키보드 focus 순서를 지정합니다. 캔버스 호스트는 선택 상태와 도구 상태를 접근성으로 노출합니다. [아키텍처 §18] [계획 XII.8.1]
- 아이콘만 있는 컨트롤에는 접근성 레이블을 붙이고, 여러 하위 뷰로 구성된 목록 행은 하나의 요소로 묶어 선택 상태를 노출합니다. [HIG Accessibility] [AppKit 문서 NSAccessibility]
- 제스처로 동작하는 커스텀 요소에는 접근성 기본 동작을 따로 연결해 VoiceOver로도 같은 결과를 얻게 합니다. 시각적으로 숨긴 요소는 접근성 트리에서도 숨깁니다. [HIG Accessibility] [AppKit 문서 NSAccessibility]
- 앱 UI 접근성(네이티브 API)과 디자인 문서의 접근성 의미 트리(§18 데이터 모델)는 서로 다른 대상입니다. 후자는 인스펙터 preview로만 읽습니다. [아키텍처 §18] [계획 XII.8.3]

### 8.4 시스템 접근성 설정

- `NSWorkspace`의 투명도 줄이기, 대비 높이기, 동작 줄이기 값을 읽고, 접근성 표시 옵션 변경 알림을 구독해 실행 중에도 반영합니다. [AppKit 문서 NSWorkspace] [HIG Accessibility]
- 표준 구성요소는 시스템이 자동으로 맞춥니다. 셸이 직접 대응할 대상은 직접 쓰는 glass, 직접 그리는 반투명 표면, 캔버스 오버레이(선택 핸들, 스냅 가이드), 애니메이션입니다. [HIG Accessibility]
  - **투명도 줄이기**: 직접 그리는 반투명 표면을 불투명 대체 표면으로 바꿉니다. [HIG Accessibility, HIG Materials] [AppKit 문서 NSWorkspace.accessibilityDisplayShouldReduceTransparency]
  - **대비 높이기**: 구분선과 선택 표시를 더 강하게 그립니다. 캔버스 오버레이는 렌더러가 그리므로 ShellTheme 이벤트로 전달합니다. [HIG Accessibility] [AppKit 문서 NSWorkspace.accessibilityDisplayShouldIncreaseContrast] [계획 XII.9.3]
  - **동작 줄이기**: 위치 이동·확대·블러 전환을 즉시 전환이나 짧은 페이드로 바꿉니다. [HIG Motion, HIG Accessibility] [AppKit 문서 NSWorkspace.accessibilityDisplayShouldReduceMotion]
- ShellTheme 이벤트(XII.9.3)의 정규화 대상에는 투명도 줄이기가 명시되어 있지 않습니다. 캔버스 오버레이가 반투명을 쓰면 이 값도 렌더러에 필요하므로, 포함 여부를 XII.9.3 구현 작업에서 정합니다(열린 결정 O6). [계획 XII.9.3]
- 검증은 Light·Dark와 대비 높이기, 투명도 줄이기, 동작 줄이기의 조합으로 합니다. 특히 Dark에서 대비 높이기와 투명도 줄이기를 함께 켠 경우를 포함합니다. [HIG Accessibility] [계획 XII.9.4]

### 8.5 모션

- 셸 애니메이션은 짧고 목적이 있을 때만 씁니다. 상태 변화를 알려 주는 용도로만 쓰고 장식으로 쓰지 않습니다. [HIG Motion]
- 동작 줄이기가 켜져 있으면 8.4절 대체 표현을 씁니다. [HIG Motion, HIG Accessibility]
- 모션 정책의 세부(대상, 방식)는 XII.9.1 구현 작업에서 정합니다. [계획 XII.9.1] [0.2절]
- 캔버스 Pan·Zoom·Drag·선택 표시는 장식 애니메이션 대상이 아니며 입력에 즉시 반응합니다. [아키텍처 §6.4]
- UI 애니메이션은 캔버스 렌더 경로와 문서 스레드를 기다리지 않습니다. [아키텍처 §9.2, §9.3]

---

## 9. 성능

- UI 스레드는 한 번에 2ms를 넘게 점유하지 않습니다. 패널 갱신도 이 예산 안에서 합니다. [아키텍처 §6.3]
- Native UI Thread에서는 이벤트 수신, 네이티브 UI 갱신, IME·접근성 연결만 수행합니다. [아키텍처 §9.2]
- 검색 질의, Git 상태 조회, 저장 직렬화, 외부 CLI 출력 처리, 썸네일 생성은 별도 큐나 프로세스에서 실행하고 결과만 메인 스레드로 전달합니다. 세대 번호가 현재와 다른 결과는 버립니다. [아키텍처 §6.4, §9.1, §9.2] [계획 XII.1.5]
- 코어에서 셸로 가는 진행·결과·승인 이벤트는 pull 방식 이벤트 큐(`stellar_runtime_poll_event`)로 받습니다. 대규모 문서 데이터는 FFI를 반복 왕복하지 않고 Snapshot·Command 단위로 주고받습니다. [ADR-0009] [아키텍처 §8.2]
- 대량 목록(레이어, Git change)은 페이지 단위로 받고 화면에 보이는 부분만 그립니다. [아키텍처 §6.3] [계획 XIV.13.1]
- 고빈도로 바뀌는 커스텀 AppKit 뷰는 변경된 영역만 무효화하고 보이는 범위만 그립니다. 관찰 대상 상태가 연속으로 바뀌면 다음 런 루프에서 한 번만 반영합니다. [AppKit 문서 NSView] [아키텍처 §6.3]
- **캔버스는 코어 렌더러 소유입니다.** 캔버스 픽셀은 Metal backend가 CAMetalLayer에 그립니다. 셸은 drawable 크기·scale과 입력 이벤트만 전달하고, pan·zoom은 camera 입력일 뿐 문서 geometry를 바꾸지 않습니다. [아키텍처 §10] [계획 II.5.4, XII.4.1, XII.4.3]
- 입력 이벤트는 공통 InputEvent로 변환해 native timestamp와 함께 넘기고, 연속 move·wheel은 병합하되 down·up과 key 순서는 보존합니다. [계획 XII.3.1, XII.3.2, XII.3.4]
- 캔버스 위 glass 요소와 패널 애니메이션 중 drawable 크기 변경이 렌더 비용에 주는 영향은 XII.4.1에서 drawable 갱신 정책으로 정하고, XVI.1 계측으로 확인합니다. [ADR-0005 Migration 5] [계획 XII.4.1, XVI.1]

---

## 10. 현재 셸과의 차이와 이행 순서

### 10.1 차이

현재 macOS 셸(II.5.1~II.5.4, XII.5.1 구현 상태)과 이 문서의 목표를 비교합니다.

| 항목 | 현재 | 목표 | 반영 WBS |
|---|---|---|---|
| 창 | 단일 창(초기 960x600, 크기 조절 가능), 제목 'Stellar' 고정, 툴바 없음 | 세션별 창, `NSToolbar`, 제목은 프로젝트·문서, AppKit 창 상태 복원 | XII.2.2, XII.9.1, 창 상태 복원은 전용 WBS 없음 |
| 분할 구조 | `NSSplitView` 2열(필드 패널 \| 캔버스) | `NSSplitViewController` 3열(사이드바 \| 캔버스 \| 인스펙터) | XII.9.1 |
| 사이드바 | 검증용 네이티브 필드 패널(검색, 레이어 이름, 값, 메모) | 문서·페이지·View 목록과 레이어 트리 | 골격은 XII.9.1, 내용은 전용 WBS 없음 |
| 인스펙터 | 없음 | 속성·Value Binding·접근성 preview | 골격은 XII.9.1, 접근성 bridge는 XII.8.3, 내용은 전용 WBS 없음 |
| 캔버스 | 창 배경 색만 칠하고 drawable 크기·scale만 전달 | Metal 렌더링, ShellTheme 전달(색 포함 여부는 O9), transient overlay | XII.4.1, XII.4.3, XII.4.4, XII.9.3 |
| 입력 | 캔버스 입력 전달 없음 | InputEvent 변환·병합, 도구 상태 기계 | XII.3.2, XII.3.4, XII.3.6, 생성 도구는 전용 WBS 없음 |
| 메뉴·명령 | 명시적 메인 메뉴 구성 없음 | 메뉴 막대, 표준 단축키, 활성 창 session 연결 | XII.6.1~XII.6.5 |
| 접근성 | 지정 없음 | label·role·focus 순서, 캔버스 상태 노출 | XII.8.1, XII.9.4 |
| 다중 창 | 단일 창, 앱 runtime handle 하나 | 같은 프로젝트 여러 창, 저장 충돌 alert | XII.2.2, XII.2.4, XII.7.3 |
| 외형·접근성 설정 | 대응 없음 | ShellTheme 정규화와 분리 전달 | XII.9.3 |
| 설정 창 | SwiftUI placeholder, 미연결 | 툴바로 pane 전환, 즉시 반영 | 전용 WBS 없음 |
| Git 패널 | 없음 | 초기 Git UI 범위 | XIV.13.5, XIV.13.7 |
| 빈 상태·첫 실행 | 없음 | 7장 | New·Open·Clone은 XII.6.1~XII.6.3, 신뢰 UI·화면 자체는 전용 WBS 없음 |
| 에이전트 패널 | 없음 | 6.5절(로드맵) | ADR-0005 Accepted 후 추가되는 WBS |

텍스트 입력은 이미 네이티브 텍스트 컨트롤을 씁니다(XII.5.1 완료). 현재 필드 패널은 텍스트 입력 검증 fixture로 쓰이므로, 사이드바로 바꿀 때 `--text-input-check` 검증 경로를 함께 옮깁니다. [계획 XII.5.1]

### 10.2 이행 순서

계획의 선행 관계를 그대로 따릅니다. 선행 ID를 건너뛰지 않습니다. [계획 XVII.2 규칙 9] 선행 작업이 모두 `[x]`가 아니면 코드를 수정하지 않으며, `[!]` 선행이 있는 행은 차단됩니다. [계획 1.1 상태 표기, 1.2 규칙 4]

아래 여러 행은 ADR-0006으로 `[!]`가 된 Windows 행을 선행으로 둡니다. 이 행들은 계획 개정이나 ADR-0006 후속 결정으로 macOS 전용 행의 Windows 선행을 걷어 내기 전에는 착수할 수 없습니다. 이 문서는 계획을 바꾸지 않으므로 차단 사실만 적습니다. [ADR-0006 결정 3] [0.2절]

1. **창·세션**: XII.2.2(창이 session handle 소유) → XII.2.4, XII.2.5(다중 창, 창별 view state).
   - Windows 차단: XII.2.4는 XII.2.3(`[!]`)을 선행으로 둡니다. XII.2.2와 XII.2.5는 Windows 선행이 없습니다.
2. **명령·메뉴**: XII.6.1~XII.6.5. 메뉴 막대와 단축키를 이 문서 8.1절 기준으로 구성합니다.
   - Windows 차단: XII.6.1은 XII.2.3(`[!]`)을 선행으로 둡니다. XII.6.2~XII.6.5는 Windows 선행이 없습니다.
3. **접근성 기초**: XII.8.1(XII.2.2, XII.6 선행).
   - Windows 차단: 선행 `XII.6`을 XII.6 그룹 전체로 읽으면 XII.6.1을 거쳐 XII.2.3(`[!]`)에 걸립니다.
4. **셸 표현**: XII.9.1(XII.2.2, XII.8.1 선행). 창 크롬, 툴바, `NSSplitViewController` 3열, 재질을 이 문서 2~5장 기준으로 바꿉니다.
   - Windows 차단: 직접 선행에는 Windows 행이 없으나, XII.8.1을 거쳐 3번과 같은 차단을 받습니다.
5. **캔버스**: XII.4.1(XI.8.2, XII.2.2 선행), XII.3.2, XII.4.3(XII.2.5, XI.2.2 선행), XII.4.4(XII.4.3 선행). XII.4.1은 1번의 XII.2.2가, XII.4.3은 1번의 XII.2.5가 `[x]`가 된 뒤 시작합니다. XII.3.2는 1~4와 병렬로 진행할 수 있으나 Xcode project를 수정하는 작업은 한 시점에 하나만 진행합니다. [계획 XVII.2 규칙 2]
   - Windows 차단: 이 단계의 macOS 행에는 Windows 선행이 없습니다.
6. **외형·접근성 설정 반영**: XII.9.3, 이어서 XII.9.4(적합성 보고서).
   - Windows 차단: XII.9.3은 XII.9.2(`[!]`)를 선행으로 둡니다. XII.9.4는 XII.9.1~XII.9.3을 선행으로 두므로 XII.9.2에 직접 걸리고, XII.9.3을 거쳐서도 걸립니다.
7. **Git 패널**: XIV.13.5, XIV.13.7.
   - Windows 차단: XIV.13.7은 XIV.13.6(`[!]`)을 선행으로 둡니다. XIV.13.5는 Windows 선행이 없습니다.
8. **로드맵**: 에이전트 패널, 하단 패널 등은 각 ADR·WBS가 확정된 뒤 진행합니다.

### 10.3 적합성 점검

- 각 셸 표현 작업은 먼저 표준 구성요소만으로 macOS 26 SDK 빌드를 확인한 뒤, 커스텀 배경, 하드코딩한 컨트롤 높이, 커스텀 툴바 처리, 하드코딩 RGB가 있는지 감사합니다. [HIG Materials] [계획 XII.9.1, XII.9.4]
- XII.9.4 적합성 보고서에는 native control 사용, 키보드 탐색, 대비, 8.4절 조합 결과를 포함합니다. [계획 XII.9.4]

### 10.4 계획 반영이 필요한 항목

아래는 이 문서가 요구하지만 계획서에 전용 WBS가 없는 항목입니다. 이 문서는 계획서를 수정하지 않으므로, 계획 개정 작업에서 WBS를 추가해야 합니다.

- 사이드바 내용(문서·페이지·View 목록, 레이어 트리 UI)
- 인스펙터 내용(Value Binding UI, 출처 경로 표시, Component·Variant 편집)
- 설정 창
- 빈 상태 화면, 프로젝트 신뢰 상태 표시·부여·철회 UI
- 창 상태(프레임, pane 폭·숨김) 저장·복원(2.6절)
- Frame, Shape, Text 생성 도구(6.1절)
- 가이드·측정 표시(1.2절 원칙 2)
- `stellar` 명령 PATH 노출 화면: ADR-0005 Open Questions 7 결정 대기(7.3절)
- 문서 연결: 이 문서를 `AGENTS.md` "주요 참고 문서"에 추가할지는 사용자 승인을 받아 정합니다. [AGENTS.md 지침 우선순위]

---

## 11. 열린 결정

| ID | 내용 | 정할 곳 |
|---|---|---|
| O1 | 툴바 스타일: unified와 unified compact 중 선택 | XII.9.1 시안 단계 |
| O2 | 캔버스를 사이드바·인스펙터 아래로 확장할지, 배경 확장 뷰로 아트보드를 미러링해도 되는지 | 제품 결정, XII.9.1 |
| O3 | `NSDocument` 구조 채택 여부 | ADR |
| O4 | Git 패널 위치: 사이드바 구획과 인스펙터 탭 중 선택 | XIV.13.5 |
| O5 | 앱 전용 외형 강제 설정 제공 여부 | 제품 결정 |
| O6 | ShellTheme 정규화에 투명도 줄이기를 포함할지 | XII.9.3 |
| O7 | 수치 입력 필드의 고정폭 숫자 사용 | 인스펙터 구현 작업 |
| O8 | 앱 고유 메뉴(객체·배치 등)의 이름과 구성 | XII.6.5 |
| O9 | ShellTheme가 렌더러가 그리는 문서 바깥 표면(캔버스 바깥 작업 영역 배경, 선택 표시 등)의 셸 색을 실어 나를지, 어떤 색인지 | XII.9.3, XII.4.1 |
| O10 | 캔버스 키 조작: 화살표·Shift-화살표 이동 단위, 드래그 중 수식키(비율 고정 등) 의미 | XII.3.6 또는 별도 ADR |

---

## 12. 확정 수치 기록

구현 작업에서 정한 수치를 여기에 기록합니다. 각 행에는 값, 근거(HIG 주제 또는 Stellar 측정), 정한 WBS를 함께 적습니다. 아직 확정한 값은 없습니다.

| 항목 | 값 | 근거 | WBS |
|---|---|---|---|
| (없음) | | | |
