import AppKit

/// macOS 셸의 최소 application delegate.
///
/// II.5.1 단계에서는 창 하나와 AppKit 캔버스 placeholder만 띄웁니다.
/// runtime lifecycle(create/destroy)과 window controller 분리는 II.5.3에서 확장합니다.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // 시작 시 FFI로 코어 버전을 읽어 로그에 기록(II.5.2).
        NSLog("stellar core version: %@", CoreRuntime.version() ?? "<unavailable>")

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Stellar"
        window.contentView = CanvasView(frame: NSRect(x: 0, y: 0, width: 960, height: 600))
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
