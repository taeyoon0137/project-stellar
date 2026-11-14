import AppKit

/// Git 패널 루트 뷰. 패널 단축키(⌘Return 커밋)를 패널 안에 포커스가 있을 때만 처리합니다.
///
/// 버튼 key equivalent는 창 전체에서 발동하므로(캔버스·다른 패널에서 ⌘Return) 쓰지 않습니다(가이드 8.1, 8.2).
final class GitPanelRootView: NSView {
    /// 처리했으면 true.
    var commitShortcutHandler: (() -> Bool)?

    /// first responder가 이 패널 안에 있는지. field editor는 편집 중인 필드의 하위로 붙으므로 함께 판정됩니다.
    var containsFirstResponder: Bool {
        guard let responder = window?.firstResponder as? NSView else { return false }
        return responder === self || responder.isDescendant(of: self)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.type == .keyDown,
           event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting([.numericPad, .function]) == .command,
           ["\r", "\u{3}"].contains(event.charactersIgnoringModifiers ?? ""),
           containsFirstResponder,
           commitShortcutHandler?() == true {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

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
            case .status: return GitPanelStrings.statusLabel
            }
        }
    }

    /// conflict 상세 표시 항목.
    enum ConflictField: CaseIterable {
        case kind, path, base, ours, theirs, choice

        var name: String {
            switch self {
            case .kind: return GitPanelStrings.kindLabel
            case .path: return GitPanelStrings.pathLabel
            case .base: return GitPanelStrings.base
            case .ours: return GitPanelStrings.ours
            case .theirs: return GitPanelStrings.theirs
            case .choice: return GitPanelStrings.choiceLabel
            }
        }
    }

    /// 메인 스레드 점유 진단(검사용). 한 런 루프 차례의 점유를 종류별로 잽니다(아키텍처 §6.3).
    struct Diagnostics {
        var drains = 0
        var events = 0
        var rejections = 0
        /// 이벤트 드레인 차례 한 번의 최대 점유(같은 차례에서 한 UI 반영 포함).
        var maxDrainSeconds: TimeInterval = 0
        /// 상태·conflict 상세 UI 반영 한 번의 최대 시간(배너 전환 애니메이션 배치 포함).
        var maxUIUpdateSeconds: TimeInterval = 0
        /// 변경 상세 반영 차례 한 번의 최대 점유.
        var maxDetailSeconds: TimeInterval = 0
        /// 이벤트 처리가 길어 UI 반영을 다음 차례로 넘긴 횟수.
        var deferredUIUpdates = 0
        /// 화면에 보이는 창에서 애니메이션 배치로 UI를 반영한 횟수.
        var animatedUIUpdates = 0

        /// 종류와 무관한 한 차례 최대 점유.
        var maxTurnSeconds: TimeInterval { max(maxDrainSeconds, maxUIUpdateSeconds, maxDetailSeconds) }
    }

    /// 미뤄 둔 UI 반영.
    private struct PendingUI {
        var status = false
        var conflictDetail = false
        var isEmpty: Bool { !status && !conflictDetail }
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
    private var uiUpdateScheduled = false
    private var pendingUI = PendingUI()
    private var requestedDetail: (generation: UInt64, node: GitChangeNodeID)?
    private var presentingDependencySheet = false
    private var displayOptionsToken: GitNotificationToken?
    private var secondaryLabels: [NSTextField] = []
    /// 보조 색으로 칠한 상태 아이콘(대비 높이기에서 label 색으로 올림).
    private var secondaryIcons: [NSImageView] = []
    /// 커밋 명령을 보냈고 결과(또는 거부)를 기다리는 중. 이 동안 메시지를 편집할 수 없고 다시 커밋할 수 없습니다.
    private(set) var isCommitInFlight = false

    /// 상태 알림을 받는 쪽(검사용). nil이면 VoiceOver 알림(announcementRequested)을 보냅니다.
    var announcementHandler: ((String, NSAccessibilityPriorityLevel) -> Void)?

    /// 미뤄 둔 이벤트·UI·상세 반영이 남았는지(검사용).
    var hasPendingWork: Bool { drainScheduled || uiUpdateScheduled || detailUpdateScheduled || changeList.isRestoring }

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
    let branchIcon = GitControls.symbolView(GitPanelSymbols.branch, label: GitPanelStrings.branchSymbol)
    let branchLabel = GitControls.label(GitPanelStrings.loading, style: .headline)
    let mergeStateLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let aheadIcon = GitControls.symbolView(GitPanelSymbols.ahead, label: GitPanelStrings.aheadSymbol, style: .callout)
    let aheadLabel = GitControls.label(style: .callout, color: .secondaryLabelColor)
    let behindIcon = GitControls.symbolView(GitPanelSymbols.behind, label: GitPanelStrings.behindSymbol, style: .callout)
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
    let autosaveBanner = GitBannerView(symbol: GitPanelSymbols.save, symbolLabel: GitPanelStrings.unsavedEditsSymbol, tint: .systemOrange)
    let semanticErrorBanner = GitBannerView(symbol: GitPanelSymbols.warning, symbolLabel: GitPanelStrings.statusErrorSymbol, tint: .systemOrange)
    let dependencyBanner = GitBannerView(symbol: GitPanelSymbols.dependency, symbolLabel: GitPanelStrings.relatedChangesSymbol, tint: .systemOrange)
    let progressBanner = GitBannerView(symbol: GitPanelSymbols.info, symbolLabel: GitPanelStrings.inProgressSymbol, tint: .secondaryLabelColor)
    let resultBanner = GitBannerView(symbol: GitPanelSymbols.info, symbolLabel: GitPanelStrings.resultSymbol, tint: .secondaryLabelColor)
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
        let root = GitPanelRootView()
        root.commitShortcutHandler = { [weak self] in self?.handleCommitShortcut() ?? false }
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
        // 여러 줄 입력에서 Return은 줄바꿈이므로 Command-Return을 쓰되, 버튼 키 등가가 아니라 패널 루트 뷰가
        // 패널 안 포커스일 때만 처리합니다(`GitPanelRootView`).
        commitButton.tintProminence = .primary
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
        secondaryIcons = [branchIcon, aheadIcon, behindIcon, changesOverlay.iconView, conflictsOverlay.iconView]

        changeList.requestPage = { [weak self] generation, parent, children in
            self?.dataSource.send(.loadChangePage(generation: generation, parent: parent, children: children))
        }
        changeList.setStaged = { [weak self] targets, staged in self?.setStaged(targets: targets, staged: staged) }
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
        install(list: changeList.scrollView, overlay: changesOverlay, in: changeListArea, rowHeight: changeList.outlineView.rowHeight)
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

    /// 대비 높이기에서는 보조 문구와 보조 색 상태 아이콘을 label 색으로 올립니다. 시스템 색(주황·빨강) 아이콘은
    /// 시스템이 대비 높이기 변형을 그리므로 그대로 둡니다(가이드 8.4).
    private func applyDisplayOptions() {
        let color = displayOptions.secondaryTextColor
        secondaryLabels.forEach { $0.textColor = color }
        secondaryIcons.forEach { $0.contentTintColor = color }
        for banner in [autosaveBanner, semanticErrorBanner, dependencyBanner, progressBanner, resultBanner] {
            banner.increaseContrast = displayOptions.increaseContrast
        }
        changeList.secondaryColor = color
        conflictList.secondaryColor = color
    }

    private func displayOptionsChanged() {
        apply(displayOptions: GitDisplayOptions.current())
    }

    /// 시스템 접근성 표시 옵션을 반영합니다. 실행 중 바뀌면 알림으로 불립니다(검사에서도 씁니다).
    func apply(displayOptions options: GitDisplayOptions) {
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

    /// 이벤트 큐를 메인 스레드 예산 안에서 비웁니다(아키텍처 §6.3 단일 점유 2ms).
    ///
    /// - 이벤트 처리는 `eventBudget`을 넘으면 멈추고 나머지를 다음 런 루프로 넘깁니다.
    /// - UI 반영(상태·배너·목록 세대·conflict 상세)은 이벤트 처리가 `inlineUIThreshold` 안에 끝났을 때만 같은
    ///   차례에서 하고, 아니면 다음 차례로 넘겨 한 차례에 둘을 겹치지 않습니다. 변경 상세는 늘 다음 차례입니다.
    func drainEvents() {
        drainScheduled = false
        let start = ProcessInfo.processInfo.systemUptime
        var needsDetail = false
        while ProcessInfo.processInfo.systemUptime - start <= GitPanelMetrics.eventBudget {
            guard let event = dataSource.pollEvent() else { break }
            diagnostics.events += 1
            switch event {
            case .status(let snapshot):
                let previous = status
                status = snapshot
                pendingUI.status = true
                if previous?.changes != snapshot.changes { needsDetail = true }
                if previous?.conflicts != snapshot.conflicts { pendingUI.conflictDetail = true }
                announceStatusChanges(from: previous, to: snapshot)
            case .changePage(let page):
                changeList.receive(page)
                if let selected = changeList.singleSelectedItem, changeList.page(page, contains: selected) {
                    needsDetail = true
                }
            case .changeDetail(let detail):
                if detail.generation == changeList.generation, let selected = changeList.singleSelectedItem,
                   selected.nodeID == detail.node {
                    currentDetail = detail
                    showDetail(detail)
                }
            case .commandRejected(let rejection):
                handleRejection(rejection)
            case .conflictPage(let page):
                conflictList.receive(page)
                pendingUI.conflictDetail = true
            case .dependencyPrompt(let prompt):
                pendingPrompt = prompt
                presentDependencyPrompt()
                if !presentingDependencySheet { announce(GitPanelStrings.dependency(prompt), priority: .high) }
                pendingUI.status = true
            case .operationProgress(let progress):
                if activeOperation?.id != progress.id, GitPanelRules.announcesProgress(progress.kind), !progress.message.isEmpty {
                    announce(progress.message, priority: .medium)
                }
                activeOperation = progress
                pendingUI.status = true
            case .operationFinished(let result):
                if activeOperation?.id == result.id { activeOperation = nil }
                handleFinished(result)
                pendingUI.status = true
            }
        }
        // 예산을 넘겨 멈췄을 수 있으므로 다음 차례에 다시 비웁니다(빈 큐면 아무 일도 하지 않음).
        if ProcessInfo.processInfo.systemUptime - start > GitPanelMetrics.eventBudget { scheduleDrain() }
        if needsDetail { scheduleDetailUpdate() }
        if !pendingUI.isEmpty {
            if ProcessInfo.processInfo.systemUptime - start <= GitPanelMetrics.inlineUIThreshold {
                applyPendingUI()
            } else {
                diagnostics.deferredUIUpdates += 1
                schedulePendingUI()
            }
        }
        diagnostics.drains += 1
        diagnostics.maxDrainSeconds = max(diagnostics.maxDrainSeconds, ProcessInfo.processInfo.systemUptime - start)
    }

    private func schedulePendingUI() {
        guard !uiUpdateScheduled else { return }
        uiUpdateScheduled = true
        DispatchQueue.main.async { [weak self] in self?.applyPendingUI() }
    }

    private func applyPendingUI() {
        uiUpdateScheduled = false
        let pending = pendingUI
        pendingUI = PendingUI()
        guard !pending.isEmpty else { return }
        let start = ProcessInfo.processInfo.systemUptime
        if pending.status { updateStatusUI() }
        if pending.conflictDetail { updateConflictDetail() }
        diagnostics.maxUIUpdateSeconds = max(diagnostics.maxUIUpdateSeconds, ProcessInfo.processInfo.systemUptime - start)
    }

    // MARK: 상태 알림

    /// 상태 변화를 VoiceOver에 알립니다. 배너 문구와 같은 내용을 보냅니다(가이드 8.3).
    private func announce(_ message: String, priority: NSAccessibilityPriorityLevel) {
        guard !message.isEmpty else { return }
        if let announcementHandler {
            announcementHandler(message, priority)
            return
        }
        let element: Any = view.window ?? NSApp.mainWindow ?? NSApp as Any
        NSAccessibility.post(
            element: element,
            notification: .announcementRequested,
            userInfo: [.announcement: message, .priority: priority.rawValue]
        )
    }

    /// 새로 나타난 배너(semantic status 오류, 저장 대기 편집)를 알립니다.
    private func announceStatusChanges(from previous: GitStatusSnapshot?, to snapshot: GitStatusSnapshot) {
        if let error = snapshot.semanticStatusError, error != previous?.semanticStatusError {
            announce(error, priority: .high)
        }
        if snapshot.pendingAutosave, previous?.pendingAutosave != true {
            announce(GitPanelStrings.pendingAutosaveBanner, priority: .medium)
        }
    }

    /// 코어가 버린 명령: 미리 보여 준 값을 캐시(코어) 값으로 되돌리고, 요청 중 표시를 지워 다시 요청할 수 있게 합니다.
    private func handleRejection(_ rejection: GitCommandRejection) {
        diagnostics.rejections += 1
        switch rejection.command {
        case .loadChangePage(let generation, let parent, let children):
            changeList.cancelRequest(generation: generation, parent: parent, children: children)
        case .loadChangeDetail(let generation, let node):
            if requestedDetail?.generation == generation, requestedDetail?.node == node { requestedDetail = nil }
        case .setStaged:
            changeList.refreshVisibleRows()
        case .loadConflictPage(let generation, let rows):
            conflictList.cancelRequest(generation: generation, rows: rows)
        case .chooseResolution:
            conflictList.refreshVisibleRows()
        case .commit:
            setCommitInFlight(false)
        default:
            break
        }
    }

    private func handleFinished(_ result: GitOperationResult) {
        if result.kind == .commit { setCommitInFlight(false) }
        switch (result.kind, result.outcome) {
        case (.commit, .succeeded(let message)):
            // 보내는 동안 메시지를 편집할 수 없었으므로 지우는 문장은 보낸 문장과 같습니다.
            clearCommitMessage()
            lastResult = result
            announce(message, priority: .medium)
        case (.push, .succeeded(let message)), (.applyResolutions, .succeeded(let message)):
            lastResult = result
            announce(message, priority: .medium)
        case (_, .failed(let message)):
            lastResult = result
            announce(message, priority: .high)
        default:
            // Stage·Unstage·새로고침 성공과 취소는 목록 변화로 충분하므로 배너를 띄우지 않습니다.
            if case .failed = lastResult?.outcome {} else { lastResult = nil }
        }
    }

    /// 커밋을 보낸 뒤 결과가 올 때까지 메시지를 잠급니다. 그 사이에 친 글자가 성공 후 함께 지워지지 않게 합니다.
    private func setCommitInFlight(_ inFlight: Bool) {
        guard isCommitInFlight != inFlight else { return }
        isCommitInFlight = inFlight
        commitTextView.isEditable = !inFlight
        updateCommitArea()
    }

    // MARK: 상태 반영

    var isBusy: Bool { activeOperation != nil }

    var commitBlockers: [GitCommitBlocker] {
        GitPanelRules.commitBlockers(
            status: status, message: commitTextView.string, operationInProgress: isBusy || isCommitInFlight)
    }

    var pushBlockers: [GitPushBlocker] {
        GitPanelRules.pushBlockers(status: status, operationInProgress: isBusy)
    }

    var applyBlockers: [GitApplyBlocker] {
        GitPanelRules.applyBlockers(status: status, operationInProgress: isBusy)
    }

    private var banners: [GitBannerView] {
        [autosaveBanner, semanticErrorBanner, dependencyBanner, progressBanner, resultBanner]
    }

    private func updateStatusUI() {
        guard isViewLoaded else { return }
        let shownBefore = Set(banners.filter { !$0.isHiddenOrHasHiddenAncestor }.map(ObjectIdentifier.init))
        // 표시 변경을 묶어 각 구획의 제약을 한 번씩만 다시 만듭니다. 배치는 즉시 바꾸고, 새로 나타난 배너만 짧게
        // 페이드합니다(디자인 가이드 8.5).
        rootColumn.batchUpdates {
            changesColumn.batchUpdates {
                conflictsColumn.batchUpdates {
                    bannerColumn.batchUpdates { applyStatusUI() }
                }
            }
        }
        fadeIn(banners.filter { !$0.isHiddenOrHasHiddenAncestor && !shownBefore.contains(ObjectIdentifier($0)) })
    }

    /// 새로 나타난 배너를 페이드로 보여 줍니다. 동작 줄이기에서는 즉시 보입니다(디자인 가이드 8.4, 8.5).
    ///
    /// 배치 변경 자체를 애니메이션 묶음 안에서 강제 배치하면 패널 전체의 frame 변경이 애니메이션이 되어, 화면에
    /// 보이는 창에서 한 번에 약 20ms를 점유했습니다(2026-11-14 Debug 측정, 즉시 배치는 1ms 미만). 그래서 배치는
    /// 즉시 바꾸고 배너의 불투명도만 움직입니다(가이드 1.2 원칙 6).
    private func fadeIn(_ appeared: [GitBannerView]) {
        let duration = displayOptions.revealDuration
        guard !appeared.isEmpty, duration > 0, let window = view.window, window.isVisible else {
            appeared.forEach { $0.alphaValue = 1 }
            return
        }
        diagnostics.animatedUIUpdates += 1
        appeared.forEach { $0.alphaValue = 0 }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = duration
            appeared.forEach { $0.animator().alphaValue = 1 }
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
            // 두 숫자를 각각 읽게 합니다(같은 문장을 두 번 읽지 않음). 도움말 태그는 전체 문장입니다.
            aheadLabel.setAccessibilityLabel(GitPanelStrings.aheadCount(upstream.ahead))
            behindLabel.setAccessibilityLabel(GitPanelStrings.behindCount(upstream.behind))
            let text = GitPanelStrings.aheadBehind(ahead: upstream.ahead, behind: upstream.behind)
            for view in [aheadIcon, aheadLabel, behindIcon, behindLabel] as [NSView] where view.toolTip != text {
                view.toolTip = text
            }
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
            rowCount: status?.changes.documentCount ?? 0,
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
                title: GitPanelStrings.restrictedTitle,
                message: GitPanelStrings.restrictedBanner,
                action: GitPanelStrings.trustProject,
                target: self,
                selector: #selector(trustProject(_:))
            )
        case .gitMissing:
            overlay.showMessage(
                symbol: GitPanelSymbols.warning,
                title: GitPanelStrings.gitMissingTitle,
                message: GitPanelStrings.gitMissingBanner,
                action: GitPanelStrings.installHelp,
                target: self,
                selector: #selector(showInstallHelp(_:))
            )
        case .notRepository:
            overlay.showMessage(
                symbol: GitPanelSymbols.warning,
                title: GitPanelStrings.noRepositoryTitle,
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
                resultBanner.setSymbol(GitPanelSymbols.error, label: GitPanelStrings.errorSymbol, tint: .systemRed)
                resultBanner.configure(message: message, buttons: [dismissResultButton])
                showResult = true
            case .succeeded(let message):
                resultBanner.setSymbol(GitPanelSymbols.clean, label: GitPanelStrings.doneSymbol, tint: .secondaryLabelColor)
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
            reason = GitPanelStrings.commitReady(changes: staged, branch: branch)
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
            reason = GitPanelStrings.applyReady(count)
        }
        applyReasonLabel.setStringIfChanged(reason)
        applyButton.setAccessibilityHelp(reason)
        conflictsColumn.setItem(continueHintLabel, visible: (status?.mergeState ?? .none) != .none)
    }

    // MARK: 상세

    private func scheduleDetailUpdate() {
        guard !detailUpdateScheduled else { return }
        detailUpdateScheduled = true
        // 선택이 연속으로 바뀌면 다음 런 루프에서 한 번만 반영합니다(디자인 가이드 9장).
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.detailUpdateScheduled = false
            let start = ProcessInfo.processInfo.systemUptime
            self.updateDetail()
            self.diagnostics.maxDetailSeconds = max(
                self.diagnostics.maxDetailSeconds, ProcessInfo.processInfo.systemUptime - start)
        }
    }

    func updateDetail() {
        let selection = changeList.outlineView.selectedRowIndexes
        guard selection.count == 1, let item = changeList.singleSelectedItem else {
            currentDetail = nil
            requestedDetail = nil
            showDetailPlaceholder(selection.isEmpty ? GitPanelStrings.selectChange : GitPanelStrings.selectedCount(selection.count))
            return
        }
        let generation = changeList.generation
        if let value = changeList.displayedRow(of: item), value.level != .change {
            currentDetail = nil
            showGroupDetail(value)
            return
        }
        guard let node = item.nodeID else {
            // 행 페이지가 오면 다시 불립니다.
            showDetailPlaceholder(GitPanelStrings.loading)
            return
        }
        if let detail = currentDetail, detail.node == node {
            if detail.generation == generation {
                showDetail(detail)
                return
            }
            // 같은 노드의 새 세대 상세가 올 때까지 이전 값을 유지합니다(깜빡임·재배치 방지).
        } else {
            showDetailPlaceholder(GitPanelStrings.loading)
        }
        if requestedDetail?.generation != generation || requestedDetail?.node != node {
            requestedDetail = (generation, node)
            dataSource.send(.loadChangeDetail(generation: generation, node: node))
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
            .kind: row.level == .document ? GitPanelStrings.documentLevel : GitPanelStrings.objectLevel,
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
        if let item = changeList.item(for: detail.node), let row = changeList.displayedRow(of: item) {
            values[.status] = GitPanelStrings.stageState(row.stageState)
        }
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
            .choice: value.kind == .raw ? GitPanelStrings.rawConflictHint : (value.choice.map(GitPanelStrings.choice) ?? GitPanelStrings.notChosen),
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
    }

    @objc private func modeChanged(_ sender: NSSegmentedControl) {
        setMode(Mode(rawValue: sender.selectedSegment) ?? .changes)
    }

    @objc func refresh(_ sender: Any?) {
        dataSource.send(.refreshStatus)
    }

    private func setStaged(targets: [GitChangeRange], staged: Bool) {
        guard status?.availability == .ready, !isBusy, pendingPrompt == nil, !targets.isEmpty else { return }
        dataSource.send(.setStaged(generation: changeList.generation, targets: targets, staged: staged))
    }

    /// ⌘Return(패널 안 포커스일 때만): Changes 화면에서 커밋합니다. 막혀 있으면 `commit(_:)`이 경고음을 내며,
    /// 이 경우에도 처리한 것으로 보아 같은 창의 다른 패널로 넘기지 않습니다.
    private func handleCommitShortcut() -> Bool {
        guard mode == .changes else { return false }
        commit(nil)
        return true
    }

    @objc func commit(_ sender: Any?) {
        guard commitBlockers.isEmpty else {
            NSSound.beep()
            return
        }
        // 결과가 올 때까지 메시지를 잠가 보낸 문장과 성공 후 지우는 문장이 같게 합니다.
        setCommitInFlight(true)
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
        alert.messageText = prompt.isStaging ? GitPanelStrings.stageRelatedTitle : GitPanelStrings.unstageRelatedTitle
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
