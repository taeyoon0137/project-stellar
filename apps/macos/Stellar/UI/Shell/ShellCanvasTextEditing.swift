import AppKit

@MainActor
final class ShellCanvasTextEditing {
    private struct Edit {
        let original: String
        let range: NSRange
        let value: String
    }

    private weak var shell: ShellProjectWindow?
    private weak var session: ShellWindowSessionHandle?
    let input: CanvasTextInputView
    private let pump: ShellRuntimeEventPump
    private let view: UUID
    private var node: UUID?
    private let parent: UUID
    private let origin: CGPoint
    private var revision: UInt64
    private var pending: [Edit] = []
    private var applying = false
    private var invalid = false
    private(set) var focusEnded = false
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.canvas-text", qos: .userInitiated)
    var onChange: ((CanvasTextBuffer) -> Void)?
    var onEnd: (() -> Void)?

    init(shell: ShellProjectWindow, session: ShellWindowSessionHandle, pump: ShellRuntimeEventPump,
         view: UUID, parent: UUID, origin: CGPoint, node: UUID?, revision: UInt64, buffer: CanvasTextBuffer) {
        self.shell = shell
        self.session = session
        self.pump = pump
        self.view = view
        self.parent = parent
        self.origin = origin
        self.node = node
        self.revision = revision
        input = CanvasTextInputView(buffer: buffer)
        input.confirmed = { [weak self] range, value, original in
            guard let self else { return }
            self.pending.append(Edit(original: original, range: range, value: value))
            self.flush()
        }
        input.changed = { [weak self] buffer in self?.onChange?(buffer) }
        input.ended = { [weak self] in
            guard let self else { return }
            self.focusEnded = true
            self.onEnd?()
            self.removeIfFinished()
        }
    }

    private func removeIfFinished() {
        guard focusEnded, !applying, pending.isEmpty, let shell, shell.canvasTextEditing === self else { return }
        shell.canvasTextEditing = nil
    }

    func stop() {
        input.finish()
        if input.window?.firstResponder === input { input.window?.makeFirstResponder(input.superview) }
        input.removeFromSuperview()
    }

    private func fail(_ message: String) {
        guard !invalid else { return }
        invalid = true
        pending.removeAll()
        applying = false
        stop()
        shell?.alertPresenter?(String(localized: "Couldn’t Edit Text"), message, shell?.window)
    }

    private func flush() {
        guard !invalid, !applying, let edit = pending.first else { removeIfFinished(); return }
        guard let shell, let session, !session.isClosed, !session.isClosing, shell.gitSource.trusted,
              shell.windowController.windowSession === session, session.info?.revision == revision,
              let viewport = session.viewInfo, UUID(uuid: viewport.active_view) == view else {
            fail(String(localized: "The document changed during text input.")); return
        }
        let handle = pump.runtimeHandle, owner = session.session
        applying = true
        if node == nil {
            let accepted = session.track({ done in
                var request: UInt64 = 0
                let data = Array(edit.value.utf8)
                let status = withUnsafeBytes(of: view.uuid) { v in
                    withUnsafeBytes(of: parent.uuid) { p in
                        data.withUnsafeBufferPointer { bytes in
                            stellar_window_insert_text(handle, owner, v.bindMemory(to: UInt8.self).baseAddress,
                                p.bindMemory(to: UInt8.self).baseAddress, revision, Float(origin.x), Float(origin.y),
                                bytes.baseAddress, bytes.count, &request)
                        }
                    }
                }
                return pump.trackTextRequest(status, request, completion: done)
            }, completion: { [weak self] event in self?.didApply(event, first: true) })
            if !accepted { fail(String(localized: "The project is busy. Try again.")) }
            return
        }
        guard let node, let original = CanvasTextBuffer(edit.original, caret: 0),
              let (start, end) = original.scalarRange(edit.range) else {
            fail(String(localized: "The selected text range changed.")); return
        }
        let targetView = view, targetRevision = revision
        queue.async { [weak self] in
            var token: UInt64 = 0
            let status = withUnsafeBytes(of: targetView.uuid) { v in
                withUnsafeBytes(of: node.uuid) { n in
                    stellar_text_begin(handle, owner, v.bindMemory(to: UInt8.self).baseAddress,
                                       n.bindMemory(to: UInt8.self).baseAddress, targetRevision, start, end, &token)
                }
            }
            let editToken = token
            DispatchQueue.main.async { [weak self] in
                guard let self, let session = self.session, !self.invalid, !session.isClosed,
                      !session.isClosing, self.shell?.gitSource.trusted == true, status == STELLAR_OK else {
                    if editToken != 0 { _ = stellar_text_cancel(handle, owner, editToken) }
                    self?.fail(String(localized: "The selected text changed during input.")); return
                }
                let accepted = session.track({ done in
                    var request: UInt64 = 0
                    let result = Array(edit.value.utf8).withUnsafeBufferPointer { bytes in
                        stellar_text_commit(handle, owner, editToken, bytes.baseAddress, bytes.count, &request)
                    }
                    return self.pump.trackTextRequest(result, request, completion: done)
                }, completion: { [weak self] event in self?.didApply(event, first: false) })
                if !accepted {
                    _ = stellar_text_cancel(handle, owner, editToken)
                    self.fail(String(localized: "The project is busy. Try again."))
                }
            }
        }
    }

    private func didApply(_ event: CoreRuntimeEvent, first: Bool) {
        guard case .windowViewChanged = event, let session, let state = session.info else {
            if case .requestFailed(_, _, let detail) = event { fail(detail) }
            else { fail(String(localized: "The text edit did not finish.")) }
            return
        }
        guard revision < UInt64.max, state.revision == revision + 1 else {
            fail(String(localized: "The document changed during text input.")); return
        }
        revision = state.revision
        if first {
            var info = StellarCanvasInfo()
            guard state.selection_count == 1,
                  stellar_window_canvas(pump.runtimeHandle, session.session, 1, 1, &info, nil, 0) == STELLAR_OK,
                  UUID(uuid: info.view) == view, info.revision == revision else {
                fail(String(localized: "The selection changed during text input.")); return
            }
            node = UUID(uuid: info.primary)
        }
        pending.removeFirst()
        applying = false
        shell?.refreshSessionCommands()
        onChange?(input.buffer)
        flush()
    }
}

extension ShellProjectWindow {
    func beginCanvasText(at point: CGPoint, canvas: CanvasView) -> Bool {
        if let edit = canvasTextEditing {
            edit.stop()
            guard canvasTextEditing == nil else { return true }
        }
        guard let session = windowController.windowSession, let project = dataSource.session, gitSource.trusted,
              !session.isClosing, !session.isClosed, let hit = canvas.frameHit(at: point, onlyFrames: true),
              hit.revision == session.info?.revision, let buffer = CanvasTextBuffer("", caret: 0) else { return false }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return false }
        let local = point.applying(transform.inverted())
        let edit = ShellCanvasTextEditing(shell: self, session: session, pump: project.pump, view: hit.viewID, parent: hit.id,
                                          origin: local, node: nil, revision: hit.revision, buffer: buffer)
        canvasTextEditing = edit
        canvas.addSubview(edit.input)
        edit.input.rectForRange = { _ in CGRect(origin: point, size: .zero) }
        edit.onEnd = { [weak edit, weak canvas] in
            edit?.input.removeFromSuperview()
            canvas?.needsDisplay = true
        }
        guard window?.makeFirstResponder(edit.input) == true else { edit.stop(); return false }
        return true
    }
}
