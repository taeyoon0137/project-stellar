import AppKit

extension CanvasTool {
    var insertsBounds: Bool { self == .frame || drawnShape != nil }

    /// Rectangle도 다른 도형과 같은 두 점 생성 경로(수식키·클릭 크기)를 씁니다.
    fileprivate var drawnShape: InspectorShapeKind? {
        switch self {
        case .rectangle: return .rectangle
        case .ellipse: return .ellipse
        case .line: return .line
        case .polygon: return .polygon
        default: return nil
        }
    }

    /// 클릭만으로 기본 크기(100×100)를 만드는 도구. Line은 끝점이 필요해 드래그로만 만듭니다(UX 명세 5.1).
    fileprivate var createsOnClick: Bool { insertsBounds && self != .line }

}

extension ShellProjectWindow {
    /// 생성 도구로 root Frame 밖을 눌렀습니다. 놓을 때까지 금지 커서를 두고 도구 막대 위에 이유를 보입니다.
    func rejectDrawOutsideFrame(_ canvas: CanvasView) -> Bool {
        _ = canvas.cancelFrameGesture?()
        canvas.window?.makeFirstResponder(canvas)
        rejectedDraw = true
        NSCursor.operationNotAllowed.set()
        windowController.toolbarController.hintView.show(String(
            localized: "Draw inside a Frame.",
            comment: "Canvas hint when a creation tool is used outside every Frame"
        ))
        return true
    }

    /// 거부한 누름의 드래그·놓기를 삼키고, 놓으면 커서를 되돌립니다.
    func continueRejectedDraw(_ event: NSEvent) -> Bool {
        guard rejectedDraw else { return false }
        switch event.type {
        case .leftMouseDragged:
            NSCursor.operationNotAllowed.set()
        case .leftMouseUp:
            rejectedDraw = false
            NSCursor.arrow.set()
        default:
            return false
        }
        return true
    }

    func handleFrameInsertion(_ event: NSEvent, point: CGPoint, canvas: CanvasView) -> Bool {
        let tool = toolsModel.snapshot.activeTool
        switch event.type {
        case .leftMouseDown:
            _ = canvas.cancelFrameGesture?()
            guard tool.insertsBounds, event.modifierFlags.intersection([.command, .control]).isEmpty,
                  let session = windowController.windowSession, !session.isClosed, !session.isClosing,
                  let viewport = session.viewInfo, viewport.has_view == 1 else { return false }
            // root Frame 밖 빈 캔버스는 거부하고 이유를 보입니다(UX 명세 5.2).
            guard let parent = canvas.frameHit(at: point, onlyFrames: true) else { return rejectDrawOutsideFrame(canvas) }
            guard UUID(uuid: viewport.active_view) == parent.viewID, session.info?.revision == parent.revision else { return false }
            canvas.window?.makeFirstResponder(canvas)
            frameDrag = CanvasFrameDrag(hit: parent, start: point, session: session.session, size: canvas.bounds.size,
                viewport: viewport, tool: tool)
            let placement = parent.localToWorld.concatenating(parent.worldToView)
            frameDrag?.snapCandidates = CoreRuntime.preparedSnapCandidates(
                canvas.snapCandidates(for: parent.id, asParent: true,
                    worldToInput: canvas.worldToView.concatenating(placement.inverted())))
            return true
        case .leftMouseDragged, .leftMouseUp:
            guard let drag = frameDrag else { return false }
            let release = event.type == .leftMouseUp
            if release { _ = canvas.cancelFrameGesture?() }
            guard tool == drag.tool, canvas.bounds.size == drag.size,
                  let session = windowController.windowSession, session.session == drag.session,
                  !session.isClosing, !session.isClosed else {
                _ = canvas.cancelFrameGesture?()
                return true
            }
            var localToView = drag.hit.localToWorld.concatenating(drag.hit.worldToView)
            let det = localToView.a * localToView.d - localToView.b * localToView.c
            guard det.isFinite, det != 0 else { return true }
            let inverse = localToView.inverted()
            let anchor = drag.start.applying(inverse), pointer = point.applying(inverse)
            guard anchor.x.isFinite, anchor.y.isFinite, pointer.x.isFinite, pointer.y.isFinite else { return true }
            let distance = hypot(point.x - drag.start.x, point.y - drag.start.y)
            if !release && distance >= 3 { frameDrag?.moved = true }
            let click = tool.createsOnClick && release && !drag.moved && distance < 3
            var flags: UInt32 = 0
            if event.modifierFlags.contains(.shift) { flags |= 1 }
            if event.modifierFlags.contains(.option) { flags |= 2 }
            if click { flags |= 4 }
            guard let result = CoreRuntime.gesturePreview(kind: tool == .line ? 5 : 4, handle: 0,
                flags: flags, rect: CGRect(x: anchor.x, y: anchor.y, width: 0, height: 0), point: pointer)
            else { return true }
            var start = CGPoint(x: CGFloat(result.line.0), y: CGFloat(result.line.1))
            var end = CGPoint(x: CGFloat(result.line.2), y: CGFloat(result.line.3))
            var rect = CGRect(x: CGFloat(result.rect.0), y: CGFloat(result.rect.1),
                              width: CGFloat(result.rect.2), height: CGFloat(result.rect.3))
            if tool != .line, !event.modifierFlags.contains(.command), snapFlags != 0 {
                let handle: Int32 = end.x < start.x ? (end.y < start.y ? 0 : 3) : (end.y < start.y ? 1 : 2)
                let scale = hypot(localToView.a, localToView.b)
                if let snapped = CoreRuntime.snap(rect: rect, mode: 1, handle: handle, scale: scale,
                                                  flags: snapFlags, candidates: drag.snapCandidates) {
                    rect = CGRect(x: CGFloat(snapped.rect.0), y: CGFloat(snapped.rect.1),
                                  width: CGFloat(snapped.rect.2), height: CGFloat(snapped.rect.3))
                    if end.x < start.x { end.x = rect.minX; start.x = rect.maxX }
                    else { end.x = rect.maxX; start.x = rect.minX }
                    if end.y < start.y { end.y = rect.minY; start.y = rect.maxY }
                    else { end.y = rect.maxY; start.y = rect.minY }
                    canvas.snapOverlay = showsSmartGuides
                        ? CanvasSnapOverlay(result: snapped, inputToView: localToView) : nil
                }
            } else { canvas.snapOverlay = nil }
            let a = start.applying(localToView), b = end.applying(localToView)
            let usable = tool == .line ? hypot(b.x - a.x, b.y - a.y) >= 3
                : rect.width * hypot(localToView.a, localToView.b) >= 3 && rect.height * hypot(localToView.c, localToView.d) >= 3
            guard usable || click else {
                canvas.insertionOutline = nil
                return true
            }
            if release {
                guard let camera = session.viewInfo, UUID(uuid: camera.active_view) == drag.hit.viewID,
                      camera.center_x == drag.viewport.center_x, camera.center_y == drag.viewport.center_y,
                      camera.zoom == drag.viewport.zoom else { return true }
                let completion: (CoreRuntimeEvent) -> Void = { [weak self] event in
                    guard let self else { return }
                    if case .windowViewChanged = event, self.toolsModel.snapshot.activeTool == tool {
                        _ = self.router.dispatch(CanvasTool.move.command)
                    }
                    self.refreshSessionCommands()
                }
                if let kind = tool.drawnShape {
                    // 새 도형 색은 생성 시점의 사용자 앱 설정(Settings ▸ Canvas)에서 읽습니다.
                    _ = session.insertShape(parent: drag.hit, kind: kind, start: start, end: end, completion: completion)
                } else {
                    _ = session.insertFrame(parent: drag.hit, rect: rect, completion: completion)
                }
            } else {
                switch tool {
                case .line:
                    let path = CGMutablePath()
                    path.move(to: a)
                    path.addLine(to: b)
                    canvas.insertionOutline = path
                case .ellipse:
                    canvas.insertionOutline = CGPath(ellipseIn: rect, transform: &localToView)
                case .polygon:
                    // PolygonParams의 기본 0°는 위 꼭짓점에서 시작해 y-down에서 시계 방향입니다.
                    let path = CGMutablePath()
                    for vertex in 0..<5 {
                        let angle = CGFloat(vertex) * 2 * .pi / 5 - .pi / 2
                        let p = CGPoint(x: rect.midX + rect.width * cos(angle) / 2,
                                        y: rect.midY + rect.height * sin(angle) / 2).applying(localToView)
                        if vertex == 0 { path.move(to: p) } else { path.addLine(to: p) }
                    }
                    path.closeSubpath()
                    canvas.insertionOutline = path
                default:
                    canvas.insertionOutline = CGPath(rect: rect, transform: &localToView)
                }
            }
            return true
        default: return false
        }
    }
}
