import AppKit

/// 비정상 종료 뒤 남은 미저장 복구본 제안(아키텍처 §22.3). 선택지는 Restore·Discard·Later입니다.
private enum DocumentRecoveryOffer: Sendable {
    case none
    case candidate(StellarRecoveryCandidate)
    case failed

    static func probe(_ runtime: UInt64, _ session: UInt64) -> Self {
        guard let candidate = CoreRuntime.probeRecovery(handle: runtime, session: session) else { return .failed }
        return candidate.has_candidate == 1 ? .candidate(candidate) : .none
    }
}

extension ShellProjectWindow {
    func offerDocumentRecovery() {
        guard let project = dataSource.session, let session = windowController.windowSession else { return }
        let runtime = project.pump.runtimeHandle, handle = session.session
        DispatchQueue.global(qos: .userInitiated).async { [weak self, weak session] in
            let offer = DocumentRecoveryOffer.probe(runtime, handle)
            DispatchQueue.main.async { [weak self, weak session] in
                guard let self, let session, self.windowController.windowSession === session, !session.isClosed,
                      let window = self.window else { return }
                switch offer {
                case .none: return
                case .failed: self.showRecoveryError(String(localized: "Couldn’t check for unsaved changes."))
                case .candidate(let candidate):
                    // 이 창에서 이미 편집했으면 복원할 수 없습니다(코어도 거부). 삭제와 나중에만 남깁니다.
                    let canRestore = session.info?.has_unsaved_changes == 0
                    let prompt = NSAlert()
                    prompt.messageText = String(localized: "Recover Unsaved Changes?")
                    prompt.informativeText = String(localized: "Restore the unsaved document from the previous session. Save when you are ready to write it to the project.")
                    if canRestore { prompt.addButton(withTitle: String(localized: "Restore Changes")) }
                    prompt.addButton(withTitle: String(localized: "Discard Changes"))
                    prompt.addButton(withTitle: String(localized: "Later"))
                    prompt.buttons[canRestore ? 1 : 0].hasDestructiveAction = true
                    self.recoveryPrompt = prompt
                    prompt.beginSheetModal(for: window) { [weak self, weak session] response in
                        guard let self, let session, self.windowController.windowSession === session, !session.isClosed else { return }
                        self.recoveryPrompt = nil
                        let restore = canRestore && response == .alertFirstButtonReturn
                        let discard = response == (canRestore ? .alertSecondButtonReturn : .alertFirstButtonReturn)
                        if restore { self.submitDocumentRecovery(candidate, action: UInt32(STELLAR_RECOVERY_RESTORE)) }
                        if discard { self.submitDocumentRecovery(candidate, action: UInt32(STELLAR_RECOVERY_DISCARD)) }
                        // Later: 복구본을 그대로 두고 다음 열기에 다시 제안합니다.
                    }
                }
            }
        }
    }

    private func submitDocumentRecovery(_ candidate: StellarRecoveryCandidate, action: UInt32) {
        guard let project = dataSource.session, let session = windowController.windowSession else { return }
        let accepted = session.track({ project.pump.recoverDocument(session.session, candidate: candidate, action: action,
                                                                    completion: $0) }) { [weak self] event in
            guard let self, self.windowController.windowSession === session else { return }
            if case .requestFailed(_, _, let message) = event { self.showRecoveryError(message) }
            else if action == UInt32(STELLAR_RECOVERY_RESTORE) {
                self.refreshSessionCommands()
                self.reloadComponentCanvas()
            }
        }
        if !accepted { showRecoveryError(String(localized: "Couldn’t start document recovery.")) }
    }

    private func showRecoveryError(_ message: String) {
        guard let window else { return }
        let alert = NSAlert()
        alert.messageText = String(localized: "Document Recovery")
        alert.informativeText = message
        alert.addButton(withTitle: String(localized: "OK"))
        alert.beginSheetModal(for: window)
    }
}
