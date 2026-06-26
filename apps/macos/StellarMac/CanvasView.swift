import AppKit

/// AppKit 캔버스 호스트 placeholder.
///
/// 렌더링은 하지 않습니다(이후 단계에서 GPU 렌더러를 연결). II.5.4에서
/// backing scale·drawable size 변경을 core에 전달하는 구조를 추가합니다.
final class CanvasView: NSView {
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
    }
}
