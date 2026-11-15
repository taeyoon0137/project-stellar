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
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("frame-insert 실패: \(message)\n".utf8))
            return false
        }
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 15) }
        let showAlert = app.alertPresenter
        app.alertPresenter = { title, message, owner in
            // 숨긴 검사 창도 실제로 sheet를 붙일 수 있게 화면 밖에서 먼저 표시합니다.
            owner?.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
            owner?.orderFront(nil)
            showAlert(title, message, owner)
        }
        app.showWelcome()
        guard let welcome = app.welcomeController?.welcomeViewController else { return fail("시작 창") }
        welcome.renderNow()
        welcome.chooseNewProjectLocation = { _, _, completion in completion(URL(fileURLWithPath: path)) }
        welcome.newProjectButton.performClick(nil)
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
        func acknowledgeFirstCommit(_ owner: NSWindow) -> Bool {
            let expectsNotice = CommandLine.arguments.contains("--expect-missing-git-identity")
            if expectsNotice,
               !awaitSidebar({ owner.attachedSheet != nil }) { return fail("첫 커밋 안내 미표시") }
            guard let sheet = owner.attachedSheet else { return !expectsNotice }
            func contents(_ view: NSView) -> [NSView] {
                [view] + view.subviews.flatMap(contents)
            }
            guard let content = sheet.contentView else { return false }
            let controls = contents(content)
            let labels = controls.compactMap { ($0 as? NSTextField)?.stringValue }
            guard labels.contains(String(localized: "Project Created Without a First Commit")),
                  labels.contains(String(localized: "Set your Git user.name and user.email, then make the first commit in this project.")),
                  let ok = controls.compactMap({ $0 as? NSButton }).first(where: { $0.title == String(localized: "OK") }) else {
                return fail("첫 커밋 안내 내용·버튼: \(labels)")
            }
            ok.performClick(nil)
            guard awaitSidebar({ owner.attachedSheet == nil }) else { return fail("첫 커밋 안내 확인 뒤 sheet 유지") }
            print("new-project ok: first-commit notice explains missing Git identity and OK dismisses it")
            return true
        }
        guard initial.generation == 1, initial.has_document == 1, !project.gitSource.trusted else { return fail("초기 제한 모드 문서") }
        guard acknowledgeFirstCommit(window) else { return false }
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
        guard wait(), let other = app.projectWindow(forPath: otherPath), let otherWindow = other.window, acknowledgeFirstCommit(otherWindow),
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
            if command == .saveNow {
                let menu = NSMenu()
                menu.addItem(MainMenuBuilder.makeItem(for: .saveNow, target: project.router))
                guard let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command,
                    timestamp: 1, windowNumber: window.windowNumber, context: nil, characters: "s",
                    charactersIgnoringModifiers: "s", isARepeat: false, keyCode: 1) else { return false }
                return menu.performKeyEquivalent(with: event) && wait()
            }
            project.router.performShellCommand(MainMenuBuilder.makeItem(for: command, target: nil))
            return wait()
        }
        let design = URL(fileURLWithPath: path).appendingPathComponent("design")
        func savedDesign() throws -> [URL: Data] {
            let files = try FileManager.default.subpathsOfDirectory(atPath: design.path)
            var bytes: [URL: Data] = [:]
            for entry in files {
                let url = design.appendingPathComponent(entry)
                if try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true {
                    bytes[url] = try Data(contentsOf: url)
                }
            }
            return bytes
        }
        guard let original = try? savedDesign(), !original.isEmpty,
              let root = canvas.frameHit(at: viewPoint(50, 50))?.id,
              key(3, "f"), project.toolsModel.snapshot.activeTool == .frame else { return fail("F 도구와 원본 문서") }
        func unchanged() -> Bool {
            guard let info = session.info else { return false }
            return info.revision == initial.revision && info.undo_depth == initial.undo_depth &&
                info.has_unsaved_changes == initial.has_unsaved_changes && canvas.insertionOutline == nil &&
                canvas.frameHit(at: viewPoint(50, 50))?.id == root && (try? savedDesign()) == original
        }
        window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        window.orderFront(nil)
        guard !project.gitSource.trusted, window.attachedSheet == nil, stroke(),
              ShellProjectTrustCheck.respondToTrustSheet(in: window, approving: false), wait(),
              !project.gitSource.trusted, unchanged() else { return fail("제한 모드 F 드래그 승인 취소·원본 보존") }
        guard stroke(), ShellProjectTrustCheck.respondToTrustSheet(in: window, approving: true), wait(),
              project.gitSource.trusted, unchanged(), project.toolsModel.snapshot.activeTool == .frame
        else { return fail("F 드래그 신뢰 승인 직후 입력 재생 없이 원본 보존") }
        if CommandLine.arguments.contains("--expect-missing-git-identity") {
            let panel = project.panelsSlot.gitPanel
            panel.loadViewIfNeeded()
            panel.dataSource.send(.refreshStatus)
            guard awaitSidebar({ panel.status?.availability == .ready && !panel.isBusy
                && ((panel.status?.changes.generation ?? 0) > 0 || panel.status?.semanticStatusError != nil) }),
                let current = panel.status, current.changes.generation > 0, current.semanticStatusError == nil,
                current.head == .unborn(branch: "main"), current.stagedChangeCount > 0,
                panel.branchLabel.stringValue == GitPanelStrings.head(.unborn(branch: "main")),
                panel.changesOverlay.titleField.stringValue == String(localized: "No Commits Yet")
            else {
                return fail("첫 커밋 전 Git: title=\(panel.changesOverlay.titleField.stringValue), generation=\(panel.status?.changes.generation ?? 0), issue=\(panel.status?.semanticStatusError ?? "")")
            }
            print("unborn Git panel: \(panel.branchLabel.stringValue), \(panel.summaryLabel.stringValue), \(panel.changesOverlay.titleField.stringValue)")
        }
        guard pointer(.leftMouseDown, 40, 30), pointer(.leftMouseDragged, 90, 70),
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
              cell.nameField.stringValue == "Frame", cell.nameField.isEditable, !cell.lockButton.isEnabled,
              !cell.visibilityButton.isEnabled, !project.sidebarSlot.sidebar.filterAccessory.searchField.isEnabled,
              other.sidebarSlot.sidebar.layersController.snapshot.layers.rows.isEmpty else { return fail("추가된 실제 행·이름 편집·창 격리") }
        guard action(.undo), awaitSidebar({ layerRow(child, in: project) == nil }), canvas.frameHit(at: viewPoint(50, 50))?.id == root, session.info?.selection_count == 0,
              action(.redo), awaitSidebar({ layerRow(child, in: project) != nil }), canvas.frameHit(at: viewPoint(50, 50))?.id == child, session.info?.selection_count == 1,
              other.inspectorSource.snapshot.state == .noSelection,
              let beforeRename = session.info,
              project.sidebarSlot.sidebar.layersController.beginRename(SidebarStableID(child)),
              let editor = window.firstResponder as? NSTextView else { return fail("Undo·Redo·이름 편집기") }
        editor.selectAll(nil)
        editor.insertText("설정 카드", replacementRange: NSRange(location: NSNotFound, length: 0))
        func layerName() -> String? { project.sidebarSlot.sidebar.layersController.snapshot.layers.rows[SidebarStableID(child)]?.name }
        guard window.makeFirstResponder(firstOutline), wait(), awaitSidebar({ layerName() == "설정 카드" }),
              session.info?.undo_depth == beforeRename.undo_depth + 1,
              action(.undo), awaitSidebar({ layerName() == "Frame" }),
              action(.redo), awaitSidebar({ layerName() == "설정 카드" }) else { return fail("레이어 이름 변경·Undo·Redo") }

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
              other.inspectorSource.snapshot.state == .noSelection else { return fail("Inspector 입력·Undo·Redo") }

        func radiusField(_ name: String, _ owner: ShellProjectWindow) -> NSTextField? {
            owner.inspectorSlot.inspector.control(identifier: "frame.cornerRadius." + name) as? NSTextField
        }
        func radiiAre(_ expected: [Double], _ owner: ShellProjectWindow) -> Bool {
            awaitSidebar {
                zip(["topLeft", "topRight", "bottomRight", "bottomLeft"], expected).allSatisfy {
                    radiusField($0.0, owner)?.doubleValue == $0.1
                }
            }
        }
        guard radiiAre([0, 0, 0, 0], project), let beforeCorners = session.info else { return fail("Frame 반경 입력 준비") }
        for (index, name) in ["topLeft", "topRight", "bottomRight", "bottomLeft"].enumerated() {
            var expected = [Double](repeating: 0, count: 4)
            expected[index] = 13
            guard let control = radiusField(name, project), enter("13", in: control), wait(),
                  radiiAre(expected, project), session.info?.undo_depth == beforeCorners.undo_depth + 1,
                  action(.undo), radiiAre([0, 0, 0, 0], project) else { return fail("개별 모서리 입력·Undo " + name) }
        }
        guard let topRight = radiusField("topRight", project), let rawBase = session.info,
              enter("NaN", in: topRight), wait(), radiiAre([0, 0, 0, 0], project),
              session.info?.revision == rawBase.revision,
              enter("128", in: topRight), wait(), radiiAre([0, 128, 0, 0], project),
              session.info?.undo_depth == beforeCorners.undo_depth + 1,
              action(.undo), radiiAre([0, 0, 0, 0], project),
              action(.redo), radiiAre([0, 128, 0, 0], project),
              canvas.frameHit(at: viewPoint(116, 32))?.id == root,
              canvas.frameHit(at: viewPoint(42, 32))?.id == child
        else { return fail("범위보다 큰 우상 원본 반경·캔버스 hit·Undo·Redo") }

        guard let oldFrame = canvas.selectedInspectorFrame, let beforeReselect = session.info,
              project.selectNodes([root]), wait(), project.selectNodes([child]), wait(),
              radiiAre([0, 128, 0, 0], project), let reselected = canvas.selectedInspectorFrame,
              let afterReselect = session.info, reselected.revision == oldFrame.revision,
              reselected.historySequence != oldFrame.historySequence else { return fail("A→B→A 선택 준비") }
        var staleFailed = false
        guard case .success = pump.editFrame(oldFrame, change: .corner(.topRight, 17), completion: {
            if case .requestFailed = $0 { staleFailed = true }
        }), wait(), staleFailed, radiiAre([0, 128, 0, 0], project),
              session.info?.revision == beforeReselect.revision, session.info?.undo_depth == afterReselect.undo_depth
        else { return fail("A→B→A 뒤 오래된 반경 요청 거부") }

        let white = InspectorColor(space: .srgb, red: 1, green: 1, blue: 1, alpha: 1)
        let p3 = InspectorColor(space: .displayP3, red: 0.75, green: 0.125, blue: 0.625, alpha: 0.375)
        func centerPixel(_ canvas: CanvasView, in window: NSWindow) -> [Int]? {
            guard let image = ShellVisibleFramesCheck.bitmap(canvas, in: window) else { return nil }
            let point = viewPoint(70, 60, canvas: canvas)
            let x = Int((point.x * CGFloat(image.pixelsWide) / canvas.bounds.width).rounded(.down))
            let y = Int((point.y * CGFloat(image.pixelsHigh) / canvas.bounds.height).rounded(.down))
            guard x >= 0, x < image.pixelsWide, y >= 0, y < image.pixelsHigh else { return nil }
            var sample = [Int](repeating: 0, count: image.samplesPerPixel)
            image.getPixel(&sample, atX: x, y: y)
            return Array(sample.prefix(3))
        }
        func fillIs(_ expected: InspectorColor?, in owner: ShellProjectWindow) -> Bool {
            awaitSidebar {
                guard let selected = owner.canvasHost.canvasView.selectedInspectorFrame,
                      let enabled = owner.inspectorSlot.inspector.control(identifier: "frame.background.enabled") as? NSSwitch,
                      let well = owner.inspectorSlot.inspector.control(identifier: "frame.background.color") as? NSColorWell,
                      selected.background == expected, enabled.state == (expected == nil ? .off : .on),
                      well.isEnabled == (expected != nil && owner.gitSource.trusted && selected.editable)
                else { return false }
                return expected.map { InspectorColor(well.color) == $0 } ?? !well.isEnabled
            }
        }
        guard fillIs(white, in: project),
              let enabled = project.inspectorSlot.inspector.control(identifier: "frame.background.enabled") as? NSSwitch,
              let well = project.inspectorSlot.inspector.control(identifier: "frame.background.color") as? NSColorWell,
              let whitePixel = centerPixel(canvas, in: window), let preFill = session.info else { return fail("실제 Frame 배경 컨트롤") }
        enabled.state = .off
        guard enabled.sendAction(enabled.action, to: enabled.target), wait(), fillIs(nil, in: project),
              action(.undo), fillIs(white, in: project), action(.redo), fillIs(nil, in: project)
        else { return fail("배경 없음·Undo·Redo") }
        enabled.state = .on
        guard enabled.sendAction(enabled.action, to: enabled.target), wait(), fillIs(white, in: project)
        else { return fail("채우기 없는 Frame에서 배경 추가") }
        well.color = p3.nsColor
        guard well.sendAction(well.action, to: well.target), fillIs(p3, in: project), wait() else {
            return fail("Display P3 배경 입력: well=\(well.color) scene=\(String(describing: canvas.selectedInspectorFrame?.background)) depth=\(session.info?.undo_depth ?? 0)")
        }
        guard let painted = centerPixel(canvas, in: window), painted != whitePixel, painted[0] > painted[1],
              painted[2] > painted[1], session.info?.undo_depth == preFill.undo_depth + 3,
              action(.undo), fillIs(white, in: project), action(.redo), fillIs(p3, in: project),
              action(.saveNow), session.info?.generation == initial.generation + 1 else {
            return fail("Display P3·알파 편집·Undo·Redo·저장")
        }
        window.close()
        guard wait() else { return fail("닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let reopened = app.projectWindow(forPath: path), reopened !== project else { return fail("재열기") }
        reopened.window?.setContentSize(NSSize(width: 1300, height: 800))
        reopened.window?.contentView?.layoutSubtreeIfNeeded()
        let shown = reopened.canvasHost.canvasView
        guard awaitSidebar({ showsActiveView(reopened) && layerRow(child, in: reopened) != nil }),
              reopened.sidebarSlot.sidebar.layersController.snapshot.layers.rows[SidebarStableID(child)]?.name == "설정 카드",
              shown.frameHit(at: viewPoint(41, 31, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(99, 89, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(117, 97, canvas: shown))?.id == child,
              shown.frameHit(at: viewPoint(120, 99, canvas: shown))?.id == root,
              reopened.selectNodes([child]), wait(), sizeIs(78.5, 68, reopened), fillIs(p3, in: reopened),
              radiiAre([0, 128, 0, 0], reopened),
              shown.frameHit(at: viewPoint(116, 32, canvas: shown))?.id == root,
              let reopenedWindow = reopened.window, centerPixel(shown, in: reopenedWindow) == painted else { return fail("재열기 후 자식 위치·크기") }
        reopened.gitSource.updateTrust(false)
        guard wait(), awaitSidebar({ field("width", reopened)?.isEnabled == false }),
              let restrictedSession = reopened.windowController.windowSession,
              let restricted = restrictedSession.info, let lockedWidth = field("width", reopened),
              !lockedWidth.isEnabled, radiusField("topRight", reopened)?.isEnabled == false, !restrictedSession.isClosing else { return fail("읽기 전용 Inspector 필드") }
        let restrictedSource = reopened.inspectorSource
        restrictedSource.submit(.setValue(InspectorPropertyCatalog.FrameCorner.topRight.field.id, .number(9), phase: .commit),
                                basedOn: restrictedSource.snapshot.generation)
        restrictedSource.submit(.setValue(InspectorPropertyID("frame.size.width"), .number(12), phase: .commit),
                                basedOn: restrictedSource.snapshot.generation)
        restrictedSource.submit(.setValue(InspectorPropertyID("frame.background.enabled"), .bool(false), phase: .commit),
                                basedOn: restrictedSource.snapshot.generation)
        guard wait(), restrictedSession.info?.revision == restricted.revision, sizeIs(78.5, 68, reopened), fillIs(p3, in: reopened),
              (reopened.inspectorSlot.inspector.control(identifier: "frame.background.color") as? NSColorWell)?.isEnabled == false
        else { return fail("읽기 전용 Inspector 크기 보존") }
        guard radiiAre([0, 128, 0, 0], reopened) else { return fail("읽기 전용 반경 보존") }
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
        func loadShapeRow(_ owner: ShellProjectWindow, _ id: UUID) -> SidebarLayerRow? {
            let sidebar = owner.sidebarSource, parentID = SidebarStableID(child), shapeID = SidebarStableID(id)
            guard awaitSidebar({ sidebar.snapshot.layers.rows[parentID] != nil || sidebar.snapshot.layers.moreParents.contains(nil) })
            else { return nil }
            if sidebar.snapshot.layers.rows[parentID] == nil { sidebar.perform(.loadMore(nil)) }
            guard awaitSidebar({ sidebar.snapshot.layers.rows[parentID] != nil }) else { return nil }
            sidebar.perform(.loadChildren(parentID))
            guard awaitSidebar({ sidebar.snapshot.layers.rows[shapeID] != nil }) else { return nil }
            return sidebar.snapshot.layers.rows[shapeID]
        }
        guard let parent = shown.frameHit(at: viewPoint(116, 88, canvas: shown)), parent.id == child,
              key(15, "r", canvas: shown), reopened.toolsModel.snapshot.activeTool == .rectangle,
              pointer(.leftMouseDown, 104, 84, canvas: shown), pointer(.leftMouseDragged, 135, 106, canvas: shown),
              shown.insertionOutline != nil, pointer(.leftMouseUp, 135, 106, canvas: shown), wait(),
              let paintedShape = shown.frameHit(at: viewPoint(112, 90, canvas: shown)), paintedShape.isRectangle,
              shown.frameHit(at: viewPoint(112, 90, canvas: shown), onlyFrames: true)?.id == child,
              shown.frameHit(at: viewPoint(128, 92, canvas: shown))?.id != paintedShape.id,
              reopened.inspectorSource.snapshot.state == .noSelection,
              let shapeRow = loadShapeRow(reopened, paintedShape.id), shapeRow.kind == .shape, shapeRow.shapeKind == .rectangle else {
            let center = shown.frameHit(at: viewPoint(112, 90, canvas: shown))
            let parentHit = shown.frameHit(at: viewPoint(112, 90, canvas: shown), onlyFrames: true)
            let found = center.flatMap { layers.snapshot.layers.rows[SidebarStableID($0.id)] }
            return fail("R 도구·부모 clip·실제 Shape 레이어: tool=\(reopened.toolsModel.snapshot.activeTool) selected=\(center?.isRectangle == true) parent=\(parentHit?.id == child) kind=\(String(describing: found?.kind))/\(String(describing: found?.shapeKind))")
        }
        func blackShape(_ owner: ShellProjectWindow, moved: Bool = false) -> Bool {
            guard let window = owner.window else { return false }
            let view = owner.canvasHost.canvasView
            guard let image = ShellVisibleFramesCheck.bitmap(view, in: window) else { return false }
            func sample(_ x: CGFloat, _ y: CGFloat) -> [Int] {
                let point = viewPoint(x, y, canvas: view)
                let col = Int(point.x * CGFloat(image.pixelsWide) / view.bounds.width)
                let row = Int(point.y * CGFloat(image.pixelsHigh) / view.bounds.height)
                guard col >= 0, col < image.pixelsWide, row >= 0, row < image.pixelsHigh else { return [] }
                var pixel = [Int](repeating: 0, count: image.samplesPerPixel)
                image.getPixel(&pixel, atX: col, y: row)
                return Array(pixel.prefix(3))
            }
            let interior = moved ? sample(95, 75) : sample(112, 90)
            return interior == [0, 0, 0] && sample(128, 92) == [255, 255, 255]

        }
        guard blackShape(reopened), reopened.performSessionCommand(.undo), wait(),
              shown.frameHit(at: viewPoint(112, 90, canvas: shown))?.id == child,
              reopened.performSessionCommand(.redo), wait(), blackShape(reopened),
              reopened.performSessionCommand(.saveNow), wait(), let shapeWindow = reopened.window else {
            return fail("사각형 픽셀·실행취소·다시실행·저장")
        }
        shapeWindow.close()
        guard wait() else { return fail("사각형 프로젝트 창 닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let again = app.projectWindow(forPath: path), again !== reopened else { return fail("사각형 재열기") }
        again.window?.setContentSize(NSSize(width: 1300, height: 800))
        again.window?.contentView?.layoutSubtreeIfNeeded()
        let visibleShape = again.canvasHost.canvasView
        guard blackShape(again), visibleShape.frameHit(at: viewPoint(112, 90, canvas: visibleShape))?.id == paintedShape.id,
              loadShapeRow(again, paintedShape.id)?.kind == .shape,
              pointer(.leftMouseDown, 112, 90, canvas: visibleShape), pointer(.leftMouseUp, 112, 90, canvas: visibleShape),
              wait(), again.windowController.windowSession?.info?.selection_count == 1,
              awaitSidebar({ again.sidebarSource.snapshot.primarySelection?.uuid == paintedShape.id }) else {
            return fail("저장된 Shape 픽셀·안정 ID·클릭 선택")
        }
        guard let movedSession = again.windowController.windowSession, let beforeMove = movedSession.info,
              key(9, "v", canvas: visibleShape), pointer(.leftMouseDown, 112, 90, canvas: visibleShape),
              pointer(.leftMouseDragged, 92, 75, canvas: visibleShape), movedSession.info?.revision == beforeMove.revision,
              pointer(.leftMouseUp, 92, 75, canvas: visibleShape), wait(),
              movedSession.info?.undo_depth == beforeMove.undo_depth + 1,
              visibleShape.frameHit(at: viewPoint(95, 75, canvas: visibleShape))?.id == paintedShape.id,
              visibleShape.frameHit(at: viewPoint(117, 90, canvas: visibleShape))?.id == child,
              blackShape(again, moved: true), again.performSessionCommand(.undo), wait(),
              visibleShape.frameHit(at: viewPoint(95, 75, canvas: visibleShape))?.id == child, blackShape(again),
              again.performSessionCommand(.redo), wait(), blackShape(again, moved: true),
              again.performSessionCommand(.saveNow), wait(), let savedShapeWindow = again.window else {
            return fail("V Rectangle 이동·Undo/Redo·저장")
        }
        savedShapeWindow.close()
        guard wait() else { return fail("이동 후 닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let restored = app.projectWindow(forPath: path), restored !== again else { return fail("이동 후 재열기") }
        restored.window?.setContentSize(NSSize(width: 1300, height: 800))
        restored.window?.contentView?.layoutSubtreeIfNeeded()
        let restoredCanvas = restored.canvasHost.canvasView
        guard blackShape(restored, moved: true),
              restoredCanvas.frameHit(at: viewPoint(95, 75, canvas: restoredCanvas))?.id == paintedShape.id,
              restoredCanvas.frameHit(at: viewPoint(117, 90, canvas: restoredCanvas))?.id == child,
              loadShapeRow(restored, paintedShape.id)?.shapeKind == .rectangle else {
            return fail("Rectangle 이동 위치·Shape 종류·픽셀 재열기")
        }
        // 메뉴와 마우스로 만든 도형의 내부와, 외접 사각형 안이지만 경로 밖인 점을 함께 확인합니다.
        func shapeAt(_ point: CGPoint, in view: CanvasView) -> UUID? {
            view.frameHit(at: viewPoint(point.x, point.y, canvas: view))?.id
        }
        let shapes: [(tool: CanvasTool, start: CGPoint, end: CGPoint, inside: CGPoint, outside: CGPoint)] = [
            (.ellipse, CGPoint(x: 180, y: 48), CGPoint(x: 220, y: 72), CGPoint(x: 200, y: 60), CGPoint(x: 181, y: 49)),
            (.line, CGPoint(x: 284, y: 72), CGPoint(x: 232, y: 48), CGPoint(x: 258, y: 60), CGPoint(x: 240, y: 70)),
            (.polygon, CGPoint(x: 244, y: 92), CGPoint(x: 284, y: 120), CGPoint(x: 264, y: 106), CGPoint(x: 245, y: 93)),
        ]
        var drawn: [UUID] = []
        for (tool, start, end, inside, outside) in shapes {
            let menu = NSMenu()
            menu.addItem(MainMenuBuilder.makeItem(for: tool.command, target: restored.router))
            menu.performActionForItem(at: 0)
            guard restored.toolsModel.snapshot.activeTool == tool,
                  let before = restored.windowController.windowSession?.info,
                  pointer(.leftMouseDown, start.x, start.y, canvas: restoredCanvas),
                  pointer(.leftMouseDragged, end.x, end.y, canvas: restoredCanvas),
                  restoredCanvas.insertionOutline != nil,
                  restored.windowController.windowSession?.info?.revision == before.revision,
                  pointer(.leftMouseUp, end.x, end.y, canvas: restoredCanvas), wait(),
                  let id = shapeAt(inside, in: restoredCanvas), id != root,
                  shapeAt(outside, in: restoredCanvas) == root,
                  restored.windowController.windowSession?.info?.undo_depth == before.undo_depth + 1,
                  restored.toolsModel.snapshot.activeTool == .move,
                  restored.performSessionCommand(.undo), wait(), shapeAt(inside, in: restoredCanvas) == root,
                  restored.performSessionCommand(.redo), wait(), shapeAt(inside, in: restoredCanvas) == id else {
                return fail("\(tool.label) 생성·실제 경로 hit·Undo/Redo")
            }
            drawn.append(id)
        }
        guard restored.performSessionCommand(.saveNow), wait(), let editedWindow = restored.window else {
            return fail("도형 세 개 저장")
        }
        editedWindow.close()
        guard wait() else { return fail("도형 창 닫기") }
        app.openProject(atPath: path, origin: .application)
        guard wait(), let final = app.projectWindow(forPath: path), final !== restored else { return fail("도형 재열기") }
        final.window?.setContentSize(NSSize(width: 1300, height: 800))
        final.window?.contentView?.layoutSubtreeIfNeeded()
        for (shape, id) in zip(shapes, drawn) {
            let view = final.canvasHost.canvasView
            guard shapeAt(shape.inside, in: view) == id, shapeAt(shape.outside, in: view) == root else {
                return fail("재열기 뒤 \(shape.tool.label) ID·경로 불일치")
            }
        }
        print("shape tools ok: Ellipse, reverse Line and Polygon from menu to path hit, single Undo, Redo, Save and reopen")
        print("frame-insert ok: F, restricted, cancel, undo/redo and reopen; Inspector size, four independent raw corners and stale A-B-A guard, optional fill, P3/alpha, invalid input, undo/redo, trust and reopen; R Rectangle Shape, V move, clip, pixels and reopen; live Layers, inline rename with undo/redo and reopen, nested disclosure, 64+1 pagination and two windows")
        return true
    }
}
