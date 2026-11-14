import AppKit

/// 키 입력을 목록 controller에 먼저 넘기는 표준 table view(디자인 가이드 6.2, 8.2). conflict 목록처럼 계층이
/// 없는 평면 목록에 씁니다.
final class GitListTableView: NSTableView {
    /// true를 반환하면 이벤트를 소비합니다.
    var keyHandler: ((NSEvent) -> Bool)?

    override func keyDown(with event: NSEvent) {
        if keyHandler?(event) == true { return }
        super.keyDown(with: event)
    }

    /// 단일 열·고정 행 높이·type select 없음으로 설정합니다.
    /// 행 높이가 고정이어야 큰 목록에서도 위치 계산이 O(1)이고, type select는 전체 행을 훑으므로 끕니다.
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
    /// 그 자리에서 다시 구성해 비용이 화면 행 수에만 비례하게 합니다. 행 수가 바뀌면(해결 적용 후 목록 축소)
    /// 행 수만 알리고, 뒤쪽 행이 밀려 다른 항목을 가리키게 된 선택은 마지막 선택 행 하나로 줄입니다.
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

/// conflict 목록 한 행의 셀: 종류 아이콘 · 제목 · 종류 문구 / Base·Ours·Theirs 선택.
final class GitConflictCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("GitConflictCell")

    let kindImageView = NSImageView()
    let titleField = GitControls.label()
    let kindField = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let choiceControl: NSSegmentedControl
    let rawHintField = GitControls.label(GitPanelStrings.rawConflictHint, style: .callout, color: .secondaryLabelColor)

    init(target: AnyObject?, choiceAction: Selector) {
        choiceControl = NSSegmentedControl(
            labels: GitConflictChoice.allCases.map(GitPanelStrings.choice),
            trackingMode: .selectOne,
            target: target,
            action: choiceAction
        )
        super.init(frame: .zero)
        identifier = Self.identifier

        choiceControl.controlSize = .small
        choiceControl.font = NSFont.systemFont(ofSize: NSFont.systemFontSize(for: .small))
        kindImageView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .body, scale: .small)
        kindImageView.imageScaling = .scaleProportionallyDown
        titleField.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        kindField.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(rawValue: 249), for: .horizontal)
        textField = titleField
        imageView = kindImageView

        for view in [kindImageView, titleField, kindField, choiceControl, rawHintField] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        let spacing = GitPanelMetrics.inlineSpacing
        NSLayoutConstraint.activate([
            kindImageView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: spacing),
            kindImageView.centerYAnchor.constraint(equalTo: titleField.centerYAnchor),
            kindImageView.heightAnchor.constraint(equalTo: titleField.heightAnchor),
            kindImageView.widthAnchor.constraint(equalTo: kindImageView.heightAnchor),
            titleField.topAnchor.constraint(equalTo: topAnchor, constant: GitPanelMetrics.rowPadding),
            titleField.leadingAnchor.constraint(equalTo: kindImageView.trailingAnchor, constant: spacing),
            kindField.leadingAnchor.constraint(equalTo: titleField.trailingAnchor, constant: spacing * 2),
            kindField.firstBaselineAnchor.constraint(equalTo: titleField.firstBaselineAnchor),
            kindField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -spacing),
            choiceControl.topAnchor.constraint(equalTo: titleField.bottomAnchor, constant: spacing),
            choiceControl.leadingAnchor.constraint(equalTo: titleField.leadingAnchor),
            choiceControl.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -spacing),
            rawHintField.topAnchor.constraint(equalTo: titleField.bottomAnchor, constant: spacing),
            rawHintField.leadingAnchor.constraint(equalTo: titleField.leadingAnchor),
            rawHintField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -spacing),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitConflictCellView는 코드로만 생성합니다")
    }

    /// 두 줄 행 높이. 실제 컨트롤 크기에서 계산합니다(디자인 가이드 4.4).
    static func preferredRowHeight() -> CGFloat {
        let probe = GitConflictCellView(target: nil, choiceAction: #selector(GitConflictListController.choiceChanged(_:)))
        probe.titleField.stringValue = "Ag"
        let second = max(probe.choiceControl.intrinsicContentSize.height, probe.rawHintField.intrinsicContentSize.height)
        return ceil(
            GitPanelMetrics.rowPadding * 2 + probe.titleField.intrinsicContentSize.height
                + GitPanelMetrics.inlineSpacing + second
        )
    }

    /// 선택 강조가 없을 때의 아이콘 색.
    private var iconTint: NSColor = .secondaryLabelColor

    /// 선택 강조(키 창의 강조 색 배경)에서는 아이콘을 시스템 대비 색으로 그립니다(디자인 가이드 8.2).
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applyIconTint() }
    }

    private func applyIconTint() {
        kindImageView.contentTintColor = backgroundStyle == .emphasized ? nil : iconTint
    }

    func configure(_ row: GitConflictRow, stale: Bool, interactive: Bool, secondaryColor: NSColor) {
        let kindText = GitPanelStrings.conflictKind(row.kind)
        kindImageView.image = GitPanelSymbols.image(GitPanelSymbols.conflictKind(row.kind), label: kindText)
        kindImageView.setAccessibilityLabel(kindText)
        // 경고 아이콘은 모양과 문구로 상태를 함께 전달하므로 색에만 의존하지 않습니다(디자인 가이드 4.2).
        iconTint = row.kind == .raw ? secondaryColor : .systemOrange
        applyIconTint()
        titleField.stringValue = row.title
        titleField.textColor = stale ? secondaryColor : .labelColor
        kindField.stringValue = kindText
        kindField.textColor = secondaryColor
        rawHintField.textColor = secondaryColor
        if toolTip != row.path { toolTip = row.path }

        let isRaw = row.kind == .raw || row.allowedChoices.isEmpty
        choiceControl.isHidden = isRaw
        rawHintField.isHidden = !isRaw
        let summaries = [row.baseSummary, row.oursSummary, row.theirsSummary]
        for (index, choice) in GitConflictChoice.allCases.enumerated() {
            choiceControl.setEnabled(interactive && !stale && row.allowedChoices.contains(choice), forSegment: index)
            let summary = summaries[index].map { "\(GitPanelStrings.choice(choice)): \($0)" } ?? GitPanelStrings.choice(choice)
            if choiceControl.toolTip(forSegment: index) != summary { choiceControl.setToolTip(summary, forSegment: index) }
        }
        if let choice = row.choice, let index = GitConflictChoice.allCases.firstIndex(of: choice) {
            choiceControl.selectedSegment = index
        } else {
            choiceControl.selectedSegment = -1
        }
        choiceControl.setAccessibilityLabel(GitPanelStrings.resolution(row.title))
    }

    func configurePlaceholder(secondaryColor: NSColor) {
        kindImageView.image = nil
        titleField.stringValue = GitPanelStrings.loading
        titleField.textColor = secondaryColor
        kindField.stringValue = ""
        choiceControl.isHidden = true
        rawHintField.isHidden = true
        toolTip = nil
    }
}

/// 가상화된 conflict 목록(아키텍처 §27.2, XIV.13.4). 변경 목록과 같은 페이지 경로를 씁니다.
@MainActor
final class GitConflictListController: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
    let tableView = GitListTableView()
    let scrollView = NSScrollView()
    private(set) var cache = GitPagedRowCache<GitConflictRow>()

    var isInteractive = true {
        didSet { if oldValue != isInteractive, !suppressRefresh { tableView.reloadAvailableRows() } }
    }
    private var suppressRefresh = false
    var secondaryColor: NSColor = .secondaryLabelColor {
        didSet { tableView.reloadAvailableRows() }
    }

    var requestPage: ((UInt64, Range<Int>) -> Void)?
    var choose: ((IndexSet, GitConflictChoice) -> Void)?
    var reveal: ((Int) -> Void)?
    var selectionChanged: (() -> Void)?

    override init() {
        super.init()
        tableView.configureVirtualizedList(accessibilityLabel: GitPanelStrings.conflictsListLabel)
        tableView.rowHeight = max(tableView.rowHeight, GitConflictCellView.preferredRowHeight())
        tableView.dataSource = self
        tableView.delegate = self
        tableView.keyHandler = { [weak self] event in self?.handleKey(event) ?? false }
        tableView.cellRefresher = { [weak self] view, row in
            guard let self, let cell = view as? GitConflictCellView else { return }
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

    func receive(_ page: GitConflictPage) {
        guard let range = cache.store(generation: page.generation, startRow: page.startRow, rows: page.rows) else {
            return
        }
        tableView.reloadAvailableRows(in: range)
    }

    func row(at index: Int) -> GitConflictRow? { cache.peek(index) }

    /// fresh 또는 stale 행(표시 유지용).
    func displayedRow(at index: Int) -> GitConflictRow? { cache.peekDisplayed(index) }

    /// 코어가 거부한 페이지 요청을 다시 요청할 수 있게 합니다.
    func cancelRequest(generation: UInt64, rows: Range<Int>) {
        cache.cancelRequest(generation: generation, rows: rows)
    }

    /// 보이는 행을 캐시 값으로 다시 구성합니다(거부된 선택처럼 미리 보여 준 값을 되돌릴 때).
    func refreshVisibleRows() {
        tableView.reloadAvailableRows()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { cache.rowCount }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: GitConflictCellView.identifier, owner: self) as? GitConflictCellView)
            ?? GitConflictCellView(target: self, choiceAction: #selector(choiceChanged(_:)))
        configure(cell, row: row)
        return cell
    }

    private func configure(_ cell: GitConflictCellView, row: Int) {
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

    @objc func choiceChanged(_ sender: NSSegmentedControl) {
        let row = tableView.row(for: sender)
        let index = sender.selectedSegment
        guard row >= 0, isInteractive, index >= 0, index < GitConflictChoice.allCases.count,
              let value = cache.peek(row)
        else {
            tableView.reloadAvailableRows()
            return
        }
        let choice = GitConflictChoice.allCases[index]
        guard value.allowedChoices.contains(choice) else { return }
        choose?(IndexSet(integer: row), choice)
    }

    /// 선택 행들에 같은 해결을 적용합니다. 허용하지 않는 행은 코어가 건너뜁니다(XIV.12.1).
    func chooseForSelection(_ choice: GitConflictChoice) {
        let rows = tableView.selectedRowIndexes
        guard isInteractive, !rows.isEmpty else { return }
        choose?(rows, choice)
    }

    /// B·O·T: 선택 conflict에 Base·Ours·Theirs를 고릅니다(목록이 first responder일 때만).
    func handleKey(_ event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              let key = event.charactersIgnoringModifiers?.lowercased()
        else { return false }
        switch key {
        case "b": chooseForSelection(.base)
        case "o": chooseForSelection(.ours)
        case "t": chooseForSelection(.theirs)
        default: return false
        }
        return true
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        menu.autoenablesItems = false
        let rows = tableView.contextTargetRows
        guard !rows.isEmpty else { return }
        let items: [(String, String, GitConflictChoice)] = [
            (GitPanelStrings.useChoice(.base), "b", .base),
            (GitPanelStrings.useChoice(.ours), "o", .ours),
            (GitPanelStrings.useChoice(.theirs), "t", .theirs),
        ]
        for (title, key, choice) in items {
            let item = NSMenuItem(title: title, action: #selector(chooseFromMenu(_:)), keyEquivalent: key)
            item.keyEquivalentModifierMask = []
            item.representedObject = GitConflictChoice.allCases.firstIndex(of: choice)
            item.target = self
            item.isEnabled = isInteractive
            menu.addItem(item)
        }
        menu.addItem(.separator())
        let reveal = NSMenuItem(title: GitPanelStrings.revealConflict, action: #selector(revealFromMenu(_:)), keyEquivalent: "")
        reveal.target = self
        reveal.isEnabled = rows.count == 1
        menu.addItem(reveal)
    }

    @objc private func chooseFromMenu(_ sender: NSMenuItem) {
        guard let index = sender.representedObject as? Int else { return }
        guard isInteractive else { return }
        choose?(tableView.contextTargetRows, GitConflictChoice.allCases[index])
    }

    @objc private func revealFromMenu(_ sender: NSMenuItem) {
        guard let row = tableView.contextTargetRows.first else { return }
        reveal?(row)
    }
}
