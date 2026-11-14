import AppKit
import SwiftUI

/// 앱 수준 셸 구성: 메인 메뉴, 시작 창, 프로젝트 창 열기·복원·닫기, 설정 창.
///
/// - 열린 프로젝트 창이 없으면 시작 창(`WelcomeWindowController`)을 보여 주고, 프로젝트 창이 열리면 닫습니다
///   (가이드 7.1 "열린 프로젝트가 없음").
/// - Open Project(메뉴·시작 창·Finder에서 폴더 열기)는 코어 `stellar_project_open`으로 열고, 그 프로젝트 session 아래에서
///   창 session(`stellar_window_session_open`, 작업 문서·history·선택, XII.2.2)까지 연 뒤 프로젝트 창을 만듭니다. 창
///   controller가 창 session handle 하나를 소유하고, 창을 닫으면 창 session → 프로젝트 session 순서로 닫습니다. 같은
///   폴더를 연 창이 이미 있으면 새 창을 만들지 않고 그 창을 앞으로 가져옵니다(가이드 2.5).
/// - 창 복원은 프로젝트 폴더 경로만 창 상태에 저장하고, 복원할 때 같은 경로를 다시 엽니다(가이드 2.6).
/// - New Project는 Git 프로젝트와 첫 문서를 만들고 창을 엽니다. Clone은 시작 창에서 사용 가능 여부를 안내합니다.
@MainActor
final class ShellAppCoordinator: NSObject {
    /// 창 복원 상태에 프로젝트 폴더 경로를 넣는 키.
    static let restorationRootPathKey = "StellarProjectRootPath"

    /// 프로젝트 열기를 시작한 곳. 실패 안내 위치가 달라집니다.
    enum OpenOrigin: Equatable {
        /// 시작 창의 Open Project…: 시작 창 안내(sheet)로 알립니다.
        case welcome
        /// 메뉴 막대 Open Project…·Finder·`open` 명령: alert로 알립니다.
        case application
    }

    let runtimeHandle: UInt64?
    let pump: ShellRuntimeEventPump?
    let applicationModel = ShellApplicationCommandModel()
    let applicationRouter: ShellCommandRouter
    let welcomeData = ShellWelcomeDataSource()
    /// 코어 command ID 표(XII.6.5). 코어에 공유 이름으로 물어 한 번 만들고 프로젝트 창마다 넘깁니다.
    let coreCommandIDs = ShellCommandCoreIDs.fromCore()
    private(set) var projectWindows: [ShellProjectWindow] = []
    private(set) var welcomeController: WelcomeWindowController?
    private(set) var settingsController: NSWindowController?
    /// 열기 요청을 보냈고 완료를 기다리는 폴더(같은 폴더 중복 요청 방지).
    private(set) var pendingOpenPaths: Set<String> = []

    /// false이면 창을 화면에 올리지 않습니다(화면 밖 검사).
    var presentsWindows = true
    /// 프로젝트 창의 AppKit 상태 저장 이름(frame·split·툴바 autosave, 복원 식별자). 검사는 앱의 실제 기록을 읽거나
    /// 덮어쓰지 않도록 검사 전용 이름을 넘깁니다.
    var windowConfiguration: () -> ShellWindowConfiguration = { .standard }
    /// 프로젝트 열기 실패를 alert로 알립니다. 검사에서 바꿉니다.
    lazy var alertPresenter: (_ title: String, _ message: String, _ window: NSWindow?) -> Void = { title, message, window in
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: String(localized: "OK", comment: "Button"))
        if let window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }
    /// 프로젝트 폴더를 고릅니다(NSOpenPanel). 검사에서 바꿉니다.
    lazy var folderChooser: (_ window: NSWindow?, _ completion: @escaping (String?) -> Void) -> Void = { window, completion in
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.prompt = String(localized: "Open", comment: "Open panel button")
        panel.message = String(localized: "Choose a Stellar project folder.", comment: "Open panel message")
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            completion(response == .OK ? panel.url?.path : nil)
        }
        if let window, window.isVisible {
            panel.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    init(runtimeHandle: UInt64?) {
        self.runtimeHandle = runtimeHandle
        pump = runtimeHandle.map { ShellRuntimeEventPump(runtimeHandle: $0) }
        applicationRouter = ShellCommandRouter(model: applicationModel)
        super.init()
        applicationModel.commandHandler = { [weak self] command in
            self?.performApplicationCommand(command)
        }
        welcomeData.coordinator = self
    }

    // MARK: - 앱 시작

    /// 메인 메뉴를 만들어 앱에 연결합니다(MainMenuBuilder + 셸 프레임 항목).
    @discardableResult
    func installMainMenu(on application: NSApplication) -> MainMenuBuilder.MainMenu {
        let mainMenu = MainMenuBuilder.build(applicationTarget: applicationRouter)
        ShellMainMenuInstaller.install(into: mainMenu)
        mainMenu.install(on: application)
        return mainMenu
    }

    /// 창 복원 진입점을 등록합니다. AppKit은 `applicationDidFinishLaunching` 전에 복원하므로 그 전에 부릅니다.
    func registerWindowRestoration() {
        ShellWindowRestoration.windowProvider = { [weak self] _, state in
            self?.restoreProjectWindow(from: state)
        }
    }

    /// 열린 프로젝트 창이 없으면 시작 창을 보여 줍니다.
    func showWelcomeIfNeeded() {
        guard projectWindows.isEmpty else { return }
        showWelcome()
    }

    func showWelcome() {
        let controller: WelcomeWindowController
        if let welcomeController {
            controller = welcomeController
        } else {
            controller = WelcomeWindowController(dataSource: welcomeData)
            welcomeController = controller
        }
        guard presentsWindows else { return }
        controller.showWindow(nil)
        controller.window?.makeKeyAndOrderFront(nil)
        controller.presentCommandInstallOfferIfNeeded()
    }

    private func closeWelcome() {
        welcomeController?.close()
        welcomeController = nil
    }

    // MARK: - 앱 수준 명령

    private func performApplicationCommand(_ command: ShellCommand) {
        switch command {
        case .newProject:
            showWelcome()
            welcomeController?.welcomeViewController.newStellarProject(nil)
        case .openProject:
            chooseProjectToOpen(origin: .application)
        case .settings:
            showSettings()
        default:
            break
        }
    }

    /// 폴더를 골라 프로젝트를 엽니다.
    func chooseProjectToOpen(origin: OpenOrigin) {
        let window = origin == .welcome ? welcomeController?.window : NSApp.keyWindow
        folderChooser(window) { [weak self] path in
            guard let path else { return }
            self?.openProject(atPath: path, origin: origin)
        }
    }

    /// 설정 창(SwiftUI placeholder, 가이드 5.5).
    func showSettings() {
        let controller: NSWindowController
        if let settingsController {
            controller = settingsController
        } else {
            let hosting = NSHostingController(rootView: SettingsPlaceholderView())
            let window = NSWindow(contentViewController: hosting)
            window.title = String(localized: "Settings", comment: "Settings window title")
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.center()
            controller = NSWindowController(window: window)
            settingsController = controller
        }
        guard presentsWindows else { return }
        controller.showWindow(nil)
        controller.window?.makeKeyAndOrderFront(nil)
    }

    // MARK: - 프로젝트 열기 (XII.6.2)

    /// 경로를 비교할 수 있게 표준화합니다(심볼릭 링크·`..` 해소).
    static func canonicalPath(_ path: String) -> String {
        URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath().path
    }

    /// 같은 폴더를 연(또는 여는) 프로젝트 창.
    func projectWindow(forPath path: String) -> ShellProjectWindow? {
        let canonical = Self.canonicalPath(path)
        return projectWindows.first {
            Self.canonicalPath($0.dataSource.rootPath) == canonical
                || $0.dataSource.session.map { Self.canonicalPath($0.rootPath) } == canonical
        }
    }

    func createProject(parentDirectory: String, folderName: String) {
        guard parentDirectory.hasPrefix("/"), !parentDirectory.utf8.contains(0), !folderName.isEmpty,
              folderName != ".", folderName != "..", !folderName.contains("/"), !folderName.utf8.contains(0) else {
            welcomeData.postNotice(.operationFailed(.projectCreationFailed(
                message: String(localized: "Choose a parent folder and enter a name for the new project.")
            )))
            return
        }
        let path = URL(fileURLWithPath: parentDirectory, isDirectory: true)
            .appendingPathComponent(folderName, isDirectory: true).path
        guard welcomeData.currentSnapshot().activity == nil else { return }
        guard !FileManager.default.fileExists(atPath: path) else {
            welcomeData.postNotice(.folderAlreadyExists(path: path))
            return
        }
        guard let pump else {
            welcomeData.postNotice(.operationFailed(.projectCreationFailed(message: ShellProjectOpenFailure.runtimeUnavailable.title)))
            return
        }
        let result = pump.createProject(path: path) { [weak self] event in
            guard let self else { return }
            self.welcomeData.setActivity(nil)
            switch event {
            case .projectCreated(_, let gitState):
                self.openProject(atPath: path, origin: .welcome, createdGitState: gitState)
            case .requestFailed(_, _, let message):
                self.welcomeData.postNotice(.operationFailed(.projectCreationFailed(message: message)))
            default:
                self.welcomeData.postNotice(.operationFailed(.projectCreationFailed(message: ShellProjectOpenFailure.unexpectedResponse.title)))
            }
        }
        switch result {
        case .success(let request):
            welcomeData.setActivity(WelcomeActivity(id: request, kind: .creating, projectName: folderName,
                                                    fractionCompleted: nil, isCancellable: false))
        case .failure(let error):
            let failure = ShellProjectOpenFailure(callStatus: error.status)
            welcomeData.postNotice(.operationFailed(.projectCreationFailed(message: failure.message(path: path))))
        }
    }

    /// 프로젝트 폴더를 엽니다. 코어가 열기 분류·안전 probe를 마친 뒤에만 창을 만듭니다.
    func openProject(atPath rawPath: String, origin: OpenOrigin, createdGitState: UInt32? = nil) {
        let path = Self.canonicalPath(rawPath)
        if let existing = projectWindow(forPath: path) {
            if presentsWindows {
                existing.windowController.showWindow(nil)
                existing.window?.makeKeyAndOrderFront(nil)
            }
            return
        }
        guard !pendingOpenPaths.contains(path) else { return }
        guard let pump else {
            presentOpenFailure(.runtimeUnavailable, path: path, origin: origin)
            return
        }
        let result = pump.openProject(path: path) { [weak self] event in
            self?.finishOpening(path: path, origin: origin, event: event, createdGitState: createdGitState)
        }
        switch result {
        case .success(let request):
            pendingOpenPaths.insert(path)
            if origin == .welcome {
                // 진행 표시의 작업 식별자는 코어 열기 요청 id입니다(열기는 취소할 수 없음).
                welcomeData.setActivity(WelcomeActivity(id: request, kind: .opening,
                                                        projectName: ShellProjectFrameDataSource.folderName(path),
                                                        fractionCompleted: nil, isCancellable: false))
            }
        case .failure(let error):
            presentOpenFailure(ShellProjectOpenFailure(callStatus: error.status), path: path, origin: origin)
        }
    }

    private func finishOpening(path: String, origin: OpenOrigin, event: CoreRuntimeEvent, createdGitState: UInt32?) {
        func finish() {
            pendingOpenPaths.remove(path)
            welcomeData.setActivity(nil)
        }
        switch event {
        case .projectOpened(_, let project, let projectID, let name, let root):
            guard let pump else { return finish() }
            let projectSession = ShellProjectSessionHandle(
                project: project, projectID: projectID, name: name, rootPath: root, pump: pump
            )
            // 창 session이 열릴 때까지 열기 중으로 둡니다(같은 폴더 중복 요청 방지 유지).
            openWindowSession(for: projectSession, path: path) { [weak self] result in
                guard let self else { return }
                finish()
                switch result {
                case .success(let windowSession):
                    let projectWindow = self.makeProjectWindow(rootPath: root)
                    projectWindow.didOpen(projectSession, windowSession: windowSession)
                    if createdGitState != nil { projectWindow.fitNewProjectFrame() }
                    self.present(projectWindow)
                    if let state = createdGitState, state != 0 {
                        let message = state == 1
                            ? String(localized: "Set your Git user.name and user.email, then make the first commit in this project.")
                            : String(localized: "Git could not complete the first commit. Check the repository status and commit the project files.")
                        self.alertPresenter(String(localized: "Project Created Without a First Commit"), message, projectWindow.window)
                    }
                case .failure(let failure):
                    self.presentOpenFailure(failure, path: path, origin: origin)
                }
            }
        case .requestFailed(_, let failure, _):
            finish()
            presentOpenFailure(Self.failure(coreCode: failure, path: path), path: path, origin: origin)
        default:
            finish()
            presentOpenFailure(.unexpectedResponse, path: path, origin: origin)
        }
    }

    /// 연 프로젝트 session 아래에서 창 session을 엽니다(XII.2.2). 실패하면 프로젝트 session을 닫고 원인을 넘깁니다.
    private func openWindowSession(
        for projectSession: ShellProjectSessionHandle,
        path: String,
        completion: @escaping (Result<ShellWindowSessionHandle, ShellProjectOpenFailure>) -> Void
    ) {
        guard let pump else {
            projectSession.close()
            completion(.failure(.runtimeUnavailable))
            return
        }
        let result = pump.openWindowSession(project: projectSession.project) { event in
            switch event {
            case .windowSessionOpened(_, let project, let session, let windowID, let documentID, let generation):
                completion(.success(ShellWindowSessionHandle(
                    session: session, project: project, windowID: windowID, documentID: documentID,
                    openedGeneration: generation, pump: pump
                )))
            case .requestFailed(_, let failure, _):
                projectSession.close()
                completion(.failure(Self.failure(coreCode: failure, path: path)))
            default:
                projectSession.close()
                completion(.failure(.unexpectedResponse))
            }
        }
        if case .failure(let error) = result {
            projectSession.close()
            completion(.failure(ShellProjectOpenFailure(callStatus: error.status)))
        }
    }

    /// 창 복원: 저장한 폴더를 열기 중 상태의 창으로 만들고 다시 엽니다. 실패하면 창에 복구 동작을 보여 줍니다.
    func restoreProjectWindow(from state: NSCoder) -> NSWindow? {
        guard let path = state.decodeObject(of: NSString.self, forKey: Self.restorationRootPathKey) as String?,
            !path.isEmpty
        else { return nil }
        if let existing = projectWindow(forPath: path) {
            return existing.window
        }
        // 같은 폴더를 다른 열기가 여는 중이면 그 열기가 창을 만듭니다(같은 폴더 창 하나, 가이드 2.5).
        guard !pendingOpenPaths.contains(Self.canonicalPath(path)) else { return nil }
        let projectWindow = makeProjectWindow(rootPath: path)
        reopen(projectWindow, atPath: path)
        return projectWindow.window
    }

    /// 이미 있는 창에서 폴더를 (다시) 엽니다(복원, Locate Folder…, Try Again).
    ///
    /// 완료 전에 창이 닫히면(또는 같은 창이 다른 요청을 보내면) 늦게 온 결과는 창에 붙이지 않고, 코어가 연 session은
    /// 바로 닫습니다. 창 닫기가 곧 프로젝트 닫기라는 규칙을 열기 중에도 지킵니다.
    func reopen(_ projectWindow: ShellProjectWindow, atPath rawPath: String) {
        let path = Self.canonicalPath(rawPath)
        projectWindow.beginOpening(rootPath: path)
        projectWindow.pendingOpenRequest = nil
        guard let pump else {
            projectWindow.didFail(.runtimeUnavailable, path: path)
            return
        }
        let result = pump.openProject(path: path) { [weak self, weak projectWindow] event in
            guard let self else { return }
            guard let projectWindow, let request = event.request, projectWindow.pendingOpenRequest == request else {
                self.discardLateOpen(event)
                return
            }
            self.finishReopening(projectWindow, path: path, event: event)
        }
        switch result {
        case .success(let request):
            projectWindow.pendingOpenRequest = request
        case .failure(let error):
            projectWindow.didFail(ShellProjectOpenFailure(callStatus: error.status), path: path)
        }
    }

    /// 받을 창이 없는 열기 결과를 버립니다. 코어가 이미 연 session은 남기지 않고 닫습니다.
    private func discardLateOpen(_ event: CoreRuntimeEvent) {
        guard case .projectOpened(_, let project, _, _, _) = event, let pump else { return }
        pump.closeProject(project)
    }

    private func finishReopening(_ projectWindow: ShellProjectWindow, path: String, event: CoreRuntimeEvent) {
        switch event {
        case .projectOpened(_, let project, let projectID, let name, let root):
            guard let pump else { return }
            let projectSession = ShellProjectSessionHandle(
                project: project, projectID: projectID, name: name, rootPath: root, pump: pump
            )
            let openingRequest = event.request
            openWindowSession(for: projectSession, path: path) { [weak projectWindow] result in
                guard let projectWindow, projectWindow.pendingOpenRequest == openingRequest else {
                    // 여는 동안 창이 닫혔으면 연 session을 남기지 않습니다.
                    if case .success(let windowSession) = result { windowSession.close() }
                    projectSession.close()
                    return
                }
                projectWindow.pendingOpenRequest = nil
                switch result {
                case .success(let windowSession):
                    projectWindow.didOpen(projectSession, windowSession: windowSession)
                case .failure(let failure):
                    projectWindow.didFail(failure, path: path)
                }
            }
        case .requestFailed(_, let failure, _):
            projectWindow.pendingOpenRequest = nil
            projectWindow.didFail(Self.failure(coreCode: failure, path: path), path: path)
        default:
            projectWindow.pendingOpenRequest = nil
            projectWindow.didFail(.unexpectedResponse, path: path)
        }
    }

    // MARK: - 실패 안내

    /// 코어 실패 코드를 안내 원인으로 바꿉니다. `NOT_A_REPOSITORY`이면 폴더가 아직 있는지 확인해 "폴더 없음"과
    /// 구분합니다(실패 경로에서만 로컬 stat 한 번).
    static func failure(coreCode: UInt32, path: String) -> ShellProjectOpenFailure {
        ShellProjectOpenFailure(coreFailure: coreCode, itemExists: ShellProjectOpenFailure.itemExists(atPath: path))
    }

    /// 열기 실패를 알립니다. 시작 창에서 연 경우 시작 창 안내(sheet), 그 밖에는 alert입니다.
    private func presentOpenFailure(_ failure: ShellProjectOpenFailure, path: String, origin: OpenOrigin) {
        if origin == .welcome, welcomeController != nil {
            welcomeData.postNotice(failure.welcomeNotice(path: path))
            return
        }
        alertPresenter(failure.title, failure.message(path: path), NSApp.keyWindow)
    }

    // MARK: - 프로젝트 창

    private func makeProjectWindow(rootPath: String) -> ShellProjectWindow {
        let projectWindow = ShellProjectWindow(
            rootPath: rootPath, runtimeHandle: runtimeHandle, configuration: windowConfiguration()
        )
        projectWindow.dataSource.commandHandler = { [weak self, weak projectWindow] id in
            guard let self, let projectWindow else { return }
            self.performFrameCommand(id, in: projectWindow)
        }
        projectWindow.toolsModel.commandHandler = { [weak projectWindow] command in
            if command == .closeProject {
                projectWindow?.window?.performClose(nil)
            } else {
                // Undo·Redo·Save Now는 이 창(key 창)의 창 session으로 갑니다(XII.6.4).
                projectWindow?.performSessionCommand(command)
            }
        }
        projectWindow.coreCommandIDs = coreCommandIDs
        projectWindow.alertPresenter = { [weak self] title, message, window in
            self?.alertPresenter(title, message, window)
        }
        let controller = projectWindow.windowController
        controller.shouldCloseHandler = { [weak self, weak projectWindow] in
            guard let self, let projectWindow else { return true }
            return self.shouldClose(projectWindow)
        }
        controller.willCloseHandler = { [weak self, weak projectWindow] in
            guard let self, let projectWindow else { return }
            self.projectWindowWillClose(projectWindow)
        }
        controller.restorableStateEncoder = { [weak projectWindow] state in
            guard let projectWindow else { return }
            let path = projectWindow.dataSource.session?.rootPath ?? projectWindow.dataSource.rootPath
            state.encode(path as NSString, forKey: Self.restorationRootPathKey)
        }
        projectWindows.append(projectWindow)
        return projectWindow
    }

    private func present(_ projectWindow: ShellProjectWindow) {
        closeWelcome()
        guard presentsWindows else { return }
        projectWindow.windowController.showWindow(nil)
        projectWindow.window?.makeKeyAndOrderFront(nil)
    }

    private func performFrameCommand(_ id: ShellCommandID, in projectWindow: ShellProjectWindow) {
        switch id {
        case ShellProjectCommandID.revealInFinder:
            guard let root = projectWindow.dataSource.session?.rootPath else { return }
            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: root)])
        case ShellProjectCommandID.closeProject:
            projectWindow.window?.performClose(nil)
        case ShellProjectCommandID.retryOpen:
            reopen(projectWindow, atPath: projectWindow.dataSource.rootPath)
        case ShellProjectCommandID.revokeTrust:
            projectWindow.gitSource.updateTrust(false)
        case ShellProjectCommandID.locateFolder:
            folderChooser(projectWindow.window) { [weak self, weak projectWindow] path in
                guard let self, let projectWindow, let path else { return }
                if let other = self.projectWindow(forPath: path), other !== projectWindow {
                    // 같은 폴더를 연 다른 창이 있으면 그 창을 보여 주고 이 창은 닫습니다(새 창 남발 방지, 가이드 2.5).
                    projectWindow.window?.performClose(nil)
                    if self.presentsWindows { other.window?.makeKeyAndOrderFront(nil) }
                    return
                }
                if self.pendingOpenPaths.contains(Self.canonicalPath(path)) {
                    // 같은 폴더를 다른 열기(메뉴·Finder)가 여는 중이면 그 열기가 창을 만듭니다. 이 창은 닫습니다.
                    projectWindow.window?.performClose(nil)
                    return
                }
                self.reopen(projectWindow, atPath: path)
            }
        default:
            break
        }
    }

    /// 활성 에이전트 세션이 있으면 닫기 전에 확인합니다(ADR-0005 결정 4.5).
    private func shouldClose(_ projectWindow: ShellProjectWindow) -> Bool {
        guard projectWindow.window?.attachedSheet == nil else { return false }
        guard !projectWindow.closeConfirmationPending, !projectWindow.isWaitingToClose else { return false }
        guard projectWindow.requiresConfirmationBeforeClosing, !projectWindow.agentCloseConfirmed,
              let window = projectWindow.window else { return projectWindow.shouldCloseAfterSaving() }
        let alert = Self.makeAgentSessionCloseAlert()
        projectWindow.closeConfirmationPending = true
        alert.beginSheetModal(for: window) { [weak self, weak projectWindow] response in
            guard let self, let projectWindow, self.projectWindows.contains(where: { $0 === projectWindow }) else { return }
            projectWindow.closeConfirmationPending = false
            guard response == .alertFirstButtonReturn else { return }
            projectWindow.agentCloseConfirmed = true
            window.performClose(nil)
        }
        return false
    }

    static func makeAgentSessionCloseAlert() -> NSAlert {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = String(
            localized: "Close the Window and Stop the Agent Session?",
            comment: "Alert title shown before closing a window with an active agent session"
        )
        alert.informativeText = String(
            localized: "Changes the agent already applied stay in the project. You can undo them later.",
            comment: "Alert message"
        )
        alert.addButton(withTitle: String(localized: "Close Window", comment: "Destructive alert button"))
        alert.addButton(withTitle: String(localized: "Cancel", comment: "Button"))
        alert.buttons.first?.hasDestructiveAction = true
        // 키보드 기본 동작만으로 확정되지 않게 Return 키를 Cancel에 둡니다(가이드 6.5).
        alert.buttons.first?.keyEquivalent = ""
        alert.buttons.last?.keyEquivalent = "\r"
        return alert
    }

    private func projectWindowWillClose(_ projectWindow: ShellProjectWindow) {
        // 열기 중이면 늦게 올 결과가 이 창에 붙지 않게 하고, 열린 session은 도착할 때 닫습니다(`reopen`).
        projectWindow.pendingOpenRequest = nil
        projectWindow.closeSessions()
        projectWindows.removeAll { $0 === projectWindow }
        // 마지막 프로젝트 창이면 닫히기 전에 시작 창을 띄워 앱이 종료되지 않게 합니다(창 닫기 = 프로젝트 닫기).
        showWelcomeIfNeeded()
    }

    /// 앱 종료 전 확인이 필요한 창이 있는지(활성 에이전트 세션).
    var requiresConfirmationBeforeTerminating: Bool {
        projectWindows.contains { $0.requiresConfirmationBeforeClosing }
    }
}

/// 앱 수준 메뉴 모델: 창이 없어도 동작하는 셸 명령(tools_menus 5.4).
///
/// 프로젝트를 만들거나 열 수 있고 설정 창을 열 수 있습니다.
final class ShellApplicationCommandModel: ToolsMenuModel {
    static let availableCommands: Set<ShellCommand> = [.newProject, .openProject, .settings]

    let snapshot = ToolsMenuSnapshot(
        generation: 1,
        activeTool: .move,
        zoomScale: 1.0,
        enabledCommands: ShellApplicationCommandModel.availableCommands,
        undoActionName: nil,
        redoActionName: nil,
        rulersVisible: false,
        selectionIsLocked: false,
        selectionIsHidden: false
    )
    let observers = ToolsMenuObservers()
    var commandHandler: ((ShellCommand) -> Void)?

    func addSnapshotObserver(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation {
        observers.add(handler)
    }

    func send(_ command: ShellCommand) {
        guard snapshot.enabledCommands.contains(command) else { return }
        commandHandler?(command)
    }
}

/// 시작 창 data source(코어 연결 부분).
///
/// 프로젝트 생성·열기를 코어에 요청합니다. 시작 창 protocol은 모든 호출이 메인 스레드에서 이루어집니다.
final class ShellWelcomeDataSource: WelcomeDataSource {
    weak var coordinator: ShellAppCoordinator?
    var snapshotDidChange: (() -> Void)?
    private var snapshot = WelcomeSnapshot(
        generation: 1,
        recentProjects: .loaded([]),
        git: .checking,
        activity: nil,
        notice: nil,
        defaultParentDirectory: nil
    )
    private var nextNoticeID: UInt64 = 1
    /// 받은 명령(검사용).
    private(set) var sentCommands: [WelcomeCommand] = []

    func currentSnapshot() -> WelcomeSnapshot {
        snapshot
    }

    func send(_ command: WelcomeCommand) {
        sentCommands.append(command)
        switch command {
        case .openProject(let directory):
            MainActor.assumeIsolated {
                coordinator?.openProject(atPath: directory, origin: .welcome)
            }
        case .createProject(let parentDirectory, let folderName):
            MainActor.assumeIsolated {
                coordinator?.createProject(parentDirectory: parentDirectory, folderName: folderName)
            }
        case .cloneProject:
            postNotice(.operationFailed(.projectCloneUnavailable))
        case .dismissNotice(let id):
            guard snapshot.notice?.id == id else { return }
            update { $0.notice = nil }
        case .openRecent, .removeRecent, .reloadRecentProjects, .recheckGit, .useGitExecutable, .cancelActivity:
            // 최근 목록·Git 탐색 코어 경로가 없고, 열기 요청은 취소할 수 없습니다(isCancellable == false).
            break
        }
    }

    func setActivity(_ activity: WelcomeActivity?) {
        guard activity != snapshot.activity else { return }
        update { $0.activity = activity }
    }

    func postNotice(_ kind: WelcomeNotice.Kind) {
        let notice = WelcomeNotice(id: nextNoticeID, kind: kind)
        nextNoticeID += 1
        update { $0.notice = notice }
    }

    private func update(_ change: (inout WelcomeSnapshot) -> Void) {
        change(&snapshot)
        snapshot.generation += 1
        snapshotDidChange?()
    }
}

/// `MainMenuBuilder` 메뉴에 셸 프레임 명령 항목을 끼웁니다(가이드 6.1 "툴바의 모든 명령은 메뉴 막대에도").
///
/// - View: 사이드바·인스펙터 묶음 뒤에 보조 패널(Show/Hide Panels), 그다음 묶음에 Back·Forward
/// - Edit ▸ Find: Search Project…
/// - File: Save Now 뒤에 Show in Finder(더 보기 메뉴와 같은 동작)
@MainActor
enum ShellMainMenuInstaller {
    /// 보조 패널 이름(`ShellPanelTabsSlot.paneTitle`과 같은 값).
    static var auxiliaryPanelTitle: String {
        String(localized: "Panels", comment: "Name of the area below the canvas; used in Show/Hide Panels")
    }

    static func install(into mainMenu: MainMenuBuilder.MainMenu) {
        let menu = mainMenu.menu
        if let view = submenu(titled: "View", in: menu),
            let inspectorIndex = view.items.firstIndex(where: { $0.action == #selector(NSSplitViewController.toggleInspector(_:)) })
        {
            if let auxiliary = ShellFrameMenuItems.makeViewMenuItems(auxiliaryTitle: auxiliaryPanelTitle).last {
                auxiliary.image = NSImage(systemSymbolName: "rectangle.bottomthird.inset.filled", accessibilityDescription: nil)
                view.insertItem(auxiliary, at: inspectorIndex + 1)
            }
            let navigation = ShellFrameMenuItems.makeNavigationMenuItems()
            var index = inspectorIndex + 2
            view.insertItem(.separator(), at: index)
            for (item, symbol) in zip(navigation.prefix(2), [ShellFrameSymbols.navigateBack, ShellFrameSymbols.navigateForward]) {
                index += 1
                item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
                view.insertItem(item, at: index)
            }
            if let find = submenu(titled: "Edit", in: menu).flatMap({ submenu(titled: "Find", in: $0) }),
                let search = navigation.last
            {
                find.addItem(.separator())
                search.image = NSImage(systemSymbolName: "text.magnifyingglass", accessibilityDescription: nil)
                find.addItem(search)
            }
        }
        if let file = submenu(titled: "File", in: menu) {
            let reveal = ShellFrameMenuItems.makeFrameActionItem(
                title: String(localized: "Show in Finder", comment: "File menu item"),
                id: ShellProjectCommandID.revealInFinder
            )
            reveal.image = NSImage(systemSymbolName: "folder", accessibilityDescription: nil)
            file.addItem(.separator())
            file.addItem(reveal)
        }
    }

    static func submenu(titled title: String, in menu: NSMenu) -> NSMenu? {
        menu.items.first { $0.submenu?.title == title }?.submenu
    }
}
