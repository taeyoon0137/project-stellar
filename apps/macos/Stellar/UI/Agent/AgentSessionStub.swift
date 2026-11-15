import Foundation

#if DEBUG
/// 패널·창 닫기 검사에서 주입하는 샘플 데이터입니다.
///
/// CLI를 실행하지 않고, 메시지를 어디에도 보내지 않으며, 문서를 바꾸지 않습니다.
/// 명령을 받으면 기록하고, 코어가 보낼 법한 이벤트를 큐에 넣어 패널의 상태 전이를 흉내 냅니다.
/// 실제 구현은 Rust 코어(FFI pull 이벤트·스냅샷, 아키텍처 §8.2)가 같은 `AgentPanelSource` 계약으로 대체합니다.
final class AgentSessionStub: AgentPanelSource {
    enum Scenario: CaseIterable {
        case activeSession
        case noSession
        case noInstalledCLI
        case authenticationRequired
        case unsupportedVersion
        case projectNotTrusted
        case loading
        case failed
    }

    static let sampleProjectName = "Sample Project"
    static let primaryCLI = AgentCLIOption(
        id: AgentCLIID(rawValue: 1), displayName: "Agent CLI (Stub)", version: "1.0.0", availability: .ready)
    static let secondaryCLI = AgentCLIOption(
        id: AgentCLIID(rawValue: 2), displayName: "Second Agent CLI (Stub)", version: "0.9.0",
        availability: .authenticationRequired)
    static let sampleRawText = """
        (Stub) Raw CLI output is shown here as plain text, without interpretation.
        > Shorten the sign-in button label and align the form fields to the 8-point grid.
        I’ll shorten the button label and move the two fields onto the grid.
        The edits are applied. Should I commit them?

        """

    /// 받은 명령 기록(검사용).
    private(set) var performedCommands: [AgentPanelCommand] = []

    private var snapshot: AgentPanelSnapshot
    private var queue: [AgentPanelEvent] = []
    private var nextItemID: UInt64 = 1000
    private var nextChangeID: UInt64 = 500

    init(scenario: Scenario = .activeSession) {
        snapshot = Self.makeSnapshot(scenario)
    }

    // MARK: AgentPanelSource

    func currentSnapshot() -> AgentPanelSnapshot {
        snapshot
    }

    func pollEvent() -> AgentPanelEvent? {
        queue.isEmpty ? nil : queue.removeFirst()
    }

    func perform(_ command: AgentPanelCommand) {
        performedCommands.append(command)
        switch command {
        case .sendMessage(let text):
            guard snapshot.header.connection == .connected else { return }
            append([
                item(.userMessage(text)),
                item(.agentMessage(
                    "(Stub) No agent is connected in this build. Your message wasn’t sent anywhere.",
                    isStreaming: false)),
            ])
            emit(.rawTextAppended("> \(text)\n(Stub) Not sent.\n"))
        case .respondToApproval(let approvalID, let decision):
            respond(to: approvalID, decision: decision)
        case .revertChange(let changeID):
            revert { $0 == changeID }
        case .revertAllSessionChanges:
            revert { _ in true }
        case .stopSession:
            endSession(releasingLeases: false)
        case .releaseAllProjectLeases:
            endSession(releasingLeases: true)
        case .startSession(let cli, let mode):
            startSession(cli: cli, mode: mode)
        case .setMode(let mode):
            snapshot.header.mode = mode
            emit(.headerChanged(snapshot.header))
            if snapshot.header.isSessionActive { append([item(.notice(.modeChanged(mode)))]) }
        case .selectCLI(let id):
            snapshot.header.selectedCLIID = id
            emit(.headerChanged(snapshot.header))
        case .refreshInstalledCLIs:
            emit(.phaseChanged(.loading(.detectingCLIs)))
            snapshot.phase = .ready
            emit(.phaseChanged(.ready))
        case .openSignInTerminal, .reviewProjectTrust, .previewApproval:
            // 코어·셸 통합 전에는 기록만 합니다.
            break
        }
    }

    // MARK: 상태 전이

    private func emit(_ change: AgentPanelEvent.Change) {
        queue.append(AgentPanelEvent(generation: snapshot.generation, change: change))
    }

    private func item(_ content: AgentConversationItem.Content) -> AgentConversationItem {
        nextItemID += 1
        return AgentConversationItem(id: AgentItemID(rawValue: nextItemID), content: content)
    }

    private func append(_ items: [AgentConversationItem]) {
        snapshot.items.append(contentsOf: items)
        emit(.itemsAppended(items))
    }

    private func replace(_ item: AgentConversationItem) {
        guard let index = snapshot.items.firstIndex(where: { $0.id == item.id }) else { return }
        snapshot.items[index] = item
        emit(.itemUpdated(item))
    }

    private func recountChanges() {
        var applied = 0
        var revertable = 0
        for item in snapshot.items {
            guard case .appliedChange(let change) = item.content else { continue }
            applied += 1
            if change.state == .applied { revertable += 1 }
        }
        snapshot.changeSummary = AgentSessionChangeSummary(appliedCount: applied, revertableCount: revertable)
        emit(.changeSummaryChanged(snapshot.changeSummary))
    }

    private func respond(to approvalID: AgentApprovalID, decision: AgentApprovalDecision) {
        guard
            let existing = snapshot.items.first(where: {
                if case .approval(let request) = $0.content { return request.approvalID == approvalID }
                return false
            }),
            case .approval(var request) = existing.content,
            request.status == .pending
        else { return }
        // 첫 응답 하나만 유효합니다.
        request.status = decision == .approve ? .approved : .declined
        replace(AgentConversationItem(id: existing.id, content: .approval(request)))
        guard decision == .approve else { return }

        switch request.kind {
        case .editBatch:
            let changes = request.diffLines.map { line -> AgentConversationItem in
                nextChangeID += 1
                return item(.appliedChange(AgentAppliedChange(
                    changeID: AgentChangeID(rawValue: nextChangeID), summary: "(Stub) \(line.target)",
                    detail: line.detail, state: .applied)))
            }
            append(changes)
            recountChanges()
        case .gitCommit, .gitPush:
            append([item(.toolStep(AgentToolStep(
                title: AgentPanelText.approvalKindTitle(request.kind),
                detail: "(Stub) Nothing was sent to Git.", status: .succeeded)))])
        }
    }

    private func revert(where matches: (AgentChangeID) -> Bool) {
        for existing in snapshot.items {
            guard case .appliedChange(var change) = existing.content, change.state == .applied,
                matches(change.changeID)
            else { continue }
            change.state = .reverted
            replace(AgentConversationItem(id: existing.id, content: .appliedChange(change)))
        }
        recountChanges()
    }

    /// 세션 종료. 대기 중인 승인은 거부로 정리하고, 적용된 변경은 롤백하지 않고 표시한 채 둡니다.
    private func endSession(releasingLeases: Bool) {
        guard snapshot.header.isSessionActive else { return }
        for existing in snapshot.items {
            guard case .approval(var request) = existing.content, request.status == .pending else { continue }
            request.status = .expired
            replace(AgentConversationItem(id: existing.id, content: .approval(request)))
        }
        snapshot.header.connection = .ended
        snapshot.header.isAgentWorking = false
        emit(.headerChanged(snapshot.header))
        var notices = [item(.notice(.sessionEnded(keptChangeCount: snapshot.changeSummary.revertableCount)))]
        if releasingLeases { notices.append(item(.notice(.leasesReleased))) }
        append(notices)
        emit(.attentionRequested(.sessionEnded))
    }

    private func startSession(cli: AgentCLIID, mode: AgentPermissionMode) {
        guard !snapshot.header.isSessionActive,
            let option = snapshot.header.installedCLIs.first(where: { $0.id == cli }), option.availability == .ready
        else { return }
        var header = snapshot.header
        header.selectedCLIID = cli
        header.mode = mode
        header.connection = .connected
        header.isAgentWorking = false
        header.reportedModelName = "stub-model"
        snapshot = AgentPanelSnapshot(
            generation: snapshot.generation + 1, header: header, phase: .ready,
            items: [item(.notice(.sessionStarted(cliName: option.displayName, mode: mode)))],
            rawText: "", changeSummary: .none)
        emit(.reset(snapshot))
    }

    // MARK: 샘플 스냅샷

    static func makeSnapshot(_ scenario: Scenario) -> AgentPanelSnapshot {
        var header = AgentSessionHeader(
            projectDisplayName: sampleProjectName, projectTrusted: true,
            installedCLIs: [primaryCLI, secondaryCLI], selectedCLIID: primaryCLI.id,
            reportedModelName: "stub-model", connection: .noSession, mode: .reviewBeforeApplying,
            isAgentWorking: false)
        var phase = AgentPanelPhase.ready
        var items: [AgentConversationItem] = []
        var rawText = ""
        var summary = AgentSessionChangeSummary.none

        switch scenario {
        case .activeSession:
            header.connection = .connected
            items = sampleConversation()
            rawText = sampleRawText
            summary = AgentSessionChangeSummary(appliedCount: 3, revertableCount: 2)
        case .noSession:
            break
        case .noInstalledCLI:
            header.installedCLIs = []
            header.selectedCLIID = nil
            header.reportedModelName = nil
        case .authenticationRequired:
            header.selectedCLIID = secondaryCLI.id
            header.reportedModelName = nil
        case .unsupportedVersion:
            var option = primaryCLI
            option.availability = .unsupportedVersion
            header.installedCLIs = [option]
            header.reportedModelName = nil
        case .projectNotTrusted:
            header.projectTrusted = false
        case .loading:
            phase = .loading(.detectingCLIs)
        case .failed:
            phase = .failed(.cliLaunchRejected)
        }
        return AgentPanelSnapshot(
            generation: 1, header: header, phase: phase, items: items, rawText: rawText, changeSummary: summary)
    }

    private static func sampleConversation() -> [AgentConversationItem] {
        var nextID: UInt64 = 0
        func item(_ content: AgentConversationItem.Content) -> AgentConversationItem {
            nextID += 1
            return AgentConversationItem(id: AgentItemID(rawValue: nextID), content: content)
        }
        return [
            item(.notice(.sessionStarted(cliName: primaryCLI.displayName, mode: .reviewBeforeApplying))),
            item(.userMessage("Shorten the sign-in button label and align the form fields to the 8-point grid.")),
            item(.toolStep(AgentToolStep(
                title: "Inspect document structure", detail: "Read 14 nodes in “Sign In”.", status: .succeeded))),
            item(.agentMessage(
                "I’ll shorten the button label and move the two fields onto the grid. "
                    + "Here is the proposed batch.", isStreaming: false)),
            item(.toolStep(AgentToolStep(
                title: "Validate command batch", detail: "3 commands checked.", status: .succeeded))),
            item(.approval(AgentApprovalRequest(
                approvalID: AgentApprovalID(rawValue: 1), kind: .editBatch,
                summary: "Edit 3 properties in “Sign In”",
                diffLines: [
                    AgentDiffLine(
                        change: .modified, target: "Sign In / Button / Label",
                        detail: "text: “Sign in to your account” → “Sign in”"),
                    AgentDiffLine(change: .modified, target: "Sign In / Email Field", detail: "y: 117 → 120"),
                    AgentDiffLine(change: .modified, target: "Sign In / Password Field", detail: "y: 163 → 168"),
                ],
                omittedDiffLineCount: 0,
                validation: AgentValidationSummary(
                    errorCount: 0, warningCount: 1,
                    messages: ["The label text no longer matches the button’s accessibility label."]),
                lockScope: "“Sign In” frame and 12 descendants",
                agentExplanation: "A shorter label stays on one line at the narrowest width.",
                status: .approved))),
            item(.appliedChange(AgentAppliedChange(
                changeID: AgentChangeID(rawValue: 1), summary: "Changed text of Sign In / Button / Label",
                detail: "“Sign in to your account” → “Sign in”", state: .applied))),
            item(.appliedChange(AgentAppliedChange(
                changeID: AgentChangeID(rawValue: 2), summary: "Moved Sign In / Email Field",
                detail: "y: 117 → 120", state: .reverted))),
            item(.appliedChange(AgentAppliedChange(
                changeID: AgentChangeID(rawValue: 3), summary: "Moved Sign In / Password Field",
                detail: "y: 163 → 168", state: .applied))),
            item(.agentMessage("The edits are applied. Should I commit them?", isStreaming: false)),
            item(.approval(AgentApprovalRequest(
                approvalID: AgentApprovalID(rawValue: 2), kind: .gitCommit,
                summary: "Create a commit with 2 changed objects",
                diffLines: [
                    AgentDiffLine(change: .modified, target: "Sign In / Button / Label", detail: "text"),
                    AgentDiffLine(change: .modified, target: "Sign In / Password Field", detail: "position"),
                ],
                omittedDiffLineCount: 0,
                validation: AgentValidationSummary(errorCount: 0, warningCount: 0, messages: []),
                lockScope: "No document lock (Git operation)",
                agentExplanation: "Suggested commit message: “Shorten sign-in label”.",
                status: .pending))),
            item(.approval(AgentApprovalRequest(
                approvalID: AgentApprovalID(rawValue: 3), kind: .editBatch,
                summary: "Add a divider below the password field",
                diffLines: [
                    AgentDiffLine(change: .added, target: "Sign In / Divider", detail: "new Shape, 1 pt line"),
                ],
                omittedDiffLineCount: 2,
                validation: AgentValidationSummary(
                    errorCount: 1, warningCount: 0, messages: ["The divider overlaps the Sign In button by 4 pt."]),
                lockScope: "“Sign In” frame and 13 descendants",
                agentExplanation: nil,
                status: .pending))),
        ]
    }
}
#endif
