import AppKit

/// Modifier stack 섹션(아키텍처 §12.6, VI.9).
///
/// 표준 `NSTableView` 목록으로 순서·활성 상태를 보여 주고, 선택한 항목의 파라미터를 목록 아래 행으로
/// 편집합니다. 재정렬은 드래그 또는 Command-Option-위/아래, 삭제는 Delete, 활성 전환은 Space입니다.
/// 순서·id·검증은 코어가 맡고 셸은 명령만 보냅니다.
///
/// 성능: 섹션 틀·추가 메뉴·목록·행 셀 몇 개는 페이지를 나눠 만들 때 함께 만듭니다(`buildSteps()`).
/// 파라미터 행은 modifier 구성(종류)별로 한 번 만들어 두고, 같은 구성의 다른 항목을 고르면 속성 id만 바꿔
/// 다시 씁니다. 처음 보는 구성은 화면 갱신을 막지 않도록 `scheduleBuild`로 나눠 만든 뒤 보입니다.
@MainActor
final class InspectorModifierStackController: NSObject, NSTableViewDataSource, NSTableViewDelegate,
    NSMenuItemValidation
{
    /// 드래그 재정렬용 앱 내부 pasteboard 형식.
    static let dragType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.inspector.modifier")
    /// 미리 만들어 둘 목록 행 셀 수.
    static let prebuiltCellCount = 3
    private static let cellIdentifier = NSUserInterfaceItemIdentifier("InspectorModifierCell")

    let sectionView: InspectorSectionView
    let tableView = InspectorModifierTableView()
    let addButton = NSPopUpButton(frame: .zero, pullsDown: true)
    let removeButton = NSButton()
    private let tableScrollView = InspectorEmbeddedScrollView()
    private let content = InspectorModifierContentView()
    private let emptyLabel = InspectorWrappingLabel(text: "No modifiers. Use the Add button to add one.")
    private let unavailableLabel = InspectorWrappingLabel(text: "")
    /// 파라미터 행 구성 키 → 행 묶음. 고른 항목의 것만 보입니다.
    private var detailViews: [String: InspectorRowsView] = [:]
    /// 나눠 만드는 중인 파라미터 행 구성 키.
    private var pendingDetailKeys: Set<String> = []
    private var cellPool: [InspectorModifierCellView] = []
    private var builtAddableKinds: [InspectorModifierKind]?
    private(set) var stack = InspectorModifierStack(availability: .available, items: [], addableKinds: [],
                                                    isEditable: false)
    private(set) var selectedID: String?
    private var forceDisabled = false
    private var hasApplied = false
    private var builtStepCount = 0
    private var warmUpRows = 0
    private var addButtonSize = NSSize.zero
    private var removeButtonSize = NSSize.zero
    private let send: (InspectorCommand) -> Void
    /// 선택·구성이 바뀌어 키보드 순서를 다시 계산해야 할 때 호출됩니다.
    var onStructureChange: (() -> Void)?
    /// 처음 보는 파라미터 행 구성을 나눠 만들 때 씁니다(키, 단계, 완료). 없으면 바로 만듭니다.
    var scheduleBuild: ((String, [InspectorBuildStep], @escaping () -> Void) -> Void)?

    init(expanded: Bool, send: @escaping (InspectorCommand) -> Void) {
        self.send = send
        sectionView = InspectorSectionView(id: .modifiers, title: "Modifiers", expanded: expanded)
        super.init()
    }

    private var isInteractive: Bool { stack.isEditable && !forceDisabled }

    // MARK: - 구성(나눠 만들기)

    /// 섹션 내용을 만드는 단계 수. 각 단계는 컨트롤 하나 정도입니다. 마지막 두 단계는 목록 예열입니다.
    static var buildStepCount: Int { 7 + prebuiltCellCount }

    /// 단계별 비용 등급(추가 popup 측정, 삭제 버튼, 목록, 행 셀, 목록 예열은 혼자 slice 하나를 씁니다).
    static func buildStepCost(_ index: Int) -> InspectorBuildStep.Cost {
        index == 0 || index == 3 ? .light : .heavy
    }

    /// `index`번째 구성 단계(0..<buildStepCount)를 실행합니다.
    func runBuildStep(_ index: Int) {
        guard index == builtStepCount, index < Self.buildStepCount else { return }
        switch index {
        case 0: buildAddButton()
        case 1: addButtonSize = InspectorLayout.measure(addButton)
        case 2: buildRemoveButton()
        case 3: sectionView.setAccessoryViews([(addButton, addButtonSize), (removeButton, removeButtonSize)])
        case 4: buildTable()
        case Self.buildStepCount - 2: warmUpTable(rows: Self.prebuiltCellCount)
        case Self.buildStepCount - 1: warmUpTable(rows: 0)
        default: cellPool.append(InspectorModifierCellView(identifier: Self.cellIdentifier))
        }
        builtStepCount += 1
    }

    /// 목록 예열: 자리표시 항목으로 행 뷰·셀을 한 번 만들고(배치·그리기는 대기 페이지에서) 비웁니다. 처음 실제
    /// stack을 보일 때 NSTableView가 재사용 대기열의 행 뷰·셀을 다시 써서 목록 첫 표시 비용이 줄어듭니다.
    private func warmUpTable(rows: Int) {
        warmUpRows = rows
        content.show(rows > 0 ? .list : .empty)
        tableView.reloadData()
        if rows > 0 {
            content.tableHeight = NSScrollView.frameSize(
                forContentSize: NSSize(width: 0, height: tableView.rect(ofRow: rows - 1).maxY),
                horizontalScrollerClass: nil, verticalScrollerClass: nil, borderType: tableScrollView.borderType,
                controlSize: .regular, scrollerStyle: .overlay
            ).height
        }
    }

    /// 목록이 보이는 항목: 예열 중이면 자리표시, 아니면 snapshot 항목.
    private var displayedItems: [InspectorModifierItem] {
        guard warmUpRows > 0 else { return stack.items }
        return (0..<warmUpRows).map {
            InspectorModifierItem(id: "warm-up-\($0)", kind: .fill, isEnabled: true, summary: "", parameters: [])
        }
    }

    /// 아직 실행하지 않은 구성 단계를 지금 모두 실행합니다.
    func buildNow() {
        while builtStepCount < Self.buildStepCount { runBuildStep(builtStepCount) }
    }

    private func buildAddButton() {
        addButton.controlSize = InspectorMetrics.controlSize
        addButton.autoenablesItems = false
        addButton.setAccessibilityLabel("Add Modifier")
        addButton.toolTip = "Add Modifier"
        addButton.identifier = NSUserInterfaceItemIdentifier("modifiers.add")
        rebuildAddMenu([])
    }

    private func buildRemoveButton() {
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
        removeButtonSize = InspectorLayout.measure(removeButton)
    }

    private func buildTable() {
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

        content.configure(unavailable: unavailableLabel, empty: emptyLabel, table: tableScrollView)
        sectionView.setContent(content)
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
        buildNow()
        if hasApplied, newStack == stack, forceDisabled == self.forceDisabled {
            // 바뀐 것이 없으면 목록을 다시 그리지 않습니다. 파라미터 필드의 낙관적 표시만 snapshot 값으로 맞춥니다.
            refreshDetail()
            return
        }
        hasApplied = true
        let previousIDs = stack.items.map(\.id)
        stack = newStack
        self.forceDisabled = forceDisabled

        if builtAddableKinds != newStack.addableKinds { rebuildAddMenu(newStack.addableKinds) }
        switch newStack.availability {
        case .unavailable(let reason):
            if unavailableLabel.stringValue != reason { unavailableLabel.stringValue = reason }
            content.show(.unavailable)
            sectionView.setAccessoriesHidden(true)
            if selectedID != nil || !previousIDs.isEmpty {
                selectedID = nil
                tableView.reloadData()
            }
            refreshDetail()
            onStructureChange?()
            return
        case .available:
            sectionView.setAccessoriesHidden(false)
        }

        let hasItems = !newStack.items.isEmpty
        content.show(hasItems ? .list : .empty)
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
            if tableView.selectedRow != index {
                tableView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
            }
        } else if tableView.selectedRow >= 0 {
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
        detailViews.values.forEach { $0.endGestures() }
    }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] {
        var views: [NSView] = [sectionView.disclosureButton]
        guard sectionView.isExpanded, stack.availability == .available else { return views }
        views += [addButton, removeButton]
        if !stack.items.isEmpty {
            views.append(tableView)
            views += visibleDetail?.keyViews ?? []
        }
        return views
    }

    /// 지금 보이는 파라미터 행 묶음.
    var visibleDetail: InspectorRowsView? { content.detail }

    private func rebuildAddMenu(_ kinds: [InspectorModifierKind]) {
        builtAddableKinds = kinds
        addButton.removeAllItems()
        let title = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        title.image = InspectorSymbols.image(InspectorSymbols.add, label: "Add Modifier")
        addButton.menu?.addItem(title)
        for kind in kinds {
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
        content.tableHeight = frame.height
    }

    /// 고른 항목의 파라미터 행을 보입니다. 같은 구성의 행 묶음이 있으면 속성 id·값만 바꿔 다시 쓰고,
    /// 처음 보는 구성이면 나눠 만들기를 맡기고 그동안 비워 둡니다.
    private func refreshDetail() {
        guard let item = selectedItem, stack.availability == .available else {
            content.detail = nil
            return
        }
        let key = InspectorRowsView.compositionKey(rows: item.parameters, note: nil)
        if let detail = detailViews[key] {
            detail.apply(rows: item.parameters, note: nil, forceDisabled: forceDisabled)
            if content.detail !== detail {
                content.detail = detail
                onStructureChange?()
            }
            return
        }
        content.detail = nil
        guard !pendingDetailKeys.contains(key) else { return }
        let detail = InspectorRowsView(send: send)
        let steps = detail.buildSteps(rows: item.parameters, note: nil, forceDisabled: forceDisabled)
        let finish = { [weak self] in
            guard let self else { return }
            self.pendingDetailKeys.remove(key)
            self.detailViews[key] = detail
            self.refreshDetail()
        }
        // 나눠 만드는 동안에도 창 안에 두어(숨김) 컨트롤 배치·측정이 단계 안에서 끝나게 합니다.
        content.addDetail(detail)
        if let scheduleBuild {
            pendingDetailKeys.insert(key)
            scheduleBuild("detail:\(key)", steps, finish)
        } else {
            steps.forEach { $0.run() }
            finish()
        }
    }

    private func refreshButtons() {
        let canAdd = isInteractive && !stack.addableKinds.isEmpty
        if addButton.isEnabled != canAdd { addButton.isEnabled = canAdd }
        let canRemove = isInteractive && selectedItem != nil
        if removeButton.isEnabled != canRemove { removeButton.isEnabled = canRemove }
    }

    private var selectedItem: InspectorModifierItem? {
        guard let selectedID else { return nil }
        return stack.items.first { $0.id == selectedID }
    }

    // MARK: - NSTableViewDataSource / Delegate

    func numberOfRows(in tableView: NSTableView) -> Int {
        displayedItems.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: Self.cellIdentifier, owner: self) as? InspectorModifierCellView)
            ?? cellPool.popLast()
            ?? InspectorModifierCellView(identifier: Self.cellIdentifier)
        cell.checkbox.target = self
        cell.checkbox.action = #selector(checkboxToggled(_:))
        cell.configure(displayedItems[row], isEditable: isInteractive && warmUpRows == 0)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard warmUpRows == 0 else { return }
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

/// modifier 목록 행: 활성 체크박스 + 종류 아이콘 + 이름 + 요약. frame으로 배치합니다.
final class InspectorModifierCellView: NSTableCellView {
    let checkbox = NSButton()
    private let iconView = NSImageView()
    private let nameLabel = NSTextField(labelWithString: "")
    private let summaryLabel = NSTextField(labelWithString: "")
    private var checkboxSize = NSSize.zero
    private var iconSize = NSSize.zero
    private var nameSize = NSSize.zero
    private var summaryHeight: CGFloat = 0
    private var iconKind: InspectorModifierKind?

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        checkbox.setButtonType(.switch)
        checkbox.title = ""
        checkbox.controlSize = InspectorMetrics.controlSize
        iconView.setAccessibilityElement(false)
        nameLabel.font = InspectorFonts.control
        nameLabel.setAccessibilityElement(false)
        summaryLabel.font = InspectorFonts.control
        summaryLabel.textColor = .secondaryLabelColor
        summaryLabel.lineBreakMode = .byTruncatingTail
        summaryLabel.alignment = .right
        summaryLabel.setAccessibilityElement(false)
        imageView = iconView
        textField = nameLabel
        for view in [checkbox, iconView, nameLabel, summaryLabel] as [NSView] { addSubview(view) }
        checkboxSize = InspectorLayout.measure(checkbox)
        summaryHeight = InspectorLayout.measure(summaryLabel).height
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorModifierCellView는 코드로만 생성합니다")
    }

    func configure(_ item: InspectorModifierItem, isEditable: Bool) {
        if iconKind != item.kind {
            iconKind = item.kind
            iconView.image = InspectorSymbols.image(InspectorSymbols.modifier(item.kind), label: item.kind.title)
            iconSize = InspectorLayout.measure(iconView)
            nameLabel.stringValue = item.kind.title
            nameSize = InspectorLayout.measure(nameLabel)
            needsLayout = true
        }
        let nameColor: NSColor = item.isEnabled ? .labelColor : .disabledControlTextColor
        if nameLabel.textColor != nameColor { nameLabel.textColor = nameColor }
        if summaryLabel.stringValue != item.summary { summaryLabel.stringValue = item.summary }
        let state: NSControl.StateValue = item.isEnabled ? .on : .off
        if checkbox.state != state { checkbox.state = state }
        if checkbox.isEnabled != isEditable { checkbox.isEnabled = isEditable }
        checkbox.setAccessibilityLabel("Enable \(item.kind.title)")
        checkbox.identifier = NSUserInterfaceItemIdentifier("modifier.\(item.id).enabled")
        // 행 하나를 한 요소로 읽히게 합니다(가이드 8.3). 체크박스는 조작 가능한 하위 요소로 남깁니다.
        let stateText = item.isEnabled ? "Enabled" : "Disabled"
        setAccessibilityLabel("\(item.kind.title), \(item.summary), \(stateText)")
    }

    override func layout() {
        super.layout()
        let height = bounds.height
        let spacing = InspectorMetrics.groupSpacing
        var x = spacing
        InspectorLayout.place(checkbox, x: x, width: checkboxSize.width, size: checkboxSize, lineY: 0,
                              lineHeight: height)
        x += checkboxSize.width + spacing
        InspectorLayout.place(iconView, x: x, width: iconSize.width, size: iconSize, lineY: 0, lineHeight: height)
        x += iconSize.width + spacing
        InspectorLayout.place(nameLabel, x: x, width: nameSize.width, size: nameSize, lineY: 0, lineHeight: height)
        x += nameSize.width + spacing
        let summaryWidth = max(0, bounds.width - spacing - x)
        InspectorLayout.place(summaryLabel, x: x, width: summaryWidth,
                              size: NSSize(width: summaryWidth, height: summaryHeight), lineY: 0, lineHeight: height)
    }
}

/// modifier 섹션 본문: 사용 불가 안내 / 빈 목록 안내 / 목록 + 고른 항목의 파라미터 행.
final class InspectorModifierContentView: InspectorFlowView {
    enum Mode {
        case unavailable
        case empty
        case list
    }

    private var unavailableLabel: InspectorWrappingLabel?
    private var emptyLabel: InspectorWrappingLabel?
    private var tableScrollView: NSScrollView?
    private var mode: Mode = .empty
    /// 목록 높이(행 수에서 계산).
    var tableHeight: CGFloat = 0 {
        didSet { if tableHeight != oldValue { invalidateFlow() } }
    }
    /// 보이는 파라미터 행 묶음. 다른 묶음은 숨깁니다.
    var detail: InspectorRowsView? {
        didSet {
            guard detail !== oldValue else { return }
            oldValue?.isHidden = true
            detail?.isHidden = mode != .list
            invalidateFlow()
        }
    }

    func configure(unavailable: InspectorWrappingLabel, empty: InspectorWrappingLabel, table: NSScrollView) {
        unavailableLabel = unavailable
        emptyLabel = empty
        tableScrollView = table
        for view in [unavailable, empty, table] as [NSView] { addSubview(view) }
        show(.empty)
    }

    /// 파라미터 행 묶음을 숨긴 채로 붙입니다(나눠 만드는 동안에도 창 안에 있게).
    func addDetail(_ view: InspectorRowsView) {
        guard view.superview !== self else { return }
        view.isHidden = true
        addSubview(view)
    }

    func show(_ newMode: Mode) {
        mode = newMode
        setHidden(unavailableLabel, newMode != .unavailable)
        setHidden(emptyLabel, newMode != .empty)
        setHidden(tableScrollView, newMode != .list)
        setHidden(detail, newMode != .list)
        invalidateFlow()
    }

    private func setHidden(_ view: NSView?, _ hidden: Bool) {
        if let view, view.isHidden != hidden { view.isHidden = hidden }
    }

    private func visibleEntries(forWidth width: CGFloat) -> [(view: NSView, height: CGFloat)] {
        var entries: [(NSView, CGFloat)] = []
        switch mode {
        case .unavailable:
            if let unavailableLabel { entries.append((unavailableLabel, unavailableLabel.flowHeight(forWidth: width))) }
        case .empty:
            if let emptyLabel { entries.append((emptyLabel, emptyLabel.flowHeight(forWidth: width))) }
        case .list:
            if let tableScrollView { entries.append((tableScrollView, tableHeight)) }
            if let detail, !detail.rowViews.isEmpty { entries.append((detail, detail.flowHeight(forWidth: width))) }
        }
        return entries
    }

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        let entries = visibleEntries(forWidth: width)
        guard !entries.isEmpty else { return 0 }
        return ceil(entries.reduce(0) { $0 + $1.height } + CGFloat(entries.count - 1) * InspectorMetrics.rowSpacing)
    }

    override func layout() {
        super.layout()
        var y: CGFloat = 0
        for entry in visibleEntries(forWidth: bounds.width) {
            let frame = NSRect(x: 0, y: y, width: bounds.width, height: entry.height)
            if entry.view.frame != frame { entry.view.frame = frame }
            y += entry.height + InspectorMetrics.rowSpacing
        }
    }
}
