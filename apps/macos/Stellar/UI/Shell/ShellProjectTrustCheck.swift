import AppKit

/// 실제 프로젝트 창의 제목 문서 메뉴 신뢰 항목과 OS 사용자별 저장소를 연결하는 검사입니다.
enum ShellProjectTrustCheck {
    static func run() -> Bool {
        guard let flag = CommandLine.arguments.firstIndex(of: "--project-trust-check"),
              CommandLine.arguments.indices.contains(flag + 1),
              let home = ProcessInfo.processInfo.environment["HOME"], home == CommandLine.arguments[flag + 1],
              FileManager.default.contents(atPath: URL(fileURLWithPath: home).appendingPathComponent("Library/Application Support/xyz.taeyoon.stellar/trust/trusted-projects.yaml").path) == nil else {
            return false
        }
        return MainActor.assumeIsolated { check(home: URL(fileURLWithPath: home, isDirectory: true)) }
    }

    @MainActor private static func check(home: URL) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let projects = ShellSavedProjectFixture(prefix: "StellarProjectTrustCheck")
        defer { projects.remove() }
        guard let path = projects.unsavedProject("Trust") else { return false }
        let runtime = RuntimeHost()
        defer { runtime.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: runtime.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarProjectTrustCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        let list = home.appendingPathComponent("Library/Application Support/xyz.taeyoon.stellar/trust/trusted-projects.yaml")
        guard let pump = coordinator.pump else { return false }
        func openWindow() -> ShellProjectWindow? {
            coordinator.openProject(atPath: path, origin: .application)
            guard pump.waitForPendingRequests(timeout: 10) else { return nil }
            return coordinator.projectWindow(forPath: path)
        }
        // 제목 문서 메뉴(상단 바 제목을 누르면 열림)의 항목을 고릅니다(UX 명세 3.3).
        func chooseTitleMenuItem(_ window: ShellProjectWindow, _ id: ShellCommandID) -> Bool {
            window.windowController.flushPendingSnapshot()
            guard let menu = window.windowController.toolbarController.titleView.titleButton.menu,
                  let index = menu.items.firstIndex(where: { ($0.representedObject as? String) == id.rawValue }),
                  menu.items[index].isEnabled else { return false }
            menu.performActionForItem(at: index)
            return true
        }
        func trustButton(_ window: ShellProjectWindow, approving: Bool) -> Bool {
            guard let owner = window.window else { return failed("창 없음") }
            owner.orderFront(nil)
            guard chooseTitleMenuItem(window, ShellProjectCommandID.trustProject) else { return failed("제목 메뉴 신뢰 항목") }
            return respondToTrustSheet(in: owner, approving: approving) && pump.waitForPendingRequests(timeout: 10)
        }
        func failed(_ step: String) -> Bool {
            FileHandle.standardError.write(Data("project-trust 실패: \(step)\n".utf8))
            return false
        }
        guard let first = openWindow(), !FileManager.default.fileExists(atPath: list.path) else { return failed("최초 restricted 열기") }
        guard trustButton(first, approving: false), !FileManager.default.fileExists(atPath: list.path) else { return failed("신뢰 sheet 취소") }
        guard trustButton(first, approving: true), FileManager.default.fileExists(atPath: list.path),
              first.dataSource.currentSnapshot.subtitle.isEmpty else { return failed("신뢰 sheet 승인") }
        first.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), let reopened = openWindow(),
              reopened !== first, reopened.dataSource.currentSnapshot.subtitle.isEmpty else { return failed("신뢰 등록 후 재열기") }
        guard chooseTitleMenuItem(reopened, ShellProjectCommandID.revokeTrust) else { return failed("제목 메뉴 신뢰 철회 항목") }
        guard pump.waitForPendingRequests(timeout: 10),
              !reopened.dataSource.currentSnapshot.subtitle.isEmpty else { return failed("신뢰 철회") }
        reopened.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), let last = openWindow(),
              !last.dataSource.currentSnapshot.subtitle.isEmpty else { return failed("철회 뒤 재열기") }
        last.window?.close()
        guard pump.waitForPendingRequests(timeout: 10) else { return false }
        print("project-trust ok: cancel, approve, reopen, revoke, reopen")
        return true
    }

    @MainActor static func respondToTrustSheet(in owner: NSWindow, approving: Bool) -> Bool {
        let until = Date().addingTimeInterval(3)
        while owner.attachedSheet == nil && Date() < until {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        guard let view = owner.attachedSheet?.contentView,
              let approve = descendantButton(view, title: String(localized: "Trust Project")),
              let cancel = descendantButton(view, title: String(localized: "Cancel")) else { return false }
        (approving ? approve : cancel).performClick(nil)
        let deadline = Date().addingTimeInterval(5)
        while owner.attachedSheet != nil && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        return owner.attachedSheet == nil
    }

    @MainActor private static func descendantButton(_ view: NSView?, title: String) -> NSButton? {
        guard let view else { return nil }
        if let button = view as? NSButton, button.title == title { return button }
        for child in view.subviews {
            if let button = descendantButton(child, title: title) { return button }
        }
        return nil
    }
}
