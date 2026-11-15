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
    private var insertedID: UUID?
    private var node: UUID?
    private let parent: UUID
    private let origin: CGPoint
    private var revision: UInt64
    private var pending: [Edit] = []
    private var persisted: String
    private var painting = false
    private var paintVersion: UInt64 = 0
    private var requiresFirstPaint = true
    private let placement: CGAffineTransform
    private var applying = false
    private var invalid = false
    private(set) var focusEnded = false
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.canvas-text", qos: .userInitiated)
    var onEnd: (() -> Void)?

    init(shell: ShellProjectWindow, session: ShellWindowSessionHandle, pump: ShellRuntimeEventPump,
         view: UUID, parent: UUID, origin: CGPoint, node: UUID?, revision: UInt64, buffer: CanvasTextBuffer, placement: CGAffineTransform) {
        self.shell = shell
        self.session = session
        self.pump = pump
        self.view = view
        self.parent = parent
        self.origin = origin
        self.node = node
        self.revision = revision
        self.placement = placement
        self.persisted = buffer.contents
        input = CanvasTextInputView(buffer: buffer)
        input.confirmed = { [weak self] range, value, original in
            guard let self else { return }
            self.pending.append(Edit(original: original, range: range, value: value))
            self.flush()
        }
        input.changed = { [weak self] buffer in self?.refreshPaint() }
        input.ended = { [weak self] in
            guard let self else { return }
            self.focusEnded = true
            self.onEnd?()
            self.removeIfFinished()
        }
    }

    func refreshPaint() {
        paintVersion &+= 1
        requestPaint()
    }

    private func requestPaint() {
        guard let shell, let session, !session.isClosed, !session.isClosing, (!focusEnded || !pending.isEmpty), !invalid,
              !painting, !applying, session.info?.revision == revision else { return }
        let version = paintVersion, editingRevision = revision, target = node ?? parent
        let isNew = node == nil, view = view, handle = pump.runtimeHandle, owner = session.session
        let buffer = input.buffer
        guard let selection = buffer.scalarRange(buffer.selection) else { return }
        let delta = CanvasTextInputView.replacement(from: persisted, to: buffer.contents,
            preferred: buffer.marked ?? NSRange(location: 0, length: 0), inserted: "")
        guard let range = CanvasTextBuffer(persisted, caret: 0)?.scalarRange(delta.0) else { return }
        painting = true
        queue.async { [weak self] in
            let paint = CanvasTypingPaint.prepare(handle: handle, session: owner, view: view, target: target, revision: editingRevision,
                                                 newText: isNew, start: range.0, end: range.1, value: delta.1, caret: selection.1)
            DispatchQueue.main.async { [weak self, weak shell] in
                guard let self, let shell else { return }
                self.painting = false
                guard shell.canvasTextEditing === self, (!self.focusEnded || !self.pending.isEmpty), !self.invalid,
                      self.session === shell.windowController.windowSession else { return }
                if self.paintVersion != version || self.revision != editingRevision || self.applying {
                    self.requestPaint()
                    return
                }
                guard let paint else {
                    if self.requiresFirstPaint {
                        self.fail(String(localized: "This text could not be laid out. Check its styles and the font selected in File → Document Font…."))
                    }
                    return
                }
                self.requiresFirstPaint = false
                self.flush()
                guard !self.focusEnded else { return }
                let canvas = shell.canvasHost.canvasView
                canvas.typingLayer = .init(parent: self.parent, node: self.node, origin: self.origin, paint: paint)
                let current = self.node.flatMap { canvas.textHit(id: $0) }
                let transform = current.map { $0.localToWorld.concatenating($0.worldToView) } ?? self.placement
                self.input.rectForRange = { _ in paint.caret?.applying(transform) }
                self.input.inputContext?.invalidateCharacterCoordinates()
            }
        }
    }

    private func removeIfFinished() {
        guard focusEnded, !applying, pending.isEmpty, let shell, shell.canvasTextEditing === self else { return }
        shell.canvasTextEditing = nil
    }

    func invalidate() {
        invalid = true
        pending.removeAll()
        stop()
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
        guard !invalid, !requiresFirstPaint, !applying, let edit = pending.first else { removeIfFinished(); return }
        guard let shell, let session, !session.isClosed, !session.isClosing,
              shell.windowController.windowSession === session, session.info?.revision == revision,
              let viewport = session.viewInfo, UUID(uuid: viewport.active_view) == view else {
            fail(String(localized: "The document changed during text input.")); return
        }
        let handle = pump.runtimeHandle, owner = session.session
        applying = true
        if node == nil {
            let accepted = session.track({ done in
                let result = CoreRuntime.insertText(handle: handle, session: owner, view: view, parent: parent,
                                                    revision: revision, origin: origin, text: edit.value)
                return pump.register(result.map { insertion in
                    insertedID = insertion.node
                    return insertion.request
                }, done)
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
            let (status, editToken) = CoreRuntime.beginText(handle: handle, session: owner, view: targetView, node: node,
                                                           revision: targetRevision, start: start, end: end)
            DispatchQueue.main.async { [weak self] in
                guard let self, let session = self.session, !self.invalid, !session.isClosed,
                      !session.isClosing, status == STELLAR_OK else {
                    if editToken != 0 { CoreRuntime.cancelText(handle: handle, session: owner, token: editToken) }
                    self?.fail(String(localized: "The selected text changed during input.")); return
                }
                let accepted = session.track({ done in
                    self.pump.register(CoreRuntime.commitText(handle: handle, session: owner, token: editToken, text: edit.value), done)
                }, completion: { [weak self] event in self?.didApply(event, first: false) })
                if !accepted {
                    CoreRuntime.cancelText(handle: handle, session: owner, token: editToken)
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
            guard state.selection_count == 1,
                  let info = CoreRuntime.canvasInfo(handle: pump.runtimeHandle, session: session.session, size: CGSize(width: 1, height: 1)),
                  UUID(uuid: info.view) == view, info.revision == revision, UUID(uuid: info.primary) == insertedID else {
                fail(String(localized: "The selection changed during text input.")); return
            }
            node = insertedID
        }
        let edit = pending.removeFirst()
        if var updated = CanvasTextBuffer(edit.original, caret: 0),
           updated.replace(edit.value, replacement: edit.range, markedSelection: nil) { persisted = updated.contents }
        applying = false
        shell?.refreshSessionCommands()
        refreshPaint()
        flush()
    }
}

extension ShellProjectWindow {
    func beginCanvasText(at point: CGPoint, canvas: CanvasView) -> Bool {
        if let edit = canvasTextEditing {
            edit.stop()
            guard canvasTextEditing == nil else { return true }
        }
        guard let session = windowController.windowSession, let project = dataSource.session,
              !session.isClosing, !session.isClosed, let hit = canvas.frameHit(at: point, onlyFrames: true),
              hit.revision == session.info?.revision, let buffer = CanvasTextBuffer("", caret: 0) else { return false }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return false }
        let local = point.applying(transform.inverted())
        let edit = ShellCanvasTextEditing(shell: self, session: session, pump: project.pump, view: hit.viewID, parent: hit.id,
                                          origin: local, node: nil, revision: hit.revision, buffer: buffer,
                                          placement: CGAffineTransform(translationX: local.x, y: local.y).concatenating(transform))
        return activateCanvasText(edit, canvas: canvas, anchor: point)
    }

    private func activateCanvasText(_ edit: ShellCanvasTextEditing, canvas: CanvasView, anchor: CGPoint) -> Bool {
        canvasTextEditing = edit
        canvas.addSubview(edit.input)
        edit.input.rectForRange = { _ in CGRect(origin: anchor, size: .zero) }
        edit.onEnd = { [weak edit, weak canvas] in
            canvas?.typingLayer = nil
            edit?.input.removeFromSuperview()
        }
        guard window?.makeFirstResponder(edit.input) == true else { edit.stop(); return false }
        edit.refreshPaint()
        return true
    }

    /// worker가 선택한 본문을 읽는 동안 포커스·선택·revision이 바뀌면 편집을 시작하지 않습니다.
    func editCanvasText(_ hit: CanvasFrameHit, at point: CGPoint?, canvas: CanvasView) -> Bool {
        if let current = canvasTextEditing { current.stop(); guard canvasTextEditing == nil else { return true } }
        guard hit.isText, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let pump = dataSource.session?.pump else { return false }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let local = point?.applying(transform.inverted()) ?? .zero
        let handle = pump.runtimeHandle, owner = session.session, view = hit.viewID, node = hit.id, revision = hit.revision
        let caret = hit.textOffset(handle: handle, session: owner, at: local)?.offset ?? 0
        canvas.window?.makeFirstResponder(canvas)
        return session.select([node]) { [weak self, weak session, weak canvas] event in
            guard case .windowViewChanged = event, let self, let session, let canvas else { return }
            self.refreshSessionCommands()
            DispatchQueue.global(qos: .userInitiated).async {
                guard let text = CoreRuntime.textAtCaret(handle: handle, session: owner, view: view, node: node,
                                                        revision: revision, caret: caret),
                      let buffer = CanvasTextBuffer(text, caret: caret) else { return }
                DispatchQueue.main.async { [weak self, weak session, weak canvas] in
                    guard let self, let session, let canvas, self.windowController.windowSession === session,
                          !session.isClosing, !session.isClosed, self.canvasTextEditing == nil,
                          self.window?.firstResponder === canvas, session.info?.selection_count == 1,
                          let current = canvas.textHit(), current.id == node, current.revision == revision else { return }
                    let edit = ShellCanvasTextEditing(shell: self, session: session, pump: pump, view: view,
                        parent: node, origin: .zero, node: node, revision: revision, buffer: buffer, placement: transform)
                    _ = self.activateCanvasText(edit, canvas: canvas, anchor: point ?? .zero)
                }
            }
        }
    }
}
