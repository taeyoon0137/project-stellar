import AppKit

/// 최근 프로젝트 목록 표. Return·Enter로 열고 Delete로 목록에서 뺍니다(HIG Lists and tables, Keyboard).
final class WelcomeRecentProjectsTableView: NSTableView {
    var onOpen: (() -> Void)?
    var onRemove: (() -> Void)?

    override func keyDown(with event: NSEvent) {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard modifiers.subtracting([.numericPad, .function]).isEmpty, selectedRow >= 0 else {
            super.keyDown(with: event)
            return
        }
        switch event.keyCode {
        case 36, 76:  // Return, keypad Enter
            onOpen?()
        case 51, 117:  // Delete, forward Delete
            onRemove?()
        default:
            super.keyDown(with: event)
        }
    }
}

/// 최근 프로젝트 행 하나. 여러 하위 뷰를 접근성 요소 하나로 묶습니다(디자인 가이드 8.3).
final class WelcomeRecentProjectCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("WelcomeRecentProjectCell")

    let iconView = WelcomeViews.symbolView("folder", style: .title2)
    let nameLabel = WelcomeViews.label("", style: .body)
    let pathLabel = WelcomeViews.label("", style: .subheadline, color: .secondaryLabelColor)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = WelcomeRecentProjectCellView.identifier
        imageView = iconView
        textField = nameLabel
        pathLabel.lineBreakMode = .byTruncatingMiddle

        let texts = NSStackView(views: [nameLabel, pathLabel])
        texts.orientation = .vertical
        texts.alignment = .leading
        texts.spacing = 0
        texts.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let row = NSStackView(views: [iconView, texts])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = WelcomeMetrics.rowSpacing * 2
        row.edgeInsets = NSEdgeInsets(
            top: WelcomeMetrics.rowSpacing,
            left: 0,
            bottom: WelcomeMetrics.rowSpacing,
            right: 0
        )
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.cell)
        // 행이 요소 하나로 이름·경로를 읽으므로, 하위 레이블은 트리에서 숨겨 VoiceOver가 같은 내용을 두 번 읽지 않게 합니다.
        nameLabel.setAccessibilityElement(false)
        pathLabel.setAccessibilityElement(false)
    }

    /// 하위 뷰를 접근성 자식으로 내보내지 않습니다. 내용은 이 요소의 레이블 하나로 읽힙니다(가이드 8.3).
    override func accessibilityChildren() -> [Any]? {
        []
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeRecentProjectCellView는 코드로만 생성합니다")
    }

    func configure(with project: WelcomeRecentProject, customActions: [NSAccessibilityCustomAction]) {
        let displayPath = (project.path as NSString).abbreviatingWithTildeInPath
        nameLabel.stringValue = project.displayName
        nameLabel.textColor = project.isMissing ? .secondaryLabelColor : .labelColor
        pathLabel.stringValue = project.isMissing ? WelcomeStrings.missingProjectPath(displayPath) : displayPath
        let symbol = project.isMissing ? "questionmark.folder" : "folder"
        iconView.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(textStyle: .title2, scale: .medium))
        toolTip = project.path

        // 상태를 색에만 기대지 않도록 "Not Found"를 레이블에도 넣습니다(디자인 가이드 4.2).
        var parts = [project.displayName, displayPath]
        if project.isMissing { parts.append(WelcomeStrings.notFound) }
        setAccessibilityLabel(parts.joined(separator: ", "))
        setAccessibilityCustomActions(customActions)
    }
}

/// 최근 프로젝트 영역: 목록과 빈·읽는 중·오류 상태.
final class WelcomeRecentProjectsView: NSView, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
    var onOpen: ((WelcomeRecentProject) -> Void)?
    var onRemove: ((WelcomeRecentProject) -> Void)?
    var onReveal: ((WelcomeRecentProject) -> Void)?
    var onRetry: (() -> Void)?

    let scrollView = NSScrollView()
    let tableView = WelcomeRecentProjectsTableView()
    let stateStack = NSStackView()
    let stateSpinner = NSProgressIndicator()
    let stateTitleLabel = WelcomeViews.label("", style: .title3, color: .secondaryLabelColor)
    let stateMessageLabel = WelcomeViews.wrappingLabel("", style: .callout, color: .secondaryLabelColor)
    let retryButton = WelcomeViews.button(WelcomeStrings.tryAgain, target: nil, action: nil)

    private(set) var projects: [WelcomeRecentProject] = []
    private(set) var state: WelcomeRecentProjectsState = .loading

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        buildTable()
        buildStateStack()
        apply(.loading)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeRecentProjectsView는 코드로만 생성합니다")
    }

    private func buildTable() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("project"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .inset
        tableView.usesAutomaticRowHeights = true
        tableView.allowsMultipleSelection = false
        tableView.allowsEmptySelection = true
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(doubleClicked(_:))
        tableView.setAccessibilityLabel(WelcomeStrings.recentProjects)
        tableView.onOpen = { [weak self] in self?.openSelected() }
        tableView.onRemove = { [weak self] in self?.removeSelected() }

        let menu = NSMenu()
        menu.delegate = self
        menu.addItem(withTitle: WelcomeStrings.open, action: #selector(openClickedRow(_:)), keyEquivalent: "")
        menu.addItem(withTitle: WelcomeStrings.showInFinder, action: #selector(revealClickedRow(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(
            withTitle: WelcomeStrings.removeFromRecents,
            action: #selector(removeClickedRow(_:)),
            keyEquivalent: ""
        )
        for item in menu.items { item.target = self }
        tableView.menu = menu

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    private func buildStateStack() {
        stateSpinner.style = .spinning
        stateSpinner.controlSize = .small
        stateSpinner.isDisplayedWhenStopped = false
        stateTitleLabel.alignment = .center
        stateMessageLabel.alignment = .center
        retryButton.target = self
        retryButton.action = #selector(retry(_:))

        for view in [stateSpinner, stateTitleLabel, stateMessageLabel, retryButton] as [NSView] {
            stateStack.addArrangedSubview(view)
        }
        stateStack.orientation = .vertical
        stateStack.alignment = .centerX
        stateStack.spacing = WelcomeMetrics.rowSpacing
        stateStack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stateStack)
        let margin = WelcomeMetrics.outerMargin
        NSLayoutConstraint.activate([
            stateStack.centerXAnchor.constraint(equalTo: centerXAnchor),
            stateStack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stateStack.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: margin),
            stateStack.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -margin),
            stateMessageLabel.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -2 * margin),
        ])
    }

    /// 상태를 반영합니다. 목록이 바뀌었을 때만 표를 다시 읽습니다.
    func apply(_ newState: WelcomeRecentProjectsState) {
        let previousState = state
        state = newState
        var showsTable = false
        stateSpinner.stopAnimation(nil)
        stateSpinner.isHidden = true
        retryButton.isHidden = true
        stateMessageLabel.isHidden = true

        switch newState {
        case .loading:
            stateSpinner.isHidden = false
            stateSpinner.startAnimation(nil)
            stateTitleLabel.stringValue = WelcomeStrings.loadingRecentProjects
            updateProjects([])
        case .loaded(let list) where list.isEmpty:
            stateTitleLabel.stringValue = WelcomeStrings.noRecentProjects
            stateMessageLabel.stringValue = WelcomeStrings.noRecentProjectsMessage
            stateMessageLabel.isHidden = false
            updateProjects([])
        case .loaded(let list):
            showsTable = true
            updateProjects(list)
        case .failed(let reason):
            stateTitleLabel.stringValue = WelcomeStrings.recentsFailed
            stateMessageLabel.stringValue = WelcomeStrings.failureReason(reason)
            stateMessageLabel.isHidden = false
            retryButton.isHidden = false
            updateProjects([])
        }
        scrollView.isHidden = !showsTable
        stateStack.isHidden = showsTable
        if previousState != newState, !showsTable, window != nil {
            // 상태 문구가 바뀌면 포커스와 관계없이 읽히도록 announcement를 요청합니다(AppKit NSAccessibility).
            WelcomeAccessibility.announce(stateTitleLabel.stringValue)
        }
    }

    private func updateProjects(_ list: [WelcomeRecentProject]) {
        guard list != projects else { return }
        let selectedID = selectedProject?.id
        projects = list
        tableView.reloadData()
        if let selectedID, let row = list.firstIndex(where: { $0.id == selectedID }) {
            tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
    }

    var selectedProject: WelcomeRecentProject? {
        project(at: tableView.selectedRow)
    }

    private func project(at row: Int) -> WelcomeRecentProject? {
        projects.indices.contains(row) ? projects[row] : nil
    }

    /// 문맥 메뉴·더블클릭은 클릭한 행, 그 밖에는 선택한 행을 대상으로 합니다.
    private var targetProject: WelcomeRecentProject? {
        tableView.clickedRow >= 0 ? project(at: tableView.clickedRow) : selectedProject
    }

    func openSelected() {
        if let project = selectedProject { onOpen?(project) }
    }

    func removeSelected() {
        if let project = selectedProject { onRemove?(project) }
    }

    /// 빈 곳을 더블클릭하면 선택한 행을 열지 않습니다.
    @objc private func doubleClicked(_ sender: Any?) {
        if let project = project(at: tableView.clickedRow) { onOpen?(project) }
    }

    @objc private func openClickedRow(_ sender: Any?) {
        if let project = targetProject { onOpen?(project) }
    }

    @objc private func revealClickedRow(_ sender: Any?) {
        if let project = targetProject { onReveal?(project) }
    }

    @objc private func removeClickedRow(_ sender: Any?) {
        if let project = targetProject { onRemove?(project) }
    }

    @objc private func retry(_ sender: Any?) {
        onRetry?()
    }

    // MARK: NSMenuDelegate

    func menuNeedsUpdate(_ menu: NSMenu) {
        let project = targetProject
        for item in menu.items where !item.isSeparatorItem {
            if item.action == #selector(revealClickedRow(_:)) {
                item.isEnabled = project.map { !$0.isMissing } ?? false
            } else {
                item.isEnabled = project != nil
            }
        }
    }

    // MARK: NSTableViewDataSource, NSTableViewDelegate

    func numberOfRows(in tableView: NSTableView) -> Int {
        projects.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let project = project(at: row) else { return nil }
        let cell = (tableView.makeView(withIdentifier: WelcomeRecentProjectCellView.identifier, owner: self)
            as? WelcomeRecentProjectCellView) ?? WelcomeRecentProjectCellView(frame: .zero)
        cell.configure(with: project, customActions: customActions(for: project))
        return cell
    }

    /// VoiceOver 사용자가 문맥 메뉴 없이도 같은 동작을 고를 수 있게 합니다(디자인 가이드 8.3).
    private func customActions(for project: WelcomeRecentProject) -> [NSAccessibilityCustomAction] {
        var actions: [NSAccessibilityCustomAction] = [
            NSAccessibilityCustomAction(name: WelcomeStrings.open) { [weak self] in
                self?.onOpen?(project)
                return true
            }
        ]
        if !project.isMissing {
            actions.append(
                NSAccessibilityCustomAction(name: WelcomeStrings.showInFinder) { [weak self] in
                    self?.onReveal?(project)
                    return true
                }
            )
        }
        actions.append(
            NSAccessibilityCustomAction(name: WelcomeStrings.removeFromRecents) { [weak self] in
                self?.onRemove?(project)
                return true
            }
        )
        return actions
    }
}
