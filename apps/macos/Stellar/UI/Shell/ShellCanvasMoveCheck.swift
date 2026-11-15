import AppKit

/// 실제 기본 창에서 저장된 Frame에 AppKit pointer 입력을 전달하고 Undo, 저장, 재열기를 확인합니다.
enum ShellCanvasMoveCheck {
    @MainActor static func mouse(_ canvas: CanvasView, _ kind: NSEvent.EventType, at point: CGPoint,
                                 modifiers: NSEvent.ModifierFlags = []) -> Bool {
        guard let owner = canvas.window else { return false }
        let location = canvas.convert(point, to: nil)
        func event(at location: NSPoint, window: Int) -> NSEvent? {
            NSEvent.mouseEvent(with: kind, location: location, modifierFlags: modifiers,
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window,
                context: nil, eventNumber: 1, clickCount: 1, pressure: 1)
        }
        // 화면에 올리지 않은 창은 AppKit이 NSEvent.window를 연결하지 않을 수 있습니다.
        guard let local = event(at: location, window: owner.windowNumber),
              let event = local.window === owner ? local : event(at: owner.convertPoint(toScreen: location), window: 0)
        else { return false }
        let actual = CanvasInput.viewPoint(of: event, in: canvas)
        guard hypot(actual.x - point.x, actual.y - point.y) < 0.01 else {
            return false
        }
        switch kind {
        case .leftMouseDown: canvas.mouseDown(with: event)
        case .leftMouseDragged: canvas.mouseDragged(with: event)
        case .leftMouseUp: canvas.mouseUp(with: event)
        default: return false
        }
        return true
    }

    @MainActor private static func checkZoomTool(_ shell: ShellProjectWindow, other: ShellProjectWindow, pump: ShellRuntimeEventPump) -> Bool {
        let canvas = shell.canvasHost.canvasView
        func designBytes() throws -> [String: Data] {
            let design = URL(fileURLWithPath: shell.dataSource.rootPath).appendingPathComponent("design")
            var files: [String: Data] = [:]
            for path in try FileManager.default.subpathsOfDirectory(atPath: design.path) {
                let url = design.appendingPathComponent(path)
                if try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true { files[path] = try Data(contentsOf: url) }
            }
            return files
        }
        guard let owner = shell.window, let session = shell.windowController.windowSession,
              let camera = session.viewInfo, let info = session.info, let otherCamera = other.windowController.windowSession?.viewInfo,
              let target = session.selectionTarget, let original = try? designBytes(), !original.isEmpty,
              let z = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1, windowNumber: owner.windowNumber,
                                       context: nil, characters: "z", charactersIgnoringModifiers: "z", isARepeat: false, keyCode: 6)
        else { return false }
        canvas.keyDown(with: z)
        let center = CGPoint(x: canvas.bounds.midX, y: canvas.bounds.midY)
        let click = CGPoint(x: center.x - 30, y: center.y)
        let hit = canvas.frameHit(at: click)?.id
        func clickWith(_ flags: NSEvent.ModifierFlags) -> Bool {
            mouse(canvas, .leftMouseDown, at: click, modifiers: flags) && mouse(canvas, .leftMouseUp, at: click, modifiers: flags) &&
                pump.waitForPendingRequests(timeout: 10)
        }
        guard shell.toolsModel.snapshot.activeTool == .zoom, hit != nil, clickWith([]), let closer = session.viewInfo,
              closer.zoom == camera.zoom * 2, closer.center_x - 30 / closer.zoom == camera.center_x - 30 / camera.zoom,
              canvas.frameHit(at: click)?.id == hit, clickWith(.option), session.viewInfo?.zoom == camera.zoom,
              session.viewInfo?.center_x == camera.center_x else { return false }
        let a = CGPoint(x: center.x + canvas.bounds.width / 4 + 12, y: center.y + canvas.bounds.height / 4 - 8)
        let b = CGPoint(x: center.x - canvas.bounds.width / 4 + 12, y: center.y - canvas.bounds.height / 4 - 8)
        guard mouse(canvas, .leftMouseDown, at: a), mouse(canvas, .leftMouseDragged, at: b), canvas.insertionOutline != nil,
              mouse(canvas, .leftMouseUp, at: b), canvas.insertionOutline == nil, pump.waitForPendingRequests(timeout: 10),
              session.viewInfo?.zoom == camera.zoom * 2, session.viewInfo?.center_x == camera.center_x + 12 / camera.zoom,
              session.viewInfo?.center_y == camera.center_y - 8 / camera.zoom else { return false }
        guard mouse(canvas, .leftMouseDown, at: a), mouse(canvas, .leftMouseDragged, at: b), canvas.insertionOutline != nil,
              let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1, windowNumber: owner.windowNumber,
                                            context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)
        else { return false }
        canvas.keyDown(with: escape)
        let undoAreaZoom = CGPoint(x: center.x + 24, y: center.y - 16)
        guard canvas.insertionOutline == nil, mouse(canvas, .leftMouseUp, at: b), pump.waitForPendingRequests(timeout: 10),
              session.viewInfo?.zoom == camera.zoom * 2, mouse(canvas, .leftMouseDown, at: undoAreaZoom, modifiers: .option),
              mouse(canvas, .leftMouseUp, at: undoAreaZoom, modifiers: .option), pump.waitForPendingRequests(timeout: 10),
              session.viewInfo?.zoom == camera.zoom, session.viewInfo?.center_x == camera.center_x,
              session.viewInfo?.center_y == camera.center_y, shell.router.dispatch(.selectMoveTool),
              let next = session.info, target.matches(session.selectionTarget), next.revision == info.revision,
              next.generation == info.generation, next.undo_depth == info.undo_depth, next.redo_depth == info.redo_depth,
              next.has_unsaved_changes == info.has_unsaved_changes, let still = other.windowController.windowSession?.viewInfo,
              still.zoom == otherCamera.zoom, still.center_x == otherCamera.center_x, still.center_y == otherCamera.center_y,
              (try? designBytes()) == original else { return false }
        print("zoom-tool ok: Z, anchored click and Option-click, reversed area drag, Escape, selection/history, saved design bytes and other window preserved")
        return true
    }

    @MainActor static func run(fixture: URL) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let projects = ShellSavedProjectFixture(prefix: "StellarCanvasMove")
        defer { projects.remove() }
        guard let path = projects.copy(fixture, as: "A"), let otherPath = projects.copy(fixture, as: "B"),
              let node = UUID(uuidString: "00000000-000b-7000-8000-00000000000b") else { return false }
        let runtime = RuntimeHost()
        defer { runtime.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: runtime.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarCanvasMove")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        guard let pump = coordinator.pump else { return false }
        func fail(_ step: String) -> Bool {
            FileHandle.standardError.write(Data("canvas-move 실패: \(step)\n".utf8))
            return false
        }
        func open(_ path: String) -> ShellProjectWindow? {
            coordinator.openProject(atPath: path, origin: .application)
            guard pump.waitForPendingRequests(timeout: 10), let project = coordinator.projectWindow(forPath: path) else { return nil }
            project.window?.setContentSize(NSSize(width: 1300, height: 800))
            project.window?.contentView?.layoutSubtreeIfNeeded()
            return project
        }
        func point(_ canvas: CanvasView, _ x: CGFloat) -> NSPoint {
            let zoom = canvas.window.flatMap { ($0.windowController as? ShellWindowController)?.windowSession?.viewInfo?.zoom } ?? 1
            return NSPoint(x: canvas.bounds.midX + x * zoom, y: canvas.bounds.midY - 10 * zoom)
        }
        func mouse(_ canvas: CanvasView, _ kind: NSEvent.EventType, _ x: CGFloat) -> Bool {
            Self.mouse(canvas, kind, at: point(canvas, x))
        }
        func bitmap(_ canvas: CanvasView) -> Data? {
            guard let rep = canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds) else { return nil }
            canvas.cacheDisplay(in: canvas.bounds, to: rep)
            guard let bytes = rep.bitmapData else { return nil }
            return Data(bytes: bytes, count: rep.bytesPerRow * rep.pixelsHigh)
        }
        func drag(_ canvas: CanvasView) -> Bool {
            mouse(canvas, .leftMouseDown, -30) && mouse(canvas, .leftMouseDragged, -5) &&
            mouse(canvas, .leftMouseDragged, 25) && mouse(canvas, .leftMouseUp, 30) &&
            pump.waitForPendingRequests(timeout: 10)
        }
        func action(_ window: ShellProjectWindow, _ command: ShellCommand) -> Bool {
            window.router.performShellCommand(MainMenuBuilder.makeItem(for: command, target: nil))
            return pump.waitForPendingRequests(timeout: 10)
        }
        guard let a = open(path), let b = open(otherPath), let sa = a.windowController.windowSession,
              let sb = b.windowController.windowSession, let initial = sa.info else { return fail("두 프로젝트 열기") }
        let canvas = a.canvasHost.canvasView
        func at(_ x: CGFloat, in canvas: CanvasView) -> UUID? { canvas.frameHit(at: point(canvas, x))?.id }
        guard at(-30, in: canvas) == node, at(30, in: canvas) != node else { return fail("저장 Frame의 최초 좌표") }
        // 제한 모드도 문서 편집을 허용합니다(사용자 결정 D1). 아래 이동 검사는 신뢰 없이 실행합니다.
        guard !a.gitSource.trusted, mouse(canvas, .leftMouseDown, -30), mouse(canvas, .leftMouseUp, -30),
              pump.waitForPendingRequests(timeout: 10), let restricted = sa.info else { return fail("제한 모드 클릭 선택") }
        guard restricted.selection_count == 1, restricted.revision == initial.revision,
              restricted.has_unsaved_changes == 0 else {
            return fail("제한 모드 선택 \(restricted.selection_count), revision \(initial.revision)→\(restricted.revision), dirty \(restricted.has_unsaved_changes)")
        }
        guard mouse(canvas, .leftMouseDown, 240), mouse(canvas, .leftMouseUp, 240),
              pump.waitForPendingRequests(timeout: 10), sa.info?.selection_count == 0,
              mouse(canvas, .leftMouseDown, -30), mouse(canvas, .leftMouseUp, -30),
              pump.waitForPendingRequests(timeout: 10), sa.info?.selection_count == 1 else {
            return fail("빈 공간 선택 해제·Frame 재선택")
        }
        // Hand는 제한 모드에서도 문서를 편집하지 않고 이 창의 카메라만 움직입니다.
        let beforeHand = sa.viewInfo
        let beforeOther = sb.viewInfo
        let beforeDocument = sa.info
        let beforeHandPixels = bitmap(canvas)
        guard a.router.dispatch(.selectHandTool),
              mouse(canvas, .leftMouseDown, -30), mouse(canvas, .leftMouseDragged, -5),
              mouse(canvas, .leftMouseDragged, 25), mouse(canvas, .leftMouseUp, 30),
              pump.waitForPendingRequests(timeout: 10), let hand = sa.viewInfo, let beforeHand,
              hand.center_x == beforeHand.center_x - 60 / beforeHand.zoom,
              hand.center_y == beforeHand.center_y, hand.zoom == beforeHand.zoom,
              sb.viewInfo?.center_x == beforeOther?.center_x, sb.viewInfo?.center_y == beforeOther?.center_y,
              sa.info?.revision == beforeDocument?.revision, sa.info?.undo_depth == beforeDocument?.undo_depth,
              sa.info?.selection_count == beforeDocument?.selection_count,
              sa.info?.has_unsaved_changes == beforeDocument?.has_unsaved_changes,
              let beforeHandPixels, let handPixels = bitmap(canvas), handPixels != beforeHandPixels,
              at(30, in: canvas) == node else { return fail("Hand 드래그 창별 팬·표시·문서 보존") }
        // mouseUp 뒤 움직임과 Escape 뒤 움직임은 팬을 계속하지 않습니다.
        guard mouse(canvas, .leftMouseDragged, 60), pump.waitForPendingRequests(timeout: 10),
              sa.viewInfo?.center_x == hand.center_x else { return fail("Hand mouseUp 뒤 팬 종료") }
        guard mouse(canvas, .leftMouseDown, 30),
              let cancel = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1,
                  windowNumber: a.window?.windowNumber ?? 0, context: nil, characters: "\u{1b}",
                  charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53) else { return fail("Hand 종료 입력") }
        canvas.keyDown(with: cancel)
        guard mouse(canvas, .leftMouseDragged, 60), mouse(canvas, .leftMouseUp, 60),
              pump.waitForPendingRequests(timeout: 10), sa.viewInfo?.center_x == hand.center_x
        else { return fail("Hand Escape 뒤 팬 취소") }
        guard sa.pan(dx: -60, dy: 0, completion: { canvas.refreshFrames() }),
              pump.waitForPendingRequests(timeout: 10),
              a.router.dispatch(.selectMoveTool), at(-30, in: canvas) == node else { return fail("Hand 카메라·scene 복원") }
        guard checkZoomTool(a, other: b, pump: pump) else { return fail("Z, Option-클릭·영역 줌·다른 창과 문서 보존") }
        guard let selectedPixels = bitmap(canvas), mouse(canvas, .leftMouseDown, -30), mouse(canvas, .leftMouseDragged, 30),
              let previewPixels = bitmap(canvas), selectedPixels != previewPixels, sa.info?.revision == initial.revision,
              let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1,
                  windowNumber: a.window?.windowNumber ?? 0, context: nil, characters: "\u{1b}",
                  charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53) else { return fail("취소 입력 생성") }
        canvas.keyDown(with: escape)
        guard mouse(canvas, .leftMouseUp, 30), pump.waitForPendingRequests(timeout: 10),
              bitmap(canvas) == selectedPixels, sa.info?.revision == initial.revision,
              let selected = sa.info else { return fail("Escape 뒤 이동 미반영") }
        guard action(a, .zoomOut), sa.viewInfo?.zoom == 0.5,
              at(-30, in: canvas) == node, sb.viewInfo?.zoom == 1 else { return fail("창별 50% 카메라") }
        let halfCamera = sa.viewInfo
        guard a.router.dispatch(.selectHandTool),
              mouse(canvas, .leftMouseDown, -30), mouse(canvas, .leftMouseDragged, 30),
              mouse(canvas, .leftMouseUp, 30), pump.waitForPendingRequests(timeout: 10),
              sa.viewInfo?.center_x == (halfCamera?.center_x ?? .nan) - 60,
              sa.viewInfo?.zoom == 0.5, at(30, in: canvas) == node,
              sa.info?.revision == selected.revision,
              sa.pan(dx: -30, dy: 0, completion: { canvas.refreshFrames() }), pump.waitForPendingRequests(timeout: 10),
              a.router.dispatch(.selectMoveTool), at(-30, in: canvas) == node
        else { return fail("50% Hand 팬·복원") }
        guard drag(canvas), sa.info?.revision == selected.revision + 1, sa.info?.undo_depth == selected.undo_depth + 1,
              at(30, in: canvas) == node, at(-30, in: canvas) != node,
              sb.info?.selection_count == 0, sb.info?.revision == initial.revision,
              at(-30, in: b.canvasHost.canvasView) == node else { return fail("Frame이 한 번만 이동·다른 창 보존") }
        guard action(a, .undo), at(-30, in: canvas) == node, action(a, .redo), at(30, in: canvas) == node,
              action(a, .saveNow), let saved = sa.info, saved.generation == initial.generation + 1,
              saved.has_unsaved_changes == 0 else { return fail("Undo·Redo·Save Now") }
        a.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), let reopened = open(path), reopened !== a,
              reopened.windowController.windowSession?.info?.generation == saved.generation,
              at(30, in: reopened.canvasHost.canvasView) == node, at(-30, in: reopened.canvasHost.canvasView) != node
        else { return fail("저장된 원본 재열기 좌표") }
        // 기존 일반 창 경로에서 Delete/Backspace → Undo/Redo → 저장·재열기를 검사합니다.
        reopened.gitSource.updateTrust(true)
        guard pump.waitForPendingRequests(timeout: 10), let deleteSession = reopened.windowController.windowSession,
              reopened.selectNodes([node]), pump.waitForPendingRequests(timeout: 10) else { return fail("삭제 선택") }
        let deleteCanvas = reopened.canvasHost.canvasView
        let board = NSPasteboard.withUniqueName()
        defer { board.releaseGlobally() }
        reopened.subtreePasteboard = board
        let clipboardDepth = deleteSession.info?.undo_depth
        guard reopened.router.dispatch(.copy) else { return fail("선택 복사 요청") }
        let clipboardType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.node-subtree.v1")
        let deadline = Date().addingTimeInterval(5)
        while board.availableType(from: [clipboardType]) == nil && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        guard let copied = board.data(forType: clipboardType), copied.count > 4096,
              clipboardDepth == deleteSession.info?.undo_depth,
              action(reopened, .delete), action(reopened, .paste),
              let pasted = at(30, in: deleteCanvas), pasted != node,
              action(reopened, .undo), at(30, in: deleteCanvas) != pasted,
              action(reopened, .redo), at(30, in: deleteCanvas) == pasted,
              action(reopened, .undo), action(reopened, .undo), at(30, in: deleteCanvas) == node
        else { return fail("원본 삭제 후 pasteboard 복제·Undo·Redo") }
        // Duplicate 메뉴(⌘D)도 실제 창의 선택과 동일한 history를 사용합니다.
        let beforeDuplicate = deleteSession.info
        guard action(reopened, .duplicate), let copy = at(30, in: deleteCanvas), copy != node,
              deleteSession.info?.selection_count == 1,
              deleteSession.info?.undo_depth == (beforeDuplicate?.undo_depth ?? 0) + 1,
              action(reopened, .undo), at(30, in: deleteCanvas) == node,
              action(reopened, .redo), at(30, in: deleteCanvas) == copy,
              action(reopened, .undo), at(30, in: deleteCanvas) == node
        else { return fail("단일 선택 복제·Undo·Redo") }
        // 재생된 Undo가 끝나도 Layers의 새 페이지는 worker에서 도착합니다.
        func awaitLayerAction(_ command: SidebarCommand) -> Bool {
            let deadline = Date().addingTimeInterval(5)
            while !reopened.sidebarSource.validate(command).isAllowed && Date() < deadline {
                RunLoop.current.run(until: Date().addingTimeInterval(0.01))
            }
            return reopened.sidebarSource.validate(command).isAllowed
        }
        let layerDuplicate = SidebarCommand.duplicate([SidebarStableID(node)])
        guard awaitLayerAction(layerDuplicate) else { return fail("Layers 복제 활성") }
        // 실제 선택만 바꾸고 셸을 갱신하지 않아 같은 revision·같은 개수의 오래된 메뉴와 Layers를 재현합니다.
        guard let sibling = reopened.sidebarSource.snapshot.layers.topLevelIDs.first(where: { $0.uuid != node })?.uuid else {
            return fail("다른 형제 레이어")
        }
        // Copy completion always follows this serial queue marker on main. No wall-clock
        // delay can turn a late pasteboard write into a false success.
        func finishCopy() -> Bool {
            var finished = false
            reopened.clipboardQueue.async { DispatchQueue.main.async { finished = true } }
            let until = Date().addingTimeInterval(5)
            while !finished && Date() < until { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            return finished
        }
        let boardVersion = board.changeCount
        guard let shown = deleteSession.selectionTarget,
              deleteSession.select([sibling], completion: { _ in }), pump.waitForPendingRequests(timeout: 10),
              let changed = deleteSession.selectionTarget, changed.revision == shown.revision,
              changed.history_sequence != shown.history_sequence, changed.selection_count == shown.selection_count,
              reopened.performSessionCommand(.copy), finishCopy(), board.changeCount == boardVersion,
              reopened.performSessionCommand(.paste), pump.waitForPendingRequests(timeout: 10),
              case .requestFailed(_, let failure, _) = reopened.lastSessionCommandEvent,
              Int32(bitPattern: failure) == STELLAR_FAILURE_UNKNOWN_VIEW,
              changed.matches(deleteSession.selectionTarget), action(reopened, .undo), shown.matches(deleteSession.selectionTarget)
        else { return fail("같은 revision에서 바뀐 선택으로 Copy/Paste가 적용됨") }


        for command in [ShellCommand.delete, .duplicate] {
            let layerCommand: SidebarCommand = command == .delete ? .delete([SidebarStableID(node)]) : layerDuplicate
            for fromLayers in [true, false] {
                let scenario = "\(fromLayers ? "Layers" : "메뉴") \(command.rawValue)"
                func diagnostics(_ reason: String) -> Bool {
                    let live = deleteSession.selectionTarget
                    let ids = reopened.sidebarSource.snapshot.selection.map { $0.uuid.uuidString }
                    return fail([
                        "\(scenario) \(reason)",
                        "live revision=\(String(describing: live?.revision)) history=\(String(describing: live?.history_sequence)) count=\(String(describing: live?.selection_count))",
                        "Layers=\(ids) expected=\(node) sibling=\(sibling)",
                        "event=\(String(describing: reopened.lastSessionCommandEvent))"
                    ].joined(separator: ", "))
                }
                guard awaitLayerAction(layerCommand) else { return diagnostics("초기 Layers 명령이 활성화되지 않음") }
                guard let shown = deleteSession.selectionTarget else { return diagnostics("선택 대상 없음") }
                var selectedEvent: CoreRuntimeEvent?
                guard deleteSession.select([sibling], completion: { selectedEvent = $0 }), pump.waitForPendingRequests(timeout: 10),
                      case .windowViewChanged = selectedEvent else { return diagnostics("선택 요청 또는 완료 실패") }
                guard let changed = deleteSession.selectionTarget, changed.revision == shown.revision,
                      changed.selection_count == shown.selection_count, changed.history_sequence != shown.history_sequence,
                      let before = deleteSession.info else { return diagnostics("선택 전환 실패, 전 history \(shown.history_sequence)") }
                let previousRequest = reopened.lastSessionCommandEvent?.request
                if fromLayers { reopened.sidebarSource.perform(layerCommand) }
                else { guard reopened.router.dispatch(command) else { return fail("오래된 메뉴 명령 제출") } }
                guard pump.waitForPendingRequests(timeout: 10),
                      case .requestFailed(let request, let failure, _) = reopened.lastSessionCommandEvent, request != previousRequest,
                      Int32(bitPattern: failure) == STELLAR_FAILURE_UNKNOWN_VIEW,
                      deleteSession.info?.revision == before.revision, deleteSession.info?.undo_depth == before.undo_depth,
                      deleteSession.info?.has_unsaved_changes == before.has_unsaved_changes,
                      deleteSession.selectionTarget?.history_sequence == changed.history_sequence,
                      action(reopened, .undo), deleteSession.selectionTarget?.history_sequence == shown.history_sequence
                else { return diagnostics("오래된 명령 거부 또는 Undo 복원 실패, 이전 요청 \(String(describing: previousRequest)), 전 history \(shown.history_sequence), 변경 history \(changed.history_sequence)") }
            }
        }
        guard awaitLayerAction(layerDuplicate) else { return fail("선택 복원 후 Layers 복제 활성") }
        reopened.sidebarSource.perform(layerDuplicate)
        guard pump.waitForPendingRequests(timeout: 10), let layerCopy = at(30, in: deleteCanvas), layerCopy != node,
              action(reopened, .undo), at(30, in: deleteCanvas) == node
        else { return fail("Layers 복제·Undo") }
        for code in [UInt16(51), UInt16(117)] {
            guard let key = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1,
                windowNumber: reopened.window?.windowNumber ?? 0, context: nil, characters: "\u{7f}",
                charactersIgnoringModifiers: "\u{7f}", isARepeat: false, keyCode: code) else { return fail("삭제 키 생성") }
            let before = deleteSession.info
            deleteCanvas.keyDown(with: key)
            guard pump.waitForPendingRequests(timeout: 10), deleteSession.info?.selection_count == 0,
                  deleteSession.info?.undo_depth == (before?.undo_depth ?? 0) + 1,
                  at(30, in: deleteCanvas) != node, action(reopened, .undo),
                  deleteSession.info?.selection_count == 1, at(30, in: deleteCanvas) == node
            else { return fail("삭제 키·Undo 선택 복원") }
        }
        // Layers의 삭제 요청도 같은 transaction으로 이어집니다.
        let layerDelete = SidebarCommand.delete([SidebarStableID(node)])
        guard awaitLayerAction(layerDelete) else { return fail("Layers 삭제 활성") }
        reopened.sidebarSource.perform(layerDelete)
        guard pump.waitForPendingRequests(timeout: 10), at(30, in: deleteCanvas) != node,
              action(reopened, .undo), at(30, in: deleteCanvas) == node,
              action(reopened, .redo), at(30, in: deleteCanvas) != node,
              action(reopened, .saveNow), deleteSession.info?.has_unsaved_changes == 0
        else { return fail("Layers 삭제·Undo·Redo·저장") }
        let deletedGeneration = deleteSession.info?.generation
        reopened.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), let deleted = open(path), deleted !== reopened,
              deleted.windowController.windowSession?.info?.generation == deletedGeneration,
              at(30, in: deleted.canvasHost.canvasView) != node else { return fail("삭제 저장 재열기") }
        print("canvas-move ok: restricted select, Hand pan/display/isolation/cancel, Escape, two drag updates/one undo, second window, save/reopen, Copy/Paste after source deletion, 4 KiB retry, stale clipboard selection, Duplicate history, Delete/Backspace and Layers deletion history/save/reopen, stale same-count menu and Layers targets")
        return true
    }
}
