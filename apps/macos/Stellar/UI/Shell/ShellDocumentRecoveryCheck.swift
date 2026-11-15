#if DEBUG
import AppKit

@MainActor
enum ShellDocumentRecoveryCheck {
    static func run(_ home: URL, mode: String) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = mode == "restore" || mode == "export"
        let preferences = ShellCheckAutosave(prefix: "StellarRecoveryCheck")
        defer { preferences.removeRecords() }
        app.windowConfiguration = { preferences.configuration }
        func fail(_ text: String, line: Int = #line) -> Bool {
            FileHandle.standardError.write(Data("recovery check \(mode) at \(line): \(text)\n".utf8))
            return false
        }
        let root = home.appendingPathComponent("Crash Project")
        if mode == "edit" { app.createProject(parentDirectory: home.path, folderName: root.lastPathComponent) }
        else { app.openProject(atPath: root.path, origin: .application) }
        guard let pump = app.pump, pump.waitForPendingRequests(timeout: 15), let shell = app.projectWindow(forPath: root.path),
              let window = shell.window, let session = shell.windowController.windowSession else { return fail("window") }
        window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        window.setContentSize(NSSize(width: 1300, height: 850))
        window.contentView?.layoutSubtreeIfNeeded()
        func until(_ predicate: () -> Bool) -> Bool {
            let end = Date().addingTimeInterval(15)
            while !predicate(), Date() < end { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            return predicate()
        }
        func say(_ s: String) { FileHandle.standardOutput.write(Data((s + "\n").utf8)) }
        if mode == "edit" {
            if !shell.gitSource.trusted { shell.gitSource.updateTrust(true) }
            guard pump.waitForPendingRequests(timeout: 15), let camera = session.viewInfo, let info = session.info else {return fail("precondition")}
            shell.refreshSessionCommands()
            let canvas = shell.canvasHost.canvasView
            let point = CGPoint(x: canvas.bounds.midX + (50 - camera.center_x) * camera.zoom,
                                y: canvas.bounds.midY + (50 - camera.center_y) * camera.zoom)
            guard let rootID = canvas.frameHit(at: point)?.id else {return fail("precondition")}
            let hit = CanvasFrameHit(isInstance: false, isRectangle: false, id: rootID, revision: info.revision, viewID: UUID(uuid: camera.active_view), localToWorld: .identity, parentToWorld: .identity, worldToView: .identity)
            guard session.insertFrame(parent: hit, rect: CGRect(x: 31, y: 27, width: 199, height: 85), rectangle: false, completion: { _ in shell.refreshSessionCommands() }),
                  pump.waitForPendingRequests(timeout: 15), let selection = shell.selectedComponentFrame,
                  session.info?.has_unsaved_changes == 1 else {return fail("precondition")}
            say(UUID(uuid: selection.node).uuidString.lowercased())
            _ = readLine() // fixture는 체크포인트 atomic rename을 관찰한 뒤 SIGKILL합니다.
            return false
        }
        window.orderBack(nil)
        if mode == "live" {
            let (status, bytes) = CoreRuntime.probeRecovery(handle: host.handle, session: session.session, capacity: 256)
            return status == STELLAR_OK && String(decoding: bytes, as: UTF8.self) == "0"
        }
        guard until({ window.attachedSheet != nil }), let prompt = window.attachedSheet else {
            let (status, bytes) = CoreRuntime.probeRecovery(handle: host.handle, session: session.session, capacity: 512)
            return fail("missing prompt. probe=\(status.rawValue) \(String(decoding: bytes, as: UTF8.self))")
        }
        func buttons(_ view: NSView) -> [String] {
            (view as? NSButton).map { [$0.title] } ?? view.subviews.flatMap(buttons)
        }
        let titles = prompt.contentView.map(buttons) ?? []
        if mode == "restore" {
            guard titles.contains("Restore Changes") else {return fail("missing Restore action: \(titles)")}
            window.endSheet(prompt, returnCode: .alertFirstButtonReturn)
            guard until({ session.info?.has_unsaved_changes == 1 }) else {
                let text = window.attachedSheet?.contentView?.subviews.description ?? "no sheet"
                return fail("restore dirty: \(text)")
            }
            guard let selection = shell.selectedComponentFrame,
                  shell.performSessionCommand(.saveNow), pump.waitForPendingRequests(timeout: 15),
                  session.info?.has_unsaved_changes == 0 else {return fail("precondition")}
            say(UUID(uuid: selection.node).uuidString.lowercased())
            return true
        }
        guard mode == "export", !titles.contains("Restore Changes"), titles.contains("Save Recovered Document…") else {return fail("precondition")}
        window.endSheet(prompt, returnCode: .alertFirstButtonReturn)
        guard until({ window.attachedSheet is NSSavePanel }), let panel = window.attachedSheet as? NSSavePanel else {return fail("precondition")}
        // macOS의 별도 프로세스 Save 패널은 제어 입력을 공개하지 않습니다. 패널 표시를 확인하고
        // 취소 뒤 실제 패널이 URL을 반환할 때와 같은 coordinator 진입점으로 fixture 경로를 전달합니다.
        window.endSheet(panel, returnCode: .cancel)
        guard until({ window.attachedSheet == nil }) else { return fail("save panel cancellation") }
        let (status, token) = CoreRuntime.probeRecovery(handle: host.handle, session: session.session, capacity: 512)
        guard status == STELLAR_OK, token.count > 1, token.first == 50 else { return fail("separate copy ticket") }
        shell.submitDocumentRecovery(String(decoding: token.dropFirst(), as: UTF8.self), destination: home.appendingPathComponent("Recovered"))
        let pointer = home.appendingPathComponent("Recovered/design/current").path
        guard until({ FileManager.default.fileExists(atPath: pointer) }), pump.waitForPendingRequests(timeout: 15),
              session.info?.has_unsaved_changes == 0 else {return fail("precondition")}
        say("exported")
        return true
    }
}
#endif
