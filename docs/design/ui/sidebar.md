# 사이드바(페이지·레이어) UI 명세

- 상태: 초안(2026-11-14), 코어 미연결(Stub provider)
- 코드: `apps/macos/StellarMac/UI/Sidebar/`
- 상위 기준: `docs/design/macos-shell-design.md`(이하 "가이드"), `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`(이하 "아키텍처")
- 근거 표기는 가이드 0.1절을 따릅니다. 이 문서는 가이드·아키텍처·구현 계획을 바꾸지 않으며, 어긋나면 그 문서들이 우선입니다.

## 1. 보여 주는 것

메인 창 3열 중 왼쪽 사이드바입니다. [가이드 2.3, 6.2] [HIG Sidebars]

```text
┌ 필터 필드 (split view item 상단 accessory, NSSearchField) ┐
├ Pages ──────────────────────────────────────────────────────┤
│ ▾ 📄 Onboarding                                            │  페이지 → View (두 단계)
│     ▭ Welcome   ← 선택 = active View                        │
│     ▭ Sign In                                               │
├──────────────── 얇은 분할선 (NSSplitView) ──────────────────┤
├ Layers ─────────────────────────────────────────────────────┤
│ ▾ ⬚ Header                              👁 🔓 (hover·선택)   │  깊이 제한 없는 레이어 트리
│     Aa Title                                                │
│   ▭ Background                                  🔒 (잠김)    │
│   ○ Glow (흐린 글자)                         👁̸ (숨김)      │
└─────────────────────────────────────────────────────────────┘
```

- **페이지 구획**: 열린 문서의 페이지와 각 페이지의 View 목록. HIG의 "두 단계 이내"에 맞춰 페이지 → View 두 단계만 둡니다. 선택은 하나이며 선택이 곧 active 페이지·View입니다. [가이드 6.2] [아키텍처 §12, 계획 VI.1.1]
- **레이어 구획**: active View의 root Frame 아래 노드 트리. root Frame 자체는 View 행이 대표하므로 행으로 두지 않고, 그 자식부터 최상위 행으로 보입니다. 깊이 제한 없이 `NSOutlineView` disclosure로 표현합니다. [가이드 6.2] [HIG Outline views]
- 두 구획 모두 source list(사이드바) 스타일 `NSOutlineView`이고, 구획 제목("Pages", "Layers")은 접을 수 없는 group row입니다. 사이드바 재질·접기·폭 유지는 `NSSplitViewItem(sidebarWithViewController:)`가 맡습니다. [가이드 2.3, 3.2]
- 레이어 행: 종류 아이콘(SF Symbol), 이름(인라인 이름 변경), 표시 토글, 잠금 토글. 행 높이·글꼴·아이콘 크기는 시스템 사이드바 크기 설정(row size style)을 따릅니다. [가이드 4.1, 4.4]
- 문서 목록(프로젝트의 여러 문서)은 이 영역에 넣지 않았습니다. 13절 참고.

## 2. 구성

| 파일 | 역할 |
|---|---|
| `SidebarModel.swift` | 값 snapshot·command·검증 결과 타입, `SidebarDataProvider` 프로토콜 |
| `SidebarStubDataProvider.swift` | 의도된 placeholder provider(메모리 트리, 예시 데이터) |
| `SidebarViewController.swift` | 구성, snapshot 반영 병합, 상태 화면 결정, split view item·accessory 연결, `SidebarMetrics` |
| `SidebarPagesController.swift` | 페이지·View outline, 공통 outline 생성(`SidebarOutlineFactory`), `SidebarOutlineItem` |
| `SidebarLayersController.swift` | 레이어 outline: 데이터·선택·펼침·이름 변경·토글·문맥 메뉴·드래그 |
| `SidebarOutlineView.swift` | `NSOutlineView` 하위 클래스: Return·Delete·단축키, 메뉴 막대용 responder action |
| `SidebarCellViews.swift` | 구획 제목 셀, 이름 셀(페이지·View), 레이어 셀(토글 포함) |
| `SidebarInlineRename.swift` | 인라인 이름 변경 확정·취소 처리 |
| `SidebarStateView.swift` | 로드 중·실패·View 없음·빈 목록·결과 없음 화면 |
| `SidebarFilterAccessoryViewController.swift` | 필터 필드 accessory(`NSSplitViewItemAccessoryViewController`) |
| `SidebarSymbols.swift` | SF Symbol 이름, UI 문자열(지역화 대상, 개발 지역 en) |
| `SidebarUICheck.swift` | 정적 검사 진입점 `SidebarUICheck.run() -> Bool` |

AppKit만 씁니다. 가이드 5.3은 빈 상태 화면에 SwiftUI를 허용하지만, 목록 위 겹침·클릭 통과 처리를 한 계층에서 다루려고 AppKit으로 만들었습니다. [가이드 5.3] [아키텍처 §8.1]

## 3. 상태

레이어 구획 위에 `SidebarStateView`를 겹쳐 보입니다. 배경을 칠하지 않으며 버튼 외 영역의 클릭은 아래 목록으로 통과합니다. 페이지 구획은 상태 화면을 두지 않습니다. [가이드 7.1] [HIG Writing]

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
- 페이지 구획 선택: 페이지 행 → `.activatePage`, View 행 → `.activateView`. 빈 곳을 눌러 선택이 비면 active 표시를 되살립니다.

### 4.2 인라인 이름 변경

- 시작: Return, 행 더블클릭, 문맥 메뉴 Rename, VoiceOver custom action.
- 확정: Return 또는 포커스 이동. 취소: Escape(필드 값을 snapshot 값으로 복원, command 없음). 값이 snapshot 이름과 같으면 command를 만들지 않습니다. [가이드 6.2] [아키텍처 §23] [계획 VII.2.5]
- Return으로 끝내거나 Escape로 취소하면 포커스가 목록으로 돌아옵니다. 클릭·Tab으로 다른 곳에 간 포커스는 건드리지 않습니다. [가이드 8.2]
- 입력은 AppKit `NSTextField`와 창 field editor이므로 IME·맞춤법·Undo가 시스템과 같습니다. [아키텍처 §11.2] [가이드 5.2]
- 이름이 빈 노드는 종류 이름(예: "Text")을 placeholder로 보입니다. 이름은 비어 있을 수 있습니다. [계획 VI.1.3]
- 편집 중 도착한 snapshot은 행을 다시 만들지 않도록 미뤄 두었다가 편집이 끝난 다음 런 루프에서 반영합니다.

### 4.3 표시·잠금 토글

- 행 오른쪽의 테두리 없는 아이콘 버튼입니다. 마우스를 올린 행·선택한 행에서 보이고, 숨김·잠김 상태인 행에서는 항상 보입니다. 대비 높이기가 켜져 있으면 모든 행에서 보입니다. 숨겨진 토글도 자리를 차지해 열 위치가 행마다 같습니다.
- 버튼은 해당 행 하나에만 적용하며 선택을 바꾸지 않습니다. 여러 행에 적용할 때(단축키·문맥 메뉴): 모두 보이면 숨기고 하나라도 숨겨져 있으면 모두 보입니다. 잠금도 같은 규칙입니다.
- 숨김은 흐린 글자만이 아니라 `eye.slash` 아이콘과 접근성 문구("Hidden")로도 알립니다. [가이드 4.2 "색에만 의존하지 않음"]

### 4.4 문맥 메뉴

- 대상: 클릭한 행이 선택에 속하면 선택 전체, 아니면 클릭한 행 하나(macOS 목록 관례). [HIG Lists and tables]
- 항목: Rename / Duplicate / Delete — Group Selection / Ungroup — Hide·Show / Lock·Unlock. 지금 쓸 수 없는 항목은 숨기지 않고 비활성화하며, 활성화 여부는 provider 사전 검증(`validate`)으로 정합니다. [가이드 8.1] [HIG The menu bar]
- Group은 대상을 새 그룹 marker(Fragment) 아래로 옮기는 command이고, Ungroup은 그룹 marker를 지우고 자식을 그 자리로 올리는 command입니다. 실제 의미(어떤 노드를 쓸지, 위치)는 코어가 정합니다. [아키텍처 §12.4]
- 페이지 구획의 문맥 메뉴는 Rename만 있습니다.

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
| ⌥⌘G / ⇧⌥⌘G | 묶기 / 풀기 | ⌘G(Find Next) 표준 의미 보존, Apple 앱(Keynote·Pages) 관례 |
| ⇧⌘H | 보이기·숨기기 | 잠정값(디자인 도구 관례), 열린 결정 O8 |
| ⇧⌘L | 잠그기·풀기 | 잠정값(디자인 도구 관례), 열린 결정 O8 |
| Tab / ⇧Tab | 필터 → 페이지 목록 → 레이어 목록 | 가이드 8.2 |

- 메뉴 막대(XII.6.5)가 생기기 전까지 ⌘ 단축키는 레이어 목록이 first responder일 때만 목록이 처리합니다. 텍스트 편집 중에는 field editor가 먼저 받으므로 발동하지 않습니다. [가이드 8.2]
- 메뉴 막대가 연결할 target 없는 responder action을 `SidebarOutlineView`에 두었습니다: `delete(_:)`, `duplicate(_:)`, `renameSidebarItem(_:)`, `groupSidebarLayers(_:)`, `ungroupSidebarLayers(_:)`, `toggleSidebarLayerVisibility(_:)`, `toggleSidebarLayerLock(_:)`. 활성화는 `validateUserInterfaceItem`이 같은 검증으로 답합니다.
- 행 안의 토글 버튼은 Tab 순서에 넣지 않습니다(목록 관례). 같은 동작은 단축키·문맥 메뉴·VoiceOver custom action으로 닿습니다.

## 6. 접근성

- 목록: "Pages", "Layers" 레이블. 필터 필드: "Filter Layers" 레이블과 도움말 태그. [가이드 8.3] [아키텍처 §18 "앱 자체 UI는 네이티브 접근성 API"]
- 접근성 서버가 내보내는 행 구조(macOS 27.2 실측): 목록(`AXOutline`) → 행(`AXRow`) → 셀(`AXCell`) 하나 → 이름 필드·보이는 토글·펼침 삼각형. `AXRow`·`AXCell`은 AppKit 내부 요소이며, AppKit은 `AXCell`의 설명과 custom action을 셀 view(`NSTableCellView`)의 `accessibilityLabel()`·`accessibilityCustomActions()`에서 가져옵니다. 그래서 행 레이블과 custom action은 셀 view에 둡니다. 행 view(`NSTableRowView`)에 둔 값은 `AXRow`에 쓰이지 않습니다. VoiceOver는 키보드 포커스 목록의 선택 행(`AXSelectedRows`)을 읽고, 그 행의 유일한 자식인 `AXCell`이 행 전체를 대표합니다. [가이드 8.3 "하나의 요소로 묶어"] [AppKit 문서 NSAccessibility]
- AppKit 컨트롤의 접근성 요소는 view가 아니라 cell(`NSButtonCell`, `NSImageCell`, `NSSearchFieldCell`)입니다. view에만 넣은 레이블·제외 설정은 그 요소에 닿지 않으므로 cell에도 둡니다(`SidebarAccessibility`). 예: view에만 레이블을 둔 토글은 symbol 기본 설명(eye → "show")을 읽어 실제 동작(Hide)과 반대로 들렸고, view만 뺀 종류 아이콘은 `AXImage`("Rectangle with a dashed outline")로 남았습니다.
- 레이어 행 레이블(`AXCell` 설명): `이름, 종류[, Hidden][, Locked][, Contains matches]`. 이름이 비면 종류 이름을 씁니다. 종류 아이콘은 장식으로 보고 접근성 트리에서 뺍니다. 선택 상태는 시스템 행(`AXRow`)이 노출합니다. [가이드 8.3] [HIG Accessibility]
- 토글 버튼 레이블은 다음 동작을 말합니다(예: "Hide Header", "Unlock Background"). 도움말 태그도 둡니다. 숨긴 토글은 접근성 트리에서도 빠지고, 보이는 토글은 화면 순서(표시 → 잠금)로 노출됩니다. [가이드 5.4, 8.1, 8.3]
- VoiceOver custom action(`AXCell`, 행마다): Rename, Hide·Show, Lock·Unlock, Duplicate, Delete. 페이지·View 행: Rename. 해당 행 하나에만 적용하고 선택을 바꾸지 않습니다. 토글이 숨겨져 있어도 이 동작으로 닿습니다. [가이드 8.3 "제스처 동작에 접근성 기본 동작 연결"]
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
- `SidebarLayerRow`: `id`, `parentID`, `kind`, `shapeKind`, `name`, `isVisible`, `isLocked`, `hasChildren`, `isFilterMatch`.
- `SidebarCommand`: `activatePage`, `activateView`, `renamePage`, `renameView`, `setSelection`, `renameNode`, `setVisible`, `setLocked`, `duplicate`, `delete`, `group`, `ungroup`, `move`, `setFilter`, `loadChildren`, `reload`.
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
- 구조가 같으면 화면에 보이는 행 가운데 값이 바뀐 행만 제자리에서 다시 채웁니다. 구조가 바뀌면 목록을 다시 읽고 펼침을 복원합니다. 행 뷰는 보이는 범위만 만듭니다(`NSOutlineView` 재사용). [가이드 6.2, 9장]
- 남은 비용: 구조 reload는 펼쳐진 행 수에 비례하는 data source 호출을 합니다. 한 부모 아래 자식이 매우 많을 때의 부모 내부 페이지 분할은 코어 연결 때 측정해 정합니다(13절).

## 10. 이 작업에서 정한 수치

가이드 0.2에 따라 수치는 구현 작업에서 정하고 가이드 12장에 기록합니다. 이 작업은 기존 파일을 수정하지 않으므로 아래를 통합 때 가이드 12장으로 옮길 후보로 둡니다.

| 항목 | 값 | 근거 | 위치 |
|---|---|---|---|
| 페이지 구획 첫 높이 비율 | safe area 아래 높이의 0.3 | 페이지(수 개)보다 레이어(수백~)가 많음. 이후 사용자 분할선 위치를 따름 | `SidebarMetrics.initialPagesFraction` |
| 구획 최소 높이 | 3행(제목 행 포함) × 시스템 행 높이 | 분할선이 사라지지 않게 함 [HIG Split views] | `SidebarMetrics.minimumPaneRows` |
| 셀 안 간격 | 시스템 간격 × 0.5 (아이콘↔이름, 이름↔토글, 토글↔끝) | 하드코딩 point 대신 시스템 간격 기준 [가이드 4.4] | `SidebarNameCellView` |
| 상태 화면 좌우 여백 | 시스템 간격 × 1 | 같음 | `SidebarStateView` |
| 검사용 사이드바 폭 | 240pt | 검사 harness 전용. 실제 폭 범위는 XII.9.1 | `SidebarUICheck` |

## 11. 검사 `SidebarUICheck.run()`

`NSSplitViewController`에 사이드바 item(필터 accessory 포함)과 빈 콘텐츠 item을 넣은 화면 밖 960×600 창에서, 사이드바 폭 240pt로 Stub 데이터를 반영한 뒤 확인합니다.

- 구조: SF Symbol 21개 해석, source list 스타일, `NSSearchField`, 사이드바 behavior, accessory 부착, 이 영역이 넣은 `NSVisualEffectView` 없음(시스템 행의 선택 재질은 제외), 행 수, group row, active View 선택, key view 순서.
- layout: Light·Dark·결과 없음 상태에서 Auto Layout 모호성 없음, 이름 필드가 토글과 겹치지 않음, 앱 크롬 색이 시스템 catalog 색.
- 접근성: setter 값을 되읽지 않고 접근성 서버가 질의하는 경로로 읽습니다(`AXNode`: NSAccessibility 프로토콜 객체는 프로토콜 메서드, AppKit 표의 `AXRow`·`AXCell`은 legacy attribute 질의). 모든 행이 `AXCell` 하나로 묶임, 행 레이블·custom action(명세로 따로 계산한 값과 비교), 장식 아이콘 `AXImage` 없음, 보이는 토글만 노출되고 레이블이 다음 동작, custom action을 접근성 서버 방식(이름 있는 AX action)으로 실행(Hide·Lock → command, 선택 유지, Rename → 편집 시작·취소), 선택 행(`AXSelectedRows`)의 레이블, 필터 필드 cell 레이블, 상태 화면 group 레이블과 아이콘 미노출, 숨김·잠김·필터 문맥 문구, 빈 이름 placeholder.
- 동작: 선택 command, 외부 선택 펼침, Return 이름 변경 확정·Escape 취소·무변경, 편집 중 snapshot 지연 반영, 토글(구조 유지 시 셀 재사용), 문맥 메뉴 활성화, ⌘D·⌥⌘G·⇧⌥⌘G·⇧⌘H·Delete, 선택 없을 때 미동작, responder action 검증, 드롭 검증(자손 거부·Shape 위 재지정·컨테이너·최상위 끝)과 이동 반영, 필터·결과 없음·필터 해제와 펼침 복원, 빈 View, 로드 중·실패·다시 시도, 지연 자식 로드, 변경 통지 병합.
- 검사하지 않는 것: 실제 마우스 드래그 세션, VoiceOver 음성 출력, 시스템 접근성 설정을 실제로 켠 상태, 화면 픽셀.

통합 때 `main.swift`에 `--sidebar-ui-check` 같은 인자로 `exit(SidebarUICheck.run() ? 0 : 1)`을 연결하고 `scripts/macos/smoke.sh`에서 호출합니다. 아직 연결되지 않았으므로(두 파일은 이 영역 밖) 지금은 자동 회귀 방지가 없습니다. 지금까지의 결과는 이 폴더의 소스와 `SidebarUICheck.run()`만 부르는 임시 진입점을 저장소 밖에서 컴파일해 얻은 것입니다.

## 12. 코어 연결 시 남은 일

| 항목 | 관련 WBS·문서 |
|---|---|
| `SidebarDataProvider` 코어 구현: poll 이벤트로 snapshot 갱신, command를 문서 스레드로 전달 | ADR-0009, 아키텍처 §8.2, §9 |
| 이름·표시·잠금 command | 계획 VII.2.5(SetProperty specialization) |
| 이동·복제·삭제 command, 사전 검증 | 계획 VII.2.3, VII.2.8, VII.2.2, VI.2.3 |
| 묶기·풀기 command(그룹 marker 생성·제거) | 전용 WBS 없음, Fragment 규칙은 아키텍처 §12.4 |
| 선택 command·창별 선택·Undo 통합 | 계획 VII.3.1~VII.3.4, VII.4.6, XII.2.5 |
| 페이지·View 이름 변경, active View | 계획 VI.1.3, XII.2.5 |
| 필터 질의 실행(문서 스레드 밖), 부모 단위·부모 내부 페이지 분할 | 아키텍처 §6.3, §6.4 |
| 메뉴 막대 항목·단축키 확정(⇧⌘H·⇧⌘L 잠정) | 계획 XII.6.5, 열린 결정 O8 |
| 창의 key view loop(캔버스·인스펙터와 연결) | 계획 XII.8.1 |
| 3열 골격에 연결(`makeSplitViewItem()`), 사이드바 폭 범위 | 계획 XII.9.1 |
| 펼침 상태·분할선 위치 유지(View 전환, 창 복원) | 계획 XII.2.5, 가이드 2.6 |

## 13. 한계와 열린 사항

- 빈 레이어 상태에 "다음 동작 버튼"이 없습니다. 가이드 7.1은 버튼을 요구하지만 노드 생성 command(Frame·Shape·Text 생성 도구)는 WBS가 없습니다(가이드 10.4). 생성 command가 생기면 버튼을 더합니다.
- 문서 목록(프로젝트의 여러 문서)은 이 영역에 없습니다. 가이드 6.2의 "문서·페이지·View 목록" 중 문서 단계는 문서 탭·창 구성과 함께 정해야 합니다.
- 컴포넌트 인스턴스 내부(Slot 등)는 펼치지 않습니다. Stub은 Frame·Fragment만 자식을 가질 수 있게 하며, 실제 규칙은 코어 Capability가 정합니다. [아키텍처 §12.3, §15]
- View를 바꾸면 펼침 상태를 버립니다. View별 유지는 WindowViewState(XII.2.5)와 함께 정합니다.
- 검사는 화면 밖 창에서 합니다. 화면 표시 상태는 임시 harness로 Dark 외형에서 한 번 캡처해 확인했으며, 시스템 접근성 설정을 실제로 켠 상태는 확인하지 않았습니다.
- VoiceOver 음성과 동작 메뉴(VO-Command-Space)는 실제로 켜서 확인하지 않았습니다. 접근성 서버 경로의 값(접근성 권한이 있는 임시 harness에서 `AXUIElement`로 읽은 값 포함)만 확인했습니다. `AXRow` 자체는 설명·custom action을 갖지 않으며(AppKit 내부 요소), 행 단위 동작을 `AXRow`에 두는 공개 경로는 `NSTableViewRowAction`(트랙패드 밀기 동작 UI가 함께 생김)뿐이라 쓰지 않았습니다. VoiceOver가 `AXRow`에 커서를 둔 상태에서 자식 `AXCell`의 custom action을 동작 메뉴에 보이는지는 실기기 VoiceOver로 확인해야 합니다.
- macOS 27.2에서 `AXUIElementCopyActionNames`가 `AXCell`의 custom action을 두 번씩 돌려줍니다(일반 NSView 요소는 한 번). AppKit 내부 동작으로 보이며, VoiceOver 동작 메뉴에 항목이 겹쳐 보이는지는 확인하지 못했습니다.
