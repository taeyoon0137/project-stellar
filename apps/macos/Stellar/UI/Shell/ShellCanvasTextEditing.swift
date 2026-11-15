import AppKit

/// 캔버스 Text 편집 session의 셸 쪽(ADR-0067 §6). 입력기 사건을 순서대로 코어 요청으로 보냅니다.
///
/// - 조합 상태는 코어가 가집니다. 조합 시작에서 `begin`, 갱신마다 `mark`, 확정에서 `commit`, 취소에서 `cancel`을 보냅니다.
/// - 조합 없는 타이핑·삭제·붙여넣기는 `replace` 한 번입니다. 연속 입력은 코어가 Undo 한 번으로 묶습니다.
/// - 새 Text는 시작할 때 코어 조합(빈 본문)을 열어 caret을 보여 주고, 첫 확정에서 만듭니다. 입력 없이 끝나면 버립니다.
/// - 요청은 하나씩 보내고 완료를 받은 뒤 다음을 보냅니다. 연속된 조합 갱신은 마지막 것만 보냅니다.
@MainActor
final class ShellCanvasTextEditing {
    private enum Op {
        case begin(start: UInt32, end: UInt32)
        case mark(String)
        case commit(String)
        case cancel
        case replace(start: UInt32, end: UInt32, text: String)
        case history(redo: Bool)

        /// 문서를 바꾸는 요청(확정·조합 없는 입력·Undo)인지. 조합 시작·갱신·취소는 문서와 history를 바꾸지 않습니다.
        var changesDocument: Bool {
            switch self {
            case .commit, .replace, .history: true
            case .begin, .mark, .cancel: false
            }
        }
    }

    private weak var shell: ShellProjectWindow?
    private weak var session: ShellWindowSessionHandle?
    let input: CanvasTextInputView
    private let pump: ShellRuntimeEventPump
    private let view: UUID
    private let parent: UUID
    private let origin: CGPoint
    private let anchor: CGPoint
    /// 문서에 있는 편집 대상. 새 Text는 첫 확정 전까지 nil입니다.
    private var node: UUID?
    /// 코어 조합의 token과 미리보기 행 node.
    private var token: UInt64?
    private var previewNode: UUID?
    /// 보낸(또는 보낼) 요청 기준으로 코어 조합이 열려 있는지, 새 Text를 만드는 확정을 이미 보냈는지.
    private var composing = false
    private var nodePending = false
    private var revision: UInt64
    private var queue: [Op] = []
    private var running = false
    private var invalid = false
    private(set) var focusEnded = false
    var onEnd: (() -> Void)?
    /// 창 닫기·앱 종료가 기다리는 동안, 아직 보내지 않은 문서 변경 요청을 모두 보낸 뒤 한 번 부릅니다.
    var onDocumentChangesSent: (() -> Void)?

    /// 아직 코어로 보내지 않은 문서 변경 요청이 있는지. 이미 보낸 요청은 창 session의 진행 요청으로 세어 닫기 저장이
    /// 완료를 기다립니다.
    var hasUnsentDocumentChanges: Bool {
        queue.dropFirst(running ? 1 : 0).contains { $0.changesDocument }
    }

    init(shell: ShellProjectWindow, session: ShellWindowSessionHandle, pump: ShellRuntimeEventPump, view: UUID,
         parent: UUID, origin: CGPoint, anchor: CGPoint, node: UUID?, revision: UInt64, buffer: CanvasTextBuffer) {
        self.shell = shell
        self.session = session
        self.pump = pump
        self.view = view
        self.parent = parent
        self.origin = origin
        self.anchor = anchor
        self.node = node
        self.revision = revision
        input = CanvasTextInputView(buffer: buffer)
        input.began = { [weak self] range, original in
            guard let self, !self.composing else { return }
            guard let scalars = CanvasTextBuffer(original, caret: 0)?.scalarRange(range) else {
                self.fail(String(localized: "The selected text range changed.")); return
            }
            self.openComposition(start: scalars.0, end: scalars.1)
        }
        input.marked = { [weak self] text in self?.enqueue(.mark(text)) }
        input.committed = { [weak self] text in self?.commit(text) }
        input.cancelled = { [weak self] in
            guard let self else { return }
            if self.creating {
                // 새 Text의 위치 조합은 남겨 두고 조합 본문만 비웁니다(caret 유지).
                self.enqueue(.mark(""))
            } else {
                self.composing = false
                self.enqueue(.cancel)
            }
        }
        input.replaced = { [weak self] range, text, original in
            guard let self else { return }
            guard let scalars = CanvasTextBuffer(original, caret: 0)?.scalarRange(range) else {
                self.fail(String(localized: "The selected text range changed.")); return
            }
            if self.creating {
                // 새 Text의 첫 입력은 위치 조합을 그 본문으로 확정해 Text를 만듭니다.
                if !self.composing { self.openComposition(start: 0, end: 0) }
                self.commit(text)
            } else {
                self.enqueue(.replace(start: scalars.0, end: scalars.1, text: text))
            }
        }
        input.changed = { [weak self] _ in self?.updateOverlay() }
        input.history = { [weak self] redo in self?.history(redo: redo) }
        input.ended = { [weak self] in
            guard let self else { return }
            self.focusEnded = true
            if self.composing {
                self.composing = false
                self.enqueue(.cancel)
            }
            self.onEnd?()
            self.removeIfFinished()
        }
    }

    /// 새 Text는 빈 코어 조합을 먼저 열어 기본 글꼴의 caret을 보여 줍니다.
    func start() {
        if creating { openComposition(start: 0, end: 0) }
        updateOverlay()
    }

    /// 새 Text를 아직 만들지 않았고 만드는 확정도 보내지 않았습니다.
    private var creating: Bool { node == nil && !nodePending }

    private func openComposition(start: UInt32, end: UInt32) {
        composing = true
        enqueue(.begin(start: start, end: end))
    }

    private func commit(_ text: String) {
        if creating { nodePending = true }
        composing = false
        enqueue(.commit(text))
    }

    private func enqueue(_ op: Op) {
        guard !invalid else { return }
        if case .mark = op, (running ? queue.count > 1 : !queue.isEmpty), case .mark = queue[queue.count - 1] {
            queue[queue.count - 1] = op
        } else {
            queue.append(op)
        }
        send()
    }

    private func history(redo: Bool) {
        if creating, !input.hasMarkedText() {
            // 입력 없는 새 Text면 편집을 끝내고 창의 일반 Undo·Redo로 넘깁니다.
            stop()
            shell?.performSessionCommand(redo ? .redo : .undo)
            return
        }
        input.unmarkText()
        enqueue(.history(redo: redo))
    }

    private func send() {
        guard !running, !invalid, let op = queue.first else { removeIfFinished(); return }
        guard let shell, let session, !session.isClosed, !session.isClosing,
              shell.windowController.windowSession === session else {
            fail(String(localized: "The document changed during text input.")); return
        }
        let handle = pump.runtimeHandle, owner = session.session
        var started: (token: UInt64, node: UUID)?
        let submit: (@escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError>
        switch op {
        case .begin(let start, let end):
            submit = { [self] done in
                let result = CoreRuntime.beginText(handle: handle, session: owner, view: view, node: node ?? parent,
                                                   revision: revision, newAt: node == nil ? origin : nil, start: start, end: end)
                if case .success(let value) = result { started = value }
                return pump.register(result.map(\.token), done)
            }
        case .mark(let text):
            guard let token else { return skip() }
            submit = { [pump] done in pump.register(CoreRuntime.markText(handle: handle, session: owner, token: token, text: text), done) }
        case .commit(let text):
            guard let token else { return skip() }
            submit = { [pump] done in pump.register(CoreRuntime.commitText(handle: handle, session: owner, token: token, text: text), done) }
        case .cancel:
            guard let token else { return skip() }
            submit = { [pump] done in pump.register(CoreRuntime.cancelText(handle: handle, session: owner, token: token), done) }
        case .replace(let start, let end, let text):
            guard let node else { return skip() }
            submit = { [self] done in
                pump.register(CoreRuntime.replaceText(handle: handle, session: owner, view: view, node: node, revision: revision,
                                                      start: start, end: end, text: text), done)
            }
        case .history(let redo):
            guard let code = shell.coreCommandIDs.coreCommandID(for: redo ? .redo : .undo) else { return skip() }
            running = true
            let accepted = session.perform(coreCommand: code) { [weak self] event in self?.didFinish(op, event: event, started: nil) }
            if !accepted { fail(String(localized: "The project is busy. Try again.")) }
            return
        }
        running = true
        let accepted = session.track(submit) { [weak self] event in self?.didFinish(op, event: event, started: started) }
        if !accepted { fail(String(localized: "The project is busy. Try again.")); return }
        if !hasUnsentDocumentChanges, let retry = onDocumentChangesSent {
            onDocumentChangesSent = nil
            DispatchQueue.main.async(execute: retry)
        }
    }

    /// 조합이 시작되지 않았거나 이미 끝난 요청은 보내지 않습니다.
    private func skip() {
        queue.removeFirst()
        send()
    }

    private func didFinish(_ op: Op, event: CoreRuntimeEvent, started: (token: UInt64, node: UUID)?) {
        running = false
        guard !invalid else { return }
        guard case .windowViewChanged = event, let session, let state = session.info else {
            if case .requestFailed(_, _, let detail) = event { fail(detail) }
            else { fail(String(localized: "The text edit did not finish.")) }
            return
        }
        queue.removeFirst()
        let before = revision
        revision = state.revision
        switch op {
        case .begin:
            token = started?.token
            previewNode = started?.node
        case .commit:
            token = nil
            if node == nil, state.revision == before {
                // 빈 본문 확정은 새 Text를 만들지 않습니다. 다음 입력이 위치 조합을 다시 엽니다.
                nodePending = false
                previewNode = nil
            } else if node == nil, previewNode != nil {
                // 새 Text를 만들었으면 코어가 그 Text를 선택합니다.
                guard state.selection_count == 1,
                      let info = CoreRuntime.canvasInfo(handle: pump.runtimeHandle, session: session.session,
                                                        size: CGSize(width: 1, height: 1)),
                      UUID(uuid: info.primary) == previewNode else {
                    fail(String(localized: "The selection changed during text input.")); return
                }
                node = previewNode
            }
        case .cancel:
            token = nil
            if node == nil { previewNode = nil }
        case .history:
            reloadContent()
        case .mark, .replace:
            break
        }
        shell?.refreshSessionCommands()
        if case .begin = op, node == nil, previewNode != nil, shell?.canvasHost.canvasView.textHit(id: previewNode!) == nil {
            fail(String(localized: "This text can’t be shown with the available fonts.")); return
        }
        updateOverlay()
        send()
    }

    /// Undo·Redo 뒤 문서의 본문을 다시 읽습니다. 편집하던 Text가 없어졌으면 편집을 끝냅니다.
    private func reloadContent() {
        guard let node, let session else { return }
        let handle = pump.runtimeHandle, owner = session.session, view = view, revision = revision
        let caret = input.buffer.scalarRange(input.buffer.selection)?.1 ?? 0
        running = true
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let text = CoreRuntime.textContent(handle: handle, session: owner, view: view, node: node, revision: revision)
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.invalid else { return }
                self.running = false
                guard let text, self.revision == revision else { self.stop(); return }
                let count = UInt32(text.unicodeScalars.count)
                if let buffer = CanvasTextBuffer(text, caret: min(caret, count)) { self.input.reload(buffer) }
                self.updateOverlay()
                self.send()
            }
        }
    }

    /// caret·선택 사각형과 입력기 후보창 위치를 코어 scene의 같은 Text 행에서 구합니다.
    private func updateOverlay() {
        guard let shell, let session, !focusEnded else { return }
        let canvas = shell.canvasHost.canvasView
        guard let target = previewNode ?? node, let hit = canvas.textHit(id: target),
              let range = input.buffer.scalarRange(input.buffer.selection) else {
            canvas.textEditOverlay = nil
            let anchor = anchor
            input.rectForRange = { _ in CGRect(origin: anchor, size: .zero) }
            return
        }
        let handle = pump.runtimeHandle
        let caret = hit.textRects(handle: handle, session: session.session, start: range.1, end: range.1, upstream: false)?.first
        let selection = range.0 < range.1
            ? hit.textRects(handle: handle, session: session.session, start: range.0, end: range.1, upstream: false) ?? []
            : []
        canvas.textEditOverlay = .init(node: target, caret: caret, selection: selection)
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        input.rectForRange = { _ in caret?.applying(transform) }
        input.inputContext?.invalidateCharacterCoordinates()
    }

    private func removeIfFinished() {
        guard focusEnded, !running, queue.isEmpty, let shell, shell.canvasTextEditing === self else { return }
        shell.canvasTextEditing = nil
    }

    func invalidate() {
        invalid = true
        queue.removeAll()
        stop()
    }

    /// 창 닫기·앱 종료: 남은 요청이 문서를 바꾸지 않으므로 더 보내지 않고 편집을 끝냅니다. 이미 보낸 요청의 늦은
    /// 완료는 버립니다(코어의 남은 조합은 창 session과 함께 닫힙니다).
    func abandonForClosing() {
        invalid = true
        queue.removeAll()
        onDocumentChangesSent = nil
        stop()
        if let shell, shell.canvasTextEditing === self { shell.canvasTextEditing = nil }
    }

    func stop() {
        input.finish()
        if input.window?.firstResponder === input { input.window?.makeFirstResponder(input.superview) }
        input.removeFromSuperview()
    }

    private func fail(_ message: String) {
        guard !invalid else { return }
        invalid = true
        queue.removeAll()
        onDocumentChangesSent = nil
        running = false
        stop()
        if let shell, shell.canvasTextEditing === self { shell.canvasTextEditing = nil }
        // 막다른 alert 대신 도구 막대 위 한 줄 안내로 알립니다(UX 명세 3.5).
        shell?.windowController.toolbarController.hintView.show(String(
            localized: "Couldn’t edit the text: \(message)",
            comment: "Canvas hint when a text edit fails; the argument is the reason"
        ))
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
                                          origin: local, anchor: point, node: nil, revision: hit.revision, buffer: buffer)
        return activateCanvasText(edit, canvas: canvas)
    }

    private func activateCanvasText(_ edit: ShellCanvasTextEditing, canvas: CanvasView) -> Bool {
        canvasTextEditing = edit
        canvas.addSubview(edit.input)
        edit.onEnd = { [weak edit, weak canvas] in
            canvas?.textEditOverlay = nil
            edit?.input.removeFromSuperview()
        }
        guard window?.makeFirstResponder(edit.input) == true else { edit.stop(); return false }
        edit.start()
        return true
    }

    /// worker가 선택한 본문을 읽는 동안 포커스·선택·revision이 바뀌면 편집을 시작하지 않습니다.
    func editCanvasText(_ hit: CanvasFrameHit, at point: CGPoint?, canvas: CanvasView) -> Bool {
        if let current = canvasTextEditing { current.stop(); guard canvasTextEditing == nil else { return true } }
        guard hit.isText, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let pump = dataSource.session?.pump else { return false }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let local = point?.applying(transform.inverted()) ?? .zero
        let handle = pump.runtimeHandle, owner = session.session, view = hit.viewID, node = hit.id
        let caret = hit.textOffset(handle: handle, session: owner, at: local)?.offset ?? 0
        canvas.window?.makeFirstResponder(canvas)
        return session.select([node]) { [weak self, weak session, weak canvas] event in
            guard case .windowViewChanged = event, let self, let session, let canvas,
                  let revision = session.info?.revision else { return }
            self.refreshSessionCommands()
            DispatchQueue.global(qos: .userInitiated).async { [weak self, weak session, weak canvas] in
                guard let text = CoreRuntime.textContent(handle: handle, session: owner, view: view, node: node,
                                                         revision: revision),
                      let buffer = CanvasTextBuffer(text, caret: min(caret, UInt32(text.unicodeScalars.count))) else { return }
                DispatchQueue.main.async { [weak self, weak session, weak canvas] in
                    guard let self, let session, let canvas, self.windowController.windowSession === session,
                          !session.isClosing, !session.isClosed, self.canvasTextEditing == nil,
                          self.window?.firstResponder === canvas, session.info?.selection_count == 1,
                          let current = canvas.textHit(), current.id == node, current.revision == revision else { return }
                    let edit = ShellCanvasTextEditing(shell: self, session: session, pump: pump, view: view, parent: node,
                                                      origin: .zero, anchor: point ?? .zero, node: node, revision: revision,
                                                      buffer: buffer)
                    _ = self.activateCanvasText(edit, canvas: canvas)
                }
            }
        }
    }
}
