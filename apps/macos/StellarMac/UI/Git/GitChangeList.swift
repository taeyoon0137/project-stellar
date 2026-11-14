import AppKit

/// 변경 계층(문서 → 객체 → 변경)을 보여 주는 표준 `NSOutlineView`(HIG Outline views, 디자인 가이드 6.2).
///
/// 펼침 표시, ←/→ 접기·펼치기·부모로 이동, Option-클릭 하위 전체 펼침, 다중 선택, 계층 수준·펼침 상태의
/// 접근성 노출은 NSOutlineView 기본 동작을 그대로 씁니다. 이 하위 클래스는 Space(Stage 토글)만 더합니다.
final class GitChangeOutlineView: NSOutlineView {
    /// true를 반환하면 이벤트를 소비합니다.
    var keyHandler: ((NSEvent) -> Bool)?

    override func keyDown(with event: NSEvent) {
        if keyHandler?(event) == true { return }
        super.keyDown(with: event)
    }

    /// 문맥 메뉴 대상: 우클릭한 행이 선택에 있으면 선택 전체, 아니면 그 행.
    var contextTargetRows: IndexSet {
        let clicked = clickedRow
        if clicked >= 0, !selectedRowIndexes.contains(clicked) { return IndexSet(integer: clicked) }
        return selectedRowIndexes
    }
}

/// outline 항목. 부모 항목과 부모 안 순번으로 위치를 나타내며, 같은 구조 번호 안에서만 유효합니다.
///
/// 행 값은 페이지 캐시에 있고 항목은 위치만 가집니다. NSOutlineView는 항목을 포인터로 구분하고 붙잡지
/// 않으므로 controller가 부모 → 자식 사전으로 붙잡아 둡니다(자식 → 부모는 weak).
final class GitChangeOutlineItem: NSObject {
    private(set) weak var parent: GitChangeOutlineItem?
    let index: Int
    let level: GitChangeRowLevel
    /// 행이 처음 도착하면 정해집니다. 구조 번호가 같으면 바뀌지 않습니다.
    var nodeID: GitChangeNodeID?
    /// outline에 마지막으로 알려 준 자식 수.
    var reportedChildCount = 0
    /// NSOutlineView가 요청해 만든 자식 항목.
    var children: [Int: GitChangeOutlineItem] = [:]

    init(parent: GitChangeOutlineItem?, index: Int, level: GitChangeRowLevel) {
        self.parent = parent
        self.index = index
        self.level = level
    }
}

/// 변경 목록 한 행의 셀: Stage 체크박스 · 종류 아이콘 · 제목 · 보조 문구.
///
/// 들여쓰기와 펼침 표시는 outline이 그립니다. VoiceOver에는 셀 하나가 제목·보조 문구·종류·Stage 상태를 함께
/// 읽는 요소로 보이고(디자인 가이드 8.3), Stage·Unstage는 셀의 custom action입니다. 체크박스는 마우스용이며
/// Tab 순서에 넣지 않습니다. 키보드는 Space와 문맥 메뉴로 같은 동작에 닿습니다(가이드 8.2).
final class GitChangeCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("GitChangeCell")
    private static let bodyFont = NSFont.preferredFont(forTextStyle: .body)
    private static let headlineFont = NSFont.preferredFont(forTextStyle: .headline)

    let stageCheckbox = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    let kindImageView = NSImageView()
    let titleField = GitControls.label()
    let detailField = GitControls.label(style: .callout, color: .secondaryLabelColor)
    /// 선택 강조가 없을 때의 아이콘 색.
    private var iconTint: NSColor = .secondaryLabelColor

    init(target: AnyObject?, checkboxAction: Selector) {
        super.init(frame: .zero)
        identifier = Self.identifier

        stageCheckbox.allowsMixedState = true
        stageCheckbox.target = target
        stageCheckbox.action = checkboxAction
        stageCheckbox.refusesFirstResponder = true
        // 셀 하나가 행 전체를 읽으므로 하위 뷰는 접근성 트리에서 뺍니다.
        stageCheckbox.setAccessibilityElement(false)
        stageCheckbox.cell?.setAccessibilityElement(false)

        kindImageView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .body, scale: .small)
        kindImageView.imageScaling = .scaleProportionallyDown
        for control in [kindImageView, titleField, detailField] as [NSControl] {
            control.setAccessibilityElement(false)
            control.cell?.setAccessibilityElement(false)
        }

        titleField.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        titleField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        detailField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        detailField.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(rawValue: 249), for: .horizontal)

        textField = titleField
        imageView = kindImageView

        for view in [stageCheckbox, kindImageView, titleField, detailField] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        let spacing = GitPanelMetrics.inlineSpacing
        NSLayoutConstraint.activate([
            stageCheckbox.leadingAnchor.constraint(equalTo: leadingAnchor),
            stageCheckbox.centerYAnchor.constraint(equalTo: centerYAnchor),
            kindImageView.leadingAnchor.constraint(equalTo: stageCheckbox.trailingAnchor, constant: spacing),
            kindImageView.centerYAnchor.constraint(equalTo: centerYAnchor),
            kindImageView.heightAnchor.constraint(equalTo: titleField.heightAnchor),
            kindImageView.widthAnchor.constraint(equalTo: kindImageView.heightAnchor),
            titleField.leadingAnchor.constraint(equalTo: kindImageView.trailingAnchor, constant: spacing),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),
            detailField.leadingAnchor.constraint(equalTo: titleField.trailingAnchor, constant: spacing * 2),
            detailField.firstBaselineAnchor.constraint(equalTo: titleField.firstBaselineAnchor),
            detailField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -spacing),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitChangeCellView는 코드로만 생성합니다")
    }

    /// 셀 내용에서 계산한 행 높이. 값을 하드코딩하지 않습니다(디자인 가이드 4.4).
    static func preferredRowHeight() -> CGFloat {
        let probe = GitChangeCellView(target: nil, checkboxAction: #selector(GitChangeListController.checkboxClicked(_:)))
        probe.titleField.stringValue = "Ag"
        let content = max(probe.stageCheckbox.intrinsicContentSize.height, probe.titleField.intrinsicContentSize.height)
        return ceil(content + GitPanelMetrics.rowPadding * 2)
    }

    /// 선택 강조(키 창의 강조 색 배경)에서는 아이콘을 시스템 대비 색으로 그립니다(디자인 가이드 8.2).
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applyIconTint() }
    }

    private func applyIconTint() {
        kindImageView.contentTintColor = backgroundStyle == .emphasized ? nil : iconTint
    }

    func configure(_ row: GitChangeRow, stale: Bool, interactive: Bool, secondaryColor: NSColor) {
        stageCheckbox.isHidden = false
        switch row.stageState {
        case .unstaged: stageCheckbox.state = .off
        case .staged: stageCheckbox.state = .on
        case .partiallyStaged: stageCheckbox.state = .mixed
        }
        stageCheckbox.isEnabled = interactive && !stale
        let stateText = GitPanelStrings.stageState(row.stageState)
        if stageCheckbox.toolTip != stateText { stageCheckbox.toolTip = stateText }

        let symbol: String
        let symbolLabel: String
        switch row.level {
        case .document:
            symbol = GitPanelSymbols.document
            symbolLabel = GitPanelStrings.documentLevel
        case .object:
            symbol = GitPanelSymbols.object
            symbolLabel = GitPanelStrings.objectLevel
        case .change:
            let kind = row.kind ?? .setProperty
            symbol = GitPanelSymbols.kind(kind)
            symbolLabel = GitPanelStrings.kind(kind)
        }
        kindImageView.image = GitPanelSymbols.image(symbol, label: symbolLabel)
        iconTint = secondaryColor
        applyIconTint()

        let font = row.level == .document ? Self.headlineFont : Self.bodyFont
        if titleField.font != font { titleField.font = font }
        titleField.stringValue = row.title
        titleField.textColor = stale ? secondaryColor : .labelColor
        detailField.stringValue = row.detail
        detailField.textColor = secondaryColor
        let help = row.detail.isEmpty ? row.title : "\(row.title) — \(row.detail)"
        if toolTip != help { toolTip = help }
        setAccessibilityLabel(GitPanelStrings.changeRowLabel(
            title: row.title, detail: row.detail, kind: symbolLabel, state: stateText))
    }

    /// 페이지가 아직 도착하지 않은 행.
    func configurePlaceholder(secondaryColor: NSColor) {
        stageCheckbox.isHidden = true
        kindImageView.image = nil
        if titleField.font != Self.bodyFont { titleField.font = Self.bodyFont }
        titleField.stringValue = GitPanelStrings.loading
        titleField.textColor = secondaryColor
        detailField.stringValue = ""
        if toolTip != nil { toolTip = nil }
        setAccessibilityLabel(GitPanelStrings.loading)
        setAccessibilityCustomActions(nil)
    }
}

/// 가상화된 변경 outline(XIV.13.1).
///
/// - 부모마다 자식 순번을 페이지로 받고, 화면에 보이는 행의 페이지만 요청합니다.
/// - 펼침은 outline의 보기 상태입니다. 노드 ID로 기억해 구조가 바뀐 뒤 행이 도착하면 다시 펼칩니다.
/// - NSOutlineView는 펼친 항목의 바로 아래 자식 항목을 한 번에 만듭니다. 항목은 위치만 가진 작은 객체이고
///   행 값은 보이는 행의 페이지만 받으므로, 비용은 펼친 묶음의 자식 수와 화면 행 수에 비례합니다.
@MainActor
final class GitChangeListController: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate {
    let outlineView = GitChangeOutlineView()
    let scrollView = NSScrollView()
    private(set) var cache = GitChangeTreeCache()
    private var hasLoaded = false
    private var rootItems: [Int: GitChangeOutlineItem] = [:]
    private var itemsByID: [GitChangeNodeID: GitChangeOutlineItem] = [:]
    /// 사용자가 펼친 묶음. 구조가 바뀐 뒤 복원합니다.
    private(set) var expandedIDs: Set<GitChangeNodeID> = []
    /// 구조가 바뀌기 전 선택. 행이 도착하면 다시 선택합니다.
    private var pendingSelection: Set<GitChangeNodeID> = []
    private var restoreCandidates: [GitChangeOutlineItem] = []
    private var restoreScheduled = false
    private var isReloading = false
    /// 다음 런 루프에서 반영할 구조 변경.
    private var pendingStructure: GitChangeTreeInfo?

    /// 조작 가능 여부(작업 진행 중·제한 모드에서는 false).
    var isInteractive = true {
        didSet { if oldValue != isInteractive, !suppressRefresh { refreshVisibleRows() } }
    }
    private var suppressRefresh = false
    var secondaryColor: NSColor = .secondaryLabelColor {
        didSet { refreshVisibleRows() }
    }

    var requestPage: ((UInt64, GitChangeNodeID?, Range<Int>) -> Void)?
    var setStaged: (([GitChangeRange], Bool) -> Void)?
    var selectionChanged: (() -> Void)?

    override init() {
        super.init()
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        outlineView.addTableColumn(column)
        outlineView.outlineTableColumn = column
        outlineView.headerView = nil
        outlineView.style = .inset
        // 행 높이가 고정이어야 큰 목록에서도 위치 계산이 싸고, type select는 행을 훑으므로 끕니다.
        outlineView.usesAutomaticRowHeights = false
        outlineView.rowHeight = max(outlineView.rowHeight, GitChangeCellView.preferredRowHeight())
        outlineView.allowsTypeSelect = false
        outlineView.allowsMultipleSelection = true
        outlineView.allowsEmptySelection = true
        outlineView.autoresizesOutlineColumn = false
        outlineView.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        outlineView.setAccessibilityLabel(GitPanelStrings.changesListLabel)
        outlineView.dataSource = self
        outlineView.delegate = self
        outlineView.keyHandler = { [weak self] event in self?.handleKey(event) ?? false }
        let menu = NSMenu()
        menu.delegate = self
        outlineView.menu = menu

        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
    }

    var generation: UInt64 { cache.generation }
    /// 최상위(문서) 행 수.
    var documentCount: Int { cache.rootCount }

    // MARK: 세대 반영

    /// 세대와 조작 가능 여부를 함께 반영합니다. 구조가 같으면 보이는 행만 다시 구성하고, 다르면 다시 읽습니다.
    ///
    /// 처음 채울 때(약 0.1ms)를 빼면 구조 다시 읽기는 다음 런 루프 차례로 미룹니다. `reloadData()`는 만든 행 뷰를
    /// 모두 떼므로 AppKit 안에서 수 ms가 걸려(2026-11-14 Debug 약 2.8ms) 상태 UI 반영과 한 차례에 겹치지 않게
    /// 합니다. 그 사이 목록은 이전 세대 값을 보여 주고, 이전 세대로 보낸 명령은 코어가 거부합니다.
    func update(_ info: GitChangeTreeInfo?, interactive: Bool) {
        let interactivityChanged = interactive != isInteractive
        suppressRefresh = true
        isInteractive = interactive
        suppressRefresh = false
        guard let info else { return }
        if pendingStructure != nil {
            pendingStructure = info
        } else if !hasLoaded {
            applyStructure(info)
        } else if info.structure != cache.structure || info.documentCount != cache.rootCount {
            pendingStructure = info
            DispatchQueue.main.async { [weak self] in
                MainActor.assumeIsolated { self?.flushPendingStructure() }
            }
        } else if info.generation != cache.generation {
            cache.advance(generation: info.generation)
            refreshVisibleRows()
        } else if interactivityChanged {
            refreshVisibleRows()
        }
    }

    private func flushPendingStructure() {
        guard let info = pendingStructure else { return }
        pendingStructure = nil
        let start = ProcessInfo.processInfo.systemUptime
        if info.structure != cache.structure || info.documentCount != cache.rootCount {
            applyStructure(info)
        } else if info.generation != cache.generation {
            cache.advance(generation: info.generation)
            refreshVisibleRows()
        }
        maxStructureReloadSeconds = max(maxStructureReloadSeconds, ProcessInfo.processInfo.systemUptime - start)
    }

    /// 진단: 미룬 구조 다시 읽기 차례 한 번의 최대 점유.
    private(set) var maxStructureReloadSeconds: TimeInterval = 0

    private func applyStructure(_ info: GitChangeTreeInfo) {
        hasLoaded = true
        pendingSelection = Set(selectedItems.compactMap(\.nodeID))
        cache.reset(generation: info.generation, structure: info.structure, rootCount: info.documentCount)
        reloadStructure()
    }

    private func reloadStructure() {
        rootItems = [:]
        itemsByID = [:]
        restoreCandidates = []
        // 처음 채울 때와 구조가 바뀐 뒤에만 다시 읽습니다. NSOutlineView는 보이는 최상위 항목만 요청합니다.
        isReloading = true
        outlineView.reloadData()
        isReloading = false
    }

    func receive(_ page: GitChangePage) {
        guard let range = cache.store(page) else { return }
        let siblings: [Int: GitChangeOutlineItem]
        if let parent = page.parent {
            guard let parentItem = itemsByID[parent] else { return }
            siblings = parentItem.children
        } else {
            siblings = rootItems
        }
        for (offset, row) in page.rows.enumerated() {
            guard let item = siblings[range.lowerBound + offset] else { continue }
            attach(row.id, to: item)
            noteRestoreCandidate(item, row: row)
        }
        refreshVisibleRows { item in
            guard range.contains(item.index) else { return false }
            if let parent = page.parent { return item.parent?.nodeID == parent }
            return item.parent == nil
        }
    }

    /// 코어가 거부한 페이지 요청을 다시 요청할 수 있게 합니다.
    func cancelRequest(generation: UInt64, parent: GitChangeNodeID?, children: Range<Int>) {
        cache.cancelRequest(generation: generation, parent: parent, children: children)
    }

    // MARK: 조회

    /// 항목이 속한 페이지의 부모 키. 부모 행이 아직 없으면 nil입니다.
    private func pageParent(of item: GitChangeOutlineItem) -> GitChangeNodeID?? {
        guard let parent = item.parent else { return .some(nil) }
        return parent.nodeID.map { .some($0) }
    }

    /// fresh 행.
    func row(of item: GitChangeOutlineItem) -> GitChangeRow? {
        guard let parent = pageParent(of: item) else { return nil }
        return cache.peek(parent: parent, index: item.index)
    }

    /// fresh 또는 stale 행(표시 유지용).
    func displayedRow(of item: GitChangeOutlineItem) -> GitChangeRow? {
        guard let parent = pageParent(of: item) else { return nil }
        return cache.peekDisplayed(parent: parent, index: item.index)
    }

    func item(for id: GitChangeNodeID) -> GitChangeOutlineItem? { itemsByID[id] }

    func item(atRow row: Int) -> GitChangeOutlineItem? { outlineView.item(atRow: row) as? GitChangeOutlineItem }

    var selectedItems: [GitChangeOutlineItem] {
        outlineView.selectedRowIndexes.compactMap { item(atRow: $0) }
    }

    /// 선택이 정확히 하나면 그 항목.
    var singleSelectedItem: GitChangeOutlineItem? {
        let rows = outlineView.selectedRowIndexes
        guard rows.count == 1, let row = rows.first else { return nil }
        return item(atRow: row)
    }

    /// 페이지가 단일 선택 항목을 담는지.
    func page(_ page: GitChangePage, contains item: GitChangeOutlineItem) -> Bool {
        guard let parent = pageParent(of: item), parent == page.parent else { return false }
        return item.index >= page.startIndex && item.index < page.startIndex + page.rows.count
    }

    private func attach(_ id: GitChangeNodeID, to item: GitChangeOutlineItem) {
        guard item.nodeID == nil else { return }
        item.nodeID = id
        itemsByID[id] = item
    }

    private func childItem(of parent: GitChangeOutlineItem?, at index: Int) -> GitChangeOutlineItem {
        let item: GitChangeOutlineItem
        if let parent {
            if let existing = parent.children[index] { return existing }
            item = GitChangeOutlineItem(parent: parent, index: index, level: parent.level.childLevel ?? .change)
            parent.children[index] = item
        } else {
            if let existing = rootItems[index] { return existing }
            item = GitChangeOutlineItem(parent: nil, index: index, level: .document)
            rootItems[index] = item
        }
        if let row = displayedRow(of: item) { attach(row.id, to: item) }
        return item
    }

    // MARK: NSOutlineViewDataSource

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? GitChangeOutlineItem else { return cache.rootCount }
        let count = displayedRow(of: item)?.childCount ?? 0
        item.reportedChildCount = count
        return count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        childItem(of: item as? GitChangeOutlineItem, at: index)
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        (item as? GitChangeOutlineItem)?.level != .change
    }

    // MARK: NSOutlineViewDelegate

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? GitChangeOutlineItem else { return nil }
        let cell = (outlineView.makeView(withIdentifier: GitChangeCellView.identifier, owner: self) as? GitChangeCellView)
            ?? GitChangeCellView(target: self, checkboxAction: #selector(checkboxClicked(_:)))
        configure(cell, item: item)
        return cell
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        selectionChanged?()
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard let item = notification.userInfo?["NSObject"] as? GitChangeOutlineItem, let id = item.nodeID else { return }
        expandedIDs.insert(id)
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard !isReloading, let item = notification.userInfo?["NSObject"] as? GitChangeOutlineItem,
              let id = item.nodeID
        else { return }
        expandedIDs.remove(id)
    }

    private func configure(_ cell: GitChangeCellView, item: GitChangeOutlineItem) {
        guard let parent = pageParent(of: item) else {
            cell.configurePlaceholder(secondaryColor: secondaryColor)
            return
        }
        switch cache.lookup(parent: parent, index: item.index) {
        case .fresh(let value):
            attach(value.id, to: item)
            cell.configure(value, stale: false, interactive: isInteractive, secondaryColor: secondaryColor)
            cell.setAccessibilityCustomActions(isInteractive ? accessibilityActions(for: item, row: value) : nil)
            noteRestoreCandidate(item, row: value)
        case .stale(let value):
            cell.configure(value, stale: true, interactive: false, secondaryColor: secondaryColor)
            cell.setAccessibilityCustomActions(nil)
            requestIfNeeded(item, parent: parent)
        case .missing:
            cell.configurePlaceholder(secondaryColor: secondaryColor)
            requestIfNeeded(item, parent: parent)
        }
    }

    private func requestIfNeeded(_ item: GitChangeOutlineItem, parent: GitChangeNodeID?) {
        let childCount = item.parent.map { displayedRow(of: $0)?.childCount ?? $0.reportedChildCount } ?? cache.rootCount
        if let range = cache.rangeToRequest(parent: parent, index: item.index, childCount: childCount) {
            requestPage?(cache.generation, parent, range)
        }
    }

    /// 화면에 만들어진 행만 다시 구성합니다(비용은 화면 행 수에 비례).
    func refreshVisibleRows(where filter: ((GitChangeOutlineItem) -> Bool)? = nil) {
        outlineView.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? GitChangeCellView, let item = self.item(atRow: row) else { return }
            if let filter, !filter(item) { return }
            self.configure(cell, item: item)
        }
    }

    // MARK: 펼침·선택 복원

    /// 구조가 바뀐 뒤 도착한 행의 펼침·선택을 되살리고, 펼친 묶음의 자식 수가 바뀌었으면 다시 읽습니다.
    /// outline 구조를 바꿨으면(펼침·다시 읽기) true입니다.
    private func restore(_ item: GitChangeOutlineItem, row: GitChangeRow) -> Bool {
        var changedStructure = false
        if outlineView.isItemExpanded(item) {
            if item.reportedChildCount != row.childCount {
                outlineView.reloadItem(item, reloadChildren: true)
                changedStructure = true
            }
        } else if row.level != .change, expandedIDs.contains(row.id), outlineView.row(forItem: item) >= 0 {
            outlineView.expandItem(item)
            changedStructure = true
        }
        if pendingSelection.contains(row.id) {
            let index = outlineView.row(forItem: item)
            if index >= 0 {
                pendingSelection.remove(row.id)
                outlineView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: true)
            }
        }
        return changedStructure
    }

    private func needsRestore(_ item: GitChangeOutlineItem, row: GitChangeRow) -> Bool {
        if outlineView.isItemExpanded(item) { return item.reportedChildCount != row.childCount }
        return (row.level != .change && expandedIDs.contains(row.id)) || pendingSelection.contains(row.id)
    }

    /// 복원은 이벤트 반영·셀 구성(outline layout) 도중에 하지 않고 다음 런 루프에서 합니다. NSOutlineView의
    /// 펼침 한 번은 AppKit 내부에서 수 ms가 걸리므로 한 런 루프에 하나씩만 하고 나머지는 다음 차례로 넘깁니다.
    private func noteRestoreCandidate(_ item: GitChangeOutlineItem, row: GitChangeRow) {
        guard needsRestore(item, row: row), !restoreCandidates.contains(where: { $0 === item }) else { return }
        restoreCandidates.append(item)
        scheduleRestore()
    }

    private func scheduleRestore() {
        guard !restoreScheduled, !restoreCandidates.isEmpty else { return }
        restoreScheduled = true
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.flushRestore() }
        }
    }

    private func flushRestore() {
        restoreScheduled = false
        while !restoreCandidates.isEmpty {
            let item = restoreCandidates.removeFirst()
            guard let row = row(of: item), restore(item, row: row) else { continue }
            break
        }
        scheduleRestore()
    }

    /// 아직 반영할 구조 변경이나 복원할 펼침·선택이 남았는지(검사용).
    var isRestoring: Bool { pendingStructure != nil || restoreScheduled || !restoreCandidates.isEmpty }

    // MARK: 동작

    /// 행 집합을 Stage 대상 범위로 바꿉니다. 조상이 함께 선택된 행은 빼고(묶음은 하위 전체), 부모별로 묶습니다.
    /// 전체 선택은 최상위 범위 하나입니다.
    func targets(forRows rows: IndexSet) -> [GitChangeRange] {
        let total = outlineView.numberOfRows
        if total > 0, rows.count >= total {
            return [GitChangeRange(parent: nil, children: IndexSet(integersIn: 0..<cache.rootCount))]
        }
        let items = rows.compactMap { item(atRow: $0) }
        let selected = Set(items.map(ObjectIdentifier.init))
        var grouped: [GitChangeNodeID?: IndexSet] = [:]
        var order: [GitChangeNodeID?] = []
        for item in items {
            var ancestor = item.parent
            var covered = false
            while let current = ancestor {
                if selected.contains(ObjectIdentifier(current)) {
                    covered = true
                    break
                }
                ancestor = current.parent
            }
            guard !covered, let parent = pageParent(of: item) else { continue }
            if grouped[parent] == nil { order.append(parent) }
            grouped[parent, default: IndexSet()].insert(item.index)
        }
        return order.compactMap { parent in grouped[parent].map { GitChangeRange(parent: parent, children: $0) } }
    }

    private func targets(for item: GitChangeOutlineItem) -> [GitChangeRange] {
        guard let parent = pageParent(of: item) else { return [] }
        return [GitChangeRange(parent: parent, children: IndexSet(integer: item.index))]
    }

    @objc func checkboxClicked(_ sender: NSButton) {
        let row = outlineView.row(for: sender)
        guard row >= 0, isInteractive, let item = item(atRow: row), let value = self.row(of: item) else {
            refreshVisibleRows()
            return
        }
        let staged = GitPanelRules.stagedTarget(afterToggling: value.stageState)
        // 결과가 올 때까지 목표 상태를 미리 보여 줍니다. 실제 값은 새 세대 페이지가, 거부되면 캐시 값이 덮어씁니다.
        sender.state = staged ? .on : .off
        setStaged?(targets(for: item), staged)
    }

    /// 선택 행의 Stage를 토글합니다. 기준은 마지막으로 선택한 행의 상태입니다.
    func toggleSelection() {
        let rows = outlineView.selectedRowIndexes
        guard isInteractive, !rows.isEmpty else { return }
        let anchor = item(atRow: outlineView.selectedRow).flatMap { self.row(of: $0) }
        let staged = anchor.map { GitPanelRules.stagedTarget(afterToggling: $0.stageState) } ?? true
        setStaged?(targets(forRows: rows), staged)
    }

    private func accessibilityActions(for item: GitChangeOutlineItem, row: GitChangeRow) -> [NSAccessibilityCustomAction] {
        let staged = GitPanelRules.stagedTarget(afterToggling: row.stageState)
        let name = staged ? GitPanelStrings.stage : GitPanelStrings.unstage
        return [
            NSAccessibilityCustomAction(name: name) { [weak self, weak item] in
                guard let self, let item, self.isInteractive else { return false }
                self.setStaged?(self.targets(for: item), staged)
                return true
            },
        ]
    }

    /// Space: Stage 토글. ←/→ 등 나머지는 NSOutlineView 기본 동작입니다.
    func handleKey(_ event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              event.charactersIgnoringModifiers == " "
        else { return false }
        toggleSelection()
        return true
    }

    // MARK: NSMenuDelegate

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.autoenablesItems = false
        let rows = outlineView.contextTargetRows
        guard !rows.isEmpty else { return }
        let stage = NSMenuItem(title: GitPanelStrings.stage, action: #selector(stageFromMenu(_:)), keyEquivalent: "")
        let unstage = NSMenuItem(title: GitPanelStrings.unstage, action: #selector(unstageFromMenu(_:)), keyEquivalent: "")
        for item in [stage, unstage] {
            item.target = self
            item.isEnabled = isInteractive
            menu.addItem(item)
        }
    }

    @objc private func stageFromMenu(_ sender: NSMenuItem) {
        guard isInteractive else { return }
        setStaged?(targets(forRows: outlineView.contextTargetRows), true)
    }

    @objc private func unstageFromMenu(_ sender: NSMenuItem) {
        guard isInteractive else { return }
        setStaged?(targets(forRows: outlineView.contextTargetRows), false)
    }
}
