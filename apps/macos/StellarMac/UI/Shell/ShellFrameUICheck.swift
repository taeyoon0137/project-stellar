import AppKit

/// 셸 프레임(메인 창 구성)을 화면에 띄우지 않고 검사합니다.
///
/// stub data source와 stub 슬롯으로 `ShellWindowController`를 만들고 일반 크기로 layout한 뒤
/// - 창 크롬: full-size content, 툴바 스타일, 시스템 툴바 항목, 가운데 도구 영역
/// - 분할 구조: 사이드바·인스펙터 behavior, 폭 범위, 창 폭 변경 시 캔버스만 변함, 캔버스가 safe area 안
/// - Auto Layout 모호성 없음(빈·로딩·오류 상태 포함)
/// - 접근성 레이블: pane, 캔버스, 툴바 항목, 도구, 검색, 상태 화면 / SF Symbol 이름 유효
/// - 명령: 응답자 체인으로 사이드바·인스펙터·보조 패널 전환(동작 줄이기·애니메이션 경로), 메뉴 제목
/// - snapshot: 다음 런 루프 병합, 오래된 세대 폐기, 내비게이션·도구·검색·더 보기 명령 전달
/// - AppKit 상태 저장: frame·split autosave 왕복, 창 복원 설정
/// 을 확인합니다. 검사용 autosave 이름은 실행마다 새로 만들고, 시작과 끝에 검사용 기록을 user defaults에서 지웁니다.
enum ShellFrameUICheck {
    static func run() -> Bool {
        MainActor.assumeIsolated {
            ShellFrameUICheckRunner().run()
        }
    }
}

@MainActor
private final class ShellFrameUICheckRunner {
    /// 검사용 autosave 이름 접두어. 앱의 실제 창 이름과 겹치지 않습니다.
    private static let namePrefix = "StellarShellFrameUICheck"
    private var failures: [String] = []
    private var checks = 0
    private let token = UUID().uuidString
    private lazy var configuration = makeConfiguration("Main")
    /// autosave 왕복 검사용. 앞 검사의 창이 늦게 기록하는 상태와 섞이지 않도록 이름을 따로 씁니다.
    private lazy var roundTripConfiguration = makeConfiguration("RoundTrip")

    private func makeConfiguration(_ label: String) -> ShellWindowConfiguration {
        ShellWindowConfiguration(
            autosaveName: "\(Self.namePrefix).\(label).\(token)",
            toolbarIdentifier: "\(Self.namePrefix).Toolbar.\(label).\(token)",
            restorationIdentifier: "\(Self.namePrefix).\(label).\(token)"
        )
    }

    func run() -> Bool {
        // 창을 만들려면 NSApplication이 필요합니다. Dock 아이콘은 띄우지 않습니다.
        NSApplication.shared.setActivationPolicy(.accessory)
        removeAutosavedState()
        defer { removeAutosavedState() }

        checkSymbols()
        autoreleasepool {
            checkMainWindow()
        }
        checkAutosaveRoundTrip()
        checkRestorationEntry()

        if failures.isEmpty {
            print("shell-frame ok: \(checks) checks (window chrome, split, layout, accessibility, commands, snapshot, autosave)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("shell-frame 실패: \(failure)\n".utf8))
        }
        return false
    }

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    // MARK: - 개별 검사

    private func checkSymbols() {
        let names = ShellFrameSymbols.all + ShellFrameSlots.stubSymbolNames
            + ShellFrameDataSourceStub.sampleTools.map(\.symbolName)
        for name in names {
            expect(NSImage(systemSymbolName: name, accessibilityDescription: nil) != nil, "SF Symbol 없음: \(name)")
        }
    }

    private func makeController(
        _ stub: ShellFrameDataSourceStub,
        configuration: ShellWindowConfiguration? = nil
    ) -> ShellWindowController {
        let controller = ShellWindowController(
            configuration: configuration ?? self.configuration,
            dataSource: stub,
            slots: .stub(runtimeHandle: nil)
        )
        controller.window?.isReleasedWhenClosed = false
        return controller
    }

    private func layout(_ window: NSWindow) {
        window.contentView?.layoutSubtreeIfNeeded()
        window.layoutIfNeeded()
    }

    private func checkMainWindow() {
        let stub = ShellFrameDataSourceStub()
        let controller = makeController(stub)
        guard let window = controller.window, let toolbar = window.toolbar else {
            expect(false, "창 또는 툴바 없음")
            return
        }
        defer { window.close() }
        let split = controller.splitController
        split.prefersReducedMotion = { true }
        window.setContentSize(ShellFrameMetrics.initialContentSize)
        layout(window)

        // 창 크롬(가이드 2.1).
        expect(window.styleMask.contains(.fullSizeContentView), "full-size content view 아님")
        expect(window.styleMask.isSuperset(of: [.titled, .closable, .miniaturizable, .resizable]), "표준 창 스타일 아님")
        expect(window.toolbarStyle == .automatic, "툴바 스타일이 automatic이 아님(O1 전)")
        expect(window.title == "Sample Project" && window.subtitle == "Page 2", "창 제목·부제목 미반영: \(window.title)")
        let identifiers = toolbar.items.map(\.itemIdentifier)
        for required in ShellToolbarController.defaultItemIdentifiers where required != .flexibleSpace {
            expect(identifiers.contains(required), "툴바 항목 없음: \(required.rawValue)")
        }
        expect(
            !identifiers.contains(NSToolbarItem.Identifier("stellar.shell.toolArea")) && toolbar.centeredItemIdentifiers.isEmpty,
            "상단 툴바에 도구 영역이 남음")
        expect(window.titlebarSeparatorStyle == .none, "상단 바 구분선이 남음")
        expect(split.splitViewItems.allSatisfy { $0.titlebarSeparatorStyle == .none }, "분할 항목 상단 바 구분선이 남음")
        let contentHost = split.contentContainer.view
        let toolBar = controller.toolbarController.toolBar
        expect(toolBar.superview === contentHost, "도구 막대가 캔버스 영역에 없음")
        expect(
            abs(toolBar.frame.midX - contentHost.bounds.midX) < 1
                && abs(toolBar.frame.minY - (contentHost.safeAreaInsets.bottom + ShellToolBarView.Metrics.bottomInset)) < 1,
            "도구 막대가 캔버스 하단 가운데가 아님: \(toolBar.frame) / \(contentHost.bounds)")
        expect(
            abs(toolBar.frame.height - ShellToolBarView.Metrics.height) < 0.5
                && abs(toolBar.frame.width - ShellToolBarView.Metrics.cellWidth * CGFloat(ShellFrameDataSourceStub.sampleTools.count)) < 0.5,
            "도구 막대 치수가 툴바 도구 그룹과 다름: \(toolBar.frame.size)")
        expect(toolbar.autosavesConfiguration && toolbar.allowsUserCustomization, "툴바 구성 autosave·커스터마이즈 꺼짐")
        expect(controller.toolbarController.navigationGroup?.isNavigational == true, "내비게이션 그룹이 navigational이 아님")

        // 분할 구조(가이드 2.3).
        expect(split.splitViewItems.count == 3, "분할 항목 수가 3이 아님")
        expect(split.sidebarItem.behavior == .sidebar, "사이드바 behavior 아님")
        expect(split.inspectorItem.behavior == .inspector, "인스펙터 behavior 아님")
        expect(split.splitView.autosaveName == configuration.mainSplitAutosaveName, "메인 split autosave 이름 없음")
        expect(
            split.centerController.splitView.autosaveName == configuration.centerSplitAutosaveName,
            "가운데 split autosave 이름 없음"
        )
        expect(window.frameAutosaveName == configuration.autosaveName, "창 frame autosave 이름 없음")
        expect(split.centerController.auxiliaryItem?.isCollapsed == true, "보조 패널이 처음에 접혀 있지 않음")
        // O2 결정 전: 캔버스를 사이드바·인스펙터 아래로 확장하지 않습니다. 캔버스가 자기 safe area에 붙어 있어
        // 겹침 검사만으로는 이 설정 변경을 잡지 못하므로 값 자체를 확인합니다.
        expect(!split.centerItem.automaticallyAdjustsSafeAreaInsets, "가운데 항목이 safe area를 자동 조정함(O2 전)")

        checkPaneGeometry(controller, window: window)
        checkNoAmbiguity(window, context: "ready")
        checkAccessibility(controller)
        checkCustomizationPalette(controller, stub: stub, toolbar: toolbar)
        checkPaneCommands(controller, window: window)
        checkSnapshotFlow(controller, stub: stub)
        checkStatusStates(controller, stub: stub, window: window)
        checkAnimatedToggle(controller, window: window)
    }

    private func checkPaneGeometry(_ controller: ShellWindowController, window: NSWindow) {
        let split = controller.splitController
        let sidebarWidth = split.sidebarItem.viewController.view.frame.width
        let inspectorWidth = split.inspectorItem.viewController.view.frame.width
        let centerWidth = split.centerItem.viewController.view.frame.width
        expect(
            ShellFrameMetrics.sidebarThickness.contains(sidebarWidth),
            "사이드바 폭 범위 밖: \(sidebarWidth)"
        )
        expect(
            ShellFrameMetrics.inspectorThickness.contains(inspectorWidth),
            "인스펙터 폭 범위 밖: \(inspectorWidth)"
        )
        expect(centerWidth >= ShellFrameMetrics.contentMinimumSize.width, "캔버스 폭이 최소값 미만: \(centerWidth)")

        // 캔버스는 safe area 안: 툴바·사이드바·인스펙터와 겹치지 않음(O2 전, 가이드 2.3).
        guard let host = split.contentContainer.contentSlot as? ShellCanvasHostViewController else {
            expect(false, "콘텐츠 슬롯이 캔버스 호스트가 아님")
            return
        }
        let hostView = host.view
        expect(hostView.safeAreaInsets.top > 0, "full-size content인데 상단 safe area inset이 0")
        let canvasInWindow = host.canvasView.convert(host.canvasView.bounds, to: nil)
        let sidebarInWindow = split.sidebarItem.viewController.view.convert(
            split.sidebarItem.viewController.view.bounds, to: nil
        )
        let inspectorInWindow = split.inspectorItem.viewController.view.convert(
            split.inspectorItem.viewController.view.bounds, to: nil
        )
        let contentTop = hostView.convert(
            NSPoint(x: 0, y: hostView.bounds.maxY - hostView.safeAreaInsets.top), to: nil
        ).y
        expect(canvasInWindow.maxY <= contentTop + 0.5, "캔버스가 툴바 아래로 확장됨")
        expect(canvasInWindow.minX >= sidebarInWindow.maxX - 0.5, "캔버스가 사이드바와 겹침")
        expect(canvasInWindow.maxX <= inspectorInWindow.minX + 0.5, "캔버스가 인스펙터와 겹침")
        let drawable = host.canvasView.currentDrawable()
        expect(drawable.width > 0 && drawable.height > 0, "캔버스 drawable 크기가 0")

        // 창 폭이 바뀌면 캔버스만 늘어납니다.
        let canvasWidth = host.canvasView.frame.width
        window.setContentSize(NSSize(
            width: ShellFrameMetrics.initialContentSize.width + 200,
            height: ShellFrameMetrics.initialContentSize.height
        ))
        layout(window)
        expect(
            abs(split.sidebarItem.viewController.view.frame.width - sidebarWidth) < 0.5
                && abs(split.inspectorItem.viewController.view.frame.width - inspectorWidth) < 0.5,
            "창 폭 변경 시 사이드바·인스펙터 폭이 바뀜"
        )
        expect(abs(host.canvasView.frame.width - canvasWidth - 200) < 0.5, "창 폭 변경분이 캔버스로 가지 않음")
        window.setContentSize(ShellFrameMetrics.initialContentSize)
        layout(window)
    }

    private func checkNoAmbiguity(_ window: NSWindow, context: String) {
        guard let root = window.contentView else { return }
        layout(window)
        var ambiguous: [String] = []
        func walk(_ view: NSView) {
            // 숨긴 뷰(접힌 pane의 분할선, stack view가 떼어 낸 뷰)는 배치되지 않으므로 제외합니다.
            if view.isHiddenOrHasHiddenAncestor { return }
            if view.hasAmbiguousLayout {
                var chain: [String] = []
                var cursor: NSView? = view
                while let current = cursor, chain.count < 4 {
                    chain.append(String(describing: type(of: current)))
                    cursor = current.superview
                }
                ambiguous.append(chain.joined(separator: "<"))
            }
            view.subviews.forEach(walk)
        }
        walk(root)
        expect(ambiguous.isEmpty, "모호한 Auto Layout(\(context)): \(ambiguous)")
    }

    private func checkAccessibility(_ controller: ShellWindowController) {
        let split = controller.splitController
        let panes: [(String, NSView)] = [
            ("Sidebar", split.sidebarItem.viewController.view),
            ("Inspector", split.inspectorItem.viewController.view),
            ("Auxiliary Area", split.centerController.auxiliaryItem?.viewController.view ?? NSView()),
        ]
        for (expected, view) in panes {
            expect(view.accessibilityLabel() == expected, "pane 접근성 레이블 불일치: \(expected)")
        }
        if let host = split.contentContainer.contentSlot as? ShellCanvasHostViewController {
            // "Canvas" 이름은 캔버스 뷰 하나에만 둡니다(컨테이너와 중복 낭독 없음).
            expect(!host.view.isAccessibilityElement(), "캔버스 호스트 컨테이너가 접근성 요소라 이름이 중복 낭독됨")
            expect(host.canvasView.isAccessibilityElement(), "캔버스가 접근성 요소가 아님")
            expect(host.canvasView.accessibilityLabel() == "Canvas", "캔버스 접근성 레이블 없음")
            expect(host.canvasView.accessibilityRole() == .layoutArea, "캔버스 접근성 역할이 layout area가 아님")
        }

        let toolbarController = controller.toolbarController
        for item in toolbarController.navigationGroup?.subitems ?? [] {
            expect(!item.label.isEmpty && item.toolTip?.isEmpty == false, "내비게이션 항목 레이블·도움말 없음")
            expect(item.image?.accessibilityDescription?.isEmpty == false, "내비게이션 아이콘 설명 없음")
            expect(!item.isBordered, "내비게이션 아이콘에 테두리가 있음(가이드 5.4)")
        }
        let tools = toolbarController.toolBar.buttons
        expect(tools.count == ShellFrameDataSourceStub.sampleTools.count, "도구 막대 항목 수 불일치")
        for button in tools {
            expect(
                button.accessibilityLabel()?.isEmpty == false && button.toolTip?.isEmpty == false
                    && button.image?.accessibilityDescription?.isEmpty == false,
                "도구 항목 레이블·도움말 없음")
        }
        if let search = toolbarController.searchToolbarItem {
            expect(search.searchField.isEditable, "검색 필드가 편집 불가")
            expect(search.searchField.accessibilityLabel()?.isEmpty == false, "검색 필드 접근성 레이블 없음")
        } else {
            expect(false, "검색 툴바 항목 없음")
        }
        if let more = toolbarController.moreToolbarItem {
            expect(!more.label.isEmpty && more.image?.accessibilityDescription?.isEmpty == false, "더 보기 레이블 없음")
        } else {
            expect(false, "더 보기 툴바 항목 없음")
        }
    }

    /// 사용자 구성 palette 경로: AppKit은 palette에 보일 사본을 `willBeInsertedIntoToolbar: false`로 만들고
    /// palette를 닫으면 해제합니다. 사본이 controller의 항목 참조를 덮어쓰면 안 됩니다.
    private func checkCustomizationPalette(
        _ controller: ShellWindowController,
        stub: ShellFrameDataSourceStub,
        toolbar: NSToolbar
    ) {
        let toolbarController = controller.toolbarController
        let identifiers = [
            ShellToolbarController.navigationItem, ShellToolbarController.searchItem, ShellToolbarController.moreItem,
        ]
        func realItem(_ identifier: NSToolbarItem.Identifier) -> NSToolbarItem? {
            toolbar.items.first { $0.itemIdentifier == identifier }
        }
        expect(toolbarController.navigationGroup === realItem(ShellToolbarController.navigationItem), "내비게이션 참조가 실제 항목이 아님")
        expect(toolbarController.searchToolbarItem === realItem(ShellToolbarController.searchItem), "검색 참조가 실제 항목이 아님")
        expect(toolbarController.moreToolbarItem === realItem(ShellToolbarController.moreItem), "더 보기 참조가 실제 항목이 아님")

        // palette 사본을 만들고(palette 열림) 해제합니다(palette 닫힘).
        autoreleasepool {
            var copies: [NSToolbarItem] = []
            for identifier in identifiers {
                if let copy = toolbarController.toolbar(
                    toolbar, itemForItemIdentifier: identifier, willBeInsertedIntoToolbar: false
                ) {
                    copies.append(copy)
                }
            }
            expect(copies.count == identifiers.count, "palette 사본 수 불일치: \(copies.count)")
            expect(copies.allSatisfy { copy in !toolbar.items.contains { $0 === copy } }, "palette 사본이 실제 항목과 같음")
            if let searchCopy = copies.first(where: { $0.itemIdentifier == ShellToolbarController.searchItem })
                as? NSSearchToolbarItem
            {
                expect(searchCopy.searchField.target == nil, "palette 검색 사본이 명령을 보낼 수 있음")
            }
        }
        expect(toolbarController.navigationGroup != nil
            && toolbarController.searchToolbarItem != nil && toolbarController.moreToolbarItem != nil,
            "palette를 닫은 뒤 항목 참조가 nil이 됨")
        expect(toolbarController.searchToolbarItem === realItem(ShellToolbarController.searchItem), "palette 뒤 검색 참조가 사본으로 바뀜")

        // palette 뒤에도 snapshot이 실제 항목과 도구 막대에 닿고, 메뉴 검증이 실제 검색 항목을 봅니다.
        // 도구 막대는 툴바 항목이 아니라 controller가 직접 가지므로 palette 사본과 관계없습니다.
        let tools = ShellFrameDataSourceStub.sampleTools
        let initialTool = stub.currentSnapshot.activeToolID
        stub.send(.selectTool(tools[2].id))
        controller.flushPendingSnapshot()
        let toolBar = toolbarController.toolBar
        expect(toolBar.selectedIndex == 2, "palette 뒤 도구 선택 snapshot이 도구 막대에 닿지 않음")
        let searchMenu = ShellFrameMenuItems.makeNavigationMenuItems()[2]
        expect(controller.validateMenuItem(searchMenu), "palette 뒤 Search Project… 메뉴가 비활성")
        stub.setStatus(ShellFrameDataSourceStub.sampleLoadingStatus)
        controller.flushPendingSnapshot()
        let realSearch = realItem(ShellToolbarController.searchItem) as? NSSearchToolbarItem
        expect(realSearch?.searchField.isEnabled == false, "palette 뒤 로딩 상태에서 실제 검색 필드가 활성")
        expect(!toolBar.isEnabled, "palette 뒤 로딩 상태에서 도구 막대가 활성")
        stub.setStatus(.ready)
        if let initialTool {
            stub.send(.selectTool(initialTool))
        }
        controller.flushPendingSnapshot()
        expect(toolBar.selectedIndex == 0, "도구 선택 복귀 실패")

        // 코어가 검색을 제공하지 않으면 ready여도 검색 필드와 메뉴를 비활성화합니다.
        var noSearch = stub.currentSnapshot
        noSearch.isSearchAvailable = false
        stub.publish(noSearch)
        controller.flushPendingSnapshot()
        expect(realSearch?.searchField.isEnabled == false, "검색 미제공인데 검색 필드가 활성")
        expect(!controller.validateMenuItem(searchMenu), "검색 미제공인데 Search Project… 메뉴가 활성")
        var withSearch = stub.currentSnapshot
        withSearch.isSearchAvailable = true
        stub.publish(withSearch)
        controller.flushPendingSnapshot()
        expect(realSearch?.searchField.isEnabled == true, "검색 제공 복귀 후 검색 필드가 비활성")
    }

    private func checkPaneCommands(_ controller: ShellWindowController, window: NSWindow) {
        let split = controller.splitController
        // first responder가 창 자체여도 pane 명령이 닿아야 합니다.
        window.makeFirstResponder(nil)
        let menuItems = ShellFrameMenuItems.makeViewMenuItems(auxiliaryTitle: split.centerController.auxiliaryTitle)
        expect(menuItems.count == 3, "View 메뉴 항목 수가 3이 아님")
        expect(menuItems.allSatisfy { $0.target == nil }, "View 메뉴 항목이 first responder action이 아님")

        let commands: [(String, Selector, NSSplitViewItem?)] = [
            ("sidebar", #selector(ShellWindowController.toggleSidebar(_:)), split.sidebarItem),
            ("inspector", #selector(ShellWindowController.toggleInspector(_:)), split.inspectorItem),
            ("auxiliary", #selector(ShellWindowController.toggleAuxiliaryPanel(_:)), split.centerController.auxiliaryItem),
        ]
        for ((name, action, item), menuItem) in zip(commands, menuItems) {
            guard let item else {
                expect(false, "\(name) 항목 없음")
                continue
            }
            let before = item.isCollapsed
            expect(controller.validateMenuItem(menuItem), "\(name) 메뉴 항목 비활성")
            let titleBefore = menuItem.title
            expect(window.tryToPerform(action, with: nil), "\(name) 명령이 응답자 체인에 닿지 않음")
            layout(window)
            expect(item.isCollapsed != before, "\(name) 전환 안 됨")
            _ = controller.validateMenuItem(menuItem)
            expect(menuItem.title != titleBefore, "\(name) 메뉴 제목이 상태를 반영하지 않음: \(titleBefore)")
            checkNoAmbiguity(window, context: "\(name) 전환 후")
            expect(window.tryToPerform(action, with: nil), "\(name) 명령 되돌리기 실패")
            layout(window)
            expect(item.isCollapsed == before, "\(name) 되돌리기 안 됨")
        }

        // 보조 패널 펼침 높이.
        if let auxiliary = split.centerController.auxiliaryItem {
            split.toggleAuxiliaryPanel(nil)
            layout(window)
            let height = auxiliary.viewController.view.frame.height
            expect(ShellFrameMetrics.auxiliaryThickness.contains(height), "보조 패널 높이 범위 밖: \(height)")
            split.toggleAuxiliaryPanel(nil)
            layout(window)
        }
    }

    private func checkSnapshotFlow(_ controller: ShellWindowController, stub: ShellFrameDataSourceStub) {
        let toolbarController = controller.toolbarController
        let tools = ShellFrameDataSourceStub.sampleTools
        expect(toolbarController.toolBar.selectedIndex == 0, "초기 도구 선택 미반영")

        // 다음 런 루프에서 반영: 받은 직후에는 그대로, flush 후 반영.
        stub.send(.selectTool(tools[2].id))
        expect(toolbarController.toolBar.selectedIndex == 0, "snapshot이 즉시 반영됨(병합 안 됨)")
        controller.flushPendingSnapshot()
        expect(toolbarController.toolBar.selectedIndex == 2, "도구 선택 snapshot 미반영")
        expect(
            toolbarController.toolBar.buttons.enumerated().allSatisfy { $0.element.isSelectedTool == ($0.offset == 2) },
            "도구 선택 표시가 한 항목에만 있지 않음")

        // 연속 snapshot은 한 번만 반영합니다.
        let appliedBefore = controller.appliedSnapshotCount
        for title in ["A", "B", "C"] {
            var next = stub.currentSnapshot
            next.title = title
            stub.publish(next)
        }
        controller.flushPendingSnapshot()
        expect(controller.appliedSnapshotCount == appliedBefore + 1, "연속 snapshot이 병합되지 않음")
        expect(controller.window?.title == "C", "마지막 snapshot이 반영되지 않음")

        // 오래된 세대는 버립니다.
        var stale = stub.currentSnapshot
        stale.generation = 1
        stale.title = "Stale"
        stub.deliverUnchecked(stale)
        controller.flushPendingSnapshot()
        expect(controller.window?.title == "C", "오래된 세대 snapshot이 반영됨")

        // 내비게이션: 툴바 항목과 메뉴 항목이 같은 동작으로 연결되고 snapshot으로 활성 상태가 바뀝니다.
        let navigation = toolbarController.navigationGroup?.subitems ?? []
        expect(navigation.count == 2 && navigation.allSatisfy { $0.target === controller }, "내비게이션 대상 불일치")
        toolbarController.validateItems()
        expect(navigation.first?.isEnabled == true && navigation.last?.isEnabled == false, "내비게이션 초기 활성 상태")
        let navigationMenu = ShellFrameMenuItems.makeNavigationMenuItems()
        expect(controller.validateMenuItem(navigationMenu[0]), "Back 메뉴 비활성")
        controller.navigateBack(nil)
        expect(stub.sentCommands.last == .navigateBack, "navigateBack 명령 미전달")
        controller.flushPendingSnapshot()
        expect(navigation.first?.isEnabled == false && navigation.last?.isEnabled == true, "내비게이션 활성 상태 미반영")
        expect(!controller.validateMenuItem(navigationMenu[0]), "Back 메뉴가 비활성화되지 않음")
        expect(controller.window?.subtitle == "Page 1", "내비게이션 부제목 미반영")
        controller.navigateForward(nil)
        controller.flushPendingSnapshot()

        // 도구 막대 UI 선택 → 명령.
        if toolbarController.toolBar.buttons.indices.contains(1) {
            toolbarController.toolBar.buttons[1].performClick(nil)
            expect(stub.sentCommands.last == .selectTool(tools[1].id), "도구 선택 명령 미전달")
            controller.flushPendingSnapshot()
            expect(toolbarController.toolBar.selectedIndex == 1, "도구 선택 후 선택 표시 미반영")
        } else {
            expect(false, "도구 막대 항목 부족")
        }

        // 검색: Return 확정 질의만 전달.
        if let field = toolbarController.searchToolbarItem?.searchField {
            expect(!field.sendsSearchStringImmediately && field.sendsWholeSearchString, "검색이 입력마다 질의함")
            field.stringValue = "Button"
            NSApplication.shared.sendAction(field.action!, to: field.target, from: field)
            expect(stub.sentCommands.last == .submitSearch(query: "Button"), "검색 명령 미전달")
        }

        // 더 보기 메뉴.
        if let menu = toolbarController.moreToolbarItem?.menu {
            expect(menu.items.count == ShellFrameDataSourceStub.sampleMoreActions.count, "더 보기 항목 수 불일치")
            expect(menu.items.last?.isEnabled == false, "비활성 더 보기 항목이 활성")
            menu.performActionForItem(at: 0)
            expect(
                stub.sentCommands.last == .perform(ShellFrameDataSourceStub.sampleMoreActions[0].id),
                "더 보기 명령 미전달"
            )
        }
    }

    private func checkStatusStates(_ controller: ShellWindowController, stub: ShellFrameDataSourceStub, window: NSWindow) {
        let container = controller.splitController.contentContainer
        let contentView = container.contentSlot.paneViewController.view
        let toolbarController = controller.toolbarController

        stub.setStatus(ShellFrameDataSourceStub.sampleLoadingStatus)
        controller.flushPendingSnapshot()
        expect(contentView.isHidden && !container.statusView.isHidden, "로딩 상태 화면 미표시")
        expect(!container.progressIndicator.isHidden, "로딩 진행 표시기 미표시")
        expect(container.progressIndicator.accessibilityLabel()?.isEmpty == false, "로딩 접근성 레이블 없음")
        expect(toolbarController.toolBar.isEnabled == false && toolbarController.toolBar.buttons.allSatisfy { !$0.isEnabled }, "로딩 중 도구 막대가 활성")
        expect(toolbarController.searchToolbarItem?.searchField.isEnabled == false, "로딩 중 검색이 활성")
        expect(!container.statusTitleField.isAccessibilityElement(), "상태 제목 레이블이 그룹 이름과 중복 낭독됨")
        // 동작 메서드도 활성화 검사와 같은 조건을 씁니다: ready가 아니면 직접 호출해도 명령을 보내지 않습니다.
        let sentBeforeGuard = stub.sentCommands.count
        if stub.currentSnapshot.navigation.canGoBack {
            controller.navigateBack(nil)
        }
        if stub.currentSnapshot.navigation.canGoForward {
            controller.navigateForward(nil)
        }
        expect(stub.sentCommands.count == sentBeforeGuard, "로딩 중 내비게이션 명령이 전달됨")
        checkNoAmbiguity(window, context: "loading")

        for (name, status, expectedTitle, expectedButtons) in [
            ("failed", ShellFrameDataSourceStub.sampleFailedStatus, "Couldn’t Open the Project", 2),
            ("empty", ShellFrameDataSourceStub.sampleEmptyStatus, "No Project Open", 3),
        ] {
            stub.setStatus(status)
            controller.flushPendingSnapshot()
            expect(container.statusTitleField.stringValue == expectedTitle, "\(name) 제목 불일치")
            expect(container.statusView.accessibilityLabel() == expectedTitle, "\(name) 접근성 레이블 불일치")
            expect(container.progressIndicator.isHidden, "\(name)에서 진행 표시기 표시")
            let buttons = container.actionButtons
            expect(buttons.count == expectedButtons, "\(name) 버튼 수 불일치: \(buttons.count)")
            expect(buttons.filter { $0.keyEquivalent == "\r" }.count == 1, "\(name) 대표 동작이 하나가 아님")
            expect(buttons.allSatisfy { !$0.title.isEmpty }, "\(name) 버튼 제목 없음")
            checkNoAmbiguity(window, context: name)
            buttons.first?.performClick(nil)
            if case .perform = stub.sentCommands.last {} else {
                expect(false, "\(name) 동작 버튼 명령 미전달")
            }
        }

        stub.setStatus(.ready)
        controller.flushPendingSnapshot()
        expect(!contentView.isHidden && container.statusView.isHidden, "ready 복귀 후 캔버스 미표시")
        expect(toolbarController.toolBar.isEnabled == true && toolbarController.toolBar.buttons.allSatisfy(\.isEnabled), "ready 복귀 후 도구 막대 비활성")
    }

    private func checkAnimatedToggle(_ controller: ShellWindowController, window: NSWindow) {
        // 동작 줄이기가 꺼져 있으면 시스템 애니메이션 경로로 전환합니다.
        let split = controller.splitController
        split.prefersReducedMotion = { false }
        for target in [true, false] {
            split.toggleSidebar(nil)
            let deadline = Date().addingTimeInterval(3)
            while split.sidebarItem.isCollapsed != target, Date() < deadline {
                RunLoop.current.run(until: Date().addingTimeInterval(0.05))
            }
            expect(split.sidebarItem.isCollapsed == target, "애니메이션 경로 사이드바 전환 실패(\(target))")
        }
        split.prefersReducedMotion = { true }
    }

    private func checkAutosaveRoundTrip() {
        // 첫 창: 인스펙터를 접고 크기를 바꾼 뒤 닫습니다.
        let savedSize = NSSize(width: 1100, height: 700)
        autoreleasepool {
            let controller = makeController(ShellFrameDataSourceStub(), configuration: roundTripConfiguration)
            guard let window = controller.window else { return }
            controller.splitController.prefersReducedMotion = { true }
            window.setContentSize(savedSize)
            layout(window)
            controller.splitController.toggleInspector(nil)
            layout(window)
            // split view는 상태를 다음 런 루프에서 기록하므로 한 번 돌린 뒤 닫습니다.
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
            window.close()
        }
        let defaults = UserDefaults.standard
        let configuration = roundTripConfiguration
        expect(defaults.string(forKey: "NSWindow Frame \(configuration.autosaveName)") != nil, "창 frame autosave 기록 없음")
        expect(
            defaults.object(forKey: "NSSplitView Subview Frames \(configuration.mainSplitAutosaveName)") != nil,
            "메인 split autosave 기록 없음"
        )

        // 같은 이름으로 다시 만든 창이 상태를 이어받습니다.
        autoreleasepool {
            let controller = makeController(ShellFrameDataSourceStub(), configuration: roundTripConfiguration)
            guard let window = controller.window else { return }
            layout(window)
            let size = window.contentRect(forFrameRect: window.frame).size
            expect(abs(size.width - savedSize.width) < 1 && abs(size.height - savedSize.height) < 1, "창 크기 복원 실패: \(size)")
            expect(controller.splitController.inspectorItem.isCollapsed, "인스펙터 접힘 상태 복원 실패")
            window.close()
        }
    }

    private func checkRestorationEntry() {
        autoreleasepool {
            let controller = makeController(ShellFrameDataSourceStub())
            guard let window = controller.window else { return }
            expect(window.isRestorable, "창 복원이 꺼져 있음")
            expect(window.identifier?.rawValue == configuration.restorationIdentifier, "창 복원 식별자 없음")
            expect(window.restorationClass == ShellWindowRestoration.self, "창 복원 클래스 미지정")
            window.close()
        }
        // provider 미등록이면 실패를 알리고 창을 만들지 않습니다.
        let previous = ShellWindowRestoration.windowProvider
        ShellWindowRestoration.windowProvider = nil
        var restored: NSWindow?
        var error: (any Error)?
        ShellWindowRestoration.restoreWindow(
            withIdentifier: NSUserInterfaceItemIdentifier(configuration.restorationIdentifier),
            state: NSKeyedArchiver(requiringSecureCoding: true)
        ) { window, failure in
            restored = window
            error = failure
        }
        expect(restored == nil && error != nil, "provider 없이 창 복원이 성공함")
        ShellWindowRestoration.windowProvider = previous
    }

    /// 검사용 autosave 기록을 지웁니다. 중간에 비정상 종료한 이전 실행의 기록도 함께 지웁니다.
    private func removeAutosavedState() {
        let defaults = UserDefaults.standard
        for key in defaults.dictionaryRepresentation().keys where key.contains(Self.namePrefix) {
            defaults.removeObject(forKey: key)
        }
    }
}
