import AppKit

/// 대화 입력창과 보내기·중지 컨트롤(아키텍처 §11.2, ADR-0005 결정 4.5).
///
/// 입력은 AppKit `NSTextView`이므로 IME, 맞춤법, 텍스트 서비스, 네이티브 Undo가 시스템과 같게 동작합니다.
/// - Return: 보내기(IME 조합 중에는 조합 확정이 우선)
/// - Shift-Return, Option-Return: 줄바꿈
/// - Command-Return: 보내기(버튼 키 등가)
/// - Command-Period: 세션 중지(버튼 키 등가)
final class AgentComposerView: NSView, NSTextViewDelegate {
    let scrollView: NSScrollView
    let textView: NSTextView
    /// 입력이 비어 있을 때 보이는 안내. 시각 전용이며 접근성은 text view의 placeholder 값으로 노출합니다.
    let placeholderLabel = AgentPanelViews.label(font: AgentPanelViews.font(.body), color: .placeholderTextColor)
    private(set) var sendButton: NSButton!
    private(set) var stopButton: NSButton!

    /// 보내기 요청. 공백만 있는 문장은 보내지 않습니다.
    var onSend: ((String) -> Void)?
    var onStop: (() -> Void)?

    /// 현재 보낼 수 있는 상태인지(연결됨, 작업 중 아님).
    var canSend = false {
        didSet { updateControls() }
    }

    var canStop = false {
        didSet { updateControls() }
    }

    var placeholder: String = AgentPanelText.composerPlaceholderActive {
        didSet {
            placeholderLabel.stringValue = placeholder
            textView.setAccessibilityPlaceholderValue(placeholder)
        }
    }

    private var heightConstraint: NSLayoutConstraint!
    private let font = AgentPanelViews.font(.body)

    override init(frame frameRect: NSRect) {
        let scroll = NSTextView.scrollableTextView()
        scrollView = scroll
        // scrollableTextView()는 항상 NSTextView를 documentView로 만듭니다.
        textView = (scroll.documentView as? NSTextView) ?? NSTextView()
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false

        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.borderType = .bezelBorder
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        textView.isRichText = false
        textView.importsGraphics = false
        textView.isEditable = true
        textView.isSelectable = true
        textView.allowsUndo = true
        textView.font = font
        textView.textColor = .textColor
        textView.delegate = self
        textView.setAccessibilityLabel(AgentPanelText.composerLabel)
        textView.setAccessibilityPlaceholderValue(placeholder)

        placeholderLabel.stringValue = placeholder
        placeholderLabel.setAccessibilityElement(false)

        sendButton = AgentPanelViews.iconButton(
            symbolName: AgentPanelSymbol.send, label: AgentPanelText.send, toolTip: AgentPanelText.sendToolTip,
            target: self, action: #selector(sendPressed(_:)))
        sendButton.keyEquivalent = "\r"
        sendButton.keyEquivalentModifierMask = [.command]
        stopButton = AgentPanelViews.iconButton(
            symbolName: AgentPanelSymbol.stop, label: AgentPanelText.stop, toolTip: AgentPanelText.stopToolTip,
            target: self, action: #selector(stopPressed(_:)))
        stopButton.keyEquivalent = "."
        stopButton.keyEquivalentModifierMask = [.command]

        let buttons = NSStackView(views: [stopButton, sendButton])
        buttons.orientation = .horizontal
        buttons.spacing = AgentPanelMetrics.itemSpacing
        buttons.translatesAutoresizingMaskIntoConstraints = false

        addSubview(scrollView)
        addSubview(placeholderLabel)
        addSubview(buttons)

        let inset = AgentPanelMetrics.edgeInset
        heightConstraint = scrollView.heightAnchor.constraint(equalToConstant: height(forLines: AgentPanelMetrics.composerMinimumLines))
        let textInset = textView.textContainerInset
        let padding = textView.textContainer?.lineFragmentPadding ?? 0
        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset),
            scrollView.topAnchor.constraint(equalTo: topAnchor, constant: AgentPanelMetrics.itemSpacing),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -inset),
            heightConstraint,
            buttons.leadingAnchor.constraint(equalTo: scrollView.trailingAnchor, constant: AgentPanelMetrics.itemSpacing),
            buttons.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset),
            buttons.bottomAnchor.constraint(equalTo: scrollView.bottomAnchor),
            placeholderLabel.leadingAnchor.constraint(
                equalTo: scrollView.leadingAnchor, constant: textInset.width + padding + 1),
            placeholderLabel.trailingAnchor.constraint(lessThanOrEqualTo: scrollView.trailingAnchor, constant: -inset),
            placeholderLabel.topAnchor.constraint(equalTo: scrollView.topAnchor, constant: textInset.height + 1),
        ])
        updateControls()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("AgentComposerView는 코드로만 생성합니다")
    }

    /// 보내기 대상 문장. 앞뒤 공백을 뺀 값입니다.
    var trimmedText: String {
        textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func clear() {
        textView.string = ""
        textDidChange(Notification(name: NSText.didChangeNotification, object: textView))
    }

    private func lineHeight() -> CGFloat {
        ceil(font.ascender - font.descender + font.leading)
    }

    private func height(forLines lines: CGFloat) -> CGFloat {
        // bezel 테두리(위아래)와 text container inset을 포함합니다.
        lineHeight() * lines + textView.textContainerInset.height * 2 + 2
    }

    private func updateControls() {
        let hasText = !trimmedText.isEmpty
        sendButton.isEnabled = canSend && hasText
        stopButton.isEnabled = canStop
        placeholderLabel.isHidden = !textView.string.isEmpty || textView.hasMarkedText()
    }

    /// 입력 높이를 최소~최대 줄 사이에서 내용에 맞춥니다. 큰 입력은 전체 레이아웃을 강제하지 않습니다(§6.3).
    private func updateHeight() {
        let minimum = height(forLines: AgentPanelMetrics.composerMinimumLines)
        let maximum = height(forLines: AgentPanelMetrics.composerMaximumLines)
        var target = maximum
        if textView.string.utf16.count <= AgentPanelMetrics.composerMeasureLimit,
            let layoutManager = textView.textLayoutManager
        {
            layoutManager.ensureLayout(for: layoutManager.documentRange)
            let used = layoutManager.usageBoundsForTextContainer.height
            target = used + textView.textContainerInset.height * 2 + 2
        }
        let clamped = min(max(target, minimum), maximum)
        if abs(heightConstraint.constant - clamped) > 0.5 {
            heightConstraint.constant = clamped
        }
    }

    // MARK: NSTextViewDelegate

    func textDidChange(_ notification: Notification) {
        updateControls()
        updateHeight()
    }

    func textView(_ textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.insertNewline(_:)) else { return false }
        // IME 조합 중 Return은 조합 확정에 씁니다.
        if textView.hasMarkedText() { return false }
        let flags = NSApp.currentEvent?.modifierFlags.intersection(.deviceIndependentFlagsMask) ?? []
        if flags.contains(.shift) || flags.contains(.option) {
            textView.insertNewlineIgnoringFieldEditor(nil)
            return true
        }
        submit()
        return true
    }

    // MARK: Actions

    /// 보낼 수 있으면 문장을 넘기고 입력을 비웁니다. 보냈으면 true.
    @discardableResult
    func submit() -> Bool {
        let text = trimmedText
        guard canSend, !text.isEmpty else {
            NSSound.beep()
            return false
        }
        onSend?(text)
        clear()
        return true
    }

    @objc private func sendPressed(_ sender: Any?) {
        submit()
    }

    @objc private func stopPressed(_ sender: Any?) {
        guard canStop else { return }
        onStop?()
    }
}
