import AppKit

/// 키 입력을 목록 controller에 먼저 넘기는 표준 table view(디자인 가이드 6.2, 8.2).
final class GitListTableView: NSTableView {
    /// true를 반환하면 이벤트를 소비합니다.
    var keyHandler: ((NSEvent) -> Bool)?

    override func keyDown(with event: NSEvent) {
        if keyHandler?(event) == true { return }
        super.keyDown(with: event)
    }

    /// 단일 열·고정 행 높이·type select 없음으로 설정합니다.
    /// 행 높이가 고정이어야 100만 행에서도 위치 계산이 O(1)이고, type select는 전체 행을 훑으므로 끕니다.
    func configureVirtualizedList(accessibilityLabel: String) {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        addTableColumn(column)
        headerView = nil
        style = .inset
        usesAutomaticRowHeights = false
        allowsTypeSelect = false
        allowsMultipleSelection = true
        allowsEmptySelection = true
        floatsGroupRows = false
        columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        setAccessibilityLabel(accessibilityLabel)
    }

    /// 이미 만든 셀을 그 자리에서 다시 구성합니다. 셀을 떼었다 붙이지 않아 제약 엔진 비용을 줄입니다.
    var cellRefresher: ((NSView, Int) -> Void)?

    /// 행 뷰가 이미 만들어진 행 중 범위에 드는 행만 다시 구성합니다.
    func reloadAvailableRows(in range: Range<Int>? = nil) {
        var fallback = IndexSet()
        enumerateAvailableRowViews { rowView, row in
            guard range?.contains(row) ?? true else { return }
            if let cell = rowView.view(atColumn: 0) as? NSView, let refresher = cellRefresher {
                refresher(cell, row)
            } else {
                fallback.insert(row)
            }
        }
        if !fallback.isEmpty {
            reloadData(forRowIndexes: fallback, columnIndexes: IndexSet(integer: 0))
        }
    }

    /// 세대가 바뀐 뒤 목록을 갱신합니다.
    ///
    /// `reloadData()`는 모든 행 뷰를 다시 만들고 선택도 잃으므로 쓰지 않습니다. 이미 만들어진 행 뷰만
    /// 그 자리에서 다시 구성해 비용이 화면 행 수에만 비례하게 합니다(100만 행 선택이 있어도 같음).
    /// 행 수가 바뀌면(묶음 접기·펼치기, 커밋 후 목록 축소) 행 수만 알리고, 뒤쪽 행이 밀려 다른 항목을
    /// 가리키게 된 선택은 마지막 선택 행 하나(접고 펼친 묶음 행 자신)로 줄입니다.
    func reloadForGeneration(previousRowCount: Int, rowCount: Int) {
        // 처음 채울 때(만든 행 뷰가 없음)는 잃을 선택이 없고, `reloadData()`가 행 뷰 생성을 layout 단계로
        // 미루므로 이 경로가 가장 쌉니다.
        if previousRowCount == 0 || rowCount == 0 {
            reloadData()
            return
        }
        if rowCount != previousRowCount {
            let anchor = selectedRow
            noteNumberOfRowsChanged()
            // 행 수만 알리면 frame이 늦게 갱신되어, 끝 근처를 보던 중 행이 줄면 보이는 영역에 행이 없게
            // 됩니다. 바로 다시 배치해 clip view가 새 높이 안으로 들어오게 합니다.
            tile()
            if anchor >= 0, anchor < rowCount {
                if selectedRowIndexes != IndexSet(integer: anchor) {
                    selectRowIndexes(IndexSet(integer: anchor), byExtendingSelection: false)
                }
            } else if !selectedRowIndexes.isEmpty {
                deselectAll(nil)
            }
        }
        reloadAvailableRows()
    }

    /// 문맥 메뉴 대상: 우클릭한 행이 선택에 있으면 선택 전체, 아니면 그 행.
    var contextTargetRows: IndexSet {
        let clicked = clickedRow
        if clicked >= 0, !selectedRowIndexes.contains(clicked) { return IndexSet(integer: clicked) }
        return selectedRowIndexes
    }
}

/// 변경 목록 한 행의 셀: 펼침 표시 · Stage 체크박스 · 종류 아이콘 · 제목 · 보조 문구.
final class GitChangeCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("GitChangeCell")
    private static let bodyFont = NSFont.preferredFont(forTextStyle: .body)
    private static let headlineFont = NSFont.preferredFont(forTextStyle: .headline)

    let disclosureButton = NSButton(title: "", target: nil, action: nil)
    let stageCheckbox = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    let kindImageView = NSImageView()
    let titleField = GitControls.label()
    let detailField = GitControls.label(style: .callout, color: .secondaryLabelColor)
    private var indentConstraint: NSLayoutConstraint!

    init(target: AnyObject?, checkboxAction: Selector, disclosureAction: Selector) {
        super.init(frame: .zero)
        identifier = Self.identifier

        disclosureButton.bezelStyle = .disclosure
        disclosureButton.setButtonType(.onOff)
        disclosureButton.title = ""
        disclosureButton.target = target
        disclosureButton.action = disclosureAction

        stageCheckbox.allowsMixedState = true
        stageCheckbox.target = target
        stageCheckbox.action = checkboxAction

        kindImageView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .body, scale: .small)
        kindImageView.contentTintColor = .secondaryLabelColor
        kindImageView.imageScaling = .scaleProportionallyDown

        titleField.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        titleField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        detailField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        detailField.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(rawValue: 249), for: .horizontal)

        textField = titleField
        imageView = kindImageView

        for view in [disclosureButton, stageCheckbox, kindImageView, titleField, detailField] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        indentConstraint = disclosureButton.leadingAnchor.constraint(equalTo: leadingAnchor)
        let spacing = GitPanelMetrics.inlineSpacing
        NSLayoutConstraint.activate([
            indentConstraint,
            disclosureButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            stageCheckbox.leadingAnchor.constraint(equalTo: disclosureButton.trailingAnchor, constant: spacing),
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
        let probe = GitChangeCellView(
            target: nil,
            checkboxAction: #selector(GitChangeListController.checkboxClicked(_:)),
            disclosureAction: #selector(GitChangeListController.disclosureClicked(_:))
        )
        probe.titleField.stringValue = "Ag"
        let content = max(
            probe.stageCheckbox.intrinsicContentSize.height,
            probe.disclosureButton.intrinsicContentSize.height,
            probe.titleField.intrinsicContentSize.height
        )
        return ceil(content + GitPanelMetrics.rowPadding * 2)
    }

    func configure(_ row: GitChangeRow, stale: Bool, interactive: Bool, secondaryColor: NSColor) {
        let level: CGFloat
        switch row.level {
        case .document: level = 0
        case .object: level = 1
        case .change: level = 2
        }
        indentConstraint.constant = level * GitPanelMetrics.indentPerLevel

        let isGroup = row.level != .change
        disclosureButton.isHidden = !isGroup
        disclosureButton.state = row.isExpanded ? .on : .off
        disclosureButton.isEnabled = !stale
        disclosureButton.setAccessibilityLabel(
            row.isExpanded ? GitPanelStrings.collapse(row.title) : GitPanelStrings.expand(row.title)
        )

        stageCheckbox.isHidden = false
        switch row.stageState {
        case .unstaged: stageCheckbox.state = .off
        case .staged: stageCheckbox.state = .on
        case .partiallyStaged: stageCheckbox.state = .mixed
        }
        stageCheckbox.isEnabled = interactive && !stale
        stageCheckbox.setAccessibilityLabel(GitPanelStrings.stageCheckbox(row.title))
        let stateText = GitPanelStrings.stageState(row.stageState)
        if stageCheckbox.toolTip != stateText { stageCheckbox.toolTip = stateText }

        let symbol: String
        let symbolLabel: String
        switch row.level {
        case .document:
            symbol = GitPanelSymbols.document
            symbolLabel = "Document"
        case .object:
            symbol = GitPanelSymbols.object
            symbolLabel = "Object"
        case .change:
            let kind = row.kind ?? .setProperty
            symbol = GitPanelSymbols.kind(kind)
            symbolLabel = GitPanelStrings.kind(kind)
        }
        kindImageView.image = GitPanelSymbols.image(symbol, label: symbolLabel)
        kindImageView.setAccessibilityLabel(symbolLabel)

        let font = row.level == .document ? Self.headlineFont : Self.bodyFont
        if titleField.font != font { titleField.font = font }
        titleField.stringValue = row.title
        titleField.textColor = stale ? secondaryColor : .labelColor
        detailField.stringValue = row.detail
        detailField.textColor = secondaryColor
        let help = row.detail.isEmpty ? row.title : "\(row.title) — \(row.detail)"
        if toolTip != help { toolTip = help }
    }

    /// 페이지가 아직 도착하지 않은 행.
    func configurePlaceholder(secondaryColor: NSColor) {
        indentConstraint.constant = 0
        disclosureButton.isHidden = true
        stageCheckbox.isHidden = true
        kindImageView.image = nil
        if titleField.font != Self.bodyFont { titleField.font = Self.bodyFont }
        titleField.stringValue = GitPanelStrings.loading
        titleField.textColor = secondaryColor
        detailField.stringValue = ""
        if toolTip != nil { toolTip = nil }
    }
}

/// 가상화된 변경 목록(XIV.13.1). 화면에 보이는 행의 페이지만 요청하고 고정 높이 행만 그립니다.
@MainActor
final class GitChangeListController: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
    let tableView = GitListTableView()
    let scrollView = NSScrollView()
    private(set) var cache = GitPagedRowCache<GitChangeRow>()

    /// 조작 가능 여부(작업 진행 중·제한 모드에서는 false).
    var isInteractive = true {
        didSet { if oldValue != isInteractive, !suppressRefresh { tableView.reloadAvailableRows() } }
    }
    private var suppressRefresh = false
    var secondaryColor: NSColor = .secondaryLabelColor {
        didSet { tableView.reloadAvailableRows() }
    }

    var requestPage: ((UInt64, Range<Int>) -> Void)?
    var setStaged: ((IndexSet, Bool) -> Void)?
    var setExpanded: ((Int, Bool) -> Void)?
    var selectionChanged: (() -> Void)?

    override init() {
        super.init()
        tableView.configureVirtualizedList(accessibilityLabel: GitPanelStrings.changesListLabel)
        tableView.rowHeight = max(tableView.rowHeight, GitChangeCellView.preferredRowHeight())
        tableView.dataSource = self
        tableView.delegate = self
        tableView.keyHandler = { [weak self] event in self?.handleKey(event) ?? false }
        tableView.cellRefresher = { [weak self] view, row in
            guard let self, let cell = view as? GitChangeCellView else { return }
            self.configure(cell, row: row)
        }
        let menu = NSMenu()
        menu.delegate = self
        tableView.menu = menu

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
    }

    var generation: UInt64 { cache.generation }
    var rowCount: Int { cache.rowCount }

    func reset(_ info: GitListInfo) {
        update(info, interactive: isInteractive)
    }

    /// 세대와 조작 가능 여부를 함께 반영해 보이는 행을 한 번만 다시 구성합니다.
    func update(_ info: GitListInfo?, interactive: Bool) {
        let interactivityChanged = interactive != isInteractive
        suppressRefresh = true
        isInteractive = interactive
        suppressRefresh = false
        if let info, info.generation != cache.generation || info.rowCount != cache.rowCount {
            let previousRowCount = cache.rowCount
            cache.reset(generation: info.generation, rowCount: info.rowCount)
            tableView.reloadForGeneration(previousRowCount: previousRowCount, rowCount: info.rowCount)
        } else if interactivityChanged {
            tableView.reloadAvailableRows()
        }
    }

    func receive(_ page: GitChangePage) {
        guard let range = cache.store(generation: page.generation, startRow: page.startRow, rows: page.rows) else {
            return
        }
        tableView.reloadAvailableRows(in: range)
    }

    func row(at index: Int) -> GitChangeRow? { cache.peek(index) }

    /// fresh 또는 stale 행(표시 유지용).
    func displayedRow(at index: Int) -> GitChangeRow? { cache.peekDisplayed(index) }

    // MARK: NSTableViewDataSource

    func numberOfRows(in tableView: NSTableView) -> Int { cache.rowCount }

    // MARK: NSTableViewDelegate

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: GitChangeCellView.identifier, owner: self) as? GitChangeCellView)
            ?? GitChangeCellView(
                target: self,
                checkboxAction: #selector(checkboxClicked(_:)),
                disclosureAction: #selector(disclosureClicked(_:))
            )
        configure(cell, row: row)
        return cell
    }

    private func configure(_ cell: GitChangeCellView, row: Int) {
        switch cache.lookup(row) {
        case .fresh(let value):
            cell.configure(value, stale: false, interactive: isInteractive, secondaryColor: secondaryColor)
        case .stale(let value):
            cell.configure(value, stale: true, interactive: false, secondaryColor: secondaryColor)
            requestIfNeeded(row)
        case .missing:
            cell.configurePlaceholder(secondaryColor: secondaryColor)
            requestIfNeeded(row)
        }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        selectionChanged?()
    }

    private func requestIfNeeded(_ row: Int) {
        if let range = cache.rangeToRequest(forRow: row) {
            requestPage?(cache.generation, range)
        }
    }

    // MARK: 동작

    @objc func checkboxClicked(_ sender: NSButton) {
        let row = tableView.row(for: sender)
        guard row >= 0, isInteractive, let value = cache.peek(row) else {
            tableView.reloadAvailableRows()
            return
        }
        let staged = GitPanelRules.stagedTarget(afterToggling: value.stageState)
        // 결과가 올 때까지 목표 상태를 미리 보여 줍니다. 실제 값은 새 세대 페이지가 덮어씁니다.
        sender.state = staged ? .on : .off
        setStaged?(IndexSet(integer: row), staged)
    }

    @objc func disclosureClicked(_ sender: NSButton) {
        let row = tableView.row(for: sender)
        guard row >= 0, let value = cache.peek(row), value.level != .change else { return }
        setExpanded?(row, !value.isExpanded)
    }

    /// 선택 행의 Stage를 토글합니다. 기준은 마지막으로 선택한 행의 상태입니다.
    func toggleSelection() {
        let rows = tableView.selectedRowIndexes
        guard isInteractive, !rows.isEmpty else { return }
        let anchor = tableView.selectedRow
        let staged = cache.peek(anchor).map { GitPanelRules.stagedTarget(afterToggling: $0.stageState) } ?? true
        setStaged?(rows, staged)
    }

    /// Space: Stage 토글, ←/→: 묶음 접기/펼치기(디자인 가이드 6.2 아웃라인 동작과 같은 키).
    func handleKey(_ event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              let characters = event.charactersIgnoringModifiers, let scalar = characters.unicodeScalars.first
        else { return false }
        switch Int(scalar.value) {
        case 0x20:
            toggleSelection()
            return true
        case NSLeftArrowFunctionKey, NSRightArrowFunctionKey:
            let expand = Int(scalar.value) == NSRightArrowFunctionKey
            let row = tableView.selectedRow
            guard tableView.selectedRowIndexes.count == 1, let value = cache.peek(row), value.level != .change
            else { return false }
            if value.isExpanded != expand { setExpanded?(row, expand) }
            return true
        default:
            return false
        }
    }

    // MARK: NSMenuDelegate

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let rows = tableView.contextTargetRows
        guard !rows.isEmpty else { return }
        let stage = NSMenuItem(title: GitPanelStrings.stage, action: #selector(stageFromMenu(_:)), keyEquivalent: "")
        let unstage = NSMenuItem(title: GitPanelStrings.unstage, action: #selector(unstageFromMenu(_:)), keyEquivalent: "")
        for item in [stage, unstage] {
            item.target = self
            item.isEnabled = isInteractive
            menu.addItem(item)
        }
        menu.autoenablesItems = false
    }

    @objc private func stageFromMenu(_ sender: NSMenuItem) {
        setStaged?(tableView.contextTargetRows, true)
    }

    @objc private func unstageFromMenu(_ sender: NSMenuItem) {
        setStaged?(tableView.contextTargetRows, false)
    }
}
