import AppKit

/// 에이전트 패널의 잠정 수치. 한곳에서만 정의합니다(macOS 셸 디자인 가이드 4.4).
///
/// 값은 HIG Layout 기준의 잠정값이며, 통합 WBS에서 측정해 확정하고 가이드 12장에 기록해야 합니다.
enum AgentPanelMetrics {
    /// 패널 바깥 여백.
    static let edgeInset: CGFloat = 12
    /// 행 안 요소 간격.
    static let itemSpacing: CGFloat = 6
    /// 행 위아래 여백.
    static let rowVerticalPadding: CGFloat = 6
    /// 아이콘 열 폭(본문 정렬용).
    static let symbolColumnWidth: CGFloat = 18
    /// 카드 모서리 반경. 동심 모서리 값은 통합 시 정합니다(가이드 3.5).
    static let cardCornerRadius: CGFloat = 8
    static let cardBorderWidth: CGFloat = 1
    /// 대비 높이기에서 구분선을 더 강하게 그립니다(가이드 8.4).
    static let cardBorderWidthIncreasedContrast: CGFloat = 2
    static let cardContentInset: CGFloat = 10
    /// 입력창 최소·최대 줄 수.
    static let composerMinimumLines: CGFloat = 2
    static let composerMaximumLines: CGFloat = 8
    /// 입력창 높이를 계산할 때 전체 레이아웃을 강제하는 최대 길이(UTF-16). 넘으면 최대 높이를 씁니다.
    static let composerMeasureLimit = 4096
    /// 한 번의 런 루프에서 처리할 최대 이벤트 수(아키텍처 §6.3 UI 스레드 점유 상한 보호).
    static let maxEventsPerPass = 64
    /// 아직 그리지 않은 대화 기록 행의 높이 추정값.
    static let estimatedRowHeight: CGFloat = 60
    /// 바닥에 붙어 있다고 판단하는 스크롤 여유.
    static let stickToBottomTolerance: CGFloat = 24
}

/// 에이전트 패널에서 쓰는 SF Symbols 이름. 이미지 asset은 쓰지 않습니다.
enum AgentPanelSymbol {
    static let more = "ellipsis.circle"
    static let send = "arrow.up"
    static let stop = "stop.fill"
    static let user = "person.crop.circle"
    static let agent = "sparkles"
    static let notice = "info.circle"
    static let toolRunning = "clock"
    static let toolSucceeded = "checkmark.circle"
    static let toolFailed = "xmark.octagon"
    static let toolRejected = "hand.raised"
    static let approval = "checkmark.shield"
    static let diffAdded = "plus.circle"
    static let diffRemoved = "minus.circle"
    static let diffModified = "pencil.circle"
    static let validationPassed = "checkmark.seal"
    static let validationWarning = "exclamationmark.triangle"
    static let validationError = "xmark.octagon"
    static let lockScope = "lock"
    static let agentNote = "quote.bubble"
    static let approved = "checkmark.circle"
    static let declined = "xmark.circle"
    static let expired = "clock.badge.xmark"
    static let applied = "sparkles"
    static let reverted = "arrow.uturn.backward.circle"
    static let revertUnavailable = "exclamationmark.circle"
    static let connected = "link"
    static let connecting = "arrow.triangle.2.circlepath"
    static let noSession = "circle.dashed"
    static let ended = "stop.circle"
    static let authenticationRequired = "person.crop.circle.badge.exclamationmark"
    static let unsupportedVersion = "exclamationmark.triangle"
    static let noCLI = "terminal"
    static let untrusted = "lock.shield"
    static let failed = "exclamationmark.octagon"
    static let startSession = "play.circle"

    /// 검사용 전체 목록.
    static let all: [String] = [
        more, send, stop, user, agent, notice, toolRunning, toolSucceeded, toolFailed, toolRejected, approval,
        diffAdded, diffRemoved, diffModified, validationPassed, validationWarning, validationError, lockScope,
        agentNote, approved, declined, expired, applied, reverted, revertUnavailable, connected, connecting,
        noSession, ended, authenticationRequired, unsupportedVersion, noCLI, untrusted, failed, startSession,
    ]
}

/// 패널 UI 문구. 지역화 전까지 한곳에 모읍니다. 프로젝트 데이터는 인자로 받은 표시 값만 씁니다.
enum AgentPanelText {
    static let panelTitle = "Agent"
    static let chatView = "Chat"
    static let rawTextView = "Raw Text"
    static let conversationViewLabel = "Conversation View"
    static let cliPopUpLabel = "Agent CLI"
    static let noCLIItem = "No CLI Found"
    static let modePopUpLabel = "Agent Mode"
    static let moreActions = "More Agent Actions"
    static let conversationLabel = "Agent Conversation"
    static let rawOutputLabel = "Agent Raw Output"
    static let composerLabel = "Message to Agent"
    static let composerPlaceholderActive = "Describe the change you want"
    static let composerPlaceholderInactive = "Start a session to send messages"
    static let composerPlaceholderWorking = "The agent is working…"
    static let send = "Send"
    static let sendToolTip = "Send (Return or ⌘Return)"
    static let stop = "Stop Session"
    static let stopToolTip = "Stop Session (⌘.)"
    static let startSessionMenu = "Start Session…"
    static let stopSessionMenu = "Stop Session"
    static let revertAllMenu = "Revert All Agent Changes in Session…"
    static let releaseLeasesMenu = "Release All Agent Leases in Project…"
    static let copy = "Copy"
    static let you = "You"
    static let agent = "Agent"
    static let agentResponding = "Agent — responding"
    static let stellar = "Stellar"
    static let stellarRecord = "Stellar record"
    static let approvalNeeded = "Approval Needed"
    static let changesSection = "Changes"
    static let validationSection = "Validation"
    static let lockScopeSection = "Lock Scope"
    static let agentNoteSection = "Agent’s Note"
    static let agentNoteProvenance = "Provided by the agent. Stellar didn’t check it."
    static let approve = "Approve"
    static let approveSensitive = "Approve…"
    static let decline = "Decline"
    static let preview = "Preview"
    static let previewToolTip = "Preview on Canvas"
    static let appliedByAgent = "Applied by agent"
    static let revertedLabel = "Reverted"
    static let revertUnavailableLabel = "Can’t revert automatically — later edits overlap this change"
    static let revert = "Revert"
    static let revertToolTip = "Revert This Agent Change"
    static let revertAll = "Revert All…"
    static let startNewSession = "Start New Session…"
    static let sessionEndedBar = "Session ended. Agent changes stay applied and marked."
    static let modelToolTip = "Model reported by the CLI"

    // 빈·진행·오류 상태(명세 4장).
    static let checkAgain = "Check Again"
    static let noCLITitle = "No agent CLI found"
    static let noCLIMessage = "Install a supported coding agent CLI and sign in with it using its own instructions. "
        + "Stellar runs the CLI you already use and doesn’t store keys or sign-in data."
    static let unsupportedVersionTitle = "This CLI version isn’t supported"
    static let authenticationRequiredMessage = "Sign in with the CLI’s own instructions. Stellar doesn’t take part in sign-in."
    static let openSignInTerminal = "Open Sign-In Terminal…"
    static let untrustedTitle = "Trust this project to use the agent"
    static let untrustedMessage = "Agent sessions start only in trusted projects."
    static let reviewProjectTrust = "Review Project Trust…"
    static let startSessionTitle = "Edit with your agent CLI"

    static func unsupportedVersionMessage(_ cliName: String) -> String {
        "Stellar runs only CLI versions it has verified. Update \(cliName), then check again."
    }

    static func authenticationRequiredTitle(_ cliName: String) -> String { "Sign in to \(cliName)" }

    static func startSessionMessage(_ cliName: String) -> String {
        "Start a session to ask \(cliName) to inspect this project and propose edits. "
            + "Changes the agent applies stay marked so you can revert them."
    }

    static func modelLabel(_ name: String) -> String { "Model: \(name)" }

    static func modeTitle(_ mode: AgentPermissionMode) -> String {
        switch mode {
        case .readOnly: return "Read Only"
        case .reviewBeforeApplying: return "Review Before Applying"
        case .applyWithinApprovedScope: return "Apply Within Approved Scope"
        }
    }

    static func cliItemTitle(_ option: AgentCLIOption) -> String {
        var title = option.displayName
        if let version = option.version { title += " \(version)" }
        switch option.availability {
        case .ready: break
        case .unsupportedVersion: title += " — Unsupported Version"
        case .authenticationRequired: title += " — Sign-In Required"
        }
        return title
    }

    static func connectionTitle(_ header: AgentSessionHeader) -> String {
        guard let cli = header.selectedCLI else { return "No agent CLI found" }
        switch cli.availability {
        case .unsupportedVersion: return "Unsupported CLI version"
        case .authenticationRequired: return "Sign-in required"
        case .ready: break
        }
        switch header.connection {
        case .noSession: return "No session"
        case .starting: return "Starting…"
        case .connected: return header.isAgentWorking ? "Connected — working" : "Connected"
        case .reconnecting: return "Reconnecting…"
        case .ended: return "Session ended"
        }
    }

    static func changeSummary(_ summary: AgentSessionChangeSummary) -> String {
        summary.appliedCount == 1
            ? "1 change applied by the agent"
            : "\(summary.appliedCount) changes applied by the agent"
    }

    static func approvalKindTitle(_ kind: AgentApprovalKind) -> String {
        switch kind {
        case .editBatch: return "Edit"
        case .gitCommit: return "Git Commit"
        case .gitPush: return "Git Push"
        }
    }

    static func approvalStatusTitle(_ status: AgentApprovalStatus) -> String {
        switch status {
        case .pending: return "Waiting for your decision"
        case .approved: return "Approved"
        case .declined: return "Declined"
        case .expired: return "Expired — treated as declined"
        }
    }

    static func toolStatusTitle(_ status: AgentToolStepStatus) -> String {
        switch status {
        case .running: return "Running"
        case .succeeded: return "Done"
        case .failed: return "Failed"
        case .rejected: return "Rejected"
        }
    }

    static func diffChangeTitle(_ change: AgentDiffLine.Change) -> String {
        switch change {
        case .added: return "Added"
        case .removed: return "Removed"
        case .modified: return "Modified"
        }
    }

    static func omittedDiffLines(_ count: Int) -> String {
        count == 1 ? "1 more change" : "\(count) more changes"
    }

    static func validationTitle(_ validation: AgentValidationSummary) -> String {
        if validation.errorCount == 0 && validation.warningCount == 0 { return "No issues" }
        var parts: [String] = []
        if validation.errorCount > 0 {
            parts.append(validation.errorCount == 1 ? "1 error" : "\(validation.errorCount) errors")
        }
        if validation.warningCount > 0 {
            parts.append(validation.warningCount == 1 ? "1 warning" : "\(validation.warningCount) warnings")
        }
        return parts.joined(separator: ", ")
    }

    static func noticeText(_ notice: AgentNotice) -> String {
        switch notice {
        case .sessionStarted(let cliName, let mode):
            return "Session started with \(cliName) in \(modeTitle(mode)) mode."
        case .modeChanged(let mode):
            return "Mode changed to \(modeTitle(mode))."
        case .sessionEnded(let kept):
            if kept == 0 { return "Session ended." }
            let changes = kept == 1 ? "1 agent change stays" : "\(kept) agent changes stay"
            return "Session ended. \(changes) applied and marked. Nothing was rolled back automatically."
        case .leasesReleased:
            return "All agent leases in this project were released."
        }
    }

    static func loadingTitle(_ reason: AgentLoadingReason) -> String {
        switch reason {
        case .detectingCLIs: return "Checking installed agent CLIs…"
        case .startingSession: return "Starting session…"
        }
    }

    static func failureTitle(_ reason: AgentFailureReason) -> String {
        switch reason {
        case .sessionEndedUnexpectedly: return "The session ended unexpectedly"
        case .cliLaunchRejected: return "Stellar didn’t start this CLI"
        case .connectionLost: return "Lost connection to the agent"
        }
    }

    static func failureMessage(_ reason: AgentFailureReason) -> String {
        switch reason {
        case .sessionEndedUnexpectedly, .connectionLost:
            return "Changes the agent already applied stay marked. You can revert them or start a new session."
        case .cliLaunchRejected:
            return "This CLI version hasn’t been verified with Stellar. Update the CLI, then check again."
        }
    }

    /// OS 알림 문구. 고정 문구만 쓰며 프로젝트 데이터를 넣지 않습니다(ADR-0005 결정 4.5, 아키텍처 §39).
    static func notificationTitle(_ kind: AgentAttentionKind) -> String {
        switch kind {
        case .approvalRequested: return "Approval Needed"
        case .sessionEnded: return "Agent Session Ended"
        case .sessionFailed: return "Agent Session Stopped"
        }
    }

    static func notificationBody(_ kind: AgentAttentionKind) -> String {
        switch kind {
        case .approvalRequested: return "An agent session is waiting for your review."
        case .sessionEnded: return "Open Stellar to review the agent’s changes."
        case .sessionFailed: return "Open Stellar to review the session."
        }
    }
}

/// 패널 공용 뷰 팩토리. 표준 AppKit 컨트롤과 시스템 의미 색만 씁니다(가이드 4.2, 5.1).
enum AgentPanelViews {
    static func symbol(_ name: String, description: String?) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: description)
    }

    /// 한 줄 레이블.
    static func label(font: NSFont, color: NSColor = .labelColor) -> NSTextField {
        let label = NSTextField(labelWithString: "")
        label.font = font
        label.textColor = color
        label.lineBreakMode = .byTruncatingTail
        label.translatesAutoresizingMaskIntoConstraints = false
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    /// 여러 줄 레이블. 폭은 제약이 정하고 높이는 내용에 맞춥니다.
    static func wrappingLabel(font: NSFont, color: NSColor = .labelColor) -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: "")
        label.font = font
        label.textColor = color
        label.isSelectable = false
        label.translatesAutoresizingMaskIntoConstraints = false
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        label.setContentHuggingPriority(.defaultLow, for: .horizontal)
        label.setContentCompressionResistancePriority(.required, for: .vertical)
        return label
    }

    /// SF Symbol 이미지 뷰. 장식용이면 접근성 트리에서 뺍니다.
    static func symbolView(
        _ name: String,
        textStyle: NSFont.TextStyle = .body,
        tint: NSColor = .secondaryLabelColor
    ) -> NSImageView {
        let view = NSImageView()
        view.translatesAutoresizingMaskIntoConstraints = false
        view.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: textStyle, scale: .medium)
        view.image = symbol(name, description: nil)
        view.contentTintColor = tint
        view.setAccessibilityElement(false)
        view.setContentHuggingPriority(.required, for: .horizontal)
        view.setContentCompressionResistancePriority(.required, for: .horizontal)
        return view
    }

    /// 텍스트 버튼. 키 등가는 지정하지 않습니다(승인·되돌리기는 기본 키로 실행되지 않음).
    static func textButton(_ title: String, target: AnyObject?, action: Selector?) -> NSButton {
        let button = NSButton(title: title, target: target, action: action)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.controlSize = .small
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        return button
    }

    /// 아이콘만 있는 버튼. 도움말 태그와 접근성 레이블을 반드시 붙입니다(가이드 8.1, 8.3).
    static func iconButton(
        symbolName: String,
        label: String,
        toolTip: String,
        target: AnyObject?,
        action: Selector?
    ) -> NSButton {
        let image = symbol(symbolName, description: label) ?? NSImage()
        let button = NSButton(image: image, target: target, action: action)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.imagePosition = .imageOnly
        button.toolTip = toolTip
        button.setAccessibilityLabel(label)
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        return button
    }

    static func font(_ style: NSFont.TextStyle) -> NSFont {
        NSFont.preferredFont(forTextStyle: style)
    }

    /// 세로 stack을 만듭니다. 폭 채우기는 `setArrangedSubviews(_:in:fill:)`가 제약으로 겁니다.
    static func verticalStack(spacing: CGFloat, alignment: NSLayoutConstraint.Attribute = .leading) -> NSStackView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = alignment
        stack.spacing = spacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        return stack
    }

    /// 세로 stack의 배치 뷰를 다시 구성하고, `fill`이 true인 뷰는 stack 폭을 채우게 합니다.
    ///
    /// 숨김으로 표시를 바꾸면 NSStackView가 뷰를 떼어 내며 바깥 제약이 사라지므로,
    /// 표시 여부가 바뀌는 항목은 숨기지 않고 이 함수로 배치 목록 자체를 바꿉니다.
    static func setArrangedSubviews(
        _ views: [NSView], in stack: NSStackView, fill: (NSView) -> Bool = { _ in true }
    ) {
        for view in stack.arrangedSubviews {
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        for view in views {
            view.isHidden = false
            stack.addArrangedSubview(view)
            if fill(view) {
                view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
            }
        }
    }

    /// 시스템 접근성 표시 설정을 읽습니다(가이드 8.4).
    static var reduceTransparency: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency }
    static var increaseContrast: Bool { NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast }
    static var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
}
