import AppKit

/// 캔버스 창을 소유하는 최소 window controller(II.5.3, II.5.4, XII.5.1).
///
/// 왼쪽에는 네이티브 필드 패널(`NativeFieldsPanel`), 오른쪽에는 캔버스(`CanvasView`)를 둡니다.
/// 실제 메뉴·툴바·document 연동은 이후 단계에서 추가합니다.
final class CanvasWindowController: NSWindowController {
    /// 패널 기본 너비(point).
    static let panelWidth: CGFloat = 240

    convenience init(runtimeHandle: UInt64?) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Stellar"

        let split = NSSplitView(frame: NSRect(x: 0, y: 0, width: 960, height: 600))
        split.isVertical = true
        split.dividerStyle = .thin

        let panel = NativeFieldsPanel(
            frame: NSRect(x: 0, y: 0, width: CanvasWindowController.panelWidth, height: 600)
        )
        let canvas = CanvasView(
            frame: NSRect(x: 0, y: 0, width: 960 - CanvasWindowController.panelWidth, height: 600)
        )
        canvas.runtimeHandle = runtimeHandle
        split.addArrangedSubview(panel)
        split.addArrangedSubview(canvas)
        split.setHoldingPriority(.defaultHigh, forSubviewAt: 0)

        window.contentView = split
        window.center()
        self.init(window: window)
    }

    /// 창의 네이티브 필드 패널(검증·후속 연결용).
    var fieldsPanel: NativeFieldsPanel? {
        (window?.contentView as? NSSplitView)?.arrangedSubviews.first as? NativeFieldsPanel
    }
}
