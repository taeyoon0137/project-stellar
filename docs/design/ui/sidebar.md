# 사이드바(페이지·레이어) UI 명세

- 상태: 기본 창의 활성 Page·View 및 문서 레이어 읽기 연결(2026-11-14). Pages 구획을 머리 줄 팝업으로 교체하고 표시·잠금 토글 연결, UX 명세 반영(2026-11-15)
- 코드: `apps/macos/Stellar/UI/Sidebar/`
- 상위 기준: `docs/design/macos-shell-design.md`(이하 "가이드"), `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`(이하 "아키텍처"), `docs/design/ux/stellar-ux-spec.md`(이하 "UX 명세") 7장
- 근거 표기는 가이드 0.1절을 따릅니다. 이 문서는 가이드·아키텍처·구현 계획을 바꾸지 않으며, 어긋나면 그 문서들이 우선입니다. `UX.n.m`은 `docs/design/ux/ux-wbs-rows.md`의 행입니다.

## 0. 목표 배치(UX 명세)

1장 이하는 구현 기록입니다(머리 줄 팝업 UX.3.5, 표시·잠금 토글 UX.3.4, 빈 상태 안내 UX.1.6 반영). 목표 배치는 다음과 같고, 남은 항목은 각 행에서 코드와 이 문서를 맞춥니다. [가이드 6.2] [UX 명세 3.2, 7장]

```text
┌ Page 1 › Home ▾ ─────────────────────────── (머리 줄 팝업) ┐
├ 레이어 · 에셋 · 변경 ───────────── (연결된 탭만, 하나면 탭 줄 없음) ┤
│ 필터 필드                                                  │
│ ▾ Header                     (표시·잠금 토글: hover·선택 시) │
│     Title                                   (상태 배지 자리) │
└────────────────────────────────────────────────────────────┘
```

- 머리 줄: Page·View 전환과 이름 변경을 `Page 1 › Home ▾` 팝업 한 줄로 합니다. 지금의 Pages 구획(사이드바 높이 30%)을 대신합니다(UX.3.5). 새 Page·새 View는 코어 Command가 생긴 뒤(UX.12)에만 넣습니다. 컴포넌트 정의 편집 중에는 `Home › Button 정의 [완료]` 경로가 됩니다. [결정 U22]
- 탭: 레이어(구조), 에셋(재사용 정의, D4, UX.9.2), 변경(Git 패널, UX.5.2). 기능이 연결된 탭만 생깁니다. [결정 U1]
- 레이어 행 끝 상태 배지(Git 변경·충돌·에이전트 출처·잠금)는 가이드 6.6절 공용 사전의 모양을 씁니다(UX.6.2). [결정 U15]
- 빈 상태는 버튼 없이 키 안내 한 줄 "F로 Frame, R로 사각형, T로 텍스트를 그립니다"입니다(UX.1.6). [가이드 7.1]
- 표시·잠금 토글은 코어 `SetProperty::visibility`·`locked`에 연결합니다(UX.3.4). 잠금은 `node.locked`이고 앱·CLI·MCP에 같은 규칙이 적용됩니다. [D8]
- 이름 변경·순서 편집 같은 레이어 편집은 신뢰 여부와 상관없이 됩니다. [D1]

## 1. 보여 주는 것

메인 창 3열 중 왼쪽 사이드바입니다(기준선 구현). [가이드 2.3, 6.2] [HIG Sidebars]

```text
┌ 필터 필드 (split view item 상단 accessory, NSSearchField) ┐
│ Onboarding › Welcome ▾      (머리 줄 팝업, NSPopUpButton)   │  누르면 Page별 View 목록
├ Layers ─────────────────────────────────────────────────────┤
│ ▾ ⬚ Header                              👁 🔓 (hover·선택)   │  깊이 제한 없는 레이어 트리
│     Aa Title                                                │
│   ▭ Background                                  🔒 (잠김)    │
│   ○ Glow (흐린 글자)                         👁̸ (숨김)      │
└─────────────────────────────────────────────────────────────┘
```

- **머리 줄**: 활성 Page와 View를 `Page › View ▾` 한 줄(pull-down `NSPopUpButton`)로 보입니다. 누르면 작업 문서의 Page별 View 목록(Page는 구획 머리 항목)이 열리고, 활성 View에 체크가 있습니다. View를 고르면 그 창의 active View로 엽니다(`.activateView`, 처음 연 View는 root에 맞춤). Page·View 이름 변경과 추가는 코어 Command가 생긴 뒤(UX.12) 같은 메뉴에 붙입니다. 사이드바 높이를 쓰던 Pages 구획(분할 view)은 없앴습니다. [UX 명세 7.1] [가이드 6.2]
- **레이어 구획**: active View의 root Frame 아래 노드 트리. root Frame 자체는 머리 줄의 View가 대표하므로 행으로 두지 않고, 그 자식부터 최상위 행으로 보입니다. 깊이 제한 없이 `NSOutlineView` disclosure로 표현합니다. [가이드 6.2] [HIG Outline views]
- 레이어 목록은 source list(사이드바) 스타일 `NSOutlineView`이고, 구획 제목("Layers")은 접을 수 없는 group row입니다. 사이드바 재질·접기·폭 유지는 `NSSplitViewItem(sidebarWithViewController:)`가 맡습니다. [가이드 2.3, 3.2]
- 머리 줄은 지금 필터 accessory 아래에 있습니다. UX 명세 3.2는 머리 줄 → 탭 → 탭 내용(필터는 레이어 탭 내용) 순서이므로, 사이드바 탭 줄(에셋·변경 탭, UX.5·UX.9)을 넣을 때 머리 줄을 맨 위로 옮기고 필터를 레이어 탭 안으로 내립니다.
- 레이어 행: 종류 아이콘(SF Symbol), 이름(인라인 이름 변경), 표시 토글, 잠금 토글. 행 높이·글꼴·아이콘 크기는 시스템 사이드바 크기 설정(row size style)을 따릅니다. [가이드 4.1, 4.4]
- 문서 목록(프로젝트의 여러 문서)은 이 영역에 넣지 않았습니다. 13절 참고.

## 2. 구성

| 파일 | 역할 |
|---|---|
| `SidebarModel.swift` | 값 snapshot·command·검증 결과 타입, `SidebarDataProvider` 프로토콜 |
| `../Shell/ShellSidebarSource.swift` | 기본 창의 코어 페이지 조회·펼친 행·창별 선택, 머리 줄 Page·View 목록 읽기(문서 revision마다) |
| `SidebarStubDataProvider.swift` | UI 상호작용 검사용 메모리 트리 |
| `SidebarViewController.swift` | 구성, snapshot 반영 병합, 상태 화면 결정, split view item·accessory 연결, `SidebarMetrics` |
| `SidebarPagePopup.swift` | 머리 줄 `Page › View ▾` 팝업과 View 전환 |
| `SidebarLayersController.swift` | 레이어 outline: 데이터·선택·펼침·이름 변경·토글·문맥 메뉴·드래그 |
| `SidebarOutlineView.swift` | `NSOutlineView` 하위 클래스: Return·Delete·단축키, 메뉴 막대용 responder action. 공통 outline 생성(`SidebarOutlineFactory`), `SidebarOutlineItem` |
| `SidebarCellViews.swift` | 구획 제목 셀, 이름 셀, 레이어 셀(토글 포함) |
| `SidebarInlineRename.swift` | 인라인 이름 변경 확정·취소 처리 |
| `SidebarStateView.swift` | 로드 중·실패·View 없음·빈 목록·결과 없음 화면 |
| `SidebarFilterAccessoryViewController.swift` | 필터 필드 accessory(`NSSplitViewItemAccessoryViewController`) |
| `SidebarSymbols.swift` | SF Symbol 이름, UI 문자열(지역화 대상, 개발 지역 en) |
| `SidebarUICheck.swift` | 정적 검사 진입점 `SidebarUICheck.run() -> Bool` |

AppKit만 씁니다. 가이드 5.3은 빈 상태 화면에 SwiftUI를 허용하지만, 목록 위 겹침·클릭 통과 처리를 한 계층에서 다루려고 AppKit으로 만들었습니다. [가이드 5.3] [아키텍처 §8.1]

## 3. 상태

레이어 구획 위에 `SidebarStateView`를 겹쳐 보입니다. 배경을 칠하지 않으며 버튼 외 영역의 클릭은 아래 목록으로 통과합니다. 머리 줄은 상태 화면을 두지 않고, 활성 View가 없으면 숨깁니다. [가이드 7.1] [HIG Writing]

| 조건(위에서부터 우선) | 화면 | 버튼 → command |
|---|---|---|
| `loadState == .loading` | 시스템 진행 표시기 + "Loading Layers…" | 없음 |
| `loadState == .failed(message)` | 경고 아이콘 + "Couldn’t Load Layers" + 사유 | "Try Again" → `.reload` |
| `activeViewID == nil` | "No View Selected" + 안내 | 없음 |
| 필터 켜짐, `matchCount == 0` | "No Results" + `No layers match “질의”.` | "Clear Filter" → 필드 비움 + `.setFilter("")` |
| 필터 꺼짐, 최상위 행 없음 | "No Layers" + 안내 | 없음(13절) |
| 그 외 | 숨김 | — |

- 실패 사유 문자열에는 프로젝트 유래 문자열을 넣지 않는 것이 코어 쪽 계약입니다. [아키텍처 §39]
- 필터 결과가 있으면 일치 행과 그 조상만 보이고 모두 펼칩니다. 직접 일치하지 않은 조상 행은 보조 label 색과 접근성 문구("Contains matches")로 구분합니다.

## 4. 상호작용

### 4.1 선택

- 레이어 구획은 다중 선택이고, 선택은 창별 선택 상태(WindowViewState)와 같습니다. 사이드바 선택 변경은 `.setSelection(ids, primary:)`로 보냅니다. 순서는 선택한 순서이며 primary는 마지막으로 더한 행입니다. [가이드 6.2] [계획 VII.3.1, VII.3.2, XII.2.5]
- snapshot의 선택이 사이드바가 마지막으로 보낸 선택과 다르면 외부(캔버스 등) 변경으로 보고, 조상 행을 펼치고 첫 선택 행이 보이게 스크롤합니다. 사이드바가 보낸 선택에는 스크롤·펼침을 하지 않습니다(사용자 조작 없이 화면을 움직이지 않음). [가이드 8.2]
- 구획 제목 행은 선택되지 않습니다(`selectionIndexesForProposedSelection`에서 제외).
- 머리 줄 팝업에서 View를 고르면 `.activateView`를 보냅니다. 이미 활성인 View는 command를 보내지 않습니다. 목록에는 provider가 실행할 수 있는 항목만 둡니다.

### 4.2 인라인 이름 변경

- 시작: Return, 행 더블클릭, 문맥 메뉴 Rename, VoiceOver custom action.
- 확정: Return 또는 포커스 이동. 취소: Escape(필드 값을 snapshot 값으로 복원, command 없음). 값이 snapshot 이름과 같으면 command를 만들지 않습니다. [가이드 6.2] [아키텍처 §23] [계획 VII.2.5]
- Return으로 끝내거나 Escape로 취소하면 포커스가 목록으로 돌아옵니다. 클릭·Tab으로 다른 곳에 간 포커스는 건드리지 않습니다. [가이드 8.2]
- 입력은 AppKit `NSTextField`와 창 field editor이므로 IME·맞춤법·Undo가 시스템과 같습니다. [아키텍처 §11.2] [가이드 5.2]
- 이름이 빈 노드는 종류 이름(예: "Text")을 placeholder로 보입니다. 이름은 비어 있을 수 있습니다. [계획 VI.1.3]
- 편집 중 도착한 snapshot은 행을 다시 만들지 않도록 미뤄 두었다가 편집이 끝난 다음 런 루프에서 반영합니다.

### 4.3 표시·잠금 토글

- 행 오른쪽의 테두리 없는 아이콘 버튼입니다. 마우스를 올린 행·선택한 행에서 보이고, 숨김·잠김 상태인 행에서는 항상 보입니다. 대비 높이기가 켜져 있으면 모든 행에서 보입니다. 숨겨진 토글도 자리를 차지해 열 위치가 행마다 같습니다.
- 클릭 대상은 symbol 크기(약 21×13pt)가 아니라 HIG macOS 최소 컨트롤 크기 20×20pt 이상입니다. 이미지는 가운데에 그대로 그립니다. [가이드 4.4] [HIG Accessibility]
- 크기 제약은 버튼의 alignment rect에 겁니다. 버튼 frame은 symbol 정렬 여백만큼 위아래로 더 커서(macOS 27.2 실측 24~27pt), 작은 사이드바 크기 설정의 행(24pt)에서는 셀 밖으로 최대 2pt 나갑니다. AppKit hit test는 셀 밖의 점을 그 안의 버튼으로 넘기지 않으므로 그 부분은 눌리지 않고 옆 행의 클릭도 가로채지 않습니다. 실제로 눌리는 영역(frame과 셀의 겹침)은 작게·중간·크게 모두 20×24pt 이상입니다(11절 검사).
- 버튼은 해당 행 하나에만 적용하며 선택을 바꾸지 않습니다. 여러 행에 적용할 때(단축키·문맥 메뉴): 모두 보이면 숨기고 하나라도 숨겨져 있으면 모두 보입니다. 잠금도 같은 규칙입니다. 대상이 창 선택 전체이면 메뉴 막대 Hide·Lock과 같은 선택 토글(`STELLAR_SELECTION_TOGGLE_*`)로 보내 방향을 코어가 정하고, 그 밖의 행(선택하지 않은 행의 버튼)은 그 행 값을 뒤집어 보냅니다(ADR-0028 "레이어 표시·잠금 변경").
- 잠금 규칙(D8: 잠긴 조상 아래 행은 바꾸지 않고, 잠긴 행은 숨기지 않음)은 코어가 행마다 계산해 행 플래그로 줍니다(표시 변경 가능·잠금 변경 가능). 셸은 그 값으로 버튼을 끄고, 꺼진 버튼의 도움말에 이유 한 가지("Locked layers can’t be hidden.", "A parent layer is locked.")를 둡니다. 표시·잠금은 프로젝트 신뢰와 무관하게 됩니다(D1).
- 숨김은 흐린 글자만이 아니라 `eye.slash` 아이콘과 접근성 문구("Hidden")로도 알립니다. [가이드 4.2 "색에만 의존하지 않음"]

### 4.4 문맥 메뉴

- 대상: 클릭한 행이 선택에 속하면 선택 전체, 아니면 클릭한 행 하나(macOS 목록 관례). [HIG Lists and tables]
- 항목: Rename / Duplicate / Delete — Group Selection / Ungroup — Hide·Show / Lock·Unlock. 캔버스 문맥 메뉴와 같은 항목이며 Command가 연결된 것만 넣습니다. 메뉴에 있는 항목이 잠깐 쓸 수 없으면 숨기지 않고 비활성화하며, 활성화 여부는 provider 사전 검증(`validate`)으로 정합니다. [가이드 8.1] [HIG The menu bar] [UX 명세 7.2]
- Group은 대상을 새 그룹 marker(Fragment) 아래로 옮기는 command이고, Ungroup은 그룹 marker를 지우고 자식을 그 자리로 올리는 command입니다. 실제 의미(어떤 노드를 쓸지, 위치)는 코어가 정합니다. [아키텍처 §12.4]
- 문맥 메뉴 항목에는 단축키 표시(key equivalent)를 넣지 않습니다. 단축키는 메뉴 막대 항목(XII.6.5)이 보입니다. [HIG Context menus]

### 4.5 드래그 재정렬·재부모화

- 같은 레이어 목록 안에서만 이동(`.move`) 드래그를 받습니다. 다른 앱·캔버스로의 드래그는 없습니다.
- 드롭 위치는 `SidebarDropTarget(parentID, index)`(이동 대상 제거 전 기준 표시 순서)로 바꾼 뒤 `provider.validate(.move(...))`로 사전 검증합니다. 표시 순서를 OrderKey로 바꾸는 일은 코어 몫입니다. [ADR-0003] [계획 VII.2.3]
- 행 "위"에 놓기가 거부되면(자식을 가질 수 없는 종류 등) 그 행 바로 위 형제 자리로 바꿔 다시 검증하고, 둘 다 거부되면 드롭을 받지 않습니다. 제목 행 위·아래는 최상위의 처음·끝입니다.
- 거부 사유는 `crates/document/src/tree_ops.rs`의 `ReparentError`(SelfParent, DescendantParent, CrossViewMove, ViewRootMove, StaleHandle)와 대응하는 `SidebarDenialReason`으로 받습니다.
- 필터가 켜져 있으면 드래그를 시작하지 않습니다(표시 순서가 실제 순서와 다름).
- 애니메이션 없이 다음 snapshot으로 목록을 갱신합니다.

### 4.6 필터

- 필터 필드는 사이드바 split view item의 상단 accessory입니다. 입력이 잠시 멈추면(시스템 검색 필드 지연) `.setFilter(질의)`를 보내고, 결과는 snapshot(`filter`, 필터된 `layers`)으로 받습니다. 질의 실행은 셸이 하지 않습니다. [가이드 2.3, 6.2, 9장] [아키텍처 §6.4, §9.2] [HIG Search fields]
- 지우기 버튼·Escape로 지우기는 시스템 검색 필드 동작입니다. 필터를 켜기 전 펼침 상태를 기억했다가 필터를 끄면 복원합니다.
- macOS 26.1 이상에서는 accessory의 scroll edge effect를 soft로 지정하고, 26.0에서는 시스템 기본값을 씁니다. [가이드 2.3] [ADR-0006]

### 4.7 펼침과 지연 로드

- 자식 목록은 부모 단위로 받습니다. `hasChildren == true`이고 `childIDs`에 없는 행을 펼치면 `.loadChildren(id)`을 보내고, 도착한 snapshot에서 자식을 보입니다. [가이드 6.2 "대량 레이어 목록은 페이지 단위로"] [아키텍처 §6.3, §8.2]
- 사용자가 펼친 행은 구조 reload 뒤에도 유지합니다. active View가 바뀌면 펼침 상태를 비웁니다(13절).

## 5. 키보드

모든 동작은 키보드로 닿을 수 있습니다. 목록 기본 동작(화살표 이동, ←·→ 접기·펼치기, ⌥ 클릭 하위 전체, 문자 입력 type-select, ⇧·⌘ 다중 선택, ⌘A)은 `NSOutlineView` 그대로입니다. [가이드 6.2, 8.1, 8.2] [HIG Keyboard, HIG Outline views]

| 키 | 동작 | 근거·비고 |
|---|---|---|
| Return / Enter | 선택한 행 이름 변경(한 행일 때) | 가이드 6.2 |
| Escape | 이름 변경 취소 | 가이드 6.2 |
| Delete(⌫) / Forward Delete(⌦) | 선택 삭제 | 레이어 목록 관례 |
| ⌘D | 복제 | Finder Duplicate와 같은 표준 의미 |
| ⌥⌘G / ⇧⌥⌘G | 묶기 / 풀기 | ⌘G(Find Next) 표준 의미 보존, macOS 문서 편집 앱 관례 |
| ⇧⌘H | 보이기·숨기기 | 잠정값(디자인 도구 관례), 열린 결정 O8 |
| ⇧⌘L | 잠그기·풀기 | 잠정값(디자인 도구 관례), 열린 결정 O8 |
| Tab / ⇧Tab | 필터 → 머리 줄 팝업 → 레이어 목록 | 가이드 8.2 |

- 메뉴 막대(XII.6.5)가 생기기 전까지 ⌘ 단축키는 레이어 목록이 first responder일 때만 목록이 처리합니다. 텍스트 편집 중에는 field editor가 먼저 받으므로 발동하지 않습니다. [가이드 8.2]
- 메뉴 막대가 연결할 target 없는 responder action을 `SidebarOutlineView`에 두었습니다: `delete(_:)`, `duplicate(_:)`, `renameSidebarItem(_:)`, `groupSidebarLayers(_:)`, `ungroupSidebarLayers(_:)`, `toggleSidebarLayerVisibility(_:)`, `toggleSidebarLayerLock(_:)`. 활성화는 `validateUserInterfaceItem`이 같은 검증으로 답합니다.
- 행 안의 토글 버튼은 Tab 순서에 넣지 않습니다(목록 관례). 같은 동작은 단축키·문맥 메뉴·VoiceOver custom action으로 닿습니다.

## 6. 접근성

- 목록: "Layers" 레이블. 머리 줄 팝업: "Page and View" 레이블(`SidebarStrings.pagePath`). 필터 필드: "Filter Layers" 레이블과 도움말 태그. [가이드 8.3] [아키텍처 §18 "앱 자체 UI는 네이티브 접근성 API"]
- 접근성 서버가 내보내는 행 구조(macOS 27.2 실측): 목록(`AXOutline`) → 행(`AXRow`) → 셀(`AXCell`) 하나 → 이름 필드·보이는 토글·펼침 삼각형. `AXRow`·`AXCell`은 AppKit 내부 요소이며, AppKit은 `AXCell`의 설명과 custom action을 셀 view(`NSTableCellView`)의 `accessibilityLabel()`·`accessibilityCustomActions()`에서 가져옵니다. 그래서 행 레이블과 custom action은 셀 view에 둡니다. 행 view(`NSTableRowView`)에 둔 값은 `AXRow`에 쓰이지 않습니다. VoiceOver는 키보드 포커스 목록의 선택 행(`AXSelectedRows`)을 읽고, 그 행의 유일한 자식인 `AXCell`이 행 전체를 대표합니다. [가이드 8.3 "하나의 요소로 묶어"] [AppKit 문서 NSAccessibility]
- AppKit 컨트롤의 접근성 요소는 view가 아니라 cell(`NSButtonCell`, `NSImageCell`, `NSSearchFieldCell`)입니다. view에만 넣은 레이블·제외 설정은 그 요소에 닿지 않으므로 cell에도 둡니다(`SidebarAccessibility`). 예: view에만 레이블을 둔 토글은 symbol 기본 설명(eye → "show")을 읽어 실제 동작(Hide)과 반대로 들렸고, view만 뺀 종류 아이콘은 `AXImage`("Rectangle with a dashed outline")로 남았습니다.
- 레이어 행 레이블(`AXCell` 설명): `이름, 종류[, Hidden][, Locked][, Contains matches]`. 이름이 비면 종류 이름을 씁니다. 종류 아이콘은 장식으로 보고 접근성 트리에서 뺍니다. 선택 상태는 시스템 행(`AXRow`)이 노출합니다. [가이드 8.3] [HIG Accessibility]
- 토글 버튼 레이블은 다음 동작을 말합니다(예: "Hide Header", "Unlock Background"). 도움말 태그도 둡니다. 숨긴 토글은 접근성 트리에서도 빠지고, 보이는 토글은 화면 순서(표시 → 잠금)로 노출됩니다. [가이드 5.4, 8.1, 8.3]
- VoiceOver custom action(`AXCell`, 행마다): Rename, Hide·Show, Lock·Unlock, Duplicate, Delete. 해당 행 하나에만 적용하고 선택을 바꾸지 않습니다. 토글이 숨겨져 있어도 이 동작으로 닿습니다. [가이드 8.3 "제스처 동작에 접근성 기본 동작 연결"]
- 상태 화면은 하나의 group 요소로 제목과 설명을 함께 읽고, 상태 아이콘은 노출하지 않습니다.
- 시스템 접근성 설정 [가이드 8.4]
  - 대비 높이기: 토글을 모든 행에서 보입니다. 색은 모두 시스템 의미 색이라 시스템이 대비를 올립니다.
  - 투명도 줄이기: 직접 그리는 반투명 표면이 없습니다. 사이드바 재질은 split view item이 시스템 설정대로 그립니다.
  - 동작 줄이기: 이 영역은 애니메이션을 쓰지 않습니다(목록 갱신은 즉시 reload, 스크롤은 즉시 이동). 진행 표시기만 시스템 컨트롤 동작을 따릅니다.
  - `NSWorkspace.accessibilityDisplayOptionsDidChangeNotification`을 구독해 실행 중 변경을 반영합니다.
- 색: 이름 `labelColor`, 숨김·필터 문맥 `secondaryLabelColor`, 둘 다 `tertiaryLabelColor`, 토글·상태 아이콘 `secondaryLabelColor`. 하드코딩 RGB 없음. [가이드 4.2]

## 7. SF Symbols

이미지 asset은 쓰지 않습니다. 모든 이름은 `SidebarUICheck`가 실행 OS에서 해석되는지 확인합니다. [가이드 5.4] [HIG SF Symbols]

| 대상 | Symbol | 대상 | Symbol |
|---|---|---|---|
| Frame | `rectangle.dashed` | Path | `point.topleft.down.to.point.bottomright.curvepath` |
| Fragment(그룹 marker) | `square.on.square.dashed` | Text | `textformat` |
| Shape(세부 종류 모름) | `square.on.circle` | Image | `photo` |
| Shape: Rectangle | `rectangle` | ComponentInstance | `diamond` |
| Shape: Ellipse | `circle` | 페이지 | `doc` |
| Shape: Line | `line.diagonal` | View | `rectangle.on.rectangle` |
| Shape: Polygon | `pentagon` | 보임 / 숨김 | `eye` / `eye.slash` |
| 빈 레이어 | `square.stack.3d.up.slash` | 잠김 / 풀림 | `lock.fill` / `lock.open` |
| 결과 없음 | `magnifyingglass` | View 없음 | `sidebar.left` |
| 로드 실패 | `exclamationmark.triangle` | | |

노드 종류 숫자 값은 `crates/document/src/node_kind.rs`(`NodeKind`, Frame=1 … ComponentInstance=7), Shape 세부 종류는 `crates/document/src/shape.rs`(`ShapeKind`, Rectangle=1 … Polygon=4)와 같게 `SidebarNodeKind`·`SidebarShapeKind`에 고정했습니다.

## 8. Provider 프로토콜

```swift
@MainActor
protocol SidebarDataProvider: AnyObject {
    var snapshot: SidebarSnapshot { get }                       // 코어 → 셸: 불변 값 snapshot
    var snapshotDidChange: (() -> Void)? { get set }            // 변경 통지(셸이 다음 런 루프로 병합)
    func validate(_ command: SidebarCommand) -> SidebarCommandValidation  // 가벼운 사전 검증
    func perform(_ command: SidebarCommand)                     // 셸 → 코어: 의도
}
```

- `SidebarSnapshot`: `generation`, `loadState`, `pages`(페이지·View 행), `activePageID`, `activeViewID`, `layers`(`structureRevision`, `topLevelIDs`, 부모별 `childIDs`, `rows`), `selection`·`primarySelection`, `filter`(`query`, `matchCount`).
- `SidebarLayerRow`: `id`, `parentID`, `kind`, `shapeKind`, `name`, `isVisible`, `isLocked`, `hasChildren`, `isFilterMatch`, `canHide`, `canLock`(코어가 잠금 규칙으로 계산).
- `SidebarCommand`: `activateView`, `setSelection`, `renameNode`, `setVisible`, `setLocked`, `duplicate`, `delete`, `group`, `ungroup`, `move`, `setFilter`, `loadChildren`, `loadMore`, `reload`. Page·View 이름 변경 command는 코어 Command와 함께 UX.12에서 더합니다.
- 식별자는 `SidebarStableID`(UUID 128비트 = StableId)이며 셸은 해석하지 않습니다. 모든 필드는 UUID·정수·Bool·UTF-8 문자열이라 FFI POD로 옮길 수 있고 Rust 타입을 노출하지 않습니다. [아키텍처 §8.2, §12.1] [ADR-0009]
- 계약
  - `generation`은 단조 증가합니다. 셸은 이미 반영한 세대보다 오래된 snapshot을 버립니다. [아키텍처 §9.1]
  - `structureRevision`은 행 집합·부모 관계·순서·`hasChildren`이 바뀔 때마다 올라갑니다. 같으면 셸은 보이는 행만 다시 채웁니다.
  - 선택된 노드의 조상 사슬은 지연 로드 중에도 snapshot에 실어 셸이 선택을 펼쳐 보일 수 있게 합니다.
  - `validate`는 메인 스레드에서 2ms 안에 끝나야 하며(드래그 중 반복 호출), 최종 판정은 command 적용 때 코어가 다시 합니다. [아키텍처 §6.3]
  - `perform`은 즉시 반환합니다. 결과와 거부는 다음 snapshot으로 돌아옵니다.

## 9. 성능

- 메인 스레드에서는 snapshot 반영과 네이티브 UI 갱신만 합니다. 필터 질의, 트리 구성, 자식 페이지 계산은 provider(코어) 몫입니다. [아키텍처 §6.3, §9.2] [가이드 9장]
- 연속된 변경 통지는 다음 런 루프(common mode, 드래그 중 포함)에서 한 번만 반영합니다. [가이드 9장]
- 구조가 같으면 이미 만들어진 행 가운데 값이 바뀐 행만 제자리에서 다시 채웁니다. 구조가 바뀌면 목록을 다시 읽고 펼침을 복원합니다. 행 뷰는 보이는 범위 근처만 만듭니다(`NSOutlineView` 재사용). [가이드 6.2, 9장]
- "이미 만들어진 행"은 `enumerateAvailableRowViews`로 돕니다. NSTableView는 스크롤 준비로 보이는 범위 밖 행 뷰도 만들어 두고(responsive scrolling overdraw) 그 행이 화면에 들어와도 `viewFor`를 다시 부르지 않으므로, 보이는 범위(`visibleRect`)만 돌면 그 행이 다음 구조 reload까지 이전 값으로 남습니다. 대비 높이기 변경도 같은 경로로 반영합니다.
- 남은 비용: 구조 reload는 펼쳐진 행 수에 비례하는 data source 호출을 합니다. 한 부모 아래 자식이 매우 많을 때의 부모 내부 페이지 분할은 코어 연결 때 측정해 정합니다(13절).

## 10. 이 작업에서 정한 수치

가이드 0.2에 따라 수치는 구현 작업에서 정하고 가이드 12장에 기록합니다. 이 작업은 기존 파일을 수정하지 않으므로 아래를 통합 때 가이드 12장으로 옮길 후보로 둡니다.

| 항목 | 값 | 근거 | 위치 |
|---|---|---|---|
| 머리 줄 여백 | 좌우 12pt, 위·아래 6pt(12pt의 절반) | 팝업 글자 시작점을 레이어 행의 들여쓰기 시작점과 맞추고, 한 줄 머리 줄이 목록과 붙어 보이지 않게 함 [가이드 4.4] | `SidebarMetrics.headerInset` |
| 셀 안 간격 | 시스템 간격 × 0.5 (아이콘↔이름, 이름↔토글, 토글↔끝) | 하드코딩 point 대신 시스템 간격 기준 [가이드 4.4] | `SidebarNameCellView` |
| 행 토글 최소 클릭 대상 | 20×20pt 이상 | HIG Accessibility의 macOS 최소 컨트롤 크기. 작은 사이드바 크기 설정의 행(24pt) 안에서도 눌리는 영역이 20×24pt 이상(4.3절, 11절 검사) [가이드 4.4] | `SidebarMetrics.minimumHitTarget` |
| 상태 화면 좌우 여백 | 시스템 간격 × 1 | 같음 | `SidebarStateView` |
| 검사용 사이드바 폭 | 240pt | 검사 harness 전용. 실제 폭 범위는 XII.9.1 | `SidebarUICheck` |

## 11. 검사 `SidebarUICheck.run()`

`NSSplitViewController`에 사이드바 item(필터 accessory 포함)과 빈 콘텐츠 item을 넣은 화면 밖 960×600 창에서, 사이드바 폭 240pt로 Stub 데이터를 반영한 뒤 확인합니다.

- 구조: SF Symbol 21개 해석, source list 스타일, `NSSearchField`, 사이드바 behavior, accessory 부착, 이 영역이 넣은 `NSVisualEffectView` 없음(시스템 행의 선택 재질은 제외), 행 수, group row, active View 선택, key view 순서.
- layout: Light·Dark·결과 없음 상태와 사이드바 크기 설정 작게·중간·크게(목록 row size style을 직접 지정, 검사 시스템의 현재 설정과 무관)에서 Auto Layout 모호성 없음, 이름 필드가 토글과 겹치지 않음, 토글의 눌리는 영역(frame과 셀의 겹침) 20×20pt 이상, 보이는 레이어 행마다 그 영역 가운데 20×20pt 정사각형의 네 모서리를 창 view 계층의 hit test로 짚으면 그 토글이 받음, 앱 크롬 색이 시스템 catalog 색.
- 접근성: setter 값을 되읽지 않고 접근성 서버가 질의하는 경로로 읽습니다(`AXNode`: NSAccessibility 프로토콜 객체는 프로토콜 메서드, AppKit 표의 `AXRow`·`AXCell`은 legacy attribute 질의). 모든 행이 `AXCell` 하나로 묶임, 행 레이블·custom action(명세로 따로 계산한 값과 비교), 장식 아이콘 `AXImage` 없음, 보이는 토글만 노출되고 레이블이 다음 동작, custom action을 접근성 서버 방식(이름 있는 AX action)으로 실행(Hide·Lock → command, 선택 유지, Rename → 편집 시작·취소), 선택 행(`AXSelectedRows`)의 레이블, 필터 필드 cell 레이블, 상태 화면 group 레이블과 아이콘 미노출, 숨김·잠김·필터 문맥 문구, 빈 이름 placeholder.
- 동작: 선택 command, 외부 선택 펼침, 페이지 행 선택 → active View 행 선택(재선택·접기·펼치기 포함), 페이지 목록 화살표·←·→ 키 이동(active 페이지 행을 지나 위아래로 이동, ←로 부모 페이지 행과 접기, →로 펼치기, 각 단계 command, active가 그대로인 snapshot에서 페이지 행 선택 유지), Return 이름 변경 확정·Escape 취소·무변경, 편집 중 snapshot 지연 반영, 토글(구조 유지 시 셀 재사용), 문맥 메뉴 활성화와 단축키 표시 없음, ⌘D·⌥⌘G·⇧⌥⌘G·⇧⌘H·Delete, 선택 없을 때 미동작, responder action 검증, 드롭 검증(자손 거부·Shape 위 재지정·컨테이너·최상위 끝)과 이동 반영, 필터·결과 없음·필터 해제와 펼침 복원, 빈 View, 로드 중·실패·다시 시도, 지연 자식 로드, 보이는 범위 밖에 만들어 둔 행의 값 변경·대비 높이기 반영, 변경 통지 병합.
- 검사하지 않는 것: 실제 마우스 드래그 세션, 실제 마우스 클릭 이벤트(토글 hit test는 view 계층의 `hitTest`만 부름), VoiceOver 음성 출력, 시스템 접근성 설정을 실제로 켠 상태, 시스템 사이드바 크기 설정을 실제로 바꾼 상태, 화면 픽셀.

`scripts/macos/smoke.sh`는 `--sidebar-ui-check`로 위 fixture를 검사합니다. 기본 창에서 실제 문서를 편집하는 동작(F 생성, Undo·Redo, 저장·재열기, 중첩 행, 두 창의 격리, 형제 65개의 다음 페이지 버튼)은 앱 내장 검사를 지우고 앱을 직접 실행하는 행동 확인으로 봅니다(`docs/design/ui/inspector.md` 9장).

## 12. 기본 창 연결 범위와 남은 일

머리 줄은 작업 문서의 모든 Page·View를 보이고 View 전환을 합니다(문서 revision마다 목록을 다시 읽고, 활성 Page·View 이름은 레이어 페이지가 함께 읽은 값을 우선). Layers는 이 View의 실제 직계 자식을 위에 그린 순서로 보여 주며, 선택과 disclosure를 사용할 수 있습니다. 숨김·잠금과 종류도 원본의 값을 사용합니다. 제한 모드에서도 열려 있는 창 문서를 읽고 편집할 수 있습니다(신뢰 여부와 상관없음, D1). 레이어 행을 선택하고 Return을 누르면 이름을 편집합니다. 자신이나 조상이 잠긴 행은 편집할 수 없으며, 이름 변경은 Undo 한 번으로 되돌릴 수 있습니다. 표시·잠금 토글은 코어가 계산한 잠금 규칙(D8)으로 켜지고 꺼집니다. 부모를 펼치거나 목록 끝의 다음 페이지 행이 화면에 들어오면 그 부모의 다음 64행을 가져옵니다(별도 버튼 없음). 편집으로 revision이 바뀌면 부모마다 이어 읽은 페이지 수만큼 다시 읽고, primary 선택으로 가는 행이 나올 때까지 같은 부모의 다음 페이지를 읽습니다. 창이 이미 다른 revision을 게시해 결과가 비면 실패 화면 없이 버립니다. 코어는 문서 스레드에서 현재 View·revision을 확인하고 형제 순서 인덱스에서 직접 다음 페이지를 읽습니다. 셸의 worker가 문자열과 행을 변환하며 창을 닫거나 View·revision이 바뀐 결과는 폐기합니다. 필터와 Group·Ungroup은 아직 문서 편집에 연결하지 않았으므로 기본 창에서는 비활성으로 두지 않고 숨깁니다.

| 항목 | 관련 WBS·문서 |
|---|---|
| 표시·잠금 command | 계획 VII.2.5(SetProperty specialization) |
| 이동·복제·삭제 command, 사전 검증 | 계획 VII.2.3, VII.2.8, VII.2.2, VI.2.3 |
| 묶기·풀기 command(그룹 marker 생성·제거) | 전용 WBS 없음(계획 반영 필요, 그때까지 `group`·`ungroup` command는 잠정), Fragment 규칙은 아키텍처 §12.4 |
| 다른 페이지·View 탐색과 이름 변경 | 계획 VI.1.3, XII.2.5 |
| 필터 질의 실행(문서 스레드 밖) | 아키텍처 §6.3, §6.4 |
| 메뉴 막대 항목·단축키 확정(⇧⌘H·⇧⌘L 잠정) | 계획 XII.6.5, 열린 결정 O8 |
| 창의 key view loop(캔버스·인스펙터와 연결) | 계획 XII.8.1 |
| 3열 골격에 연결(`makeSplitViewItem()`), 사이드바 폭 범위 | 계획 XII.9.1 |
| 펼침 상태·분할선 위치 유지(View 전환, 창 복원) | 계획 XII.2.5, 가이드 2.6 |

## 13. 한계와 열린 사항

- 빈 레이어 상태에는 버튼을 두지 않고 키 안내 한 줄로 다음 동작을 알립니다(가이드 7.1, UX.1.6). 동작이 캔버스에 있기 때문입니다(F를 눌러 Frame 안에 첫 자식을 드래그, 계획 XII.10.26). 기준선 코드는 아직 "No Layers"와 안내만 보입니다.
- 문서 목록(프로젝트의 여러 문서)은 이 영역에 없습니다. 가이드 6.2의 "문서·페이지·View 목록" 중 문서 단계는 문서 탭·창 구성과 함께 정해야 합니다.
- 컴포넌트 인스턴스 내부(Slot 등)는 펼치지 않습니다. Stub은 Frame·Fragment만 자식을 가질 수 있게 하며, 실제 규칙은 코어 Capability가 정합니다. [아키텍처 §12.3, §15]
- View를 바꾸면 펼침 상태를 버립니다. View별 유지는 WindowViewState(XII.2.5)와 함께 정합니다.
- 검사는 화면 밖 창에서 합니다. 화면 표시 상태는 임시 harness로 Dark 외형에서 한 번 캡처해 확인했으며, 시스템 접근성 설정을 실제로 켠 상태는 확인하지 않았습니다.
- VoiceOver 음성과 동작 메뉴(VO-Command-Space)는 실제로 켜서 확인하지 않았습니다. 접근성 서버 경로의 값(접근성 권한이 있는 임시 harness에서 `AXUIElement`로 읽은 값 포함)만 확인했습니다. `AXRow` 자체는 설명·custom action을 갖지 않으며(AppKit 내부 요소), 행 단위 동작을 `AXRow`에 두는 공개 경로는 `NSTableViewRowAction`(트랙패드 밀기 동작 UI가 함께 생김)뿐이라 쓰지 않았습니다. VoiceOver가 `AXRow`에 커서를 둔 상태에서 자식 `AXCell`의 custom action을 동작 메뉴에 보이는지는 실기기 VoiceOver로 확인해야 합니다.
- macOS 27.2에서 `AXUIElementCopyActionNames`가 `AXCell`의 custom action을 두 번씩 돌려줍니다(일반 NSView 요소는 한 번). AppKit 내부 동작으로 보이며, VoiceOver 동작 메뉴에 항목이 겹쳐 보이는지는 확인하지 못했습니다.
