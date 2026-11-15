import AppKit

/// 메인 창 통합 툴바(가이드 6.1).
///
/// - 앞쪽 끝: 사이드바 토글(시스템 항목), 사이드바 경계 추적 구분자, 제목·부제목(누르면 제목 문서 메뉴)
/// - 뒤쪽 끝: 인스펙터 경계 추적 구분자, 인스펙터 토글(시스템 항목)
///
/// 뒤로·앞으로와 프로젝트 검색은 기능이 없어 두지 않습니다. 기능이 생길 때 다시 설계합니다(UX 명세 3.3).
/// 창 제목은 시스템 제목 대신 제목 항목(`ShellTitleMenuView`)이 그립니다. 제목을 누르면 문서 메뉴(Finder에서 보기,
/// 프로젝트 신뢰…·신뢰 철회, 프로젝트 닫기)가 열립니다(UX 명세 3.3). `NSWindow.title`·`subtitle`은 창 메뉴와
/// 접근성을 위해 그대로 둡니다.
///
/// 도구 영역(placeholder, 도구 상태 기계는 XII.3.6)은 툴바가 아니라 캔버스 하단 가운데의
/// `ShellToolBarView`에 둡니다. 이 controller가 그 막대의 도구 목록·선택·활성 상태를 맡습니다.
///
/// 항목 위치는 시스템 식별자(`.toggleSidebar`, `.sidebarTrackingSeparator`, `.toggleInspector`,
/// `.inspectorTrackingSeparator`)로만 정하고 직접 계산하지 않습니다.
/// 사용자 구성은 `autosavesConfiguration`으로 AppKit이 저장합니다.
///
/// 다른 영역의 툴바 항목(예: 캔버스 줌, `ToolsToolbarController`)은 `ShellToolbarItemProviding`으로 받아
/// 같은 툴바에 조합합니다. provider의 뒤쪽 항목은 인스펙터 경계 앞에 놓입니다. 도구는 툴바에 두지 않으므로 provider도
/// 가운데 항목을 내주지 않습니다. 캔버스 도구 목록은 셸 프레임 snapshot의 `tools`로 하단 도구 막대에 그립니다.
final class ShellToolbarController: NSObject, NSToolbarDelegate {
    static let titleItem = NSToolbarItem.Identifier("stellar.shell.title")

    /// 기본 항목 순서.
    static let defaultItemIdentifiers: [NSToolbarItem.Identifier] = [
        .toggleSidebar,
        .sidebarTrackingSeparator,
        titleItem,
        .flexibleSpace,
        .inspectorTrackingSeparator,
        .flexibleSpace,
        .toggleInspector,
    ]

    /// 사용자 구성에서 옮기거나 뺄 수 없는 앞쪽 끝 항목(가이드 6.1). 제목이 빠지면 창 제목이 보이지 않으므로 제목도
    /// 고정합니다.
    static let immovableItemIdentifiers: Set<NSToolbarItem.Identifier> = [
        .toggleSidebar, .sidebarTrackingSeparator, titleItem,
    ]

    let toolbar: NSToolbar
    /// 다른 영역이 내주는 툴바 항목(인스펙터 경계 앞 뒤쪽 항목). nil이면 셸 프레임 항목만 둡니다.
    let itemProvider: ShellToolbarItemProviding?
    /// 이 툴바의 기본 항목 순서(provider 반영).
    let defaultItemIdentifiers: [NSToolbarItem.Identifier]
    /// 툴바 동작을 코어 명령으로 보냅니다. 셸 창 controller가 지정합니다.
    var commandHandler: ((ShellFrameCommand) -> Void)?

    private(set) var snapshot: ShellFrameSnapshot
    /// 캔버스 하단 가운데 도구 막대. 셸 창 controller가 캔버스 영역에 붙입니다.
    let toolBar = ShellToolBarView()
    /// 상단 바 앞쪽 제목·부제목과 제목 문서 메뉴. 툴바 제목 항목이 이 뷰를 담습니다.
    let titleView = ShellTitleMenuView()
    /// 도구 막대 위 한 줄 안내. 도구 막대와 함께 캔버스 영역에 붙습니다.
    let hintView = ShellCanvasHintView()
    /// 현재 도구 영역 항목이 만들어진 도구 목록.
    private var builtTools: [ShellToolDescriptor] = []

    init(identifier: String, snapshot: ShellFrameSnapshot, itemProvider: ShellToolbarItemProviding? = nil) {
        self.snapshot = snapshot
        self.itemProvider = itemProvider
        if let itemProvider {
            defaultItemIdentifiers = Self.defaultItemIdentifiers.flatMap { identifier -> [NSToolbarItem.Identifier] in
                identifier == .inspectorTrackingSeparator
                    ? itemProvider.trailingItemIdentifiers + [identifier] : [identifier]
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
        titleView.actionHandler = { [weak self] id in
            self?.commandHandler?(.perform(id))
        }
        titleView.apply(title: snapshot.title, subtitle: snapshot.subtitle, actions: snapshot.moreActions)
        rebuildTools()
    }

    /// 저장된 사용자 구성에 제목 항목이 없으면(이전 버전 구성) 고정 위치에 넣습니다. 창에 툴바를 붙인 뒤 부릅니다.
    func ensureTitleItem() {
        guard !toolbar.items.contains(where: { $0.itemIdentifier == Self.titleItem }) else { return }
        let index = toolbar.items.firstIndex { $0.itemIdentifier == .sidebarTrackingSeparator }.map { $0 + 1 } ?? 0
        toolbar.insertItem(withItemIdentifier: Self.titleItem, at: min(index, toolbar.items.count))
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
        if newSnapshot.title != previous.title || newSnapshot.subtitle != previous.subtitle
            || newSnapshot.moreActions != previous.moreActions
        {
            titleView.apply(title: newSnapshot.title, subtitle: newSnapshot.subtitle, actions: newSnapshot.moreActions)
        }
        validateItems()
    }

    /// 셸 프레임 항목의 활성 상태를 즉시 다시 계산합니다. 도구 막대는 snapshot에서 바로 정합니다.
    func validateItems() {
        toolBar.isEnabled = snapshot.status == .ready && !builtTools.isEmpty
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
        // 사이드바·제목 영역(앞쪽 끝 고정 항목 앞)에는 다른 항목을 넣지 않습니다.
        Self.immovableItemIdentifiers.contains(itemIdentifier) || index >= Self.immovableItemIdentifiers.count
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        // `flag == false`는 사용자 구성 palette에 보일 사본입니다. 실제 제목 뷰는 툴바 항목 하나에만 둡니다.
        if let itemProvider, itemProvider.providesItem(itemIdentifier) {
            return itemProvider.item(for: itemIdentifier, willBeInsertedIntoToolbar: flag)
        }
        switch itemIdentifier {
        case Self.titleItem:
            return makeTitleItem(binds: flag)
        default:
            return nil
        }
    }

    // MARK: - 항목 생성

    private func rebuildTools() {
        builtTools = snapshot.tools
        toolBar.setTools(builtTools)
        syncToolSelection()
    }

    private func makeTitleItem(binds: Bool) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: Self.titleItem)
        item.label = ShellFrameStrings.title
        item.paletteLabel = ShellFrameStrings.title
        // 창 제목 자리(앞쪽)에 놓고, 툴바 단추 모양을 쓰지 않습니다.
        item.isNavigational = true
        item.isBordered = false
        item.visibilityPriority = .high
        if binds {
            item.view = titleView
        } else {
            let copy = ShellTitleMenuView()
            copy.apply(title: snapshot.title, subtitle: snapshot.subtitle, actions: [])
            item.view = copy
        }
        return item
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
}

/// 상단 바 앞쪽 제목(프로젝트 이름)·부제목과 제목 문서 메뉴(UX 명세 3.3).
///
/// 제목은 pull-down 단추라 누르거나 키보드·VoiceOver로 누르면 문서 메뉴가 열립니다. 부제목은 누를 수 없는 표시입니다.
/// 메뉴 항목은 셸 프레임 snapshot의 `moreActions`이고, 고르면 `actionHandler`로 명령 ID를 보냅니다.
final class ShellTitleMenuView: NSView {
    let titleButton = NSPopUpButton(frame: .zero, pullsDown: true)
    let subtitleField = NSTextField(labelWithString: "")
    /// 메뉴 항목을 고르면 그 명령 ID로 부릅니다.
    var actionHandler: ((ShellCommandID) -> Void)?
    private let stack = NSStackView()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        titleButton.isBordered = false
        // 시스템 툴바 제목(`window.title`)과 같은 모양입니다: 제목은 bold 시스템 글꼴 13pt, 부제목은 일반 11pt
        // secondaryLabelColor(macOS 27.2에서 시스템 제목·부제목 필드의 글꼴을 읽어 맞춘 값). `NSFont.titleBarFont`는
        // headline 굵기라 시스템 툴바 제목과 다릅니다.
        titleButton.font = .boldSystemFont(ofSize: NSFont.systemFontSize)
        titleButton.lineBreakMode = .byTruncatingTail
        (titleButton.cell as? NSPopUpButtonCell)?.arrowPosition = .arrowAtCenter
        titleButton.autoenablesItems = false
        titleButton.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        titleButton.toolTip = ShellFrameStrings.documentMenu

        subtitleField.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        subtitleField.textColor = .secondaryLabelColor
        subtitleField.lineBreakMode = .byTruncatingTail
        subtitleField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(titleButton)
        stack.addArrangedSubview(subtitleField)
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stack.topAnchor.constraint(greaterThanOrEqualTo: topAnchor),
            widthAnchor.constraint(lessThanOrEqualToConstant: ShellFrameMetrics.titleMaxWidth),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellTitleMenuView는 코드로만 생성합니다")
    }

    /// 제목·부제목·메뉴 항목을 바꿉니다. pull-down 단추는 첫 항목 제목을 단추 제목으로 보입니다.
    func apply(title: String, subtitle: String, actions: [ShellActionDescriptor]) {
        let menu = NSMenu(title: ShellFrameStrings.documentMenu)
        menu.autoenablesItems = false
        menu.addItem(NSMenuItem(title: title, action: nil, keyEquivalent: ""))
        for action in actions {
            let item = NSMenuItem(title: action.title, action: #selector(actionChosen(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = action.id.rawValue
            item.isEnabled = action.isEnabled
            menu.addItem(item)
        }
        titleButton.menu = menu
        titleButton.isEnabled = !actions.isEmpty
        titleButton.setAccessibilityLabel(title)
        subtitleField.stringValue = subtitle
        subtitleField.isHidden = subtitle.isEmpty
    }

    @objc private func actionChosen(_ sender: NSMenuItem) {
        guard let rawValue = sender.representedObject as? String else { return }
        actionHandler?(ShellCommandID(rawValue))
    }
}

/// 셸 툴바에 항목을 내주는 다른 영역(가이드 6.1).
///
/// 항목 생성과 상태 반영은 provider가 맡습니다. `willBeInsertedIntoToolbar`가 false이면 사용자 구성 palette용
/// 사본을 돌려줘야 합니다(실제 항목 인스턴스를 palette에 넘기지 않음). 도구 영역은 툴바가 아니라 캔버스 하단 가운데
/// 도구 막대(`ShellToolBarView`)에 있으므로 provider는 가운데 항목을 내주지 않습니다(2026-11-14 사용자 결정).
protocol ShellToolbarItemProviding: AnyObject {
    /// 인스펙터 경계 앞에 놓을 뒤쪽 항목.
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
