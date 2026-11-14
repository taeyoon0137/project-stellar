import AppKit

extension ShellProjectWindow {
    func handleFrameInsertion(_ event: NSEvent, point: CGPoint, canvas: CanvasView) -> Bool {
        let tool = toolsModel.snapshot.activeTool
        switch event.type {
        case .leftMouseDown:
            _ = canvas.cancelFrameGesture?()
            guard tool == .frame, gitSource.trusted, event.modifierFlags.intersection([.command, .shift, .control, .option]).isEmpty,
                  let session = windowController.windowSession, !session.isClosed, !session.isClosing,
                  let viewport = session.viewInfo, viewport.has_view == 1, let parent = canvas.frameHit(at: point),
                  UUID(uuid: viewport.active_view) == parent.viewID, session.info?.revision == parent.revision else { return false }
            canvas.window?.makeFirstResponder(canvas)
            frameDrag = CanvasFrameDrag(hit: parent, start: point, session: session.session, size: canvas.bounds.size,
                viewport: viewport, canMove: true, tool: .frame)
            return true
        case .leftMouseDragged, .leftMouseUp:
            guard let drag = frameDrag else { return false }
            let release = event.type == .leftMouseUp
            if release { _ = canvas.cancelFrameGesture?() }
            guard tool == .frame, gitSource.trusted, canvas.bounds.size == drag.size,
                  let session = windowController.windowSession, session.session == drag.session,
                  !session.isClosing, !session.isClosed else {
                _ = canvas.cancelFrameGesture?()
                return true
            }
            var localToView = drag.hit.localToWorld.concatenating(drag.hit.worldToView)
            let det = localToView.a * localToView.d - localToView.b * localToView.c
            guard det.isFinite, det != 0 else { return true }
            let inverse = localToView.inverted()
            let start = drag.start.applying(inverse), end = point.applying(inverse)
            let rect = CGRect(x: min(start.x, end.x), y: min(start.y, end.y),
                width: abs(end.x - start.x), height: abs(end.y - start.y))
            // 양축 모두 화면 3 point 이상이어야 실제 면적의 Frame을 만듭니다.
            guard rect.width * hypot(localToView.a, localToView.b) >= 3,
                  rect.height * hypot(localToView.c, localToView.d) >= 3 else {
                canvas.insertionOutline = nil
                return true
            }
            if release {
                guard let camera = session.viewInfo, UUID(uuid: camera.active_view) == drag.hit.viewID,
                      camera.center_x == drag.viewport.center_x, camera.center_y == drag.viewport.center_y,
                      camera.zoom == drag.viewport.zoom else { return true }
                _ = session.insertFrame(parent: drag.hit, rect: rect) { [weak self] event in
                    guard let self else { return }
                    if case .windowViewChanged = event, self.toolsModel.snapshot.activeTool == .frame {
                        _ = self.router.dispatch(CanvasTool.move.command)
                    }
                    self.refreshSessionCommands()
                }
            } else {
                canvas.insertionOutline = CGPath(rect: rect, transform: &localToView)
            }
            return true
        default: return false
        }
    }
}
