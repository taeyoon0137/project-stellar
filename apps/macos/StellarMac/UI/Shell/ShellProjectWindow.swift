import AppKit

// 프로젝트 창 하나의 구성: 코어 프로젝트 session handle, 셸 프레임 data source, 창별 도구 모델, 영역 슬롯, 툴바·메뉴
// 라우팅을 묶습니다. 앱 coordinator(`ShellAppCoordinator`)가 프로젝트를 열 때 만듭니다.

/// 창이 소유하는 코어 프로젝트 session handle(`stellar_project_open` 결과).
///
/// 창 하나가 session 하나를 소유하고, 창이 닫히면 `close()`로 한 번만 닫습니다(`stellar_project_close`).
/// 코어의 `ProjectWindowSession`(XII.2.1)은 아직 없으므로 이 handle은 프로젝트 session만 가리킵니다. 창별 문서
/// state·view state·history는 코어 타입이 생기면 이 handle 자리에 연결합니다(XII.2.2, XII.2.5).
@MainActor
final class ShellProjectSessionHandle {
    let project: UInt64
    let projectID: UUID
    let name: String
    let rootPath: String
    private let pump: ShellRuntimeEventPump
    private(set) var isClosed = false

    init(project: UInt64, projectID: UUID, name: String, rootPath: String, pump: ShellRuntimeEventPump) {
        self.project = project
        self.projectID = projectID
        self.name = name
        self.rootPath = rootPath
        self.pump = pump
    }

    /// session을 닫습니다. 여러 번 불러도 한 번만 요청합니다.
    func close(completion: ((CoreRuntimeEvent) -> Void)? = nil) {
        guard !isClosed else { return }
        isClosed = true
        pump.closeProject(project, completion: completion)
    }
}

/// 프로젝트 창의 셸 프레임 명령 ID(더 보기 메뉴·상태 화면 버튼).
enum ShellProjectCommandID {
    static let revealInFinder = ShellCommandID("project.revealInFinder")
    static let closeProject = ShellCommandID("project.close")
    static let locateFolder = ShellCommandID("project.locate")
    static let retryOpen = ShellCommandID("project.retryOpen")
}

/// 프로젝트 창의 셸 프레임 data source.
///
/// 제목은 프로젝트 이름, 부제목은 신뢰 상태(코어가 모든 프로젝트를 신뢰되지 않음으로 열므로 "Restricted Mode",
/// 가이드 7.2)입니다. 프로젝트 내비게이션·검색 코어 경로가 없으므로 뒤로·앞으로와 검색은 비활성입니다. 상태는
/// 열기 중(loading) → ready, 또는 실패(failed, 복구 동작 포함)입니다.
@MainActor
final class ShellProjectFrameDataSource: ShellFrameDataSource {
    private(set) var currentSnapshot: ShellFrameSnapshot
    var snapshotHandler: ((ShellFrameSnapshot) -> Void)?
    /// 셸 프레임 명령을 받는 쪽(프로젝트 창). 명령 ID 해석은 창이 합니다.
    var commandHandler: ((ShellCommandID) -> Void)?
    /// 지금 연 session. 열기 전·실패 시 nil입니다.
    private(set) var session: ShellProjectSessionHandle?
    /// 이 창이 여는(또는 연) 프로젝트 폴더.
    private(set) var rootPath: String

    init(rootPath: String) {
        self.rootPath = rootPath
        currentSnapshot = ShellFrameSnapshot(
            generation: 1,
            title: Self.folderName(rootPath),
            subtitle: "",
            status: Self.loadingStatus(rootPath),
            navigation: ShellNavigationState(canGoBack: false, canGoForward: false),
            tools: [],
            activeToolID: nil,
            moreActions: [],
            searchPlaceholder: ShellFrameStrings.searchProject,
            isSearchAvailable: false
        )
    }

    /// 경로의 마지막 구성 요소(폴더 이름). 화면에는 전체 경로 대신 이 이름만 보여 줍니다.
    nonisolated static func folderName(_ path: String) -> String {
        let name = (path as NSString).lastPathComponent
        return name.isEmpty ? path : name
    }

    static func loadingStatus(_ path: String) -> ShellFrameStatus {
        .loading(message: String(
            localized: "Opening “\(folderName(path))”…",
            comment: "Window status while a project opens; the argument is the folder name"
        ))
    }

    /// 다른 폴더를 다시 엽니다(복구 동작 Locate Folder…). 열기 중 상태로 바꿉니다.
    func beginOpening(rootPath: String) {
        self.rootPath = rootPath
        var next = currentSnapshot
        next.title = Self.folderName(rootPath)
        next.subtitle = ""
        next.status = Self.loadingStatus(rootPath)
        next.moreActions = []
        publish(next)
    }

    /// 코어가 프로젝트를 열었습니다.
    func didOpen(_ session: ShellProjectSessionHandle) {
        self.session = session
        rootPath = session.rootPath
        var next = currentSnapshot
        next.title = session.name
        next.subtitle = String(localized: "Restricted Mode", comment: "Window subtitle: the project isn't trusted yet")
        next.status = .ready
        next.moreActions = [
            ShellActionDescriptor(
                id: ShellProjectCommandID.revealInFinder,
                title: String(localized: "Show in Finder", comment: "More menu item")
            ),
            ShellActionDescriptor(
                id: ShellProjectCommandID.closeProject,
                title: String(localized: "Close Project", comment: "More menu item")
            ),
        ]
        publish(next)
    }

    /// 열기에 실패했습니다. 다음에 할 일(폴더 다시 찾기, 다시 시도, 창 닫기)을 함께 둡니다(가이드 7.1).
    func didFail(title: String, message: String) {
        session = nil
        var next = currentSnapshot
        next.status = .failed(title: title, message: message, recoveryActions: [
            ShellActionDescriptor(
                id: ShellProjectCommandID.locateFolder,
                title: String(localized: "Locate Folder…", comment: "Recovery button"),
                isPrimary: true
            ),
            ShellActionDescriptor(
                id: ShellProjectCommandID.retryOpen,
                title: String(localized: "Try Again", comment: "Recovery button")
            ),
            ShellActionDescriptor(
                id: ShellProjectCommandID.closeProject,
                title: String(localized: "Close Window", comment: "Recovery button")
            ),
        ])
        next.moreActions = []
        publish(next)
    }

    func send(_ command: ShellFrameCommand) {
        switch command {
        case .perform(let id):
            commandHandler?(id)
        case .navigateBack, .navigateForward, .selectTool, .submitSearch:
            // 코어 내비게이션·검색 경로가 아직 없습니다. 해당 항목은 snapshot으로 비활성화되어 있습니다.
            break
        }
    }

    private func publish(_ snapshot: ShellFrameSnapshot) {
        var next = snapshot
        next.generation = currentSnapshot.generation + 1
        currentSnapshot = next
        snapshotHandler?(next)
    }
}

/// 프로젝트 창의 도구·메뉴 모델(의도적 placeholder, 계획 1.2 규칙 6).
///
/// 코어 도구 상태 기계(XII.3.6)와 command ID 체계(XII.6.5)가 없으므로, 창별 현재 도구만 셸이 보관합니다. 제한:
/// - 실행 가능한 명령은 도구 선택 11개와 Close Project뿐입니다. Undo·Redo·Save Now(XII.6.4), 편집·객체 명령, 줌
///   (camera, XII.4.3)은 코어 경로가 없어 숨기지 않고 비활성으로 둡니다.
/// - 프로젝트가 ready가 아니면 도구 선택도 비활성입니다.
/// - 도구 선택은 코어로 보내지 않습니다. 다른 명령은 `commandHandler`(프로젝트 창)로 넘깁니다.
final class ShellWindowToolsModelStub: ToolsMenuModel {
    /// ready 상태에서 실행할 수 있는 명령.
    static let readyCommands: Set<ShellCommand> = Set(CanvasTool.allCases.map(\.command)).union([.closeProject])
    /// ready가 아닐 때(열기 중·실패) 실행할 수 있는 명령.
    static let notReadyCommands: Set<ShellCommand> = [.closeProject]

    private(set) var snapshot: ToolsMenuSnapshot
    let observers = ToolsMenuObservers()
    /// 도구 선택 외 명령을 받는 쪽.
    var commandHandler: ((ShellCommand) -> Void)?

    init() {
        snapshot = ToolsMenuSnapshot(
            generation: 1,
            activeTool: .move,
            zoomScale: 1.0,
            enabledCommands: Self.notReadyCommands,
            undoActionName: nil,
            redoActionName: nil,
            rulersVisible: false,
            selectionIsLocked: false,
            selectionIsHidden: false
        )
    }

    func addSnapshotObserver(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation {
        observers.add(handler)
    }

    func send(_ command: ShellCommand) {
        guard snapshot.enabledCommands.contains(command) else { return }
        if let tool = command.canvasTool {
            guard tool != snapshot.activeTool else { return }
            var next = snapshot
            next.activeTool = tool
            publish(next)
            return
        }
        commandHandler?(command)
    }

    /// 프로젝트 상태에 맞춰 실행 가능한 명령을 바꿉니다.
    func setProjectReady(_ ready: Bool) {
        let commands = ready ? Self.readyCommands : Self.notReadyCommands
        guard commands != snapshot.enabledCommands else { return }
        var next = snapshot
        next.enabledCommands = commands
        publish(next)
    }

    private func publish(_ next: ToolsMenuSnapshot) {
        var next = next
        next.generation = snapshot.generation + 1
        snapshot = next
        observers.notify(next)
    }
}

/// 프로젝트 창 하나의 구성 요소 묶음.
@MainActor
final class ShellProjectWindow {
    let windowController: ShellWindowController
    let dataSource: ShellProjectFrameDataSource
    let toolsModel: ShellWindowToolsModelStub
    let router: ShellCommandRouter
    let toolsToolbar: ToolsToolbarController
    let canvasHost: ShellCanvasHostViewController
    let sidebarSlot: ShellSidebarAreaSlot
    let inspectorSlot: ShellInspectorAreaSlot
    let panelsSlot: ShellPanelTabsSlot

    var window: NSWindow? { windowController.window }

    /// 이 창이 완료를 기다리는 코어 열기 요청 id(복원·Try Again·Locate Folder…). 창이 닫히면 nil로 지웁니다.
    /// 완료 event의 요청 id가 이 값과 다르면(창이 닫혔거나 다른 요청으로 바뀜) 결과를 이 창에 붙이지 않습니다.
    var pendingOpenRequest: UInt64?

    /// - Parameters:
    ///   - rootPath: 열 프로젝트 폴더(절대 경로). 창은 열기 중 상태로 시작합니다.
    ///   - runtimeHandle: 캔버스 drawable·입력을 받을 코어 runtime handle.
    init(rootPath: String, runtimeHandle: UInt64?, configuration: ShellWindowConfiguration = .standard) {
        dataSource = ShellProjectFrameDataSource(rootPath: rootPath)
        toolsModel = ShellWindowToolsModelStub()
        router = ShellCommandRouter(model: toolsModel)
        toolsToolbar = ToolsToolbarController(model: toolsModel, router: router)
        canvasHost = ShellCanvasHostViewController(runtimeHandle: runtimeHandle)
        sidebarSlot = ShellSidebarAreaSlot(
            sidebar: SidebarViewController(provider: ShellAreaDataStubs.makeSidebarProvider())
        )
        inspectorSlot = ShellInspectorAreaSlot(
            inspector: InspectorViewController(dataSource: ShellAreaDataStubs.makeInspectorDataSource())
        )
        panelsSlot = ShellPanelTabsSlot(
            gitPanel: GitPanelViewController(dataSource: ShellAreaDataStubs.makeGitDataSource()),
            agentPanel: AgentPanelViewController(source: ShellAreaDataStubs.makeAgentSource())
        )
        windowController = ShellWindowController(
            configuration: configuration,
            dataSource: dataSource,
            slots: ShellFrameSlots(
                sidebar: sidebarSlot, content: canvasHost, inspector: inspectorSlot, auxiliary: panelsSlot
            ),
            toolbarItemProvider: toolsToolbar
        )
        if let window = windowController.window {
            // 응답자 체인: content view → router → split controller → 창. 텍스트 편집 중에는 field editor가 먼저 받습니다.
            router.install(in: window)
        }
        // 캔버스 포커스 중 단일 키 도구 단축키(tools_menus 6.2). 도구로 해석되지 않거나 비활성이면 코어 입력으로 보냅니다.
        canvasHost.canvasView.keyDownHandler = { [weak router] event in
            guard let router, let tool = CanvasToolShortcut.tool(for: event) else { return false }
            return router.dispatch(tool.command)
        }
        canvasHost.canvasView.setAccessibilityHelp(String(
            localized: "Press V, F, R, O, L, P, T, H, or Z to choose a tool.",
            comment: "Accessibility help of the canvas"
        ))
    }

    /// 창을 닫기 전에 확인이 필요한지(활성 에이전트 세션, ADR-0005 결정 4.5).
    var requiresConfirmationBeforeClosing: Bool {
        panelsSlot.requiresConfirmationBeforeClosing
    }

    /// 코어가 프로젝트를 열었습니다.
    func didOpen(_ session: ShellProjectSessionHandle) {
        dataSource.didOpen(session)
        toolsModel.setProjectReady(true)
        window?.invalidateRestorableState()
    }

    /// 열기에 실패했습니다. 창에 원인과 복구 동작을 보여 줍니다.
    func didFail(_ failure: ShellProjectOpenFailure, path: String) {
        dataSource.didFail(title: failure.title, message: failure.message(path: path))
        toolsModel.setProjectReady(false)
    }

    /// 다른 폴더로 다시 엽니다.
    func beginOpening(rootPath: String) {
        dataSource.beginOpening(rootPath: rootPath)
        toolsModel.setProjectReady(false)
        window?.invalidateRestorableState()
    }
}
