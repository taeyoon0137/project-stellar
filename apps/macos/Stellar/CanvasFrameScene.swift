import AppKit

/// 코어가 정한 선택 상자 하나(ADR-0071). 셸은 이 값으로 외곽선·핸들·크기 라벨만 그립니다.
struct CanvasSelectionBox: Equatable {
    let id: UUID
    /// 1 Frame, 2 Rectangle, 3 Ellipse, 4 Line, 5 Polygon, 6 Text, 7 Image, 8 Instance.
    let kind: UInt32
    let flags: UInt32
    /// local → world.
    let world: CGAffineTransform
    /// local 상자.
    let rect: CGRect
    /// tl, tr, br, bl.
    let radii: [CGFloat]
    /// Line의 local 시작·끝점.
    let lineStart: CGPoint?
    let lineEnd: CGPoint?

    init(_ item: StellarCanvasSelectionItem) {
        id = UUID(uuid: item.id)
        kind = item.kind
        flags = item.flags
        let w = item.world
        world = CGAffineTransform(a: CGFloat(w.0), b: CGFloat(w.1), c: CGFloat(w.2), d: CGFloat(w.3),
                                  tx: CGFloat(w.4), ty: CGFloat(w.5))
        rect = CGRect(x: CGFloat(item.rect.0), y: CGFloat(item.rect.1),
                      width: CGFloat(item.rect.2), height: CGFloat(item.rect.3))
        radii = [CGFloat(item.radii.0), CGFloat(item.radii.1), CGFloat(item.radii.2), CGFloat(item.radii.3)]
        let isLine = item.kind == 4
        lineStart = isLine ? CGPoint(x: CGFloat(item.line.0), y: CGFloat(item.line.1)) : nil
        lineEnd = isLine ? CGPoint(x: CGFloat(item.line.2), y: CGFloat(item.line.3)) : nil
    }

    func has(_ flag: Int32) -> Bool { flags & UInt32(flag) != 0 }
}

/// 선택 위 핸들(UX 명세 5.4). 크기는 0 tl, 1 tr, 2 br, 3 bl, 4 top, 5 right, 6 bottom, 7 left입니다.
enum CanvasHandle: Equatable {
    case resize(Int)
    /// false 시작점, true 끝점.
    case lineEnd(Bool)
    /// 반경 핸들. 0 tl, 1 tr, 2 br, 3 bl(모서리별), -1은 Polygon의 값 하나입니다.
    case radius(Int)
}

/// 핸들 드래그 중 보여 줄 새 상자. 문서는 놓을 때 한 번만 바꿉니다.
struct CanvasGesturePreview {
    let id: UUID
    var rect: CGRect
    var lineStart: CGPoint?
    var lineEnd: CGPoint?
    /// 반경 핸들 드래그 중의 반경(tl, tr, br, bl).
    var radii: [CGFloat]?
}

extension CGRect {
    /// 크기 핸들 `handle`을 local 점 `point`로 끈 상자. Shift는 비율 유지, Option은 중심 기준입니다. 최소 1입니다.
    func resized(handle: Int, to point: CGPoint, keepRatio: Bool, fromCenter: Bool) -> CGRect {
        func axis(movesMin: Bool, movesMax: Bool, lo: CGFloat, hi: CGFloat, value: CGFloat)
            -> (anchor: CGFloat, extent: CGFloat)? {
            guard movesMin || movesMax else { return nil }
            let anchor = fromCenter ? (lo + hi) / 2 : (movesMin ? hi : lo)
            return (anchor, value - anchor)
        }
        let x = axis(movesMin: [0, 3, 7].contains(handle), movesMax: [1, 2, 5].contains(handle),
                     lo: minX, hi: maxX, value: point.x)
        let y = axis(movesMin: [0, 1, 4].contains(handle), movesMax: [2, 3, 6].contains(handle),
                     lo: minY, hi: maxY, value: point.y)
        let reference = CGSize(width: fromCenter ? width / 2 : width, height: fromCenter ? height / 2 : height)
        var scale: CGFloat?
        if keepRatio, reference.width > 0, reference.height > 0 {
            scale = max(x.map { abs($0.extent) / reference.width } ?? 0, y.map { abs($0.extent) / reference.height } ?? 0)
        }
        func range(_ a: (anchor: CGFloat, extent: CGFloat)?, lo: CGFloat, hi: CGFloat, ref: CGFloat) -> (CGFloat, CGFloat) {
            if let a {
                var e = a.extent
                if let scale { e = (e < 0 ? -1 : 1) * ref * scale }
                let r = fromCenter ? (a.anchor - abs(e), a.anchor + abs(e)) : (min(a.anchor, a.anchor + e), max(a.anchor, a.anchor + e))
                return (r.0, max(r.1, r.0 + 1))
            }
            guard let scale else { return (lo, hi) }
            let mid = (lo + hi) / 2, half = max((hi - lo) * scale, 1) / 2
            return (mid - half, mid + half)
        }
        let (x0, x1) = range(x, lo: minX, hi: maxX, ref: reference.width)
        let (y0, y1) = range(y, lo: minY, hi: maxY, ref: reference.height)
        return CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)
    }
}

struct CanvasFrameHit {
    var textQuery: StellarCanvasTextQuery? = nil
    var isText: Bool { textQuery != nil }
    var isImage = false
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

func canvasColor(_ value: StellarCanvasColor) -> CGColor? {
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

/// Rust의 local path를 한 번 읽고 같은 경로로 채우기·stroke·hit를 계산합니다.
private final class CanvasShapeDrawing {
    let path: CGPath
    let stroke: CGPath?
    let worldStroke: CGPath?
    let localBounds: CGRect
    let hitsFill: Bool

    init?(handle: UInt64, session: UInt64, query: StellarCanvasTextQuery, row: StellarCanvasFrame) {
        guard let (elements, flags) = CoreRuntime.canvasShapePath(handle: handle, session: session, query: query)
        else { return nil }
        let outline = CGMutablePath()
        var pointCap: CGRect?
        for element in elements {
            let (x, y, a, b, c, d) = element.points
            switch element.op {
            case 1: outline.move(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 2: outline.addLine(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case 3: outline.addCurve(to: CGPoint(x: CGFloat(c), y: CGFloat(d)),
                                     control1: CGPoint(x: CGFloat(x), y: CGFloat(y)), control2: CGPoint(x: CGFloat(a), y: CGFloat(b)))
            case 4: outline.closeSubpath()
            case 5: pointCap = CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(a), height: CGFloat(b))
            default: return nil
            }
        }
        path = outline
        let cap: CGLineCap
        switch flags & 3 {
        case 0: cap = .butt
        case 1: cap = .round
        case 2: cap = .square
        default: return nil
        }
        if row.border.space != 0, row.border_width > 0 {
            let width = CGFloat(row.border_width)
            if let box = pointCap {
                switch cap {
                case .round: stroke = CGPath(ellipseIn: box, transform: nil)
                case .square: stroke = CGPath(rect: box, transform: nil)
                default: stroke = nil
                }
            } else {
                stroke = outline.copy(strokingWithWidth: width, lineCap: cap, lineJoin: .miter, miterLimit: 10)
            }
        } else { stroke = nil }
        hitsFill = row.fill.space != 0 || row.border.space == 0
        var placement = transform(row.world)
        worldStroke = stroke?.copy(using: &placement)
        localBounds = outline.boundingBoxOfPath.union(stroke?.boundingBoxOfPath ?? .null)
    }
}

private struct ProjectedFrame {
    let text: CanvasDocumentText?
    let shape: CanvasShapeDrawing?
    var isFrame: Bool { !isImage && !isRectangle && shape == nil && !isInstance && text == nil }
    let id: UUID
    let parent: Int?
    let world: CGAffineTransform
    let localBounds: CGRect
    let localPath: CGPath
    let worldPath: CGPath
    let worldBounds: CGRect
    var subtreeBounds: CGRect
    let isInstance: Bool
    let isImage: Bool
    let isRectangle: Bool
    let clips: Bool
    let pickable: Bool
    let fill: CGColor?
    let border: CGColor?
    let borderWidth: CGFloat
    var lastDescendant: Int

    init(_ row: StellarCanvasFrame, index: Int, text: CanvasDocumentText?, shape: CanvasShapeDrawing?) {
        self.shape = shape
        self.text = text
        id = UUID(uuid: row.id)
        parent = row.parent == .max ? nil : Int(row.parent)
        var world = transform(row.world)
        self.world = world
        localBounds = shape?.localBounds ?? CGRect(x: 0, y: 0, width: CGFloat(row.size.0), height: CGFloat(row.size.1))
        localPath = shape?.path ?? text?.hitPath ?? outline(row)
        worldPath = localPath.copy(using: &world)!
        if let worldStroke = shape?.worldStroke { worldBounds = worldPath.boundingBox.union(worldStroke.boundingBox) }
        else if let text, !text.inkBounds.isNull {
            worldBounds = worldPath.boundingBox.union(text.inkBounds.applying(world))
        } else {
            worldBounds = worldPath.boundingBox
        }
        subtreeBounds = worldBounds
        isImage = row.flags & 32 != 0
        isInstance = row.flags & 8 != 0
        isRectangle = row.flags & 4 != 0
        clips = row.flags & 1 != 0
        pickable = row.flags & 2 != 0
        fill = canvasColor(row.fill)
        border = canvasColor(row.border)
        borderWidth = CGFloat(row.border_width)
        lastDescendant = index
    }
    func contains(_ point: CGPoint) -> Bool {
        guard let shape else { return worldPath.contains(point) }
        return (shape.hitsFill && worldPath.contains(point)) || (shape.worldStroke?.contains(point) == true)
    }

}

/// 한 창에서 마지막으로 표시한 scene. 카메라 또는 선택만 바뀌면 path를 다시 만들지 않습니다.
final class CanvasFrameScene {
    private var frames: [ProjectedFrame] = []
    let png = CanvasPNGCache()
    private var runtimeHandle: UInt64?
    var redrawImage: ((CGRect) -> Void)?
    private var documentTexts: [UUID: CanvasDocumentText] = [:]
    private var viewID: UUID?
    private var revision: UInt64 = 0
    private var session: UInt64?
    private static let emptyID = UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    private var primary: UUID?
    private var primaryIndex: Int?
    private var previewIDs: [UUID] = []
    /// 드래그 중 함께 옮겨 그리는 subtree 행 범위. 서로 겹치지 않습니다(조상이 있으면 자손 범위는 뺍니다).
    private var previewRanges: [ClosedRange<Int>] = []
    private var previewOffset = CGVector.zero
    private var indexByID: [UUID: Int] = [:]
    /// 코어가 정한 선택 상자(ADR-0071).
    private(set) var selection: [CanvasSelectionBox] = []
    /// 활성 View의 root Frame.
    private(set) var rootID: UUID?
    /// 핸들 드래그 중의 새 상자(선택 외곽선·핸들·크기 라벨이 따라갑니다).
    var gesturePreview: CanvasGesturePreview?
    /// 이보다 작은 화면 상자에는 모서리 핸들만 둡니다.
    private static let edgeHandleMinimum: CGFloat = 24
    /// 이보다 작은 화면 상자에는 반경 핸들을 두지 않습니다.
    private static let radiusHandleMinimum: CGFloat = 40
    /// 반경이 작아도 반경 핸들을 모서리에서 이만큼(화면 pt) 안쪽에 둡니다.
    private static let radiusHandleInset: CGFloat = 12

    /// 반경 핸들 `index`의 모서리와 안쪽 대각 방향(local).
    static func radiusCorner(_ index: Int, in r: CGRect) -> (corner: CGPoint, inward: CGVector) {
        switch index {
        case 0: return (CGPoint(x: r.minX, y: r.minY), CGVector(dx: 1, dy: 1))
        case 1: return (CGPoint(x: r.maxX, y: r.minY), CGVector(dx: -1, dy: 1))
        case 2: return (CGPoint(x: r.maxX, y: r.maxY), CGVector(dx: -1, dy: -1))
        default: return (CGPoint(x: r.minX, y: r.maxY), CGVector(dx: 1, dy: -1))
        }
    }

    /// 반경 핸들을 local 점 `point`로 끈 반경(0 ... 짧은 변의 절반).
    static func radius(handle index: Int, in r: CGRect, at point: CGPoint) -> CGFloat {
        let limit = min(r.width, r.height) / 2
        guard index >= 0 else { return min(max(point.y - r.minY, 0), limit) }
        let (corner, inward) = radiusCorner(index, in: r)
        let along = ((point.x - corner.x) * inward.dx + (point.y - corner.y) * inward.dy) / 2
        return min(max(along, 0), limit)
    }

    private func previewing(_ index: Int) -> Bool { previewRanges.contains { $0.contains(index) } }

    func setDragPreview(ids: [UUID], offset: CGVector) {
        if previewIDs != ids {
            previewIDs = ids
            let ranges = ids.compactMap { indexByID[$0] }.map { $0...frames[$0].lastDescendant }
            previewRanges = ranges.filter { range in
                !ranges.contains { $0 != range && $0.contains(range.lowerBound) && $0.count > range.count }
            }
        }
        let inverse = worldToView.inverted()
        previewOffset = CGVector(dx: inverse.a * offset.dx + inverse.c * offset.dy,
                                 dy: inverse.b * offset.dx + inverse.d * offset.dy)
    }
    private(set) var worldToView = CGAffineTransform.identity
    private(set) var skipped: UInt32 = 0

    /// 다음 `refresh`가 같은 revision이라도 행을 다시 읽게 합니다.
    func invalidateRows() { viewID = nil }

    func clear() {
        png.reset()
        frames.removeAll()
        documentTexts.removeAll()
        viewID = nil
        session = nil
        skipped = 0
        primary = nil
        primaryIndex = nil
        previewIDs = []
        previewRanges = []
        indexByID = [:]
        selection = []
        rootID = nil
    }

    func refresh(handle: UInt64, session: UInt64, size: CGSize) {
        runtimeHandle = handle
        let width = Float(size.width), height = Float(size.height)
        guard var info = CoreRuntime.canvasInfo(handle: handle, session: session, size: size) else {
            clear()
            return
        }
        if self.session != session || viewID != UUID(uuid: info.view) || revision != info.revision {
            previewIDs = []
            previewRanges = []
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
                var shape: CanvasShapeDrawing?
                if row.flags & 64 != 0 {
                    let query = StellarCanvasTextQuery(view: info.view, node: row.id, revision: info.revision, row: UInt32(index))
                    guard let drawing = CanvasShapeDrawing(handle: handle, session: session, query: query, row: row) else { clear(); return }
                    shape = drawing
                }
                projected.append(ProjectedFrame(row, index: index, text: text, shape: shape))
            }
            frames = projected
            // revision이 바뀌어도 남은 이미지 행은 다시 확인할 때까지 이전 픽셀을 그립니다(깜박임 없음).
            png.retain(Set(frames.lazy.filter(\.isImage).map(\.id)))
            indexByID = Dictionary(frames.enumerated().map { ($1.id, $0) }, uniquingKeysWith: { first, _ in first })
            rootID = frames.first { $0.parent == nil }?.id
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
        selection = CoreRuntime.canvasSelection(handle: handle, session: session)?.items.map(CanvasSelectionBox.init) ?? []
    }

    /// 선택 상자(핸들 드래그 중이면 새 상자)의 local 사각형.
    private func displayedRect(_ box: CanvasSelectionBox) -> CGRect {
        gesturePreview.flatMap { $0.id == box.id ? $0.rect : nil } ?? box.rect
    }

    /// 하나뿐인 선택의 핸들과 화면 위치. 크기를 바꿀 수 없는 node에는 없습니다.
    func handles() -> [(handle: CanvasHandle, point: CGPoint)] {
        guard selection.count == 1, let box = selection.first, box.has(STELLAR_CANVAS_ITEM_RESIZABLE) else { return [] }
        let placement = selectionPlacement(box)
        if let start = box.lineStart, let end = box.lineEnd {
            let preview = gesturePreview?.id == box.id ? gesturePreview : nil
            return [(.lineEnd(false), (preview?.lineStart ?? start).applying(placement)),
                    (.lineEnd(true), (preview?.lineEnd ?? end).applying(placement))]
        }
        let r = displayedRect(box)
        let corners = [CGPoint(x: r.minX, y: r.minY), CGPoint(x: r.maxX, y: r.minY),
                       CGPoint(x: r.maxX, y: r.maxY), CGPoint(x: r.minX, y: r.maxY)].map { $0.applying(placement) }
        var result = corners.enumerated().map { (CanvasHandle.resize($0.offset), $0.element) }
        let screenWidth = hypot(corners[1].x - corners[0].x, corners[1].y - corners[0].y)
        let screenHeight = hypot(corners[3].x - corners[0].x, corners[3].y - corners[0].y)
        if screenWidth >= Self.edgeHandleMinimum && screenHeight >= Self.edgeHandleMinimum {
            let edges = [CGPoint(x: r.midX, y: r.minY), CGPoint(x: r.maxX, y: r.midY),
                         CGPoint(x: r.midX, y: r.maxY), CGPoint(x: r.minX, y: r.midY)]
            result += edges.enumerated().map { (CanvasHandle.resize($0.offset + 4), $0.element.applying(placement)) }
        }
        if box.has(STELLAR_CANVAS_ITEM_RADIUS), screenWidth >= Self.radiusHandleMinimum, screenHeight >= Self.radiusHandleMinimum {
            let scale = max(sqrt(abs(placement.a * placement.d - placement.b * placement.c)), .ulpOfOne)
            let inset = Self.radiusHandleInset / scale
            let limit = min(r.width, r.height) / 2
            let radii = gesturePreview.flatMap { $0.id == box.id ? $0.radii : nil } ?? box.radii
            if box.has(STELLAR_CANVAS_ITEM_RADIUS_PER_CORNER) {
                for index in 0..<4 {
                    let (corner, inward) = Self.radiusCorner(index, in: r)
                    let offset = min(max(radii[index], inset), limit)
                    let point = CGPoint(x: corner.x + inward.dx * offset, y: corner.y + inward.dy * offset)
                    result.append((.radius(index), point.applying(placement)))
                }
            } else {
                let offset = min(max(radii[0], inset), limit)
                result.append((.radius(-1), CGPoint(x: r.midX, y: r.minY + offset).applying(placement)))
            }
        }
        return result
    }

    /// 화면 점 아래의 핸들(6pt 이내).
    func handle(at point: CGPoint) -> (box: CanvasSelectionBox, handle: CanvasHandle)? {
        guard let box = selection.first else { return nil }
        let hit = handles().min { hypot($0.point.x - point.x, $0.point.y - point.y) < hypot($1.point.x - point.x, $1.point.y - point.y) }
        guard let hit, hypot(hit.point.x - point.x, hit.point.y - point.y) <= 6 else { return nil }
        return (box, hit.handle)
    }

    private static func cssNumber(_ value: CGFloat) -> String {
        let rounded = (value * 100).rounded() / 100
        return rounded == rounded.rounded() ? String(Int(rounded)) : String(format: "%g", Double(rounded))
    }

    /// 외곽선, 핸들, 선택 아래 크기 라벨(`320 × 200`, 자동 크기 축은 CSS 값 `max-content`·`auto`).
    private func drawSelection(in context: CGContext) {
        guard !selection.isEmpty else { return }
        context.saveGState()
        let accent = NSColor.controlAccentColor.cgColor
        context.setStrokeColor(accent)
        context.setLineWidth(1.5)
        var screen = CGRect.null
        var world = CGRect.null
        for box in selection {
            var placement = selectionPlacement(box)
            let rect = displayedRect(box)
            let path: CGPath
            if let start = box.lineStart, let end = box.lineEnd {
                let preview = gesturePreview?.id == box.id ? gesturePreview : nil
                let line = CGMutablePath()
                line.move(to: (preview?.lineStart ?? start).applying(placement))
                line.addLine(to: (preview?.lineEnd ?? end).applying(placement))
                path = line
            } else {
                path = CGPath(rect: rect, transform: &placement)
            }
            context.addPath(path)
            screen = screen.union(path.boundingBox)
            world = world.union(rect.applying(box.world))
        }
        context.strokePath()
        for (handle, point) in handles() {
            let square = CGRect(x: point.x - 3.5, y: point.y - 3.5, width: 7, height: 7)
            context.setFillColor(NSColor.white.cgColor)
            context.setLineWidth(1)
            if case .radius = handle {
                context.fillEllipse(in: square)
                context.strokeEllipse(in: square)
            } else {
                context.fill(square)
                context.stroke(square)
            }
        }
        context.restoreGState()
        let label: String
        if selection.count == 1, let box = selection.first {
            let size = displayedRect(box).size
            let line = box.lineStart.flatMap { start in box.lineEnd.map { CGSize(width: abs($0.x - start.x), height: abs($0.y - start.y)) } }
            let shown = line ?? size
            // 고정 크기가 아닌 축은 자동 크기 Text의 CSS 값입니다(canvas_editing.md: `width: max-content`, `height: auto`).
            let width = box.has(STELLAR_CANVAS_ITEM_FIXED_WIDTH) ? Self.cssNumber(shown.width) : "max-content"
            let height = box.has(STELLAR_CANVAS_ITEM_FIXED_HEIGHT) ? Self.cssNumber(shown.height) : "auto"
            label = "\(width) × \(height)"
        } else {
            label = "\(Self.cssNumber(world.width)) × \(Self.cssNumber(world.height))"
        }
        guard !screen.isNull else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: NSColor.white,
        ]
        let text = label as NSString
        let size = text.size(withAttributes: attributes)
        let pill = CGRect(x: screen.midX - size.width / 2 - 6, y: screen.maxY + 6, width: size.width + 12, height: size.height + 4)
        NSGraphicsContext.saveGraphicsState()
        NSColor.controlAccentColor.setFill()
        NSBezierPath(roundedRect: pill, xRadius: 4, yRadius: 4).fill()
        text.draw(at: CGPoint(x: pill.minX + 6, y: pill.minY + 2), withAttributes: attributes)
        NSGraphicsContext.restoreGraphicsState()
    }

    /// 선택 상자의 local → 화면 변환. 드래그 미리보기 중이면 같은 만큼 옮깁니다.
    func selectionPlacement(_ box: CanvasSelectionBox) -> CGAffineTransform {
        var placement = box.world
        if let index = indexByID[box.id], previewing(index) {
            placement = placement.concatenating(CGAffineTransform(translationX: previewOffset.dx, y: previewOffset.dy))
        }
        return placement.concatenating(worldToView)
    }

    func hit(at point: CGPoint, onlyFrames: Bool) -> CanvasFrameHit? {
        guard let viewID else { return nil }
        let world = point.applying(worldToView.inverted())
        for frame in frames.reversed() where (!onlyFrames || frame.isFrame) && frame.pickable && frame.worldBounds.contains(world) && frame.contains(world) {
            var ancestor = frame.parent
            while let i = ancestor, !frames[i].clips || frames[i].worldPath.contains(world) { ancestor = frames[i].parent }
            if ancestor == nil {
                // 정의의 자식 위를 눌러도 이동하는 문서 노드는 인스턴스 하나입니다.
                var parent = frame.parent
                if frame.isInstance {
                    while let i = parent, frames[i].isInstance && frames[i].id == frame.id { parent = frames[i].parent }
                }
                return CanvasFrameHit(textQuery: frame.text?.query, isImage: frame.isImage, isInstance: frame.isInstance, isRectangle: frame.isRectangle, id: frame.id, revision: revision, viewID: viewID,
                                      localToWorld: frame.world, parentToWorld: parent.map { frames[$0].world } ?? .identity, worldToView: worldToView)
            }
        }
        return nil
    }

    /// 편집 중인 Text의 caret과 선택 사각형(그 Text local 좌표). 글자는 코어 scene의 Text 행이 그립니다(ADR-0067 §6).
    struct TextEditOverlay {
        let node: UUID
        let caret: CGRect?
        let selection: [CGRect]
    }
    var textEditOverlay: TextEditOverlay?

    func textHit(id: UUID? = nil) -> CanvasFrameHit? {
        guard let viewID else { return nil }
        let index = id.map { wanted in frames.firstIndex { $0.id == wanted } } ?? primaryIndex
        guard let i = index, let text = frames[i].text else { return nil }
        let frame = frames[i]
        return CanvasFrameHit(textQuery: text.query, isInstance: false, isRectangle: false, id: frame.id,
                              revision: revision, viewID: viewID, localToWorld: frame.world,
                              parentToWorld: frame.parent.map { frames[$0].world } ?? .identity, worldToView: worldToView)
    }

    private func drawTextEdit(_ overlay: TextEditOverlay, in context: CGContext) {
        guard let frame = frames.first(where: { $0.id == overlay.node }) else { return }
        context.saveGState()
        context.concatenate(worldToView)
        context.concatenate(frame.world)
        if !overlay.selection.isEmpty {
            context.setFillColor(NSColor.selectedTextBackgroundColor.cgColor)
            context.fill(overlay.selection)
        }
        if var caret = overlay.caret {
            // caret은 배율과 관계없이 화면 1pt 너비로 그립니다.
            let unit = context.convertToUserSpace(CGSize(width: 1, height: 0))
            caret.size.width = max(caret.width, hypot(unit.width, unit.height))
            context.setFillColor(NSColor.textColor.cgColor)
            context.fill(caret)
        }
        context.restoreGState()
    }

    private func imageReady(at index: Int) {
        guard frames.indices.contains(index) else { return }
        var damage = frames[index].worldBounds
        if previewing(index) {
            damage = damage.offsetBy(dx: previewOffset.dx, dy: previewOffset.dy)
        }
        redrawImage?(damage.applying(worldToView).insetBy(dx: -1, dy: -1))
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
            let frameQuery = previewing(index) ? previewQuery : query
            let visible = frame.worldBounds.intersects(frameQuery)
            if !visible {
                // 원래 subtree 전체가 화면 밖이어도 드래그 중인 자식이 들어올 수 있습니다.
                let movingChild = previewRanges.contains { index < $0.lowerBound && $0.lowerBound <= frame.lastDescendant }
                if frame.clips || (!movingChild && !frame.subtreeBounds.intersects(frameQuery)) {
                    index = frame.lastDescendant + 1
                    continue
                }
            }
            if previewRanges.contains(where: { $0.lowerBound == index }) {
                context.saveGState()
                context.translateBy(x: previewOffset.dx, y: previewOffset.dy)
                clipEnds.append(frame.lastDescendant)
            }
            if visible && (frame.isImage || frame.fill != nil || frame.border != nil) {
                context.saveGState()
                context.concatenate(frame.world)
                if frame.isImage, let recordSession = session, let handle = runtimeHandle {
                    let row = index
                    png.request(handle: handle, session: recordSession, revision: revision, row: row, node: frame.id) { [weak self] in
                        self?.imageReady(at: row)
                    }
                    if let image = png.image(frame.id) {
                        context.saveGState()
                        context.translateBy(x: 0, y: frame.localBounds.height)
                        context.scaleBy(x: 1, y: -1)
                        let scale = min(frame.localBounds.width / CGFloat(image.width), frame.localBounds.height / CGFloat(image.height))
                        let w = CGFloat(image.width) * scale, h = CGFloat(image.height) * scale
                        context.draw(image, in: CGRect(x: (frame.localBounds.width - w)/2, y: (frame.localBounds.height - h)/2, width: w, height: h))
                        context.restoreGState()
                    }
                }
                if let fill = frame.fill {
                    context.setFillColor(fill)
                    if let text = frame.text {
                        text.draw(in: context)
                    } else {
                        context.addPath(frame.localPath)
                        context.fillPath()
                    }
                }
                if let shape = frame.shape {
                    if let stroke = shape.stroke, let color = frame.border {
                        context.setFillColor(color)
                        context.addPath(stroke)
                        context.fillPath()
                    }
                } else if let border = frame.border, frame.borderWidth > 0 {
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
        if let overlay = textEditOverlay { drawTextEdit(overlay, in: context) }
        // 선택 외곽선은 조상의 clip과 관계없이 scene 위에 표시합니다. 상자는 코어가 정합니다(ADR-0071).
        drawSelection(in: context)
    }
}
