import AppKit

/// 캔버스 도구 그룹과 줌 컨트롤 툴바 항목(디자인 가이드 6.1, 계획 XII.3.6·XII.8.1).
///
/// 표준 구성요소만 씁니다: 도구는 `NSToolbarItemGroup`(selectOne, 펼친 표현, 좁으면 시스템 overflow),
/// 줌은 `NSMenuToolbarItem`(현재 배율을 제목으로 보이는 pull-down). 커스텀 뷰가 없으므로 Liquid Glass,
/// 투명도 줄이기, 대비 높이기, 동작 줄이기, Light/Dark는 시스템이 처리합니다(디자인 가이드 3.2, 8.4).
///
/// 창의 툴바 delegate로 그대로 쓰거나, 다른 delegate가 `item(for:)`로 항목만 가져가 조합할 수 있습니다.
/// snapshot이 바뀌면 바뀐 값만 다시 씁니다(자동 검증을 끄고 snapshot으로만 갱신, 아키텍처 §6.3).
final class ToolsToolbarController: NSObject, NSToolbarDelegate {
    static let toolsItemIdentifier = NSToolbarItem.Identifier("xyz.taeyoon.stellar.toolbar.tools")
    static let zoomItemIdentifier = NSToolbarItem.Identifier("xyz.taeyoon.stellar.toolbar.zoom")

    /// 기본 배치: 앞쪽 끝 사이드바 토글, 가운데 도구, 뒤쪽 끝 줌·인스펙터 토글(디자인 가이드 6.1).
    static let defaultItemIdentifiers: [NSToolbarItem.Identifier] = [
        .toggleSidebar, .sidebarTrackingSeparator,
        toolsItemIdentifier,
        .flexibleSpace,
        zoomItemIdentifier,
        .inspectorTrackingSeparator, .toggleInspector,
    ]

    let model: ToolsMenuModel
    let router: ShellCommandRouter
    /// 도구 그룹(target·action이 이 컨트롤러이므로 init 뒤에 만듭니다).
    private(set) lazy var toolsGroup: NSToolbarItemGroup = makeToolsGroup()
    let zoomItem: NSMenuToolbarItem
    private var appliedGeneration: UInt64?

    /// - Parameters:
    ///   - model: 창 session의 모델. `snapshotDidChange` 구독자를 이 컨트롤러로 지정합니다.
    ///   - router: 같은 창의 router. 도구·줌 명령을 이 router로 보냅니다.
    init(model: ToolsMenuModel, router: ShellCommandRouter) {
        self.model = model
        self.router = router
        zoomItem = Self.makeZoomItem(router: router)
        super.init()
        model.snapshotDidChange = { [weak self] snapshot in
            self?.apply(snapshot)
        }
        apply(model.snapshot)
    }

    /// 툴바를 만듭니다. 사용자 설정 저장은 호출자가 정합니다(검사에서는 끔).
    func makeToolbar(identifier: NSToolbar.Identifier, autosavesConfiguration: Bool) -> NSToolbar {
        let toolbar = NSToolbar(identifier: identifier)
        toolbar.delegate = self
        toolbar.allowsUserCustomization = true
        toolbar.autosavesConfiguration = autosavesConfiguration
        toolbar.centeredItemIdentifiers = [Self.toolsItemIdentifier]
        return toolbar
    }

    /// 이 영역의 툴바 항목. 다른 identifier는 nil(시스템 항목은 AppKit이 만듭니다).
    func item(for identifier: NSToolbarItem.Identifier) -> NSToolbarItem? {
        switch identifier {
        case Self.toolsItemIdentifier: return toolsGroup
        case Self.zoomItemIdentifier: return zoomItem
        default: return nil
        }
    }

    // MARK: Snapshot

    /// snapshot을 툴바 항목에 반영합니다. 오래된 세대는 버립니다.
    func apply(_ snapshot: ToolsMenuSnapshot) {
        if let applied = appliedGeneration, snapshot.generation < applied { return }
        appliedGeneration = snapshot.generation

        let tools = CanvasTool.allCases
        if let index = tools.firstIndex(of: snapshot.activeTool), toolsGroup.selectedIndex != index {
            toolsGroup.selectedIndex = index
        }
        var anyToolEnabled = false
        for (subitem, tool) in zip(toolsGroup.subitems, tools) {
            let enabled = snapshot.enabledCommands.contains(tool.command)
            if subitem.isEnabled != enabled { subitem.isEnabled = enabled }
            anyToolEnabled = anyToolEnabled || enabled
        }
        if toolsGroup.isEnabled != anyToolEnabled { toolsGroup.isEnabled = anyToolEnabled }

        let title = ZoomFormat.percentString(snapshot.zoomScale)
        if zoomItem.title != title { zoomItem.title = title }
        let zoomEnabled = ShellCommand.zoomCommands.contains { snapshot.enabledCommands.contains($0) }
        if zoomItem.isEnabled != zoomEnabled { zoomItem.isEnabled = zoomEnabled }
    }

    // MARK: Action

    /// 도구 그룹 선택 변경. 명령이 거부되면 snapshot의 도구로 선택을 되돌립니다.
    @objc func toolSelectionChanged(_ sender: Any?) {
        let index = toolsGroup.selectedIndex
        let tools = CanvasTool.allCases
        guard tools.indices.contains(index) else { return }
        if !router.dispatch(tools[index].command) {
            appliedGeneration = nil
            apply(model.snapshot)
        }
    }

    // MARK: NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.defaultItemIdentifiers
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.defaultItemIdentifiers + [.space]
    }

    func toolbarImmovableItemIdentifiers(_ toolbar: NSToolbar) -> Set<NSToolbarItem.Identifier> {
        // 앞쪽 끝(사이드바 토글)은 커스터마이즈 대상에서 뺍니다(HIG Toolbars).
        [.toggleSidebar, .sidebarTrackingSeparator]
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        item(for: itemIdentifier)
    }

    // MARK: 항목 생성

    private func makeToolsGroup() -> NSToolbarItemGroup {
        let tools = CanvasTool.allCases
        let images = tools.map { tool in
            NSImage(systemSymbolName: tool.symbolName, accessibilityDescription: tool.label) ?? NSImage()
        }
        let group = NSToolbarItemGroup(
            itemIdentifier: Self.toolsItemIdentifier,
            images: images,
            selectionMode: .selectOne,
            labels: tools.map(\.label),
            target: self,
            action: #selector(toolSelectionChanged(_:))
        )
        group.label = "Tools"
        group.paletteLabel = "Canvas Tools"
        group.toolTip = "Canvas tools"
        // 도구 전환은 한 번 클릭이어야 하므로 펼친 표현을 고정합니다. macOS 26 SDK(macOS 27.2 실행)에서
        // 측정한 결과 `.automatic`은 이 11개 그룹을 폭과 무관하게 pull-down으로 접었습니다. 창이 좁으면
        // 시스템 overflow 메뉴가 처리하며, 우선순위가 낮은 줌 항목이 먼저 넘어갑니다(HIG Toolbars).
        group.controlRepresentation = .expanded
        group.visibilityPriority = .high
        group.autovalidates = false
        for (subitem, tool) in zip(group.subitems, tools) {
            subitem.toolTip = tool.helpTag
            subitem.autovalidates = false
        }
        return group
    }

    private static func makeZoomItem(router: ShellCommandRouter) -> NSMenuToolbarItem {
        let item = NSMenuToolbarItem(itemIdentifier: zoomItemIdentifier)
        item.label = "Zoom"
        item.paletteLabel = "Zoom"
        item.toolTip = "Zoom level"
        item.showsIndicator = true
        item.autovalidates = false
        // 같은 명령이 View 메뉴와 단축키에도 있으므로 공간이 부족하면 먼저 overflow로 보냅니다.
        item.visibilityPriority = .low
        let menu = NSMenu(title: "Zoom")
        for (index, command) in ShellCommand.zoomCommands.enumerated() {
            if index == 2 { menu.addItem(.separator()) }
            // 단축키는 View 메뉴에만 둡니다(같은 key equivalent를 두 메뉴에 두지 않음).
            menu.addItem(MainMenuBuilder.makeItem(for: command, target: router, includeShortcut: false))
        }
        item.menu = menu
        return item
    }
}
