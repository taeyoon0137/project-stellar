import AppKit

/// 네이티브 텍스트 컨트롤 검사용 최소 window controller(II.5.3, II.5.4, XII.5.1).
///
/// 왼쪽에는 네이티브 필드 패널(`NativeFieldsPanel`), 오른쪽에는 캔버스(`CanvasView`)를 둡니다.
/// 앱의 프로젝트 창은 셸 프레임(`ShellWindowController`, `ShellAppCoordinator`)이 만들고, 이 창은
/// `--text-input-check`가 `NativeTextControls` 필드를 검사하는 fixture로만 씁니다.
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
