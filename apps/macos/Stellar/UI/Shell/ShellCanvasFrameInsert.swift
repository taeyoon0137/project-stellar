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

    /// 부모 local 공간의 그린 두 점. 클릭은 기본 크기, Shift는 정비례(Line은 45° 단위), Option은 누른 점 중심입니다.
    fileprivate func endpoints(from anchor: CGPoint, to pointer: CGPoint, click: Bool,
                               modifiers: NSEvent.ModifierFlags) -> (CGPoint, CGPoint) {
        let centered = modifiers.contains(.option)
        var dx = pointer.x - anchor.x, dy = pointer.y - anchor.y
        if click {
            dx = centered ? 50 : 100
            dy = dx
        } else if modifiers.contains(.shift) {
            if self == .line {
                let octant = Int((atan2(dy, dx) / (.pi / 4)).rounded())
                let length = hypot(dx, dy)
                let angle = CGFloat(octant) * .pi / 4
                let x = cos(angle), y = sin(angle)
                dx = length * (octant.isMultiple(of: 2) ? x.rounded() : x)
                dy = length * (octant.isMultiple(of: 2) ? y.rounded() : y)
            } else {
                let extent = max(abs(dx), abs(dy))
                dx = dx < 0 ? -extent : extent
                dy = dy < 0 ? -extent : extent
            }
        }
        let start = centered ? CGPoint(x: anchor.x - dx, y: anchor.y - dy) : anchor
        return (start, CGPoint(x: anchor.x + dx, y: anchor.y + dy))
    }
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
            let (start, end) = tool.endpoints(from: anchor, to: pointer, click: click, modifiers: event.modifierFlags)
            let rect = CGRect(x: min(start.x, end.x), y: min(start.y, end.y),
                              width: abs(end.x - start.x), height: abs(end.y - start.y))
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
