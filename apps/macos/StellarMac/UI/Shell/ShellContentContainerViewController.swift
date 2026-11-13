import AppKit

/// 가운데 콘텐츠 슬롯을 담고, 창 상태가 `ready`가 아니면 빈·로딩·오류 상태 화면을 보여 줍니다.
///
/// 상태 화면은 표준 AppKit 컨트롤(이미지 뷰, 레이블, 진행 표시기, 버튼)로만 만들고 배경을 직접
/// 칠하지 않습니다. 콘텐츠 계층이므로 glass를 쓰지 않습니다(가이드 3.1, 5.1, 7.1).
final class ShellContentContainerViewController: NSViewController {
    /// 담고 있는 콘텐츠 슬롯.
    let contentSlot: any ShellContentSlot
    /// 상태 화면 버튼이 눌리면 호출합니다. 셸 창 controller가 코어 명령으로 바꿉니다.
    var actionHandler: ((ShellCommandID) -> Void)?

    /// 마지막으로 반영한 상태.
    private(set) var status: ShellFrameStatus = .ready

    /// 상태 화면 전체(검사·후속 연결용).
    let statusView = NSView()
    let statusImageView = NSImageView()
    let progressIndicator = NSProgressIndicator()
    let statusTitleField = NSTextField(wrappingLabelWithString: "")
    let statusMessageField = NSTextField(wrappingLabelWithString: "")
    private let actionStack = NSStackView()
    /// 현재 상태 화면의 동작 버튼. 순서는 상태의 동작 배열과 같습니다.
    private(set) var actionButtons: [NSButton] = []
    private var currentActions: [ShellActionDescriptor] = []

    init(contentSlot: any ShellContentSlot) {
        self.contentSlot = contentSlot
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellContentContainerViewController는 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView()

        // 콘텐츠 슬롯은 자기 safe area를 스스로 처리하므로 가장자리까지 붙입니다.
        let content = contentSlot.paneViewController
        addChild(content)
        content.view.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(content.view)

        configureStatusView()
        statusView.translatesAutoresizingMaskIntoConstraints = false
        statusView.isHidden = true
        root.addSubview(statusView)

        let safeArea = root.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            content.view.topAnchor.constraint(equalTo: root.topAnchor),
            content.view.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            content.view.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            content.view.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            statusView.topAnchor.constraint(equalTo: safeArea.topAnchor),
            statusView.bottomAnchor.constraint(equalTo: safeArea.bottomAnchor),
            statusView.leadingAnchor.constraint(equalTo: safeArea.leadingAnchor),
            statusView.trailingAnchor.constraint(equalTo: safeArea.trailingAnchor),
        ])
        view = root
    }

    /// 창 상태를 반영합니다. 같은 상태면 아무것도 하지 않습니다(가이드 9장 "보이는 것만 갱신").
    func apply(status newStatus: ShellFrameStatus) {
        loadViewIfNeeded()
        guard newStatus != status else { return }
        status = newStatus

        let contentView = contentSlot.paneViewController.view
        switch newStatus {
        case .ready:
            contentView.isHidden = false
            statusView.isHidden = true
            progressIndicator.stopAnimation(nil)
            setActions([])
        case .empty(let title, let message, let actions):
            showStatus(symbolName: ShellFrameSymbols.emptyStatus, title: title, message: message, actions: actions)
        case .loading(let message):
            showStatus(symbolName: nil, title: message, message: "", actions: [])
            progressIndicator.isHidden = false
            progressIndicator.setAccessibilityLabel(message)
            progressIndicator.startAnimation(nil)
        case .failed(let title, let message, let recoveryActions):
            showStatus(
                symbolName: ShellFrameSymbols.failedStatus, title: title, message: message, actions: recoveryActions
            )
        }
    }

    // MARK: - 상태 화면 구성

    private func configureStatusView() {
        statusView.setAccessibilityElement(true)
        statusView.setAccessibilityRole(.group)

        statusImageView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title1, scale: .large)
        statusImageView.contentTintColor = .secondaryLabelColor
        // 장식 이미지이므로 접근성 트리에서 숨기고, 의미는 제목 레이블이 전달합니다(가이드 8.3).
        statusImageView.setAccessibilityElement(false)

        progressIndicator.style = .spinning
        progressIndicator.isIndeterminate = true
        progressIndicator.isDisplayedWhenStopped = false
        progressIndicator.controlSize = .regular

        for (field, style, color) in [
            (statusTitleField, NSFont.TextStyle.title3, NSColor.labelColor),
            (statusMessageField, NSFont.TextStyle.body, NSColor.secondaryLabelColor),
        ] {
            field.font = .preferredFont(forTextStyle: style)
            field.textColor = color
            field.alignment = .center
            field.isSelectable = false
            field.preferredMaxLayoutWidth = ShellFrameMetrics.statusTextMaxWidth
        }

        actionStack.orientation = .horizontal
        actionStack.distribution = .fill
        actionStack.spacing = ShellFrameMetrics.statusStackSpacing
        actionStack.isHidden = true

        let stack = NSStackView(views: [
            statusImageView, progressIndicator, statusTitleField, statusMessageField, actionStack,
        ])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = ShellFrameMetrics.statusStackSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        statusView.addSubview(stack)

        // 가운데 정렬 내용이라 창 모서리에 닿지 않으므로 모서리 적응 없이 margins 영역을 씁니다.
        // 모서리 적응 영역은 pane 이동 애니메이션 중 제약 갱신을 반복시킬 수 있습니다(명세 참고).
        let margins = statusView.layoutGuide(for: .margins())
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: margins.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: margins.centerYAnchor),
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: margins.leadingAnchor),
            stack.topAnchor.constraint(greaterThanOrEqualTo: margins.topAnchor),
        ])
    }

    private func showStatus(symbolName: String?, title: String, message: String, actions: [ShellActionDescriptor]) {
        contentSlot.paneViewController.view.isHidden = true
        statusView.isHidden = false
        progressIndicator.stopAnimation(nil)
        progressIndicator.isHidden = true

        if let symbolName, let image = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil) {
            statusImageView.image = image
            statusImageView.isHidden = false
        } else {
            statusImageView.image = nil
            statusImageView.isHidden = true
        }
        statusTitleField.stringValue = title
        statusMessageField.stringValue = message
        statusMessageField.isHidden = message.isEmpty
        statusView.setAccessibilityLabel(title)
        setActions(actions)
    }

    private func setActions(_ actions: [ShellActionDescriptor]) {
        actionStack.isHidden = actions.isEmpty
        guard actions != currentActions else { return }
        currentActions = actions
        for button in actionButtons {
            actionStack.removeArrangedSubview(button)
            button.removeFromSuperview()
        }
        var primaryAssigned = false
        actionButtons = actions.enumerated().map { index, action in
            let button = NSButton(title: action.title, target: self, action: #selector(actionButtonPressed(_:)))
            button.tag = index
            button.isEnabled = action.isEnabled
            // 대표 동작 하나만 Return 키 등가로 지정해 시스템이 강조하게 합니다(가이드 5.1).
            if action.isPrimary, !primaryAssigned {
                button.keyEquivalent = "\r"
                primaryAssigned = true
            }
            return button
        }
        for button in actionButtons {
            actionStack.addArrangedSubview(button)
        }
    }

    @objc private func actionButtonPressed(_ sender: NSButton) {
        guard currentActions.indices.contains(sender.tag) else { return }
        actionHandler?(currentActions[sender.tag].id)
    }
}
