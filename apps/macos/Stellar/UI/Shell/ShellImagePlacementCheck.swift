import AppKit
import UniformTypeIdentifiers

@MainActor
enum ShellImagePlacementCheck {
    // One fixed 24×16 PNG test fixture. Its two colors catch vertical orientation errors.
    static let sample = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAABgAAAAQCAIAAACDRijCAAAAHklEQVR4nGN4JqZFFcQwatCINkjN8RFV0KhBI9ogADpE01CaPNZgAAAAAElFTkSuQmCC")!

    static func run(directory: URL) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarPNG")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        guard let pump = app.pump else { return false }
        let project = directory.appendingPathComponent("Image Placement")
        let source = directory.appendingPathComponent("Swatch.png")
        func fail(_ text: String) -> Bool { FileHandle.standardError.write(Data("PNG check: \(text)\n".utf8)); return false }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 12) }
        func until(_ predicate: () -> Bool) -> Bool {
            let end = Date().addingTimeInterval(7)
            while !predicate() && Date() < end { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
            return predicate()
        }
        func point(_ x: CGFloat, _ y: CGFloat, in p: ShellProjectWindow) -> CGPoint {
            let canvas = p.canvasHost.canvasView
            let camera = p.windowController.windowSession?.viewInfo
            return CGPoint(x: canvas.bounds.midX + (x - (camera?.center_x ?? 0)) * (camera?.zoom ?? 1),
                           y: canvas.bounds.midY + (y - (camera?.center_y ?? 0)) * (camera?.zoom ?? 1))
        }
        func layout(_ p: ShellProjectWindow) {
            p.window?.setContentSize(.init(width: 1300, height: 800))
            p.window?.contentView?.layoutSubtreeIfNeeded()
        }
        func hasPixels(_ p: ShellProjectWindow) -> Bool {
            let canvas = p.canvasHost.canvasView
            guard let window = p.window, let bitmap = ShellVisibleFramesCheck.bitmap(canvas, in: window) else { return false }
            func color(_ y: CGFloat) -> NSColor? {
                let pos = point(52, y, in: p)
                return bitmap.colorAt(x: Int(pos.x * CGFloat(bitmap.pixelsWide) / canvas.bounds.width),
                                      y: Int(pos.y * CGFloat(bitmap.pixelsHigh) / canvas.bounds.height))?.usingColorSpace(.sRGB)
            }
            guard let top = color(44), let bottom = color(52) else { return false }
            return top.redComponent > 0.7 && top.blueComponent < 0.35 && bottom.blueComponent > 0.7 && bottom.redComponent < 0.35
        }
        do { try sample.write(to: source) } catch { return fail("write fixture") }
        app.createProject(parentDirectory: directory.path, folderName: project.lastPathComponent)
        guard wait(), let p = app.projectWindow(forPath: project.path), let session = p.windowController.windowSession else { return fail("create") }
        layout(p)
        guard !p.router.dispatch(.selectImageTool) else { return fail("restricted tool") }
        p.gitSource.updateTrust(true)
        guard wait(), until({ p.gitSource.trusted }), let before = session.info else { return fail("trust") }
        var picked = 0
        p.choosePNG = { panel, owner, completion in
            guard owner === p.window, panel.allowedContentTypes == [.png], !panel.allowsMultipleSelection else { return }
            picked += 1
            completion(picked == 1 ? nil : source)
        }
        guard p.router.dispatch(.selectImageTool), p.toolsModel.snapshot.activeTool == .move,
              p.router.dispatch(.selectImageTool), until({ p.pickedPNG != nil }) else { return fail("picker cancel / decode") }
        let canvas = p.canvasHost.canvasView
        let click = point(40, 40, in: p)
        var errors = 0
        p.alertPresenter = { _, _, _ in errors += 1 }
        do { try Data("changed after selection".utf8).write(to: source, options: .atomic) } catch { return fail("change source") }
        guard ShellCanvasMoveCheck.mouse(canvas, .leftMouseDown, at: click), wait(), errors == 1,
              session.info?.revision == before.revision, session.info?.undo_depth == before.undo_depth
        else { return fail("stale source changed history") }
        do { try sample.write(to: source, options: .atomic) } catch { return fail("restore fixture") }
        guard p.router.dispatch(.selectImageTool), until({ p.pickedPNG != nil }) else { return fail("fresh selection") }
        guard ShellCanvasMoveCheck.mouse(canvas, .leftMouseDown, at: point(40, 40, in: p)),
              wait(), session.info?.revision == before.revision + 1, session.info?.undo_depth == before.undo_depth + 1,
              let hit = canvas.frameHit(at: point(45, 45, in: p)), hit.isImage,
              canvas.frameHit(at: point(65, 45, in: p))?.id != hit.id,
              until({ hasPixels(p) }) else { return fail("position, pixels or history") }
        guard p.router.dispatch(.undo), wait(), canvas.frameHit(at: point(45,45,in:p))?.id != hit.id,
              p.router.dispatch(.redo), wait(), canvas.frameHit(at: point(45,45,in:p))?.id == hit.id,
              p.router.dispatch(.saveNow), wait(), session.info?.has_unsaved_changes == 0 else { return fail("undo, redo, save") }
        do { try FileManager.default.removeItem(at: source) } catch { return fail("remove original") }
        p.window?.close()
        guard wait() else { return fail("close") }
        app.openProject(atPath: project.path, origin: .application)
        guard wait(), let reopened = app.projectWindow(forPath: project.path), reopened !== p else { return fail("reopen") }
        layout(reopened)
        guard until({ reopened.gitSource.trusted && hasPixels(reopened) }),
              reopened.canvasHost.canvasView.frameHit(at: point(45,45,in:reopened))?.id == hit.id
        else { return fail("reopened project content after removing original") }
        print("png-place ok: picker cancellation, trusted PNG, 24×16 exact location, red/blue pixel orientation, Undo/Redo/Save, reopened from asset without source; node=\(hit.id.uuidString.lowercased())")
        return true
    }
}
