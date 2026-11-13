import AppKit

/// 내장 에이전트 패널(로드맵, 가이드 6.5). 인스펙터 영역 탭 후보 자리에 넣을 수 있는 view controller입니다.
///
/// 구성(위→아래): 헤더(CLI·연결 상태·보기 전환·모드), 대화 기록(채팅 보기 기본 / 원문 텍스트 보기),
/// 빈·진행·오류 상태, 세션 변경 막대, 입력창(보내기·중지).
///
/// 데이터 흐름: `AgentPanelSource`에서 스냅샷과 이벤트를 pull하고 사용자 의도를 명령으로 보냅니다.
/// 이 컨트롤러는 문서 의미를 소유하지 않습니다(가이드 1.2 원칙 4). 되돌리기·승인·중지는 모두 명령이며
/// 결과는 이벤트로 돌아옵니다. 세션 종료·연결 끊김에서 자동 롤백을 하지 않습니다(PO 2026-11-14).
final class AgentPanelViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate,
    NSMenuDelegate, NSMenuItemValidation, AgentConversationActions
{
    let source: AgentPanelSource
    /// 확인 화면 표시 방식(검사에서 교체).
    var confirmationPresenter: AgentConfirmationPresenting = AgentAlertConfirmationPresenter()
    /// 주의 요청(알림) 처리. 종류만 받으므로 프로젝트 데이터를 알림에 넣을 수 없습니다.
    var attentionHandler: ((AgentAttentionKind) -> Void)?

    /// 현재 상태. 원문 텍스트는 text view만 보관하므로 `rawText`는 비워 둡니다.
    private(set) var model: AgentPanelSnapshot
    private var rowForItem: [AgentItemID: Int] = [:]
    private(set) var displayMode: AgentConversationDisplayMode = .chat

    private(set) var headerView: AgentPanelHeaderView!
    let tableView = NSTableView()
    let chatScrollView = NSScrollView()
    private(set) var rawScrollView: NSScrollView!
    private(set) var rawTextView: NSTextView!
    let stateView = AgentPanelStateView()
    private(set) var sessionBar: AgentSessionBarView!
    let composerView = AgentComposerView()
    private let contextMenu = NSMenu()

    private var contentToComposer: NSLayoutConstraint!
    private var sessionBarConstraints: [NSLayoutConstraint] = []
    private var pumpScheduled = false
    private var widthReloadScheduled = false
    private var wantsScrollToBottom = false
    private var bottomScrollScheduled = false
    private var displayOptionsObserver: NSObjectProtocol?

    init(source: AgentPanelSource) {
        self.source = source
        model = source.currentSnapshot()
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentPanelViewController는 코드로만 생성합니다")
    }

    deinit {
        if let displayOptionsObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(displayOptionsObserver)
        }
    }

    /// 활성 세션이 있으면 창 닫기·앱 종료 전에 확인해야 합니다(ADR-0005 결정 4.5). 통합 시 창 delegate가 씁니다.
    var requiresConfirmationBeforeClosing: Bool {
        model.header.isSessionActive
    }

    var canStartSession: Bool {
        let header = model.header
        if case .loading = model.phase { return false }
        return !header.isSessionActive && header.projectTrusted && header.selectedCLI?.availability == .ready
    }

    // MARK: - View

    override func loadView() {
        let root = NSView()
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel(AgentPanelText.panelTitle)

        headerView = AgentPanelHeaderView()
        configureHeader()
        configureTable()
        configureRawTextView()

        sessionBar = AgentSessionBarView(
            target: self,
            revertAllAction: #selector(revertAllAgentSessionChanges(_:)),
            startAction: #selector(startAgentSession(_:)))
        composerView.onSend = { [weak self] text in self?.send(.sendMessage(text)) }
        composerView.onStop = { [weak self] in self?.stopAgentSession(nil) }
        stateView.onAction = { [weak self] action in self?.performEmptyStateAction(action) }

        let content = NSView()
        content.translatesAutoresizingMaskIntoConstraints = false
        for view in [chatScrollView, rawScrollView!, stateView] as [NSView] {
            content.addSubview(view)
            NSLayoutConstraint.activate([
                view.leadingAnchor.constraint(equalTo: content.leadingAnchor),
                view.trailingAnchor.constraint(equalTo: content.trailingAnchor),
                view.topAnchor.constraint(equalTo: content.topAnchor),
                view.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            ])
        }
        content.setContentHuggingPriority(.init(1), for: .vertical)
        content.setContentCompressionResistancePriority(.init(1), for: .vertical)

        // 위→아래: 헤더, 기록(남는 높이 전부), [세션 막대], 입력창. 세션 막대는 보일 때만 계층에 둡니다.
        for subview in [headerView!, content, composerView] as [NSView] {
            root.addSubview(subview)
            NSLayoutConstraint.activate([
                subview.leadingAnchor.constraint(equalTo: root.leadingAnchor),
                subview.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            ])
        }
        NSLayoutConstraint.activate([
            headerView.topAnchor.constraint(equalTo: root.topAnchor),
            content.topAnchor.constraint(equalTo: headerView.bottomAnchor),
            composerView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        contentToComposer = content.bottomAnchor.constraint(equalTo: composerView.topAnchor)
        sessionBarConstraints = [
            sessionBar.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            sessionBar.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            sessionBar.topAnchor.constraint(equalTo: content.bottomAnchor),
            sessionBar.bottomAnchor.constraint(equalTo: composerView.topAnchor),
        ]
        contentToComposer.isActive = true
        view = root

        configureKeyViewLoop()
        applySnapshot(model)
        observeAccessibilityDisplayOptions()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        pumpEvents()
    }

    /// 열 하나가 대화 기록 폭 전체를 쓰게 합니다. 폭이 바뀌면 `tableViewColumnDidResize`가 높이를 다시 잽니다.
    override func viewDidLayout() {
        super.viewDidLayout()
        guard let column = tableView.tableColumns.first else { return }
        let available = chatScrollView.contentView.bounds.width
        if available > 0, abs(column.width - available) > 0.5 {
            // 폭 변경은 tableViewColumnDidResize가 재측정한 뒤 바닥 고정을 처리합니다.
            tableView.sizeLastColumnToFit()
        } else if wantsScrollToBottom, available > 0 {
            scheduleScrollToBottom()
        }
    }

    private func configureHeader() {
        headerView.cliPopUp.target = self
        headerView.cliPopUp.action = #selector(cliSelectionChanged(_:))
        headerView.modePopUp.target = self
        headerView.modePopUp.action = #selector(modeSelectionChanged(_:))
        headerView.displayModeControl.target = self
        headerView.displayModeControl.action = #selector(displayModeChanged(_:))

        let menu = NSMenu()
        menu.autoenablesItems = true
        let titleItem = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        titleItem.image = AgentPanelViews.symbol(AgentPanelSymbol.more, description: AgentPanelText.moreActions)
        menu.addItem(titleItem)
        func add(_ title: String, _ action: Selector) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        add(AgentPanelText.startSessionMenu, #selector(startAgentSession(_:)))
        add(AgentPanelText.stopSessionMenu, #selector(stopAgentSession(_:)))
        menu.addItem(.separator())
        add(AgentPanelText.revertAllMenu, #selector(revertAllAgentSessionChanges(_:)))
        menu.addItem(.separator())
        add(AgentPanelText.releaseLeasesMenu, #selector(releaseAllAgentLeases(_:)))
        headerView.moreButton.menu = menu
    }

    private func configureTable() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("AgentConversation"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .plain
        tableView.usesAutomaticRowHeights = true
        // 아직 그리지 않은 행의 높이 추정값(자동 행 높이). 실제 높이는 보일 때 잽니다.
        tableView.rowHeight = AgentPanelMetrics.estimatedRowHeight
        tableView.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        tableView.intercellSpacing = NSSize(width: 0, height: 2)
        tableView.backgroundColor = .clear
        tableView.allowsMultipleSelection = false
        tableView.allowsEmptySelection = true
        tableView.allowsTypeSelect = false
        tableView.setAccessibilityLabel(AgentPanelText.conversationLabel)
        tableView.dataSource = self
        tableView.delegate = self
        contextMenu.delegate = self
        tableView.menu = contextMenu

        chatScrollView.documentView = tableView
        chatScrollView.hasVerticalScroller = true
        chatScrollView.autohidesScrollers = true
        chatScrollView.drawsBackground = false
        chatScrollView.translatesAutoresizingMaskIntoConstraints = false
    }

    private func configureRawTextView() {
        let scroll = NSTextView.scrollableTextView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        // scrollableTextView()는 항상 NSTextView를 documentView로 만듭니다.
        let textView = (scroll.documentView as? NSTextView) ?? NSTextView()
        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.importsGraphics = false
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.font = Self.rawTextFont
        textView.textColor = .textColor
        textView.textContainerInset = NSSize(width: AgentPanelMetrics.itemSpacing, height: AgentPanelMetrics.itemSpacing)
        textView.setAccessibilityLabel(AgentPanelText.rawOutputLabel)
        rawScrollView = scroll
        rawTextView = textView
    }

    /// 패널 안 Tab 순서: 헤더 → 대화 기록 → 입력창 → 중지 → 보내기. 숨긴 뷰는 AppKit이 건너뜁니다.
    private func configureKeyViewLoop() {
        headerView.cliPopUp.nextKeyView = headerView.moreButton
        headerView.moreButton.nextKeyView = headerView.displayModeControl
        headerView.displayModeControl.nextKeyView = headerView.modePopUp
        headerView.modePopUp.nextKeyView = tableView
        tableView.nextKeyView = rawTextView
        rawTextView.nextKeyView = stateView.actionButton
        stateView.actionButton.nextKeyView = composerView.textView
        composerView.textView.nextKeyView = composerView.stopButton
        composerView.stopButton.nextKeyView = composerView.sendButton
    }

    private static let rawTextFont = NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)

    private func observeAccessibilityDisplayOptions() {
        displayOptionsObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            // queue: .main으로 등록했으므로 메인 스레드에서 실행됩니다.
            MainActor.assumeIsolated {
                self?.accessibilityDisplayOptionsChanged()
            }
        }
    }

    /// 투명도 줄이기·대비 높이기 변경을 보이는 카드에 반영합니다(가이드 8.4).
    func accessibilityDisplayOptionsChanged() {
        tableView.enumerateAvailableRowViews { rowView, _ in
            (rowView.view(atColumn: 0) as? AgentApprovalCellView)?.refreshAccessibilityDisplayOptions()
        }
    }

    // MARK: - 이벤트 pull

    /// 대기 중인 이벤트를 한 번에 최대 `maxEventsPerPass`개까지 적용합니다.
    /// 남은 이벤트가 있으면 다음 런 루프로 넘겨 UI 스레드 단일 점유를 짧게 유지합니다(아키텍처 §6.3).
    /// 통합 시 앱의 이벤트 펌프가 코어 이벤트 도착 때마다 이 메서드를 부릅니다.
    func pumpEvents() {
        pumpScheduled = false
        guard isViewLoaded else { return }
        var processed = 0
        var chromeChanged = false
        while processed < AgentPanelMetrics.maxEventsPerPass, let event = source.pollEvent() {
            processed += 1
            if apply(event) { chromeChanged = true }
        }
        if chromeChanged { updateChrome() }
        if processed == AgentPanelMetrics.maxEventsPerPass { schedulePump() }
    }

    private func schedulePump() {
        guard !pumpScheduled else { return }
        pumpScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.pumpEvents()
        }
    }

    /// 이벤트 하나를 적용합니다. 헤더·빈 상태 등 크롬 갱신이 필요하면 true.
    private func apply(_ event: AgentPanelEvent) -> Bool {
        if case .reset(let snapshot) = event.change {
            guard snapshot.generation >= model.generation else { return false }
            applySnapshot(snapshot)
            return false
        }
        // 세대 번호가 현재와 다른 결과는 버립니다(아키텍처 §9.2).
        guard event.generation == model.generation else { return false }
        switch event.change {
        case .reset:
            return false
        case .headerChanged(let header):
            model.header = header
            return true
        case .phaseChanged(let phase):
            model.phase = phase
            return true
        case .itemsAppended(let items):
            appendItems(items)
            return true
        case .itemUpdated(let item):
            updateItem(item)
            return false
        case .rawTextAppended(let text):
            appendRawText(text)
            return false
        case .changeSummaryChanged(let summary):
            model.changeSummary = summary
            return true
        case .attentionRequested(let kind):
            attentionHandler?(kind)
            return false
        }
    }

    private func applySnapshot(_ snapshot: AgentPanelSnapshot) {
        model = snapshot
        model.rawText = ""
        rowForItem = [:]
        for (row, item) in snapshot.items.enumerated() {
            rowForItem[item.id] = row
        }
        rawTextView.textStorage?.setAttributedString(
            NSAttributedString(string: snapshot.rawText, attributes: rawTextAttributes))
        tableView.reloadData()
        // 최신 항목이 보이도록 레이아웃(열 폭 확정) 뒤에 바닥으로 스크롤합니다.
        wantsScrollToBottom = true
        view.needsLayout = true
        updateChrome()
    }

    private var rawTextAttributes: [NSAttributedString.Key: Any] {
        [.font: Self.rawTextFont, .foregroundColor: NSColor.textColor]
    }

    private var rowAnimation: NSTableView.AnimationOptions {
        AgentPanelViews.reduceMotion ? [] : .effectFade
    }

    private var isConversationAtBottom: Bool {
        let visible = chatScrollView.contentView.bounds
        return visible.maxY >= tableView.bounds.maxY - AgentPanelMetrics.stickToBottomTolerance
    }

    /// 마지막 행으로 스크롤합니다. 자동 행 높이의 추정값이 실측으로 바뀌며 위치가 밀리므로
    /// 마지막 행이 실제로 보일 때까지 몇 번 반복합니다. 레이아웃 pass 밖에서만 부릅니다.
    private func scrollConversationToBottom() {
        guard !model.items.isEmpty else { return }
        for _ in 0..<4 {
            tableView.scrollRowToVisible(model.items.count - 1)
            chatScrollView.layoutSubtreeIfNeeded()
            if isLastRowVisible { return }
        }
    }

    private var isLastRowVisible: Bool {
        guard !model.items.isEmpty else { return true }
        let last = tableView.rect(ofRow: model.items.count - 1)
        return tableView.visibleRect.maxY >= last.maxY - AgentPanelMetrics.stickToBottomTolerance
    }

    /// 레이아웃 pass 안에서 요청된 바닥 스크롤을 다음 런 루프로 미룹니다.
    private func scheduleScrollToBottom() {
        guard !bottomScrollScheduled else { return }
        bottomScrollScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.bottomScrollScheduled = false
            // 한 번만 시도합니다. 남겨 두면 사용자의 스크롤과 다툴 수 있습니다.
            self.wantsScrollToBottom = false
            self.scrollConversationToBottom()
        }
    }

    private func appendItems(_ items: [AgentConversationItem]) {
        guard !items.isEmpty else { return }
        let wasAtBottom = isConversationAtBottom
        let start = model.items.count
        for (offset, item) in items.enumerated() {
            rowForItem[item.id] = start + offset
        }
        model.items.append(contentsOf: items)
        tableView.insertRows(at: IndexSet(start..<model.items.count), withAnimation: rowAnimation)
        if wasAtBottom { scrollConversationToBottom() }
    }

    private func updateItem(_ item: AgentConversationItem) {
        guard let row = rowForItem[item.id], row < model.items.count else { return }
        model.items[row] = item
        let rows = IndexSet(integer: row)
        tableView.reloadData(forRowIndexes: rows, columnIndexes: IndexSet(integer: 0))
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            self.tableView.noteHeightOfRows(withIndexesChanged: rows)
        }
    }

    private func appendRawText(_ text: String) {
        guard let storage = rawTextView.textStorage, !text.isEmpty else { return }
        let visible = rawScrollView.contentView.bounds
        let wasAtBottom = visible.maxY >= rawTextView.bounds.maxY - AgentPanelMetrics.stickToBottomTolerance
        storage.append(NSAttributedString(string: text, attributes: rawTextAttributes))
        if wasAtBottom && !rawScrollView.isHidden { rawTextView.scrollToEndOfDocument(nil) }
    }

    // MARK: - 크롬(헤더·빈 상태·입력창·세션 막대)

    private func updateChrome() {
        let header = model.header
        headerView.update(header: header, displayMode: displayMode)

        if let empty = emptyState() {
            stateView.configure(empty)
            stateView.isHidden = false
            chatScrollView.isHidden = true
            rawScrollView.isHidden = true
        } else {
            stateView.isHidden = true
            chatScrollView.isHidden = displayMode != .chat
            rawScrollView.isHidden = displayMode != .rawText
        }

        let ready = header.connection == .connected && model.phase == .ready && header.selectedCLI?.availability == .ready
        composerView.canSend = ready && !header.isAgentWorking
        composerView.canStop = header.isSessionActive
        if header.isAgentWorking {
            composerView.placeholder = AgentPanelText.composerPlaceholderWorking
        } else {
            composerView.placeholder = ready
                ? AgentPanelText.composerPlaceholderActive : AgentPanelText.composerPlaceholderInactive
        }

        var endedMessage: String?
        if case .failed(let reason) = model.phase, !model.items.isEmpty {
            endedMessage = AgentPanelText.failureTitle(reason)
        } else if header.connection == .ended, !model.items.isEmpty {
            endedMessage = AgentPanelText.sessionEndedBar
        }
        sessionBar.endedLabel.stringValue = endedMessage ?? AgentPanelText.sessionEndedBar
        let showsBar = sessionBar.update(
            summary: model.changeSummary, showsEnded: endedMessage != nil, canStart: canStartSession)
        setSessionBarVisible(showsBar)
    }

    /// 세션 막대를 계층에 넣거나 뺍니다. 빈 막대가 레이아웃을 모호하게 만들지 않도록 숨김 대신 제거합니다.
    private func setSessionBarVisible(_ visible: Bool) {
        let attached = sessionBar.superview != nil
        guard visible != attached else { return }
        if visible {
            contentToComposer.isActive = false
            view.addSubview(sessionBar)
            NSLayoutConstraint.activate(sessionBarConstraints)
        } else {
            NSLayoutConstraint.deactivate(sessionBarConstraints)
            sessionBar.removeFromSuperview()
            contentToComposer.isActive = true
        }
    }

    /// 세션 막대가 보이는지(검사용).
    var isSessionBarVisible: Bool {
        sessionBar.superview != nil
    }

    /// 대화 기록이 비어 있을 때만 빈 상태를 보여 줍니다. 기록이 있으면 에이전트 변경을 가리지 않습니다.
    func emptyState() -> AgentPanelEmptyState? {
        guard model.items.isEmpty else { return nil }
        let header = model.header
        switch model.phase {
        case .loading(let reason):
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.noSession, title: AgentPanelText.loadingTitle(reason), message: "",
                actionTitle: nil, action: nil, showsProgress: true)
        case .failed(let reason):
            let retry: AgentPanelEmptyState.Action = reason == .cliLaunchRejected ? .refreshInstalledCLIs : .startSession
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.failed, title: AgentPanelText.failureTitle(reason),
                message: AgentPanelText.failureMessage(reason),
                actionTitle: retry == .startSession ? AgentPanelText.startNewSession : "Check Again",
                action: retry == .startSession && !canStartSession ? nil : retry, showsProgress: false)
        case .ready:
            break
        }
        guard let cli = header.selectedCLI else {
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.noCLI, title: "No agent CLI found",
                message: "Install a supported coding agent CLI and sign in with it using its own instructions. "
                    + "Stellar runs the CLI you already use and doesn’t store keys or sign-in data.",
                actionTitle: "Check Again", action: .refreshInstalledCLIs, showsProgress: false)
        }
        switch cli.availability {
        case .unsupportedVersion:
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.unsupportedVersion, title: "This CLI version isn’t supported",
                message: "Stellar runs only CLI versions it has verified. Update \(cli.displayName), then check again.",
                actionTitle: "Check Again", action: .refreshInstalledCLIs, showsProgress: false)
        case .authenticationRequired:
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.authenticationRequired, title: "Sign in to \(cli.displayName)",
                message: "Sign in with the CLI’s own instructions. Stellar doesn’t take part in sign-in.",
                actionTitle: "Open Sign-In Terminal…", action: .openSignInTerminal, showsProgress: false)
        case .ready:
            break
        }
        if !header.projectTrusted {
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.untrusted, title: "Trust this project to use the agent",
                message: "Agent sessions start only in trusted projects.",
                actionTitle: "Review Project Trust…", action: .reviewProjectTrust, showsProgress: false)
        }
        if header.connection == .noSession || header.connection == .ended {
            return AgentPanelEmptyState(
                symbolName: AgentPanelSymbol.startSession, title: "Edit with your agent CLI",
                message: "Start a session to ask \(cli.displayName) to inspect this project and propose edits. "
                    + "Changes the agent applies stay marked so you can revert them.",
                actionTitle: AgentPanelText.startSessionMenu, action: .startSession, showsProgress: false)
        }
        return nil
    }

    private func performEmptyStateAction(_ action: AgentPanelEmptyState.Action) {
        switch action {
        case .refreshInstalledCLIs:
            send(.refreshInstalledCLIs)
        case .openSignInTerminal:
            if let cli = model.header.selectedCLI { send(.openSignInTerminal(cli.id)) }
        case .reviewProjectTrust:
            send(.reviewProjectTrust)
        case .startSession:
            startAgentSession(nil)
        }
    }

    // MARK: - 명령

    private func send(_ command: AgentPanelCommand) {
        source.perform(command)
        schedulePump()
    }

    private func confirm(_ confirmation: AgentConfirmation, then action: @escaping () -> Void) {
        confirmationPresenter.present(confirmation, for: view.window) { confirmed in
            if confirmed { action() }
        }
    }

    func setDisplayMode(_ mode: AgentConversationDisplayMode) {
        guard mode != displayMode else { return }
        displayMode = mode
        updateChrome()
        if mode == .rawText { rawTextView.scrollToEndOfDocument(nil) }
    }

    @objc func cliSelectionChanged(_ sender: NSPopUpButton) {
        guard let number = sender.selectedItem?.representedObject as? NSNumber else { return }
        let id = AgentCLIID(rawValue: number.uint64Value)
        guard id != model.header.selectedCLIID else { return }
        send(.selectCLI(id))
    }

    @objc func modeSelectionChanged(_ sender: NSPopUpButton) {
        let modes = AgentPermissionMode.allCases
        let index = sender.indexOfSelectedItem
        guard modes.indices.contains(index), modes[index] != model.header.mode else { return }
        send(.setMode(modes[index]))
    }

    @objc func displayModeChanged(_ sender: NSSegmentedControl) {
        setDisplayMode(AgentConversationDisplayMode(rawValue: sender.selectedSegment) ?? .chat)
    }

    /// View 메뉴 연결용(통합 시 메인 메뉴 항목이 first responder action으로 부릅니다).
    @objc func showAgentChatView(_ sender: Any?) {
        setDisplayMode(.chat)
    }

    @objc func showAgentRawTextView(_ sender: Any?) {
        setDisplayMode(.rawText)
    }

    @objc func startAgentSession(_ sender: Any?) {
        guard canStartSession, let cli = model.header.selectedCLI else { return }
        let mode = model.header.mode
        let confirmation = AgentConfirmation.startSession(
            projectName: model.header.projectDisplayName, cli: cli, mode: mode)
        confirm(confirmation) { [weak self] in
            self?.send(.startSession(cli: cli.id, mode: mode))
        }
    }

    /// 현재 세션 중지. 자동 롤백 없이 에이전트 변경을 표시한 채 남깁니다.
    @objc func stopAgentSession(_ sender: Any?) {
        guard model.header.isSessionActive else { return }
        send(.stopSession)
    }

    @objc func revertAllAgentSessionChanges(_ sender: Any?) {
        let count = model.changeSummary.revertableCount
        guard count > 0 else { return }
        confirm(.revertAllSessionChanges(count: count)) { [weak self] in
            self?.send(.revertAllSessionChanges)
        }
    }

    @objc func releaseAllAgentLeases(_ sender: Any?) {
        guard model.header.isSessionActive else { return }
        confirm(.releaseAllLeases()) { [weak self] in
            self?.send(.releaseAllProjectLeases)
        }
    }

    /// 선택한 행의 내용을 plain text로 복사합니다.
    @objc func copy(_ sender: Any?) {
        let row = tableView.selectedRow
        guard model.items.indices.contains(row) else { return }
        copyItem(at: row)
    }

    private func copyItem(at row: Int) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(Self.plainText(for: model.items[row]), forType: .string)
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(startAgentSession(_:))?:
            return canStartSession
        case #selector(stopAgentSession(_:))?, #selector(releaseAllAgentLeases(_:))?:
            return model.header.isSessionActive
        case #selector(revertAllAgentSessionChanges(_:))?:
            return model.changeSummary.revertableCount > 0
        case #selector(showAgentChatView(_:))?:
            menuItem.state = displayMode == .chat ? .on : .off
            return true
        case #selector(showAgentRawTextView(_:))?:
            menuItem.state = displayMode == .rawText ? .on : .off
            return true
        case #selector(copy(_:))?:
            return model.items.indices.contains(tableView.selectedRow)
        default:
            return true
        }
    }

    // MARK: - AgentConversationActions

    func conversationRequestsDecision(_ decision: AgentApprovalDecision, for request: AgentApprovalRequest) {
        guard request.status == .pending else { return }
        let command = AgentPanelCommand.respondToApproval(request.approvalID, decision)
        if decision == .approve && request.kind.requiresSensitiveConfirmation {
            // 민감 확인은 파괴적 동작 확인 패턴의 sheet를 거칩니다(ADR-0005 결정 4.5).
            confirm(.sensitiveApproval(request.kind)) { [weak self] in
                self?.send(command)
            }
        } else {
            send(command)
        }
    }

    func conversationRequestsPreview(_ approvalID: AgentApprovalID) {
        send(.previewApproval(approvalID))
    }

    func conversationRequestsRevert(_ changeID: AgentChangeID) {
        send(.revertChange(changeID))
    }

    // MARK: - NSTableViewDataSource / Delegate

    func numberOfRows(in tableView: NSTableView) -> Int {
        model.items.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard model.items.indices.contains(row) else { return nil }
        let item = model.items[row]
        switch item.content {
        case .approval(let request):
            let cell = dequeue(AgentApprovalCellView.reuseIdentifier) { AgentApprovalCellView(frame: .zero) }
            cell.actions = self
            cell.configure(request: request)
            return cell
        case .appliedChange(let change):
            let cell = dequeue(AgentChangeCellView.reuseIdentifier) { AgentChangeCellView(frame: .zero) }
            cell.actions = self
            cell.configure(change: change)
            return cell
        case .userMessage, .agentMessage, .toolStep, .notice:
            let cell = dequeue(AgentTextRowCellView.reuseIdentifier) { AgentTextRowCellView(frame: .zero) }
            cell.actions = self
            cell.configure(content: item.content)
            return cell
        }
    }

    private func dequeue<Cell: NSView>(_ identifier: NSUserInterfaceItemIdentifier, make: () -> Cell) -> Cell {
        (tableView.makeView(withIdentifier: identifier, owner: self) as? Cell) ?? make()
    }

    /// 열 폭이 바뀌면 여러 줄 레이블 높이를 다시 재도록 보이는 행만 다시 불러옵니다(다음 런 루프로 합침).
    func tableViewColumnDidResize(_ notification: Notification) {
        guard !widthReloadScheduled else { return }
        widthReloadScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.widthReloadScheduled = false
            let wasAtBottom = self.isConversationAtBottom || self.wantsScrollToBottom
            let selection = self.tableView.selectedRowIndexes
            self.tableView.reloadData()
            self.tableView.selectRowIndexes(selection, byExtendingSelection: false)
            self.wantsScrollToBottom = false
            if wasAtBottom { self.scrollConversationToBottom() }
        }
    }

    // MARK: - 문맥 메뉴

    private enum ContextAction: Int {
        case copy = 1
        case approve
        case decline
        case preview
        case revert
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let row = tableView.clickedRow >= 0 ? tableView.clickedRow : tableView.selectedRow
        guard model.items.indices.contains(row) else { return }
        func add(_ title: String, _ action: ContextAction) {
            let item = NSMenuItem(title: title, action: #selector(contextMenuAction(_:)), keyEquivalent: "")
            item.target = self
            item.tag = action.rawValue
            item.representedObject = NSNumber(value: row)
            menu.addItem(item)
        }
        add(AgentPanelText.copy, .copy)
        switch model.items[row].content {
        case .approval(let request) where request.status == .pending:
            menu.addItem(.separator())
            add(request.kind.requiresSensitiveConfirmation ? AgentPanelText.approveSensitive : AgentPanelText.approve,
                .approve)
            add(AgentPanelText.decline, .decline)
            add(AgentPanelText.previewToolTip, .preview)
        case .appliedChange(let change) where change.state == .applied:
            menu.addItem(.separator())
            add(AgentPanelText.revertToolTip, .revert)
        default:
            break
        }
    }

    @objc private func contextMenuAction(_ sender: NSMenuItem) {
        guard let row = (sender.representedObject as? NSNumber)?.intValue, model.items.indices.contains(row),
            let action = ContextAction(rawValue: sender.tag)
        else { return }
        let content = model.items[row].content
        switch (action, content) {
        case (.copy, _):
            copyItem(at: row)
        case (.approve, .approval(let request)):
            conversationRequestsDecision(.approve, for: request)
        case (.decline, .approval(let request)):
            conversationRequestsDecision(.decline, for: request)
        case (.preview, .approval(let request)):
            conversationRequestsPreview(request.approvalID)
        case (.revert, .appliedChange(let change)):
            if change.state == .applied { conversationRequestsRevert(change.changeID) }
        default:
            break
        }
    }

    // MARK: - 텍스트 표현

    /// 복사·검사용 plain text. 에이전트 설명은 출처를 함께 적습니다.
    static func plainText(for item: AgentConversationItem) -> String {
        switch item.content {
        case .userMessage(let text):
            return "\(AgentPanelText.you): \(text)"
        case .agentMessage(let text, _):
            return "\(AgentPanelText.agent): \(text)"
        case .toolStep(let step):
            var text = "\(step.title) (\(AgentPanelText.toolStatusTitle(step.status)), \(AgentPanelText.stellarRecord))"
            if let detail = step.detail { text += "\n\(detail)" }
            return text
        case .approval(let request):
            var lines = ["\(AgentPanelText.approvalNeeded) · \(AgentPanelText.approvalKindTitle(request.kind))",
                         request.summary]
            for line in request.diffLines {
                var text = "\(AgentPanelText.diffChangeTitle(line.change)): \(line.target)"
                if let detail = line.detail { text += " — \(detail)" }
                lines.append(text)
            }
            if request.omittedDiffLineCount > 0 {
                lines.append(AgentPanelText.omittedDiffLines(request.omittedDiffLineCount))
            }
            lines.append("\(AgentPanelText.validationSection): \(AgentPanelText.validationTitle(request.validation))")
            lines.append(contentsOf: request.validation.messages)
            lines.append("\(AgentPanelText.lockScopeSection): \(request.lockScope)")
            if let explanation = request.agentExplanation {
                lines.append("\(AgentPanelText.agentNoteSection) (\(AgentPanelText.agentNoteProvenance)): \(explanation)")
            }
            lines.append(AgentPanelText.approvalStatusTitle(request.status))
            return lines.joined(separator: "\n")
        case .appliedChange(let change):
            var text = "\(AgentPanelText.appliedByAgent): \(change.summary)"
            if let detail = change.detail { text += "\n\(detail)" }
            return text
        case .notice(let notice):
            return AgentPanelText.noticeText(notice)
        }
    }
}
