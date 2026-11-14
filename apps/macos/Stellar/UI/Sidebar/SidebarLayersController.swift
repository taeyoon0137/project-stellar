import AppKit

/// 레이어 트리(사이드바 하단 구획).
///
/// - 깊이 제한 없는 `NSOutlineView` disclosure 트리(디자인 가이드 6.2, HIG Outline views).
/// - 다중 선택은 창별 선택 상태와 같습니다(VII.3). 사이드바 선택 변경은 `.setSelection`으로 보내고,
///   캔버스 등 외부에서 바뀐 선택은 조상을 펼쳐 보이게 합니다.
/// - 인라인 이름 변경, 표시·잠금 토글, 문맥 메뉴, 드래그 재정렬·재부모화(드롭 사전 검증 포함).
/// - snapshot 반영: 구조 번호가 같으면 이미 만들어진 행만 갱신하고, 다르면 목록을 다시 읽습니다(아키텍처 §6.3).
@MainActor
final class SidebarLayersController: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate,
    SidebarOutlineActionHandling, SidebarLayerCellDelegate {
    static let dragType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.sidebar-layer")

    let scrollView: NSScrollView
    let outline: SidebarOutlineView
    private let provider: SidebarDataProvider
    private(set) var snapshot: SidebarSnapshot = .empty

    private let header = SidebarOutlineItem(role: .header, id: nil)
    private var items: [SidebarStableID: SidebarOutlineItem] = [:]
    private var moreItems: [SidebarStableID?: SidebarOutlineItem] = [:]
    /// 사용자가 펼친 레이어. 구조 reload 뒤 복원합니다.
    private(set) var expandedIDs: Set<SidebarStableID> = []
    /// 필터를 켜기 전 펼침 상태. 필터를 끄면 복원합니다.
    private var expandedBeforeFilter: Set<SidebarStableID>?
    private var isApplyingSnapshot = false
    private var pendingSnapshot: SidebarSnapshot?
    /// 마지막으로 보낸(또는 반영한) 선택. snapshot 선택이 이것과 다르면 외부 변경으로 봅니다.
    private var knownSelection: [SidebarStableID] = []
    private var draggedIDs: [SidebarStableID] = []
    private let rename = SidebarInlineRename()
    private let contextMenu = NSMenu()
    /// 문맥 메뉴가 열린 동안 동작 대상.
    private var menuTargets: [SidebarStableID] = []

    /// `true`이면 행의 표시·잠금 토글을 항상 보입니다(대비 높이기).
    var showsTogglesAlways = false {
        didSet {
            guard oldValue != showsTogglesAlways else { return }
            forEachAvailableLayerCell { cell, _ in cell.showsTogglesAlways = showsTogglesAlways }
        }
    }

    init(provider: SidebarDataProvider) {
        self.provider = provider
        (scrollView, outline) = SidebarOutlineFactory.make(
            accessibilityLabel: SidebarStrings.layersOutline,
            allowsMultipleSelection: true
        )
        super.init()
        outline.dataSource = self
        outline.delegate = self
        outline.actionHandler = self
        outline.target = self
        outline.doubleAction = #selector(doubleClicked(_:))
        outline.registerForDraggedTypes([Self.dragType])
        outline.setDraggingSourceOperationMask(.move, forLocal: true)
        outline.setDraggingSourceOperationMask([], forLocal: false)

        contextMenu.delegate = self
        contextMenu.autoenablesItems = false
        outline.menu = contextMenu

        rename.outline = outline
        rename.originalValue = { [weak self] field in self?.row(of: field)?.name }
        rename.commit = { [weak self] field, value in self?.commitRename(field, value: value) }
        rename.didEnd = { [weak self] in self?.schedulePendingSnapshot() }
    }

    // MARK: Snapshot 반영

    func apply(_ new: SidebarSnapshot) {
        guard new.generation >= snapshot.generation else { return }
        // 이름 편집 중에는 행을 다시 만들지 않습니다. 편집이 끝나면 최신 snapshot을 반영합니다.
        if SidebarInlineRename.isEditing(in: outline) {
            pendingSnapshot = new
            return
        }
        let old = snapshot
        snapshot = new
        isApplyingSnapshot = true
        defer { isApplyingSnapshot = false }

        let viewChanged = old.activeViewID != new.activeViewID
        if viewChanged {
            expandedIDs = []
            expandedBeforeFilter = nil
        }
        let filterTurnedOn = old.filter == nil && new.filter != nil
        let filterTurnedOff = old.filter != nil && new.filter == nil
        if filterTurnedOn, expandedBeforeFilter == nil {
            expandedBeforeFilter = expandedIDs
        }
        if filterTurnedOff, let saved = expandedBeforeFilter {
            expandedIDs = saved
            expandedBeforeFilter = nil
        }

        if viewChanged || old.generation == 0 || old.layers.structureRevision != new.layers.structureRevision {
            reloadStructure()
        } else {
            refreshAvailableRows(previous: old.layers)
        }
        syncSelection(previousTree: old.layers)
    }

    /// 편집 종료 알림은 field editor가 아직 first responder인 동안 오므로, 미뤄 둔 snapshot은 다음 런 루프에서
    /// 반영합니다(편집 중인 행을 종료 처리 도중에 다시 만들지 않기 위함).
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

    private func reloadStructure() {
        // snapshot에서 사라진 행의 항목 객체는 버립니다.
        items = items.filter { snapshot.layers.rows[$0.key] != nil }
        moreItems = moreItems.filter { snapshot.layers.moreParents.contains($0.key) }
        outline.reloadData()
        outline.expandItem(header)
        if snapshot.filter != nil {
            // 필터 결과는 일치 행과 그 조상만 담으므로 모두 펼쳐 보입니다.
            outline.expandItem(header, expandChildren: true)
        } else {
            restoreExpansion()
        }
    }

    /// 구조 reload 뒤 펼침 상태를 위에서부터 복원합니다(펼친 행의 자식 행이 이어서 검사됩니다).
    private func restoreExpansion() {
        var row = 0
        while row < outline.numberOfRows {
            if let item = outline.item(atRow: row) as? SidebarOutlineItem,
               let id = item.layerID,
               expandedIDs.contains(id) {
                if !outline.isItemExpanded(item) {
                    outline.expandItem(item)
                } else if snapshot.layers.rows[id]?.hasChildren == true, snapshot.layers.childIDs[id] == nil {
                    // AppKit이 펼침을 유지하면 새 문서 revision에서도 willExpand가 다시 오지 않습니다.
                    provider.perform(.loadChildren(id))
                }
            }
            row += 1
        }
    }

    /// 구조가 같을 때: 이미 만들어진 행 가운데 값이 바뀐 행만 제자리에서 다시 채웁니다.
    private func refreshAvailableRows(previous: SidebarLayerTree) {
        forEachAvailableLayerCell { cell, id in
            guard let row = snapshot.layers.rows[id], previous.rows[id] != row else { return }
            configure(cell, row: row)
        }
    }

    /// 목록이 만들어 둔 모든 레이어 행 셀을 돕니다.
    ///
    /// 보이는 범위(`visibleRect`)만 돌면 안 됩니다. NSTableView는 스크롤을 위해 보이는 범위 밖 행 뷰도 미리 만들어
    /// 두고(responsive scrolling overdraw), 그 행이 화면에 들어와도 `viewFor`를 다시 부르지 않기 때문입니다.
    private func forEachAvailableLayerCell(_ body: (SidebarLayerCellView, SidebarStableID) -> Void) {
        outline.enumerateAvailableRowViews { rowView, row in
            guard let id = (outline.item(atRow: row) as? SidebarOutlineItem)?.layerID,
                  let cell = rowView.view(atColumn: 0) as? SidebarLayerCellView else {
                return
            }
            body(cell, id)
        }
    }

    /// snapshot 선택을 목록에 반영합니다. 외부(캔버스 등)에서 바뀐 선택이면 조상을 펼치고 첫 행이 보이게 합니다.
    private func syncSelection(previousTree: SidebarLayerTree) {
        let ids = snapshot.selection
        let external = ids != knownSelection || ids.contains {
            previousTree.rows[$0] == nil && snapshot.layers.rows[$0] != nil
        }
        knownSelection = ids
        if external {
            for id in ids { revealAncestors(of: id) }
        }
        var indexes = IndexSet()
        for id in ids {
            guard let item = items[id] else { continue }
            let row = outline.row(forItem: item)
            if row >= 0 { indexes.insert(row) }
        }
        if indexes != outline.selectedRowIndexes {
            outline.selectRowIndexes(indexes, byExtendingSelection: false)
        }
        if external, let first = indexes.first {
            outline.scrollRowToVisible(first)
        }
    }

    private func revealAncestors(of id: SidebarStableID) {
        var chain: [SidebarStableID] = []
        var current = snapshot.layers.rows[id]?.parentID
        while let parent = current {
            chain.append(parent)
            current = snapshot.layers.rows[parent]?.parentID
        }
        for ancestor in chain.reversed() {
            let item = layerItem(ancestor)
            if outline.row(forItem: item) >= 0, !outline.isItemExpanded(item) {
                outline.expandItem(item)
            }
        }
    }

    // MARK: 항목

    private func layerItem(_ id: SidebarStableID) -> SidebarOutlineItem {
        if let item = items[id] { return item }
        let item = SidebarOutlineItem(role: .layer, id: id)
        items[id] = item
        return item
    }

    private func childIDs(of item: SidebarOutlineItem?) -> [SidebarStableID] {
        guard let item else { return [] }
        switch item.role {
        case .header: return snapshot.layers.topLevelIDs
        case .layer: return item.id.flatMap { snapshot.layers.childIDs[$0] } ?? []
        case .page, .view, .more: return []
        }
    }

    private var selectedIDs: [SidebarStableID] {
        outline.selectedRowIndexes.compactMap { (outline.item(atRow: $0) as? SidebarOutlineItem)?.layerID }
    }

    private func row(of field: NSTextField) -> SidebarLayerRow? {
        guard let cell = field.superview as? SidebarLayerCellView, let id = cell.itemID else { return nil }
        return snapshot.layers.rows[id]
    }

    private func displayName(_ row: SidebarLayerRow) -> String {
        row.name.isEmpty ? SidebarStrings.kindName(row.kind, shapeKind: row.shapeKind) : row.name
    }

    // MARK: NSOutlineViewDataSource

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? SidebarOutlineItem else { return 1 }
        let more = (item.role == .header || item.role == .layer) && snapshot.layers.moreParents.contains(item.id)
        return childIDs(of: item).count + (more ? 1 : 0)
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let item = item as? SidebarOutlineItem else { return header }
        let ids = childIDs(of: item)
        if ids.indices.contains(index) { return layerItem(ids[index]) }
        if let more = moreItems[item.id] { return more }
        let more = SidebarOutlineItem(role: .more, id: item.id)
        moreItems[item.id] = more
        return more
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let item = item as? SidebarOutlineItem else { return false }
        if item.role == .header { return true }
        return item.layerID.flatMap { snapshot.layers.rows[$0]?.hasChildren } ?? false
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
            cell.configure(title: SidebarStrings.layers)
            return cell
        }
        if item.role == .more {
            let reuseID = NSUserInterfaceItemIdentifier("SidebarMoreLayers")
            let button = outlineView.makeView(withIdentifier: reuseID, owner: nil) as? NSButton ??
                NSButton(title: String(localized: "Show More Layers"), target: self, action: #selector(showMore(_:)))
            button.identifier = reuseID
            button.bezelStyle = .inline
            button.controlSize = .small
            return button
        }
        guard let id = item.layerID, let row = snapshot.layers.rows[id] else { return nil }
        let cell = outlineView.makeView(withIdentifier: SidebarCellIdentifier.layer, owner: nil)
            as? SidebarLayerCellView ?? SidebarLayerCellView()
        cell.delegate = self
        cell.nameField.delegate = rename
        cell.showsTogglesAlways = showsTogglesAlways
        configure(cell, row: row)
        return cell
    }

    private func configure(_ cell: SidebarLayerCellView, row: SidebarLayerRow) {
        cell.configure(row: row)
        let id = row.id
        let actions: [(SidebarOutlineAction, String)] = [
            (.rename, SidebarStrings.rename),
            (.toggleVisibility, row.isVisible ? SidebarStrings.hide : SidebarStrings.show),
            (.toggleLock, row.isLocked ? SidebarStrings.unlock : SidebarStrings.lock),
            (.duplicate, SidebarStrings.duplicate),
            (.delete, SidebarStrings.delete),
        ]
        let allowed = actions.filter { canPerform($0.0, targets: [id]) }
        cell.nameField.isEditable = allowed.contains { $0.0 == .rename }
        cell.visibilityButton.isEnabled = allowed.contains { $0.0 == .toggleVisibility }
        cell.lockButton.isEnabled = allowed.contains { $0.0 == .toggleLock }
        cell.setAccessibilityCustomActions(allowed.map { action, title in
            NSAccessibilityCustomAction(name: title) { [weak self] in
                self?.perform(action, targets: [id]) ?? false
            }
        })
    }

    @objc private func showMore(_ sender: NSButton) {
        let row = outline.row(for: sender)
        guard row >= 0, let more = outline.item(atRow: row) as? SidebarOutlineItem, more.role == .more,
              provider.validate(.loadMore(more.id)).isAllowed else { return }
        provider.perform(.loadMore(more.id))
    }

    func outlineView(_ outlineView: NSOutlineView, typeSelectStringFor tableColumn: NSTableColumn?, item: Any) -> String? {
        guard let id = (item as? SidebarOutlineItem)?.layerID, let row = snapshot.layers.rows[id] else { return nil }
        return displayName(row)
    }

    func outlineView(_ outlineView: NSOutlineView, selectionIndexesForProposedSelection proposedSelectionIndexes: IndexSet) -> IndexSet {
        proposedSelectionIndexes.filteredIndexSet { row in
            (outlineView.item(atRow: row) as? SidebarOutlineItem)?.layerID != nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSnapshot else { return }
        let selected = selectedIDs
        // 선택 순서: 기존 순서를 유지하고 새로 더한 행은 행 순서로 뒤에 붙입니다.
        let selectedSet = Set(selected)
        var ordered = knownSelection.filter { selectedSet.contains($0) }
        let kept = Set(ordered)
        ordered.append(contentsOf: selected.filter { !kept.contains($0) })
        guard ordered != knownSelection else { return }
        let primary = (outline.item(atRow: outline.selectedRow) as? SidebarOutlineItem)?.layerID ?? ordered.last
        knownSelection = ordered
        provider.perform(.setSelection(ordered, primary: primary))
    }

    func outlineViewItemWillExpand(_ notification: Notification) {
        // 자식 목록이 아직 없으면 코어에 해당 부모의 자식 페이지를 요청합니다.
        guard let item = notification.userInfo?["NSObject"] as? SidebarOutlineItem, let id = item.layerID,
              snapshot.layers.rows[id]?.hasChildren == true, snapshot.layers.childIDs[id] == nil else {
            return
        }
        provider.perform(.loadChildren(id))
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard snapshot.filter == nil,
              let id = (notification.userInfo?["NSObject"] as? SidebarOutlineItem)?.layerID else { return }
        expandedIDs.insert(id)
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard snapshot.filter == nil,
              let id = (notification.userInfo?["NSObject"] as? SidebarOutlineItem)?.layerID else { return }
        expandedIDs.remove(id)
    }

    // MARK: 이름 변경

    @discardableResult
    func beginRename(_ id: SidebarStableID) -> Bool {
        guard canPerform(.rename, targets: [id]), let item = items[id] else { return false }
        let row = outline.row(forItem: item)
        guard row >= 0 else { return false }
        if !outline.selectedRowIndexes.contains(row) {
            outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        outline.scrollRowToVisible(row)
        outline.editColumn(0, row: row, with: nil, select: true)
        return true
    }

    private func commitRename(_ field: NSTextField, value: String) {
        guard let row = row(of: field), row.name != value else { return }
        provider.perform(.renameNode(row.id, name: value))
    }

    @objc private func doubleClicked(_ sender: Any?) {
        let row = outline.clickedRow
        guard row >= 0, !SidebarInlineRename.isEditing(in: outline),
              let id = (outline.item(atRow: row) as? SidebarOutlineItem)?.layerID else {
            return
        }
        beginRename(id)
    }

    // MARK: 동작

    /// 동작을 대상 행에 대한 command로 바꿉니다. 이름 변경은 검증용 command만 만듭니다.
    func command(for action: SidebarOutlineAction, targets: [SidebarStableID]) -> SidebarCommand? {
        guard !targets.isEmpty else { return nil }
        let rows = targets.compactMap { snapshot.layers.rows[$0] }
        switch action {
        case .rename:
            guard targets.count == 1, let row = rows.first else { return nil }
            return .renameNode(row.id, name: row.name)
        case .duplicate: return .duplicate(targets)
        case .delete: return .delete(targets)
        case .group: return .group(targets)
        case .ungroup: return .ungroup(targets)
        case .toggleVisibility:
            // 모두 보이면 숨기고, 하나라도 숨겨져 있으면 모두 보입니다.
            return .setVisible(targets, !rows.allSatisfy(\.isVisible))
        case .toggleLock:
            // 모두 잠겨 있으면 풀고, 하나라도 풀려 있으면 모두 잠급니다.
            return .setLocked(targets, !rows.allSatisfy(\.isLocked))
        }
    }

    func canPerform(_ action: SidebarOutlineAction, targets: [SidebarStableID]) -> Bool {
        guard let command = command(for: action, targets: targets) else { return false }
        return provider.validate(command).isAllowed
    }

    @discardableResult
    func perform(_ action: SidebarOutlineAction, targets: [SidebarStableID]) -> Bool {
        guard canPerform(action, targets: targets), let command = command(for: action, targets: targets) else {
            return false
        }
        if action == .rename {
            return beginRename(targets[0])
        }
        provider.perform(command)
        return true
    }

    func sidebarOutline(_ outline: SidebarOutlineView, canPerform action: SidebarOutlineAction) -> Bool {
        canPerform(action, targets: selectedIDs)
    }

    func sidebarOutline(_ outline: SidebarOutlineView, perform action: SidebarOutlineAction) {
        perform(action, targets: selectedIDs)
    }

    // MARK: SidebarLayerCellDelegate

    func layerCellDidToggleVisibility(_ cell: SidebarLayerCellView) {
        guard let id = cell.itemID else { return }
        perform(.toggleVisibility, targets: [id])
    }

    func layerCellDidToggleLock(_ cell: SidebarLayerCellView) {
        guard let id = cell.itemID else { return }
        perform(.toggleLock, targets: [id])
    }

    // MARK: 문맥 메뉴

    /// 클릭한 행이 선택에 속하면 선택 전체, 아니면 클릭한 행만 대상으로 합니다(macOS 목록 관례).
    private func contextTargets() -> [SidebarStableID] {
        let clicked = outline.clickedRow
        if clicked >= 0, let id = (outline.item(atRow: clicked) as? SidebarOutlineItem)?.layerID {
            return outline.selectedRowIndexes.contains(clicked) ? selectedIDs : [id]
        }
        return selectedIDs
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        fill(menu, targets: contextTargets())
    }

    /// 대상에 대한 문맥 메뉴 항목을 채웁니다. 활성화 여부는 provider 사전 검증으로 정합니다.
    ///
    /// 문맥 메뉴에는 단축키 표시를 넣지 않습니다(HIG Context menus). 단축키는 메뉴 막대 항목(XII.6.5)이 보입니다.
    func fill(_ menu: NSMenu, targets: [SidebarStableID]) {
        menu.removeAllItems()
        menuTargets = targets
        guard !targets.isEmpty else { return }
        let rows = targets.compactMap { snapshot.layers.rows[$0] }
        let allVisible = rows.allSatisfy(\.isVisible)
        let allLocked = rows.allSatisfy(\.isLocked)
        let groups: [[(SidebarOutlineAction, String)]] = [
            [(.rename, SidebarStrings.rename), (.duplicate, SidebarStrings.duplicate), (.delete, SidebarStrings.delete)],
            [(.group, SidebarStrings.group), (.ungroup, SidebarStrings.ungroup)],
            [
                (.toggleVisibility, allVisible ? SidebarStrings.hide : SidebarStrings.show),
                (.toggleLock, allLocked ? SidebarStrings.unlock : SidebarStrings.lock),
            ],
        ]
        for (index, group) in groups.enumerated() {
            if index > 0 { menu.addItem(.separator()) }
            for (action, title) in group {
                let item = NSMenuItem(title: title, action: #selector(contextAction(_:)), keyEquivalent: "")
                item.target = self
                item.tag = SidebarOutlineAction.allCases.firstIndex(of: action) ?? 0
                item.isEnabled = canPerform(action, targets: targets)
                menu.addItem(item)
            }
        }
    }

    @objc private func contextAction(_ sender: NSMenuItem) {
        let actions = SidebarOutlineAction.allCases
        guard actions.indices.contains(sender.tag) else { return }
        perform(actions[sender.tag], targets: menuTargets)
    }

    // MARK: 드래그 재정렬·재부모화

    func outlineView(_ outlineView: NSOutlineView, pasteboardWriterForItem item: Any) -> NSPasteboardWriting? {
        // 필터 결과의 표시 순서는 실제 순서와 다르므로 필터 중에는 드래그를 시작하지 않습니다.
        guard snapshot.filter == nil, let id = (item as? SidebarOutlineItem)?.layerID,
              let row = snapshot.layers.rows[id],
              provider.validate(.move([id], to: SidebarDropTarget(parentID: row.parentID, index: 0))).isAllowed
        else { return nil }
        let pasteboardItem = NSPasteboardItem()
        pasteboardItem.setString(id.stringValue, forType: Self.dragType)
        return pasteboardItem
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        draggingSession session: NSDraggingSession,
        willBeginAt screenPoint: NSPoint,
        forItems draggedItems: [Any]
    ) {
        draggedIDs = draggedItems.compactMap { ($0 as? SidebarOutlineItem)?.layerID }
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        draggingSession session: NSDraggingSession,
        endedAt screenPoint: NSPoint,
        operation: NSDragOperation
    ) {
        draggedIDs = []
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        validateDrop info: NSDraggingInfo,
        proposedItem item: Any?,
        proposedChildIndex index: Int
    ) -> NSDragOperation {
        guard (info.draggingSource as? NSOutlineView) === outline,
              let resolved = resolveDrop(ids: draggedIDs, proposedItem: item as? SidebarOutlineItem, childIndex: index) else {
            return []
        }
        if resolved.item !== (item as? SidebarOutlineItem) || resolved.childIndex != index {
            outlineView.setDropItem(resolved.item, dropChildIndex: resolved.childIndex)
        }
        return .move
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        acceptDrop info: NSDraggingInfo,
        item: Any?,
        childIndex index: Int
    ) -> Bool {
        guard (info.draggingSource as? NSOutlineView) === outline,
              let resolved = resolveDrop(ids: draggedIDs, proposedItem: item as? SidebarOutlineItem, childIndex: index) else {
            return false
        }
        provider.perform(.move(draggedIDs, to: resolved.target))
        return true
    }

    /// 드롭 제안을 코어 좌표(부모 id, 표시 순서 index)로 바꾸고 provider로 사전 검증합니다.
    ///
    /// 행 "위"에 놓기(`childIndex == -1`)가 거부되면(자식을 못 가지는 종류 등) 그 행 바로 위 형제 자리로
    /// 바꿔 다시 검증합니다. 둘 다 거부되면 `nil`입니다.
    func resolveDrop(
        ids: [SidebarStableID],
        proposedItem: SidebarOutlineItem?,
        childIndex: Int
    ) -> (target: SidebarDropTarget, item: SidebarOutlineItem, childIndex: Int)? {
        guard !ids.isEmpty, snapshot.filter == nil else { return nil }
        let candidate: (SidebarDropTarget, SidebarOutlineItem, Int)
        switch proposedItem?.role {
        case nil:
            // outline 최상위(제목 행 위·아래)는 레이어 최상위의 처음·끝으로 바꿉니다.
            let index = childIndex <= 0 ? 0 : snapshot.layers.topLevelIDs.count
            candidate = (SidebarDropTarget(parentID: nil, index: index), header, index)
        case .header?:
            let index = max(0, childIndex)
            candidate = (SidebarDropTarget(parentID: nil, index: index), header, index)
        case .layer?:
            guard let item = proposedItem, let id = item.id else { return nil }
            let index = max(0, childIndex)
            candidate = (SidebarDropTarget(parentID: id, index: index), item, index)
        case .page?, .view?, .more?:
            return nil
        }
        if provider.validate(.move(ids, to: candidate.0)).isAllowed {
            return candidate
        }
        guard childIndex == -1, let id = proposedItem?.layerID, let row = snapshot.layers.rows[id] else { return nil }
        let siblings = row.parentID.map { snapshot.layers.childIDs[$0] ?? [] } ?? snapshot.layers.topLevelIDs
        guard let position = siblings.firstIndex(of: id) else { return nil }
        let target = SidebarDropTarget(parentID: row.parentID, index: position)
        guard provider.validate(.move(ids, to: target)).isAllowed else { return nil }
        let parentItem = row.parentID.map { layerItem($0) } ?? header
        return (target, parentItem, position)
    }
}
