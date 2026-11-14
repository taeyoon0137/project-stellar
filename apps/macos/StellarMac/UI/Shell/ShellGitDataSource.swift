import Foundation

/// 열린 프로젝트의 OS 신뢰 등록을 코어 요청으로 읽고, 현재 창에 속하는 완료만 반영합니다.
@MainActor
final class ShellGitDataSource: GitPanelDataSource {
    var onEventsAvailable: (() -> Void)?
    var trustDidChange: ((Bool) -> Void)?
    private weak var session: ShellProjectSessionHandle?
    private var events: [GitPanelEvent] = []
    private var trusted = false
    private var message: String?
    private var trustRequest: UInt64?

    func attach(_ session: ShellProjectSessionHandle?) {
        self.session = session
        trustRequest = nil
        events.removeAll()
        updateState(false)
        if session != nil { updateTrust(nil) }
    }

    func updateTrust(_ decision: Bool?) {
        guard let session, !session.isClosed else { return }
        let result = session.pump.projectTrust(session.project, decision: decision) { [weak self, weak session] event in
            guard let self, let session, self.session === session, !session.isClosed,
                  event.request == self.trustRequest else { return }
            self.trustRequest = nil
            switch event {
            case .projectTrust(_, let project, let trusted) where project == session.project:
                self.updateState(trusted)
            case .requestFailed(_, _, let message):
                self.updateState(false, message: message)
            default:
                self.updateState(false, message: String(localized: "Couldn’t verify project trust."))
            }
        }
        switch result {
        case .success(let request):
            trustRequest = request
        case .failure:
            trustRequest = nil
            updateState(false, message: String(localized: "Couldn’t submit the request to verify project trust."))
        }
    }

    func send(_ command: GitPanelCommand) {
        switch command {
        case .refreshStatus:
            if session == nil || trustRequest != nil { emitStatus() } else { updateTrust(nil) }
        default:
            emit(.commandRejected(GitCommandRejection(command: command, reason: .unavailable)))
        }
    }

    private func updateState(_ trusted: Bool, message: String? = nil) {
        self.trusted = trusted
        self.message = message
        trustDidChange?(trusted)
        emitStatus()
    }

    private func emitStatus() {
        let status = GitStatusSnapshot(availability: trusted ? .ready : .restricted, head: .unknown,
            upstream: nil, remotes: [], identityConfigured: false, stagedChangeCount: 0, unstagedChangeCount: 0,
            mergeState: .none, semanticConflictCount: 0, unresolvedConflictCount: 0, rawConflictCount: 0,
            stagedSnapshotIssue: nil, semanticStatusError: message ?? (trusted ? String(localized: "Changes haven’t been loaded.") : nil),
            pendingAutosave: false, changes: .empty, conflicts: .empty)
        emit(.status(status))
    }

    private func emit(_ event: GitPanelEvent) {
        events.append(event)
        onEventsAvailable?()
    }

    func pollEvent() -> GitPanelEvent? { events.isEmpty ? nil : events.removeFirst() }
}
