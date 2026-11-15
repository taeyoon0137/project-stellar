import AppKit

private enum DocumentRecoveryOffer: Sendable {
    case none
    case checkpoint(String, canRestore: Bool)
    case failed(String)

    static func probe(_ runtime: UInt64, _ session: UInt64) -> Self {
        let (status, bytes) = CoreRuntime.probeRecovery(handle: runtime, session: session)
        guard status == STELLAR_OK, let tag = bytes.first else {
            return .failed(String(localized: "Couldn’t check for unsaved changes."))
        }
        let message = String(decoding: bytes.dropFirst(), as: UTF8.self)
        switch tag {
        case 48: return .none
        case 49, 50: return .checkpoint(message, canRestore: tag == 49)
        default: return .failed(message)
        }
    }
}

extension ShellProjectWindow {
    func offerDocumentRecovery() {
        guard let project = dataSource.session, let session = windowController.windowSession else { return }
        let runtime = project.pump.runtimeHandle, handle = session.session
        DispatchQueue.global(qos: .userInitiated).async { [weak self, weak session] in
            let offer = DocumentRecoveryOffer.probe(runtime, handle)
            DispatchQueue.main.async { [weak self, weak session] in
                guard let self, let session, self.windowController.windowSession === session, !session.isClosed, let window = self.window else { return }
                switch offer {
                case .none: return
                case .failed(let message): self.showRecoveryError(message)
                case .checkpoint(let ticket, let baseMatches):
                    let canRestore = baseMatches && session.info?.has_unsaved_changes == 0
                    let prompt = NSAlert()
                    prompt.messageText = String(localized: "Recover Unsaved Changes?")
                    prompt.informativeText = canRestore
                        ? String(localized: "Restore the unsaved document from the previous session. Save when you are ready to write it to the project.")
                        : String(localized: "The project or open document has changed. You can save the recovered document in a new project folder. Shared components and assets stay in the original project.")
                    if canRestore { prompt.addButton(withTitle: String(localized: "Restore Changes")) }
                    prompt.addButton(withTitle: String(localized: "Save Recovered Document…"))
                    prompt.addButton(withTitle: String(localized: "Later"))
                    self.recoveryPrompt = prompt
                    prompt.beginSheetModal(for: window) { [weak self, weak session] response in
                        guard let self, let session, self.windowController.windowSession === session, !session.isClosed else { return }
                        self.recoveryPrompt = nil
                        if canRestore && response == .alertFirstButtonReturn {
                            self.submitDocumentRecovery(ticket, destination: nil)
                        } else if response == (canRestore ? .alertSecondButtonReturn : .alertFirstButtonReturn) {
                            let save = NSSavePanel()
                            save.title = String(localized: "Save Recovered Document")
                            save.message = String(localized: "Choose a new folder outside the original project. Only the recovered document is included.")
                            save.nameFieldStringValue = project.name + String(localized: " Recovered")
                            save.directoryURL = URL(fileURLWithPath: project.rootPath).deletingLastPathComponent()
                            save.beginSheetModal(for: window) { [weak self] response in
                                guard response == .OK, let url = save.url, let self, self.windowController.windowSession === session else { return }
                                self.submitDocumentRecovery(ticket, destination: url)
                            }
                        }
                    }
                }
            }
        }
    }

    func submitDocumentRecovery(_ ticket: String, destination: URL?) {
        guard let project = dataSource.session, let session = windowController.windowSession else { return }
        let accepted = session.track({ project.pump.recoverDocument(session.session, ticket: ticket, destination: destination, completion: $0) }) { [weak self] event in
            guard let self, self.windowController.windowSession === session else { return }
            if case .requestFailed(_, _, let message) = event { self.showRecoveryError(message) }
            else {
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
