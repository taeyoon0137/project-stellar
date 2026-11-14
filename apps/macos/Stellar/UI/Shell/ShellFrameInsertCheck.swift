import AppKit

/// 새 프로젝트가 저장한 800×600 Frame 안에서 실제 F 도구와 마우스 이벤트를 실행합니다.
@MainActor
enum ShellFrameInsertCheck {
    static func run(directory: URL) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let app = ShellAppCoordinator(runtimeHandle: host.handle)
        app.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarFrameInsert")
        defer { autosave.removeRecords() }
        app.windowConfiguration = { autosave.configuration }
        guard let pump = app.pump else { return false }
        let path = directory.appendingPathComponent("Frame Insert").path
        app.createProject(parentDirectory: directory.path, folderName: "Frame Insert")
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("frame-insert 실패: \(message)\n".utf8))
            return false
        }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 15) }
        guard wait(), let project = app.projectWindow(forPath: path), let window = project.window,
              let session = project.windowController.windowSession, let initial = session.info else { return fail("새 프로젝트 열기") }
        window.setContentSize(NSSize(width: 1300, height: 800))
        window.contentView?.layoutSubtreeIfNeeded()
        let canvas = project.canvasHost.canvasView
        func viewPoint(_ x: CGFloat, _ y: CGFloat, canvas target: CanvasView = canvas) -> NSPoint {
            // 실제 카메라와 800×600 root의 local 좌표를 사용합니다.
            let camera = (target.window?.windowController as? ShellWindowController)?.windowSession?.viewInfo
            let zoom = camera?.zoom ?? 1
            return CGPoint(x: target.bounds.midX + (x - (camera?.center_x ?? 0)) * zoom,
                           y: target.bounds.midY + (y - (camera?.center_y ?? 0)) * zoom)
        }
        func key(_ number: UInt16, _ chars: String, canvas target: CanvasView = canvas) -> Bool {
            guard let targetWindow = target.window else { return false }
            guard let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 1,
                windowNumber: targetWindow.windowNumber, context: nil, characters: chars, charactersIgnoringModifiers: chars,
                isARepeat: false, keyCode: number) else { return false }
            target.keyDown(with: event)
            return true
        }
        func pointer(_ kind: NSEvent.EventType, _ x: CGFloat, _ y: CGFloat, canvas target: CanvasView = canvas) -> Bool {
            ShellCanvasMoveCheck.mouse(target, kind, at: viewPoint(x, y, canvas: target))
        }
        func stroke() -> Bool {
            pointer(.leftMouseDown, 100, 90) && pointer(.leftMouseDragged, 65, 60) &&
                pointer(.leftMouseDragged, 40, 30) && pointer(.leftMouseUp, 40, 30) && wait()
        }
        func action(_ command: ShellCommand) -> Bool {
            project.router.performShellCommand(MainMenuBuilder.makeItem(for: command, target: nil))
            return wait()
        }
        guard let root = canvas.frameHit(at: viewPoint(50, 50))?.id,
              key(3, "f"), project.toolsModel.snapshot.activeTool == .frame,
              !project.gitSource.trusted, stroke(), session.info?.revision == initial.revision else { return fail("제한 모드에서 F 드래그") }
        project.gitSource.updateTrust(true)
        guard wait(), project.gitSource.trusted, pointer(.leftMouseDown, 40, 30), pointer(.leftMouseDragged, 90, 70),
              canvas.insertionOutline != nil, key(53, "\u{1b}"), pointer(.leftMouseUp, 90, 70), wait(),
              canvas.insertionOutline == nil, session.info?.revision == initial.revision,
              pointer(.leftMouseDown, 40, 30), pointer(.leftMouseDragged, 41, 31), pointer(.leftMouseUp, 41, 31), wait(),
              session.info?.undo_depth == initial.undo_depth else { return fail("취소·작은 드래그 보존") }
        guard action(.zoomOut), session.viewInfo?.zoom != nil else { return fail("삽입 전 창 배율 변경") }
        guard stroke(), let child = canvas.frameHit(at: viewPoint(50, 50))?.id, child != root,
              canvas.frameHit(at: viewPoint(101, 91))?.id == root,
              project.toolsModel.snapshot.activeTool == .move, session.info?.selection_count == 1,
              session.info?.undo_depth == initial.undo_depth + 1, session.info?.revision == initial.revision + 1
        else { return fail("Frame 생성·선택·도구 복귀") }
        guard action(.undo), canvas.frameHit(at: viewPoint(50, 50))?.id == root, session.info?.selection_count == 0,
              action(.redo), canvas.frameHit(at: viewPoint(50, 50))?.id == child, session.info?.selection_count == 1,
              action(.saveNow), session.info?.generation == initial.generation + 1 else { return fail("Undo·Redo·저장") }
        window.close()
        guard wait() else { return fail("닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let reopened = app.projectWindow(forPath: path), reopened !== project else { return fail("재열기") }
        reopened.window?.setContentSize(NSSize(width: 1300, height: 800))
        reopened.window?.contentView?.layoutSubtreeIfNeeded()
        let shown = reopened.canvasHost.canvasView
        guard shown.frameHit(at: viewPoint(41, 31, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(99, 89, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(101, 91, canvas: shown))?.id == root else { return fail("재열기 후 자식 위치·크기") }
        guard key(3, "f", canvas: shown), pointer(.leftMouseDown, 45, 36, canvas: shown),
              pointer(.leftMouseDragged, 62, 52, canvas: shown), pointer(.leftMouseUp, 62, 52, canvas: shown), wait(),
              let nested = shown.frameHit(at: viewPoint(46, 37, canvas: shown))?.id,
              nested != root, nested != child, shown.frameHit(at: viewPoint(61, 51, canvas: shown))?.id == nested,
              shown.frameHit(at: viewPoint(63, 53, canvas: shown))?.id == child else { return fail("이동된 부모의 local 좌표") }
        print("frame-insert ok: new project, F, restricted, cancel, tiny drag, undo/redo with selection, saved child geometry, translated parent")
        return true
    }
}
