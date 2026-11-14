import AppKit

/// Modifier stack 섹션(아키텍처 §12.6, VI.9).
///
/// 표준 `NSTableView` 목록으로 순서·활성 상태를 보여 주고, 선택한 항목의 파라미터를 목록 아래 행으로
/// 편집합니다. 재정렬은 드래그 또는 Command-Option-위/아래, 삭제는 Delete, 활성 전환은 Space입니다.
/// 순서·id·검증은 코어가 맡고 셸은 명령만 보냅니다.
@MainActor
final class InspectorModifierStackController: NSObject, NSTableViewDataSource, NSTableViewDelegate,
    NSMenuItemValidation
{
    /// 드래그 재정렬용 앱 내부 pasteboard 형식.
    static let dragType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.inspector.modifier")

    let sectionView: InspectorSectionView
    let tableView = InspectorModifierTableView()
    let addButton = NSPopUpButton(frame: .zero, pullsDown: true)
    let removeButton: NSButton
    private let tableScrollView = InspectorEmbeddedScrollView()
    private var tableHeight: NSLayoutConstraint?
    private let emptyLabel = InspectorWrappingLabel(text: "No modifiers. Use the Add button to add one.")
    private let unavailableLabel = InspectorWrappingLabel(text: "")
    private let detailRows: InspectorRowsView
    private(set) var stack = InspectorModifierStack(availability: .available, items: [], addableKinds: [],
                                                    isEditable: false)
    private(set) var selectedID: String?
    private var forceDisabled = false
    private var hasApplied = false
    private let send: (InspectorCommand) -> Void
    /// 선택·구성이 바뀌어 키보드 순서를 다시 계산해야 할 때 호출됩니다.
    var onStructureChange: (() -> Void)?

    init(expanded: Bool, send: @escaping (InspectorCommand) -> Void) {
        self.send = send
        sectionView = InspectorSectionView(id: .modifiers, title: "Modifiers", expanded: expanded)
        removeButton = NSButton()
        detailRows = InspectorRowsView(send: send)
        super.init()
        build()
    }

    private var isInteractive: Bool { stack.isEditable && !forceDisabled }

    // MARK: - 구성

    private func build() {
        addButton.controlSize = InspectorMetrics.controlSize
        addButton.autoenablesItems = false
        addButton.setAccessibilityLabel("Add Modifier")
        addButton.toolTip = "Add Modifier"
        addButton.identifier = NSUserInterfaceItemIdentifier("modifiers.add")
        InspectorFieldController.hug(addButton)

        if let image = InspectorSymbols.image(InspectorSymbols.remove, label: "Remove Modifier") {
            removeButton.image = image
            removeButton.imagePosition = .imageOnly
        }
        removeButton.title = ""
        removeButton.bezelStyle = .push
        // modifier 삭제는 파괴 동작입니다(가이드 5.1).
        removeButton.hasDestructiveAction = true
        removeButton.controlSize = InspectorMetrics.controlSize
        removeButton.target = self
        removeButton.action = #selector(removeSelected(_:))
        removeButton.setAccessibilityLabel("Remove Modifier")
        removeButton.toolTip = "Remove Modifier"
        removeButton.identifier = NSUserInterfaceItemIdentifier("modifiers.remove")
        InspectorFieldController.hug(removeButton)
        sectionView.setAccessoryViews([addButton, removeButton])

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("modifier"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .plain
        tableView.rowSizeStyle = .small
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        tableView.allowsMultipleSelection = false
        tableView.allowsEmptySelection = true
        tableView.dataSource = self
        tableView.delegate = self
        tableView.registerForDraggedTypes([Self.dragType])
        tableView.setDraggingSourceOperationMask(.move, forLocal: true)
        tableView.setAccessibilityLabel("Modifier Stack")
        tableView.identifier = NSUserInterfaceItemIdentifier("modifiers.table")
        tableView.keyHandler = { [weak self] event in self?.handleKey(event) ?? false }
        tableView.menu = makeContextMenu()

        tableScrollView.documentView = tableView
        tableScrollView.hasVerticalScroller = false
        tableScrollView.hasHorizontalScroller = false
        tableScrollView.borderType = .lineBorder
        tableScrollView.translatesAutoresizingMaskIntoConstraints = false
        let height = tableScrollView.heightAnchor.constraint(equalToConstant: 0)
        height.isActive = true
        tableHeight = height

        sectionView.setContent([unavailableLabel, emptyLabel, tableScrollView, detailRows])
    }

    private func makeContextMenu() -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = true
        let items: [(String, Selector)] = [
            ("Move Up", #selector(moveUpPressed(_:))),
            ("Move Down", #selector(moveDownPressed(_:))),
            ("", #selector(toggleEnabledPressed(_:))),
            ("Remove", #selector(removeClicked(_:))),
        ]
        for (index, (title, action)) in items.enumerated() {
            if index == 2 { menu.addItem(.separator()) }
            let item = NSMenuItem(title: title.isEmpty ? "Disable" : title, action: action, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        return menu
    }

    // MARK: - snapshot 반영

    /// stack snapshot을 반영합니다. 선택한 항목은 id로 유지합니다.
    func apply(_ newStack: InspectorModifierStack, forceDisabled: Bool) {
        if hasApplied, newStack == stack, forceDisabled == self.forceDisabled {
            // 바뀐 것이 없으면 목록을 다시 그리지 않습니다. 파라미터 필드의 낙관적 표시만 snapshot 값으로 맞춥니다.
            refreshDetail()
            return
        }
        hasApplied = true
        let previousIDs = stack.items.map(\.id)
        stack = newStack
        self.forceDisabled = forceDisabled

        rebuildAddMenu()
        switch newStack.availability {
        case .unavailable(let reason):
            unavailableLabel.stringValue = reason
            unavailableLabel.isHidden = false
            emptyLabel.isHidden = true
            tableScrollView.isHidden = true
            detailRows.isHidden = true
            sectionView.setAccessoryViews([])
            selectedID = nil
            onStructureChange?()
            return
        case .available:
            unavailableLabel.isHidden = true
            if addButton.superview == nil {
                sectionView.setAccessoryViews([addButton, removeButton])
            }
        }

        let hasItems = !newStack.items.isEmpty
        emptyLabel.isHidden = hasItems
        tableScrollView.isHidden = !hasItems
        if previousIDs == newStack.items.map(\.id) {
            // 구성이 같으면 보이는 행만 다시 그립니다.
            tableView.reloadData(forRowIndexes: IndexSet(integersIn: 0..<newStack.items.count),
                                 columnIndexes: IndexSet(integer: 0))
        } else {
            tableView.reloadData()
        }
        if let selectedID, !newStack.items.contains(where: { $0.id == selectedID }) {
            self.selectedID = nil
        }
        if self.selectedID == nil {
            self.selectedID = newStack.items.first?.id
        }
        if let selectedID, let index = newStack.items.firstIndex(where: { $0.id == selectedID }) {
            tableView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        } else {
            tableView.deselectAll(nil)
        }
        updateTableHeight()
        refreshDetail()
        refreshButtons()
        if previousIDs != newStack.items.map(\.id) {
            onStructureChange?()
        }
    }

    /// 진행 중인 파라미터 연속 조작을 끝냅니다.
    func endGestures() {
        detailRows.endGestures()
    }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] {
        var views: [NSView] = [sectionView.disclosureButton]
        guard sectionView.isExpanded, stack.availability == .available else { return views }
        views += [addButton, removeButton]
        if !stack.items.isEmpty {
            views.append(tableView)
            views += detailRows.keyViews
        }
        return views
    }

    private func rebuildAddMenu() {
        addButton.removeAllItems()
        let title = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        title.image = InspectorSymbols.image(InspectorSymbols.add, label: "Add Modifier")
        addButton.menu?.addItem(title)
        for kind in stack.addableKinds {
            let item = NSMenuItem(title: kind.title, action: #selector(addPressed(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = Int(kind.rawValue)
            item.image = InspectorSymbols.image(InspectorSymbols.modifier(kind), label: kind.title)
            addButton.menu?.addItem(item)
        }
    }

    private func updateTableHeight() {
        let rows = stack.items.count
        let contentHeight = rows > 0 ? tableView.rect(ofRow: rows - 1).maxY : 0
        let frame = NSScrollView.frameSize(
            forContentSize: NSSize(width: 0, height: contentHeight),
            horizontalScrollerClass: nil,
            verticalScrollerClass: nil,
            borderType: tableScrollView.borderType,
            controlSize: .regular,
            scrollerStyle: .overlay
        )
        tableHeight?.constant = frame.height
    }

    private func refreshDetail() {
        guard let item = selectedItem else {
            detailRows.isHidden = true
            detailRows.apply(rows: [], note: nil, forceDisabled: true)
            return
        }
        detailRows.isHidden = false
        detailRows.apply(rows: item.parameters, note: nil, forceDisabled: forceDisabled)
    }

    private func refreshButtons() {
        addButton.isEnabled = isInteractive && !stack.addableKinds.isEmpty
        removeButton.isEnabled = isInteractive && selectedItem != nil
    }

    private var selectedItem: InspectorModifierItem? {
        guard let selectedID else { return nil }
        return stack.items.first { $0.id == selectedID }
    }

    // MARK: - NSTableViewDataSource / Delegate

    func numberOfRows(in tableView: NSTableView) -> Int {
        stack.items.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("InspectorModifierCell")
        let cell = (tableView.makeView(withIdentifier: identifier, owner: self) as? InspectorModifierCellView)
            ?? InspectorModifierCellView(identifier: identifier)
        cell.checkbox.target = self
        cell.checkbox.action = #selector(checkboxToggled(_:))
        cell.configure(stack.items[row], isEditable: isInteractive)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = tableView.selectedRow
        let newID = stack.items.indices.contains(row) ? stack.items[row].id : nil
        guard newID != selectedID else { return }
        selectedID = newID
        refreshDetail()
        refreshButtons()
        onStructureChange?()
    }

    func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> NSPasteboardWriting? {
        guard isInteractive, stack.items.indices.contains(row) else { return nil }
        let item = NSPasteboardItem()
        item.setString(stack.items[row].id, forType: Self.dragType)
        return item
    }

    func tableView(
        _ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int,
        proposedDropOperation dropOperation: NSTableView.DropOperation
    ) -> NSDragOperation {
        guard isInteractive, (info.draggingSource as? NSTableView) === tableView else { return [] }
        if dropOperation == .on {
            tableView.setDropRow(row, dropOperation: .above)
        }
        return .move
    }

    func tableView(
        _ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int,
        dropOperation: NSTableView.DropOperation
    ) -> Bool {
        guard isInteractive, let id = info.draggingPasteboard.string(forType: Self.dragType),
              let from = stack.items.firstIndex(where: { $0.id == id }) else { return false }
        let target = from < row ? row - 1 : row
        guard target != from else { return false }
        send(.moveModifier(id: id, toIndex: target))
        return true
    }

    // MARK: - 동작

    @objc private func checkboxToggled(_ sender: NSButton) {
        let row = tableView.row(for: sender)
        guard isInteractive, stack.items.indices.contains(row) else { return }
        send(.setModifierEnabled(id: stack.items[row].id, isEnabled: sender.state == .on))
    }

    @objc private func addPressed(_ sender: NSMenuItem) {
        guard isInteractive, let raw = sender.representedObject as? Int,
              let kind = InspectorModifierKind(rawValue: UInt16(raw)) else { return }
        send(.addModifier(kind))
    }

    @objc private func removeSelected(_ sender: Any?) {
        guard isInteractive, let item = selectedItem else { return }
        send(.removeModifier(id: item.id))
    }

    @objc private func removeClicked(_ sender: NSMenuItem) {
        guard let item = targetItem else { return }
        send(.removeModifier(id: item.id))
    }

    @objc private func moveUpPressed(_ sender: NSMenuItem) {
        move(targetItem, by: -1)
    }

    @objc private func moveDownPressed(_ sender: NSMenuItem) {
        move(targetItem, by: 1)
    }

    @objc private func toggleEnabledPressed(_ sender: NSMenuItem) {
        guard let item = targetItem else { return }
        send(.setModifierEnabled(id: item.id, isEnabled: !item.isEnabled))
    }

    /// 문맥 메뉴 대상: 우클릭한 행, 없으면 선택한 행.
    private var targetItem: InspectorModifierItem? {
        let row = tableView.clickedRow >= 0 ? tableView.clickedRow : tableView.selectedRow
        return stack.items.indices.contains(row) ? stack.items[row] : nil
    }

    private func move(_ item: InspectorModifierItem?, by offset: Int) {
        guard isInteractive, let item, let index = stack.items.firstIndex(of: item) else { return }
        let target = index + offset
        guard stack.items.indices.contains(target) else { return }
        send(.moveModifier(id: item.id, toIndex: target))
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        guard isInteractive, let item = targetItem, let index = stack.items.firstIndex(of: item) else { return false }
        switch menuItem.action {
        case #selector(moveUpPressed(_:)): return index > 0
        case #selector(moveDownPressed(_:)): return index < stack.items.count - 1
        case #selector(toggleEnabledPressed(_:)):
            menuItem.title = item.isEnabled ? "Disable" : "Enable"
            return true
        default: return true
        }
    }

    /// 목록 키보드 조작. 처리했으면 true.
    private func handleKey(_ event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if event.specialKey == .delete || event.specialKey == .deleteForward {
            removeSelected(nil)
            return true
        }
        if event.charactersIgnoringModifiers == " ", flags.isEmpty {
            if let item = selectedItem, isInteractive {
                send(.setModifierEnabled(id: item.id, isEnabled: !item.isEnabled))
            }
            return true
        }
        if flags.contains([.command, .option]) {
            if event.specialKey == .upArrow {
                move(selectedItem, by: -1)
                return true
            }
            if event.specialKey == .downArrow {
                move(selectedItem, by: 1)
                return true
            }
        }
        return false
    }
}

/// 키 처리를 controller에 넘기는 modifier 목록.
final class InspectorModifierTableView: NSTableView {
    var keyHandler: ((NSEvent) -> Bool)?

    override func keyDown(with event: NSEvent) {
        if keyHandler?(event) == true { return }
        super.keyDown(with: event)
    }
}

/// modifier 목록 행: 활성 체크박스 + 종류 아이콘 + 이름 + 요약.
final class InspectorModifierCellView: NSTableCellView {
    let checkbox = NSButton()
    private let iconView = NSImageView()
    private let nameLabel = NSTextField(labelWithString: "")
    private let summaryLabel = NSTextField(labelWithString: "")

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        checkbox.setButtonType(.switch)
        checkbox.title = ""
        checkbox.controlSize = InspectorMetrics.controlSize
        InspectorFieldController.hug(checkbox)
        iconView.setAccessibilityElement(false)
        InspectorFieldController.hug(iconView)
        nameLabel.font = InspectorFonts.control
        nameLabel.setAccessibilityElement(false)
        InspectorFieldController.hug(nameLabel)
        summaryLabel.font = InspectorFonts.control
        summaryLabel.textColor = .secondaryLabelColor
        summaryLabel.lineBreakMode = .byTruncatingTail
        summaryLabel.alignment = .right
        summaryLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        summaryLabel.setContentHuggingPriority(.init(1), for: .horizontal)
        summaryLabel.setAccessibilityElement(false)
        imageView = iconView
        textField = nameLabel

        let stack = NSStackView(views: [checkbox, iconView, nameLabel, summaryLabel])
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = InspectorMetrics.groupSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: InspectorMetrics.groupSpacing),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -InspectorMetrics.groupSpacing),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorModifierCellView는 코드로만 생성합니다")
    }

    func configure(_ item: InspectorModifierItem, isEditable: Bool) {
        iconView.image = InspectorSymbols.image(InspectorSymbols.modifier(item.kind), label: item.kind.title)
        nameLabel.stringValue = item.kind.title
        nameLabel.textColor = item.isEnabled ? .labelColor : .disabledControlTextColor
        summaryLabel.stringValue = item.summary
        checkbox.state = item.isEnabled ? .on : .off
        checkbox.isEnabled = isEditable
        checkbox.setAccessibilityLabel("Enable \(item.kind.title)")
        checkbox.identifier = NSUserInterfaceItemIdentifier("modifier.\(item.id).enabled")
        // 행 하나를 한 요소로 읽히게 합니다(가이드 8.3). 체크박스는 조작 가능한 하위 요소로 남깁니다.
        let state = item.isEnabled ? "Enabled" : "Disabled"
        setAccessibilityLabel("\(item.kind.title), \(item.summary), \(state)")
    }
}
