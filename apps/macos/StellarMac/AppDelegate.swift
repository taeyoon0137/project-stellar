import AppKit

/// macOS 셸의 application delegate(II.5.3).
///
/// 코어 runtime은 `RuntimeHost`가 소유합니다. 시작 시 1회 생성하고,
/// 마지막 창이 닫혀 앱이 종료될 때 `applicationWillTerminate`에서 정확히 1회 파괴합니다.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let runtimeHost = RuntimeHost()
    private var windowController: CanvasWindowController?
    /// 앱 업데이트 background 확인(XVI.9.5). feed URL이 설정된 배포 빌드에서만 만듭니다.
    private var updateScheduler: UpdateScheduler?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // 시작 시 FFI로 코어 버전을 읽어 로그에 기록(II.5.2).
        NSLog("stellar core version: %@", runtimeHost.version() ?? "<unavailable>")

        let controller = CanvasWindowController(
            runtimeHandle: runtimeHost.created ? runtimeHost.handle : nil
        )
        controller.showWindow(nil)
        controller.window?.makeKeyAndOrderFront(nil)
        windowController = controller

        // 업데이트는 background에서 확인만 하고, 다운로드는 사용자가 승인한 뒤에만 시작합니다(XVI.9.5).
        updateScheduler = UpdateConfiguration.makeScheduler()
        updateScheduler?.start()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateScheduler?.stop()
        // 종료 시 runtime을 정확히 1회 파괴(중복 호출은 RuntimeHost가 무시).
        runtimeHost.shutdown()
    }
}
