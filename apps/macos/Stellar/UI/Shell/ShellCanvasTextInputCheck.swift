import AppKit

#if DEBUG
@MainActor
enum ShellCanvasTextInputCheck {
    static func run(fixture: URL, font: URL) -> Bool {
        let copies = ShellSavedProjectFixture(prefix: "StellarCanvasIME")
        defer { copies.remove() }
        guard let path = copies.copy(fixture, as: "Canvas Text Input") else { return false }
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarCanvasIME")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        var notices: [String] = []
        app.alertPresenter = { _, message, _ in notices.append(message) }
        guard let pump = app.pump else { return false }
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("canvas-ime check: \(message); \(notices)\n".utf8)); return false
        }
        func until(_ test: () -> Bool) -> Bool {
            let end = Date().addingTimeInterval(20)
            while !test(), Date() < end {
                _ = pump.drain()
                RunLoop.current.run(until: Date().addingTimeInterval(0.01))
            }
            return test()
        }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 20) }
        func open() -> ShellProjectWindow? {
            app.openProject(atPath: path, origin: .application)
            guard wait(), let project = app.projectWindow(forPath: path), let window = project.window else { return nil }
            window.setContentSize(CGSize(width: 1280, height: 790))
            window.contentView?.layoutSubtreeIfNeeded()
            project.fitNewProjectFrame()
            return wait() ? project : nil
        }
        guard let shell = open(), let window = shell.window, let session = shell.windowController.windowSession,
              let before = session.info else { return fail("open") }
        shell.gitSource.updateTrust(true)
        var fill = StellarCanvasColor()
        fill.space = 1; fill.rgba = (0, 0, 0, 1)
        guard wait(), shell.applyDocumentFont(font.path, face: 0, size: 1536, fill: fill, revision: before.revision), wait(),
              let base = session.info, let camera = session.viewInfo else { return fail("project font") }
        let canvas = shell.canvasHost.canvasView
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(x: canvas.bounds.midX + (x - CGFloat(camera.center_x)) * CGFloat(camera.zoom),
                    y: canvas.bounds.midY + (y - CGFloat(camera.center_y)) * CGFloat(camera.zoom))
        }
        let location = point(370, 265)
        func key(_ code: UInt16, _ chars: String) -> Bool {
            guard let e = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
                characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code) else { return false }
            canvas.keyDown(with: e); return true
        }
        func click() -> Bool {
            ShellCanvasMoveCheck.mouse(canvas, .leftMouseDown, at: location) && ShellCanvasMoveCheck.mouse(canvas, .leftMouseUp, at: location)
        }
        func ink() -> [UInt8]? {
            guard let rep = ShellVisibleFramesCheck.bitmap(canvas, in: window) else { return nil }
            var result: [UInt8] = []
            for y in 0..<28 {
                for x in 3..<50 {
                    let p = point(370 + CGFloat(x), 265 + CGFloat(y))
                    let ix = Int(p.x * CGFloat(rep.pixelsWide) / canvas.bounds.width)
                    let iy = Int(p.y * CGFloat(rep.pixelsHigh) / canvas.bounds.height)
                    guard ix >= 0, iy >= 0, ix < rep.pixelsWide, iy < rep.pixelsHigh else { return nil }
                    var pixel = [Int](repeating: 0, count: rep.samplesPerPixel)
                    rep.getPixel(&pixel, atX: ix, y: iy)
                    result.append(UInt8(clamping: pixel[0]))
                }
            }
            return result
        }
        guard key(17, "t"), click(), until({ canvas.typingLayer?.paint.caret != nil }),
              let empty = window.firstResponder as? CanvasTextInputView else { return fail("T click/empty caret") }
        empty.cancelOperation(nil)
        guard until({ shell.canvasTextEditing == nil }), session.info?.revision == base.revision,
              session.info?.undo_depth == base.undo_depth, let blank = ink() else { return fail("empty cancel") }
        guard key(17, "t"), click(), until({ canvas.typingLayer != nil }),
              let input = window.firstResponder as? CanvasTextInputView else { return fail("second T") }
        let missing = NSRange(location: NSNotFound, length: 0)
        for part in ["ㅎ", "하", "한"] {
            input.setMarkedText(part, selectedRange: NSRange(location: 1, length: 0), replacementRange: missing)
            guard input.hasMarkedText(), session.info?.revision == base.revision else { return fail("transient state") }
        }
        guard until({ canvas.typingLayer?.paint.geometry.paths.isEmpty == false }), let marked = ink(),
              marked != blank, session.info?.undo_depth == base.undo_depth else { return fail("transient glyph pixels; paths \(canvas.typingLayer?.paint.geometry.paths.count ?? -1), region dark \(ink()?.filter { $0 < 128 }.count ?? -1), baseline \(blank.filter { $0 < 128 }.count)") }
        input.cancelOperation(nil)
        guard !input.hasMarkedText(), session.info?.revision == base.revision else { return fail("cancel mark") }
        input.setMarkedText("한", selectedRange: NSRange(location: 1, length: 0), replacementRange: missing)
        input.insertText("한", replacementRange: missing)
        guard until({ session.info?.revision == base.revision + 1 && canvas.textHit() != nil }),
              let id = canvas.textHit()?.id else { return fail("first commit and selection") }
        input.insertText("글", replacementRange: missing)
        input.insertText("!", replacementRange: missing)
        guard until({ session.info?.revision == base.revision + 3 }) else { return fail("two rapid commits") }
        window.makeFirstResponder(canvas)
        guard until({ shell.canvasTextEditing == nil }), key(36, "\r"),
              until({ window.firstResponder is CanvasTextInputView }), let reopened = window.firstResponder as? CanvasTextInputView,
              reopened.buffer.contents == "한글!" else { return fail("Return edits existing Text") }
        reopened.selectAll(nil)
        reopened.setMarkedText("🎨", selectedRange: NSRange(location: 2, length: 0), replacementRange: missing)
        reopened.insertText("재편집", replacementRange: missing)
        guard until({ session.info?.revision == base.revision + 4 }) else { return fail("replace selection") }
        window.makeFirstResponder(canvas)
        func action(_ command: ShellCommand) -> Bool { shell.router.performShellCommand(MainMenuBuilder.makeItem(for: command, target: nil)); return wait() }
        guard action(.undo), action(.redo), action(.saveNow) else { return fail("history and save") }
        shell.window?.close()
        guard wait(), let next = open(), let w = next.window, let hit = next.canvasHost.canvasView.textHit(id: id),
              next.editCanvasText(hit, at: nil, canvas: next.canvasHost.canvasView), until({ w.firstResponder is CanvasTextInputView }),
              (w.firstResponder as? CanvasTextInputView)?.buffer.contents == "재편집", notices.isEmpty else { return fail("saved text after reopen") }
        print("canvas-ime ok: T click, empty cancel, real marked Hangul outlines, cancel, commit, rapid inserts, Return/range editing, Undo/Redo/save/reopen")
        return true
    }
}
#endif
