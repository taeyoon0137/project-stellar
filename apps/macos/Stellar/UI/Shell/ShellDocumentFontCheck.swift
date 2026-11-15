import AppKit

/// qa_sample_project의 Desktop/Header/Title("Stellar QA Sample")를 일반 창에서 확인합니다.
@MainActor
enum ShellDocumentFontCheck {
    static func run(fixture: URL, font: URL) -> Bool {
        let copies = ShellSavedProjectFixture(prefix: "StellarDocumentFont")
        defer { copies.remove() }
        guard let path = copies.copy(fixture, as: "Document Font") else { return false }
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarDocumentFont")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        let handle = host.handle
        guard let pump = app.pump, handle != 0 else { return false }
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("document-font check: \(message)\n".utf8))
            return false
        }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 15) }
        func open() -> ShellProjectWindow? {
            app.openProject(atPath: path, origin: .application)
            guard wait(), let project = app.projectWindow(forPath: path), let window = project.window else {
                return nil
            }
            window.setContentSize(CGSize(width: 1280, height: 790))
            window.contentView?.layoutSubtreeIfNeeded()
            project.performSessionCommand(.zoomToFit)
            return wait() ? project : nil
        }
        func sample(_ project: ShellProjectWindow) -> (CanvasFrameHit?, [UInt8])? {
            let canvas = project.canvasHost.canvasView
            guard let window = project.window, let session = project.windowController.windowSession,
                let camera = session.viewInfo, let rep = ShellVisibleFramesCheck.bitmap(canvas, in: window)
            else { return nil }
            func screen(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
                CGPoint(
                    x: (x - CGFloat(camera.center_x)) * CGFloat(camera.zoom) + canvas.bounds.midX,
                    y: (y - CGFloat(camera.center_y)) * CGFloat(camera.zoom) + canvas.bounds.midY)
            }
            var data: [UInt8] = []
            for y in 28..<61 {
                for x in 88..<250 {
                    let point = screen(CGFloat(x), CGFloat(y))
                    let col = Int(point.x * CGFloat(rep.pixelsWide) / canvas.bounds.width)
                    let row = Int(point.y * CGFloat(rep.pixelsHigh) / canvas.bounds.height)
                    guard col >= 0, row >= 0, col < rep.pixelsWide, row < rep.pixelsHigh else { return nil }
                    var pixel = [Int](repeating: 0, count: rep.samplesPerPixel)
                    rep.getPixel(&pixel, atX: col, y: row)
                    data.append(UInt8(clamping: pixel[0]))
                }
            }
            return (canvas.frameHit(at: screen(98, 41)), data)
        }
        guard let project = open(), let session = project.windowController.windowSession,
            let base = session.info
        else { return fail("open") }
        var fill = StellarCanvasColor()
        fill.space = 1
        fill.rgba = (0, 0, 0, 1)
        guard !project.applyDocumentFont(font.path, face: 0, size: 1536, fill: fill, revision: base.revision),
            let before = sample(project), before.0?.isText != true
        else { return fail("restricted") }
        project.gitSource.updateTrust(true)
        guard wait(), project.gitSource.trusted,
            project.applyDocumentFont(font.path, face: 0, size: 1536, fill: fill, revision: base.revision), wait(),
            session.info?.undo_depth == base.undo_depth + 1, let after = sample(project), let hit = after.0, hit.isText,
            zip(before.1, after.1).filter({ Int($0.0) - Int($0.1) > 35 }).count > 40,
            let offset = hit.textOffset(handle: handle, session: session.session, at: CGPoint(x: 12, y: 9)),
            let caret = hit.textRects(
                handle: handle, session: session.session, start: offset.offset, end: offset.offset,
                upstream: offset.upstream), caret.count == 1, caret[0].height > 0,
            let selection = hit.textRects(handle: handle, session: session.session, start: 0, end: 7, upstream: false),
            selection.contains(where: { $0.width > 0 })
        else { return fail("font selection, glyph pixels, line and scalar geometry") }
        guard project.performSessionCommand(.undo), wait(), sample(project)?.1 == before.1,
            hit.textOffset(handle: handle, session: session.session, at: CGPoint(x: 1, y: 1)) == nil,
            project.performSessionCommand(.redo), wait(), sample(project)?.1 == after.1,
            project.performSessionCommand(.saveNow), wait(), session.info?.has_unsaved_changes == 0
        else { return fail("undo, stale geometry, redo and save") }
        project.window?.close()
        guard wait(), let reopened = open(), let verified = sample(reopened), verified.0?.id == hit.id,
            verified.0?.isText == true, verified.1 == after.1
        else { return fail("saved IDs, project font and glyph pixels after reopen") }
        print(
            "document-font ok: restricted, explicit face, real outlines, hit/caret/selection, stale revision, undo/redo, save/reopen"
        )
        return true
    }
}
