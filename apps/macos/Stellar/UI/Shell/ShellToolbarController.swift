import AppKit

/// 메인 창 통합 툴바(가이드 6.1).
///
/// - 앞쪽 끝: 사이드바 토글(시스템 항목), 사이드바 경계 추적 구분자
/// - 뒤쪽 끝: 더 보기 메뉴, 인스펙터 경계 추적 구분자, 인스펙터 토글(시스템 항목)
///
/// 뒤로·앞으로와 프로젝트 검색은 기능이 없어 두지 않습니다. 기능이 생길 때 다시 설계합니다(UX 명세 3.3).
///
/// 도구 영역(placeholder, 도구 상태 기계는 XII.3.6)은 툴바가 아니라 캔버스 하단 가운데의
/// `ShellToolBarView`에 둡니다. 이 controller가 그 막대의 도구 목록·선택·활성 상태를 맡습니다.
///
/// 항목 위치는 시스템 식별자(`.toggleSidebar`, `.sidebarTrackingSeparator`, `.toggleInspector`,
/// `.inspectorTrackingSeparator`)로만 정하고 직접 계산하지 않습니다.
/// 사용자 구성은 `autosavesConfiguration`으로 AppKit이 저장합니다.
///
/// 다른 영역의 툴바 항목(예: 캔버스 줌, `ToolsToolbarController`)은 `ShellToolbarItemProviding`으로 받아
/// 같은 툴바에 조합합니다. provider의 뒤쪽 항목은 더 보기 메뉴 앞에 놓입니다. 도구는 툴바에 두지 않으므로 provider도
/// 가운데 항목을 내주지 않습니다. 캔버스 도구 목록은 셸 프레임 snapshot의 `tools`로 하단 도구 막대에 그립니다.
final class ShellToolbarController: NSObject, NSToolbarDelegate, NSToolbarItemValidation {
    static let moreItem = NSToolbarItem.Identifier("stellar.shell.more")

    /// 기본 항목 순서.
    static let defaultItemIdentifiers: [NSToolbarItem.Identifier] = [
        .toggleSidebar,
        .sidebarTrackingSeparator,
        .flexibleSpace,
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
    /// 다른 영역이 내주는 툴바 항목(더 보기 메뉴 앞 뒤쪽 항목). nil이면 셸 프레임 항목만 둡니다.
    let itemProvider: ShellToolbarItemProviding?
    /// 이 툴바의 기본 항목 순서(provider 반영).
    let defaultItemIdentifiers: [NSToolbarItem.Identifier]
    /// 툴바 동작을 코어 명령으로 보냅니다. 셸 창 controller가 지정합니다.
    var commandHandler: ((ShellFrameCommand) -> Void)?

    private(set) var snapshot: ShellFrameSnapshot
    /// 캔버스 하단 가운데 도구 막대. 셸 창 controller가 캔버스 영역에 붙입니다.
    let toolBar = ShellToolBarView()
    private(set) weak var moreToolbarItem: NSMenuToolbarItem?
    /// 현재 도구 영역 항목이 만들어진 도구 목록.
    private var builtTools: [ShellToolDescriptor] = []

    init(identifier: String, snapshot: ShellFrameSnapshot, itemProvider: ShellToolbarItemProviding? = nil) {
        self.snapshot = snapshot
        self.itemProvider = itemProvider
        if let itemProvider {
            defaultItemIdentifiers = Self.defaultItemIdentifiers.flatMap { identifier -> [NSToolbarItem.Identifier] in
                identifier == Self.moreItem ? itemProvider.trailingItemIdentifiers + [Self.moreItem] : [identifier]
            }
        } else {
            defaultItemIdentifiers = Self.defaultItemIdentifiers
        }
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
        validateItems()
    }

    /// 셸 프레임 항목의 활성 상태를 즉시 다시 계산합니다.
    ///
    /// 도구 막대와 더 보기 항목은 snapshot에서 바로 정하므로 자동 검사를 끄고 여기서 지정합니다.
    func validateItems() {
        toolBar.isEnabled = snapshot.status == .ready && !builtTools.isEmpty
        if let moreToolbarItem {
            moreToolbarItem.isEnabled = validateToolbarItem(moreToolbarItem)
        }
    }

    // MARK: - NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        defaultItemIdentifiers
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        var allowed: [NSToolbarItem.Identifier] = []
        for identifier in defaultItemIdentifiers + [.space] where !allowed.contains(identifier) {
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
        if let itemProvider, itemProvider.providesItem(itemIdentifier) {
            return itemProvider.item(for: itemIdentifier, willBeInsertedIntoToolbar: flag)
        }
        let item: NSToolbarItem?
        switch itemIdentifier {
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

    private func rebuildTools() {
        builtTools = snapshot.tools
        toolBar.setTools(builtTools)
        syncToolSelection()
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

/// 셸 툴바에 항목을 내주는 다른 영역(가이드 6.1).
///
/// 항목 생성과 상태 반영은 provider가 맡습니다. `willBeInsertedIntoToolbar`가 false이면 사용자 구성 palette용
/// 사본을 돌려줘야 합니다(실제 항목 인스턴스를 palette에 넘기지 않음). 도구 영역은 툴바가 아니라 캔버스 하단 가운데
/// 도구 막대(`ShellToolBarView`)에 있으므로 provider는 가운데 항목을 내주지 않습니다(2026-11-14 사용자 결정).
protocol ShellToolbarItemProviding: AnyObject {
    /// 더 보기 메뉴 앞에 놓을 뒤쪽 항목.
    var trailingItemIdentifiers: [NSToolbarItem.Identifier] { get }
    /// 툴바 delegate 요청 하나에 맞는 항목.
    func item(for identifier: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem?
}

extension ShellToolbarItemProviding {
    func providesItem(_ identifier: NSToolbarItem.Identifier) -> Bool {
        trailingItemIdentifiers.contains(identifier)
    }
}

/// 캔버스 줌(뒤쪽)을 셸 툴바에 조합합니다(tools_menus 3.1). 캔버스 도구 그룹은 셸 툴바에 넣지 않고, 같은 도구를
/// 프로젝트 창 data source가 셸 프레임 snapshot 도구 목록으로 하단 도구 막대에 보냅니다.
extension ToolsToolbarController: ShellToolbarItemProviding {
    var trailingItemIdentifiers: [NSToolbarItem.Identifier] { [Self.zoomItemIdentifier] }
}
