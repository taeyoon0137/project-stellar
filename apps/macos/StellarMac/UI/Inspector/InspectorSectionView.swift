import AppKit

/// 접고 펼 수 있는 인스펙터 섹션(가이드 6.3, HIG Disclosure controls).
///
/// 위에서 아래로 구분선 → 머리줄(표준 disclosure 버튼 + 제목 + 선택적 accessory 버튼) → 본문입니다.
/// frame으로 배치합니다(`InspectorLayout`). 접기·펼치기 전환은 동작 줄이기가 켜져 있으면 즉시 바꿉니다
/// (가이드 8.4, 8.5).
final class InspectorSectionView: InspectorFlowView {
    let sectionID: InspectorSectionID
    private(set) var isExpanded: Bool
    /// 사용자가 접힘 상태를 바꿨을 때 호출됩니다.
    var onExpansionChange: ((InspectorSectionID, Bool) -> Void)?

    let disclosureButton: NSButton
    private let titleLabel: NSTextField
    private let separator = NSBox()
    private var accessories: [(view: NSView, size: NSSize)] = []
    private(set) var content: InspectorFlowContent?
    private var disclosureSize = NSSize.zero
    private var titleHeight: CGFloat = 0
    private var separatorHeight: CGFloat = 1

    init(id: InspectorSectionID, title: String, expanded: Bool) {
        sectionID = id
        isExpanded = expanded
        // 편의 생성자는 생성 시 sizeToFit을 호출해 비용이 큽니다(macOS 26). 기본 생성 후 구성합니다.
        disclosureButton = NSButton()
        titleLabel = NSTextField(labelWithString: title)
        super.init(frame: .zero)
        build(title: title)
    }

    private func build(title: String) {
        separator.boxType = .separator
        addSubview(separator)
        separatorHeight = max(InspectorLayout.measure(separator).height, 1)

        disclosureButton.bezelStyle = .disclosure
        disclosureButton.setButtonType(.pushOnPushOff)
        disclosureButton.title = ""
        disclosureButton.state = isExpanded ? .on : .off
        disclosureButton.target = self
        disclosureButton.action = #selector(disclosurePressed(_:))
        disclosureButton.identifier = NSUserInterfaceItemIdentifier("section.\(sectionID.rawValue).disclosure")
        addSubview(disclosureButton)
        disclosureSize = InspectorLayout.measure(disclosureButton)

        titleLabel.font = InspectorFonts.sectionTitle
        titleLabel.textColor = .labelColor
        titleLabel.lineBreakMode = .byTruncatingTail
        // 제목 클릭도 disclosure와 같은 동작입니다. 접근성은 disclosure 버튼 하나로 노출합니다.
        titleLabel.setAccessibilityElement(false)
        titleLabel.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(titleClicked(_:))))
        addSubview(titleLabel)
        titleHeight = InspectorLayout.measure(titleLabel).height
        setTitle(title)

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    /// 제목을 바꿉니다.
    func setTitle(_ title: String) {
        if titleLabel.stringValue != title { titleLabel.stringValue = title }
        disclosureButton.setAccessibilityLabel(title)
        disclosureButton.toolTip = title
        setAccessibilityLabel(title)
    }

    /// 머리줄 오른쪽 accessory 뷰(예: modifier 추가·삭제)를 바꿉니다. 크기는 지금 잽니다.
    func setAccessoryViews(_ views: [NSView]) {
        setAccessoryViews(views.map { ($0, InspectorLayout.measure($0)) })
    }

    /// 크기를 이미 잰 accessory 뷰를 붙입니다(나눠 만들기에서 측정과 붙이기를 다른 단계로 나눌 때).
    func setAccessoryViews(_ measured: [(view: NSView, size: NSSize)]) {
        accessories.forEach { $0.view.removeFromSuperview() }
        accessories = measured
        measured.forEach { addSubview($0.view) }
        invalidateFlow()
    }

    /// 머리줄 accessory 뷰를 보이거나 숨깁니다(다시 만들거나 재지 않음).
    func setAccessoriesHidden(_ hidden: Bool) {
        guard accessories.contains(where: { $0.view.isHidden != hidden }) else { return }
        accessories.forEach { $0.view.isHidden = hidden }
        invalidateFlow()
    }

    /// 본문 뷰를 정합니다.
    func setContent(_ view: InspectorFlowContent) {
        content?.removeFromSuperview()
        content = view
        view.isHidden = !isExpanded
        addSubview(view)
        invalidateFlow()
    }

    /// 접힘 상태를 바꿉니다.
    func setExpanded(_ expanded: Bool, animated: Bool) {
        guard expanded != isExpanded else { return }
        isExpanded = expanded
        disclosureButton.state = expanded ? .on : .off
        content?.isHidden = !expanded
        invalidateFlow()
        let animate = animated && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        if animate, let page = enclosingPage {
            NSAnimationContext.runAnimationGroup { context in
                context.allowsImplicitAnimation = true
                page.layoutSubtreeIfNeeded()
            }
        }
        onExpansionChange?(sectionID, expanded)
    }

    private var enclosingPage: NSView? {
        var current = superview
        while let view = current, !(view is InspectorPageView) { current = view.superview }
        return current
    }

    @objc private func disclosurePressed(_ sender: NSButton) {
        setExpanded(sender.state == .on, animated: true)
    }

    @objc private func titleClicked(_ sender: NSClickGestureRecognizer) {
        setExpanded(!isExpanded, animated: true)
    }

    // MARK: - 배치

    private var headerHeight: CGFloat {
        var height = max(disclosureSize.height, titleHeight)
        for accessory in accessories where !accessory.view.isHidden {
            height = max(height, accessory.size.height)
        }
        return ceil(height)
    }

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        var height = separatorHeight + InspectorMetrics.rowSpacing + headerHeight
        if isExpanded, let content {
            height += InspectorMetrics.rowSpacing + content.flowHeight(forWidth: width)
        }
        return ceil(height)
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        let separatorFrame = NSRect(x: 0, y: 0, width: width, height: separatorHeight)
        if separator.frame != separatorFrame { separator.frame = separatorFrame }

        let headerY = separatorHeight + InspectorMetrics.rowSpacing
        let header = headerHeight
        InspectorLayout.place(disclosureButton, x: 0, width: disclosureSize.width, size: disclosureSize,
                              lineY: headerY, lineHeight: header)
        let visibleAccessories = accessories.filter { !$0.view.isHidden }
        var accessoryX = width - visibleAccessories.reduce(0) { $0 + $1.size.width }
            - CGFloat(max(visibleAccessories.count - 1, 0)) * InspectorMetrics.groupSpacing
        let titleX = disclosureSize.width + InspectorMetrics.groupSpacing
        let titleRight = visibleAccessories.isEmpty ? width : accessoryX - InspectorMetrics.groupSpacing
        InspectorLayout.place(titleLabel, x: titleX, width: max(0, titleRight - titleX),
                              size: NSSize(width: 0, height: titleHeight), lineY: headerY, lineHeight: header)
        accessoryX = max(accessoryX, titleX)
        for accessory in visibleAccessories {
            InspectorLayout.place(accessory.view, x: accessoryX, width: accessory.size.width, size: accessory.size,
                                  lineY: headerY, lineHeight: header)
            accessoryX += accessory.size.width + InspectorMetrics.groupSpacing
        }

        if let content, isExpanded {
            let y = headerY + header + InspectorMetrics.rowSpacing
            let frame = NSRect(x: 0, y: y, width: width, height: content.flowHeight(forWidth: width))
            if content.frame != frame { content.frame = frame }
        }
    }
}

/// 선택 요약 머리줄: 종류 아이콘 + 이름 + 종류·개수.
final class InspectorHeaderView: InspectorFlowView {
    private let iconView = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")
    private let subtitleLabel = NSTextField(labelWithString: "")
    private var iconSize = NSSize.zero
    private var iconSymbol: String?
    private var titleHeight: CGFloat = 0
    private var subtitleHeight: CGFloat = 0

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        iconView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title3)
        iconView.setAccessibilityElement(false)

        titleLabel.font = InspectorFonts.headline
        titleLabel.lineBreakMode = .byTruncatingTail
        subtitleLabel.font = InspectorFonts.caption
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.setAccessibilityElement(false)
        subtitleLabel.setAccessibilityElement(false)
        for view in [iconView, titleLabel, subtitleLabel] { addSubview(view) }
        titleHeight = InspectorLayout.measure(titleLabel).height
        subtitleHeight = InspectorLayout.measure(subtitleLabel).height

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Selection")
    }

    func update(_ selection: InspectorSelectionSummary) {
        let kindTitles = selection.nodeKinds.map { kind -> String in
            if kind == .shape, let shape = selection.shapeKind { return shape.title }
            return kind.title
        }
        let symbol: String
        let title: String
        let subtitle: String
        if selection.count == 1, let kind = selection.nodeKinds.first {
            symbol = InspectorSymbols.node(kind, shape: selection.shapeKind)
            title = selection.displayName ?? kindTitles.first ?? kind.title
            subtitle = kindTitles.first ?? kind.title
        } else if selection.nodeKinds.count == 1, let kind = selection.nodeKinds.first {
            symbol = InspectorSymbols.node(kind, shape: selection.shapeKind)
            title = "\(selection.count) Layers"
            subtitle = kindTitles.first ?? kind.title
        } else {
            symbol = InspectorSymbols.mixedSelection
            title = "\(selection.count) Layers"
            subtitle = kindTitles.joined(separator: ", ")
        }
        if iconSymbol != symbol || iconView.image?.accessibilityDescription != subtitle {
            iconView.image = InspectorSymbols.image(symbol, label: subtitle)
            iconSymbol = symbol
            let size = InspectorLayout.measure(iconView)
            if size != iconSize {
                iconSize = size
                invalidateFlow()
            }
            needsLayout = true
        }
        if titleLabel.stringValue != title { titleLabel.stringValue = title }
        if subtitleLabel.stringValue != subtitle { subtitleLabel.stringValue = subtitle }
        setAccessibilityValue("\(title), \(subtitle)")
    }

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        ceil(max(iconSize.height, titleHeight + subtitleHeight))
    }

    override func layout() {
        super.layout()
        let height = bounds.height
        InspectorLayout.place(iconView, x: 0, width: iconSize.width, size: iconSize, lineY: 0, lineHeight: height)
        let textX = iconSize.width + InspectorMetrics.rowSpacing
        let textWidth = max(0, bounds.width - textX)
        let textTop = (height - titleHeight - subtitleHeight) / 2
        InspectorLayout.place(titleLabel, alignmentRect: NSRect(x: textX, y: textTop, width: textWidth,
                                                                height: titleHeight))
        InspectorLayout.place(subtitleLabel, alignmentRect: NSRect(x: textX, y: textTop + titleHeight,
                                                                   width: textWidth, height: subtitleHeight))
    }
}

/// 잠금·읽기 전용 안내 줄. 편집 가능하면 숨깁니다.
final class InspectorBannerView: InspectorFlowView {
    private let iconView = NSImageView()
    private let messageLabel = InspectorWrappingLabel(text: "", font: InspectorFonts.control, color: .secondaryLabelColor)
    let actionButton = NSButton()
    /// 잠금 해제 요청.
    var onUnlock: (() -> Void)?
    private var iconSize = NSSize.zero
    private var buttonSize = NSSize.zero

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        iconView.setAccessibilityElement(false)
        messageLabel.setAccessibilityElement(false)
        actionButton.title = "Unlock"
        actionButton.bezelStyle = .push
        actionButton.controlSize = InspectorMetrics.controlSize
        actionButton.font = InspectorFonts.control
        actionButton.target = self
        actionButton.action = #selector(unlockPressed(_:))
        actionButton.identifier = NSUserInterfaceItemIdentifier("banner.unlock")
        actionButton.setAccessibilityLabel("Unlock Layer")
        for view in [iconView, messageLabel, actionButton] as [NSView] { addSubview(view) }
        isHidden = true
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    /// Unlock 버튼 크기를 잽니다(첫 잠금 표시 전에 나눠 만들기 단계에서 부릅니다).
    func prepare() {
        if buttonSize == .zero { buttonSize = InspectorLayout.measure(actionButton) }
    }

    func update(_ editability: InspectorEditability, canUnlock: Bool) {
        let wasHidden = isHidden
        let previousMessage = messageLabel.stringValue
        let previousButtonHidden = actionButton.isHidden
        switch editability {
        case .editable:
            isHidden = true
        case .locked:
            isHidden = false
            setIcon(InspectorSymbols.locked, label: "Locked")
            messageLabel.stringValue = "This layer is locked. Unlock it to edit its properties."
            actionButton.isHidden = !canUnlock
            setAccessibilityLabel("Locked")
            setAccessibilityValue(messageLabel.stringValue)
        case .readOnly(let reason):
            isHidden = false
            setIcon(InspectorSymbols.readOnly, label: "Read Only")
            messageLabel.stringValue = reason
            actionButton.isHidden = true
            setAccessibilityLabel("Read Only")
            setAccessibilityValue(reason)
        }
        if wasHidden != isHidden || previousMessage != messageLabel.stringValue
            || previousButtonHidden != actionButton.isHidden {
            prepare()
            invalidateFlow()
        }
    }

    private func setIcon(_ symbol: String, label: String) {
        guard iconView.image?.accessibilityDescription != label else { return }
        iconView.image = InspectorSymbols.image(symbol, label: label)
        iconSize = InspectorLayout.measure(iconView)
    }

    private func messageWidth(forWidth width: CGFloat) -> CGFloat {
        var available = width - iconSize.width - InspectorMetrics.rowSpacing
        if !actionButton.isHidden { available -= buttonSize.width + InspectorMetrics.rowSpacing }
        return max(available, 1)
    }

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        let message = messageLabel.flowHeight(forWidth: messageWidth(forWidth: width))
        return ceil(max(iconSize.height, message, actionButton.isHidden ? 0 : buttonSize.height))
    }

    override func layout() {
        super.layout()
        let height = bounds.height
        InspectorLayout.place(iconView, x: 0, width: iconSize.width, size: iconSize, lineY: 0, lineHeight: height)
        let messageX = iconSize.width + InspectorMetrics.rowSpacing
        let width = messageWidth(forWidth: bounds.width)
        let messageHeight = messageLabel.flowHeight(forWidth: width)
        let messageFrame = NSRect(x: messageX, y: (height - messageHeight) / 2, width: width, height: messageHeight)
        if messageLabel.frame != messageFrame { messageLabel.frame = messageFrame }
        if !actionButton.isHidden {
            InspectorLayout.place(actionButton, x: bounds.width - buttonSize.width, width: buttonSize.width,
                                  size: buttonSize, lineY: 0, lineHeight: height)
        }
    }

    @objc private func unlockPressed(_ sender: NSButton) {
        onUnlock?()
    }
}

/// 선택 없음·로딩·오류 상태 화면(가이드 7.1). 보이는 요소를 세로 가운데에 쌓습니다.
final class InspectorStateView: InspectorFlowView {
    private let iconView = NSImageView()
    private let spinner = NSProgressIndicator()
    private let titleLabel = NSTextField(labelWithString: "")
    private let messageLabel = InspectorWrappingLabel(text: "")
    let retryButton = NSButton()
    /// 다시 시도 요청.
    var onRetry: (() -> Void)?
    private var iconSize = NSSize.zero
    private var spinnerSize = NSSize.zero
    private var titleHeight: CGFloat = 0
    private var retrySize = NSSize.zero
    private var shownState: InspectorContentState?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        iconView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title1)
        iconView.contentTintColor = .secondaryLabelColor
        iconView.setAccessibilityElement(false)
        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        spinner.setAccessibilityLabel("Loading")
        titleLabel.font = InspectorFonts.headline
        titleLabel.textColor = .secondaryLabelColor
        titleLabel.alignment = .center
        titleLabel.lineBreakMode = .byTruncatingTail
        messageLabel.alignment = .center
        retryButton.title = "Try Again"
        retryButton.bezelStyle = .push
        retryButton.target = self
        retryButton.action = #selector(retryPressed(_:))
        retryButton.identifier = NSUserInterfaceItemIdentifier("state.retry")
        for view in [iconView, spinner, titleLabel, messageLabel, retryButton] as [NSView] { addSubview(view) }
        spinnerSize = InspectorLayout.measure(spinner)
        titleHeight = InspectorLayout.measure(titleLabel).height
        retrySize = InspectorLayout.measure(retryButton)
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    func show(_ state: InspectorContentState) {
        guard state != shownState else { return }
        shownState = state
        iconView.isHidden = true
        spinner.stopAnimation(nil)
        spinner.isHidden = true
        retryButton.isHidden = true
        messageLabel.isHidden = false
        switch state {
        case .noSelection:
            setIcon(InspectorSymbols.noSelection, label: "No Selection")
            titleLabel.stringValue = "No Selection"
            messageLabel.stringValue = "Select a layer on the canvas or in the layer list to see its properties."
        case .loading:
            spinner.isHidden = false
            spinner.startAnimation(nil)
            titleLabel.stringValue = "Loading Properties…"
            messageLabel.isHidden = true
        case .failed(let message):
            setIcon(InspectorSymbols.failure, label: "Error")
            titleLabel.stringValue = "Can’t Show Properties"
            messageLabel.stringValue = message
            retryButton.isHidden = false
        case .ready:
            break
        }
        setAccessibilityLabel(titleLabel.stringValue)
        setAccessibilityValue(messageLabel.isHidden ? nil : messageLabel.stringValue)
        needsLayout = true
    }

    private func setIcon(_ symbol: String, label: String) {
        iconView.isHidden = false
        iconView.image = InspectorSymbols.image(symbol, label: label)
        iconSize = InspectorLayout.measure(iconView)
    }

    override func layout() {
        super.layout()
        let contentWidth = max(bounds.width - 2 * InspectorMetrics.panelInset, 1)
        var entries: [(view: NSView, size: NSSize)] = []
        if !iconView.isHidden { entries.append((iconView, iconSize)) }
        if !spinner.isHidden { entries.append((spinner, spinnerSize)) }
        entries.append((titleLabel, NSSize(width: contentWidth, height: titleHeight)))
        if !messageLabel.isHidden {
            entries.append((messageLabel, NSSize(width: contentWidth, height: messageLabel.flowHeight(forWidth: contentWidth))))
        }
        if !retryButton.isHidden { entries.append((retryButton, retrySize)) }
        let total = entries.reduce(0) { $0 + $1.size.height }
            + CGFloat(max(entries.count - 1, 0)) * InspectorMetrics.rowSpacing
        var y = max(0, (bounds.height - total) / 2)
        for entry in entries {
            let x = (bounds.width - entry.size.width) / 2
            if entry.view === messageLabel {
                let frame = NSRect(x: x, y: y, width: entry.size.width, height: entry.size.height)
                if messageLabel.frame != frame { messageLabel.frame = frame }
            } else {
                InspectorLayout.place(entry.view, alignmentRect: NSRect(origin: NSPoint(x: x, y: y), size: entry.size))
            }
            y += entry.size.height + InspectorMetrics.rowSpacing
        }
    }

    @objc private func retryPressed(_ sender: NSButton) {
        onRetry?()
    }
}
