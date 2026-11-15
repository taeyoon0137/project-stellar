import AppKit

/// 누른 시점의 실제 표시 상태. 이동 중에는 문서 history에 쓰지 않습니다.
struct CanvasFrameDrag {
    let hit: CanvasFrameHit
    let start: NSPoint
    let session: UInt64
    let size: NSSize
    let viewport: StellarWindowViewInfo
    let canMove: Bool
    let tool: CanvasTool
    var moved = false
}

extension ShellProjectWindow {
    func connectFrameInteractions() {
        let canvas = canvasHost.canvasView
        var handDrag: (point: NSPoint, session: UInt64, size: NSSize)?
        var zoomDrag: (start: NSPoint, session: UInt64, camera: StellarWindowViewInfo, bounds: CGRect)?
        canvas.cancelFrameGesture = { [weak self, weak canvas] in
            guard let self, self.frameDrag != nil || handDrag != nil || zoomDrag != nil else { return false }
            handDrag = nil
            zoomDrag = nil
            self.frameDrag = nil
            canvas?.setFrameDragPreview(id: nil, offset: .zero)
            canvas?.insertionOutline = nil
            return true
        }
        canvas.framePointerHandler = { [weak self, weak canvas] event in
            guard let self, let canvas else { return false }
            let point = CanvasInput.viewPoint(of: event, in: canvas)
            if self.toolsModel.snapshot.activeTool == .zoom {
                if event.type == .leftMouseDown {
                    _ = canvas.cancelFrameGesture?()
                    guard event.modifierFlags.intersection([.command, .shift, .control]).isEmpty,
                          let session = self.windowController.windowSession, !session.isClosing, !session.isClosed,
                          let camera = session.viewInfo, camera.has_view == 1 else { return false }
                    canvas.window?.makeFirstResponder(canvas)
                    zoomDrag = (point, session.session, camera, canvas.bounds)
                    return true
                }
                guard let drag = zoomDrag else { return true }
                guard let session = self.windowController.windowSession, let camera = session.viewInfo,
                      !session.isClosing, !session.isClosed, session.session == drag.session, canvas.bounds == drag.bounds,
                      UUID(uuid: camera.active_view) == UUID(uuid: drag.camera.active_view),
                      camera.center_x == drag.camera.center_x, camera.center_y == drag.camera.center_y,
                      camera.zoom == drag.camera.zoom else { _ = canvas.cancelFrameGesture?(); return true }
                let area = CGRect(x: min(point.x, drag.start.x), y: min(point.y, drag.start.y),
                                  width: abs(point.x - drag.start.x), height: abs(point.y - drag.start.y))
                let isDrag = hypot(area.width, area.height) >= 3
                if event.type == .leftMouseDragged {
                    canvas.insertionOutline = isDrag ? CGPath(rect: area, transform: nil) : nil
                    return true
                }
                _ = canvas.cancelFrameGesture?()
                guard event.type == .leftMouseUp, let project = self.dataSource.session,
                      !isDrag || (area.width > 0 && area.height > 0) else { return true }
                let center = isDrag ? CGPoint(x: area.midX, y: area.midY) : drag.start
                let anchor = CGPoint(x: center.x - drag.bounds.midX, y: center.y - drag.bounds.midY)
                let fitFactor = isDrag ? Double(min(drag.bounds.width / area.width, drag.bounds.height / area.height)) : nil
                _ = session.track({ project.pump.zoomAt(session: session.session, anchor: anchor, fitFactor: fitFactor,
                                                        zoomOut: event.modifierFlags.contains(.option), completion: $0) }) { [weak self, weak session] _ in
                    guard let self, self.windowController.windowSession === session else { return }
                    self.refreshSessionCommands()
                }
                return true
            }
            if event.type == .leftMouseDown, event.clickCount == 2, self.toolsModel.snapshot.activeTool == .move,
               let hit = canvas.frameHit(at: point), hit.isText {
                return self.editCanvasText(hit, at: point, canvas: canvas)
            }
            if event.type == .leftMouseDown, self.toolsModel.snapshot.activeTool == .text,
               event.modifierFlags.intersection([.shift, .control, .command, .option]).isEmpty {
                return self.beginCanvasText(at: point, canvas: canvas)
            }
            if self.toolsModel.snapshot.activeTool == .image {
                if event.type == .leftMouseDown { return self.placePickedImage(at: point, canvas: canvas) }
                return true
            }
            if self.frameDrag?.tool.insertsBounds == true || (event.type == .leftMouseDown && self.toolsModel.snapshot.activeTool.insertsBounds) {
                return self.handleFrameInsertion(event, point: point, canvas: canvas)
            }
            if event.type == .leftMouseDown, self.toolsModel.snapshot.activeTool == .hand {
                _ = canvas.cancelFrameGesture?()
                guard event.modifierFlags.intersection([.command, .control, .shift, .option]).isEmpty,
                      let session = self.windowController.windowSession,
                      !session.isClosed, !session.isClosing, session.viewInfo?.has_view == 1 else { return false }
                canvas.window?.makeFirstResponder(canvas)
                handDrag = (point, session.session, canvas.bounds.size)
                return true
            }
            if let drag = handDrag, event.type == .leftMouseDragged || event.type == .leftMouseUp {
                guard self.toolsModel.snapshot.activeTool == .hand, canvas.bounds.size == drag.size,
                      let session = self.windowController.windowSession, session.session == drag.session else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                let dx = Double(point.x - drag.point.x), dy = Double(point.y - drag.point.y)
                guard dx.isFinite, dy.isFinite else { return true }
                handDrag = event.type == .leftMouseUp ? nil : (point, drag.session, drag.size)
                if !session.pan(dx: dx, dy: dy, completion: { [weak canvas] in canvas?.refreshFrames() }) {
                    _ = canvas.cancelFrameGesture?()
                }
                return true
            }
            switch event.type {
            case .leftMouseDown:
                _ = canvas.cancelFrameGesture?()
                guard self.toolsModel.snapshot.activeTool == .move,
                      event.modifierFlags.intersection([.command, .control, .shift, .option]).isEmpty,
                      let session = self.windowController.windowSession,
                      !session.isClosed, !session.isClosing, let viewport = session.viewInfo,
                      viewport.has_view == 1 else { return false }
                canvas.window?.makeFirstResponder(canvas)
                guard let hit = canvas.frameHit(at: point) else { return self.selectNodes([]) }
                guard session.info?.revision == hit.revision, UUID(uuid: viewport.active_view) == hit.viewID,
                      self.selectNodes([hit.id]) else { return false }
                if hit.isText || hit.isImage { return true }
                self.frameDrag = CanvasFrameDrag(hit: hit, start: point, session: session.session,
                                                 size: canvas.bounds.size, viewport: viewport, canMove: self.gitSource.trusted, tool: .move)
                return true
            case .leftMouseDragged:
                guard var drag = self.frameDrag else { return false }
                guard self.gitSource.trusted, self.toolsModel.snapshot.activeTool == .move else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                guard drag.canMove else { return true }
                let offset = CGVector(dx: point.x - drag.start.x, dy: point.y - drag.start.y)
                if !drag.moved {
                    guard hypot(offset.dx, offset.dy) >= 3 else { return true }
                    drag.moved = true
                    self.frameDrag = drag
                }
                canvas.setFrameDragPreview(id: drag.hit.id, offset: offset)
                return true
            case .leftMouseUp:
                guard let drag = self.frameDrag else { return false }
                _ = canvas.cancelFrameGesture?()
                guard drag.moved, drag.canMove, self.gitSource.trusted, self.toolsModel.snapshot.activeTool == .move,
                      canvas.bounds.size == drag.size, let session = self.windowController.windowSession,
                      session.session == drag.session, let camera = session.viewInfo,
                      camera.center_x == drag.viewport.center_x, camera.center_y == drag.viewport.center_y,
                      camera.zoom == drag.viewport.zoom else { return true }
                let parentToView = drag.hit.parentToWorld.concatenating(drag.hit.worldToView)
                let determinant = parentToView.a * parentToView.d - parentToView.b * parentToView.c
                guard determinant.isFinite, determinant != 0 else { return true }
                // 벡터에는 역변환의 translation이 들어가지 않습니다.
                let inverse = parentToView.inverted()
                let dx = point.x - drag.start.x, dy = point.y - drag.start.y
                let delta = CGVector(dx: inverse.a * dx + inverse.c * dy,
                                     dy: inverse.b * dx + inverse.d * dy)
                guard delta.dx.isFinite, delta.dy.isFinite, delta != .zero else { return true }
                _ = session.moveFrame(drag.hit, delta: delta) { [weak self] _ in
                    self?.refreshSessionCommands()
                }
                return true
            default: return false
            }
        }
    }
}
