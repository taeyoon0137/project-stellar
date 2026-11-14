import AppKit

/// outline 항목 객체.
///
/// NSOutlineView는 항목을 객체 동일성으로 추적하므로(펼침·선택 유지) id마다 같은 객체를 재사용합니다.
final class SidebarOutlineItem: NSObject {
    enum Role {
        case header
        case page
        case view
        case layer
        case more
    }

    let role: Role
    let id: SidebarStableID?

    init(role: Role, id: SidebarStableID?) {
        self.role = role
        self.id = id
    }

    var layerID: SidebarStableID? { role == .layer ? id : nil }
}

/// 사이드바 outline 공통 구성.
enum SidebarOutlineFactory {
    /// source list(사이드바) 스타일 outline과 그것을 담은 scroll view를 만듭니다(HIG Sidebars, 디자인 가이드 6.2).
    @MainActor
    static func make(accessibilityLabel: String, allowsMultipleSelection: Bool) -> (NSScrollView, SidebarOutlineView) {
        let outline = SidebarOutlineView()
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("SidebarColumn"))
        column.isEditable = true
        column.resizingMask = .autoresizingMask
        outline.addTableColumn(column)
        outline.outlineTableColumn = column
        outline.headerView = nil
        outline.style = .sourceList
        // 행 높이·글꼴은 시스템 사이드바 크기 설정을 따릅니다(디자인 가이드 4.4).
        outline.rowSizeStyle = .default
        outline.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        outline.floatsGroupRows = true
        outline.allowsMultipleSelection = allowsMultipleSelection
        outline.allowsEmptySelection = true
        outline.allowsTypeSelect = true
        outline.setAccessibilityLabel(accessibilityLabel)

        let scrollView = NSScrollView()
        scrollView.documentView = outline
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        // 사이드바 재질은 split view item이 그립니다. 목록 배경을 따로 칠하지 않습니다(디자인 가이드 3.2).
        scrollView.drawsBackground = false
        return (scrollView, outline)
    }
}

/// 페이지·View 목록(사이드바 상단 구획).
///
/// HIG Sidebars의 "두 단계 이내"에 맞춰 페이지 → View 두 단계만 둡니다(디자인 가이드 6.2).
/// 선택은 하나이며, 선택이 곧 active 페이지·View입니다.
@MainActor
final class SidebarPagesController: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate,
    SidebarOutlineActionHandling {
    let scrollView: NSScrollView
    let outline: SidebarOutlineView
    private let provider: SidebarDataProvider
    private(set) var snapshot: SidebarSnapshot = .empty

    private let header = SidebarOutlineItem(role: .header, id: nil)
    private var pageItems: [SidebarStableID: SidebarOutlineItem] = [:]
    private var viewItems: [SidebarStableID: SidebarOutlineItem] = [:]
    /// 사용자가 접은 페이지. 나머지 페이지는 펼쳐 둡니다.
    private var collapsedPageIDs: Set<SidebarStableID> = []
    private var isApplyingSnapshot = false
    private var pendingSnapshot: SidebarSnapshot?
    private let rename = SidebarInlineRename()
    private let contextMenu = NSMenu()

    init(provider: SidebarDataProvider) {
        self.provider = provider
        (scrollView, outline) = SidebarOutlineFactory.make(
            accessibilityLabel: SidebarStrings.pagesOutline,
            allowsMultipleSelection: false
        )
        super.init()
        outline.dataSource = self
        outline.delegate = self
        outline.actionHandler = self
        outline.target = self
        outline.doubleAction = #selector(doubleClicked(_:))
        contextMenu.delegate = self
        contextMenu.autoenablesItems = false
        outline.menu = contextMenu

        rename.outline = outline
        rename.originalValue = { [weak self] field in self?.currentName(of: field) }
        rename.commit = { [weak self] field, value in self?.commitRename(field, value: value) }
        rename.didEnd = { [weak self] in self?.schedulePendingSnapshot() }
    }

    // MARK: Snapshot

    func apply(_ new: SidebarSnapshot) {
        guard new.generation >= snapshot.generation else { return }
        if SidebarInlineRename.isEditing(in: outline) {
            pendingSnapshot = new
            return
        }
        let old = snapshot
        snapshot = new
        isApplyingSnapshot = true
        defer { isApplyingSnapshot = false }

        // active 페이지·View가 바뀐 snapshot에서만 선택을 active View 행으로 옮깁니다. 그 밖에는 사용자가 고른
        // active 페이지 행 선택을 그대로 둡니다(키보드로 그 행을 지나갈 수 있게). 목록 reload가 선택을 비우므로
        // 그 전에 기억해 둡니다.
        let activeChanged = old.generation == 0 || old.activePageID != new.activePageID || old.activeViewID != new.activeViewID
        let keptPageItem = activeChanged ? nil : selectedActivePageItem

        if old.pages != new.pages || old.generation == 0 {
            let pageIDs = Set(new.pages.map(\.id))
            let viewIDs = Set(new.pages.flatMap { $0.views.map(\.id) })
            pageItems = pageItems.filter { pageIDs.contains($0.key) }
            viewItems = viewItems.filter { viewIDs.contains($0.key) }
            outline.reloadData()
            outline.expandItem(header)
            for page in new.pages where !collapsedPageIDs.contains(page.id) {
                outline.expandItem(item(page: page.id))
            }
        }
        let keptRow = keptPageItem.map { outline.row(forItem: $0) } ?? -1
        if keptRow >= 0 {
            if outline.selectedRowIndexes != IndexSet(integer: keptRow) {
                outline.selectRowIndexes(IndexSet(integer: keptRow), byExtendingSelection: false)
            }
        } else {
            syncSelection()
        }
    }

    /// 선택이 active 페이지 행 하나이면 그 항목.
    private var selectedActivePageItem: SidebarOutlineItem? {
        guard outline.selectedRowIndexes.count == 1,
              let item = outline.item(atRow: outline.selectedRow) as? SidebarOutlineItem,
              item.role == .page, item.id == snapshot.activePageID else {
            return nil
        }
        return item
    }

    /// 편집 종료 알림은 field editor가 아직 first responder인 동안 오므로 다음 런 루프에서 반영합니다.
    private func schedulePendingSnapshot() {
        guard pendingSnapshot != nil else { return }
        NSObject.cancelPreviousPerformRequests(withTarget: self, selector: #selector(applyPendingSnapshot), object: nil)
        perform(#selector(applyPendingSnapshot), with: nil, afterDelay: 0, inModes: [.common])
    }

    @objc private func applyPendingSnapshot() {
        guard let pending = pendingSnapshot else { return }
        pendingSnapshot = nil
        apply(pending)
    }

    private func item(page id: SidebarStableID) -> SidebarOutlineItem {
        if let item = pageItems[id] { return item }
        let item = SidebarOutlineItem(role: .page, id: id)
        pageItems[id] = item
        return item
    }

    private func item(view id: SidebarStableID) -> SidebarOutlineItem {
        if let item = viewItems[id] { return item }
        let item = SidebarOutlineItem(role: .view, id: id)
        viewItems[id] = item
        return item
    }

    /// active View 행(페이지가 펼쳐져 있을 때) 또는 active 페이지 행을 선택합니다.
    ///
    /// 페이지 행을 눌러 그 페이지가 active가 되어도, 코어가 고른 active View 행이 보이면 그 행으로 선택을 옮깁니다
    /// (선택 = active 페이지·View).
    private func syncSelection() {
        var row = -1
        if let viewID = snapshot.activeViewID, let item = viewItems[viewID] {
            row = outline.row(forItem: item)
        }
        if row < 0, let pageID = snapshot.activePageID, let item = pageItems[pageID] {
            row = outline.row(forItem: item)
        }
        if row >= 0 {
            if outline.selectedRowIndexes != IndexSet(integer: row) {
                outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            }
        } else if !outline.selectedRowIndexes.isEmpty {
            outline.deselectAll(nil)
        }
    }

    /// 사용자 조작 뒤 선택 표시를 active 상태에 다시 맞춥니다(command를 만들지 않음).
    private func resyncSelection() {
        isApplyingSnapshot = true
        defer { isApplyingSnapshot = false }
        syncSelection()
    }

    // MARK: NSOutlineViewDataSource

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? SidebarOutlineItem else { return 1 }
        switch item.role {
        case .header: return snapshot.pages.count
        case .page: return page(item.id)?.views.count ?? 0
        case .view, .layer, .more: return 0
        }
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let item = item as? SidebarOutlineItem else { return header }
        if item.role == .header, snapshot.pages.indices.contains(index) {
            return self.item(page: snapshot.pages[index].id)
        }
        if let views = page(item.id)?.views, views.indices.contains(index) {
            return self.item(view: views[index].id)
        }
        // numberOfChildren와 같은 snapshot을 읽으므로 도달하지 않습니다.
        return SidebarOutlineItem(role: .view, id: nil)
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let item = item as? SidebarOutlineItem else { return false }
        switch item.role {
        case .header: return true
        case .page: return !(page(item.id)?.views.isEmpty ?? true)
        case .view, .layer, .more: return false
        }
    }

    private func page(_ id: SidebarStableID?) -> SidebarPageRow? {
        snapshot.pages.first { $0.id == id }
    }

    private func view(_ id: SidebarStableID?) -> SidebarViewRow? {
        for page in snapshot.pages {
            if let view = page.views.first(where: { $0.id == id }) { return view }
        }
        return nil
    }

    // MARK: NSOutlineViewDelegate

    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
        (item as? SidebarOutlineItem)?.role == .header
    }

    func outlineView(_ outlineView: NSOutlineView, shouldCollapseItem item: Any) -> Bool {
        (item as? SidebarOutlineItem)?.role != .header
    }

    func outlineView(_ outlineView: NSOutlineView, shouldShowOutlineCellForItem item: Any) -> Bool {
        (item as? SidebarOutlineItem)?.role != .header
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? SidebarOutlineItem else { return nil }
        if item.role == .header {
            let cell = outlineView.makeView(withIdentifier: SidebarCellIdentifier.header, owner: nil)
                as? SidebarHeaderCellView ?? SidebarHeaderCellView()
            cell.configure(title: SidebarStrings.pages)
            return cell
        }
        let cell = outlineView.makeView(withIdentifier: SidebarCellIdentifier.item, owner: nil)
            as? SidebarNameCellView
            ?? SidebarNameCellView(identifier: SidebarCellIdentifier.item, trailingAccessories: [])
        cell.nameField.delegate = rename
        if item.role == .page, let page = page(item.id) {
            cell.configure(
                id: page.id,
                name: page.name,
                placeholder: SidebarStrings.pageKind,
                symbol: SidebarSymbols.page,
                kindLabel: SidebarStrings.pageKind
            )
        } else if let view = view(item.id) {
            cell.configure(
                id: view.id,
                name: view.name,
                placeholder: SidebarStrings.viewKind,
                symbol: SidebarSymbols.view,
                kindLabel: SidebarStrings.viewKind
            )
        }
        if let id = item.id {
            let canRename = canRename(id: id, role: item.role)
            cell.nameField.isEditable = canRename
            cell.setAccessibilityCustomActions(canRename ? [
                NSAccessibilityCustomAction(name: SidebarStrings.rename) { [weak self] in
                    self?.beginRename(id: id, role: item.role) ?? false
                },
            ] : [])
        }
        return cell
    }

    func outlineView(_ outlineView: NSOutlineView, typeSelectStringFor tableColumn: NSTableColumn?, item: Any) -> String? {
        guard let item = item as? SidebarOutlineItem else { return nil }
        switch item.role {
        case .page: return page(item.id)?.name
        case .view: return view(item.id)?.name
        case .header, .layer, .more: return nil
        }
    }

    func outlineView(_ outlineView: NSOutlineView, selectionIndexesForProposedSelection proposedSelectionIndexes: IndexSet) -> IndexSet {
        proposedSelectionIndexes.filteredIndexSet { row in
            (outlineView.item(atRow: row) as? SidebarOutlineItem)?.role != .header
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSnapshot else { return }
        guard let item = outline.item(atRow: outline.selectedRow) as? SidebarOutlineItem, let id = item.id else {
            // 빈 곳 클릭 등으로 선택이 비면 active 페이지·View 표시를 되살립니다(선택이 곧 active 상태).
            resyncSelection()
            return
        }
        switch item.role {
        case .page where id != snapshot.activePageID || snapshot.activeViewID == nil:
            provider.perform(.activatePage(id))
        case .view where id != snapshot.activeViewID:
            provider.perform(.activateView(id))
        default:
            // 이미 active인 페이지·View 행을 고르면 바뀌는 것이 없습니다. 선택을 되돌리지 않습니다: active 페이지 행도
            // active 상태를 나타내며, 되돌리면 화살표·← 이동이 그 행에서 막힙니다(키보드 함정).
            break
        }
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        if let item = notification.userInfo?["NSObject"] as? SidebarOutlineItem, item.role == .page, let id = item.id {
            collapsedPageIDs.insert(id)
        }
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        if let item = notification.userInfo?["NSObject"] as? SidebarOutlineItem, item.role == .page, let id = item.id {
            collapsedPageIDs.remove(id)
        }
    }

    // MARK: 이름 변경

    private func canRename(id: SidebarStableID, role: SidebarOutlineItem.Role) -> Bool {
        provider.validate(role == .page ? .renamePage(id, name: "") : .renameView(id, name: "")).isAllowed
    }

    @discardableResult
    func beginRename(id: SidebarStableID, role: SidebarOutlineItem.Role) -> Bool {
        guard canRename(id: id, role: role) else { return false }
        let item = role == .page ? pageItems[id] : viewItems[id]
        guard let item else { return false }
        let row = outline.row(forItem: item)
        guard row >= 0 else { return false }
        if outline.selectedRow != row {
            outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        outline.scrollRowToVisible(row)
        outline.editColumn(0, row: row, with: nil, select: true)
        return true
    }

    private func currentName(of field: NSTextField) -> String? {
        guard let cell = field.superview as? SidebarNameCellView, let id = cell.itemID else { return nil }
        return page(id)?.name ?? view(id)?.name
    }

    private func commitRename(_ field: NSTextField, value: String) {
        guard let cell = field.superview as? SidebarNameCellView, let id = cell.itemID else { return }
        if let page = page(id), page.name != value {
            provider.perform(.renamePage(id, name: value))
        } else if let view = view(id), view.name != value {
            provider.perform(.renameView(id, name: value))
        }
    }

    @objc private func doubleClicked(_ sender: Any?) {
        let row = outline.clickedRow
        guard row >= 0, !SidebarInlineRename.isEditing(in: outline),
              let item = outline.item(atRow: row) as? SidebarOutlineItem, let id = item.id else {
            return
        }
        beginRename(id: id, role: item.role)
    }

    // MARK: SidebarOutlineActionHandling

    func sidebarOutline(_ outline: SidebarOutlineView, canPerform action: SidebarOutlineAction) -> Bool {
        guard action == .rename, let item = outline.item(atRow: outline.selectedRow) as? SidebarOutlineItem,
              let id = item.id else {
            return false
        }
        return canRename(id: id, role: item.role)
    }

    func sidebarOutline(_ outline: SidebarOutlineView, perform action: SidebarOutlineAction) {
        guard action == .rename, let item = outline.item(atRow: outline.selectedRow) as? SidebarOutlineItem,
              let id = item.id else {
            return
        }
        beginRename(id: id, role: item.role)
    }

    // MARK: 문맥 메뉴

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let row = outline.clickedRow
        guard row >= 0, let item = outline.item(atRow: row) as? SidebarOutlineItem, let id = item.id else { return }
        // 문맥 메뉴에는 단축키 표시를 넣지 않습니다(HIG Context menus).
        let menuItem = NSMenuItem(title: SidebarStrings.rename, action: #selector(renameFromMenu(_:)), keyEquivalent: "")
        menuItem.target = self
        menuItem.representedObject = item
        menuItem.isEnabled = canRename(id: id, role: item.role)
        menu.addItem(menuItem)
    }

    @objc private func renameFromMenu(_ sender: NSMenuItem) {
        guard let item = sender.representedObject as? SidebarOutlineItem, let id = item.id else { return }
        beginRename(id: id, role: item.role)
    }
}
