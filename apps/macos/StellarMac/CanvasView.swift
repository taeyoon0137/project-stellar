import AppKit

/// AppKit 캔버스 호스트(II.5.4).
///
/// 렌더링은 하지 않습니다. backing scale과 drawable 픽셀 크기 변경을
/// `stellar_runtime_set_drawable_size`로 코어에 전달하는 구조만 만듭니다.
final class CanvasView: NSView {
    /// drawable 변경을 전달할 대상 runtime handle. 미설정 시 전달하지 않습니다.
    var runtimeHandle: UInt64?

    override var isFlipped: Bool { true }

    /// 현재 bounds(point)와 backing scale로부터 drawable 픽셀 크기를 계산합니다.
    func currentDrawable() -> (width: UInt32, height: UInt32, scale: Double) {
        let scale = Double(window?.backingScaleFactor ?? 1.0)
        let size = CanvasMetrics.pixelSize(pointSize: bounds.size, scale: scale)
        return (size.width, size.height, scale)
    }

    /// 계산한 drawable 픽셀 크기·scale을 코어로 전달합니다.
    func forwardDrawableSize() {
        guard let handle = runtimeHandle else { return }
        let drawable = currentDrawable()
        _ = stellar_runtime_set_drawable_size(handle, drawable.width, drawable.height, drawable.scale)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        forwardDrawableSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        forwardDrawableSize()
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        forwardDrawableSize()
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
    }
}
