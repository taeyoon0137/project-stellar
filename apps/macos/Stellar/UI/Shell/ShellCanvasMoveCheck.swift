import AppKit

/// 실제 기본 창에서 저장된 Frame에 AppKit pointer 입력을 전달하고 Undo, 저장, 재열기를 확인합니다.
enum ShellCanvasMoveCheck {
    @MainActor static func run(fixture: URL) -> Bool {
        let projects = ShellSavedProjectFixture(prefix: "StellarCanvasMove")
        defer { projects.remove() }
        guard let path = projects.copy(fixture, as: "A"), let otherPath = projects.copy(fixture, as: "B"),
              let node = UUID(uuidString: "00000000-000b-7000-8000-00000000000b") else { return false }
        NSApplication.shared.setActivationPolicy(.accessory)
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
            guard let owner = canvas.window else { return false }
            let location = canvas.convert(point(canvas, x), to: nil)
            func event(at location: NSPoint, window: Int) -> NSEvent? {
                NSEvent.mouseEvent(with: kind, location: location, modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window,
                    context: nil, eventNumber: 1, clickCount: 1, pressure: 1)
            }
            // 화면에 올리지 않은 창은 AppKit이 NSEvent.window를 연결하지 않을 수 있습니다.
            guard let local = event(at: location, window: owner.windowNumber),
                  let event = local.window === owner ? local : event(at: owner.convertPoint(toScreen: location), window: 0)
            else { return false }
            let actual = CanvasInput.viewPoint(of: event, in: canvas)
            guard hypot(actual.x - point(canvas, x).x, actual.y - point(canvas, x).y) < 0.01 else {
                return fail("합성 pointer가 실제 canvas 좌표에 도착하지 않음: \(point(canvas, x)) → \(actual), event.window \(String(describing: event.window?.windowNumber))")
            }
            switch kind {
            case .leftMouseDown: canvas.mouseDown(with: event)
            case .leftMouseDragged: canvas.mouseDragged(with: event)
            case .leftMouseUp: canvas.mouseUp(with: event)
            default: return false
            }
            return true
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
        guard !a.gitSource.trusted, drag(canvas), let restricted = sa.info else { return fail("제한 모드 입력 제출") }
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
        a.gitSource.updateTrust(true)
        guard pump.waitForPendingRequests(timeout: 10), a.gitSource.trusted else { return fail("격리된 프로젝트 신뢰") }
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
        print("canvas-move ok: restricted select, Hand pan/display/isolation/cancel, Escape, two drag updates/one undo, second window, save and reopen")
        return true
    }
}
