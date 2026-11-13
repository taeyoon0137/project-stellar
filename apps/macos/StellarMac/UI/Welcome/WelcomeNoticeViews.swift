import AppKit

/// 시스템 Git이 없거나 쓸 수 없을 때의 안내(아키텍처 §24.1).
///
/// 설치를 실행하지 않습니다. 사용자가 Terminal에서 실행할 명령을 보여 주고 복사만 하게 하며,
/// 다시 검사하거나 Git 실행 파일을 직접 지정(탐색 순서 1: 사용자 지정 경로)할 수 있게 합니다.
final class WelcomeGitNoticeView: NSView {
    var onCheckAgain: (() -> Void)?
    var onLocateGit: (() -> Void)?

    let iconView = WelcomeViews.symbolView(
        "exclamationmark.triangle.fill",
        style: .headline,
        multicolor: true
    )
    let titleLabel = WelcomeViews.label("", style: .headline)
    let messageLabel = WelcomeViews.wrappingLabel("", style: .callout, color: .secondaryLabelColor)
    let commandLabel = WelcomeViews.codeLabel(WelcomeStrings.gitInstallCommand)
    let copyButton = WelcomeCopyButton(accessibilityLabel: WelcomeStrings.copyCommand)
    let commandRow = NSStackView()
    let checkAgainButton = WelcomeViews.button(WelcomeStrings.checkAgain, target: nil, action: nil)
    let locateButton = WelcomeViews.button(WelcomeStrings.locateGit, target: nil, action: nil)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        copyButton.textToCopy = WelcomeStrings.gitInstallCommand
        checkAgainButton.target = self
        checkAgainButton.action = #selector(checkAgain(_:))
        locateButton.target = self
        locateButton.action = #selector(locateGit(_:))
        checkAgainButton.controlSize = .small
        locateButton.controlSize = .small

        // 가로 묶음은 내용 폭에 맞춥니다(hugging). 이미지에는 baseline이 없으므로 세로 가운데 정렬합니다.
        let header = WelcomeViews.row([iconView, titleLabel])
        for view in [commandLabel, copyButton] as [NSView] { commandRow.addArrangedSubview(view) }
        WelcomeViews.configureRow(commandRow)
        let buttons = WelcomeViews.row([checkAgainButton, locateButton])

        let stack = NSStackView(views: [header, messageLabel, commandRow, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = WelcomeMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            messageLabel.widthAnchor.constraint(equalTo: stack.widthAnchor),
            commandRow.widthAnchor.constraint(lessThanOrEqualTo: stack.widthAnchor),
        ])

        // 안내 전체를 하나의 묶음으로 노출하고, 안의 버튼은 그대로 접근할 수 있게 둡니다.
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeGitNoticeView는 코드로만 생성합니다")
    }

    /// Git 상태를 반영합니다. 안내가 필요 없으면 false를 돌려줍니다.
    @discardableResult
    func apply(_ state: WelcomeGitState) -> Bool {
        switch state {
        case .checking, .available:
            return false
        case .notFound:
            titleLabel.stringValue = WelcomeStrings.gitNotFoundTitle
            messageLabel.stringValue = WelcomeStrings.gitNotFoundMessage
            commandRow.isHidden = false
        case .unusable(let detail):
            titleLabel.stringValue = WelcomeStrings.gitUnusableTitle
            messageLabel.stringValue = detail
            commandRow.isHidden = true
        }
        setAccessibilityLabel(titleLabel.stringValue)
        return true
    }

    @objc private func checkAgain(_ sender: Any?) {
        onCheckAgain?()
    }

    @objc private func locateGit(_ sender: Any?) {
        onLocateGit?()
    }
}

/// 진행 중인 생성·열기·Clone 작업 표시.
final class WelcomeActivityView: NSView {
    var onCancel: (() -> Void)?

    let titleLabel = WelcomeViews.label("", style: .callout)
    let progressIndicator = NSProgressIndicator()
    let cancelButton = WelcomeViews.button(WelcomeStrings.cancel, target: nil, action: nil)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        progressIndicator.style = .bar
        progressIndicator.controlSize = .small
        progressIndicator.minValue = 0
        progressIndicator.maxValue = 1
        progressIndicator.setContentHuggingPriority(.defaultLow, for: .horizontal)
        cancelButton.controlSize = .small
        cancelButton.target = self
        cancelButton.action = #selector(cancel(_:))

        let row = NSStackView(views: [progressIndicator, cancelButton])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = WelcomeMetrics.rowSpacing

        let stack = NSStackView(views: [titleLabel, row])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = WelcomeMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.widthAnchor.constraint(equalTo: stack.widthAnchor),
            titleLabel.widthAnchor.constraint(lessThanOrEqualTo: stack.widthAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeActivityView는 코드로만 생성합니다")
    }

    func apply(_ activity: WelcomeActivity) {
        let title = WelcomeStrings.activity(activity.kind, name: activity.projectName)
        titleLabel.stringValue = title
        setAccessibilityLabel(title)
        if let fraction = activity.fractionCompleted {
            progressIndicator.isIndeterminate = false
            progressIndicator.doubleValue = min(max(fraction, 0), 1)
            progressIndicator.stopAnimation(nil)
        } else {
            progressIndicator.isIndeterminate = true
            progressIndicator.startAnimation(nil)
        }
        cancelButton.isHidden = !activity.isCancellable
    }

    func stop() {
        progressIndicator.stopAnimation(nil)
    }

    @objc private func cancel(_ sender: Any?) {
        onCancel?()
    }
}
