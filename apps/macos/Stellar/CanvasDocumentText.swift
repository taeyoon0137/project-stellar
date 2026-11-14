import AppKit

/// Rust 문서 layout과 같은 revision의 glyph 및 line box입니다.
struct CanvasDocumentText {
    let query: StellarCanvasTextQuery
    let glyphs: [CGPath]
    let hitPath: CGPath
    let inkBounds: CGRect

    init?(handle: UInt64, session: UInt64, query: StellarCanvasTextQuery) {
        var count = 0
        guard stellar_window_text_paint(handle, session, query, nil, 0, &count) == STELLAR_OK else { return nil }
        var elements = [StellarCanvasTextElement](repeating: StellarCanvasTextElement(), count: count)
        let status = elements.withUnsafeMutableBufferPointer {
            stellar_window_text_paint(handle, session, query, $0.baseAddress, $0.count, &count)
        }
        guard status == STELLAR_OK else { return nil }
        guard let geometry = CanvasGlyphPaths(elements) else { return nil }
        self.query = query
        glyphs = geometry.paths
        hitPath = geometry.hits
        inkBounds = geometry.bounds
    }
}

extension CanvasFrameHit {
    /// Text hit의 local 좌표를 문서 전체 scalar index로 변환합니다.
    func textOffset(handle: UInt64, session: UInt64, at local: CGPoint) -> (offset: UInt32, upstream: Bool)? {
        guard let textQuery else { return nil }
        var scalar: UInt32 = 0
        var affinity: UInt32 = 0
        guard
            stellar_window_text_hit(handle, session, textQuery, Float(local.x), Float(local.y), &scalar, &affinity)
                == STELLAR_OK
        else { return nil }
        return (scalar, affinity != 0)
    }

    /// 같은 layout의 local caret 또는 선택 사각형입니다.
    func textRects(handle: UInt64, session: UInt64, start: UInt32, end: UInt32, upstream: Bool) -> [CGRect]? {
        guard let textQuery else { return nil }
        var count = 0
        let affinity: UInt32 = upstream ? 1 : 0
        guard stellar_window_text_rects(handle, session, textQuery, start, end, affinity, nil, 0, &count) == STELLAR_OK
        else { return nil }
        var rects = [(Float, Float, Float, Float)](repeating: (0, 0, 0, 0), count: count)
        let status = rects.withUnsafeMutableBufferPointer {
            stellar_window_text_rects(
                handle, session, textQuery, start, end, affinity, $0.baseAddress, $0.count, &count)
        }
        guard status == STELLAR_OK else { return nil }
        return rects.map { CGRect(x: CGFloat($0.0), y: CGFloat($0.1), width: CGFloat($0.2), height: CGFloat($0.3)) }
    }
}


struct CanvasGlyphPaths {
    let paths: [CGPath]
    let hits: CGPath
    let bounds: CGRect

    init?<Elements: Sequence>(_ elements: Elements) where Elements.Element == StellarCanvasTextElement {
        var paths: [CGPath] = []
        var path = CGMutablePath()
        let hits = CGMutablePath()
        var bounds = CGRect.null
        for e in elements {
            let (x, y, a, b, c, d) = e.points
            switch e.op {
            case 1: path.move(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 2: path.addLine(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 3:
                path.addCurve(
                    to: CGPoint(x: CGFloat(c), y: CGFloat(d)), control1: CGPoint(x: CGFloat(x), y: CGFloat(y)),
                    control2: CGPoint(x: CGFloat(a), y: CGFloat(b)))
            case 4: path.closeSubpath()
            case 5: hits.addRect(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(a), height: CGFloat(b)))
            case 6:
                paths.append(path)
                bounds = bounds.union(path.boundingBoxOfPath)
                path = CGMutablePath()
            default: return nil
            }
        }
        self.paths = paths
        self.hits = hits
        self.bounds = bounds
    }
}

struct CanvasTypingPaint {
    let geometry: CanvasGlyphPaths
    let fill: CGColor
    let caret: CGRect?

    func draw(in context: CGContext) {
        context.setFillColor(fill)
        for path in geometry.paths { context.addPath(path); context.fillPath() }
        if var caret {
            caret.size.width = max(caret.width, hypot(context.convertToUserSpace(CGSize(width: 1, height: 0)).width,
                                                       context.convertToUserSpace(CGSize(width: 1, height: 0)).height))
            context.fill(caret)
        }
    }
}

private final class CanvasPaintReceiver {
    var paint: CanvasTypingPaint?
}

extension CanvasTypingPaint {
    static func prepare(handle: UInt64, session: UInt64, view: UUID, target: UUID, revision: UInt64, newText: Bool,
                        start: UInt32, end: UInt32, value: String, caret: UInt32) -> CanvasTypingPaint? {
        let receiver = CanvasPaintReceiver()
        let status = withUnsafeBytes(of: view.uuid) { v in
            withUnsafeBytes(of: target.uuid) { n in
                Array(value.utf8).withUnsafeBufferPointer { bytes in
                    stellar_text_input_paint(handle, session, v.bindMemory(to: UInt8.self).baseAddress,
                        n.bindMemory(to: UInt8.self).baseAddress, revision, newText ? 1 : 0, start, end,
                        bytes.baseAddress, bytes.count, caret, Unmanaged.passUnretained(receiver).toOpaque()) { box, fill, caret, rows, count in
                            guard let box, let color = canvasColor(fill),
                                  let geometry = CanvasGlyphPaths(UnsafeBufferPointer(start: rows, count: count)) else { return }
                            let position = caret.map { CGRect(x: CGFloat($0[0]), y: CGFloat($0[1]), width: CGFloat($0[2]), height: CGFloat($0[3])) }
                            Unmanaged<CanvasPaintReceiver>.fromOpaque(box).takeUnretainedValue().paint =
                                CanvasTypingPaint(geometry: geometry, fill: color, caret: position)
                        }
                }
            }
        }
        return status == STELLAR_OK ? receiver.paint : nil
    }
}
