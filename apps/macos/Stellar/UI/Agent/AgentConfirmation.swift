import AppKit

/// 에이전트 패널의 확인 화면 내용. 네이티브 alert(창 sheet)로만 보여 줍니다(ADR-0005 결정 4.5).
struct AgentConfirmation: Equatable {
    enum Kind: Equatable {
        /// 세션 시작 확인. 첫 연결 승인으로 기록됩니다.
        case startSession
        /// 민감 확인(GitCommit, GitPush).
        case sensitiveApproval(AgentApprovalKind)
        case revertAllSessionChanges
        case releaseAllLeases
    }

    let kind: Kind
    let messageText: String
    let informativeText: String
    let confirmTitle: String
    /// 파괴적 동작이면 확인 버튼을 기본(Return) 버튼으로 두지 않습니다(가이드 5.1).
    let isDestructive: Bool

    static let cancelTitle = "Cancel"

    /// 세션 시작 확인: 대상 프로젝트, 전송 대상 CLI, 모드를 표시합니다(ADR-0005 결정 4.5 "세션 시작 조건").
    static func startSession(projectName: String, cli: AgentCLIOption, mode: AgentPermissionMode) -> AgentConfirmation {
        AgentConfirmation(
            kind: .startSession,
            messageText: "Start an agent session?",
            informativeText: "Project: \(projectName)\nCLI: \(AgentPanelText.cliItemTitle(cli))\n"
                + "Mode: \(AgentPanelText.modeTitle(mode))\n\n"
                + "Messages you send are passed to this CLI as you write them. "
                + "The CLI uses your own subscription and sign-in.",
            confirmTitle: "Start Session",
            isDestructive: false)
    }

    static func sensitiveApproval(_ kind: AgentApprovalKind) -> AgentConfirmation {
        let action: String
        let confirm: String
        switch kind {
        case .gitCommit:
            action = "create a Git commit"
            confirm = "Commit"
        case .gitPush:
            action = "push to the remote repository"
            confirm = "Push"
        case .editBatch:
            action = "apply these edits"
            confirm = "Approve"
        }
        return AgentConfirmation(
            kind: .sensitiveApproval(kind),
            messageText: "Allow the agent to \(action)?",
            informativeText: "Review the changes and validation summary in the agent panel before you continue. "
                + "The agent’s note is provided by the agent and isn’t checked by Stellar.",
            confirmTitle: confirm,
            isDestructive: true)
    }

    static func revertAllSessionChanges(count: Int) -> AgentConfirmation {
        AgentConfirmation(
            kind: .revertAllSessionChanges,
            messageText: count == 1
                ? "Revert the agent’s change in this session?"
                : "Revert all \(count) agent changes in this session?",
            informativeText: "Only changes marked as applied by the agent are reverted. Your own edits stay as they are.",
            confirmTitle: "Revert All",
            isDestructive: true)
    }

    static func releaseAllLeases() -> AgentConfirmation {
        AgentConfirmation(
            kind: .releaseAllLeases,
            messageText: "Release all agent leases in this project?",
            informativeText: "Agent sessions connected to Stellar for this project end, and pending requests are declined. "
                + "Changes the agent already applied stay marked so you can revert them.",
            confirmTitle: "Release All",
            isDestructive: true)
    }
}

/// 확인 화면 표시 방식. 검사에서는 기록용 구현으로 바꿉니다.
protocol AgentConfirmationPresenting: AnyObject {
    /// 확인 결과를 completion으로 돌려줍니다. 표시할 창이 없으면 거부(false)로 끝냅니다(fail closed).
    func present(_ confirmation: AgentConfirmation, for window: NSWindow?, completion: @escaping (Bool) -> Void)
}

/// `NSAlert` 창 sheet로 확인을 받습니다.
final class AgentAlertConfirmationPresenter: AgentConfirmationPresenting {
    /// alert와 확인 버튼의 응답 값을 만듭니다.
    ///
    /// 파괴적 확인은 Cancel을 첫 버튼(Return 기본)으로 두고, 확인 버튼은 `hasDestructiveAction`과
    /// 빈 키 등가를 줍니다. 그래서 키보드 기본 동작만으로는 승인되지 않고, 보조 기술로는 누를 수 있습니다.
    static func makeAlert(for confirmation: AgentConfirmation) -> (alert: NSAlert, confirmResponse: NSApplication.ModalResponse) {
        let alert = NSAlert()
        alert.messageText = confirmation.messageText
        alert.informativeText = confirmation.informativeText
        if confirmation.isDestructive {
            alert.alertStyle = .warning
            let cancel = alert.addButton(withTitle: AgentConfirmation.cancelTitle)
            cancel.keyEquivalent = "\r"
            let confirm = alert.addButton(withTitle: confirmation.confirmTitle)
            confirm.hasDestructiveAction = true
            confirm.keyEquivalent = ""
            return (alert, .alertSecondButtonReturn)
        } else {
            alert.alertStyle = .informational
            alert.addButton(withTitle: confirmation.confirmTitle)
            let cancel = alert.addButton(withTitle: AgentConfirmation.cancelTitle)
            cancel.keyEquivalent = "\u{1b}"
            return (alert, .alertFirstButtonReturn)
        }
    }

    func present(_ confirmation: AgentConfirmation, for window: NSWindow?, completion: @escaping (Bool) -> Void) {
        guard let window else {
            completion(false)
            return
        }
        let (alert, confirmResponse) = Self.makeAlert(for: confirmation)
        alert.beginSheetModal(for: window) { response in
            completion(response == confirmResponse)
        }
    }
}
