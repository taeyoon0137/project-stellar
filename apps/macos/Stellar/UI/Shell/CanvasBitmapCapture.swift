import AppKit

/// 남은 앱 검사들이 함께 쓰는 캔버스 bitmap 캡처 helper입니다(삭제한 가시 Frame 검사에서 분리).
@MainActor
enum CanvasBitmapCapture {
    static func bitmap(_ canvas: NSView, in window: NSWindow) -> NSBitmapImageRep? {
        // 픽셀 참조는 현재 사용자 모니터 ICC의 색역/의도에 영향을 받지 않는 명시적 sRGB destination입니다.
        let scale = window.backingScaleFactor
        let width = Int(ceil(canvas.bounds.width * scale)), height = Int(ceil(canvas.bounds.height * scale))
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let cg = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                 space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        cg.scaleBy(x: scale, y: scale)
        canvas.displayIgnoringOpacity(canvas.bounds, in: NSGraphicsContext(cgContext: cg, flipped: false))
        return cg.makeImage().map(NSBitmapImageRep.init(cgImage:))
    }
}
