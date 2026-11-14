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
        canvas.cancelFrameGesture = { [weak self, weak canvas] in
            guard let self, self.frameDrag != nil || handDrag != nil else { return false }
            handDrag = nil
            self.frameDrag = nil
            canvas?.setFrameDragPreview(id: nil, offset: .zero)
            canvas?.insertionOutline = nil
            return true
        }
        canvas.framePointerHandler = { [weak self, weak canvas] event in
            guard let self, let canvas else { return false }
            let point = CanvasInput.viewPoint(of: event, in: canvas)
            if event.type == .leftMouseDown, self.toolsModel.snapshot.activeTool == .text,
               event.modifierFlags.intersection([.shift, .control, .command, .option]).isEmpty {
                return self.beginCanvasText(at: point, canvas: canvas)
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
                if hit.isInstance { return true }
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
