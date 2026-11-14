import AppKit

/// 메인 창 통합 툴바(가이드 6.1).
///
/// - 앞쪽 끝: 사이드바 토글(시스템 항목), 사이드바 경계 추적 구분자, 뒤로·앞으로 내비게이션
/// - 뒤쪽 끝: 프로젝트 검색, 더 보기 메뉴, 인스펙터 경계 추적 구분자, 인스펙터 토글(시스템 항목)
///
/// 도구 영역(placeholder, 도구 상태 기계는 XII.3.6)은 툴바가 아니라 캔버스 하단 가운데의
/// `ShellToolBarView`에 둡니다. 이 controller가 그 막대의 도구 목록·선택·활성 상태를 맡습니다.
///
/// 항목 위치는 시스템 식별자(`.toggleSidebar`, `.sidebarTrackingSeparator`, `.toggleInspector`,
/// `.inspectorTrackingSeparator`)로만 정하고 직접 계산하지 않습니다.
/// 사용자 구성은 `autosavesConfiguration`으로 AppKit이 저장합니다.
final class ShellToolbarController: NSObject, NSToolbarDelegate, NSToolbarItemValidation {
    static let navigationItem = NSToolbarItem.Identifier("stellar.shell.navigation")
    static let navigateBackItem = NSToolbarItem.Identifier("stellar.shell.navigation.back")
    static let navigateForwardItem = NSToolbarItem.Identifier("stellar.shell.navigation.forward")
    static let searchItem = NSToolbarItem.Identifier("stellar.shell.search")
    static let moreItem = NSToolbarItem.Identifier("stellar.shell.more")

    /// 기본 항목 순서.
    static let defaultItemIdentifiers: [NSToolbarItem.Identifier] = [
        .toggleSidebar,
        .sidebarTrackingSeparator,
        navigationItem,
        .flexibleSpace,
        searchItem,
        moreItem,
        .inspectorTrackingSeparator,
        .flexibleSpace,
        .toggleInspector,
    ]

    /// 사용자 구성에서 옮기거나 뺄 수 없는 앞쪽 끝 항목(가이드 6.1).
    static let immovableItemIdentifiers: Set<NSToolbarItem.Identifier> = [
        .toggleSidebar, .sidebarTrackingSeparator,
    ]

    let toolbar: NSToolbar
    /// 툴바 동작을 코어 명령으로 보냅니다. 셸 창 controller가 지정합니다.
    var commandHandler: ((ShellFrameCommand) -> Void)?
    /// 뒤로·앞으로 동작 대상. 메뉴 항목과 같은 동작 메서드를 부르도록 셸 창 controller를 지정합니다.
    weak var navigationTarget: AnyObject?

    private(set) var snapshot: ShellFrameSnapshot
    private(set) weak var navigationGroup: NSToolbarItemGroup?
    /// 캔버스 하단 가운데 도구 막대. 셸 창 controller가 캔버스 영역에 붙입니다.
    let toolBar = ShellToolBarView()
    private(set) weak var searchToolbarItem: NSSearchToolbarItem?
    private(set) weak var moreToolbarItem: NSMenuToolbarItem?
    /// 현재 도구 영역 항목이 만들어진 도구 목록.
    private var builtTools: [ShellToolDescriptor] = []

    init(identifier: String, snapshot: ShellFrameSnapshot) {
        self.snapshot = snapshot
        toolbar = NSToolbar(identifier: NSToolbar.Identifier(identifier))
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = true
        toolbar.autosavesConfiguration = true
        toolBar.selectionHandler = { [weak self] index in
            self?.toolSelected(at: index)
        }
        rebuildTools()
    }

    /// snapshot을 반영합니다. 바뀐 항목만 갱신합니다(가이드 9장).
    func apply(_ newSnapshot: ShellFrameSnapshot) {
        let previous = snapshot
        snapshot = newSnapshot

        if newSnapshot.tools != builtTools {
            // 도구 목록 자체가 바뀌면 도구 막대를 다시 만듭니다(드문 경우).
            rebuildTools()
        } else if newSnapshot.activeToolID != previous.activeToolID {
            syncToolSelection()
        }
        if newSnapshot.moreActions != previous.moreActions, let item = moreToolbarItem {
            item.menu = makeMoreMenu()
        }
        if newSnapshot.searchPlaceholder != previous.searchPlaceholder {
            searchToolbarItem?.searchField.placeholderString = newSnapshot.searchPlaceholder
        }
        if let field = searchToolbarItem?.searchField, field.isEnabled != isSearchUsable {
            field.isEnabled = isSearchUsable
        }
        validateItems()
    }

    /// 셸 프레임 항목의 활성 상태를 즉시 다시 계산합니다.
    ///
    /// 내비게이션 항목은 셸 창 controller의 `validateToolbarItem(_:)`로 자동 검사합니다. 도구 막대와
    /// 더 보기 항목은 snapshot에서 바로 정하므로 자동 검사를 끄고 여기서 지정합니다.
    func validateItems() {
        navigationGroup?.subitems.forEach { $0.validate() }
        toolBar.isEnabled = snapshot.status == .ready && !builtTools.isEmpty
        if let moreToolbarItem {
            moreToolbarItem.isEnabled = validateToolbarItem(moreToolbarItem)
        }
    }

    // MARK: - NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.defaultItemIdentifiers
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        var allowed: [NSToolbarItem.Identifier] = []
        for identifier in Self.defaultItemIdentifiers + [.space] where !allowed.contains(identifier) {
            allowed.append(identifier)
        }
        return allowed
    }

    func toolbarImmovableItemIdentifiers(_ toolbar: NSToolbar) -> Set<NSToolbarItem.Identifier> {
        Self.immovableItemIdentifiers
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemIdentifier: NSToolbarItem.Identifier,
        canBeInsertedAt index: Int
    ) -> Bool {
        // 사이드바 영역(앞쪽 끝 고정 항목 앞)에는 다른 항목을 넣지 않습니다.
        Self.immovableItemIdentifiers.contains(itemIdentifier) || index >= Self.immovableItemIdentifiers.count
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        // `flag == false`는 사용자 구성 palette에 보일 사본입니다. palette가 닫히면 사본은 해제되므로
        // controller의 항목 참조는 실제 툴바에 들어갈 항목(`flag == true`)에만 묶습니다. 사본에 묶으면
        // palette를 닫은 뒤 weak 참조가 nil이 되어 snapshot 반영과 메뉴 검증이 실제 항목에 닿지 않습니다.
        let bindsToController = flag
        let item: NSToolbarItem?
        switch itemIdentifier {
        case Self.navigationItem:
            item = makeNavigationGroup(binds: bindsToController)
        case Self.searchItem:
            item = makeSearchItem(binds: bindsToController)
        case Self.moreItem:
            item = makeMoreItem(binds: bindsToController)
        default:
            item = nil
        }
        // 항목이 늦게 만들어져도 현재 snapshot의 활성 상태로 시작합니다.
        if bindsToController {
            validateItems()
        }
        return item
    }

    // MARK: - 항목 생성

    private func makeNavigationGroup(binds: Bool) -> NSToolbarItemGroup {
        let back = makeButtonItem(
            Self.navigateBackItem, label: ShellFrameStrings.back, symbolName: ShellFrameSymbols.navigateBack,
            action: #selector(ShellWindowController.navigateBack(_:))
        )
        let forward = makeButtonItem(
            Self.navigateForwardItem, label: ShellFrameStrings.forward, symbolName: ShellFrameSymbols.navigateForward,
            action: #selector(ShellWindowController.navigateForward(_:))
        )
        let group = NSToolbarItemGroup(itemIdentifier: Self.navigationItem)
        group.subitems = [back, forward]
        group.label = ShellFrameStrings.backForward
        group.paletteLabel = ShellFrameStrings.backForward
        // 내비게이션 항목은 창 제목 앞쪽에 놓입니다.
        group.isNavigational = true
        if binds {
            navigationGroup = group
        }
        return group
    }

    private func makeButtonItem(
        _ identifier: NSToolbarItem.Identifier,
        label: String,
        symbolName: String,
        action: Selector
    ) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.image = NSImage(systemSymbolName: symbolName, accessibilityDescription: label)
        item.label = label
        item.paletteLabel = label
        item.toolTip = label
        // 툴바 아이콘은 테두리 없이 둡니다(가이드 5.4). 눌림·강조 표현은 시스템이 그립니다.
        item.isBordered = false
        item.target = navigationTarget
        item.action = action
        return item
    }

    private func rebuildTools() {
        builtTools = snapshot.tools
        toolBar.setTools(builtTools)
        syncToolSelection()
    }

    private func makeSearchItem(binds: Bool) -> NSSearchToolbarItem {
        let item = NSSearchToolbarItem(itemIdentifier: Self.searchItem)
        item.label = ShellFrameStrings.search
        item.paletteLabel = ShellFrameStrings.searchProject
        item.toolTip = ShellFrameStrings.searchProject
        let field = item.searchField
        field.placeholderString = snapshot.searchPlaceholder
        // Return으로 확정한 질의만 보냅니다. 입력마다 질의하지 않아 메인 스레드 부담을 줄입니다(가이드 9장).
        field.sendsSearchStringImmediately = false
        field.sendsWholeSearchString = true
        field.setAccessibilityLabel(ShellFrameStrings.searchProject)
        if binds {
            field.target = self
            field.action = #selector(searchSubmitted(_:))
            field.isEnabled = isSearchUsable
            searchToolbarItem = item
        } else {
            field.isEnabled = false
        }
        return item
    }

    /// 검색 필드를 쓸 수 있는지: 창이 ready이고 코어가 검색을 제공할 때만.
    var isSearchUsable: Bool {
        snapshot.status == .ready && snapshot.isSearchAvailable
    }

    private func makeMoreItem(binds: Bool) -> NSMenuToolbarItem {
        let item = NSMenuToolbarItem(itemIdentifier: Self.moreItem)
        item.image = NSImage(
            systemSymbolName: ShellFrameSymbols.moreMenu, accessibilityDescription: ShellFrameStrings.more
        )
        item.label = ShellFrameStrings.more
        item.paletteLabel = ShellFrameStrings.more
        item.toolTip = ShellFrameStrings.more
        item.showsIndicator = false
        item.autovalidates = false
        item.menu = makeMoreMenu(binds: binds)
        if binds {
            moreToolbarItem = item
        }
        return item
    }

    private func makeMoreMenu(binds: Bool = true) -> NSMenu {
        let menu = NSMenu(title: ShellFrameStrings.more)
        menu.autoenablesItems = false
        for action in snapshot.moreActions {
            let menuItem = NSMenuItem(
                title: action.title, action: binds ? #selector(moreActionChosen(_:)) : nil, keyEquivalent: ""
            )
            menuItem.target = binds ? self : nil
            menuItem.representedObject = action.id.rawValue
            menuItem.isEnabled = action.isEnabled
            menu.addItem(menuItem)
        }
        return menu
    }

    private func syncToolSelection() {
        if let activeID = snapshot.activeToolID, let index = builtTools.firstIndex(where: { $0.id == activeID }) {
            toolBar.selectedIndex = index
        } else {
            toolBar.selectedIndex = -1
        }
    }

    // MARK: - 동작

    /// 도구 막대에서 도구를 골랐습니다. 선택 표시는 코어 snapshot이 돌아올 때 바뀝니다.
    func toolSelected(at index: Int) {
        guard builtTools.indices.contains(index) else { return }
        commandHandler?(.selectTool(builtTools[index].id))
    }

    @objc func searchSubmitted(_ sender: NSSearchField) {
        commandHandler?(.submitSearch(query: sender.stringValue))
    }

    @objc func moreActionChosen(_ sender: NSMenuItem) {
        guard let rawValue = sender.representedObject as? String else { return }
        commandHandler?(.perform(ShellCommandID(rawValue)))
    }

    // MARK: - NSToolbarItemValidation

    func validateToolbarItem(_ item: NSToolbarItem) -> Bool {
        switch item.itemIdentifier {
        case Self.moreItem:
            return snapshot.moreActions.contains(where: \.isEnabled)
        default:
            return true
        }
    }
}
