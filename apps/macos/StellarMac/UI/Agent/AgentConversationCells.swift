import AppKit

/// 대화 기록 셀이 패널 컨트롤러에 요청하는 동작.
@MainActor
protocol AgentConversationActions: AnyObject {
    func conversationRequestsDecision(_ decision: AgentApprovalDecision, for request: AgentApprovalRequest)
    func conversationRequestsPreview(_ approvalID: AgentApprovalID)
    func conversationRequestsRevert(_ changeID: AgentChangeID)
}

/// 대화 기록 셀의 공통 바탕.
///
/// 행 선택 하이라이트(키 창은 강조 색)에서 레이블 색이 시스템 규칙대로 바뀌도록
/// background style을 하위 컨트롤 cell에 전달합니다(가이드 8.2).
class AgentConversationCellView: NSTableCellView {
    weak var actions: AgentConversationActions?

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { Self.propagate(backgroundStyle, in: self) }
    }

    private static func propagate(_ style: NSView.BackgroundStyle, in view: NSView) {
        for subview in view.subviews {
            if let field = subview as? NSTextField {
                field.cell?.backgroundStyle = style
            } else if let imageView = subview as? NSImageView {
                imageView.cell?.backgroundStyle = style
            }
            propagate(style, in: subview)
        }
    }

    /// 셀 안의 모든 여러 줄 레이블(검사용).
    var wrappingLabels: [NSTextField] {
        var result: [NSTextField] = []
        func collect(_ view: NSView) {
            for subview in view.subviews where !subview.isHidden {
                if let field = subview as? NSTextField, field.cell?.wraps == true, !field.isEditable {
                    result.append(field)
                }
                collect(subview)
            }
        }
        collect(self)
        return result
    }

    /// 행 하나를 VoiceOver 요소 하나로 묶습니다(가이드 8.3). 하위 레이블은 트리에서 뺍니다.
    func groupAsSingleAccessibilityElement(label: String) {
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel(label)
    }

    static func horizontalStack(_ views: [NSView], alignment: NSLayoutConstraint.Attribute = .top) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal
        stack.alignment = alignment
        stack.spacing = AgentPanelMetrics.itemSpacing
        stack.distribution = .fill  // 가장 낮은 hugging 항목(본문)이 남는 폭을 채움
        stack.translatesAutoresizingMaskIntoConstraints = false
        return stack
    }

    static func verticalStack(spacing: CGFloat) -> NSStackView {
        AgentPanelViews.verticalStack(spacing: spacing)
    }
}

// MARK: - 메시지·도구 단계·알림 행

/// 사용자 메시지, 에이전트 메시지, Stellar 도구 단계, Stellar 알림을 표시하는 행.
final class AgentTextRowCellView: AgentConversationCellView {
    static let reuseIdentifier = NSUserInterfaceItemIdentifier("AgentTextRowCell")

    let symbolView = AgentPanelViews.symbolView(AgentPanelSymbol.user)
    let captionLabel = AgentPanelViews.label(font: AgentPanelViews.font(.subheadline), color: .secondaryLabelColor)
    let bodyLabel = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.body))
    let detailLabel = AgentPanelViews.wrappingLabel(
        font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
    private let textStack = AgentConversationCellView.verticalStack(spacing: 2)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = Self.reuseIdentifier

        textStack.setHuggingPriority(.defaultLow, for: .horizontal)
        AgentPanelViews.setArrangedSubviews([captionLabel, bodyLabel], in: textStack)

        let row = Self.horizontalStack([symbolView, textStack])
        addSubview(row)
        let metrics = AgentPanelMetrics.self
        NSLayoutConstraint.activate([
            symbolView.widthAnchor.constraint(equalToConstant: metrics.symbolColumnWidth),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: metrics.edgeInset),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -metrics.edgeInset),
            row.topAnchor.constraint(equalTo: topAnchor, constant: metrics.rowVerticalPadding),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -metrics.rowVerticalPadding),
        ])
        // 행 전체를 요소 하나로 읽으므로 하위 레이블은 접근성 트리에서 뺍니다(가이드 8.3).
        for label in [captionLabel, bodyLabel, detailLabel] {
            label.setAccessibilityElement(false)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentTextRowCellView는 코드로만 생성합니다")
    }

    func configure(content: AgentConversationItem.Content) {
        var symbolName = AgentPanelSymbol.notice
        var tint: NSColor = .secondaryLabelColor
        var caption = ""
        var body = ""
        var bodyColor: NSColor = .labelColor
        var detail: String?

        switch content {
        case .userMessage(let text):
            symbolName = AgentPanelSymbol.user
            caption = AgentPanelText.you
            body = text
        case .agentMessage(let text, let isStreaming):
            symbolName = AgentPanelSymbol.agent
            caption = isStreaming ? AgentPanelText.agentResponding : AgentPanelText.agent
            body = text
        case .toolStep(let step):
            symbolName = Self.symbolName(for: step.status)
            tint = Self.tint(for: step.status)
            caption = "\(AgentPanelText.toolStatusTitle(step.status)) · \(AgentPanelText.stellarRecord)"
            body = step.title
            detail = step.detail
        case .notice(let notice):
            symbolName = AgentPanelSymbol.notice
            caption = AgentPanelText.stellar
            body = AgentPanelText.noticeText(notice)
            bodyColor = .secondaryLabelColor
        case .approval, .appliedChange:
            // 이 셀은 카드형 항목을 표시하지 않습니다(컨트롤러가 다른 셀을 고릅니다).
            break
        }

        symbolView.image = AgentPanelViews.symbol(symbolName, description: nil)
        symbolView.contentTintColor = tint
        captionLabel.stringValue = caption
        bodyLabel.stringValue = body
        bodyLabel.textColor = bodyColor
        detailLabel.stringValue = detail ?? ""
        let hasDetail = !(detail ?? "").isEmpty
        AgentPanelViews.setArrangedSubviews(
            hasDetail ? [captionLabel, bodyLabel, detailLabel] : [captionLabel, bodyLabel], in: textStack)

        var spoken = "\(caption): \(body)"
        if let detail, !detail.isEmpty { spoken += ". \(detail)" }
        groupAsSingleAccessibilityElement(label: spoken)
    }

    static func symbolName(for status: AgentToolStepStatus) -> String {
        switch status {
        case .running: return AgentPanelSymbol.toolRunning
        case .succeeded: return AgentPanelSymbol.toolSucceeded
        case .failed: return AgentPanelSymbol.toolFailed
        case .rejected: return AgentPanelSymbol.toolRejected
        }
    }

    static func tint(for status: AgentToolStepStatus) -> NSColor {
        switch status {
        case .running, .succeeded: return .secondaryLabelColor
        case .failed: return .systemRed
        case .rejected: return .systemOrange
        }
    }
}

// MARK: - 에이전트 적용 변경 행

/// 에이전트가 적용한 변경. 출처 표시("Applied by agent")와 개별 되돌리기를 제공합니다.
final class AgentChangeCellView: AgentConversationCellView {
    static let reuseIdentifier = NSUserInterfaceItemIdentifier("AgentChangeCell")

    let symbolView = AgentPanelViews.symbolView(AgentPanelSymbol.applied)
    let captionLabel = AgentPanelViews.label(font: AgentPanelViews.font(.subheadline), color: .secondaryLabelColor)
    let summaryLabel = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.body))
    let detailLabel = AgentPanelViews.wrappingLabel(
        font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
    private(set) var revertButton: NSButton!
    private var change: AgentAppliedChange?
    private let textStack = AgentConversationCellView.verticalStack(spacing: 2)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = Self.reuseIdentifier

        revertButton = AgentPanelViews.textButton(AgentPanelText.revert, target: self, action: #selector(revert(_:)))
        revertButton.toolTip = AgentPanelText.revertToolTip

        textStack.setHuggingPriority(.defaultLow, for: .horizontal)
        AgentPanelViews.setArrangedSubviews([captionLabel, summaryLabel], in: textStack)

        let row = Self.horizontalStack([symbolView, textStack, revertButton])
        addSubview(row)
        let metrics = AgentPanelMetrics.self
        NSLayoutConstraint.activate([
            symbolView.widthAnchor.constraint(equalToConstant: metrics.symbolColumnWidth),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: metrics.edgeInset),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -metrics.edgeInset),
            row.topAnchor.constraint(equalTo: topAnchor, constant: metrics.rowVerticalPadding),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -metrics.rowVerticalPadding),
        ])
        // 카드가 아니어도 같은 행 안에서 되돌리기 버튼이 보조 기술에 노출되도록 그룹으로 둡니다.
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentChangeCellView는 코드로만 생성합니다")
    }

    func configure(change: AgentAppliedChange) {
        self.change = change
        summaryLabel.stringValue = change.summary
        detailLabel.stringValue = change.detail ?? ""
        let hasDetail = !(change.detail ?? "").isEmpty
        AgentPanelViews.setArrangedSubviews(
            hasDetail ? [captionLabel, summaryLabel, detailLabel] : [captionLabel, summaryLabel], in: textStack)

        let caption: String
        let symbolName: String
        switch change.state {
        case .applied:
            caption = AgentPanelText.appliedByAgent
            symbolName = AgentPanelSymbol.applied
        case .reverted:
            caption = "\(AgentPanelText.appliedByAgent) · \(AgentPanelText.revertedLabel)"
            symbolName = AgentPanelSymbol.reverted
        case .revertUnavailable:
            caption = "\(AgentPanelText.appliedByAgent) · \(AgentPanelText.revertUnavailableLabel)"
            symbolName = AgentPanelSymbol.revertUnavailable
        }
        captionLabel.stringValue = caption
        captionLabel.lineBreakMode = .byTruncatingTail
        captionLabel.toolTip = caption
        symbolView.image = AgentPanelViews.symbol(symbolName, description: nil)
        revertButton.isHidden = change.state != .applied
        revertButton.setAccessibilityLabel("\(AgentPanelText.revert): \(change.summary)")

        setAccessibilityLabel("\(caption): \(change.summary)")
        if change.state == .applied {
            setAccessibilityCustomActions([
                NSAccessibilityCustomAction(name: AgentPanelText.revertToolTip) { [weak self] in
                    self?.revert(nil)
                    return true
                }
            ])
        } else {
            setAccessibilityCustomActions(nil)
        }
    }

    @objc private func revert(_ sender: Any?) {
        guard let change, change.state == .applied else { return }
        actions?.conversationRequestsRevert(change.changeID)
    }
}

// MARK: - 승인 카드

/// 카드 배경. 콘텐츠 계층이므로 glass를 쓰지 않고 시스템 의미 색으로 그립니다(가이드 3.1, 4.2).
///
/// 투명도 줄이기에서는 불투명 대체 색을, 대비 높이기에서는 더 굵은 테두리를 씁니다(가이드 8.4).
/// draw 시점에 동적 색을 해석하므로 Light·Dark 전환에 자동으로 다시 그려집니다.
final class AgentCardBackgroundView: NSView {
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        let borderWidth = AgentPanelViews.increaseContrast
            ? AgentPanelMetrics.cardBorderWidthIncreasedContrast
            : AgentPanelMetrics.cardBorderWidth
        let fill: NSColor = AgentPanelViews.reduceTransparency ? .controlBackgroundColor : .quaternarySystemFill
        let rect = bounds.insetBy(dx: borderWidth / 2, dy: borderWidth / 2)
        let radius = AgentPanelMetrics.cardCornerRadius
        let path = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
        fill.setFill()
        path.fill()
        path.lineWidth = borderWidth
        NSColor.separatorColor.setStroke()
        path.stroke()
    }
}

/// 승인 카드. 주 내용(diff, validation, 잠금 범위)은 코어가 만든 값이고,
/// 에이전트 설명은 출처를 표시한 보조 정보로 따로 둡니다(ADR-0005 결정 4.5).
final class AgentApprovalCellView: AgentConversationCellView {
    static let reuseIdentifier = NSUserInterfaceItemIdentifier("AgentApprovalCell")

    let cardView = AgentCardBackgroundView()
    let contentStack = AgentConversationCellView.verticalStack(spacing: AgentPanelMetrics.itemSpacing)
    private(set) var approveButton: NSButton?
    private(set) var declineButton: NSButton?
    private(set) var previewButton: NSButton?
    /// 에이전트 설명 레이블(검사용: 출처 표시 확인).
    private(set) var agentNoteLabel: NSTextField?
    private(set) var agentProvenanceLabel: NSTextField?
    private var request: AgentApprovalRequest?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = Self.reuseIdentifier
        cardView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(cardView)
        cardView.addSubview(contentStack)
        let metrics = AgentPanelMetrics.self
        let inset = metrics.cardContentInset
        NSLayoutConstraint.activate([
            cardView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: metrics.edgeInset),
            cardView.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -metrics.edgeInset),
            cardView.topAnchor.constraint(equalTo: topAnchor, constant: metrics.rowVerticalPadding),
            cardView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -metrics.rowVerticalPadding),
            contentStack.leadingAnchor.constraint(equalTo: cardView.leadingAnchor, constant: inset),
            contentStack.trailingAnchor.constraint(equalTo: cardView.trailingAnchor, constant: -inset),
            contentStack.topAnchor.constraint(equalTo: cardView.topAnchor, constant: inset),
            contentStack.bottomAnchor.constraint(equalTo: cardView.bottomAnchor, constant: -inset),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentApprovalCellView는 코드로만 생성합니다")
    }

    /// 접근성 표시 설정이 바뀌면 카드 배경을 다시 그립니다.
    func refreshAccessibilityDisplayOptions() {
        cardView.needsDisplay = true
    }

    func configure(request: AgentApprovalRequest) {
        self.request = request
        var rows: [NSView] = []
        approveButton = nil
        declineButton = nil
        previewButton = nil
        agentNoteLabel = nil
        agentProvenanceLabel = nil

        // 제목: 승인 종류. 색이 아니라 기호와 문구로 구분합니다.
        let title = AgentPanelViews.label(font: AgentPanelViews.font(.headline))
        title.stringValue = "\(AgentPanelText.approvalNeeded) · \(AgentPanelText.approvalKindTitle(request.kind))"
        let titleSymbol = AgentPanelViews.symbolView(
            AgentPanelSymbol.approval, textStyle: .headline,
            tint: request.kind.requiresSensitiveConfirmation ? .systemOrange : .secondaryLabelColor)
        rows.append(Self.horizontalStack([titleSymbol, title], alignment: .centerY))

        // 코어 요약.
        let summary = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.body))
        summary.stringValue = request.summary
        rows.append(summary)

        // 코어 semantic diff 요약.
        if !request.diffLines.isEmpty || request.omittedDiffLineCount > 0 {
            rows.append(sectionLabel(AgentPanelText.changesSection))
            for line in request.diffLines {
                rows.append(diffRow(line))
            }
            if request.omittedDiffLineCount > 0 {
                let more = AgentPanelViews.label(font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
                more.stringValue = AgentPanelText.omittedDiffLines(request.omittedDiffLineCount)
                rows.append(more)
            }
        }

        // 코어 validation 결과.
        rows.append(sectionLabel(AgentPanelText.validationSection))
        rows.append(validationRow(request.validation))
        for message in request.validation.messages {
            let messageLabel = AgentPanelViews.wrappingLabel(
                font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
            messageLabel.stringValue = message
            rows.append(messageLabel)
        }

        // 코어 잠금 범위.
        rows.append(sectionLabel(AgentPanelText.lockScopeSection))
        rows.append(symbolTextRow(AgentPanelSymbol.lockScope, text: request.lockScope))

        // 에이전트 설명: 출처를 표시한 보조 정보.
        if let explanation = request.agentExplanation, !explanation.isEmpty {
            let header = Self.horizontalStack(
                [AgentPanelViews.symbolView(AgentPanelSymbol.agentNote, textStyle: .subheadline),
                 sectionLabel(AgentPanelText.agentNoteSection)],
                alignment: .centerY)
            rows.append(header)
            let provenance = AgentPanelViews.wrappingLabel(
                font: AgentPanelViews.font(.caption1), color: .secondaryLabelColor)
            provenance.stringValue = AgentPanelText.agentNoteProvenance
            rows.append(provenance)
            let note = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.callout), color: .secondaryLabelColor)
            note.stringValue = explanation
            note.setAccessibilityLabel("\(AgentPanelText.agentNoteSection), \(AgentPanelText.agentNoteProvenance) \(explanation)")
            rows.append(note)
            agentNoteLabel = note
            agentProvenanceLabel = provenance
        }

        if request.status == .pending {
            rows.append(buttonRow(for: request))
        } else {
            rows.append(statusRow(request.status))
        }
        AgentPanelViews.setArrangedSubviews(rows, in: contentStack)

        setAccessibilityLabel(
            "\(AgentPanelText.approvalNeeded), \(AgentPanelText.approvalKindTitle(request.kind)): \(request.summary). "
                + "\(AgentPanelText.validationSection): \(AgentPanelText.validationTitle(request.validation)). "
                + AgentPanelText.approvalStatusTitle(request.status))
        if request.status == .pending {
            setAccessibilityCustomActions([
                NSAccessibilityCustomAction(
                    name: request.kind.requiresSensitiveConfirmation
                        ? AgentPanelText.approveSensitive : AgentPanelText.approve
                ) { [weak self] in
                    self?.approve(nil)
                    return true
                },
                NSAccessibilityCustomAction(name: AgentPanelText.decline) { [weak self] in
                    self?.decline(nil)
                    return true
                },
            ])
        } else {
            setAccessibilityCustomActions(nil)
        }
        cardView.needsDisplay = true
    }

    private func sectionLabel(_ text: String) -> NSTextField {
        let label = AgentPanelViews.label(font: AgentPanelViews.font(.subheadline), color: .secondaryLabelColor)
        label.stringValue = text
        label.setAccessibilityRole(.staticText)
        return label
    }

    private func symbolTextRow(_ symbolName: String, text: String, tint: NSColor = .secondaryLabelColor) -> NSStackView {
        let symbol = AgentPanelViews.symbolView(symbolName, textStyle: .callout, tint: tint)
        let label = AgentPanelViews.wrappingLabel(font: AgentPanelViews.font(.callout))
        label.stringValue = text
        let row = Self.horizontalStack([symbol, label])
        symbol.widthAnchor.constraint(equalToConstant: AgentPanelMetrics.symbolColumnWidth).isActive = true
        return row
    }

    private func diffRow(_ line: AgentDiffLine) -> NSStackView {
        let symbolName: String
        switch line.change {
        case .added: symbolName = AgentPanelSymbol.diffAdded
        case .removed: symbolName = AgentPanelSymbol.diffRemoved
        case .modified: symbolName = AgentPanelSymbol.diffModified
        }
        var text = line.target
        if let detail = line.detail, !detail.isEmpty { text += " — \(detail)" }
        let row = symbolTextRow(symbolName, text: text)
        // 기호(모양)로 구분한 변경 종류를 문구로도 노출합니다.
        if let label = row.arrangedSubviews.last as? NSTextField {
            label.setAccessibilityLabel("\(AgentPanelText.diffChangeTitle(line.change)): \(text)")
        }
        return row
    }

    private func validationRow(_ validation: AgentValidationSummary) -> NSStackView {
        let symbolName: String
        let tint: NSColor
        if validation.errorCount > 0 {
            symbolName = AgentPanelSymbol.validationError
            tint = .systemRed
        } else if validation.warningCount > 0 {
            symbolName = AgentPanelSymbol.validationWarning
            tint = .systemOrange
        } else {
            symbolName = AgentPanelSymbol.validationPassed
            tint = .secondaryLabelColor
        }
        return symbolTextRow(symbolName, text: AgentPanelText.validationTitle(validation), tint: tint)
    }

    private func statusRow(_ status: AgentApprovalStatus) -> NSStackView {
        let symbolName: String
        switch status {
        case .pending: symbolName = AgentPanelSymbol.approval
        case .approved: symbolName = AgentPanelSymbol.approved
        case .declined: symbolName = AgentPanelSymbol.declined
        case .expired: symbolName = AgentPanelSymbol.expired
        }
        return symbolTextRow(symbolName, text: AgentPanelText.approvalStatusTitle(status))
    }

    private func buttonRow(for request: AgentApprovalRequest) -> NSStackView {
        let preview = AgentPanelViews.textButton(AgentPanelText.preview, target: self, action: #selector(preview(_:)))
        preview.toolTip = AgentPanelText.previewToolTip
        let decline = AgentPanelViews.textButton(AgentPanelText.decline, target: self, action: #selector(decline(_:)))
        let sensitive = request.kind.requiresSensitiveConfirmation
        let approve = AgentPanelViews.textButton(
            sensitive ? AgentPanelText.approveSensitive : AgentPanelText.approve,
            target: self, action: #selector(approve(_:)))
        // 민감 확인은 파괴적 동작 패턴을 따르고, 어떤 승인 버튼도 Return 키 등가를 갖지 않습니다.
        approve.hasDestructiveAction = sensitive
        approve.keyEquivalent = ""
        decline.keyEquivalent = ""

        let row = NSStackView()
        row.orientation = .horizontal
        row.spacing = AgentPanelMetrics.itemSpacing
        row.translatesAutoresizingMaskIntoConstraints = false
        row.setViews([preview], in: .leading)
        row.setViews([decline, approve], in: .trailing)
        approveButton = approve
        declineButton = decline
        previewButton = preview
        return row
    }

    @objc private func approve(_ sender: Any?) {
        guard let request, request.status == .pending else { return }
        actions?.conversationRequestsDecision(.approve, for: request)
    }

    @objc private func decline(_ sender: Any?) {
        guard let request, request.status == .pending else { return }
        actions?.conversationRequestsDecision(.decline, for: request)
    }

    @objc private func preview(_ sender: Any?) {
        guard let request else { return }
        actions?.conversationRequestsPreview(request.approvalID)
    }
}
