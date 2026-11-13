import AppKit

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

    func configure(_ row: GitConflictRow, stale: Bool, interactive: Bool, secondaryColor: NSColor) {
        let kindText = GitPanelStrings.conflictKind(row.kind)
        kindImageView.image = GitPanelSymbols.image(GitPanelSymbols.conflictKind(row.kind), label: kindText)
        kindImageView.setAccessibilityLabel(kindText)
        // 경고 아이콘은 모양과 문구로 상태를 함께 전달하므로 색에만 의존하지 않습니다(디자인 가이드 4.2).
        kindImageView.contentTintColor = row.kind == .raw ? secondaryColor : .systemOrange
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
            ("Use \(GitPanelStrings.base)", "b", .base),
            ("Use \(GitPanelStrings.ours)", "o", .ours),
            ("Use \(GitPanelStrings.theirs)", "t", .theirs),
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
