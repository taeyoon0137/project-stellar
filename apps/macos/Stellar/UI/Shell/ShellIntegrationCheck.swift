import AppKit

/// 앱 셸 통합 검사(`--shell-integration-check`).
///
/// 실제 앱과 같은 구성(`ShellAppCoordinator`, 실제 코어 runtime, 영역 view controller, `MainMenuBuilder` 메뉴)을 화면에
/// 띄우지 않고 만들어 다음을 확인합니다.
/// - 메인 메뉴: 앱 수준 명령 target·활성 상태, 셸 프레임 항목 설치, key equivalent 중복 없음
/// - 시작 창: 열린 프로젝트가 없으면 표시, 프로젝트를 열면 닫힘, 마지막 창을 닫으면 다시 표시
/// - Open Project: 코어 `stellar_project_open` → 완료 event → 프로젝트 창(제목, 제한 모드 부제목, ready)
/// - 프로젝트 창 구성: 캔버스 호스트(runtime handle), 사이드바·인스펙터·Git·에이전트 영역, 하단 도구 막대(캔버스 도구)·줌 툴바 조합,
///   응답자 체인 router, 도구 선택·단일 키 단축키, 코어 경로 없는 명령 비활성
/// - 같은 폴더 재열기 시 새 창 없음, 창 복원 상태 왕복, 없어진 폴더 복원의 복구 동작, 창 닫기 시 코어 session 닫기
/// - 열기 중 창 닫기 시 늦게 연 session 닫기, Locate Folder…로 여는 중인 폴더를 골라도 같은 폴더 창 하나
enum ShellIntegrationCheck {
    static func run() -> Bool {
        MainActor.assumeIsolated {
            ShellIntegrationCheckRunner().run()
        }
    }
}

@MainActor
private final class ShellIntegrationCheckRunner {
    private var failures: [String] = []
    private var checks = 0
    private let fixtureRoot = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        .appendingPathComponent("StellarShellIntegrationCheck-\(UUID().uuidString)", isDirectory: true)
    private var alerts: [(title: String, message: String)] = []
    /// `folderChooser`가 돌려줄 다음 경로.
    private var nextChosenFolder: String?

    static let projectName = "Integration Fixture"

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    func run() -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        defer { try? FileManager.default.removeItem(at: fixtureRoot) }

        let host = RuntimeHost()
        guard host.created else {
            FileHandle.standardError.write(Data("shell-integration 실패: runtime create 실패\n".utf8))
            return false
        }
        defer { host.shutdown() }

        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarShellIntegrationCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        // 앱의 실제 창 기록은 검사 전후로 그대로여야 합니다.
        let standard = ShellWindowConfiguration.standard
        let appRecordKeys = [
            "NSWindow Frame \(standard.autosaveName)",
            "NSSplitView Subview Frames \(standard.mainSplitAutosaveName)",
            "NSToolbar Configuration \(standard.toolbarIdentifier)",
        ]
        func appRecords() -> [NSObject?] {
            appRecordKeys.map { UserDefaults.standard.object(forKey: $0) as? NSObject }
        }
        let recordsBefore = appRecords()
        coordinator.alertPresenter = { [weak self] title, message, _ in
            self?.alerts.append((title, message))
        }
        coordinator.folderChooser = { [weak self] _, completion in
            completion(self?.nextChosenFolder)
        }

        let previousMenu = NSApp.mainMenu
        let mainMenu = coordinator.installMainMenu(on: NSApp)
        defer { NSApp.mainMenu = previousMenu }

        checkMainMenu(mainMenu, coordinator: coordinator)
        autoreleasepool {
            checkProjectLifecycle(coordinator, host: host)
        }
        // split view는 상태를 다음 런 루프에서 기록하므로 한 번 돌린 뒤 비교합니다.
        RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        expect(appRecords() == recordsBefore, "검사가 앱의 실제 창 autosave 기록을 바꿈")

        if failures.isEmpty {
            print("shell-integration ok: \(checks) checks (menu, welcome, create and open via core, window composition, tools, restore, close)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("shell-integration 실패: \(failure)\n".utf8))
        }
        return false
    }

    // MARK: - 메뉴

    private func item(_ command: ShellCommand, in menu: NSMenu) -> NSMenuItem? {
        for entry in menu.items {
            if let identifier = entry.identifier, ShellCommand(menuItemIdentifier: identifier) == command, !entry.isHidden {
                return entry
            }
            if let submenu = entry.submenu, let found = item(command, in: submenu) {
                return found
            }
        }
        return nil
    }

    private func items(withAction action: Selector, in menu: NSMenu) -> [NSMenuItem] {
        menu.items.flatMap { entry -> [NSMenuItem] in
            (entry.action == action ? [entry] : []) + (entry.submenu.map { items(withAction: action, in: $0) } ?? [])
        }
    }

    private func checkMainMenu(_ mainMenu: MainMenuBuilder.MainMenu, coordinator: ShellAppCoordinator) {
        let menu = mainMenu.menu
        expect(NSApp.mainMenu === menu, "메인 메뉴가 앱에 설치되지 않음")
        let router = coordinator.applicationRouter
        for (command, enabled) in [
            (ShellCommand.openProject, true), (.settings, true), (.newProject, true), (.cloneProject, true),
        ] {
            guard let entry = item(command, in: menu) else {
                expect(false, "\(command.rawValue) 메뉴 항목 없음")
                continue
            }
            expect(entry.target === router, "\(command.rawValue) target이 앱 수준 router가 아님")
            expect(router.validateMenuItem(entry) == enabled, "\(command.rawValue) 활성 상태가 \(enabled)가 아님")
        }

        // 셸 프레임 항목(가이드 6.1): Finder에서 보기. 캔버스 아래 보조 패널과 그 토글은 없습니다(UX 명세 3.2).
        let view = ShellMainMenuInstaller.submenu(titled: "View", in: menu)
        expect(view?.items.contains { $0.title.hasSuffix("Panels") } == false, "View 메뉴에 보조 패널 토글이 남음")
        let actions = items(withAction: #selector(ShellWindowController.performFrameAction(_:)), in: menu)
        let file = ShellMainMenuInstaller.submenu(titled: "File", in: menu)
        for id in [ShellProjectCommandID.revealInFinder] {
            let matches = actions.filter { ($0.representedObject as? String) == id.rawValue }
            expect(file != nil && matches.count == 1 && matches[0].menu === file && matches[0].target == nil,
                   "File 메뉴의 활성 창 명령 설치 불일치: \(id.rawValue)")
        }
        let trustIDs = [ShellProjectCommandID.trustProject.rawValue, ShellProjectCommandID.revokeTrust.rawValue]
        let trustItems = actions.filter { ($0.representedObject as? [String]) == trustIDs }
        expect(file != nil && trustItems.count == 1 && trustItems[0].menu === file && trustItems[0].target == nil,
               "File 메뉴에 신뢰 부여·철회 항목이 없음")
        expect(view?.items.contains { $0.action == NSSelectorFromString("toggleToolbarShown:") } == false,
               "View 메뉴에 툴바 숨김 항목이 남음")

        // key equivalent는 한 명령에만 둡니다(⌘[·⌘]는 Object 메뉴 소유).
        var seen: [String: String] = [:]
        var duplicates: [String] = []
        func walk(_ menu: NSMenu) {
            for entry in menu.items {
                if !entry.keyEquivalent.isEmpty {
                    let key = "\(entry.keyEquivalentModifierMask.rawValue)-\(entry.keyEquivalent)"
                    if let other = seen[key], other != entry.title { duplicates.append("\(other)/\(entry.title)") }
                    seen[key] = entry.title
                }
                if let submenu = entry.submenu { walk(submenu) }
            }
        }
        walk(menu)
        expect(duplicates.isEmpty, "메인 메뉴 key equivalent 중복: \(duplicates)")
    }

    // MARK: - 프로젝트 창

    private func makeFixture(_ name: String, manifest: String?, git: Bool = true) -> String {
        let folder = fixtureRoot.appendingPathComponent(name, isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        if git {
            try? FileManager.default.createDirectory(
                at: folder.appendingPathComponent(".git", isDirectory: true), withIntermediateDirectories: true
            )
        }
        if let manifest {
            try? manifest.write(to: folder.appendingPathComponent("stellar.yaml"), atomically: true, encoding: .utf8)
        }
        return folder.path
    }

    static func validManifest(name: String) -> String {
        "schema_version: \"1\"\nproject:\n  id: \"0195c530-3acd-7bc1-8414-c213f61cb965\"\n  name: \(name)\n"
            + "target_profiles: []\ndependencies: {}\n"
    }

    private func layout(_ window: NSWindow?) {
        window?.contentView?.layoutSubtreeIfNeeded()
        window?.layoutIfNeeded()
    }

    private func projectCount(_ host: RuntimeHost) -> UInt64 {
        CoreRuntime.state(handle: host.handle)?.project_count ?? .max
    }

    private func checkProjectLifecycle(_ coordinator: ShellAppCoordinator, host: RuntimeHost) {
        guard let pump = coordinator.pump else {
            expect(false, "runtime event 펌프 없음")
            return
        }
        // 시작 창: 열린 프로젝트가 없으면 표시합니다.
        coordinator.showWelcomeIfNeeded()
        expect(coordinator.welcomeController != nil, "열린 프로젝트가 없는데 시작 창이 없음")

        // Open Project(시작 창) → 코어 열기 → 프로젝트 창.
        let projectPath = makeFixture("Project", manifest: Self.validManifest(name: Self.projectName))
        coordinator.openProject(atPath: projectPath, origin: .welcome)
        expect(coordinator.welcomeData.currentSnapshot().activity?.kind == .opening, "열기 중 시작 창 진행 표시 없음")
        expect(pump.isPolling && pump.pendingRequestCount == 1, "열기 요청 대기 중인데 event 펌프가 꺼져 있음")
        coordinator.openProject(atPath: projectPath, origin: .welcome)
        expect(pump.pendingRequestCount == 1, "열기 중인 같은 폴더를 다시 요청함")
        expect(pump.waitForPendingRequests(timeout: 10), "프로젝트 열기 완료 event가 오지 않음")
        expect(!pump.isPolling, "대기 요청이 없는데 event 펌프가 계속 켜져 있음")
        expect(coordinator.welcomeData.currentSnapshot().activity == nil, "열기 후 진행 표시가 남음")
        expect(coordinator.projectWindows.count == 1, "프로젝트 창 수가 1이 아님: \(coordinator.projectWindows.count)")
        expect(coordinator.welcomeController == nil, "프로젝트를 열었는데 시작 창이 남음")
        expect(projectCount(host) == 1, "코어 프로젝트 session 수가 1이 아님")
        guard let projectWindow = coordinator.projectWindows.first, let window = projectWindow.window else {
            expect(false, "프로젝트 창 없음")
            return
        }
        projectWindow.windowController.splitController.prefersReducedMotion = { true }
        window.setContentSize(ShellFrameMetrics.initialContentSize)
        projectWindow.windowController.flushPendingSnapshot()
        layout(window)

        checkComposition(projectWindow, window: window, host: host)
        checkTools(projectWindow, window: window)

        // 같은 폴더를 다시 열면 새 창·새 session 없이 기존 창을 씁니다(가이드 2.5).
        coordinator.openProject(atPath: projectPath + "/", origin: .application)
        expect(pump.pendingRequestCount == 0 && coordinator.projectWindows.count == 1, "같은 폴더를 다시 열어 새 창을 만듦")

        // 창 복원 상태: 프로젝트 경로를 secure coding으로 저장하고, 같은 폴더가 열려 있으면 기존 창을 돌려줍니다.
        let state = encodeRestorableState(projectWindow)
        expect(state != nil, "창 복원 상태 인코딩 실패")
        if let state {
            expect(coordinator.restoreProjectWindow(from: state) === window, "열린 폴더 복원이 기존 창을 돌려주지 않음")
        }

        // 창 닫기 → 코어 session 닫기 → 시작 창.
        let session = projectWindow.dataSource.session
        window.close()
        expect(session?.isClosed == true, "창을 닫았는데 프로젝트 session을 닫지 않음")
        expect(coordinator.projectWindows.isEmpty, "닫은 창이 목록에 남음")
        expect(coordinator.welcomeController != nil, "마지막 프로젝트 창을 닫았는데 시작 창이 없음")
        expect(pump.waitForPendingRequests(timeout: 10), "프로젝트 닫기 완료 event가 오지 않음")
        expect(projectCount(host) == 0, "창을 닫은 뒤 코어 프로젝트 session이 남음")

        // 복원: 저장한 경로로 열기 중 창을 만들고 다시 엽니다.
        if let state = encodeRestorableState(path: projectPath) {
            let restored = coordinator.restoreProjectWindow(from: state)
            expect(restored != nil, "창 복원이 창을 만들지 않음")
            let restoredWindow = coordinator.projectWindows.first
            if case .loading = restoredWindow?.dataSource.currentSnapshot.status {} else {
                expect(false, "복원한 창이 열기 중 상태로 시작하지 않음")
            }
            expect(pump.waitForPendingRequests(timeout: 10), "복원 열기 완료 event가 오지 않음")
            expect(restoredWindow?.dataSource.currentSnapshot.status == .ready, "복원한 창이 ready가 아님")
            expect(restoredWindow?.dataSource.currentSnapshot.title == Self.projectName, "복원한 창 제목 불일치")
            restoredWindow?.window?.close()
            _ = pump.waitForPendingRequests(timeout: 10)
        }

        // 없어진 폴더 복원: 창은 만들되 실패 상태와 복구 동작(폴더 찾기, 다시 시도, 창 닫기)을 보여 줍니다.
        let missingPath = fixtureRoot.appendingPathComponent("Missing").path
        if let state = encodeRestorableState(path: missingPath), coordinator.restoreProjectWindow(from: state) != nil,
            let failedWindow = coordinator.projectWindows.first
        {
            expect(pump.waitForPendingRequests(timeout: 10), "없는 폴더 복원 완료 event가 오지 않음")
            failedWindow.windowController.flushPendingSnapshot()
            if case .failed(_, _, let actions) = failedWindow.dataSource.currentSnapshot.status {
                expect(actions.map(\.id) == [
                    ShellProjectCommandID.locateFolder, ShellProjectCommandID.retryOpen, ShellProjectCommandID.closeProject,
                ], "복원 실패 복구 동작 불일치: \(actions.map(\.id.rawValue))")
                expect(actions.filter(\.isPrimary).count == 1, "복원 실패 대표 동작이 하나가 아님")
            } else {
                expect(false, "없는 폴더 복원이 실패 상태가 아님")
            }
            expect(failedWindow.toolsModel.snapshot.enabledCommands == ShellWindowToolsModelStub.notReadyCommands,
                   "실패 상태에서 도구 명령이 활성")
            expect(!failedWindow.windowController.toolbarController.toolBar.isEnabled, "실패 상태에서 도구 막대가 활성")
            // 복구: Locate Folder…로 올바른 폴더를 고르면 같은 창에서 엽니다.
            nextChosenFolder = projectPath
            failedWindow.dataSource.send(.perform(ShellProjectCommandID.locateFolder))
            expect(pump.waitForPendingRequests(timeout: 10), "Locate Folder 열기 완료 event가 오지 않음")
            expect(failedWindow.dataSource.currentSnapshot.status == .ready, "Locate Folder 뒤 창이 ready가 아님")
            expect(coordinator.projectWindows.count == 1, "Locate Folder가 새 창을 만듦")
            failedWindow.window?.close()
            _ = pump.waitForPendingRequests(timeout: 10)
        } else {
            expect(false, "없는 폴더 복원 창이 없음")
        }
        expect(projectCount(host) == 0, "복원 검사 끝에 코어 프로젝트 session이 남음")

        checkCloseWhileOpening(coordinator, host: host, projectPath: projectPath)
        checkLocateFolderWhileOpening(coordinator, host: host, projectPath: projectPath, missingPath: missingPath)
        expect(projectCount(host) == 0, "검사 끝에 코어 프로젝트 session이 남음")

        // 앱 수준 Open Project 실패는 alert로, 시작 창 Open Project 실패는 시작 창 안내로 알립니다.
        let notRepository = makeFixture("Plain", manifest: nil, git: false)
        coordinator.openProject(atPath: notRepository, origin: .application)
        _ = pump.waitForPendingRequests(timeout: 10)
        expect(alerts.count == 1 && coordinator.projectWindows.isEmpty, "앱 수준 열기 실패가 alert로 오지 않음")
        let notStellar = makeFixture("GitOnly", manifest: nil)
        coordinator.openProject(atPath: notStellar, origin: .welcome)
        _ = pump.waitForPendingRequests(timeout: 10)
        if case .notStellarProject? = coordinator.welcomeData.currentSnapshot().notice?.kind {} else {
            expect(false, "시작 창 열기 실패가 'Stellar 프로젝트 아님' 안내가 아님")
        }
        checkNewProject(coordinator, pump: pump)

        // 설정 창.
        coordinator.showSettings()
        expect(coordinator.settingsController?.window?.contentViewController != nil, "설정 창 없음")
    }

    private func checkNewProject(_ coordinator: ShellAppCoordinator, pump: ShellRuntimeEventPump) {
        let name = "새 디자인"
        let folder = fixtureRoot.appendingPathComponent(name)
        let create = WelcomeCommand.createProject(parentDirectory: fixtureRoot.path, folderName: name)
        // 실제 메뉴 action과 시작 창의 이름 패널을 사용하고, 파일 선택 응답만 주입합니다.
        coordinator.showWelcome()
        guard let welcome = coordinator.welcomeController?.welcomeViewController,
              let menu = NSApp.mainMenu, let entry = item(.newProject, in: menu), let action = entry.action else {
            expect(false, "New Project 메뉴나 시작 창이 없음")
            return
        }
        if let notice = coordinator.welcomeData.currentSnapshot().notice {
            coordinator.welcomeData.send(.dismissNotice(id: notice.id))
        }
        welcome.renderNow()
        var location: URL?
        var panelCount = 0
        welcome.chooseNewProjectLocation = { panel, _, completion in
            panelCount += 1
            self.expect(panel.title == WelcomeStrings.newProjectPanelTitle, "새 프로젝트 이름 패널이 아님")
            completion(location)
        }
        expect(NSApp.sendAction(action, to: entry.target, from: entry) && panelCount == 1,
               "New Project 메뉴가 이름 패널을 열지 않음")
        expect(pump.pendingRequestCount == 0 && !FileManager.default.fileExists(atPath: folder.path), "취소했는데 생성이 진행됨")
        location = folder
        expect(NSApp.sendAction(action, to: entry.target, from: entry) && panelCount == 2,
               "New Project 메뉴에서 두 번째 이름 패널을 열지 못함")
        expect(coordinator.welcomeData.currentSnapshot().activity?.kind == .creating, "생성 진행 표시 없음")
        coordinator.welcomeData.send(create)
        expect(pump.pendingRequestCount == 1, "중복 생성 요청이 접수됨")
        expect(pump.waitForPendingRequests(timeout: 10), "프로젝트 생성과 열기를 완료하지 못함")
        let session = coordinator.projectWindows.first?.windowController.windowSession
        expect(coordinator.projectWindows.count == 1 && session?.documentID != nil && session?.openedGeneration == 1,
               "새 프로젝트 창에 최초 문서와 generation 1이 없음")
        for path in [".git", "stellar.yaml", "design/current"] {
            expect(FileManager.default.fileExists(atPath: folder.appendingPathComponent(path).path), "새 프로젝트 파일 누락: \(path)")
        }
        if let shell = coordinator.projectWindows.first {
            checkInitialFrame(shell, pump: pump)
        }
        coordinator.projectWindows.first?.window?.close()
        expect(pump.waitForPendingRequests(timeout: 10), "새 프로젝트 창을 닫지 못함")
        coordinator.welcomeData.send(create)
        if case .folderAlreadyExists? = coordinator.welcomeData.currentSnapshot().notice?.kind {} else {
            expect(false, "기존 폴더에 대한 중복 생성 안내가 없음")
        }
        expect(pump.pendingRequestCount == 0, "같은 폴더를 다시 생성하려 함")
        coordinator.openProject(atPath: folder.path, origin: .application)
        expect(pump.waitForPendingRequests(timeout: 10), "생성한 프로젝트 재열기 실패")
        expect(session?.documentID != nil && coordinator.projectWindows.first?.windowController.windowSession?.documentID == session?.documentID,
               "재열기에서 최초 문서의 ID가 바뀜")
        let reopened = coordinator.projectWindows.first?.windowController.windowSession?.viewInfo
        expect(reopened?.zoom == 1 && reopened?.center_x == 0 && reopened?.center_y == 0, "기존 프로젝트 재열기에 생성 시 카메라 맞춤이 적용됨")
        if let shell = coordinator.projectWindows.first { checkZoomToFit(shell, folder: folder, menu: menu, pump: pump) }
        coordinator.projectWindows.first?.window?.close()
        expect(pump.waitForPendingRequests(timeout: 10), "재열기 창을 닫지 못함")
    }

    private func checkFittedFrame(_ shell: ShellProjectWindow, capture: Bool = false) {
        guard let window = shell.window, let session = shell.windowController.windowSession,
              let initial = session.viewInfo else { expect(false, "새 창 viewport 없음"); return }
        let canvas = shell.canvasHost.canvasView
        let size = canvas.bounds.size
        let margin = min(24, min(size.width, size.height) / 10)
        let zoom = min(1, (size.width - 2 * margin) / 800, (size.height - 2 * margin) / 600)
        expect(initial.center_x == 400 && initial.center_y == 300 && abs(initial.zoom - zoom) < 0.00001,
               "실제 canvas에 Frame 전체가 맞지 않음: \(size), \(initial.zoom)")
        guard let image = CanvasBitmapCapture.bitmap(canvas, in: window),
              let hit = canvas.frameHit(at: CGPoint(x: size.width / 2, y: size.height / 2)) else {
            expect(false, "새 창 Frame의 bitmap 또는 hit 없음")
            return
        }
        func pixel(_ position: CGPoint) -> [Int] {
            var values = [Int](repeating: 0, count: image.samplesPerPixel)
            image.getPixel(&values, atX: Int(position.x / size.width * CGFloat(image.pixelsWide)),
                           y: Int(position.y / size.height * CGFloat(image.pixelsHigh)))
            return values
        }
        let bounds = CGRect(x: (size.width - zoom * 800) / 2, y: (size.height - zoom * 600) / 2,
                            width: zoom * 800, height: zoom * 600)
        let background = pixel(.zero), white = [255, 255, 255, 255]
        for (x, y, dx, dy) in [(bounds.minX, bounds.minY, 1.0, 1.0), (bounds.maxX, bounds.minY, -1, 1),
                               (bounds.maxX, bounds.maxY, -1, -1), (bounds.minX, bounds.maxY, 1, -1)] {
            let inside = CGPoint(x: x + dx * 2, y: y + dy * 2), outside = CGPoint(x: x - dx * 2, y: y - dy * 2)
            expect(pixel(inside) == white && pixel(outside) == background, "새 Frame의 모서리/여백 픽셀 불일치")
            expect(canvas.frameHit(at: inside)?.id == hit.id && canvas.frameHit(at: outside) == nil, "새 Frame 모서리 hit 불일치")
        }
        if capture, let path = ProcessInfo.processInfo.environment["STELLAR_TEST_NEW_CANVAS_PNG"],
           let png = image.representation(using: .png, properties: [:]) {
            do { try png.write(to: URL(fileURLWithPath: path)) } catch { expect(false, "새 프로젝트 화면 PNG 기록 실패") }
        }
    }

    /// Welcome에서 생성한 실제 generation과 최초 창. Resize 및 사용자의 카메라 변경은 다시 맞추지 않습니다.
    private func checkInitialFrame(_ shell: ShellProjectWindow, pump: ShellRuntimeEventPump) {
        checkFittedFrame(shell, capture: true)
        guard let window = shell.window, let session = shell.windowController.windowSession,
              let initial = session.viewInfo else { return }
        let canvas = shell.canvasHost.canvasView
        let size = canvas.bounds.size
        expect(shell.performSessionCommand(.zoomOut) && pump.waitForPendingRequests(timeout: 10), "사용자 축소 명령 실패")
        guard let changed = session.viewInfo else { expect(false, "축소된 viewport 없음"); return }
        let oldSize = window.contentView?.bounds.size ?? size
        window.setContentSize(NSSize(width: oldSize.width + 160, height: oldSize.height + 60))
        window.contentView?.layoutSubtreeIfNeeded()
        expect(pump.waitForPendingRequests(timeout: 10), "resize 처리 실패")
        expect(canvas.bounds.size != size && changed.zoom == initial.zoom / 2 && session.viewInfo?.zoom == changed.zoom &&
               session.viewInfo?.center_x == changed.center_x && session.viewInfo?.center_y == changed.center_y,
               "창 크기 변경이 사용자 viewport를 덮어씀")
        expect(session.info?.has_unsaved_changes == 0 && session.info?.revision == 0, "최초 맞춤이 문서를 수정함")
    }

    private func checkZoomToFit(_ shell: ShellProjectWindow, folder: URL, menu: NSMenu, pump: ShellRuntimeEventPump) {
        guard let window = shell.window, let session = shell.windowController.windowSession, let initial = session.info,
              let viewMenu = ShellMainMenuInstaller.submenu(titled: "View", in: menu),
              let entry = item(.zoomToFit, in: viewMenu), let toolbar = item(.zoomToFit, in: shell.toolsToolbar.zoomItem.menu),
              let key = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 1,
                  windowNumber: window.windowNumber, context: nil, characters: "9", charactersIgnoringModifiers: "9",
                  isARepeat: false, keyCode: 25) else { expect(false, "재열기 맞춤 메뉴·창 없음"); return }
        func projectFiles() throws -> [String: Data] {
            var files: [String: Data] = [:]
            for path in try FileManager.default.subpathsOfDirectory(atPath: folder.path) {
                let url = folder.appendingPathComponent(path)
                if try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true { files[path] = try Data(contentsOf: url) }
            }
            return files
        }
        guard let bytes = try? projectFiles(), bytes[".git/HEAD"] != nil, bytes["design/current"] != nil else {
            expect(false, "프로젝트·Git 원본 읽기 실패"); return
        }
        // 화면 밖 창을 key로 만들지 않고 실제 View 메뉴의 key equivalent를 그 창의 router로 보냅니다.
        let target = entry.target
        entry.target = shell.router
        defer { entry.target = target }
        expect(entry.keyEquivalent == "9" && viewMenu.performKeyEquivalent(with: key) &&
               pump.waitForPendingRequests(timeout: 10), "재열기 후 ⌘9 실패")
        checkFittedFrame(shell)
        expect(session.pan(dx: 900, dy: -700) { shell.refreshSessionCommands() } && pump.waitForPendingRequests(timeout: 10) &&
               session.viewInfo?.center_x != 400, "툴바 검사 전 pan 실패")
        guard let action = toolbar.action else { expect(false, "툴바의 Zoom to Fit action 없음"); return }
        expect(toolbar.keyEquivalent.isEmpty && shell.router.validateMenuItem(toolbar) &&
               NSApp.sendAction(action, to: toolbar.target, from: toolbar) && pump.waitForPendingRequests(timeout: 10),
               "툴바의 Zoom to Fit 실패")
        checkFittedFrame(shell)
        let after = session.info
        expect(after?.revision == initial.revision && after?.generation == initial.generation &&
               after?.undo_depth == initial.undo_depth && after?.redo_depth == initial.redo_depth &&
               after?.selection_count == initial.selection_count && after?.has_unsaved_changes == initial.has_unsaved_changes &&
               (try? projectFiles()) == bytes, "루트 맞춤이 문서·history·Git 파일을 변경")
    }

    /// 열기 중 창 닫기: 복원(Try Again·Locate Folder…도 같은 경로) 열기가 끝나기 전에 창을 닫으면, 늦게 온
    /// `PROJECT_OPENED`를 닫힌 창에 붙이지 않고 그 session을 바로 닫습니다(창 닫기 = 프로젝트 닫기).
    private func checkCloseWhileOpening(_ coordinator: ShellAppCoordinator, host: RuntimeHost, projectPath: String) {
        for afterProjectOpened in [false, true] {
            checkCloseWhileOpening(coordinator, host: host, projectPath: projectPath, afterProjectOpened: afterProjectOpened)
        }
    }

    private func checkCloseWhileOpening(
        _ coordinator: ShellAppCoordinator, host: RuntimeHost, projectPath: String, afterProjectOpened: Bool
    ) {
        guard let pump = coordinator.pump, let state = encodeRestorableState(path: projectPath),
            coordinator.restoreProjectWindow(from: state) != nil
        else {
            expect(false, "열기 중 닫기 검사용 복원 창이 없음")
            return
        }
        // 창 객체를 붙잡아 둡니다: 닫혔지만 아직 살아 있는 창에도 늦은 결과가 붙지 않아야 합니다.
        let loadingWindow = coordinator.projectWindows.first
        if case .loading = loadingWindow?.dataSource.currentSnapshot.status {} else {
            expect(false, "열기 중 닫기 검사 창이 열기 중 상태가 아님")
        }
        expect(pump.pendingRequestCount == 1, "복원 열기 요청이 대기 중이 아님: \(pump.pendingRequestCount)")
        if afterProjectOpened {
            // timer/run loop 없이 첫 완료만 전달해 창 session 열기 직후에 닫습니다.
            let deadline = Date().addingTimeInterval(10)
            var drained = 0
            while drained == 0 && Date() < deadline {
                drained = pump.drain(limit: 1)
                if drained == 0 { Thread.sleep(forTimeInterval: 0.001) }
            }
            expect(drained == 1 && loadingWindow?.pendingOpenRequest != nil,
                   "프로젝트 열기 뒤 창 session 대기 단계에 도달하지 못함")
            expect(loadingWindow?.windowController.windowSession == nil, "창 session 완료를 먼저 전달함")
        }
        loadingWindow?.window?.close()
        expect(coordinator.projectWindows.isEmpty, "열기 중 닫은 창이 목록에 남음")
        expect(coordinator.welcomeController != nil, "열기 중 마지막 창을 닫았는데 시작 창이 없음")
        expect(pump.waitForPendingRequests(timeout: 10), "열기 중 닫은 창의 늦은 열기·닫기 완료 event가 오지 않음")
        expect(loadingWindow?.dataSource.session == nil, "닫은 창에 늦게 연 session이 붙음")
        expect(projectCount(host) == 0, "열기 중 창을 닫았는데 늦게 연 코어 프로젝트 session이 남음: \(projectCount(host))")
        expect(CoreRuntime.state(handle: host.handle)?.window_session_count == 0, "늦게 연 창 session이 남음")
    }

    /// Locate Folder…로 고른 폴더를 다른 열기(메뉴·Finder)가 이미 여는 중이면, 같은 폴더 창을 둘 만들지 않고 이 창을
    /// 닫습니다. 여는 중인 열기가 끝나면 그 창 하나만 남습니다(가이드 2.5).
    private func checkLocateFolderWhileOpening(
        _ coordinator: ShellAppCoordinator, host: RuntimeHost, projectPath: String, missingPath: String
    ) {
        guard let pump = coordinator.pump, let state = encodeRestorableState(path: missingPath),
            coordinator.restoreProjectWindow(from: state) != nil, let failedWindow = coordinator.projectWindows.first
        else {
            expect(false, "Locate Folder 중복 검사용 복원 창이 없음")
            return
        }
        expect(pump.waitForPendingRequests(timeout: 10), "없는 폴더 복원 완료 event가 오지 않음")
        coordinator.openProject(atPath: projectPath, origin: .application)
        expect(coordinator.pendingOpenPaths.contains(ShellAppCoordinator.canonicalPath(projectPath)),
               "앱 수준 열기가 대기 중이 아님")
        // 여는 중인 폴더의 창 복원도 새 창을 만들지 않습니다.
        if let pendingState = encodeRestorableState(path: projectPath) {
            expect(coordinator.restoreProjectWindow(from: pendingState) == nil && coordinator.projectWindows.count == 1,
                   "여는 중인 폴더 복원이 새 창을 만듦")
        }
        nextChosenFolder = projectPath
        failedWindow.dataSource.send(.perform(ShellProjectCommandID.locateFolder))
        expect(pump.waitForPendingRequests(timeout: 10), "Locate Folder 중복 검사 완료 event가 오지 않음")
        expect(coordinator.projectWindows.count == 1,
               "여는 중인 폴더를 Locate Folder…로 골라 창이 \(coordinator.projectWindows.count)개가 됨")
        expect(!coordinator.projectWindows.contains { $0 === failedWindow }, "여는 중인 폴더를 고른 실패 창이 남음")
        expect(coordinator.projectWindows.first?.dataSource.currentSnapshot.status == .ready, "여는 중이던 폴더 창이 ready가 아님")
        expect(projectCount(host) == 1, "같은 폴더 코어 프로젝트 session이 1이 아님: \(projectCount(host))")
        for projectWindow in coordinator.projectWindows {
            projectWindow.window?.close()
        }
        _ = pump.waitForPendingRequests(timeout: 10)
    }

    private func encodeRestorableState(_ projectWindow: ShellProjectWindow) -> NSCoder? {
        guard let window = projectWindow.window else { return nil }
        let archiver = NSKeyedArchiver(requiringSecureCoding: true)
        projectWindow.windowController.window(window, willEncodeRestorableState: archiver)
        archiver.finishEncoding()
        return try? NSKeyedUnarchiver(forReadingFrom: archiver.encodedData)
    }

    private func encodeRestorableState(path: String) -> NSCoder? {
        let archiver = NSKeyedArchiver(requiringSecureCoding: true)
        archiver.encode(path as NSString, forKey: ShellAppCoordinator.restorationRootPathKey)
        archiver.finishEncoding()
        return try? NSKeyedUnarchiver(forReadingFrom: archiver.encodedData)
    }

    private func checkComposition(_ projectWindow: ShellProjectWindow, window: NSWindow, host: RuntimeHost) {
        let controller = projectWindow.windowController
        let snapshot = controller.snapshot
        expect(window.title == Self.projectName, "창 제목이 프로젝트 이름이 아님: \(window.title)")
        expect(window.subtitle == "Restricted Mode", "제한 모드 부제목 없음: \(window.subtitle)")
        expect(snapshot.status == .ready, "프로젝트 창이 ready가 아님")
        expect(projectWindow.dataSource.session?.name == Self.projectName, "session 이름 불일치")
        expect(window.isRestorable && window.restorationClass == ShellWindowRestoration.self, "프로젝트 창 복원 설정 없음")
        expect(controller.configuration.autosaveName != ShellWindowConfiguration.standard.autosaveName,
               "검사 창이 앱의 실제 autosave 이름을 씀")

        let split = controller.splitController
        expect(split.contentContainer.contentSlot === projectWindow.canvasHost, "가운데 슬롯이 캔버스 호스트가 아님")
        expect(projectWindow.canvasHost.canvasView.runtimeHandle == host.handle, "캔버스에 runtime handle이 없음")
        expect(split.sidebarItem.viewController is SidebarViewController, "사이드바 슬롯이 SidebarViewController가 아님")
        expect(split.inspectorItem.viewController is InspectorViewController, "인스펙터 슬롯이 InspectorViewController가 아님")
        // 캔버스 아래 보조 패널(Git·Agent 탭)은 없습니다(UX 명세 3.2). 창 안 어디에도 Git 패널이 없어야 합니다(에이전트
        // 패널 코드는 쓰이지 않아 지웠습니다).
        expect(split.splitViewItems.count == 3 && split.centerItem.viewController === split.contentContainer,
               "캔버스 아래에 보조 패널 분할이 남음")
        func containsPanel(_ view: NSView?) -> Bool {
            guard let view else { return false }
            if view.nextResponder is GitPanelViewController { return true }
            return view.subviews.contains { containsPanel($0) }
        }
        expect(!containsPanel(window.contentView), "창에 Git 패널이 남음")
        expect(split.sidebarItem.topAlignedAccessoryViewControllers.contains { $0 is SidebarFilterAccessoryViewController },
               "사이드바 필터 accessory 없음")
        expect(!projectWindow.requiresConfirmationBeforeClosing, "에이전트 세션이 없는데 닫기 확인이 필요함")
        let drawable = projectWindow.canvasHost.canvasView.currentDrawable()
        expect(drawable.width > 0 && drawable.height > 0, "캔버스 drawable 크기가 0")

        // 툴바: 줌을 셸 툴바(인스펙터 경계 앞)에 조합합니다. 캔버스 도구는 상단 툴바가 아니라 캔버스 하단 가운데 도구 막대에
        // 있습니다(2026-11-14 사용자 결정).
        guard let toolbar = window.toolbar else {
            expect(false, "툴바 없음")
            return
        }
        let identifiers = toolbar.items.map(\.itemIdentifier)
        expect(!identifiers.contains(ToolsToolbarController.toolsItemIdentifier), "상단 툴바에 캔버스 도구 그룹이 남음")
        expect(identifiers.contains(ToolsToolbarController.zoomItemIdentifier), "툴바에 줌 항목 없음")
        expect(!identifiers.contains(NSToolbarItem.Identifier("stellar.shell.toolArea")), "상단 툴바에 셸 도구 영역이 남음")
        expect(toolbar.centeredItemIdentifiers.isEmpty, "상단 툴바에 가운데 고정 항목이 남음")
        let toolBar = controller.toolbarController.toolBar
        expect(toolBar.superview === split.contentContainer.view, "도구 막대가 캔버스 영역에 없음")
        expect(toolBar.buttons.map(\.tool) == CanvasTool.allCases.map(\.shellToolDescriptor),
               "도구 막대 도구 목록이 캔버스 도구와 다름")
        expect(toolBar.selectedIndex == CanvasTool.allCases.firstIndex(of: projectWindow.toolsModel.snapshot.activeTool),
               "도구 막대 선택이 창 도구 모델의 현재 도구가 아님")
        expect(identifiers.firstIndex(of: ToolsToolbarController.zoomItemIdentifier).map { zoom in
            identifiers.firstIndex(of: .inspectorTrackingSeparator).map { zoom < $0 } ?? false
        } ?? false, "줌 항목이 인스펙터 경계 앞에 있지 않음")
        // 제목 문서 메뉴: 제한 모드에서도 신뢰 부여 항목이 늘 있습니다(UX 명세 3.3).
        let titleMenu = controller.toolbarController.titleView.titleButton.menu?.items ?? []
        expect(identifiers.contains(ShellToolbarController.titleItem) && window.titleVisibility == .hidden,
               "상단 바 제목 항목이 없거나 시스템 제목이 겹쳐 보임")
        expect(titleMenu.first?.title == Self.projectName
               && titleMenu.dropFirst().map { $0.representedObject as? String } == [
                   ShellProjectCommandID.revealInFinder.rawValue, ShellProjectCommandID.trustProject.rawValue,
                   ShellProjectCommandID.closeProject.rawValue,
               ], "제한 모드 제목 문서 메뉴 항목 불일치: \(titleMenu.map(\.title))")

        // 응답자 체인: content view → split controller → router → 창(순환 없음).
        expect(window.contentView?.nextResponder === split, "content view 다음이 split controller가 아님")
        expect(split.nextResponder === projectWindow.router && projectWindow.router.nextResponder === window,
               "router가 split controller와 창 사이에 없음")
    }

    private func checkTools(_ projectWindow: ShellProjectWindow, window: NSWindow) {
        let router = projectWindow.router
        let model = projectWindow.toolsModel
        let controller = projectWindow.windowController
        let toolBar = controller.toolbarController.toolBar
        let tools = CanvasTool.allCases
        expect(model.snapshot.enabledCommands == ShellWindowToolsModelStub.readyCommands, "ready 창의 실행 가능 명령 불일치")
        expect(toolBar.isEnabled && toolBar.buttons.allSatisfy(\.isEnabled), "ready 창에서 도구 막대가 비활성")

        // 하단 도구 막대 → 창 모델 → 메뉴 체크, 막대 선택 표시는 창 모델 snapshot으로 바뀝니다.
        if let frame = tools.firstIndex(of: .frame), toolBar.buttons.indices.contains(frame) {
            toolBar.buttons[frame].performClick(nil)
            expect(model.snapshot.activeTool == .frame, "도구 막대 선택이 창 모델에 반영되지 않음")
            controller.flushPendingSnapshot()
            expect(toolBar.selectedIndex == frame, "도구 막대 선택 표시가 창 모델 도구로 바뀌지 않음")
            let frameItem = MainMenuBuilder.makeItem(for: .selectFrameTool, target: nil)
            _ = router.validateMenuItem(frameItem)
            expect(frameItem.state == .on, "Frame 메뉴 체크 없음")
        } else {
            expect(false, "도구 막대에 Frame 없음")
        }

        // 캔버스 단일 키: R → Rectangle, 수식키가 있거나 도구가 아닌 키는 코어 입력으로 보냅니다.
        let canvas = projectWindow.canvasHost.canvasView
        expect(window.makeFirstResponder(canvas), "캔버스 first responder 실패")
        if let event = keyEvent("r", keyCode: 0x0F, window: window) {
            canvas.keyDown(with: event)
            expect(model.snapshot.activeTool == .rectangle, "캔버스 R 키가 Rectangle 도구로 바뀌지 않음")
            controller.flushPendingSnapshot()
            expect(toolBar.selectedIndex == tools.firstIndex(of: .rectangle), "캔버스 R 키 뒤 도구 막대 선택 표시 미반영")
        }
        if let event = keyEvent("1", keyCode: 0x12, window: window), let handler = canvas.keyDownHandler {
            expect(!handler(event), "도구가 아닌 키를 셸이 삼킴")
        }
        if let event = keyEvent("r", keyCode: 0x0F, modifiers: .command, window: window), let handler = canvas.keyDownHandler {
            expect(!handler(event), "⌘R을 단일 키 도구로 해석함")
        }

        // 코어 경로가 없는 명령(복제)과, 저장한 문서·history가 없는 창 session의 Undo·Redo·Save Now(XII.6.4)·줌(XII.4.3,
        // active view 없음)은 숨기지 않고 비활성, Close Project는 활성.
        for command in [ShellCommand.undo, .redo, .saveNow, .duplicate, .zoomIn] {
            let entry = MainMenuBuilder.makeItem(for: command, target: nil)
            expect(!router.validateMenuItem(entry), "실행할 수 없는 \(command.rawValue)가 활성")
        }
        let close = MainMenuBuilder.makeItem(for: .closeProject, target: nil)
        expect(router.validateMenuItem(close), "Close Project가 비활성")
        let reveal = ShellFrameMenuItems.makeFrameActionItem(title: "Show in Finder", id: ShellProjectCommandID.revealInFinder)
        expect(controller.validateMenuItem(reveal), "Show in Finder 메뉴가 비활성")
        // 메뉴 막대 신뢰 항목: 제한 모드 창에서는 Trust Project…로 활성입니다(툴바를 거치지 않는 신뢰 경로).
        let trust = ShellFrameMenuItems.makeFrameActionItem(
            title: "", alternatives: [ShellProjectCommandID.trustProject, ShellProjectCommandID.revokeTrust])
        expect(controller.validateMenuItem(trust) && trust.title == String(localized: "Trust Project…", comment: "Title document menu item"),
               "메뉴 막대 신뢰 항목이 비활성이거나 제목이 다름: \(trust.title)")
        window.makeFirstResponder(nil)
    }

    private func keyEvent(
        _ characters: String,
        keyCode: UInt16,
        modifiers: NSEvent.ModifierFlags = [],
        window: NSWindow
    ) -> NSEvent? {
        NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: characters,
            charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode
        )
    }
}

/// 검사 전용 AppKit 상태 저장 이름. 앱의 실제 창 기록(`ShellWindowConfiguration.standard`)을 읽거나 덮어쓰지 않고,
/// 시작과 끝에 같은 접두어의 user defaults 기록(중간에 비정상 종료한 이전 실행 포함)을 지웁니다.
@MainActor
final class ShellCheckAutosave {
    let prefix: String
    let configuration: ShellWindowConfiguration

    init(prefix: String) {
        self.prefix = prefix
        let token = UUID().uuidString
        configuration = ShellWindowConfiguration(
            autosaveName: "\(prefix).Window.\(token)",
            toolbarIdentifier: "\(prefix).Toolbar.\(token)",
            restorationIdentifier: "\(prefix).Window.\(token)"
        )
        removeRecords()
    }

    func removeRecords() {
        let defaults = UserDefaults.standard
        for key in defaults.dictionaryRepresentation().keys where key.contains(prefix) {
            defaults.removeObject(forKey: key)
        }
    }
}
