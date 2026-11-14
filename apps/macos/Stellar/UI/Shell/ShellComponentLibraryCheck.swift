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
        guard let pump = app.pump, pump.waitForPendingRequests(timeout: 10), let shell = app.projectWindow(forPath: root),
              let window = shell.window else { return false }
        let menu = MainMenuBuilder.makeItem(for: .componentLibrary, target: nil)
        guard shell.router.validateMenuItem(menu) else { return false }
        window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        shell.router.performShellCommand(menu)
        guard let library = shell.componentLibrary else { return false }
        while let command = readLine() {
            if command == "done" { window.close(); return pump.waitForPendingRequests(timeout: 10) }
            if command == "revoke" {
                shell.gitSource.updateTrust(false)
                guard pump.waitForPendingRequests(timeout: 10),
                      shell.componentLibrary == nil, !library.panel.isVisible, library.rows.isEmpty,
                      !shell.router.validateMenuItem(menu) else { return false }
            } else if command == "initial" || command == "refresh" {
                if command == "refresh" { library.refresh() }
                let deadline = Date().addingTimeInterval(10)
                while library.isLoading, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
                guard !library.isLoading else { return false }
            } else { return false }
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
