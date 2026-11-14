import AppKit

/// Python fixture가 CLI와 실제 Git을 실행하는 동안 동일 NSWindow에서 Refresh하고 화면의 row를 보냅니다.
@MainActor
enum ShellComponentLibraryCheck {
    static func run(directory: URL) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarComponentLibrary")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        let root = directory.appendingPathComponent("Component Project").path
        app.openProject(atPath: root, origin: .application)
        guard let pump = app.pump, pump.waitForPendingRequests(timeout: 10), let opened = app.projectWindow(forPath: root),
              let window = opened.window else { return false }
        var shell = opened
        let menu = MainMenuBuilder.makeItem(for: .componentLibrary, target: nil)
        guard shell.router.validateMenuItem(menu) else { return false }
        window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        shell.router.performShellCommand(menu)
        guard let firstLibrary = shell.componentLibrary else { return false }
        var library = firstLibrary
        while let command = readLine() {
            if command == "done" { shell.window?.close(); return pump.waitForPendingRequests(timeout: 10) }
            if command == "revoke" {
                guard let previous = library.rows.first, let session = shell.windowController.windowSession?.session else { return false }
                shell.gitSource.updateTrust(false)
                guard pump.waitForPendingRequests(timeout: 10),
                      shell.componentLibrary == nil, !library.panel.isVisible, library.rows.isEmpty,
                      !shell.router.validateMenuItem(menu), !library.applySize.isEnabled else { return false }
                let runtime = host.handle
                var rejected: Bool?
                Task { @MainActor in
                    rejected = await Task.detached {
                        CoreRuntime.resizeComponent(handle: runtime, session: session, entry: previous, width: 713, height: 317) != nil
                    }.value
                }
                let deadline = Date().addingTimeInterval(10)
                while rejected == nil, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
                guard rejected == true else { return false }
            } else if command == "reopen" {
                shell.window?.close()
                guard pump.waitForPendingRequests(timeout: 10) else { return false }
                app.openProject(atPath: root, origin: .application)
                guard pump.waitForPendingRequests(timeout: 10), let reopened = app.projectWindow(forPath: root), reopened !== shell else { return false }
                shell = reopened
                shell.window?.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
                shell.router.performShellCommand(menu)
                guard let reopenedLibrary = shell.componentLibrary else { return false }
                library = reopenedLibrary
            } else if command == "initial" || command == "refresh" {
                if command == "refresh" { library.refresh() }
            } else if command.hasPrefix("select ") {
                guard let id = UUID(uuidString: String(command.dropFirst(7))), let index = library.rows.firstIndex(where: { $0.id == id }) else { return false }
                library.table.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
            } else if command.hasPrefix("resize ") {
                let parts = command.split(separator: " ")
                guard parts.count == 3, library.applySize.isEnabled else { return false }
                library.rootWidth.stringValue = String(parts[1])
                library.rootHeight.stringValue = String(parts[2])
                library.applySize.performClick(nil)
            } else { return false }
            let deadline = Date().addingTimeInterval(10)
            while library.isLoading, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            guard !library.isLoading else { return false }
            let rows: [[String: Any]] = library.rows.map {
                ["id": $0.id.uuidString.lowercased(), "root": $0.root.uuidString.lowercased(), "name": $0.name,
                 "node_count": $0.nodeCount, "width": $0.width, "height": $0.height, "filename": $0.filename, "source_hash": $0.hash]
            }
            guard let bytes = try? JSONSerialization.data(withJSONObject: ["entries": rows, "message": library.message.stringValue]) else { return false }
            FileHandle.standardOutput.write(bytes + Data([10]))
        }
        return false
    }
}
