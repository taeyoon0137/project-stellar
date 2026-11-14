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
        func awaitSidebar(_ matches: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(5)
            while !matches(), Date() < deadline {
                RunLoop.current.run(until: Date().addingTimeInterval(0.01))
            }
            return matches()
        }
        func layerRow(_ id: UUID, in owner: ShellProjectWindow) -> Int? {
            let outline = owner.sidebarSlot.sidebar.layersController.outline
            return (0..<outline.numberOfRows).first {
                (outline.item(atRow: $0) as? SidebarOutlineItem)?.layerID?.uuid == id
            }
        }
        func showsActiveView(_ owner: ShellProjectWindow) -> Bool {
            let sidebar = owner.sidebarSlot.sidebar
            let state = sidebar.pagesController.snapshot
            guard let view = owner.windowController.windowSession?.viewInfo,
                  state.loadState == .ready, state.pages.count == 1, let page = state.pages.first,
                  page.id == state.activePageID, page.name == "Page 1", page.views.count == 1,
                  page.views.first?.id.uuid == UUID(uuid: view.active_view), page.views.first?.name == "View 1",
                  sidebar.pagesController.outline.numberOfRows == 3 else { return false }
            return true
        }
        guard awaitSidebar({ showsActiveView(project) }),
              project.sidebarSlot.sidebar.layersController.snapshot.layers.topLevelIDs.isEmpty else { return fail("새 프로젝트의 Pages·빈 Layers") }
        app.createProject(parentDirectory: directory.path, folderName: "Other Window")
        let otherPath = directory.appendingPathComponent("Other Window").path
        guard wait(), let other = app.projectWindow(forPath: otherPath),
              awaitSidebar({ showsActiveView(other) }), other.sidebarSlot.sidebar.pagesController.snapshot.activeViewID !=
              project.sidebarSlot.sidebar.pagesController.snapshot.activeViewID else { return fail("다른 창의 실제 View") }
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
        let firstOutline = project.sidebarSlot.sidebar.layersController.outline
        guard awaitSidebar({ layerRow(child, in: project) != nil }), let index = layerRow(child, in: project),
              firstOutline.selectedRow == index,
              let cell = firstOutline.view(atColumn: 0, row: index, makeIfNecessary: true) as? SidebarLayerCellView,
              cell.nameField.stringValue == "Frame", !cell.nameField.isEditable, !cell.lockButton.isEnabled,
              !cell.visibilityButton.isEnabled, !project.sidebarSlot.sidebar.filterAccessory.searchField.isEnabled,
              other.sidebarSlot.sidebar.layersController.snapshot.layers.rows.isEmpty else { return fail("추가된 실제 행·읽기 전용 편집·창 격리") }
        guard action(.undo), awaitSidebar({ layerRow(child, in: project) == nil }), canvas.frameHit(at: viewPoint(50, 50))?.id == root, session.info?.selection_count == 0,
              action(.redo), awaitSidebar({ layerRow(child, in: project) != nil }), canvas.frameHit(at: viewPoint(50, 50))?.id == child, session.info?.selection_count == 1,
              other.inspectorSource.snapshot.state == .noSelection else { return fail("Undo·Redo·창별 선택") }

        func field(_ axis: String, _ owner: ShellProjectWindow) -> NSTextField? {
            owner.inspectorSlot.inspector.control(identifier: "frame.size." + axis) as? NSTextField
        }
        func sizeIs(_ width: Double, _ height: Double, _ owner: ShellProjectWindow) -> Bool {
            awaitSidebar {
                guard let w = field("width", owner), let h = field("height", owner) else { return false }
                return w.doubleValue == width && h.doubleValue == height
            }
        }
        func enter(_ text: String, in field: NSTextField) -> Bool {
            field.stringValue = text
            return field.sendAction(field.action, to: field.target)
        }
        guard sizeIs(60, 60, project), let width = field("width", project), let height = field("height", project),
              let before = session.info, enter("28oops", in: width), enter("NaN", in: height), wait(),
              sizeIs(60, 60, project), session.info?.revision == before.revision,
              enter("78.5", in: width), enter("68", in: height), wait(), sizeIs(78.5, 68, project),
              session.info?.undo_depth == before.undo_depth + 2,
              action(.undo), sizeIs(78.5, 60, project), action(.undo), sizeIs(60, 60, project),
              action(.redo), sizeIs(78.5, 60, project), action(.redo), sizeIs(78.5, 68, project),
              action(.saveNow), session.info?.generation == initial.generation + 1 else { return fail("Inspector 입력·Undo·Redo·저장") }

        window.close()
        guard wait() else { return fail("닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let reopened = app.projectWindow(forPath: path), reopened !== project else { return fail("재열기") }
        reopened.window?.setContentSize(NSSize(width: 1300, height: 800))
        reopened.window?.contentView?.layoutSubtreeIfNeeded()
        let shown = reopened.canvasHost.canvasView
        guard awaitSidebar({ showsActiveView(reopened) && layerRow(child, in: reopened) != nil }),
              shown.frameHit(at: viewPoint(41, 31, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(99, 89, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(117, 97, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(120, 99, canvas: shown))?.id == root,
              reopened.selectNodes([child]), wait(), sizeIs(78.5, 68, reopened) else { return fail("재열기 후 자식 위치·크기") }
        reopened.gitSource.updateTrust(false)
        guard wait(), awaitSidebar({ field("width", reopened)?.isEnabled == false }),
              let restrictedSession = reopened.windowController.windowSession,
              let restricted = restrictedSession.info, let lockedWidth = field("width", reopened),
              !lockedWidth.isEnabled, !restrictedSession.isClosing else { return fail("읽기 전용 Inspector 필드") }
        let restrictedSource = reopened.inspectorSource
        restrictedSource.submit(.setValue(InspectorPropertyID("frame.size.width"), .number(12), phase: .commit),
                                basedOn: restrictedSource.snapshot.generation)
        guard wait(), restrictedSession.info?.revision == restricted.revision, sizeIs(78.5, 68, reopened)
        else { return fail("읽기 전용 Inspector 크기 보존") }
        reopened.gitSource.updateTrust(true)
        guard wait(), awaitSidebar({ field("width", reopened)?.isEnabled == true }) else { return fail("신뢰 후 Inspector 복구") }
        guard key(3, "f", canvas: shown), pointer(.leftMouseDown, 45, 36, canvas: shown),
              pointer(.leftMouseDragged, 62, 52, canvas: shown), pointer(.leftMouseUp, 62, 52, canvas: shown), wait(),
              let nested = shown.frameHit(at: viewPoint(46, 37, canvas: shown))?.id,
              nested != root, nested != child, shown.frameHit(at: viewPoint(61, 51, canvas: shown))?.id == nested,
              shown.frameHit(at: viewPoint(63, 53, canvas: shown))?.id == child else { return fail("이동된 부모의 local 좌표") }
        let layers = reopened.sidebarSlot.sidebar.layersController
        guard awaitSidebar({ layerRow(nested, in: reopened) != nil }),
              let nestedRow = layerRow(nested, in: reopened),
              (layers.outline.parent(forItem: layers.outline.item(atRow: nestedRow)) as? SidebarOutlineItem)?.id?.uuid == child
        else { return fail("새 중첩 Frame의 조상 펼침·자식 행") }

        // 실제 F 도구로 루트에 형제 64개를 추가합니다. 첫 자식은 마지막 페이지에 남습니다.
        var latest: UUID?
        for i in 0..<64 {
            let x = CGFloat(145 + (i % 8) * 20), y = CGFloat(145 + (i / 8) * 20)
            guard key(3, "f", canvas: shown), pointer(.leftMouseDown, x, y, canvas: shown),
                  pointer(.leftMouseDragged, x + 14, y + 14, canvas: shown),
                  pointer(.leftMouseUp, x + 14, y + 14, canvas: shown), wait(),
                  let inserted = shown.frameHit(at: viewPoint(x + 5, y + 5, canvas: shown))?.id,
                  inserted != root else { return fail("페이지 검사용 실제 Frame 생성") }
            latest = inserted
        }
        guard awaitSidebar({ layers.snapshot.layers.topLevelIDs.first?.uuid == latest && layers.snapshot.layers.moreParents.contains(nil) }),
              layers.snapshot.layers.topLevelIDs.count == 64, layerRow(child, in: reopened) == nil,
              let moreIndex = (0..<layers.outline.numberOfRows).first(where: {
                  (layers.outline.item(atRow: $0) as? SidebarOutlineItem)?.role == .more
              }) else { return fail("65개 형제 중 첫 페이지 64개") }
        layers.outline.scrollRowToVisible(moreIndex)
        guard let more = layers.outline.view(atColumn: 0, row: moreIndex, makeIfNecessary: true) as? NSButton else {
            return fail("다음 페이지 버튼")
        }
        more.performClick(nil)
        guard awaitSidebar({ layerRow(child, in: reopened) != nil && !layers.snapshot.layers.moreParents.contains(nil) }),
              layers.snapshot.layers.topLevelIDs.count == 65,
              let parentIndex = layerRow(child, in: reopened), let parentItem = layers.outline.item(atRow: parentIndex),
              other.sidebarSlot.sidebar.layersController.snapshot.layers.rows.isEmpty else { return fail("다음 페이지·다른 창 격리") }
        layers.outline.expandItem(parentItem)
        guard awaitSidebar({ layerRow(nested, in: reopened) != nil }) else { return fail("저장된 자식 disclosure") }
        guard let row = layerRow(nested, in: reopened), let beforeSelect = reopened.windowController.windowSession?.info else {
            return fail("중첩 행 선택 준비")
        }
        layers.outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        guard wait(), reopened.windowController.windowSession?.info?.selection_count == 1,
              reopened.windowController.windowSession?.info?.undo_depth == beforeSelect.undo_depth + 1,
              awaitSidebar({
                  guard let selected = layerRow(nested, in: reopened) else { return false }
                  return layers.snapshot.primarySelection?.uuid == nested &&
                      layers.outline.selectedRowIndexes.contains(selected) && layers.snapshot.layers.topLevelIDs.count == 65
              }),
              other.windowController.windowSession?.info?.selection_count == 0,
              other.sidebarSlot.sidebar.layersController.snapshot.selection.isEmpty else {
            return fail("Layers 선택 창 격리: undo=\(reopened.windowController.windowSession?.info?.undo_depth ?? 0)/\(beforeSelect.undo_depth + 1) top=\(layers.snapshot.layers.topLevelIDs.count) selectedRow=\(layers.outline.selectedRow) other=\(other.windowController.windowSession?.info?.selection_count ?? 0)")
        }
        print("frame-insert ok: F, restricted, cancel, undo/redo and reopen; Inspector size, invalid input, undo/redo, trust and reopen; live Layers, nested disclosure, 64+1 pagination and two windows")
        return true
    }
}
