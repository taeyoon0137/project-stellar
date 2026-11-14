import AppKit

/// Rust 문서 layout과 같은 revision의 glyph 및 line box입니다.
struct CanvasDocumentText {
    private(set) var query: StellarCanvasTextQuery
    let paintToken: UInt64
    private struct Glyph {
        let outline: CGPath
        let origin: CGPoint
    }
    private let glyphs: [Glyph]
    let hitPath: CGPath
    let inkBounds: CGRect

    init?(handle: UInt64, session: UInt64, query: StellarCanvasTextQuery, previous: CanvasDocumentText? = nil) {
        var count = 0
        var token: UInt64 = 0
        guard stellar_window_text_parts(handle, session, query, nil, 0, &count, &token) == STELLAR_OK else { return nil }
        if var cached = previous, cached.paintToken == token {
            cached.query = query
            self = cached
            return
        }
        var elements = [StellarCanvasTextPart](repeating: StellarCanvasTextPart(), count: count)
        let status = elements.withUnsafeMutableBufferPointer {
            stellar_window_text_parts(handle, session, query, $0.baseAddress, $0.count, &count, &token)
        }
        guard status == STELLAR_OK else { return nil }
        var outlines: [(CGPath, CGRect)] = []
        var glyphs: [Glyph] = []
        var path = CGMutablePath()
        let hits = CGMutablePath()
        var bounds = CGRect.null
        for e in elements {
            let (x, y, a, b, c, d) = e.points
            switch e.op {
            case 1: path.move(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 2: path.addLine(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 3:
                path.addCurve(to: CGPoint(x: CGFloat(c), y: CGFloat(d)),
                              control1: CGPoint(x: CGFloat(x), y: CGFloat(y)),
                              control2: CGPoint(x: CGFloat(a), y: CGFloat(b)))
            case 4: path.closeSubpath()
            case 5: hits.addRect(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(a), height: CGFloat(b)))
            case 6:
                outlines.append((path, path.boundingBoxOfPath))
                path = CGMutablePath()
            case 7:
                guard let index = Int(exactly: e.index), outlines.indices.contains(index) else { return nil }
                let (outline, ink) = outlines[index]
                let origin = CGPoint(x: CGFloat(x), y: CGFloat(y))
                glyphs.append(Glyph(outline: outline, origin: origin))
                bounds = bounds.union(ink.offsetBy(dx: origin.x, dy: origin.y))
            default: return nil
            }
        }
        self.query = query
        paintToken = token
        self.glyphs = glyphs
        hitPath = hits
        inkBounds = bounds
    }

    func draw(in context: CGContext) {
        for glyph in glyphs {
            context.saveGState()
            context.translateBy(x: glyph.origin.x, y: glyph.origin.y)
            context.addPath(glyph.outline)
            context.fillPath()
            context.restoreGState()
        }
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
