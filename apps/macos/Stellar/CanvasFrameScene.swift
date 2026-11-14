import AppKit

struct CanvasFrameHit {
    let id: UUID
    let revision: UInt64
    let viewID: UUID
    let parentToWorld: CGAffineTransform
    let worldToView: CGAffineTransform
}

private typealias CanvasAffine = (Float, Float, Float, Float, Float, Float)

private func transform(_ m: CanvasAffine) -> CGAffineTransform {
    CGAffineTransform(a: CGFloat(m.0), b: CGFloat(m.1), c: CGFloat(m.2), d: CGFloat(m.3), tx: CGFloat(m.4), ty: CGFloat(m.5))
}

private func color(_ value: StellarCanvasColor) -> CGColor? {
    guard value.space != 0 else { return nil }
    let space = value.space == 2 ? CGColorSpace.extendedDisplayP3 : CGColorSpace.extendedSRGB
    guard let space = CGColorSpace(name: space) else { return nil }
    let c = value.rgba
    return CGColor(colorSpace: space, components: [CGFloat(c.0), CGFloat(c.1), CGFloat(c.2), CGFloat(c.3)])
}

/// 둥근 경계를 하나 만들어 그리기, 자식 clip과 picking에서 공유합니다. 반경은 코어의 실효 값입니다.
private func outline(_ row: StellarCanvasFrame) -> CGPath {
    let w = CGFloat(row.size.0), h = CGFloat(row.size.1)
    let (tl, tr, br, bl) = row.radii
    if tl == 0 && tr == 0 && br == 0 && bl == 0 {
        return CGPath(rect: CGRect(x: 0, y: 0, width: w, height: h), transform: nil)
    }
    let p = CGMutablePath()
    p.move(to: CGPoint(x: CGFloat(tl), y: 0))
    // 접점이 이미 현재점인 경우도 있으므로 시작·끝점을 명시합니다.
    func arc(_ x: CGFloat, _ y: CGFloat, _ r: Float, _ start: CGFloat, _ end: CGFloat) {
        let radius = CGFloat(r)
        p.addLine(to: CGPoint(x: x + radius * cos(start), y: y + radius * sin(start)))
        if r > 0 { p.addArc(center: CGPoint(x: x, y: y), radius: radius, startAngle: start, endAngle: end, clockwise: false) }
    }
    arc(w - CGFloat(tr), CGFloat(tr), tr, -.pi / 2, 0)
    arc(w - CGFloat(br), h - CGFloat(br), br, 0, .pi / 2)
    arc(CGFloat(bl), h - CGFloat(bl), bl, .pi / 2, .pi)
    arc(CGFloat(tl), CGFloat(tl), tl, .pi, 3 * .pi / 2)
    p.closeSubpath()
    return p
}

private struct ProjectedFrame {
    let id: UUID
    let parent: Int?
    let world: CGAffineTransform
    let localBounds: CGRect
    let localPath: CGPath
    let worldPath: CGPath
    let clips: Bool
    let pickable: Bool
    let fill: CGColor?
    let border: CGColor?
    let borderWidth: CGFloat
    var lastDescendant: Int

    init(_ row: StellarCanvasFrame, index: Int) {
        id = UUID(uuid: row.id)
        parent = row.parent == .max ? nil : Int(row.parent)
        var world = transform(row.world)
        self.world = world
        localBounds = CGRect(x: 0, y: 0, width: CGFloat(row.size.0), height: CGFloat(row.size.1))
        localPath = outline(row)
        worldPath = localPath.copy(using: &world)!
        clips = row.flags & 1 != 0
        pickable = row.flags & 2 != 0
        fill = color(row.fill)
        border = color(row.border)
        borderWidth = CGFloat(row.border_width)
        lastDescendant = index
    }
}

/// 한 창에서 마지막으로 표시한 scene. 카메라 또는 선택만 바뀌면 path를 다시 만들지 않습니다.
final class CanvasFrameScene {
    private var frames: [ProjectedFrame] = []
    private var viewID: UUID?
    private var revision: UInt64 = 0
    private var session: UInt64?
    private var primary: UUID?
    private var previewID: UUID?
    private var previewOffset = CGVector.zero

    func setDragPreview(id: UUID?, offset: CGVector) {
        previewID = id
        let inverse = worldToView.inverted()
        previewOffset = CGVector(dx: inverse.a * offset.dx + inverse.c * offset.dy,
                                 dy: inverse.b * offset.dx + inverse.d * offset.dy)
    }
    private(set) var worldToView = CGAffineTransform.identity
    private(set) var skipped: UInt32 = 0

    func clear() {
        frames.removeAll()
        viewID = nil
        session = nil
        skipped = 0
        primary = nil
        previewID = nil
    }

    func refresh(handle: UInt64, session: UInt64, size: CGSize) {
        var info = StellarCanvasInfo()
        let width = Float(size.width), height = Float(size.height)
        guard stellar_window_canvas(handle, session, width, height, &info, nil, 0) == STELLAR_OK else {
            clear()
            return
        }
        if self.session != session || viewID != UUID(uuid: info.view) || revision != info.revision {
            previewID = nil
            // 사이에 document thread가 더 큰 revision을 게시할 수 있습니다. 두 번째 호출의 metadata와 행을 함께 채택합니다.
            var rows = [StellarCanvasFrame](repeating: StellarCanvasFrame(), count: max(1, Int(info.count)))
            var status = rows.withUnsafeMutableBufferPointer { stellar_window_canvas(handle, session, width, height, &info, $0.baseAddress, $0.count) }
            if status == STELLAR_BUFFER_TOO_SMALL {
                rows = [StellarCanvasFrame](repeating: StellarCanvasFrame(), count: Int(info.count))
                status = rows.withUnsafeMutableBufferPointer { stellar_window_canvas(handle, session, width, height, &info, $0.baseAddress, $0.count) }
            }
            guard status == STELLAR_OK else {
                clear()
                return
            }
            frames = rows.prefix(Int(info.count)).enumerated().map { ProjectedFrame($0.element, index: $0.offset) }
            for i in frames.indices.reversed() {
                if let parent = frames[i].parent {
                    let last = frames[i].lastDescendant
                    frames[parent].lastDescendant = max(frames[parent].lastDescendant, last)
                }
            }
        }
        self.session = session
        viewID = UUID(uuid: info.view)
        revision = info.revision
        primary = UUID(uuid: info.primary)
        skipped = info.skipped
        worldToView = transform(info.camera)
    }

    func hit(at point: CGPoint) -> CanvasFrameHit? {
        guard let viewID else { return nil }
        let world = point.applying(worldToView.inverted())
        for frame in frames.reversed() where frame.pickable && frame.worldPath.boundingBox.contains(world) && frame.worldPath.contains(world) {
            var ancestor = frame.parent
            while let i = ancestor, !frames[i].clips || frames[i].worldPath.contains(world) { ancestor = frames[i].parent }
            if ancestor == nil {
                return CanvasFrameHit(id: frame.id, revision: revision, viewID: viewID,
                                      parentToWorld: frame.parent.map { frames[$0].world } ?? .identity, worldToView: worldToView)
            }
        }
        return nil
    }

    func draw(in context: CGContext) {
        context.saveGState()
        context.concatenate(worldToView)
        var clipEnds: [Int] = []
        var selectionPath: CGPath?
        var previewEnd = -1
        for (index, frame) in frames.enumerated() {
            while let end = clipEnds.last, end < index {
                context.restoreGState()
                clipEnds.removeLast()
            }
            if frame.id == previewID {
                context.saveGState()
                context.translateBy(x: previewOffset.dx, y: previewOffset.dy)
                clipEnds.append(frame.lastDescendant)
                previewEnd = frame.lastDescendant
            }
            if frame.id == primary {
                var placement = frame.world
                if index <= previewEnd {
                    placement = placement.concatenating(CGAffineTransform(translationX: previewOffset.dx, y: previewOffset.dy))
                }
                placement = placement.concatenating(worldToView)
                selectionPath = CGPath(rect: frame.localBounds, transform: &placement)
            }
            if (frame.fill != nil || frame.border != nil) && context.boundingBoxOfClipPath.intersects(frame.worldPath.boundingBox) {
                context.saveGState()
                context.concatenate(frame.world)
                if let fill = frame.fill {
                    context.setFillColor(fill)
                    context.addPath(frame.localPath)
                    context.fillPath()
                }
                if let border = frame.border, frame.borderWidth > 0 {
                    context.addPath(frame.localPath)
                    context.clip()
                    context.setStrokeColor(border)
                    context.setLineWidth(frame.borderWidth * 2)
                    context.addPath(frame.localPath)
                    context.strokePath()
                }
                context.restoreGState()
            }
            if frame.clips && frame.lastDescendant > index {
                context.saveGState()
                context.addPath(frame.worldPath)
                context.clip()
                clipEnds.append(frame.lastDescendant)
            }
        }
        for _ in clipEnds { context.restoreGState() }
        context.restoreGState()
        if let path = selectionPath {
            context.saveGState()
            context.setStrokeColor(NSColor.controlAccentColor.cgColor)
            context.setLineWidth(1.5)
            context.addPath(path)
            context.strokePath()
            context.restoreGState()
        }
        if skipped > 0 {
            let message = String(localized: "This canvas displays fixed Free Frames. Some content is unavailable.")
            (message as NSString).draw(at: CGPoint(x: 8, y: 8), withAttributes: [.foregroundColor: NSColor.labelColor,
                                       .backgroundColor: NSColor.windowBackgroundColor, .font: NSFont.systemFont(ofSize: 12)])
        }
    }
}
