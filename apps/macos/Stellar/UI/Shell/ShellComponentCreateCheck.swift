import AppKit

/// 메뉴의 실제 입력 sheet, 창 재열기 및 component-source CLI와 공유할 프로젝트 fixture.
@MainActor
enum ShellComponentCreateCheck {
    static func run(directory: URL, fromSelection: Bool = false) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        let autosave = ShellCheckAutosave(prefix: "StellarComponentCreate")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        app.presentsWindows = false
        var notices: [String] = []
        app.alertPresenter = { _, message, _ in notices.append(message) }
        guard let pump = app.pump else { return false }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 15) }
        func until(_ condition: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(5)
            while !condition(), Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            return condition()
        }
        func fail(_ detail: String) -> Bool {
            FileHandle.standardError.write(Data(("component-create failed: " + detail + "\n").utf8))
            return false
        }
        let project = directory.appendingPathComponent("Component Project")
        app.createProject(parentDirectory: directory.path, folderName: "Component Project")
        guard wait(), let first = app.projectWindow(forPath: project.path), let firstWindow = first.window,
              let session = first.windowController.windowSession, let info = session.info else { return fail("open") }
        let menu = MainMenuBuilder.makeItem(for: fromSelection ? .componentFromSelection : .newComponent, target: nil)
        guard !first.router.validateMenuItem(menu) else { return fail("restricted menu") }
        let folder = project.appendingPathComponent("design/components")
        var denied = false
        _ = pump.createComponent(session: session.session, name: "Blocked") { event in
            if case .requestFailed = event { denied = true }
        }
        guard wait(), denied, !FileManager.default.fileExists(atPath: folder.path) else { return fail("restricted write") }
        first.gitSource.updateTrust(true)
        guard wait(), first.router.validateMenuItem(menu) == !fromSelection else { return fail("trust") }
        func input(_ shell: ShellProjectWindow, _ name: String, cancel: Bool = false) -> Bool {
            guard let window = shell.window, shell.router.validateMenuItem(menu) else { return false }
            // AppKit은 표시되지 않은 부모 창에는 sheet를 연결하지 않습니다.
            window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
            window.orderBack(nil)
            shell.router.performShellCommand(menu)
            guard until({ window.attachedSheet != nil }), let sheet = window.attachedSheet else { return fail("missing sheet") }
            func nameField(in view: NSView) -> NSTextField? {
                if let field = view as? NSTextField, field.isEditable { return field }
                return view.subviews.lazy.compactMap { nameField(in: $0) }.first
            }
            guard let content = sheet.contentView, let field = nameField(in: content) else { return fail("missing name field") }
            field.stringValue = name
            window.endSheet(sheet, returnCode: cancel ? .alertSecondButtonReturn : .alertFirstButtonReturn)
            guard until({ window.attachedSheet == nil }) else { return fail("sheet still attached") }
            if cancel { return true }
            return until { !notices.isEmpty } && wait()
        }
        if fromSelection {
            firstWindow.setContentSize(NSSize(width: 1300, height: 800))
            firstWindow.contentView?.layoutSubtreeIfNeeded()
            let canvas = first.canvasHost.canvasView
            first.refreshSessionCommands()
            guard let camera = session.viewInfo else { return fail("active view") }
            let center = CGPoint(x: canvas.bounds.midX + (50 - camera.center_x) * camera.zoom,
                                 y: canvas.bounds.midY + (50 - camera.center_y) * camera.zoom)
            guard let viewRoot = canvas.frameHit(at: center)?.id else { return fail("view root") }
            func insert(_ parent: UUID, _ rect: CGRect, rectangle: Bool = false) -> Bool {
                guard let state = session.info else { return false }
                let hit = CanvasFrameHit(isInstance: false, isRectangle: false, id: parent, revision: state.revision,
                    viewID: UUID(uuid: camera.active_view), localToWorld: .identity,
                    parentToWorld: .identity, worldToView: .identity)
                var applied = false
                _ = pump.insertFrame(session: session.session, parent: hit, rect: rect, rectangle: rectangle) { event in
                    if case .windowViewChanged = event { applied = true }
                }
                guard wait(), applied else { return false }
                first.refreshSessionCommands()
                return true
            }
            guard insert(viewRoot, CGRect(x: 40, y: 30, width: 230, height: 97)),
                  let parent = first.selectedComponentFrame.map({ UUID(uuid: $0.node) }),
                  insert(parent, CGRect(x: 12, y: 6, width: 80, height: 34)),
                  let child = first.selectedComponentFrame.map({ UUID(uuid: $0.node) }),
                  insert(child, CGRect(x: 2, y: 3, width: 12, height: 8)),
                  first.selectNodes([parent]), wait() else { return fail("insert selected subtree") }
            let before = session.info
            notices.removeAll()
            guard input(first, "Card"), notices.first?.hasPrefix("design/components/Card-") == true,
                  session.info?.revision == before?.revision, session.info?.undo_depth == before?.undo_depth,
                  session.info?.generation == before?.generation, session.info?.has_unsaved_changes == before?.has_unsaved_changes,
                  first.selectedComponentFrame.map({ UUID(uuid: $0.node) }) == parent,
                  let path = try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil).first,
                  let bytes = try? Data(contentsOf: path) else { return fail("save without touching selection/history") }
            guard insert(child, CGRect(x: 3, y: 4, width: 4, height: 2), rectangle: true),
                  first.selectNodes([parent]), wait() else { return fail("insert unsupported grandchild") }
            let state = session.info
            notices.removeAll()
            guard input(first, "Lossy"), notices.count == 1, !notices[0].hasPrefix("design/components/"),
                  (try? FileManager.default.contentsOfDirectory(atPath: folder.path).count) == 1,
                  (try? Data(contentsOf: path)) == bytes, session.info?.undo_depth == state?.undo_depth,
                  session.info?.revision == state?.revision, session.info?.generation == state?.generation else { return fail("unsupported subtree") }
            print("selected-component ok: three unsaved Frames, menu and sheet, nested Shape rejected, original selection, generation and history preserved")
            return true
        }
        notices.removeAll()
        guard input(first, "Cancelled", cancel: true), !FileManager.default.fileExists(atPath: folder.path) else {
            return fail("cancel")
        }
        for (name, succeeds) in [("Bad/Name", false), ("버튼", true)] {
            notices.removeAll()
            guard input(first, name) else { return fail("first sheet " + name) }
            let created = notices.contains { $0.hasPrefix("design/components/버튼-") }
            guard created == succeeds else { return fail("first outcome " + notices.joined()) }
        }
        guard session.info?.undo_depth == info.undo_depth, session.info?.generation == info.generation else {
            return fail("canvas history or generation changed")
        }
        let files = (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? []
        guard files.count == 1, let existing = files.first, let original = try? Data(contentsOf: existing) else {
            return fail("first source")
        }
        firstWindow.close()
        guard wait() else { return fail("close") }
        app.openProject(atPath: project.path, origin: .application)
        guard wait(), let reopened = app.projectWindow(forPath: project.path), reopened !== first else { return fail("reopen") }
        notices.removeAll()
        guard input(reopened, "버튼"), notices.count == 1, notices[0].hasPrefix("design/components/버튼-"),
              (try? Data(contentsOf: existing)) == original,
              (try? FileManager.default.contentsOfDirectory(atPath: folder.path).count) == 2 else {
            return fail("second source " + notices.joined())
        }
        reopened.window?.close()
        guard wait() else { return fail("close after second create") }
        app.openProject(atPath: project.path, origin: .application)
        guard wait(), app.projectWindow(forPath: project.path)?.router.validateMenuItem(menu) == true else {
            return fail("final reopen")
        }
        print("component-create ok: restricted, cancel, invalid name, two independent same-name files, reopen, generation and undo preserved")
        return true
    }
}
