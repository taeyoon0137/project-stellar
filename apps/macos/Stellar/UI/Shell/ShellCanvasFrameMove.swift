import AppKit

/// 누른 시점의 실제 표시 상태. 이동 중에는 문서 history에 쓰지 않습니다.
struct CanvasFrameDrag {
    let hit: CanvasFrameHit
    let start: NSPoint
    let session: UInt64
    let size: NSSize
    let viewport: StellarWindowViewInfo
    let tool: CanvasTool
    var moved = false
}

extension ShellProjectWindow {
    /// 화면 중심 기준 `anchor` point를 고정하고 배율을 `factor`배 합니다(핀치·⌘ 스크롤). 문서·history는 그대로입니다.
    @discardableResult
    func zoomCanvas(by factor: Double, anchor: CGPoint) -> Bool {
        guard factor.isFinite, factor > 0, let project = dataSource.session,
              let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              session.viewInfo?.has_view == 1 else { return false }
        _ = canvasHost.canvasView.cancelFrameGesture?()
        return session.track({ project.pump.zoomBy(session: session.session, anchor: anchor, factor: factor, completion: $0) }) {
            [weak self, weak session] _ in
            guard let self, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }

    /// 핸들 위 커서. 크기 핸들은 그 가장자리·모서리 방향, Line 끝점은 십자입니다.
    static func cursor(for handle: CanvasHandle) -> NSCursor {
        switch handle {
        case .resize(let index):
            let positions: [NSCursor.FrameResizePosition] = [.topLeft, .topRight, .bottomRight, .bottomLeft,
                                                              .top, .right, .bottom, .left]
            return NSCursor.frameResize(position: positions[index], directions: .all)
        case .lineEnd:
            return .crosshair
        case .radius:
            return .pointingHand
        }
    }

    /// 캔버스 편집 요청 하나를 보내고 끝나면 표시를 갱신합니다(ADR-0071).
    @discardableResult
    func submitCanvasEdit(
        _ submit: (ShellRuntimeEventPump, UInt64, @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError>
    ) -> Bool {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return false }
        return session.track({ submit(project.pump, session.session, $0) }) { [weak self, weak session] _ in
            guard let self, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }

    /// 화살표 키의 이동량(부모 공간 px). Shift는 10px입니다. ⌘·Option·Control이 있으면 nil입니다.
    static func arrowNudge(_ event: NSEvent) -> CGVector? {
        guard event.modifierFlags.intersection([.command, .option, .control]).isEmpty else { return nil }
        let step: CGFloat = event.modifierFlags.contains(.shift) ? 10 : 1
        switch event.keyCode {
        case 123: return CGVector(dx: -step, dy: 0)
        case 124: return CGVector(dx: step, dy: 0)
        case 125: return CGVector(dx: 0, dy: step)
        case 126: return CGVector(dx: 0, dy: -step)
        default: return nil
        }
    }

    /// 선택 전부를 옮깁니다(ADR-0071). 한 번이 Undo 한 번입니다.
    @discardableResult
    func moveSelection(anchor: UUID?, delta: CGVector, flags: UInt32) -> Bool {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed, let view = session.viewInfo, view.has_view == 1 else { return false }
        return session.track({ project.pump.canvasMove(session: session.session, view: UUID(uuid: view.active_view),
                                                       anchor: anchor, delta: delta, flags: flags, completion: $0) }) {
            [weak self, weak session] _ in
            guard let self, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }

    /// 그린 행에서 고른 선택을 코어로 보냅니다(ADR-0071). leaf가 nil이면 모든 Frame 밖입니다.
    @discardableResult
    func pickCanvas(view: UUID, leaf: UUID?, flags: UInt32) -> Bool {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return false }
        return session.track({ project.pump.canvasPick(session: session.session, view: view, leaf: leaf, flags: flags,
                                                       completion: $0) }) { [weak self, weak session] _ in
            guard let self, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }

    /// world 영역 선택을 코어로 보냅니다(ADR-0071).
    @discardableResult
    func pickCanvasArea(view: UUID, from: CGPoint, to: CGPoint, flags: UInt32) -> Bool {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return false }
        return session.track({ project.pump.canvasPickArea(session: session.session, view: view, from: from, to: to,
                                                           flags: flags, completion: $0) }) { [weak self, weak session] _ in
            guard let self, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }

    func connectFrameInteractions() {
        let canvas = canvasHost.canvasView
        // temporary: Space를 누르는 동안 시작한 임시 Hand. 도중에 Space를 놓아도 마우스를 놓을 때까지 팬합니다.
        var handDrag: (point: NSPoint, session: UInt64, size: NSSize, temporary: Bool)?
        var zoomDrag: (start: NSPoint, session: UInt64, camera: StellarWindowViewInfo, bounds: CGRect)?
        // 빈 곳(root 배경 또는 모든 Frame 밖)에서 시작한 영역 선택. leaf는 클릭으로 끝날 때 고를 행입니다.
        var marquee: (start: NSPoint, leaf: UUID?, view: UUID, session: UInt64, size: NSSize, moved: Bool)?
        // 선택 핸들 드래그. placement는 누른 시점의 local → 화면 변환입니다.
        var handleDrag: (box: CanvasSelectionBox, handle: CanvasHandle, start: NSPoint, session: UInt64, view: UUID,
                         size: NSSize, viewport: StellarWindowViewInfo, placement: CGAffineTransform)?
        // 진행 중인 캔버스 제스처 하나라도 있으면 true. 새 제스처 상태(크기 조절·영역 선택 등)도 여기에 더합니다.
        canvas.frameGestureActive = { [weak self] in
            self?.frameDrag != nil || handDrag != nil || zoomDrag != nil || marquee != nil || handleDrag != nil
        }
        canvas.hoverCursor = { [weak self, weak canvas] point in
            guard let self, let canvas, self.toolsModel.snapshot.activeTool == .move,
                  let (_, handle) = canvas.selectionHandle(at: point) else { return nil }
            return Self.cursor(for: handle)
        }
        canvas.cancelFrameGesture = { [weak self, weak canvas] in
            guard let self, self.frameDrag != nil || handDrag != nil || zoomDrag != nil || marquee != nil || handleDrag != nil
                || self.rejectedDraw
            else { return false }
            handDrag = nil
            zoomDrag = nil
            marquee = nil
            handleDrag = nil
            canvas?.gesturePreview = nil
            self.frameDrag = nil
            if self.rejectedDraw {
                self.rejectedDraw = false
                NSCursor.arrow.set()
            }
            canvas?.setFrameDragPreview(id: nil, offset: .zero)
            canvas?.insertionOutline = nil
            return true
        }
        canvas.framePointerHandler = { [weak self, weak canvas] event in
            guard let self, let canvas else { return false }
            let point = CanvasInput.viewPoint(of: event, in: canvas)
            if self.continueRejectedDraw(event) { return true }
            // 한 줄 안내는 다음 누름에서 사라집니다(UX 명세 3.2).
            if event.type == .leftMouseDown { self.windowController.toolbarController.hintView.hide() }
            // Hand 도구 또는 Space를 누르는 동안의 임시 Hand(UX 명세 5.1). 다른 도구보다 먼저 받습니다.
            if event.type == .leftMouseDown, self.toolsModel.snapshot.activeTool == .hand || canvas.isSpaceHeld {
                _ = canvas.cancelFrameGesture?()
                guard event.modifierFlags.intersection([.command, .control, .shift, .option]).isEmpty,
                      let session = self.windowController.windowSession,
                      !session.isClosed, !session.isClosing, session.viewInfo?.has_view == 1 else { return false }
                canvas.window?.makeFirstResponder(canvas)
                let temporary = self.toolsModel.snapshot.activeTool != .hand
                if temporary { NSCursor.closedHand.set() }
                handDrag = (point, session.session, canvas.bounds.size, temporary)
                return true
            }
            if let drag = handDrag, event.type == .leftMouseDragged || event.type == .leftMouseUp {
                guard drag.temporary || self.toolsModel.snapshot.activeTool == .hand, canvas.bounds.size == drag.size,
                      let session = self.windowController.windowSession, session.session == drag.session else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                let dx = Double(point.x - drag.point.x), dy = Double(point.y - drag.point.y)
                guard dx.isFinite, dy.isFinite else { return true }
                handDrag = event.type == .leftMouseUp ? nil : (point, drag.session, drag.size, drag.temporary)
                if event.type == .leftMouseUp, drag.temporary {
                    (canvas.isSpaceHeld ? NSCursor.openHand : NSCursor.arrow).set()
                }
                if !session.pan(dx: dx, dy: dy, completion: { [weak canvas] in canvas?.refreshFrames() }) {
                    _ = canvas.cancelFrameGesture?()
                }
                return true
            }
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
                if self.canvasTextEditing == nil, self.windowController.windowSession?.viewInfo?.has_view == 1,
                   canvas.frameHit(at: point, onlyFrames: true) == nil {
                    return self.rejectDrawOutsideFrame(canvas)
                }
                return self.beginCanvasText(at: point, canvas: canvas)
            }
            if self.toolsModel.snapshot.activeTool == .image {
                if event.type == .leftMouseDown {
                    // root Frame 밖 빈 캔버스는 파일을 고르기 전에 거부하고 이유를 보입니다(UX 명세 5.2).
                    if self.windowController.windowSession?.viewInfo?.has_view == 1,
                       canvas.frameHit(at: point, onlyFrames: true) == nil {
                        return self.rejectDrawOutsideFrame(canvas)
                    }
                    return self.placeImage(at: point, canvas: canvas)
                }
                return true
            }
            if self.frameDrag?.tool.insertsBounds == true || (event.type == .leftMouseDown && self.toolsModel.snapshot.activeTool.insertsBounds) {
                return self.handleFrameInsertion(event, point: point, canvas: canvas)
            }
            if let drag = handleDrag, event.type == .leftMouseDragged || event.type == .leftMouseUp {
                guard self.toolsModel.snapshot.activeTool == .move, canvas.bounds.size == drag.size,
                      let session = self.windowController.windowSession, session.session == drag.session else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                let det = drag.placement.a * drag.placement.d - drag.placement.b * drag.placement.c
                guard det.isFinite, det != 0 else { return true }
                let local = point.applying(drag.placement.inverted())
                guard local.x.isFinite, local.y.isFinite else { return true }
                let modifiers = event.modifierFlags
                var preview = CanvasGesturePreview(id: drag.box.id, rect: drag.box.rect,
                                                   lineStart: drag.box.lineStart, lineEnd: drag.box.lineEnd)
                switch drag.handle {
                case .resize(let index):
                    preview.rect = drag.box.rect.resized(handle: index, to: local, keepRatio: modifiers.contains(.shift),
                                                         fromCenter: modifiers.contains(.option))
                case .lineEnd(let end):
                    guard let other = end ? drag.box.lineStart : drag.box.lineEnd else { return true }
                    var target = local
                    if modifiers.contains(.shift) {
                        // Shift는 다른 끝점 기준 45° 단위입니다.
                        let dx = local.x - other.x, dy = local.y - other.y
                        let angle = (atan2(dy, dx) / (.pi / 4)).rounded() * (.pi / 4)
                        let length = hypot(dx, dy)
                        target = CGPoint(x: other.x + length * cos(angle), y: other.y + length * sin(angle))
                    }
                    if end { preview.lineEnd = target } else { preview.lineStart = target }
                case .radius(let index):
                    // 기본은 네 모서리, Option은 그 모서리 longhand 하나입니다(Polygon은 값 하나).
                    let value = CanvasFrameScene.radius(handle: index, in: drag.box.rect, at: local)
                    let single = index >= 0 && modifiers.contains(.option)
                    preview.radii = drag.box.radii.enumerated().map { !single || $0.offset == index ? value : $0.element }
                }
                if event.type == .leftMouseDragged {
                    if hypot(point.x - drag.start.x, point.y - drag.start.y) >= 1 { canvas.gesturePreview = preview }
                    return true
                }
                _ = canvas.cancelFrameGesture?()
                guard hypot(point.x - drag.start.x, point.y - drag.start.y) >= 1, let camera = session.viewInfo,
                      camera.center_x == drag.viewport.center_x, camera.center_y == drag.viewport.center_y,
                      camera.zoom == drag.viewport.zoom else { return true }
                let node = drag.box.id, view = drag.view
                switch drag.handle {
                case .resize:
                    // 크기 핸들 드래그 한 번이 Undo 한 번입니다. 어떤 longhand를 바꿀지는 코어가 정합니다.
                    self.submitCanvasEdit { pump, session, done in
                        pump.canvasResize(session: session, view: view, node: node, rect: preview.rect, completion: done)
                    }
                case .lineEnd(let end):
                    guard let target = end ? preview.lineEnd : preview.lineStart else { return true }
                    self.submitCanvasEdit { pump, session, done in
                        pump.canvasLineEnd(session: session, view: view, node: node, end: end, point: target, completion: done)
                    }
                case .radius(let index):
                    let value = CanvasFrameScene.radius(handle: index, in: drag.box.rect, at: local)
                    let corner = index >= 0 && modifiers.contains(.option) ? index : nil
                    self.submitCanvasEdit { pump, session, done in
                        pump.canvasRadius(session: session, view: view, node: node, corner: corner,
                                          radius: value.rounded(), completion: done)
                    }
                }
                return true
            }
            if var area = marquee, event.type == .leftMouseDragged || event.type == .leftMouseUp {
                guard self.toolsModel.snapshot.activeTool == .move, canvas.bounds.size == area.size,
                      let session = self.windowController.windowSession, session.session == area.session else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                let rect = CGRect(x: min(point.x, area.start.x), y: min(point.y, area.start.y),
                                  width: abs(point.x - area.start.x), height: abs(point.y - area.start.y))
                if !area.moved, hypot(rect.width, rect.height) >= 3 {
                    area.moved = true
                    marquee = area
                }
                if event.type == .leftMouseDragged {
                    canvas.insertionOutline = area.moved ? CGPath(rect: rect, transform: nil) : nil
                    return true
                }
                _ = canvas.cancelFrameGesture?()
                let modifiers = event.modifierFlags
                if area.moved {
                    // 영역은 world로 바꿔 보내고, 무엇을 고를지는 코어가 정합니다(Option 완전 포함, Shift 더하기).
                    let toWorld = canvas.worldToView.inverted()
                    var flags: UInt32 = 0
                    if modifiers.contains(.option) { flags |= UInt32(STELLAR_AREA_CONTAIN) }
                    if modifiers.contains(.shift) { flags |= UInt32(STELLAR_AREA_ADD) }
                    self.pickCanvasArea(view: area.view, from: area.start.applying(toWorld), to: point.applying(toWorld),
                                        flags: flags)
                } else {
                    self.pickCanvas(view: area.view, leaf: area.leaf,
                                    flags: modifiers.contains(.shift) ? UInt32(STELLAR_PICK_TOGGLE) : 0)
                }
                return true
            }
            switch event.type {
            case .leftMouseDown:
                _ = canvas.cancelFrameGesture?()
                guard self.toolsModel.snapshot.activeTool == .move,
                      event.modifierFlags.intersection([.control]).isEmpty,
                      let session = self.windowController.windowSession,
                      !session.isClosed, !session.isClosing, let viewport = session.viewInfo,
                      viewport.has_view == 1 else { return false }
                canvas.window?.makeFirstResponder(canvas)
                let view = UUID(uuid: viewport.active_view)
                if let (box, handle) = canvas.selectionHandle(at: point) {
                    handleDrag = (box, handle, point, session.session, view, canvas.bounds.size, viewport,
                                  canvas.selectionPlacement(box))
                    return true
                }
                guard let hit = canvas.frameHit(at: point), hit.id != canvas.rootFrameID else {
                    marquee = (point, canvas.frameHit(at: point)?.id, view, session.session, canvas.bounds.size, false)
                    return true
                }
                guard session.info?.revision == hit.revision, view == hit.viewID else { return false }
                // 무엇을 고를지(직계 자식, Shift 넣고 빼기, ⌘ 깊은 선택, 더블클릭 한 단계 들어가기)는 코어가 정합니다.
                // 누름(PRESS): 이미 고른 node를 누르면 코어가 선택을 그대로 둬서 여러 개를 함께 끌 수 있습니다.
                var flags = UInt32(STELLAR_PICK_PRESS)
                if event.modifierFlags.contains(.shift) { flags |= UInt32(STELLAR_PICK_TOGGLE) }
                if event.modifierFlags.contains(.command) { flags |= UInt32(STELLAR_PICK_DEEP) }
                if event.clickCount >= 2 { flags |= UInt32(STELLAR_PICK_ENTER) }
                guard self.pickCanvas(view: view, leaf: hit.id, flags: flags) else { return false }
                // Text·Image를 포함한 모든 node를 끌어 옮깁니다. 무엇을 옮길지는 놓을 때 코어 선택이 정합니다.
                self.frameDrag = CanvasFrameDrag(hit: hit, start: point, session: session.session,
                                                 size: canvas.bounds.size, viewport: viewport, tool: .move)
                return true
            case .leftMouseDragged:
                guard var drag = self.frameDrag else { return false }
                guard self.toolsModel.snapshot.activeTool == .move else {
                    _ = canvas.cancelFrameGesture?()
                    return true
                }
                let offset = CGVector(dx: point.x - drag.start.x, dy: point.y - drag.start.y)
                if !drag.moved {
                    guard hypot(offset.dx, offset.dy) >= 3 else { return true }
                    drag.moved = true
                    self.frameDrag = drag
                }
                let moving = canvas.selectionBoxes.filter { $0.has(STELLAR_CANVAS_ITEM_MOVABLE) }
                if event.modifierFlags.contains(.option) {
                    // Option: 원본은 그대로 두고 복제본 자리를 윤곽으로 보입니다.
                    canvas.setFrameDragPreview(ids: [], offset: .zero)
                    let ghost = CGMutablePath()
                    for box in moving {
                        var placement = box.world.concatenating(canvas.worldToView)
                            .concatenating(CGAffineTransform(translationX: offset.dx, y: offset.dy))
                        ghost.addPath(CGPath(rect: box.rect, transform: &placement))
                    }
                    canvas.insertionOutline = ghost
                } else {
                    canvas.insertionOutline = nil
                    canvas.setFrameDragPreview(ids: moving.isEmpty ? [drag.hit.id] : moving.map { $0.id }, offset: offset)
                }
                return true
            case .leftMouseUp:
                guard let drag = self.frameDrag else { return false }
                _ = canvas.cancelFrameGesture?()
                if !drag.moved {
                    // 끌지 않고 놓으면 누를 때 미룬 선택 변경(하나로 줄이기, Shift 빼기)을 코어가 적용합니다.
                    if let session = self.windowController.windowSession, session.session == drag.session {
                        self.pickCanvas(view: drag.hit.viewID, leaf: drag.hit.id, flags: UInt32(STELLAR_PICK_RELEASE))
                    }
                    return true
                }
                guard self.toolsModel.snapshot.activeTool == .move,
                      canvas.bounds.size == drag.size, let session = self.windowController.windowSession,
                      session.session == drag.session, let camera = session.viewInfo,
                      camera.center_x == drag.viewport.center_x, camera.center_y == drag.viewport.center_y,
                      camera.zoom == drag.viewport.zoom else { return true }
                // 화면 이동량을 world 벡터로 바꿔 보냅니다(벡터라 역변환의 translation은 쓰지 않음). 부모 공간 변환은 코어가 합니다.
                let inverse = drag.hit.worldToView.inverted()
                let dx = point.x - drag.start.x, dy = point.y - drag.start.y
                let delta = CGVector(dx: inverse.a * dx + inverse.c * dy, dy: inverse.b * dx + inverse.d * dy)
                guard delta.dx.isFinite, delta.dy.isFinite, delta != .zero else { return true }
                let flags = event.modifierFlags.contains(.option) ? UInt32(STELLAR_MOVE_DUPLICATE) : 0
                self.moveSelection(anchor: drag.hit.id, delta: delta, flags: flags)
                return true
            default: return false
            }
        }
    }
}
