import AppKit

/// 사이드바 UI를 화면에 띄우지 않고 검사합니다(`scripts/macos/smoke.sh` 연결 예정).
///
/// `SidebarStubDataProvider` 데이터로 실제 배치(`NSSplitViewController` + 사이드바 item + 필터 accessory)를
/// 일반적인 창 크기에서 만들고 layout을 강제한 뒤 다음을 확인합니다.
/// - SF Symbols 이름이 실행 OS에서 해석됨, source list 스타일, 네이티브 텍스트 컨트롤, `NSVisualEffectView` 없음
/// - Auto Layout 모호성 없음(Light·Dark 모두), 앱 크롬 색이 시스템 의미 색(catalog)임, 토글 클릭 대상 최소 크기
/// - 접근성: setter 값을 되읽지 않고 접근성 서버가 질의하는 경로(`AXNode`)로 읽습니다. 목록 → `AXRow` → `AXCell`의
///   행 레이블·custom action(실행 포함), 장식 아이콘 미노출, 토글·필터·상태 화면 레이블, 선택 행, key view 순서
/// - 선택(페이지 행 → active View 행 포함), 인라인 이름 변경(확정·Escape 취소·무변경), 토글, 문맥 메뉴 검증
///   (단축키 표시 없음), 키보드 단축키, 드롭 사전 검증·재지정, 필터·결과 없음·필터 해제 후 펼침 복원, 빈 View,
///   로드 중·실패·재시도, 지연 자식 로드, 보이는 범위 밖에 만들어 둔 행 갱신, snapshot 변경 통지 병합
/// 실제 마우스 드래그 세션, VoiceOver 음성 출력, 화면 픽셀은 검사하지 않습니다.
enum SidebarUICheck {
    static func run() -> Bool {
        MainActor.assumeIsolated { runOnMainActor() }
    }

    private struct Failure: Error {
        let message: String
    }

    @MainActor
    private static func runOnMainActor() -> Bool {
        do {
            let summary = try check()
            print("sidebar-ui ok: \(summary)")
            return true
        } catch let failure as Failure {
            FileHandle.standardError.write(Data("sidebar-ui 실패: \(failure.message)\n".utf8))
            return false
        } catch {
            FileHandle.standardError.write(Data("sidebar-ui 실패: \(error)\n".utf8))
            return false
        }
    }

    private static func require(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition { throw Failure(message: message()) }
    }

    // MARK: 구성

    @MainActor
    private final class Harness {
        let provider: SidebarStubDataProvider
        let sidebar: SidebarViewController
        let splitController = NSSplitViewController()
        let window: NSWindow

        init(provider: SidebarStubDataProvider) {
            self.provider = provider
            sidebar = SidebarViewController(provider: provider)
            splitController.addSplitViewItem(sidebar.makeSplitViewItem())
            let canvas = NSViewController()
            canvas.view = NSView()
            splitController.addSplitViewItem(NSSplitViewItem(viewController: canvas))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                backing: .buffered,
                defer: false
            )
            window.isReleasedWhenClosed = false
            window.contentViewController = splitController
            window.setContentSize(NSSize(width: 960, height: 600))
            window.contentView?.layoutSubtreeIfNeeded()
            // 사이드바 폭 범위는 셸 골격(XII.9.1)이 정합니다. 검사는 일반적인 사이드바 폭에서 합니다.
            splitController.splitView.setPosition(Self.sidebarWidth, ofDividerAt: 0)
            refresh()
        }

        /// 검사용 일반 사이드바 폭(point).
        static let sidebarWidth: CGFloat = 240

        var layers: SidebarLayersController { sidebar.layersController }
        var pages: SidebarPagesController { sidebar.pagesController }

        /// 최신 snapshot 반영과 layout.
        func refresh() {
            sidebar.applySnapshotNow()
            window.contentView?.layoutSubtreeIfNeeded()
            window.contentView?.displayIfNeeded()
        }

        func commands(since mark: Int) -> [SidebarCommand] {
            Array(provider.performedCommands[mark...])
        }

        func row(of id: SidebarStableID, in outline: NSOutlineView) -> Int {
            for row in 0..<outline.numberOfRows {
                if (outline.item(atRow: row) as? SidebarOutlineItem)?.id == id { return row }
            }
            return -1
        }

        func cell(at row: Int, in outline: NSOutlineView) -> NSTableCellView? {
            outline.rowView(atRow: row, makeIfNecessary: true)?.view(atColumn: 0) as? NSTableCellView
        }

        func layerCell(_ id: SidebarStableID) -> SidebarLayerCellView? {
            let row = row(of: id, in: layers.outline)
            return row >= 0 ? cell(at: row, in: layers.outline) as? SidebarLayerCellView : nil
        }

        /// 접근성 서버가 보는 `id` 행의 `AXCell`(행 레이블·custom action을 내보내는 요소).
        func axCell(_ id: SidebarStableID, in outline: NSOutlineView) -> AXNode? {
            let index = row(of: id, in: outline)
            let rows = AXNode.rows(of: outline)
            guard index >= 0, rows.indices.contains(index) else { return nil }
            return rows[index].cell
        }

        func id(_ name: String) throws -> SidebarStableID {
            guard let id = provider.nodeID(named: name) else { throw Failure(message: "예시 노드 없음: \(name)") }
            return id
        }

        func key(_ characters: String, keyCode: UInt16, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
            NSEvent.keyEvent(
                with: .keyDown,
                location: .zero,
                modifierFlags: modifiers,
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber,
                context: nil,
                characters: characters,
                charactersIgnoringModifiers: characters,
                isARepeat: false,
                keyCode: keyCode
            )!
        }

        func close() {
            window.close()
        }
    }

    // MARK: 검사

    @MainActor
    private static func check() throws -> String {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        for name in SidebarSymbols.allNames {
            try require(SidebarSymbols.image(name) != nil, "SF Symbol을 해석하지 못함: \(name)")
        }

        let harness = Harness(provider: SidebarStubDataProvider())
        defer { harness.close() }
        let layers = harness.layers
        let pages = harness.pages
        let sidebar = harness.sidebar

        try checkStructure(harness)
        try checkLayout(harness, label: "Light", appearance: .aqua)
        try checkAccessibility(harness)

        // 선택: 사이드바 선택 → .setSelection.
        let headerID = try harness.id("Header")
        var mark = harness.provider.performedCommands.count
        layers.outline.selectRowIndexes(IndexSet(integer: harness.row(of: headerID, in: layers.outline)), byExtendingSelection: false)
        try require(
            harness.commands(since: mark) == [.setSelection([headerID], primary: headerID)],
            "선택 command 불일치: \(harness.commands(since: mark))"
        )
        harness.refresh()
        // VoiceOver는 키보드 포커스 목록의 선택 행(AXSelectedRows)을 읽습니다. 그 행의 AXCell이 행 레이블을 냅니다.
        let selectedAX = AXNode.selectedRows(of: layers.outline)
        try require(selectedAX.count == 1 && selectedAX[0].cell?.label == "Header, \(SidebarStrings.kindName(.frame, shapeKind: nil))",
                    "선택 행의 접근성 레이블 불일치: \(selectedAX.map { $0.cell?.label ?? "nil" })")

        // 외부 선택(캔버스 등): 깊은 노드를 선택하면 조상이 펼쳐지고 행이 선택됩니다.
        let privacyID = try harness.id("Privacy")
        harness.provider.perform(.setSelection([privacyID], primary: privacyID))
        harness.refresh()
        let privacyRow = harness.row(of: privacyID, in: layers.outline)
        try require(privacyRow >= 0 && layers.outline.selectedRowIndexes == IndexSet(integer: privacyRow), "외부 선택을 펼쳐 보이지 못함")
        try require(layers.expandedIDs.contains(try harness.id("Footer")), "외부 선택의 조상 펼침이 기록되지 않음")

        try checkRename(harness, headerID: headerID)
        try checkToggles(harness)
        try checkMenuAndShortcuts(harness)
        try checkDrop(harness)
        try checkFilter(harness)

        // 빈 View: 페이지 목록에서 View를 고르면 .activateView, 레이어가 없으면 빈 상태.
        guard let generalID = harness.provider.viewID(named: "General") else { throw Failure(message: "예시 View 없음") }
        mark = harness.provider.performedCommands.count
        let generalRow = harness.row(of: generalID, in: pages.outline)
        try require(generalRow >= 0, "페이지 목록에 View 행 없음")
        pages.outline.selectRowIndexes(IndexSet(integer: generalRow), byExtendingSelection: false)
        try require(harness.commands(since: mark) == [.activateView(generalID)], "View 활성화 command 불일치")
        harness.refresh()
        try require(sidebar.stateView.content == .empty && !sidebar.stateView.isHidden, "빈 레이어 상태가 보이지 않음")
        try require(layers.outline.numberOfRows == 1, "빈 View에 레이어 행이 남음")
        try checkStateAccessibility(sidebar.stateView, label: "빈 상태")

        // 로드 중·실패·재시도.
        harness.provider.simulateLoadState(.loading)
        harness.refresh()
        try require(sidebar.stateView.content == .loading && !sidebar.stateView.spinner.isHidden, "로드 중 상태 불일치")
        harness.provider.simulateLoadState(.failed(message: "The document couldn’t be read."))
        harness.refresh()
        try require(sidebar.stateView.content == .failed(message: "The document couldn’t be read."), "실패 상태 불일치")
        try require(!sidebar.stateView.actionButton.isHidden, "실패 상태에 다시 시도 버튼 없음")
        try checkStateAccessibility(sidebar.stateView, label: "실패 상태")
        mark = harness.provider.performedCommands.count
        sidebar.stateView.actionButton.performClick(nil)
        try require(harness.commands(since: mark) == [.reload], "다시 시도 command 불일치")
        harness.refresh()
        try require(sidebar.stateView.content == .empty, "다시 시도 후 상태 불일치")

        try checkPageSelection(harness)
        try checkLayout(harness, label: "Dark", appearance: .darkAqua)
        try checkLazyChildren()
        try checkAvailableRows()
        try checkCoalescing()

        return "symbols \(SidebarSymbols.allNames.count), pages+layers outline, AX rows/cells/custom actions, "
            + "rename/toggle/menu/keyboard/drop/filter/empty/loading/failed/page selection/lazy/offscreen rows/coalescing, "
            + "no ambiguous layout (Light, Dark)"
    }

    /// 상태 화면: 하나의 group 요소가 제목·설명을 레이블로 내고, 장식 아이콘은 노출하지 않습니다.
    @MainActor
    private static func checkStateAccessibility(_ stateView: SidebarStateView, label: String) throws {
        guard let node = AXNode(stateView) else { throw Failure(message: "\(label) 접근성 요소 없음") }
        try require(node.role == NSAccessibility.Role.group.rawValue, "\(label)이 group 요소가 아님: \(node.role ?? "nil")")
        try require(!(node.label ?? "").isEmpty && (node.label ?? "").hasPrefix(stateView.titleField.stringValue),
                    "\(label) 접근성 레이블 불일치: \(node.label ?? "nil")")
        try require(node.descendants(role: .image).isEmpty, "\(label) 장식 아이콘이 AXImage로 노출됨")
    }

    /// 페이지 목록: 선택 = active 페이지·View. 페이지 행을 고르면 코어가 고른 active View 행으로 선택이 옮겨 갑니다.
    @MainActor
    private static func checkPageSelection(_ harness: Harness) throws {
        let pages = harness.pages
        let outline = pages.outline
        guard let onboarding = harness.provider.snapshot.pages.first(where: { $0.name == "Onboarding" }),
              let welcomeID = harness.provider.viewID(named: "Welcome") else {
            throw Failure(message: "예시 페이지 없음")
        }
        // 다른 페이지(Settings)가 active인 상태에서 Onboarding 페이지 행을 고름 → .activatePage → 첫 View 행 선택.
        var mark = harness.provider.performedCommands.count
        outline.selectRowIndexes(IndexSet(integer: harness.row(of: onboarding.id, in: outline)), byExtendingSelection: false)
        try require(harness.commands(since: mark) == [.activatePage(onboarding.id)], "페이지 활성화 command 불일치: \(harness.commands(since: mark))")
        harness.refresh()
        try require(harness.provider.snapshot.activeViewID == welcomeID, "예시 active View 불일치")
        try require(outline.selectedRow == harness.row(of: welcomeID, in: outline),
                    "페이지 활성화 후 active View 행이 선택되지 않음(선택 행 \(outline.selectedRow))")

        // 이미 active인 페이지 행을 다시 고름 → command 없음, 선택은 active View 행으로 돌아감.
        mark = harness.provider.performedCommands.count
        outline.selectRowIndexes(IndexSet(integer: harness.row(of: onboarding.id, in: outline)), byExtendingSelection: false)
        try require(harness.commands(since: mark).isEmpty, "active 페이지 재선택이 command를 만듦: \(harness.commands(since: mark))")
        try require(outline.selectedRow == harness.row(of: welcomeID, in: outline), "active 페이지 재선택 후 선택이 View 행으로 돌아가지 않음")

        // 페이지를 접으면 페이지 행, 다시 펼치면 active View 행.
        guard let pageItem = outline.item(atRow: harness.row(of: onboarding.id, in: outline)) else {
            throw Failure(message: "페이지 행 없음")
        }
        outline.collapseItem(pageItem)
        try require(outline.selectedRow == harness.row(of: onboarding.id, in: outline), "접은 active 페이지 행이 선택되지 않음")
        outline.expandItem(pageItem)
        try require(outline.selectedRow == harness.row(of: welcomeID, in: outline), "다시 펼친 뒤 active View 행이 선택되지 않음")
        try require(harness.commands(since: mark).isEmpty, "접기·펼치기가 command를 만듦: \(harness.commands(since: mark))")
        try require(harness.layers.outline.numberOfRows == 9, "Welcome 레이어가 다시 보이지 않음: \(harness.layers.outline.numberOfRows)")
    }

    // MARK: 구조·layout·접근성

    @MainActor
    private static func checkStructure(_ harness: Harness) throws {
        let layers = harness.layers
        let pages = harness.pages
        let sidebar = harness.sidebar
        try require(layers.outline.style == .sourceList && pages.outline.style == .sourceList, "outline이 source list 스타일이 아님")
        try require(type(of: sidebar.filterAccessory.searchField) == NSSearchField.self, "필터 필드가 NSSearchField가 아님")
        let item = harness.splitController.splitViewItems.first
        try require(item?.behavior == .sidebar, "사이드바 split view item이 아님")
        try require(
            item?.topAlignedAccessoryViewControllers.contains { $0 === sidebar.filterAccessory } ?? false,
            "필터 accessory가 사이드바 item에 붙지 않음"
        )
        // 시스템 NSTableRowView가 source list 선택 표시에 쓰는 재질은 AppKit 내부 구현이므로 제외합니다.
        // 이 영역이 직접 넣은 재질 뷰만 찾습니다(디자인 가이드 3.2).
        let effectViews = paths(of: NSVisualEffectView.self, in: sidebar.view)
            .filter { !$0.contains("NSTableRowView > NSVisualEffectView") }
        try require(effectViews.isEmpty, "사이드바에 NSVisualEffectView가 있음: \(effectViews)")

        // 예시: 페이지 2개(View 2 + 1, 모두 펼침) + 제목 행, 레이어 최상위 8개 + 제목 행.
        try require(pages.outline.numberOfRows == 6, "페이지 행 수 불일치: \(pages.outline.numberOfRows)")
        try require(layers.outline.numberOfRows == 9, "레이어 행 수 불일치: \(layers.outline.numberOfRows)")
        guard let pagesHeader = pages.outline.item(atRow: 0), let layersHeader = layers.outline.item(atRow: 0) else {
            throw Failure(message: "구획 제목 행 없음")
        }
        try require(pages.outlineView(pages.outline, isGroupItem: pagesHeader) && layers.outlineView(layers.outline, isGroupItem: layersHeader),
                    "구획 제목 행이 group row가 아님")
        guard let welcomeID = harness.provider.viewID(named: "Welcome") else { throw Failure(message: "예시 View 없음") }
        try require(pages.outline.selectedRow == harness.row(of: welcomeID, in: pages.outline), "active View가 선택되지 않음")
        try require(sidebar.stateView.isHidden, "레이어가 있는데 상태 화면이 보임")

        // 키보드 탐색 순서: 필터 → 페이지 → 레이어.
        try require(sidebar.filterAccessory.searchField.nextKeyView === pages.outline, "필터 다음 key view가 페이지 목록이 아님")
        try require(pages.outline.nextKeyView === layers.outline, "페이지 다음 key view가 레이어 목록이 아님")
        try require(layers.outline.acceptsFirstResponder && pages.outline.acceptsFirstResponder, "목록이 first responder를 받지 않음")
    }

    @MainActor
    private static func checkLayout(_ harness: Harness, label: String, appearance: NSAppearance.Name) throws {
        harness.window.appearance = NSAppearance(named: appearance)
        harness.window.contentView?.needsLayout = true
        harness.refresh()
        // 보이는 행의 셀을 모두 만들게 합니다.
        for outline in [harness.pages.outline, harness.layers.outline] as [NSOutlineView] {
            for row in 0..<outline.numberOfRows { _ = harness.cell(at: row, in: outline) }
        }
        harness.window.contentView?.layoutSubtreeIfNeeded()

        var ambiguous: [String] = []
        var nonSemantic: [String] = []
        var overlapping: [String] = []
        var smallTargets: [String] = []
        let roots: [NSView] = [harness.sidebar.view, harness.sidebar.filterAccessory.view]
        for root in roots {
            visit(root) { view in
                // 이름 필드는 폭이 있고 토글 열과 겹치지 않아야 합니다.
                if let cell = view as? SidebarLayerCellView,
                   cell.nameField.frame.width <= 0 || cell.nameField.frame.maxX > cell.visibilityButton.frame.minX {
                    overlapping.append(cell.nameField.stringValue)
                }
                // 토글 클릭 대상: HIG 최소 컨트롤 크기 이상이고 행 높이 안에 들어감(디자인 가이드 4.4).
                if let cell = view as? SidebarLayerCellView {
                    for button in [cell.visibilityButton, cell.lockButton] {
                        let size = button.frame.size
                        if size.width < SidebarMetrics.minimumHitTarget || size.height < SidebarMetrics.minimumHitTarget
                            || button.frame.minY < cell.bounds.minY || button.frame.maxY > cell.bounds.maxY {
                            smallTargets.append("\(cell.nameField.stringValue) \(size)")
                        }
                    }
                }
                if view.hasAmbiguousLayout {
                    let owner = view.superview.map { String(describing: type(of: $0)) } ?? "-"
                    let text = (view as? NSTextField).map { " '\($0.stringValue)'" } ?? ""
                    ambiguous.append("\(type(of: view)) in \(owner)\(text)")
                }
                if let field = view as? NSTextField, let color = field.textColor, color.type != .catalog {
                    nonSemantic.append("\(type(of: field)).textColor")
                }
                if let button = view as? NSButton, let color = button.contentTintColor, color.type != .catalog {
                    nonSemantic.append("\(type(of: button)).contentTintColor")
                }
                if let image = view as? NSImageView, let color = image.contentTintColor, color.type != .catalog {
                    nonSemantic.append("\(type(of: image)).contentTintColor")
                }
            }
        }
        try require(ambiguous.isEmpty, "[\(label)] Auto Layout 모호성: \(Set(ambiguous).sorted())")
        try require(nonSemantic.isEmpty, "[\(label)] 시스템 의미 색이 아닌 색: \(Set(nonSemantic).sorted())")
        try require(overlapping.isEmpty, "[\(label)] 이름 필드 폭 없음 또는 토글과 겹침: \(overlapping)")
        try require(smallTargets.isEmpty, "[\(label)] 토글 클릭 대상이 최소 크기보다 작거나 행 밖으로 나감: \(smallTargets)")
        let sidebarWidth = harness.sidebar.view.frame.width
        try require(abs(sidebarWidth - Harness.sidebarWidth) < 1, "[\(label)] 사이드바 폭이 검사 폭과 다름: \(sidebarWidth)")

        // 실제 그리기 경로(외형별 동적 색 해석)를 한 번 통과시킵니다.
        let view = harness.sidebar.view
        if let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: bitmap)
        }
        try require(harness.sidebar.view.frame.width > 0 && harness.sidebar.view.frame.height > 0, "[\(label)] 사이드바 크기가 0")
    }

    @MainActor
    private static func checkAccessibility(_ harness: Harness) throws {
        let layers = harness.layers
        let pages = harness.pages
        try require(AXNode(layers.outline)?.label == SidebarStrings.layersOutline, "레이어 목록 접근성 레이블 불일치")
        try require(AXNode(pages.outline)?.label == SidebarStrings.pagesOutline, "페이지 목록 접근성 레이블 불일치")
        // 필터 필드: 접근성 요소는 검색 필드 cell입니다.
        let searchField = harness.sidebar.filterAccessory.searchField
        try require(AXNode(NSAccessibility.unignoredDescendant(of: searchField))?.label == SidebarStrings.filterLabel,
                    "필터 필드 접근성 레이블이 접근성 요소에 없음")

        // 행: 목록 → AXRow → AXCell 하나. 행 레이블과 custom action은 AXCell에 있고, 장식 아이콘은 노출되지 않습니다.
        for (outline, title) in [(pages.outline, SidebarStrings.pages), (layers.outline, SidebarStrings.layers)] {
            let rows = AXNode.rows(of: outline)
            try require(rows.count == outline.numberOfRows, "[\(title)] AXRow 수 불일치: \(rows.count) != \(outline.numberOfRows)")
            for (index, row) in rows.enumerated() {
                guard let item = outline.item(atRow: index) as? SidebarOutlineItem,
                      let expected = expectedAccessibility(of: item, snapshot: harness.provider.snapshot, headerTitle: title) else {
                    throw Failure(message: "[\(title)] 행 \(index) 항목 없음")
                }
                guard row.children.count == 1, let cell = row.cell else {
                    throw Failure(message: "[\(title)] 행 \(index) AXRow가 AXCell 하나로 묶이지 않음: \(row.children.map(\.role))")
                }
                try require(cell.label == expected.label,
                            "[\(title)] 행 \(index) AXCell 레이블 불일치: \(cell.label ?? "nil") != \(expected.label)")
                try require(cell.customActionNames == expected.actions,
                            "[\(title)] 행 \(index) custom action 불일치: \(cell.customActionNames) != \(expected.actions)")
                try require(cell.descendants(role: .image).isEmpty, "[\(title)] 행 \(index) 장식 아이콘이 AXImage로 노출됨")

                // 토글: 보이는 토글만 노출되고, 레이블은 누르면 일어날 동작입니다.
                guard let id = item.layerID, let view = harness.layerCell(id), let layer = harness.provider.snapshot.layers.rows[id] else {
                    continue
                }
                try require(view.visibilityButton.toolTip != nil && view.lockButton.toolTip != nil, "행 \(index) 토글 도움말 없음")
                let display = layer.name.isEmpty ? SidebarStrings.kindName(layer.kind, shapeKind: layer.shapeKind) : layer.name
                var expectedButtons: [String] = []
                if !view.visibilityButton.isHidden {
                    expectedButtons.append(layer.isVisible ? SidebarStrings.hideLayer(display) : SidebarStrings.showLayer(display))
                }
                if !view.lockButton.isHidden {
                    expectedButtons.append(layer.isLocked ? SidebarStrings.unlockLayer(display) : SidebarStrings.lockLayer(display))
                }
                let buttons = cell.descendants(role: .button).map { $0.label ?? "nil" }
                try require(buttons == expectedButtons, "행 \(index) 토글 접근성 레이블 불일치: \(buttons) != \(expectedButtons)")
            }
        }

        // 상태 표시: 숨김·잠김은 아이콘 모양과 레이블로도 알립니다. 기본 상태의 토글은 마우스를 올리기 전까지 숨김
        // (접근성 트리에서도 빠짐)이며, 같은 동작은 custom action으로 닿습니다.
        let glowID = try harness.id("Glow")
        let backgroundID = try harness.id("Background")
        let dividerID = try harness.id("Divider")
        guard let glow = harness.layerCell(glowID), let background = harness.layerCell(backgroundID),
              let glowAX = harness.axCell(glowID, in: layers.outline), let backgroundAX = harness.axCell(backgroundID, in: layers.outline),
              let dividerAX = harness.axCell(dividerID, in: layers.outline) else {
            throw Failure(message: "예시 숨김·잠김 행 없음")
        }
        try require(!glow.visibilityButton.isHidden && (glowAX.label ?? "").contains(SidebarStrings.hiddenState), "숨김 행 표시 불일치")
        try require(!background.lockButton.isHidden && (backgroundAX.label ?? "").contains(SidebarStrings.lockedState), "잠김 행 표시 불일치")
        try require(dividerAX.descendants(role: .button).isEmpty, "숨긴 토글이 접근성 트리에 남음")

        // 이름이 빈 행은 종류 이름을 placeholder와 레이블로 씁니다.
        guard let unnamed = harness.provider.nodeID(named: ""), let cell = harness.layerCell(unnamed),
              let unnamedAX = harness.axCell(unnamed, in: layers.outline) else {
            throw Failure(message: "빈 이름 예시 행 없음")
        }
        try require(cell.nameField.placeholderString == SidebarStrings.kindName(.text, shapeKind: nil), "빈 이름 placeholder 불일치")
        try require((unnamedAX.label ?? "").hasPrefix(SidebarStrings.kindName(.text, shapeKind: nil)), "빈 이름 레이블 불일치")
    }

    // MARK: 동작

    @MainActor
    private static func checkRename(_ harness: Harness, headerID: SidebarStableID) throws {
        let layers = harness.layers
        let outline = layers.outline
        let window = harness.window
        try require(window.makeFirstResponder(outline), "레이어 목록 first responder 실패")
        outline.selectRowIndexes(IndexSet(integer: harness.row(of: headerID, in: outline)), byExtendingSelection: false)
        harness.refresh()

        // Return → 편집 시작 → 값 변경 후 포커스 이동으로 확정.
        outline.keyDown(with: harness.key("\r", keyCode: 36))
        try require(SidebarInlineRename.isEditing(in: outline), "Return으로 이름 편집이 시작되지 않음")
        guard let editor = window.firstResponder as? NSTextView else { throw Failure(message: "field editor 없음") }
        var mark = harness.provider.performedCommands.count
        replaceText(in: editor, with: "Hero")
        try require(window.makeFirstResponder(outline), "편집 종료 실패")
        try require(harness.commands(since: mark) == [.renameNode(headerID, name: "Hero")],
                    "이름 변경 command 불일치: \(harness.commands(since: mark))")
        harness.refresh()
        try require(harness.provider.snapshot.layers.rows[headerID]?.name == "Hero", "이름 변경이 반영되지 않음")
        try require(harness.layerCell(headerID)?.nameField.stringValue == "Hero", "이름 변경 후 행 값 불일치")

        // Escape → 취소: command 없음, 원래 값 복원.
        mark = harness.provider.performedCommands.count
        try require(layers.beginRename(headerID), "이름 편집 시작 실패")
        guard let editor2 = window.firstResponder as? NSTextView else { throw Failure(message: "field editor 없음") }
        replaceText(in: editor2, with: "Discard")
        editor2.doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        try require(!SidebarInlineRename.isEditing(in: outline), "Escape로 편집이 끝나지 않음")
        try require(harness.commands(since: mark).isEmpty, "Escape 취소가 command를 만듦: \(harness.commands(since: mark))")
        try require(harness.layerCell(headerID)?.nameField.stringValue == "Hero", "Escape 후 값이 복원되지 않음")
        try require(window.firstResponder === outline, "Escape 후 포커스가 목록으로 돌아오지 않음")

        // 값을 바꾸지 않고 끝내면 command가 없습니다.
        try require(layers.beginRename(headerID), "이름 편집 시작 실패")
        try require(window.makeFirstResponder(outline), "편집 종료 실패")
        try require(harness.commands(since: mark).isEmpty, "무변경 편집이 command를 만듦")

        // 편집 중 도착한 snapshot은 편집이 끝난 뒤 반영합니다.
        try require(layers.beginRename(headerID), "이름 편집 시작 실패")
        let titleID = try harness.id("Title")
        harness.provider.perform(.setVisible([titleID], false))
        harness.refresh()
        try require(SidebarInlineRename.isEditing(in: outline), "snapshot 반영이 편집을 끊음")
        try require(layers.snapshot.layers.rows[titleID]?.isVisible == true, "편집 중 snapshot이 바로 반영됨")
        try require(window.makeFirstResponder(outline), "편집 종료 실패")
        // 미뤄 둔 snapshot은 편집 종료 다음 런 루프에서 반영됩니다(행을 종료 처리 도중에 다시 만들지 않음).
        spinRunLoop(until: { layers.snapshot.layers.rows[titleID]?.isVisible == false })
        try require(layers.snapshot.layers.rows[titleID]?.isVisible == false, "편집 뒤 미뤄 둔 snapshot이 반영되지 않음")
        harness.provider.perform(.setVisible([titleID], true))
        harness.refresh()
    }

    @MainActor
    private static func checkToggles(_ harness: Harness) throws {
        let dividerID = try harness.id("Divider")
        guard let cell = harness.layerCell(dividerID) else { throw Failure(message: "Divider 행 없음") }
        let structure = harness.provider.snapshot.layers.structureRevision
        let mark = harness.provider.performedCommands.count
        cell.visibilityButton.performClick(nil)
        cell.lockButton.performClick(nil)
        try require(harness.commands(since: mark) == [.setVisible([dividerID], false), .setLocked([dividerID], true)],
                    "토글 command 불일치: \(harness.commands(since: mark))")
        try require(harness.provider.snapshot.layers.structureRevision == structure, "토글이 구조 변경으로 처리됨")
        harness.refresh()
        // 구조가 같으면 같은 셀을 제자리에서 다시 채웁니다.
        try require(harness.layerCell(dividerID) === cell, "값만 바뀌었는데 셀이 교체됨")
        let axLabel = harness.axCell(dividerID, in: harness.layers.outline)?.label ?? ""
        try require(axLabel.contains(SidebarStrings.hiddenState), "숨김 반영 실패: \(axLabel)")
        try require(axLabel.contains(SidebarStrings.lockedState), "잠금 반영 실패: \(axLabel)")
        let buttonLabels = harness.axCell(dividerID, in: harness.layers.outline)?.descendants(role: .button).map { $0.label ?? "" } ?? []
        try require(buttonLabels.first?.hasPrefix(SidebarStrings.show) ?? false, "토글 레이블이 다음 동작을 말하지 않음: \(buttonLabels)")
        harness.provider.perform(.setVisible([dividerID], true))
        harness.provider.perform(.setLocked([dividerID], false))
        harness.refresh()

        // VoiceOver custom action: 접근성 서버가 실행하는 방식(AXCell의 이름 있는 action)으로 실행합니다.
        // 행 하나에만 적용하고 선택을 바꾸지 않습니다.
        let layers = harness.layers
        let selectionBefore = layers.outline.selectedRowIndexes
        var mark2 = harness.provider.performedCommands.count
        guard let dividerAX = harness.axCell(dividerID, in: layers.outline) else { throw Failure(message: "Divider AXCell 없음") }
        try require(dividerAX.performCustomAction(named: SidebarStrings.hide), "Hide custom action을 실행하지 못함")
        harness.refresh()
        guard let hiddenAX = harness.axCell(dividerID, in: layers.outline) else { throw Failure(message: "Divider AXCell 없음") }
        try require(hiddenAX.customActionNames.contains(SidebarStrings.show), "숨긴 뒤 custom action이 Show로 바뀌지 않음: \(hiddenAX.customActionNames)")
        try require(hiddenAX.performCustomAction(named: SidebarStrings.lock), "Lock custom action을 실행하지 못함")
        try require(harness.commands(since: mark2) == [.setVisible([dividerID], false), .setLocked([dividerID], true)],
                    "custom action command 불일치: \(harness.commands(since: mark2))")
        try require(layers.outline.selectedRowIndexes == selectionBefore, "custom action이 선택을 바꿈")
        harness.provider.perform(.setVisible([dividerID], true))
        harness.provider.perform(.setLocked([dividerID], false))
        harness.refresh()

        // Rename custom action은 인라인 편집을 시작합니다(취소 시 command 없음).
        mark2 = harness.provider.performedCommands.count
        guard let renameAX = harness.axCell(dividerID, in: layers.outline) else { throw Failure(message: "Divider AXCell 없음") }
        try require(renameAX.performCustomAction(named: SidebarStrings.rename) && SidebarInlineRename.isEditing(in: layers.outline),
                    "Rename custom action이 편집을 시작하지 않음")
        guard let editor = harness.window.firstResponder as? NSTextView else { throw Failure(message: "field editor 없음") }
        editor.doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        try require(!SidebarInlineRename.isEditing(in: layers.outline), "Rename custom action 편집이 취소되지 않음")
        try require(harness.commands(since: mark2).filter { if case .renameNode = $0 { return true }; return false }.isEmpty,
                    "취소한 Rename custom action이 command를 만듦")
        harness.refresh()

        // 페이지·View 행의 Rename custom action.
        guard let welcomeID = harness.provider.viewID(named: "Welcome"),
              let welcomeAX = harness.axCell(welcomeID, in: harness.pages.outline) else {
            throw Failure(message: "Welcome AXCell 없음")
        }
        try require(welcomeAX.performCustomAction(named: SidebarStrings.rename) && SidebarInlineRename.isEditing(in: harness.pages.outline),
                    "View 행 Rename custom action이 편집을 시작하지 않음")
        guard let pageEditor = harness.window.firstResponder as? NSTextView else { throw Failure(message: "field editor 없음") }
        pageEditor.doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        try require(!SidebarInlineRename.isEditing(in: harness.pages.outline), "View 행 Rename 편집이 취소되지 않음")
        harness.refresh()
    }

    @MainActor
    private static func checkMenuAndShortcuts(_ harness: Harness) throws {
        let layers = harness.layers
        let outline = layers.outline
        let iconsID = try harness.id("Icons")
        let starID = try harness.id("Star")
        let dividerID = try harness.id("Divider")

        var shortcutTitles: [String] = []
        func menuState(_ targets: [SidebarStableID]) -> [String: Bool] {
            let menu = NSMenu()
            layers.fill(menu, targets: targets)
            var state: [String: Bool] = [:]
            for item in menu.items where !item.isSeparatorItem {
                state[item.title] = item.isEnabled
                if !item.keyEquivalent.isEmpty { shortcutTitles.append(item.title) }
            }
            return state
        }

        let fragmentMenu = menuState([iconsID])
        try require(fragmentMenu.count == 7, "문맥 메뉴 항목 수 불일치: \(fragmentMenu)")
        // 문맥 메뉴에는 단축키 표시가 없습니다(HIG Context menus). 단축키는 메뉴 막대가 보입니다.
        try require(shortcutTitles.isEmpty, "문맥 메뉴에 단축키 표시가 있음: \(shortcutTitles)")
        try require(fragmentMenu[SidebarStrings.ungroup] == true, "그룹 marker에서 Ungroup이 비활성")
        try require(fragmentMenu[SidebarStrings.rename] == true, "단일 대상 Rename이 비활성")
        let mixedMenu = menuState([starID, dividerID])
        try require(mixedMenu[SidebarStrings.group] == false, "부모가 다른 행의 Group이 활성")
        try require(mixedMenu[SidebarStrings.rename] == false, "다중 대상 Rename이 활성")
        try require(mixedMenu[SidebarStrings.ungroup] == false, "그룹 marker 없는 Ungroup이 활성")

        // 키보드: ⌘D 복제, ⌥⌘G 묶기, ⇧⌥⌘G 풀기, ⇧⌘H 숨기기, Delete 삭제.
        try require(harness.window.makeFirstResponder(outline), "레이어 목록 first responder 실패")
        let signatureRow = harness.row(of: dividerID, in: outline)
        outline.selectRowIndexes(IndexSet(integer: signatureRow), byExtendingSelection: false)
        harness.refresh()

        var mark = harness.provider.performedCommands.count
        try require(outline.performKeyEquivalent(with: harness.key("d", keyCode: 2, modifiers: [.command])), "⌘D 미처리")
        try require(harness.commands(since: mark) == [.duplicate([dividerID])], "⌘D command 불일치: \(harness.commands(since: mark))")
        harness.refresh()
        let copies = harness.provider.snapshot.selection
        try require(copies.count == 1 && copies[0] != dividerID, "복제본이 선택되지 않음")

        mark = harness.provider.performedCommands.count
        try require(outline.performKeyEquivalent(with: harness.key("g", keyCode: 5, modifiers: [.command, .option])), "⌥⌘G 미처리")
        try require(harness.commands(since: mark) == [.group(copies)], "⌥⌘G command 불일치")
        harness.refresh()
        let groupID = harness.provider.snapshot.selection.first
        try require(groupID.flatMap { harness.provider.snapshot.layers.rows[$0]?.kind } == .fragment, "묶기 결과가 Fragment가 아님")

        mark = harness.provider.performedCommands.count
        try require(outline.performKeyEquivalent(with: harness.key("G", keyCode: 5, modifiers: [.command, .option, .shift])), "⇧⌥⌘G 미처리")
        try require(harness.commands(since: mark) == [.ungroup([groupID!])], "⇧⌥⌘G command 불일치")
        harness.refresh()
        try require(harness.provider.snapshot.selection == copies, "풀기 후 선택 불일치")

        mark = harness.provider.performedCommands.count
        try require(outline.performKeyEquivalent(with: harness.key("H", keyCode: 4, modifiers: [.command, .shift])), "⇧⌘H 미처리")
        try require(harness.commands(since: mark) == [.setVisible(copies, false)], "⇧⌘H command 불일치")
        harness.refresh()

        mark = harness.provider.performedCommands.count
        outline.keyDown(with: harness.key("\u{7F}", keyCode: 51))
        try require(harness.commands(since: mark) == [.delete(copies)], "Delete command 불일치: \(harness.commands(since: mark))")
        harness.refresh()
        try require(harness.provider.snapshot.layers.rows[copies[0]] == nil, "삭제가 반영되지 않음")

        // 선택이 없으면 동작하지 않습니다(키는 기본 처리로 넘어감).
        outline.deselectAll(nil)
        harness.refresh()
        mark = harness.provider.performedCommands.count
        try require(!outline.performKeyEquivalent(with: harness.key("d", keyCode: 2, modifiers: [.command])), "선택 없이 ⌘D 처리")
        try require(harness.commands(since: mark).filter { if case .duplicate = $0 { return true }; return false }.isEmpty,
                    "선택 없이 복제 command 생성")

        // 메뉴 막대 연결용 responder action 검증.
        let deleteItem = NSMenuItem(title: SidebarStrings.delete, action: #selector(SidebarOutlineView.delete(_:)), keyEquivalent: "")
        try require(!outline.validateUserInterfaceItem(deleteItem), "선택 없이 Delete 메뉴가 활성")
    }

    @MainActor
    private static func checkDrop(_ harness: Harness) throws {
        let layers = harness.layers
        let headerID = try harness.id("Hero")
        let titleID = try harness.id("Title")
        let logoID = try harness.id("Logo")
        let backgroundID = try harness.id("Background")
        let footerID = try harness.id("Footer")
        let starID = try harness.id("Star")

        func item(_ id: SidebarStableID) -> SidebarOutlineItem? {
            let row = harness.row(of: id, in: layers.outline)
            return row >= 0 ? layers.outline.item(atRow: row) as? SidebarOutlineItem : nil
        }

        // 펼쳐서 드롭 대상 행을 만듭니다.
        for id in [headerID, footerID, try harness.id("Icons")] {
            if let item = item(id) { layers.outline.expandItem(item) }
        }
        harness.refresh()
        guard let titleItem = item(titleID), let backgroundItem = item(backgroundID), let footerItem = item(footerID) else {
            throw Failure(message: "드롭 대상 행 없음")
        }

        // 자기 자손 위: 위에 놓기·형제 자리 모두 거부(ReparentError::DescendantParent와 같은 규칙).
        try require(layers.resolveDrop(ids: [headerID], proposedItem: titleItem, childIndex: -1) == nil, "자손으로 이동을 허용함")
        try require(harness.provider.validate(.move([headerID], to: SidebarDropTarget(parentID: titleID, index: 0)))
                        == .denied(.descendantParent), "자손 이동 거부 사유 불일치")

        // 자식을 못 가지는 Shape 위: 그 행 바로 위 형제 자리로 재지정.
        guard let retarget = layers.resolveDrop(ids: [logoID], proposedItem: backgroundItem, childIndex: -1) else {
            throw Failure(message: "Shape 위 드롭 재지정 실패")
        }
        let topLevel = harness.provider.snapshot.layers.topLevelIDs
        try require(retarget.target == SidebarDropTarget(parentID: nil, index: topLevel.firstIndex(of: backgroundID) ?? -1),
                    "재지정 위치 불일치: \(retarget.target)")

        // 컨테이너 위: 맨 위 자식 자리.
        guard let into = layers.resolveDrop(ids: [starID], proposedItem: footerItem, childIndex: -1) else {
            throw Failure(message: "Frame 위 드롭 거부")
        }
        try require(into.target == SidebarDropTarget(parentID: footerID, index: 0), "컨테이너 드롭 위치 불일치")

        // 제목 행 아래(outline 최상위 끝): 레이어 최상위 끝.
        guard let end = layers.resolveDrop(ids: [starID], proposedItem: nil, childIndex: 1) else {
            throw Failure(message: "최상위 끝 드롭 거부")
        }
        try require(end.target == SidebarDropTarget(parentID: nil, index: topLevel.count), "최상위 끝 위치 불일치")

        // 적용: 드롭 결과 command가 그대로 적용되고 구조가 바뀝니다.
        harness.provider.perform(.move([starID], to: into.target))
        harness.refresh()
        try require(harness.provider.snapshot.layers.childIDs[footerID]?.first == starID, "이동이 반영되지 않음")
        try require(harness.row(of: starID, in: layers.outline) >= 0, "이동 후 행이 보이지 않음(펼침 복원 실패)")
    }

    @MainActor
    private static func checkFilter(_ harness: Harness) throws {
        let layers = harness.layers
        let sidebar = harness.sidebar
        let accessory = sidebar.filterAccessory
        let expandedBefore = layers.expandedIDs
        let rowsBefore = layers.outline.numberOfRows

        // 질의 → command → 필터 결과(일치 행 + 조상, 모두 펼침).
        var mark = harness.provider.performedCommands.count
        accessory.searchField.stringValue = "Privacy"
        accessory.searchField.sendAction(accessory.searchField.action, to: accessory.searchField.target)
        try require(harness.commands(since: mark) == [.setFilter("Privacy")], "필터 command 불일치")
        harness.refresh()
        try require(layers.outline.numberOfRows == 4, "필터 결과 행 수 불일치: \(layers.outline.numberOfRows)")
        try require(sidebar.stateView.isHidden, "필터 결과가 있는데 상태 화면이 보임")
        let footerID = try harness.id("Footer")
        let footerRow = harness.row(of: footerID, in: layers.outline)
        guard footerRow >= 0, let footerItem = layers.outline.item(atRow: footerRow),
              let footerAX = harness.axCell(footerID, in: layers.outline) else {
            throw Failure(message: "필터 문맥 행 없음")
        }
        try require((footerAX.label ?? "").contains(SidebarStrings.contextOnly), "필터 문맥 행 레이블 없음: \(footerAX.label ?? "nil")")
        try require(layers.outlineView(layers.outline, pasteboardWriterForItem: footerItem) == nil, "필터 중 드래그를 허용함")

        // 결과 없음 → "Clear Filter" → 필터 해제와 펼침 복원.
        accessory.searchField.stringValue = "zzz"
        accessory.searchField.sendAction(accessory.searchField.action, to: accessory.searchField.target)
        harness.refresh()
        try require(sidebar.stateView.content == .noResults(query: "zzz") && !sidebar.stateView.isHidden, "결과 없음 상태 불일치")
        try checkLayout(harness, label: "Light, no results", appearance: .aqua)
        try require(sidebar.stateView.actionButton.title == SidebarStrings.clearFilter, "결과 없음 버튼 불일치")
        mark = harness.provider.performedCommands.count
        sidebar.stateView.actionButton.performClick(nil)
        try require(harness.commands(since: mark) == [.setFilter("")], "필터 지우기 command 불일치")
        harness.refresh()
        try require(accessory.searchField.stringValue.isEmpty, "필터 필드가 비워지지 않음")
        try require(layers.expandedIDs == expandedBefore, "필터 해제 후 펼침 상태가 복원되지 않음")
        try require(layers.outline.numberOfRows == rowsBefore, "필터 해제 후 행 수 불일치: \(layers.outline.numberOfRows) != \(rowsBefore)")
    }

    // MARK: 지연 로드·병합

    @MainActor
    private static func checkLazyChildren() throws {
        let harness = Harness(provider: SidebarStubDataProvider(loadsChildrenLazily: true))
        defer { harness.close() }
        let layers = harness.layers
        let headerID = try harness.id("Header")
        try require(harness.provider.snapshot.layers.childIDs[headerID] == nil, "지연 모드인데 자식이 미리 로드됨")
        let row = harness.row(of: headerID, in: layers.outline)
        guard row >= 0, let item = layers.outline.item(atRow: row) else { throw Failure(message: "지연 모드 행 없음") }
        try require(layers.outline.isExpandable(item), "자식 미로드 행이 펼침 표시를 잃음")
        let mark = harness.provider.performedCommands.count
        layers.outline.expandItem(item)
        try require(harness.commands(since: mark) == [.loadChildren(headerID)], "펼침 시 자식 요청 불일치")
        harness.refresh()
        try require(layers.outline.isItemExpanded(item), "자식 로드 후 펼침이 유지되지 않음")
        try require(harness.row(of: try harness.id("Title"), in: layers.outline) > row, "로드된 자식이 보이지 않음")
    }

    /// 보이는 범위 밖에 만들어 둔 행 뷰(NSTableView가 스크롤 준비로 미리 만드는 행)도 값만 바뀐 snapshot과
    /// 대비 높이기 변경을 반영해야 합니다. 그 행은 화면에 들어올 때 `viewFor`를 다시 거치지 않습니다.
    @MainActor
    private static func checkAvailableRows() throws {
        let harness = Harness(provider: SidebarStubDataProvider())
        defer { harness.close() }
        let layers = harness.layers
        let outline = layers.outline
        // 모두 펼쳐 행이 레이어 구획 높이를 넘게 합니다.
        outline.expandItem(nil, expandChildren: true)
        harness.refresh()
        outline.scrollRowToVisible(0)
        harness.refresh()
        let visible = outline.rows(in: outline.visibleRect)
        let last = outline.numberOfRows - 1
        try require(last >= visible.location + visible.length, "검사 전제: 보이는 범위 밖 행 없음(보임 \(visible), 행 \(outline.numberOfRows))")
        guard let id = (outline.item(atRow: last) as? SidebarOutlineItem)?.layerID,
              let cell = outline.view(atColumn: 0, row: last, makeIfNecessary: true) as? SidebarLayerCellView else {
            throw Failure(message: "보이는 범위 밖 행 셀을 만들지 못함")
        }
        var available: [Int] = []
        outline.enumerateAvailableRowViews { _, row in available.append(row) }
        try require(available.contains(last), "검사 전제: 보이는 범위 밖 행 뷰가 목록에 남지 않음(\(available))")

        // 값만 바뀐 snapshot(구조 같음): 보이는 범위 밖 행도 제자리에서 다시 채웁니다.
        let structure = harness.provider.snapshot.layers.structureRevision
        harness.provider.perform(.setVisible([id], false))
        try require(harness.provider.snapshot.layers.structureRevision == structure, "검사 전제: 값 변경이 구조 변경으로 처리됨")
        harness.sidebar.applySnapshotNow()
        try require(outline.view(atColumn: 0, row: last, makeIfNecessary: false) === cell, "검사 전제: 보이는 범위 밖 행 셀이 교체됨")
        try require((cell.accessibilityLabel() ?? "").contains(SidebarStrings.hiddenState),
                    "보이는 범위 밖 행이 값 변경을 반영하지 않음: \(cell.accessibilityLabel() ?? "nil")")

        // 대비 높이기: 보이는 범위 밖 행도 토글을 항상 보입니다.
        try require(cell.lockButton.isHidden, "검사 전제: 기본 상태 잠금 토글이 보임")
        layers.showsTogglesAlways = true
        try require(cell.showsTogglesAlways && !cell.lockButton.isHidden, "보이는 범위 밖 행에 대비 높이기가 반영되지 않음")
        layers.showsTogglesAlways = false
        try require(cell.lockButton.isHidden, "대비 높이기 해제가 보이는 범위 밖 행에 반영되지 않음")
    }

    @MainActor
    private static func checkCoalescing() throws {
        let harness = Harness(provider: SidebarStubDataProvider())
        defer { harness.close() }
        let start = harness.sidebar.appliedGeneration
        let titleID = try harness.id("Title")
        harness.provider.perform(.setVisible([titleID], false))
        harness.provider.perform(.setLocked([titleID], true))
        harness.provider.perform(.renameNode(titleID, name: "Heading"))
        try require(harness.sidebar.appliedGeneration == start, "변경 통지가 즉시 반영됨(병합되지 않음)")
        spinRunLoop(until: { harness.sidebar.appliedGeneration == harness.provider.snapshot.generation })
        try require(harness.sidebar.appliedGeneration == harness.provider.snapshot.generation, "다음 런 루프에서 반영되지 않음")
        try require(harness.layers.snapshot.layers.rows[titleID]?.name == "Heading", "병합 반영 값 불일치")
    }

    // MARK: 접근성 서버 경로

    /// 접근성 서버(VoiceOver가 질의하는 쪽)가 보는 요소 하나.
    ///
    /// AppKit 접근성 서버는 NSAccessibility 프로토콜을 쓰는 객체(`NSView`, `NSCell`)에는 프로토콜 메서드로 묻고,
    /// 그렇지 않은 객체에는 legacy attribute 질의(`accessibilityAttributeValue:`)로 묻습니다. view-based 표의 행
    /// (`AXRow`)과 셀(`AXCell`)은 후자에 속하는 AppKit 내부 객체입니다. 이 타입은 같은 규칙으로 읽으므로, setter에 넣은
    /// 값이 아니라 실제로 노출되는 값을 확인합니다. legacy 메서드는 구현 쪽 API로는 deprecated라 selector로 부릅니다.
    @MainActor
    struct AXNode {
        let object: NSObject

        private static let roleSelector = NSSelectorFromString("accessibilityRole")
        private static let attributeSelector = NSSelectorFromString("accessibilityAttributeValue:")
        private static let actionNamesSelector = NSSelectorFromString("accessibilityActionNames")
        private static let performActionSelector = NSSelectorFromString("accessibilityPerformAction:")
        private static let customActionsSelector = NSSelectorFromString("accessibilityCustomActions")
        private static let selectedRowsSelector = NSSelectorFromString("accessibilitySelectedRows")

        init?(_ value: Any?) {
            guard let object = value as? NSObject else { return nil }
            self.object = object
        }

        /// 프로토콜 메서드로 답하는 요소인지(아니면 legacy attribute 질의).
        private var usesProtocol: Bool { object.responds(to: Self.roleSelector) }

        private func call(_ selector: Selector) -> Any? {
            guard object.responds(to: selector) else { return nil }
            return object.perform(selector)?.takeUnretainedValue()
        }

        private func legacy(_ attribute: String) -> Any? {
            guard object.responds(to: Self.attributeSelector) else { return nil }
            return object.perform(Self.attributeSelector, with: attribute as NSString)?.takeUnretainedValue()
        }

        var role: String? {
            if usesProtocol { return ((object as AnyObject).accessibilityRole?() ?? nil)?.rawValue }
            return legacy(NSAccessibility.Attribute.role.rawValue) as? String
        }

        /// 접근성 설명(VoiceOver가 읽는 레이블, AXDescription).
        var label: String? {
            if usesProtocol { return (object as AnyObject).accessibilityLabel?() ?? nil }
            return legacy(NSAccessibility.Attribute.description.rawValue) as? String
        }

        var children: [AXNode] {
            if usesProtocol {
                let children = ((object as AnyObject).accessibilityChildren?() ?? nil) ?? []
                return NSAccessibility.unignoredChildren(from: children).compactMap(AXNode.init)
            }
            return ((legacy(NSAccessibility.Attribute.children.rawValue) as? [Any]) ?? []).compactMap(AXNode.init)
        }

        var descendants: [AXNode] { children.flatMap { [$0] + $0.descendants } }

        func children(role: NSAccessibility.Role) -> [AXNode] { children.filter { $0.role == role.rawValue } }

        func descendants(role: NSAccessibility.Role) -> [AXNode] { descendants.filter { $0.role == role.rawValue } }

        /// 행 안의 유일한 `AXCell`.
        var cell: AXNode? {
            let cells = children(role: .cell)
            return cells.count == 1 ? cells[0] : nil
        }

        /// 이 요소가 내보내는 custom action 이름(VoiceOver 동작 메뉴 항목).
        var customActionNames: [String] {
            ((call(Self.customActionsSelector) as? [NSAccessibilityCustomAction]) ?? []).map(\.name)
        }

        /// 접근성 서버가 custom action을 실행하는 방식(이름이 담긴 AX action 실행)으로 실행합니다.
        @discardableResult
        func performCustomAction(named name: String) -> Bool {
            guard let names = call(Self.actionNamesSelector) as? [String],
                  let action = names.first(where: { $0.hasPrefix("Name:\(name)\n") }),
                  object.responds(to: Self.performActionSelector) else {
                return false
            }
            _ = object.perform(Self.performActionSelector, with: action as NSString)
            return true
        }

        /// 목록의 `AXRow` 요소(표시 순서).
        static func rows(of outline: NSOutlineView) -> [AXNode] {
            AXNode(outline)?.children(role: .row) ?? []
        }

        /// 목록이 선택 행으로 내보내는 `AXRow` 요소.
        static func selectedRows(of outline: NSOutlineView) -> [AXNode] {
            ((AXNode(outline)?.call(selectedRowsSelector) as? [Any]) ?? []).compactMap(AXNode.init)
        }
    }

    /// 행 항목이 내보내야 할 `AXCell` 레이블과 custom action(명세 기준으로 따로 계산).
    @MainActor
    private static func expectedAccessibility(
        of item: SidebarOutlineItem,
        snapshot: SidebarSnapshot,
        headerTitle: String
    ) -> (label: String, actions: [String])? {
        switch item.role {
        case .header:
            return (headerTitle, [])
        case .page:
            guard let page = snapshot.pages.first(where: { $0.id == item.id }) else { return nil }
            let name = page.name.isEmpty ? SidebarStrings.pageKind : page.name
            return ("\(name), \(SidebarStrings.pageKind)", [SidebarStrings.rename])
        case .view:
            guard let view = snapshot.pages.flatMap(\.views).first(where: { $0.id == item.id }) else { return nil }
            let name = view.name.isEmpty ? SidebarStrings.viewKind : view.name
            return ("\(name), \(SidebarStrings.viewKind)", [SidebarStrings.rename])
        case .layer:
            guard let id = item.layerID, let row = snapshot.layers.rows[id] else { return nil }
            let kind = SidebarStrings.kindName(row.kind, shapeKind: row.shapeKind)
            var parts = [row.name.isEmpty ? kind : row.name, kind]
            if !row.isVisible { parts.append(SidebarStrings.hiddenState) }
            if row.isLocked { parts.append(SidebarStrings.lockedState) }
            if !row.isFilterMatch { parts.append(SidebarStrings.contextOnly) }
            return (parts.joined(separator: ", "), [
                SidebarStrings.rename,
                row.isVisible ? SidebarStrings.hide : SidebarStrings.show,
                row.isLocked ? SidebarStrings.unlock : SidebarStrings.lock,
                SidebarStrings.duplicate,
                SidebarStrings.delete,
            ])
        }
    }

    // MARK: 보조

    /// 조건이 참이 되거나 1초가 지날 때까지 default mode 런 루프를 돌립니다.
    @MainActor
    private static func spinRunLoop(until condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(1)
        while !condition(), Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.01))
        }
    }

    /// 사용자 입력과 같은 경로(insertText)로 field editor 내용을 바꿉니다.
    @MainActor
    private static func replaceText(in editor: NSTextView, with text: String) {
        editor.selectAll(nil)
        editor.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    @MainActor
    private static func visit(_ view: NSView, _ body: (NSView) -> Void) {
        body(view)
        for subview in view.subviews { visit(subview, body) }
    }

    /// `root` 아래에서 `T` 뷰를 찾아 superview 경로(클래스 이름)를 반환합니다.
    @MainActor
    private static func paths<T: NSView>(of type: T.Type, in root: NSView) -> [String] {
        var found: [String] = []
        visit(root) { view in
            guard view is T else { return }
            var chain: [String] = []
            var current: NSView? = view
            while let node = current, node !== root.superview {
                chain.append(String(describing: Swift.type(of: node)))
                current = node.superview
            }
            found.append(chain.reversed().joined(separator: " > "))
        }
        return found
    }
}
