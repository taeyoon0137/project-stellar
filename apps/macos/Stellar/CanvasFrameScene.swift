import AppKit

struct CanvasInspectorFrame: Equatable {
    let session: UInt64
    let view: UUID
    let node: UUID
    let revision: UInt64
    let size: CGSize
    let background: InspectorColor?
    let editable: Bool
}

struct CanvasFrameHit {
    var textQuery: StellarCanvasTextQuery? = nil
    var isText: Bool { textQuery != nil }
    let isInstance: Bool
    let isRectangle: Bool
    let id: UUID
    let revision: UInt64
    let viewID: UUID
    let localToWorld: CGAffineTransform
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
    let text: CanvasDocumentText?
    let id: UUID
    let parent: Int?
    let world: CGAffineTransform
    let localBounds: CGRect
    let localPath: CGPath
    let worldPath: CGPath
    let worldBounds: CGRect
    var subtreeBounds: CGRect
    let isInstance: Bool
    let isRectangle: Bool
    let clips: Bool
    let pickable: Bool
    let fill: CGColor?
    let border: CGColor?
    let borderWidth: CGFloat
    var lastDescendant: Int

    init(_ row: StellarCanvasFrame, index: Int, text: CanvasDocumentText?) {
        self.text = text
        id = UUID(uuid: row.id)
        parent = row.parent == .max ? nil : Int(row.parent)
        var world = transform(row.world)
        self.world = world
        localBounds = CGRect(x: 0, y: 0, width: CGFloat(row.size.0), height: CGFloat(row.size.1))
        localPath = text?.hitPath ?? outline(row)
        worldPath = localPath.copy(using: &world)!
        if let text, !text.inkBounds.isNull {
            worldBounds = worldPath.boundingBox.union(text.inkBounds.applying(world))
        } else {
            worldBounds = worldPath.boundingBox
        }
        subtreeBounds = worldBounds
        isInstance = row.flags & 8 != 0
        isRectangle = row.flags & 4 != 0
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
    private var documentTexts: [UUID: CanvasDocumentText] = [:]
    private var viewID: UUID?
    private var revision: UInt64 = 0
    private var session: UInt64?
    private static let emptyID = UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    private var primary: UUID?
    private var primaryIndex: Int?
    private var previewID: UUID?
    private var previewRange: ClosedRange<Int>?
    private var previewOffset = CGVector.zero

    func setDragPreview(id: UUID?, offset: CGVector) {
        if previewID != id {
            previewID = id
            previewRange = id.flatMap { id in frames.firstIndex { $0.id == id } }
                .map { $0...frames[$0].lastDescendant }
        }
        let inverse = worldToView.inverted()
        previewOffset = CGVector(dx: inverse.a * offset.dx + inverse.c * offset.dy,
                                 dy: inverse.b * offset.dx + inverse.d * offset.dy)
    }
    private(set) var worldToView = CGAffineTransform.identity
    private(set) var skipped: UInt32 = 0

    func clear() {
        frames.removeAll()
        documentTexts.removeAll()
        viewID = nil
        session = nil
        skipped = 0
        primary = nil
        primaryIndex = nil
        previewID = nil
        previewRange = nil
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
            previewRange = nil
            primary = nil
            primaryIndex = nil
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
            var projected: [ProjectedFrame] = []
            var nextTexts: [UUID: CanvasDocumentText] = [:]
            projected.reserveCapacity(Int(info.count))
            for (index, row) in rows.prefix(Int(info.count)).enumerated() {
                var text: CanvasDocumentText?
                if row.flags & 16 != 0 {
                    let query = StellarCanvasTextQuery(view: info.view, node: row.id, revision: info.revision, row: UInt32(index))
                    let id = UUID(uuid: row.id)
                    guard let prepared = CanvasDocumentText(handle: handle, session: session, query: query, previous: documentTexts[id]) else { clear(); return }
                    text = prepared
                    nextTexts[id] = prepared
                }
                projected.append(ProjectedFrame(row, index: index, text: text))
            }
            frames = projected
            documentTexts = nextTexts
            for i in frames.indices.reversed() {
                if let parent = frames[i].parent {
                    let bounds = frames[i].subtreeBounds
                    frames[parent].subtreeBounds = frames[parent].subtreeBounds.union(bounds)
                    let last = frames[i].lastDescendant
                    frames[parent].lastDescendant = max(frames[parent].lastDescendant, last)
                }
            }
        }
        self.session = session
        viewID = UUID(uuid: info.view)
        revision = info.revision
        let selected = UUID(uuid: info.primary)
        if primary != selected {
            primary = selected
            primaryIndex = selected == Self.emptyID ? nil : frames.firstIndex { $0.id == selected }
        }
        skipped = info.skipped
        worldToView = transform(info.camera)
    }

    var selectedInspectorFrame: CanvasInspectorFrame? {
        guard let session, let viewID, let primaryIndex else { return nil }
        let selected = frames[primaryIndex]
        guard !selected.isRectangle, !selected.isInstance, selected.text == nil else { return nil }
        let background: InspectorColor?
        if let fill = selected.fill, let rgba = fill.components, rgba.count == 4 {
            let space: InspectorColor.Space = fill.colorSpace?.name == CGColorSpace.extendedDisplayP3 ? .displayP3 : .srgb
            background = InspectorColor(space: space, red: rgba[0], green: rgba[1], blue: rgba[2], alpha: rgba[3])
        } else {
            background = nil
        }
        return CanvasInspectorFrame(session: session, view: viewID, node: selected.id, revision: revision,
                                     size: selected.localBounds.size, background: background, editable: selected.pickable)
    }

    func hit(at point: CGPoint, onlyFrames: Bool) -> CanvasFrameHit? {
        guard let viewID else { return nil }
        let world = point.applying(worldToView.inverted())
        for frame in frames.reversed() where (!onlyFrames || (!frame.isRectangle && !frame.isInstance && frame.text == nil)) && frame.pickable && frame.worldBounds.contains(world) && frame.worldPath.contains(world) {
            var ancestor = frame.parent
            while let i = ancestor, !frames[i].clips || frames[i].worldPath.contains(world) { ancestor = frames[i].parent }
            if ancestor == nil {
                return CanvasFrameHit(textQuery: frame.text?.query, isInstance: frame.isInstance, isRectangle: frame.isRectangle, id: frame.id, revision: revision, viewID: viewID,
                                      localToWorld: frame.world, parentToWorld: frame.parent.map { frames[$0].world } ?? .identity, worldToView: worldToView)
            }
        }
        return nil
    }

    func draw(in context: CGContext, dirtyRect: CGRect) {
        // damage 경계의 안티앨리어싱을 보존하도록 한 포인트의 여유를 둡니다.
        let dirty = dirtyRect.intersection(context.boundingBoxOfClipPath)
        let query = dirty.isNull ? CGRect.null : dirty.insetBy(dx: -1, dy: -1).applying(worldToView.inverted())
        let previewQuery = query.offsetBy(dx: -previewOffset.dx, dy: -previewOffset.dy)
        context.saveGState()
        context.concatenate(worldToView)
        var clipEnds: [Int] = []
        var index = 0
        while index < frames.count {
            let frame = frames[index]
            while let end = clipEnds.last, end < index {
                context.restoreGState()
                clipEnds.removeLast()
            }
            let frameQuery = previewRange?.contains(index) == true ? previewQuery : query
            let visible = frame.worldBounds.intersects(frameQuery)
            if !visible {
                // 원래 subtree 전체가 화면 밖이어도 드래그 중인 자식이 들어올 수 있습니다.
                let movingChild = previewRange.map { index < $0.lowerBound && $0.lowerBound <= frame.lastDescendant } ?? false
                if frame.clips || (!movingChild && !frame.subtreeBounds.intersects(frameQuery)) {
                    index = frame.lastDescendant + 1
                    continue
                }
            }
            if index == previewRange?.lowerBound {
                context.saveGState()
                context.translateBy(x: previewOffset.dx, y: previewOffset.dy)
                clipEnds.append(frame.lastDescendant)
            }
            if visible && (frame.fill != nil || frame.border != nil) {
                context.saveGState()
                context.concatenate(frame.world)
                if let fill = frame.fill {
                    context.setFillColor(fill)
                    if let text = frame.text {
                        text.draw(in: context)
                    } else {
                        context.addPath(frame.localPath)
                        context.fillPath()
                    }
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
            index += 1
        }
        for _ in clipEnds { context.restoreGState() }
        context.restoreGState()
        // 선택 외곽선은 조상의 clip과 관계없이 scene 위에 표시합니다.
        if let i = primaryIndex {
            let frame = frames[i]
            var placement = frame.world
            if previewRange?.contains(i) == true {
                placement = placement.concatenating(CGAffineTransform(translationX: previewOffset.dx, y: previewOffset.dy))
            }
            placement = placement.concatenating(worldToView)
            let path = CGPath(rect: frame.localBounds, transform: &placement)
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
