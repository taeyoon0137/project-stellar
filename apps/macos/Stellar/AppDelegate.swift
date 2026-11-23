import AppKit

/// macOS 셸의 application delegate(II.5.3).
///
/// UI는 재설계 전까지 비워 둡니다(ADR-0098). 이 셸은 코어 runtime 수명, 업데이트 확인, 앱 메뉴(종료)와
/// 빈 창 하나만 가집니다.
///
/// 코어 runtime은 `RuntimeHost`가 소유합니다. 시작 시 1회 생성하고, 앱이 종료될 때 `applicationWillTerminate`에서
/// 정확히 1회 파괴합니다.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let runtimeHost = RuntimeHost()
    private var window: NSWindow?
    /// 앱 업데이트(XVI.9.5, ADR-0017). 구성이 완전한 빌드에서만 Sparkle updater를 시작합니다.
    private var updateController: AppUpdateController?

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApplication.shared.mainMenu = Self.makeMainMenu()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // 시작 시 FFI로 코어 버전을 읽어 로그에 기록(II.5.2).
        NSLog("stellar core version: %@", runtimeHost.version() ?? "<unavailable>")
        showWindow()

        // 업데이트는 background에서 확인만 하고, 다운로드·설치는 사용자가 승인한 뒤에만 합니다(XVI.9.5).
        let updates = AppUpdateController()
        NSLog("stellar update: %@", updates.summary)
        updateController = updates
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { showWindow() }
        return true
    }

    /// 다른 앱으로 전환하면 대기 중인 자동 저장을 바로 저장합니다(기술 아키텍처 22.1).
    func applicationDidResignActive(_ notification: Notification) {
        runtimeHost.flushAutosave()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        // 종료 시 runtime을 정확히 1회 파괴(중복 호출은 RuntimeHost가 무시).
        runtimeHost.shutdown()
    }

    private func showWindow() {
        if let window {
            window.makeKeyAndOrderFront(nil)
            return
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Stellar"
        window.isReleasedWhenClosed = false
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
    }

    private static func makeMainMenu() -> NSMenu {
        let mainMenu = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(
            withTitle: String(localized: "Quit Stellar", comment: "App menu item"),
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        appItem.submenu = appMenu
        mainMenu.addItem(appItem)
        return mainMenu
    }
}
