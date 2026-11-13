import AppKit

/// 최소 Git 패널(아키텍처 §24.4, §26, §27; 계획 XIV.13.5 범위).
///
/// Status·Semantic Diff, 객체·속성 단위 Stage·Unstage, Commit, Push(강제 없음), Semantic Conflict 해결만
/// 제공합니다. 사이드바 구획이나 인스펙터 탭 어느 쪽에도 넣을 수 있는 독립 view controller입니다
/// (디자인 가이드 6.4, 열린 결정 O4). Git 작업은 data source가 메인 스레드 밖에서 처리하고,
/// 이 controller는 이벤트를 받아 네이티브 컨트롤만 갱신합니다(아키텍처 §6.4, §9.2).
@MainActor
final class GitPanelViewController: NSViewController, NSTextViewDelegate {
    enum Mode: Int {
        case changes = 0
        case conflicts = 1
    }

    /// 변경 상세 표시 항목.
    enum DetailField: CaseIterable {
        case kind, object, property, head, index, working, requires, status

        var name: String {
            switch self {
            case .kind: return GitPanelStrings.kindLabel
            case .object: return GitPanelStrings.objectLabel
            case .property: return GitPanelStrings.propertyLabel
            case .head: return GitPanelStrings.headLabel
            case .index: return GitPanelStrings.indexLabel
            case .working: return GitPanelStrings.workingLabel
            case .requires: return GitPanelStrings.requiresLabel
            case .status: return "Status"
            }
        }
    }

    /// conflict 상세 표시 항목.
    enum ConflictField: CaseIterable {
        case kind, path, base, ours, theirs, choice

        var name: String {
            switch self {
            case .kind: return GitPanelStrings.kindLabel
            case .path: return "Path"
            case .base: return GitPanelStrings.base
            case .ours: return GitPanelStrings.ours
            case .theirs: return GitPanelStrings.theirs
            case .choice: return "Choice"
            }
        }
    }

    /// 이벤트 드레인 진단(검사용).
    struct Diagnostics {
        var drains = 0
        var events = 0
        var maxDrainSeconds: TimeInterval = 0
    }

    let dataSource: GitPanelDataSource
    weak var host: GitPanelHostActions?

    private(set) var status: GitStatusSnapshot?
    private(set) var activeOperation: GitOperationProgress?
    private(set) var lastResult: GitOperationResult?
    private(set) var pendingPrompt: GitDependencyPrompt?
    private(set) var currentDetail: GitChangeDetail?
    private(set) var mode: Mode = .changes
    private(set) var displayOptions = GitDisplayOptions.current()
    private(set) var diagnostics = Diagnostics()

    private var drainScheduled = false
    private var detailUpdateScheduled = false
    private var requestedDetail: (generation: UInt64, row: Int)?
    private var presentingDependencySheet = false
    private var displayOptionsToken: GitNotificationToken?
    private var secondaryLabels: [NSTextField] = []

    // MARK: 머리 영역

    let rootColumn = GitColumnView(
        spacing: GitPanelMetrics.rowSpacing,
        insets: NSEdgeInsets(
            top: GitPanelMetrics.outerInset,
            left: GitPanelMetrics.outerInset,
            bottom: GitPanelMetrics.outerInset,
            right: GitPanelMetrics.outerInset
        )
    )
    let branchIcon = GitControls.symbolView(GitPanelSymbols.branch, label: "Branch")
    let branchLabel = GitControls.label(GitPanelStrings.loading, style: .headline)
    let mergeStateLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let aheadIcon = GitControls.symbolView(GitPanelSymbols.ahead, label: "To push", style: .callout)
    let aheadLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let behindIcon = GitControls.symbolView(GitPanelSymbols.behind, label: "To pull", style: .callout)
    let behindLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let refreshButton = GitControls.iconButton(
        symbol: GitPanelSymbols.refresh,
        label: GitPanelStrings.refresh,
        target: nil,
        action: #selector(GitPanelViewController.refresh(_:))
    )
    let summaryLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let modeControl = NSSegmentedControl(
        labels: [GitPanelStrings.changesTab, GitPanelStrings.conflictsTab],
        trackingMode: .selectOne,
        target: nil,
        action: nil
    )

    // MARK: 배너

    let bannerColumn = GitColumnView()
    let autosaveBanner = GitBannerView(symbol: GitPanelSymbols.save, symbolLabel: "Unsaved edits", tint: .systemOrange)
    let semanticErrorBanner = GitBannerView(symbol: GitPanelSymbols.warning, symbolLabel: "Status error", tint: .systemOrange)
    let dependencyBanner = GitBannerView(symbol: GitPanelSymbols.dependency, symbolLabel: "Related changes", tint: .systemOrange)
    let progressBanner = GitBannerView(symbol: GitPanelSymbols.info, symbolLabel: "In progress", tint: .secondaryLabelColor)
    let resultBanner = GitBannerView(symbol: GitPanelSymbols.info, symbolLabel: "Result", tint: .secondaryLabelColor)
    let saveNowButton = NSButton(title: GitPanelStrings.saveNow, target: nil, action: nil)
    let useSavedButton = NSButton(title: GitPanelStrings.useSavedVersion, target: nil, action: nil)
    let retryButton = NSButton(title: GitPanelStrings.refresh, target: nil, action: nil)
    let includeRequiredButton = NSButton(title: "", target: nil, action: nil)
    let declineRequiredButton = NSButton(title: GitPanelStrings.cancel, target: nil, action: nil)
    let cancelOperationButton = NSButton(title: GitPanelStrings.cancel, target: nil, action: nil)
    let dismissResultButton = NSButton(title: GitPanelStrings.dismiss, target: nil, action: nil)

    // MARK: 변경 화면

    let changesColumn = GitColumnView()
    let changeList = GitChangeListController()
    let changeListArea = NSView()
    let changesOverlay = GitStateOverlayView()
    let detailTitle = GitControls.label(GitPanelStrings.details, style: .headline)
    let detailPlaceholder = GitControls.label(GitPanelStrings.selectChange, style: .callout, color: .secondaryLabelColor)
    let detailColumn = GitColumnView(spacing: GitPanelMetrics.inlineSpacing)
    let detailRows: [DetailField: GitDetailRowView]
    let commitLabel = GitControls.label(GitPanelStrings.commitMessage, style: .callout, color: .secondaryLabelColor)
    let commitScrollView: NSScrollView
    let commitTextView: NSTextView
    let commitButton = NSButton(title: GitPanelStrings.commit, target: nil, action: nil)
    let commitReasonLabel = GitControls.wrappingLabel()
    let identityButton = NSButton(title: GitPanelStrings.setUpIdentity, target: nil, action: nil)
    let identityRow: NSStackView
    let pushButton = NSButton(title: GitPanelStrings.push, target: nil, action: nil)
    let remotePopup = NSPopUpButton(frame: .zero, pullsDown: false)
    let pushRow: NSStackView
    let pushInfoLabel = GitControls.wrappingLabel()

    // MARK: conflict 화면

    let conflictsColumn = GitColumnView()
    let conflictList = GitConflictListController()
    let conflictListArea = NSView()
    let conflictsOverlay = GitStateOverlayView()
    let conflictDetailTitle = GitControls.label(GitPanelStrings.details, style: .headline)
    let conflictDetailPlaceholder = GitControls.label(GitPanelStrings.selectConflict, style: .callout, color: .secondaryLabelColor)
    let conflictDetailColumn = GitColumnView(spacing: GitPanelMetrics.inlineSpacing)
    let conflictRows: [ConflictField: GitDetailRowView]
    let revealButton = NSButton(title: GitPanelStrings.revealConflict, target: nil, action: nil)
    let revealRow: NSStackView
    let applyButton = NSButton(title: GitPanelStrings.applyResolutions, target: nil, action: nil)
    let applyReasonLabel = GitControls.wrappingLabel()
    let continueHintLabel = GitControls.wrappingLabel(GitPanelStrings.continueInTerminal)

    init(dataSource: GitPanelDataSource, host: GitPanelHostActions? = nil) {
        self.dataSource = dataSource
        self.host = host
        let detailWidth = GitDetailRowView.nameColumnWidth(for: DetailField.allCases.map(\.name))
        var rows: [DetailField: GitDetailRowView] = [:]
        for field in DetailField.allCases {
            rows[field] = GitDetailRowView(name: field.name, nameWidth: detailWidth, monospaced: field == .property)
        }
        detailRows = rows
        let conflictWidth = GitDetailRowView.nameColumnWidth(for: ConflictField.allCases.map(\.name))
        var conflictFields: [ConflictField: GitDetailRowView] = [:]
        for field in ConflictField.allCases {
            conflictFields[field] = GitDetailRowView(name: field.name, nameWidth: conflictWidth, monospaced: field == .path)
        }
        conflictRows = conflictFields
        let message = NativeTextControls.makeMultilineTextView()
        commitScrollView = message.scrollView
        commitTextView = message.textView
        identityRow = GitControls.leadingRow([identityButton])
        pushRow = GitControls.leadingRow([pushButton, remotePopup])
        revealRow = GitControls.leadingRow([revealButton])
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitPanelViewController는 코드로만 생성합니다")
    }

    // MARK: 생명주기

    override func loadView() {
        let root = NSView()
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel(GitPanelStrings.panelTitle)
        view = root
        configureControls()
        buildLayout()
        setMode(.changes)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        dataSource.onEventsAvailable = { [weak self] in self?.scheduleDrain() }
        let center = NSWorkspace.shared.notificationCenter
        let token = center.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.displayOptionsChanged() }
        }
        displayOptionsToken = GitNotificationToken(center: center, token: token)
        applyDisplayOptions()
        updateStatusUI()
        dataSource.send(.refreshStatus)
    }

    // MARK: 구성

    private func configureControls() {
        refreshButton.target = self
        modeControl.target = self
        modeControl.action = #selector(modeChanged(_:))
        modeControl.segmentDistribution = .fillEqually
        modeControl.selectedSegment = Mode.changes.rawValue
        modeControl.setAccessibilityLabel(GitPanelStrings.modeLabel)
        // 화살표 아이콘은 옆 숫자 label이 같은 뜻을 전체 문장으로 읽으므로 VoiceOver에서 뺍니다.
        aheadIcon.setAccessibilityElement(false)
        behindIcon.setAccessibilityElement(false)
        aheadLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        behindLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        mergeStateLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)

        let actions: [(NSButton, Selector)] = [
            (saveNowButton, #selector(saveBeforeStatus(_:))),
            (useSavedButton, #selector(useSavedVersion(_:))),
            (retryButton, #selector(refresh(_:))),
            (includeRequiredButton, #selector(includeRequired(_:))),
            (declineRequiredButton, #selector(declineRequired(_:))),
            (cancelOperationButton, #selector(cancelGitOperation(_:))),
            (dismissResultButton, #selector(dismissResult(_:))),
            (commitButton, #selector(commit(_:))),
            (identityButton, #selector(setUpIdentity(_:))),
            (pushButton, #selector(push(_:))),
            (revealButton, #selector(revealConflict(_:))),
            (applyButton, #selector(applyResolutions(_:))),
        ]
        for (button, action) in actions {
            button.target = self
            button.action = action
            button.bezelStyle = .push
        }

        // 커밋: 이 문맥의 대표 동작 하나만 prominent로 둡니다(디자인 가이드 4.2, 5.1).
        // 여러 줄 입력에서 Return은 줄바꿈이므로 Command-Return을 키 등가로 씁니다.
        commitButton.tintProminence = .primary
        commitButton.keyEquivalent = "\r"
        commitButton.keyEquivalentModifierMask = [.command]
        commitButton.toolTip = "\(GitPanelStrings.commit) (⌘↩)"
        applyButton.tintProminence = .primary
        pushButton.image = GitPanelSymbols.image(GitPanelSymbols.ahead, label: GitPanelStrings.push)
        pushButton.imagePosition = .imageLeading
        revealButton.image = GitPanelSymbols.image(GitPanelSymbols.reveal, label: GitPanelStrings.revealConflict)
        revealButton.imagePosition = .imageLeading
        revealButton.controlSize = .small
        identityButton.controlSize = .small

        remotePopup.setAccessibilityLabel(GitPanelStrings.pushRemote)
        remotePopup.toolTip = GitPanelStrings.pushRemote

        commitTextView.delegate = self
        commitTextView.setAccessibilityLabel(GitPanelStrings.commitMessage)
        commitTextView.isAutomaticQuoteSubstitutionEnabled = false
        commitTextView.isAutomaticDashSubstitutionEnabled = false
        commitTextView.isContinuousSpellCheckingEnabled = true
        commitScrollView.translatesAutoresizingMaskIntoConstraints = false

        secondaryLabels = [mergeStateLabel, aheadLabel, behindLabel, summaryLabel, detailPlaceholder, commitLabel,
                           commitReasonLabel, pushInfoLabel, conflictDetailPlaceholder, applyReasonLabel, continueHintLabel]
            + detailRows.values.map(\.nameField) + conflictRows.values.map(\.nameField)

        changeList.requestPage = { [weak self] generation, rows in
            self?.dataSource.send(.loadChangePage(generation: generation, rows: rows))
        }
        changeList.setStaged = { [weak self] rows, staged in self?.setStaged(rows: rows, staged: staged) }
        changeList.setExpanded = { [weak self] row, expanded in
            guard let self else { return }
            self.dataSource.send(.setGroupExpanded(generation: self.changeList.generation, row: row, expanded: expanded))
        }
        changeList.selectionChanged = { [weak self] in self?.scheduleDetailUpdate() }

        conflictList.requestPage = { [weak self] generation, rows in
            self?.dataSource.send(.loadConflictPage(generation: generation, rows: rows))
        }
        conflictList.choose = { [weak self] rows, choice in
            guard let self else { return }
            self.dataSource.send(.chooseResolution(generation: self.conflictList.generation, rows: rows, choice: choice))
        }
        conflictList.reveal = { [weak self] row in self?.reveal(row: row) }
        conflictList.selectionChanged = { [weak self] in self?.updateConflictDetail() }
    }

    private func buildLayout() {
        let header = GitControls.leadingRow(
            [branchIcon, branchLabel, mergeStateLabel],
            trailing: [aheadIcon, aheadLabel, behindIcon, behindLabel, refreshButton]
        )
        header.setAccessibilityElement(false)

        bannerColumn.setItems([autosaveBanner, semanticErrorBanner, dependencyBanner, progressBanner, resultBanner])
        for banner in [autosaveBanner, semanticErrorBanner, dependencyBanner, progressBanner, resultBanner] {
            bannerColumn.setItem(banner, visible: false)
        }

        // 변경 목록 영역: 목록 위에 빈 상태·로딩 오버레이를 겹칩니다.
        install(list: changeList.scrollView, overlay: changesOverlay, in: changeListArea, rowHeight: changeList.tableView.rowHeight)
        install(list: conflictList.scrollView, overlay: conflictsOverlay, in: conflictListArea, rowHeight: conflictList.tableView.rowHeight)

        detailColumn.setItems([detailPlaceholder] + DetailField.allCases.compactMap { detailRows[$0] })
        DetailField.allCases.compactMap { detailRows[$0] }.forEach { detailColumn.setItem($0, visible: false) }
        conflictDetailColumn.setItems([conflictDetailPlaceholder] + ConflictField.allCases.compactMap { conflictRows[$0] })
        ConflictField.allCases.compactMap { conflictRows[$0] }.forEach { conflictDetailColumn.setItem($0, visible: false) }

        // 커밋 메시지 높이: 실제 글꼴 줄 높이에서 계산합니다(디자인 가이드 4.4).
        let font = commitTextView.font ?? NSFont.preferredFont(forTextStyle: .body)
        let lineHeight = NSLayoutManager().defaultLineHeight(for: font)
        let contentHeight = ceil(lineHeight * GitPanelMetrics.commitMessageLines + commitTextView.textContainerInset.height * 2)
        let frameSize = NSScrollView.frameSize(
            forContentSize: NSSize(width: 0, height: contentHeight),
            horizontalScrollerClass: nil,
            verticalScrollerClass: NSScroller.self,
            borderType: commitScrollView.borderType,
            controlSize: .regular,
            scrollerStyle: NSScroller.preferredScrollerStyle
        )
        commitScrollView.heightAnchor.constraint(equalToConstant: ceil(frameSize.height)).isActive = true

        changesColumn.setItems([
            changeListArea, GitControls.separator(), detailTitle, detailColumn, GitControls.separator(),
            commitLabel, commitScrollView, commitButton, commitReasonLabel, identityRow, GitControls.separator(),
            pushRow, pushInfoLabel,
        ])
        changesColumn.flexibleView = changeListArea
        changesColumn.setItem(identityRow, visible: false)

        conflictsColumn.setItems([
            conflictListArea, GitControls.separator(), conflictDetailTitle, conflictDetailColumn, revealRow,
            GitControls.separator(), applyButton, applyReasonLabel, continueHintLabel,
        ])
        conflictsColumn.flexibleView = conflictListArea
        conflictsColumn.setItem(revealRow, visible: false)

        rootColumn.setItems([header, summaryLabel, modeControl, bannerColumn, changesColumn, conflictsColumn])
        rootColumn.setItem(bannerColumn, visible: false)
        view.addSubview(rootColumn)
        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            rootColumn.topAnchor.constraint(equalTo: guide.topAnchor),
            rootColumn.leadingAnchor.constraint(equalTo: guide.leadingAnchor),
            rootColumn.trailingAnchor.constraint(equalTo: guide.trailingAnchor),
            rootColumn.bottomAnchor.constraint(equalTo: guide.bottomAnchor),
        ])
    }

    private func install(list: NSScrollView, overlay: GitStateOverlayView, in area: NSView, rowHeight: CGFloat) {
        area.translatesAutoresizingMaskIntoConstraints = false
        area.addSubview(list)
        area.addSubview(overlay)
        NSLayoutConstraint.activate([
            list.topAnchor.constraint(equalTo: area.topAnchor),
            list.leadingAnchor.constraint(equalTo: area.leadingAnchor),
            list.trailingAnchor.constraint(equalTo: area.trailingAnchor),
            list.bottomAnchor.constraint(equalTo: area.bottomAnchor),
            overlay.topAnchor.constraint(equalTo: area.topAnchor),
            overlay.leadingAnchor.constraint(equalTo: area.leadingAnchor),
            overlay.trailingAnchor.constraint(equalTo: area.trailingAnchor),
            overlay.bottomAnchor.constraint(equalTo: area.bottomAnchor),
            area.heightAnchor.constraint(greaterThanOrEqualToConstant: ceil(rowHeight * GitPanelMetrics.minimumVisibleRows)),
        ])
    }

    /// 키보드 포커스 순서(디자인 가이드 8.2). 숨긴 화면의 컨트롤은 건너뜁니다.
    private func configureKeyLoop() {
        let chain: [NSView]
        switch mode {
        case .changes:
            chain = [modeControl, changeList.tableView, commitTextView, commitButton, identityButton, pushButton, remotePopup, refreshButton]
        case .conflicts:
            chain = [modeControl, conflictList.tableView, revealButton, applyButton, refreshButton]
        }
        for (index, view) in chain.enumerated() {
            view.nextKeyView = chain[(index + 1) % chain.count]
        }
    }

    private func applyDisplayOptions() {
        let color = displayOptions.secondaryTextColor
        secondaryLabels.forEach { $0.textColor = color }
        changeList.secondaryColor = color
        conflictList.secondaryColor = color
    }

    private func displayOptionsChanged() {
        let options = GitDisplayOptions.current()
        guard options != displayOptions else { return }
        displayOptions = options
        applyDisplayOptions()
    }

    // MARK: 이벤트

    private func scheduleDrain() {
        guard !drainScheduled else { return }
        drainScheduled = true
        DispatchQueue.main.async { [weak self] in self?.drainEvents() }
    }

    /// 이벤트 큐를 메인 스레드 예산 안에서 비웁니다. 예산을 넘으면 다음 런 루프로 넘깁니다(아키텍처 §6.3).
    func drainEvents() {
        drainScheduled = false
        let start = ProcessInfo.processInfo.systemUptime
        var needsStatusUI = false
        var needsDetail = false
        var needsConflictDetail = false
        while let event = dataSource.pollEvent() {
            diagnostics.events += 1
            switch event {
            case .status(let snapshot):
                let previous = status
                status = snapshot
                needsStatusUI = true
                if previous?.changes != snapshot.changes { needsDetail = true }
                if previous?.conflicts != snapshot.conflicts { needsConflictDetail = true }
            case .changePage(let page):
                changeList.receive(page)
                let selected = changeList.tableView.selectedRow
                if selected >= page.startRow, selected < page.startRow + page.rows.count { needsDetail = true }
            case .changeDetail(let detail):
                let selection = changeList.tableView.selectedRowIndexes
                if detail.generation == changeList.generation, selection.count == 1, selection.first == detail.row {
                    currentDetail = detail
                    showDetail(detail)
                }
            case .conflictPage(let page):
                conflictList.receive(page)
                needsConflictDetail = true
            case .dependencyPrompt(let prompt):
                pendingPrompt = prompt
                presentDependencyPrompt()
                needsStatusUI = true
            case .operationProgress(let progress):
                activeOperation = progress
                needsStatusUI = true
            case .operationFinished(let result):
                if activeOperation?.id == result.id { activeOperation = nil }
                handleFinished(result)
                needsStatusUI = true
            }
            if ProcessInfo.processInfo.systemUptime - start > GitPanelMetrics.mainThreadBudget {
                scheduleDrain()
                break
            }
        }
        if needsStatusUI { updateStatusUI() }
        if needsDetail { scheduleDetailUpdate() }
        if needsConflictDetail { updateConflictDetail() }
        diagnostics.drains += 1
        diagnostics.maxDrainSeconds = max(diagnostics.maxDrainSeconds, ProcessInfo.processInfo.systemUptime - start)
    }

    private func handleFinished(_ result: GitOperationResult) {
        switch (result.kind, result.outcome) {
        case (.commit, .succeeded):
            clearCommitMessage()
            lastResult = result
        case (.push, .succeeded), (.applyResolutions, .succeeded):
            lastResult = result
        case (_, .failed):
            lastResult = result
        default:
            // Stage·Unstage·새로고침 성공과 취소는 목록 변화로 충분하므로 배너를 띄우지 않습니다.
            if case .failed = lastResult?.outcome {} else { lastResult = nil }
        }
    }

    // MARK: 상태 반영

    var isBusy: Bool { activeOperation != nil }

    var commitBlockers: [GitCommitBlocker] {
        GitPanelRules.commitBlockers(status: status, message: commitTextView.string, operationInProgress: isBusy)
    }

    var pushBlockers: [GitPushBlocker] {
        GitPanelRules.pushBlockers(status: status, operationInProgress: isBusy)
    }

    var applyBlockers: [GitApplyBlocker] {
        GitPanelRules.applyBlockers(status: status, operationInProgress: isBusy)
    }

    private func updateStatusUI() {
        guard isViewLoaded else { return }
        // 표시 변경을 묶어 각 구획의 제약을 한 번씩만 다시 만듭니다. 배너가 나타나고 사라지는 전환은
        // 동작 줄이기가 꺼져 있고 창이 보일 때만 짧게 움직입니다(디자인 가이드 8.5).
        animateLayout {
            rootColumn.batchUpdates {
                changesColumn.batchUpdates {
                    conflictsColumn.batchUpdates {
                        bannerColumn.batchUpdates { applyStatusUI() }
                    }
                }
            }
        }
    }

    private func applyStatusUI() {
        let ready = status?.availability == .ready
        branchLabel.setStringIfChanged(status.map { GitPanelStrings.head($0.head) } ?? GitPanelStrings.loading)
        changeList.update(status?.changes, interactive: ready && !isBusy && pendingPrompt == nil)
        conflictList.update(status?.conflicts, interactive: ready && !isBusy)
        let merge = status.flatMap { GitPanelStrings.mergeState($0.mergeState) }
        mergeStateLabel.setStringIfChanged(merge ?? "")
        mergeStateLabel.isHidden = merge == nil

        let upstream = ready ? status?.upstream : nil
        for view in [aheadIcon, aheadLabel, behindIcon, behindLabel] as [NSView] {
            view.isHidden = upstream == nil
        }
        if let upstream {
            aheadLabel.setStringIfChanged("\(upstream.ahead)")
            behindLabel.setStringIfChanged("\(upstream.behind)")
            let text = GitPanelStrings.aheadBehind(ahead: upstream.ahead, behind: upstream.behind)
            aheadLabel.setAccessibilityLabel(text)
            behindLabel.setAccessibilityLabel(text)
        }
        if let status, ready {
            summaryLabel.setStringIfChanged(GitPanelStrings.statusSummary(
                staged: status.stagedChangeCount,
                unstaged: status.unstagedChangeCount
            ))
        }
        rootColumn.setItem(summaryLabel, visible: ready)
        let conflictTotal = (status?.semanticConflictCount ?? 0) + (status?.rawConflictCount ?? 0)
        let conflictsLabel = GitPanelStrings.conflictsTab(count: conflictTotal)
        if modeControl.label(forSegment: Mode.conflicts.rawValue) != conflictsLabel {
            modeControl.setLabel(conflictsLabel, forSegment: Mode.conflicts.rawValue)
        }
        updateOverlays()
        updateBanners()
        updateCommitArea()
        updatePushArea()
        updateApplyArea()
    }

    private func updateOverlays() {
        configureOverlay(
            changesOverlay,
            list: changeList.scrollView,
            rowCount: status?.changes.rowCount ?? 0,
            emptyTitle: GitPanelStrings.noChangesTitle,
            emptyMessage: GitPanelStrings.noChangesMessage
        )
        configureOverlay(
            conflictsOverlay,
            list: conflictList.scrollView,
            rowCount: status?.conflicts.rowCount ?? 0,
            emptyTitle: GitPanelStrings.noConflictsTitle,
            emptyMessage: GitPanelStrings.noConflictsMessage
        )
    }

    private func configureOverlay(
        _ overlay: GitStateOverlayView,
        list: NSScrollView,
        rowCount: Int,
        emptyTitle: String,
        emptyMessage: String
    ) {
        guard let status else {
            overlay.showLoading(GitPanelStrings.loadingStatus)
            overlay.isHidden = false
            list.isHidden = true
            return
        }
        switch status.availability {
        case .restricted:
            // 제한 모드에서는 이유와 신뢰 부여 동작을 함께 보여 줍니다(디자인 가이드 6.4).
            overlay.showMessage(
                symbol: GitPanelSymbols.restricted,
                title: "Restricted Mode",
                message: GitPanelStrings.restrictedBanner,
                action: GitPanelStrings.trustProject,
                target: self,
                selector: #selector(trustProject(_:))
            )
        case .gitMissing:
            overlay.showMessage(
                symbol: GitPanelSymbols.warning,
                title: "Git Not Found",
                message: GitPanelStrings.gitMissingBanner,
                action: GitPanelStrings.installHelp,
                target: self,
                selector: #selector(showInstallHelp(_:))
            )
        case .notRepository:
            overlay.showMessage(
                symbol: GitPanelSymbols.warning,
                title: "No Repository",
                message: GitPanelStrings.notRepositoryBanner,
                action: nil,
                target: nil,
                selector: nil
            )
        case .ready:
            guard rowCount == 0 else {
                overlay.isHidden = true
                list.isHidden = false
                return
            }
            overlay.showMessage(
                symbol: GitPanelSymbols.clean,
                title: emptyTitle,
                message: emptyMessage,
                action: GitPanelStrings.refresh,
                target: self,
                selector: #selector(refresh(_:))
            )
        }
        overlay.isHidden = false
        list.isHidden = true
    }

    private func updateBanners() {
        let showAutosave = status?.pendingAutosave == true
        if showAutosave {
            autosaveBanner.configure(message: GitPanelStrings.pendingAutosaveBanner, buttons: [saveNowButton, useSavedButton])
        }
        let semanticError = status?.semanticStatusError
        if let semanticError {
            semanticErrorBanner.configure(message: semanticError, buttons: [retryButton])
        }
        let showDependency = pendingPrompt != nil && !presentingDependencySheet
        if let prompt = pendingPrompt {
            includeRequiredButton.title = GitPanelStrings.includeRequired(prompt)
            dependencyBanner.configure(
                message: GitPanelStrings.dependency(prompt),
                buttons: [includeRequiredButton, declineRequiredButton]
            )
        }
        if let operation = activeOperation {
            progressBanner.configure(
                message: operation.message.isEmpty ? GitPanelStrings.loading : operation.message,
                buttons: operation.isCancellable ? [cancelOperationButton] : [],
                busy: true
            )
        }
        var showResult = false
        if let result = lastResult {
            switch result.outcome {
            case .failed(let message):
                resultBanner.setSymbol(GitPanelSymbols.error, label: "Error", tint: .systemRed)
                resultBanner.configure(message: message, buttons: [dismissResultButton])
                showResult = true
            case .succeeded(let message):
                resultBanner.setSymbol(GitPanelSymbols.clean, label: "Done", tint: .secondaryLabelColor)
                resultBanner.configure(message: message, buttons: [dismissResultButton])
                showResult = true
            case .cancelled:
                break
            }
        }
        bannerColumn.setItem(autosaveBanner, visible: showAutosave)
        bannerColumn.setItem(semanticErrorBanner, visible: semanticError != nil)
        bannerColumn.setItem(dependencyBanner, visible: showDependency)
        bannerColumn.setItem(progressBanner, visible: activeOperation != nil)
        bannerColumn.setItem(resultBanner, visible: showResult)
        rootColumn.setItem(bannerColumn, visible: bannerColumn.hasVisibleItems)
    }

    private func updateCommitArea() {
        let blockers = commitBlockers
        commitButton.isEnabled = blockers.isEmpty
        let reason: String
        if let first = blockers.first {
            reason = GitPanelStrings.commitBlocker(first)
        } else {
            let staged = status?.stagedChangeCount ?? 0
            let branch = status.map { GitPanelStrings.head($0.head) } ?? ""
            reason = "Commit \(GitPanelStrings.changeCount(staged)) to \(branch)."
        }
        commitReasonLabel.setStringIfChanged(reason)
        commitButton.setAccessibilityHelp(reason)
        changesColumn.setItem(identityRow, visible: blockers.contains(.identityMissing))
    }

    private func updatePushArea() {
        let blockers = pushBlockers
        let remotes = status?.remotes ?? []
        if remotePopup.itemTitles != remotes {
            remotePopup.removeAllItems()
            remotePopup.addItems(withTitles: remotes)
            if let upstream = status?.upstream?.remote, remotes.contains(upstream) {
                remotePopup.selectItem(withTitle: upstream)
            }
        }
        remotePopup.isHidden = remotes.isEmpty
        remotePopup.isEnabled = blockers.isEmpty
        pushButton.isEnabled = blockers.isEmpty
        let info: String
        if let first = blockers.first {
            info = GitPanelStrings.pushBlocker(first)
        } else if let upstream = status?.upstream {
            info = GitPanelStrings.pushInfo(ahead: upstream.ahead, remote: upstream.remote, branch: upstream.branch)
        } else {
            info = GitPanelStrings.pushNewBranch(remote: remotePopup.titleOfSelectedItem ?? remotes.first ?? "")
        }
        pushInfoLabel.setStringIfChanged(info)
        pushButton.setAccessibilityHelp(info)
    }

    private func updateApplyArea() {
        let blockers = applyBlockers
        applyButton.isEnabled = blockers.isEmpty
        let reason: String
        if let first = blockers.first {
            reason = GitPanelStrings.applyBlocker(first)
        } else {
            let count = status?.semanticConflictCount ?? 0
            reason = count == 1 ? "Apply 1 resolution and mark it resolved." : "Apply \(count) resolutions and mark them resolved."
        }
        applyReasonLabel.setStringIfChanged(reason)
        applyButton.setAccessibilityHelp(reason)
        conflictsColumn.setItem(continueHintLabel, visible: (status?.mergeState ?? .none) != .none)
    }

    /// 상태 표시 전환. 동작 줄이기에서는 즉시 바꿉니다(디자인 가이드 8.4, 8.5).
    private func animateLayout(_ changes: () -> Void) {
        let duration = displayOptions.revealDuration
        guard duration > 0, let window = view.window, window.isVisible else {
            changes()
            return
        }
        withoutActuallyEscaping(changes) { body in
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = duration
                context.allowsImplicitAnimation = true
                body()
                self.view.layoutSubtreeIfNeeded()
            }, completionHandler: nil)
        }
    }

    // MARK: 상세

    private func scheduleDetailUpdate() {
        guard !detailUpdateScheduled else { return }
        detailUpdateScheduled = true
        // 선택이 연속으로 바뀌면 다음 런 루프에서 한 번만 반영합니다(디자인 가이드 9장).
        DispatchQueue.main.async { [weak self] in
            self?.detailUpdateScheduled = false
            self?.updateDetail()
        }
    }

    func updateDetail() {
        let selection = changeList.tableView.selectedRowIndexes
        guard selection.count == 1, let row = selection.first else {
            currentDetail = nil
            requestedDetail = nil
            showDetailPlaceholder(selection.isEmpty ? GitPanelStrings.selectChange : GitPanelStrings.selectedCount(selection.count))
            return
        }
        let generation = changeList.generation
        if let value = changeList.displayedRow(at: row), value.level != .change {
            currentDetail = nil
            showGroupDetail(value)
            return
        }
        if let detail = currentDetail, detail.row == row {
            if detail.generation == generation {
                showDetail(detail)
                return
            }
            // 같은 행의 새 세대 상세가 올 때까지 이전 값을 유지합니다(깜빡임·재배치 방지).
        } else {
            showDetailPlaceholder(GitPanelStrings.loading)
        }
        if requestedDetail?.generation != generation || requestedDetail?.row != row {
            requestedDetail = (generation, row)
            dataSource.send(.loadChangeDetail(generation: generation, row: row))
        }
    }

    private func showDetailPlaceholder(_ text: String) {
        detailPlaceholder.setStringIfChanged(text)
        detailColumn.batchUpdates {
            detailColumn.setItem(detailPlaceholder, visible: true)
            for row in detailRows.values { detailColumn.setItem(row, visible: false) }
        }
    }

    private func showDetailFields(_ values: [DetailField: String]) {
        detailColumn.batchUpdates {
            detailColumn.setItem(detailPlaceholder, visible: values.isEmpty)
            for field in DetailField.allCases {
                guard let row = detailRows[field] else { continue }
                if let value = values[field], !value.isEmpty {
                    row.setValue(value)
                    detailColumn.setItem(row, visible: true)
                } else {
                    detailColumn.setItem(row, visible: false)
                }
            }
        }
    }

    private func showGroupDetail(_ row: GitChangeRow) {
        showDetailFields([
            .kind: row.level == .document ? "Document" : "Object",
            .object: row.title,
            .status: "\(GitPanelStrings.stageState(row.stageState)) · \(GitPanelStrings.changeCount(row.changeCount))",
        ])
    }

    private func showDetail(_ detail: GitChangeDetail) {
        var values: [DetailField: String] = [.object: detail.objectPath]
        if let kind = detail.kind { values[.kind] = GitPanelStrings.kind(kind) }
        values[.property] = detail.propertyPath
        values[.head] = detail.headValue
        values[.index] = detail.indexValue
        values[.working] = detail.workingValue
        if !detail.requiredChanges.isEmpty { values[.requires] = detail.requiredChanges.joined(separator: ", ") }
        if let row = changeList.displayedRow(at: detail.row) { values[.status] = GitPanelStrings.stageState(row.stageState) }
        showDetailFields(values)
    }

    func updateConflictDetail() {
        conflictsColumn.batchUpdates {
            conflictDetailColumn.batchUpdates { applyConflictDetail() }
        }
    }

    private func applyConflictDetail() {
        let selection = conflictList.tableView.selectedRowIndexes
        if selection.count == 1, let row = selection.first, conflictList.displayedRow(at: row) == nil {
            // 새 세대 페이지를 기다리는 중입니다. 자리 표시로 바꾸지 않고 현재 표시를 유지합니다.
            return
        }
        guard selection.count == 1, let row = selection.first, let value = conflictList.displayedRow(at: row) else {
            conflictDetailPlaceholder.setStringIfChanged(selection.count > 1
                ? GitPanelStrings.selectedCount(selection.count)
                : GitPanelStrings.selectConflict)
            conflictDetailColumn.setItem(conflictDetailPlaceholder, visible: true)
            conflictRows.values.forEach { conflictDetailColumn.setItem($0, visible: false) }
            conflictsColumn.setItem(revealRow, visible: false)
            return
        }
        let values: [ConflictField: String?] = [
            .kind: GitPanelStrings.conflictKind(value.kind),
            .path: value.path,
            .base: value.baseSummary,
            .ours: value.oursSummary,
            .theirs: value.theirsSummary,
            .choice: value.kind == .raw ? GitPanelStrings.rawConflictHint : (value.choice.map(GitPanelStrings.choice) ?? "Not chosen"),
        ]
        conflictDetailColumn.setItem(conflictDetailPlaceholder, visible: false)
        for field in ConflictField.allCases {
            guard let rowView = conflictRows[field] else { continue }
            if let text = values[field] ?? nil, !text.isEmpty {
                rowView.setValue(text)
                conflictDetailColumn.setItem(rowView, visible: true)
            } else {
                conflictDetailColumn.setItem(rowView, visible: false)
            }
        }
        conflictsColumn.setItem(revealRow, visible: value.kind != .raw)
    }

    // MARK: 동작

    func setMode(_ newMode: Mode) {
        mode = newMode
        modeControl.selectedSegment = newMode.rawValue
        rootColumn.setItem(changesColumn, visible: newMode == .changes)
        rootColumn.setItem(conflictsColumn, visible: newMode == .conflicts)
        rootColumn.flexibleView = newMode == .changes ? changesColumn : conflictsColumn
        configureKeyLoop()
    }

    @objc private func modeChanged(_ sender: NSSegmentedControl) {
        setMode(Mode(rawValue: sender.selectedSegment) ?? .changes)
    }

    @objc func refresh(_ sender: Any?) {
        dataSource.send(.refreshStatus)
    }

    private func setStaged(rows: IndexSet, staged: Bool) {
        guard status?.availability == .ready, !isBusy, pendingPrompt == nil else { return }
        dataSource.send(.setStaged(generation: changeList.generation, rows: rows, staged: staged))
    }

    @objc func commit(_ sender: Any?) {
        guard commitBlockers.isEmpty else {
            NSSound.beep()
            return
        }
        dataSource.send(.commit(message: commitTextView.string))
    }

    /// 커밋 성공 후 메시지를 비웁니다. 텍스트 시스템 경로로 지워 Undo로 되살릴 수 있습니다.
    private func clearCommitMessage() {
        let length = (commitTextView.string as NSString).length
        guard length > 0 else { return }
        let range = NSRange(location: 0, length: length)
        if commitTextView.shouldChangeText(in: range, replacementString: "") {
            commitTextView.textStorage?.replaceCharacters(in: range, with: "")
            commitTextView.didChangeText()
        }
    }

    @objc func push(_ sender: Any?) {
        guard pushBlockers.isEmpty, let remote = remotePopup.titleOfSelectedItem ?? status?.remotes.first else {
            NSSound.beep()
            return
        }
        dataSource.send(.push(remote: remote))
    }

    @objc func applyResolutions(_ sender: Any?) {
        guard applyBlockers.isEmpty else {
            NSSound.beep()
            return
        }
        dataSource.send(.applyResolutions)
    }

    @objc private func revealConflict(_ sender: Any?) {
        let row = conflictList.tableView.selectedRow
        guard row >= 0 else { return }
        reveal(row: row)
    }

    private func reveal(row: Int) {
        dataSource.send(.revealConflict(generation: conflictList.generation, row: row))
    }

    @objc private func saveBeforeStatus(_ sender: Any?) {
        dataSource.send(.resolvePendingAutosave(saveFirst: true))
    }

    @objc private func useSavedVersion(_ sender: Any?) {
        dataSource.send(.resolvePendingAutosave(saveFirst: false))
    }

    @objc private func includeRequired(_ sender: Any?) {
        answerDependencyPrompt(include: true)
    }

    @objc private func declineRequired(_ sender: Any?) {
        answerDependencyPrompt(include: false)
    }

    @objc private func cancelGitOperation(_ sender: Any?) {
        guard let operation = activeOperation, operation.isCancellable else { return }
        dataSource.send(.cancelOperation(id: operation.id))
    }

    @objc private func dismissResult(_ sender: Any?) {
        lastResult = nil
        updateStatusUI()
    }

    @objc private func trustProject(_ sender: Any?) {
        host?.gitPanelRequestsProjectTrust()
    }

    @objc private func setUpIdentity(_ sender: Any?) {
        host?.gitPanelRequestsIdentitySetup()
    }

    @objc private func showInstallHelp(_ sender: Any?) {
        host?.gitPanelRequestsGitInstallHelp()
    }

    /// 필수 동반 변경 확인. 창이 보이면 sheet, 아니면 배너로 묻습니다. 자동 포함하지 않습니다(XIV.5.3).
    private func presentDependencyPrompt() {
        guard let prompt = pendingPrompt, let window = view.window, window.isVisible, window.attachedSheet == nil else {
            return
        }
        presentingDependencySheet = true
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = prompt.isStaging ? "Stage Related Changes Too?" : "Unstage Related Changes Too?"
        alert.informativeText = GitPanelStrings.dependency(prompt)
        alert.addButton(withTitle: GitPanelStrings.includeRequired(prompt))
        alert.addButton(withTitle: GitPanelStrings.cancel)
        alert.beginSheetModal(for: window) { [weak self] response in
            MainActor.assumeIsolated {
                self?.presentingDependencySheet = false
                self?.answerDependencyPrompt(include: response == .alertFirstButtonReturn)
            }
        }
    }

    func answerDependencyPrompt(include: Bool) {
        guard let prompt = pendingPrompt else { return }
        pendingPrompt = nil
        dataSource.send(.answerDependencyPrompt(requestID: prompt.requestID, includeRequired: include))
        updateStatusUI()
    }

    // MARK: NSTextViewDelegate

    func textDidChange(_ notification: Notification) {
        updateCommitArea()
    }

    /// Tab·Shift-Tab은 포커스를 옮기고, 탭 문자는 Option-Tab으로 넣습니다(AppKit 기본 규칙).
    func textView(_ textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.insertTab(_:)):
            textView.window?.selectNextKeyView(nil)
            return true
        case #selector(NSResponder.insertBacktab(_:)):
            textView.window?.selectPreviousKeyView(nil)
            return true
        default:
            return false
        }
    }
}
