import AppKit

/// 대화 기록 표시 방식. 채팅 보기가 기본이고 원문 텍스트 보기를 대안으로 둡니다(PO 2026-11-14).
enum AgentConversationDisplayMode: Int, CaseIterable {
    case chat = 0
    case rawText = 1
}

// MARK: - 헤더

/// 패널 헤더: 사용 중인 CLI, 연결 상태, CLI가 보고한 모델, 보기 전환, 모드, 더 보기 메뉴.
///
/// 표준 `NSPopUpButton`·`NSSegmentedControl`만 씁니다(가이드 5.1). 비용·사용량은 표시하지 않습니다.
final class AgentPanelHeaderView: NSView {
    let cliPopUp = NSPopUpButton(frame: .zero, pullsDown: false)
    let moreButton = NSPopUpButton(frame: .zero, pullsDown: true)
    let statusSymbolView = AgentPanelViews.symbolView(AgentPanelSymbol.noSession, textStyle: .callout)
    let statusLabel = AgentPanelViews.label(font: AgentPanelViews.font(.callout), color: .secondaryLabelColor)
    let modelLabel = AgentPanelViews.label(font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
    let displayModeControl = NSSegmentedControl(
        labels: [AgentPanelText.chatView, AgentPanelText.rawTextView],
        trackingMode: .selectOne, target: nil, action: nil)
    let modePopUp = NSPopUpButton(frame: .zero, pullsDown: false)
    private var lastCLIOptions: [AgentCLIOption] = []

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false

        cliPopUp.translatesAutoresizingMaskIntoConstraints = false
        cliPopUp.setAccessibilityLabel(AgentPanelText.cliPopUpLabel)
        cliPopUp.toolTip = AgentPanelText.cliPopUpLabel
        cliPopUp.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        cliPopUp.setContentHuggingPriority(.defaultLow, for: .horizontal)

        moreButton.translatesAutoresizingMaskIntoConstraints = false
        moreButton.imagePosition = .imageOnly
        moreButton.setAccessibilityLabel(AgentPanelText.moreActions)
        moreButton.toolTip = AgentPanelText.moreActions
        moreButton.setContentHuggingPriority(.required, for: .horizontal)
        moreButton.setContentCompressionResistancePriority(.required, for: .horizontal)

        statusLabel.setAccessibilityLabel(nil)
        modelLabel.toolTip = AgentPanelText.modelToolTip
        modelLabel.setContentHuggingPriority(.required, for: .horizontal)

        displayModeControl.translatesAutoresizingMaskIntoConstraints = false
        displayModeControl.controlSize = .small
        displayModeControl.selectedSegment = AgentConversationDisplayMode.chat.rawValue
        displayModeControl.setAccessibilityLabel(AgentPanelText.conversationViewLabel)
        displayModeControl.setContentHuggingPriority(.required, for: .horizontal)
        displayModeControl.setContentCompressionResistancePriority(.required, for: .horizontal)

        modePopUp.translatesAutoresizingMaskIntoConstraints = false
        modePopUp.controlSize = .small
        modePopUp.font = NSFont.systemFont(ofSize: NSFont.systemFontSize(for: .small))
        modePopUp.setAccessibilityLabel(AgentPanelText.modePopUpLabel)
        modePopUp.toolTip = AgentPanelText.modePopUpLabel
        modePopUp.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for mode in AgentPermissionMode.allCases {
            modePopUp.addItem(withTitle: AgentPanelText.modeTitle(mode))
        }

        let cliRow = Self.row([cliPopUp, moreButton])
        let statusRow = Self.row([statusSymbolView, statusLabel, modelLabel])
        statusRow.spacing = AgentPanelMetrics.itemSpacing / 2
        statusRow.setCustomSpacing(AgentPanelMetrics.itemSpacing, after: statusLabel)
        let controlRow = NSStackView()
        controlRow.orientation = .horizontal
        controlRow.spacing = AgentPanelMetrics.itemSpacing
        controlRow.translatesAutoresizingMaskIntoConstraints = false
        controlRow.setViews([displayModeControl], in: .leading)
        controlRow.setViews([modePopUp], in: .trailing)

        let stack = AgentPanelViews.verticalStack(spacing: AgentPanelMetrics.itemSpacing)
        AgentPanelViews.setArrangedSubviews([cliRow, statusRow, controlRow], in: stack)
        addSubview(stack)
        let inset = AgentPanelMetrics.edgeInset
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: inset),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -AgentPanelMetrics.itemSpacing),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentPanelHeaderView는 코드로만 생성합니다")
    }

    private static func row(_ views: [NSView]) -> NSStackView {
        let row = NSStackView(views: views)
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = AgentPanelMetrics.itemSpacing
        row.distribution = .fill  // 가장 낮은 hugging 항목(본문)이 남는 폭을 채움
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }

    /// 헤더 값을 갱신합니다. 바뀐 항목만 다시 만듭니다.
    func update(header: AgentSessionHeader, displayMode: AgentConversationDisplayMode) {
        if header.installedCLIs != lastCLIOptions || cliPopUp.numberOfItems == 0 {
            lastCLIOptions = header.installedCLIs
            cliPopUp.removeAllItems()
            if header.installedCLIs.isEmpty {
                cliPopUp.addItem(withTitle: AgentPanelText.noCLIItem)
            } else {
                for option in header.installedCLIs {
                    cliPopUp.addItem(withTitle: AgentPanelText.cliItemTitle(option))
                    cliPopUp.lastItem?.representedObject = NSNumber(value: option.id.rawValue)
                }
            }
        }
        if let selected = header.selectedCLIID,
            let index = header.installedCLIs.firstIndex(where: { $0.id == selected })
        {
            cliPopUp.selectItem(at: index)
        }
        // 세션 중에는 CLI를 바꾸지 않습니다(모델이 세션 중에 바뀌지 않게, ADR-0005 결정 4.1).
        cliPopUp.isEnabled = !header.installedCLIs.isEmpty && !header.isSessionActive

        let status = AgentPanelText.connectionTitle(header)
        statusLabel.stringValue = status
        statusLabel.toolTip = status
        let (symbolName, tint) = Self.statusSymbol(header)
        statusSymbolView.image = AgentPanelViews.symbol(symbolName, description: nil)
        statusSymbolView.contentTintColor = tint
        // 모델 이름은 세션이 시작된 뒤 CLI가 보고한 값만 표시합니다.
        if let model = header.reportedModelName, header.selectedCLI?.availability == .ready,
            header.connection != .noSession
        {
            modelLabel.stringValue = AgentPanelText.modelLabel(model)
            modelLabel.isHidden = false
        } else {
            modelLabel.stringValue = ""
            modelLabel.isHidden = true
        }
        // 상태 줄 전체를 VoiceOver 요소 하나로 읽습니다.
        statusLabel.setAccessibilityLabel(
            modelLabel.isHidden ? status : "\(status), \(modelLabel.stringValue)")
        modelLabel.setAccessibilityElement(false)

        if let index = AgentPermissionMode.allCases.firstIndex(of: header.mode) {
            modePopUp.selectItem(at: index)
        }
        displayModeControl.selectedSegment = displayMode.rawValue
    }

    static func statusSymbol(_ header: AgentSessionHeader) -> (String, NSColor) {
        guard let cli = header.selectedCLI else { return (AgentPanelSymbol.noCLI, .secondaryLabelColor) }
        switch cli.availability {
        case .unsupportedVersion: return (AgentPanelSymbol.unsupportedVersion, .systemOrange)
        case .authenticationRequired: return (AgentPanelSymbol.authenticationRequired, .systemOrange)
        case .ready: break
        }
        switch header.connection {
        case .noSession: return (AgentPanelSymbol.noSession, .secondaryLabelColor)
        case .starting, .reconnecting: return (AgentPanelSymbol.connecting, .secondaryLabelColor)
        case .connected: return (AgentPanelSymbol.connected, .systemGreen)
        case .ended: return (AgentPanelSymbol.ended, .secondaryLabelColor)
        }
    }
}

// MARK: - 빈 상태·진행·오류

/// 빈 상태·진행·오류 화면의 내용. 다음 할 일과 그 동작을 함께 둡니다(가이드 7.1).
struct AgentPanelEmptyState: Equatable {
    enum Action: Equatable {
        case refreshInstalledCLIs
        case openSignInTerminal
        case reviewProjectTrust
        case startSession
    }

    var symbolName: String
    var title: String
    var message: String
    var actionTitle: String?
    var action: Action?
    var showsProgress: Bool
}

/// 빈 상태·진행·오류 표시 뷰. 진행 표시는 시스템 `NSProgressIndicator`를 씁니다.
final class AgentPanelStateView: NSView {
    let progressIndicator = NSProgressIndicator()
    let symbolView = AgentPanelViews.symbolView(AgentPanelSymbol.noSession, textStyle: .title1)
    let titleLabel = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.headline))
    let messageLabel = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.callout), color: .secondaryLabelColor)
    private(set) var actionButton: NSButton!
    var onAction: ((AgentPanelEmptyState.Action) -> Void)?
    private(set) var state: AgentPanelEmptyState?
    private let stack = AgentPanelViews.verticalStack(spacing: 0, alignment: .centerX)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        progressIndicator.style = .spinning
        progressIndicator.controlSize = .small
        progressIndicator.isIndeterminate = true
        progressIndicator.isDisplayedWhenStopped = false
        progressIndicator.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.alignment = .center
        messageLabel.alignment = .center
        actionButton = NSButton(title: "", target: self, action: #selector(performAction(_:)))
        actionButton.translatesAutoresizingMaskIntoConstraints = false

        stack.spacing = AgentPanelMetrics.itemSpacing
        addSubview(stack)
        let inset = AgentPanelMetrics.edgeInset
        let centerY = stack.centerYAnchor.constraint(equalTo: centerYAnchor)
        centerY.priority = .defaultHigh
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset),
            stack.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: inset),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -inset),
            centerY,
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentPanelStateView는 코드로만 생성합니다")
    }

    /// 동작 버튼이 배치되어 있는지(숨김 대신 배치 목록에서 뺍니다).
    var showsActionButton: Bool {
        stack.arrangedSubviews.contains(actionButton)
    }

    func configure(_ state: AgentPanelEmptyState) {
        // 같은 상태면 다시 배치하지 않습니다(포커스 유지, 불필요한 레이아웃 방지).
        guard state != self.state else { return }
        self.state = state
        symbolView.image = AgentPanelViews.symbol(state.symbolName, description: nil)
        titleLabel.stringValue = state.title
        messageLabel.stringValue = state.message
        var views: [NSView] = [state.showsProgress ? progressIndicator : symbolView, titleLabel]
        if !state.message.isEmpty { views.append(messageLabel) }
        if let title = state.actionTitle, state.action != nil {
            actionButton.title = title
            views.append(actionButton)
        }
        // 레이블만 폭을 채워 가운데 정렬로 줄바꿈하고, 기호·버튼은 고유 크기를 씁니다.
        AgentPanelViews.setArrangedSubviews(views, in: stack) { $0 === self.titleLabel || $0 === self.messageLabel }
        if state.showsProgress {
            progressIndicator.startAnimation(nil)
        } else {
            progressIndicator.stopAnimation(nil)
        }
        setAccessibilityLabel(state.message.isEmpty ? state.title : "\(state.title). \(state.message)")
    }

    @objc private func performAction(_ sender: Any?) {
        guard let action = state?.action else { return }
        onAction?(action)
    }
}

// MARK: - 세션 변경 막대

/// 세션 변경 집계와 세션 종료 안내. 같은 동작은 더 보기 메뉴에도 있습니다(가이드 2.4).
final class AgentSessionBarView: NSView {
    let changesLabel = AgentPanelViews.label(font: AgentPanelViews.font(.callout), color: .secondaryLabelColor)
    let endedLabel = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.callout), color: .secondaryLabelColor)
    private(set) var revertAllButton: NSButton!
    private(set) var startButton: NSButton!
    private var changesRow: NSStackView!
    private var endedRow: NSStackView!
    private let stack = AgentPanelViews.verticalStack(spacing: AgentPanelMetrics.itemSpacing)
    private(set) var showsChangesRow = false
    private(set) var showsEndedRow = false
    private var rowsBuilt = false

    init(target: AnyObject, revertAllAction: Selector, startAction: Selector) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        revertAllButton = AgentPanelViews.textButton(AgentPanelText.revertAll, target: target, action: revertAllAction)
        revertAllButton.hasDestructiveAction = true
        startButton = AgentPanelViews.textButton(AgentPanelText.startNewSession, target: target, action: startAction)
        endedLabel.stringValue = AgentPanelText.sessionEndedBar

        changesRow = Self.row([
            AgentPanelViews.symbolView(AgentPanelSymbol.applied, textStyle: .callout), changesLabel, revertAllButton,
        ])
        endedRow = Self.row([
            AgentPanelViews.symbolView(AgentPanelSymbol.ended, textStyle: .callout), endedLabel, startButton,
        ])

        // 줄 구성은 첫 update에서 정합니다.
        addSubview(stack)
        let inset = AgentPanelMetrics.edgeInset
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: AgentPanelMetrics.itemSpacing),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -AgentPanelMetrics.itemSpacing),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentSessionBarView는 코드로만 생성합니다")
    }

    private static func row(_ views: [NSView]) -> NSStackView {
        let row = NSStackView(views: views)
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = AgentPanelMetrics.itemSpacing
        row.distribution = .fill  // 가장 낮은 hugging 항목(본문)이 남는 폭을 채움
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }

    /// 표시할 줄이 있으면 true를 반환합니다.
    @discardableResult
    func update(summary: AgentSessionChangeSummary, showsEnded: Bool, canStart: Bool) -> Bool {
        let showsChanges = summary.appliedCount > 0
        changesLabel.stringValue = AgentPanelText.changeSummary(summary)
        changesLabel.toolTip = changesLabel.stringValue
        revertAllButton.isEnabled = summary.revertableCount > 0
        startButton.isEnabled = canStart
        if !rowsBuilt || showsChanges != showsChangesRow || showsEnded != showsEndedRow {
            rowsBuilt = true
            showsChangesRow = showsChanges
            showsEndedRow = showsEnded
            var rows: [NSView] = []
            if showsChanges { rows.append(changesRow) }
            if showsEnded { rows.append(endedRow) }
            AgentPanelViews.setArrangedSubviews(rows, in: stack)
        }
        return showsChanges || showsEnded
    }
}
