import AppKit

/// 캔버스 창을 소유하는 최소 window controller(II.5.3, II.5.4).
///
/// 실제 메뉴·툴바·document 연동은 이후 단계에서 추가합니다.
final class CanvasWindowController: NSWindowController {
    convenience init(runtimeHandle: UInt64?) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Stellar"
        let canvas = CanvasView(frame: NSRect(x: 0, y: 0, width: 960, height: 600))
        canvas.runtimeHandle = runtimeHandle
        window.contentView = canvas
        window.center()
        self.init(window: window)
    }
}
