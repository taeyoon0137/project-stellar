import AppKit

/// 접고 펼 수 있는 인스펙터 섹션(가이드 6.3, HIG Disclosure controls).
///
/// 머리줄은 표준 disclosure 버튼 + 제목 + 선택적 accessory 버튼, 본문은 호출자가 넣는 뷰입니다.
/// 접기·펼치기 전환은 동작 줄이기가 켜져 있으면 즉시 바꿉니다(가이드 8.4, 8.5).
final class InspectorSectionView: NSView {
    let sectionID: InspectorSectionID
    private(set) var isExpanded: Bool
    /// 사용자가 접힘 상태를 바꿨을 때 호출됩니다.
    var onExpansionChange: ((InspectorSectionID, Bool) -> Void)?

    let disclosureButton: NSButton
    private let titleLabel: NSTextField
    private let headerStack = NSStackView()
    private let accessoryStack = NSStackView()
    private let contentStack = NSStackView()
    private let outerStack = NSStackView()
    private let separator = NSBox()

    init(id: InspectorSectionID, title: String, expanded: Bool) {
        sectionID = id
        isExpanded = expanded
        // 편의 생성자는 생성 시 sizeToFit을 호출해 비용이 큽니다(macOS 26). 기본 생성 후 구성합니다.
        disclosureButton = NSButton()
        titleLabel = NSTextField(labelWithString: title)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build(title: title)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorSectionView는 코드로만 생성합니다")
    }

    private func build(title: String) {
        separator.boxType = .separator
        separator.translatesAutoresizingMaskIntoConstraints = false

        disclosureButton.bezelStyle = .disclosure
        disclosureButton.setButtonType(.pushOnPushOff)
        disclosureButton.title = ""
        disclosureButton.state = isExpanded ? .on : .off
        disclosureButton.target = self
        disclosureButton.action = #selector(disclosurePressed(_:))
        disclosureButton.identifier = NSUserInterfaceItemIdentifier("section.\(sectionID.rawValue).disclosure")
        InspectorFieldController.hug(disclosureButton)

        titleLabel.font = InspectorFonts.sectionTitle
        titleLabel.textColor = .labelColor
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        titleLabel.setContentHuggingPriority(.init(1), for: .horizontal)
        // 제목 클릭도 disclosure와 같은 동작입니다. 접근성은 disclosure 버튼 하나로 노출합니다.
        titleLabel.setAccessibilityElement(false)
        titleLabel.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(titleClicked(_:))))
        setTitle(title)

        accessoryStack.orientation = .horizontal
        accessoryStack.spacing = InspectorMetrics.groupSpacing
        accessoryStack.setHuggingPriority(.defaultHigh, for: .horizontal)
        accessoryStack.isHidden = true

        headerStack.orientation = .horizontal
        headerStack.alignment = .centerY
        headerStack.spacing = InspectorMetrics.groupSpacing
        headerStack.addArrangedSubview(disclosureButton)
        headerStack.addArrangedSubview(titleLabel)
        headerStack.addArrangedSubview(accessoryStack)

        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = InspectorMetrics.rowSpacing
        contentStack.isHidden = !isExpanded

        outerStack.orientation = .vertical
        outerStack.alignment = .leading
        outerStack.spacing = InspectorMetrics.rowSpacing
        outerStack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(outerStack)
        for view in [separator, headerStack, contentStack] as [NSView] {
            outerStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: outerStack.widthAnchor).isActive = true
        }
        NSLayoutConstraint.activate([
            outerStack.leadingAnchor.constraint(equalTo: leadingAnchor),
            outerStack.trailingAnchor.constraint(equalTo: trailingAnchor),
            outerStack.topAnchor.constraint(equalTo: topAnchor),
            outerStack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    /// 제목을 바꿉니다.
    func setTitle(_ title: String) {
        titleLabel.stringValue = title
        disclosureButton.setAccessibilityLabel(title)
        disclosureButton.toolTip = title
        setAccessibilityLabel(title)
    }

    /// 머리줄 오른쪽 accessory 뷰(예: modifier 추가·삭제)를 바꿉니다.
    func setAccessoryViews(_ views: [NSView]) {
        accessoryStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        views.forEach { accessoryStack.addArrangedSubview($0) }
        // 빈 stack은 폭이 정해지지 않으므로 머리줄에서 뺍니다.
        accessoryStack.isHidden = views.isEmpty
    }

    /// 본문 뷰를 바꿉니다. 각 뷰는 섹션 폭에 맞춥니다.
    func setContent(_ views: [NSView]) {
        contentStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for view in views {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
    }

    /// 접힘 상태를 바꿉니다.
    func setExpanded(_ expanded: Bool, animated: Bool) {
        guard expanded != isExpanded else { return }
        isExpanded = expanded
        disclosureButton.state = expanded ? .on : .off
        let animate = animated && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        if animate, let container = window?.contentView {
            NSAnimationContext.runAnimationGroup { context in
                context.allowsImplicitAnimation = true
                contentStack.isHidden = !expanded
                container.layoutSubtreeIfNeeded()
            }
        } else {
            contentStack.isHidden = !expanded
        }
        onExpansionChange?(sectionID, expanded)
    }

    @objc private func disclosurePressed(_ sender: NSButton) {
        setExpanded(sender.state == .on, animated: true)
    }

    @objc private func titleClicked(_ sender: NSClickGestureRecognizer) {
        setExpanded(!isExpanded, animated: true)
    }
}

/// 선택 요약 머리줄: 종류 아이콘 + 이름 + 종류·개수.
final class InspectorHeaderView: NSView {
    private let iconView = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")
    private let subtitleLabel = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        iconView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title3)
        iconView.setAccessibilityElement(false)
        InspectorFieldController.hug(iconView)

        titleLabel.font = InspectorFonts.headline
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        subtitleLabel.font = InspectorFonts.caption
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingTail
        subtitleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        titleLabel.setAccessibilityElement(false)
        subtitleLabel.setAccessibilityElement(false)

        let texts = NSStackView(views: [titleLabel, subtitleLabel])
        texts.orientation = .vertical
        texts.alignment = .leading
        texts.spacing = 0
        let stack = NSStackView(views: [iconView, texts])
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = InspectorMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Selection")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorHeaderView는 코드로만 생성합니다")
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
        iconView.image = InspectorSymbols.image(symbol, label: subtitle)
        titleLabel.stringValue = title
        subtitleLabel.stringValue = subtitle
        setAccessibilityValue("\(title), \(subtitle)")
    }
}

/// 잠금·읽기 전용 안내 줄. 편집 가능하면 숨깁니다.
final class InspectorBannerView: NSView {
    private let iconView = NSImageView()
    private let messageLabel = InspectorWrappingLabel(text: "", font: InspectorFonts.control, color: .secondaryLabelColor)
    let actionButton = NSButton()
    /// 잠금 해제 요청.
    var onUnlock: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        iconView.setAccessibilityElement(false)
        InspectorFieldController.hug(iconView)
        messageLabel.setAccessibilityElement(false)
        actionButton.title = "Unlock"
        actionButton.bezelStyle = .push
        actionButton.controlSize = InspectorMetrics.controlSize
        actionButton.font = InspectorFonts.control
        actionButton.target = self
        actionButton.action = #selector(unlockPressed(_:))
        actionButton.identifier = NSUserInterfaceItemIdentifier("banner.unlock")
        actionButton.setAccessibilityLabel("Unlock Layer")
        InspectorFieldController.hug(actionButton)

        let stack = NSStackView(views: [iconView, messageLabel, actionButton])
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = InspectorMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorBannerView는 코드로만 생성합니다")
    }

    func update(_ editability: InspectorEditability, canUnlock: Bool) {
        switch editability {
        case .editable:
            isHidden = true
        case .locked:
            isHidden = false
            iconView.image = InspectorSymbols.image(InspectorSymbols.locked, label: "Locked")
            messageLabel.stringValue = "This layer is locked. Unlock it to edit its properties."
            actionButton.isHidden = !canUnlock
            setAccessibilityLabel("Locked")
            setAccessibilityValue(messageLabel.stringValue)
        case .readOnly(let reason):
            isHidden = false
            iconView.image = InspectorSymbols.image(InspectorSymbols.readOnly, label: "Read Only")
            messageLabel.stringValue = reason
            actionButton.isHidden = true
            setAccessibilityLabel("Read Only")
            setAccessibilityValue(reason)
        }
    }

    @objc private func unlockPressed(_ sender: NSButton) {
        onUnlock?()
    }
}

/// 선택 없음·로딩·오류 상태 화면(가이드 7.1).
final class InspectorStateView: NSView {
    private let iconView = NSImageView()
    private let spinner = NSProgressIndicator()
    private let titleLabel = NSTextField(labelWithString: "")
    private let messageLabel = InspectorWrappingLabel(text: "")
    let retryButton = NSButton()
    /// 다시 시도 요청.
    var onRetry: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
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
        messageLabel.alignment = .center
        retryButton.title = "Try Again"
        retryButton.bezelStyle = .push
        retryButton.target = self
        retryButton.action = #selector(retryPressed(_:))
        retryButton.identifier = NSUserInterfaceItemIdentifier("state.retry")

        let stack = NSStackView(views: [iconView, spinner, titleLabel, messageLabel, retryButton])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = InspectorMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: InspectorMetrics.panelInset),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -InspectorMetrics.panelInset),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stack.topAnchor.constraint(greaterThanOrEqualTo: topAnchor),
            messageLabel.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorStateView는 코드로만 생성합니다")
    }

    func show(_ state: InspectorContentState) {
        iconView.isHidden = true
        spinner.stopAnimation(nil)
        spinner.isHidden = true
        retryButton.isHidden = true
        messageLabel.isHidden = false
        switch state {
        case .noSelection:
            iconView.isHidden = false
            iconView.image = InspectorSymbols.image(InspectorSymbols.noSelection, label: "No Selection")
            titleLabel.stringValue = "No Selection"
            messageLabel.stringValue = "Select a layer on the canvas or in the layer list to see its properties."
        case .loading:
            spinner.isHidden = false
            spinner.startAnimation(nil)
            titleLabel.stringValue = "Loading Properties…"
            messageLabel.isHidden = true
        case .failed(let message):
            iconView.isHidden = false
            iconView.image = InspectorSymbols.image(InspectorSymbols.failure, label: "Error")
            titleLabel.stringValue = "Can’t Show Properties"
            messageLabel.stringValue = message
            retryButton.isHidden = false
        case .ready:
            break
        }
        setAccessibilityLabel(titleLabel.stringValue)
        setAccessibilityValue(messageLabel.isHidden ? nil : messageLabel.stringValue)
    }

    @objc private func retryPressed(_ sender: NSButton) {
        onRetry?()
    }
}

/// 위에서 아래로 쌓는 스크롤 문서 뷰.
final class InspectorFlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// 인스펙터 스크롤 안에 넣는 고정 높이 목록용 scroll view. 스크롤 휠은 바깥 인스펙터로 넘깁니다.
final class InspectorEmbeddedScrollView: NSScrollView {
    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }
}
