import AppKit

/// 앱 셸 통합 검사(`--shell-integration-check`).
///
/// 실제 앱과 같은 구성(`ShellAppCoordinator`, 실제 코어 runtime, 영역 view controller, `MainMenuBuilder` 메뉴)을 화면에
/// 띄우지 않고 만들어 다음을 확인합니다.
/// - 메인 메뉴: 앱 수준 명령 target·활성 상태, 셸 프레임 항목 설치, key equivalent 중복 없음
/// - 시작 창: 열린 프로젝트가 없으면 표시, 프로젝트를 열면 닫힘, 마지막 창을 닫으면 다시 표시
/// - Open Project: 코어 `stellar_project_open` → 완료 event → 프로젝트 창(제목, 제한 모드 부제목, ready)
/// - 프로젝트 창 구성: 캔버스 호스트(runtime handle), 사이드바·인스펙터·Git·에이전트 영역, 캔버스 도구 그룹·줌 툴바 조합,
///   응답자 체인 router, 도구 선택·단일 키 단축키, 코어 경로 없는 명령 비활성
/// - 같은 폴더 재열기 시 새 창 없음, 창 복원 상태 왕복, 없어진 폴더 복원의 복구 동작, 창 닫기 시 코어 session 닫기
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
            "NSSplitView Subview Frames \(standard.centerSplitAutosaveName)",
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
            print("shell-integration ok: \(checks) checks (menu, welcome, open via core, window composition, tools, restore, close)")
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
            (ShellCommand.openProject, true), (.settings, true), (.newProject, false), (.cloneProject, false),
        ] {
            guard let entry = item(command, in: menu) else {
                expect(false, "\(command.rawValue) 메뉴 항목 없음")
                continue
            }
            expect(entry.target === router, "\(command.rawValue) target이 앱 수준 router가 아님")
            expect(router.validateMenuItem(entry) == enabled, "\(command.rawValue) 활성 상태가 \(enabled)가 아님")
        }

        // 셸 프레임 항목(가이드 6.1): 보조 패널 전환, 뒤로·앞으로, 프로젝트 검색, Finder에서 보기.
        let auxiliary = items(withAction: #selector(ShellWindowController.toggleAuxiliaryPanel(_:)), in: menu)
        expect(auxiliary.count == 1 && auxiliary.first?.title == "Show Panels", "View 메뉴에 Show Panels 없음")
        for action in [#selector(ShellWindowController.navigateBack(_:)), #selector(ShellWindowController.navigateForward(_:)),
                       #selector(ShellWindowController.focusProjectSearch(_:)), #selector(ShellWindowController.performFrameAction(_:))]
        {
            let found = items(withAction: action, in: menu)
            expect(found.count == 1 && found.allSatisfy { $0.target == nil }, "셸 프레임 메뉴 항목 설치 불일치: \(action)")
        }

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
        coordinator.welcomeData.send(.createProject(parentDirectory: fixtureRoot.path, folderName: "New"))
        if case .operationFailed? = coordinator.welcomeData.currentSnapshot().notice?.kind {} else {
            expect(false, "New Project 미지원 안내 없음")
        }
        expect(FileManager.default.fileExists(atPath: fixtureRoot.appendingPathComponent("New").path) == false,
               "미지원 New Project가 폴더를 만듦")

        // 설정 창.
        coordinator.showSettings()
        expect(coordinator.settingsController?.window?.contentViewController != nil, "설정 창 없음")
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
        let tabs = split.centerController.auxiliaryItem?.viewController as? NSTabViewController
        expect(tabs?.tabViewItems.map(\.viewController).contains { $0 is GitPanelViewController } == true, "보조 패널에 Git 패널 없음")
        expect(tabs?.tabViewItems.map(\.viewController).contains { $0 is AgentPanelViewController } == true, "보조 패널에 에이전트 패널 없음")
        expect(split.centerController.auxiliaryItem?.isCollapsed == true, "보조 패널이 처음에 펼쳐짐")
        expect(split.sidebarItem.topAlignedAccessoryViewControllers.contains { $0 is SidebarFilterAccessoryViewController },
               "사이드바 필터 accessory 없음")
        expect(!projectWindow.requiresConfirmationBeforeClosing, "에이전트 세션이 없는데 닫기 확인이 필요함")
        let drawable = projectWindow.canvasHost.canvasView.currentDrawable()
        expect(drawable.width > 0 && drawable.height > 0, "캔버스 drawable 크기가 0")

        // 툴바: 캔버스 도구 그룹(가운데)과 줌을 셸 툴바에 조합합니다.
        guard let toolbar = window.toolbar else {
            expect(false, "툴바 없음")
            return
        }
        let identifiers = toolbar.items.map(\.itemIdentifier)
        expect(identifiers.contains(ToolsToolbarController.toolsItemIdentifier), "툴바에 캔버스 도구 그룹 없음")
        expect(identifiers.contains(ToolsToolbarController.zoomItemIdentifier), "툴바에 줌 항목 없음")
        expect(!identifiers.contains(ShellToolbarController.toolAreaItem), "provider가 있는데 셸 도구 영역이 남음")
        expect(toolbar.centeredItemIdentifiers == [ToolsToolbarController.toolsItemIdentifier], "가운데 항목이 캔버스 도구 그룹이 아님")
        let toolsItem = toolbar.items.first { $0.itemIdentifier == ToolsToolbarController.toolsItemIdentifier }
        expect(toolsItem === projectWindow.toolsToolbar.toolsGroup, "툴바 도구 그룹이 도구 controller 항목이 아님")
        expect(identifiers.firstIndex(of: ToolsToolbarController.zoomItemIdentifier).map { zoom in
            identifiers.firstIndex(of: ShellToolbarController.searchItem).map { zoom < $0 } ?? false
        } ?? false, "줌 항목이 검색 앞에 있지 않음")
        expect(controller.toolbarController.searchToolbarItem?.searchField.isEnabled == false, "코어 검색 없이 검색 필드가 활성")

        // 응답자 체인: content view → split controller → router → 창(순환 없음).
        expect(window.contentView?.nextResponder === split, "content view 다음이 split controller가 아님")
        expect(split.nextResponder === projectWindow.router && projectWindow.router.nextResponder === window,
               "router가 split controller와 창 사이에 없음")
    }

    private func checkTools(_ projectWindow: ShellProjectWindow, window: NSWindow) {
        let router = projectWindow.router
        let model = projectWindow.toolsModel
        let group = projectWindow.toolsToolbar.toolsGroup
        let tools = CanvasTool.allCases
        expect(model.snapshot.enabledCommands == ShellWindowToolsModelStub.readyCommands, "ready 창의 실행 가능 명령 불일치")
        expect(group.isEnabled, "ready 창에서 도구 그룹이 비활성")

        // 툴바 도구 그룹 → 창 모델 → 메뉴 체크.
        if let pen = tools.firstIndex(of: .pen) {
            group.selectedIndex = pen
            projectWindow.toolsToolbar.toolSelectionChanged(group)
            expect(model.snapshot.activeTool == .pen, "도구 그룹 선택이 창 모델에 반영되지 않음")
            let penItem = MainMenuBuilder.makeItem(for: .selectPenTool, target: nil)
            _ = router.validateMenuItem(penItem)
            expect(penItem.state == .on, "Pen 메뉴 체크 없음")
        }

        // 캔버스 단일 키: R → Rectangle, 수식키가 있거나 도구가 아닌 키는 코어 입력으로 보냅니다.
        let canvas = projectWindow.canvasHost.canvasView
        expect(window.makeFirstResponder(canvas), "캔버스 first responder 실패")
        if let event = keyEvent("r", keyCode: 0x0F, window: window) {
            canvas.keyDown(with: event)
            expect(model.snapshot.activeTool == .rectangle, "캔버스 R 키가 Rectangle 도구로 바뀌지 않음")
        }
        if let event = keyEvent("1", keyCode: 0x12, window: window), let handler = canvas.keyDownHandler {
            expect(!handler(event), "도구가 아닌 키를 셸이 삼킴")
        }
        if let event = keyEvent("r", keyCode: 0x0F, modifiers: .command, window: window), let handler = canvas.keyDownHandler {
            expect(!handler(event), "⌘R을 단일 키 도구로 해석함")
        }

        // 코어 경로가 없는 명령은 숨기지 않고 비활성, Close Project는 활성.
        for command in [ShellCommand.undo, .redo, .saveNow, .duplicate, .zoomIn] {
            let entry = MainMenuBuilder.makeItem(for: command, target: nil)
            expect(!router.validateMenuItem(entry), "코어 경로 없는 \(command.rawValue)가 활성")
        }
        let close = MainMenuBuilder.makeItem(for: .closeProject, target: nil)
        expect(router.validateMenuItem(close), "Close Project가 비활성")
        let controller = projectWindow.windowController
        expect(!controller.validateMenuItem(ShellFrameMenuItems.makeNavigationMenuItems()[2]), "코어 검색 없이 Search Project…가 활성")
        let reveal = ShellFrameMenuItems.makeFrameActionItem(title: "Show in Finder", id: ShellProjectCommandID.revealInFinder)
        expect(controller.validateMenuItem(reveal), "Show in Finder 메뉴가 비활성")
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
