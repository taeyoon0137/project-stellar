import AppKit

// 프로젝트 창 하나의 구성: 코어 프로젝트 session handle, 셸 프레임 data source, 창별 도구 모델, 영역 슬롯, 툴바·메뉴
// 라우팅을 묶습니다. 앱 coordinator(`ShellAppCoordinator`)가 프로젝트를 열 때 만듭니다.

/// 창이 소유하는 코어 프로젝트 session handle(`stellar_project_open` 결과).
///
/// 창 하나가 session 하나를 소유하고, 창이 닫히면 `close()`로 한 번만 닫습니다(`stellar_project_close`).
/// 창별 문서 state·view state·history는 이 프로젝트 session 아래의 창 session(`ShellWindowSessionHandle`)이 가리킵니다.
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

/// 창 controller가 소유하는 코어 창 session handle(`stellar_window_session_open` 결과, XII.2.2).
///
/// 코어의 `ProjectWindowSession`(XII.2.1) 하나를 가리킵니다: 이 창만의 작업 문서 state, view state(선택), history,
/// 창을 열 때 읽은 generation. 창 controller(`ShellWindowController.windowSession`) 하나가 handle 하나를 소유하고, 창이
/// 닫히면 `close()`로 한 번만 닫습니다(`stellar_window_session_close`). 닫으면 저장하지 않은 편집도 폐기합니다.
@MainActor
final class ShellWindowSessionHandle {
    let session: UInt64
    let project: UInt64
    let windowID: UUID
    /// 작업 문서 id. 저장한 적 없는 프로젝트는 nil입니다.
    let documentID: UUID?
    /// 창을 열 때 읽은 generation 번호(저장한 적 없으면 0).
    let openedGeneration: UInt64
    private let pump: ShellRuntimeEventPump
    private(set) var isClosed = false

    init(
        session: UInt64, project: UInt64, windowID: UUID, documentID: UUID?, openedGeneration: UInt64,
        pump: ShellRuntimeEventPump
    ) {
        self.session = session
        self.project = project
        self.windowID = windowID
        self.documentID = documentID
        self.openedGeneration = openedGeneration
        self.pump = pump
    }

    /// 코어 상태 snapshot의 최신 창 session 정보. 닫혔으면 nil입니다(document thread를 기다리지 않음).
    var info: StellarWindowSessionInfo? {
        guard !isClosed else { return nil }
        return CoreRuntime.windowSessionInfo(handle: pump.runtimeHandle, session: session)
    }

    /// 코어 상태 snapshot의 최신 창 view 정보(탭 수, active view, viewport, XII.4.3). 닫혔으면 nil입니다.
    var viewInfo: StellarWindowViewInfo? {
        guard !isClosed else { return nil }
        return CoreRuntime.windowViewInfo(handle: pump.runtimeHandle, session: session)
    }

    /// 창 session을 닫습니다. 여러 번 불러도 한 번만 요청합니다.
    func close(completion: ((CoreRuntimeEvent) -> Void)? = nil) {
        guard !isClosed else { return }
        isClosed = true
        pump.closeWindowSession(session, completion: completion)
    }

    /// 이 창 session에 코어 명령 코드를 보냅니다(XII.6.4). 닫혔거나 요청을 받지 않으면 false입니다.
    @discardableResult
    func perform(coreCommand code: UInt32, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed else { return false }
        if case .success = pump.perform(session: session, command: code, completion: completion) {
            return true
        }
        return false
    }

    /// 이 창 session의 선택을 바꿉니다(그 창 history에 기록). 닫혔거나 요청을 받지 않으면 false입니다.
    @discardableResult
    func select(_ ids: [UUID], completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed else { return false }
        if case .success = pump.select(session: session, ids: ids, completion: completion) {
            return true
        }
        return false
    }
}

extension ShellCommandCoreIDs {
    /// 코어에 공유 이름으로 물어 만든 command ID 표(XII.6.5). 앱 구성이 한 번 만들어 프로젝트 창에 넘깁니다.
    static func fromCore() -> ShellCommandCoreIDs {
        resolve { CoreRuntime.commandCode(name: $0) }
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
/// 열기 중(loading) → ready, 또는 실패(failed, 복구 동작 포함)입니다. 도구 목록과 현재 도구는 창 도구 모델에서 받아
/// 하단 도구 막대에 보내고(`setTools`), 도구 막대 선택은 `toolSelectionHandler`로 창 도구 모델에 넘깁니다.
@MainActor
final class ShellProjectFrameDataSource: ShellFrameDataSource {
    private(set) var currentSnapshot: ShellFrameSnapshot
    var snapshotHandler: ((ShellFrameSnapshot) -> Void)?
    /// 셸 프레임 명령을 받는 쪽(프로젝트 창). 명령 ID 해석은 창이 합니다.
    var commandHandler: ((ShellCommandID) -> Void)?
    /// 하단 도구 막대에서 고른 도구의 명령 ID를 받는 쪽(프로젝트 창이 창 도구 모델로 보냅니다).
    var toolSelectionHandler: ((ShellCommandID) -> Void)?
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

    /// 하단 도구 막대의 도구 목록과 현재 도구를 바꿉니다. 바뀐 것이 없으면 snapshot을 보내지 않습니다.
    func setTools(_ tools: [ShellToolDescriptor], activeToolID: ShellCommandID?) {
        guard tools != currentSnapshot.tools || activeToolID != currentSnapshot.activeToolID else { return }
        var next = currentSnapshot
        next.tools = tools
        next.activeToolID = activeToolID
        publish(next)
    }

    func send(_ command: ShellFrameCommand) {
        switch command {
        case .perform(let id):
            commandHandler?(id)
        case .selectTool(let id):
            toolSelectionHandler?(id)
        case .navigateBack, .navigateForward, .submitSearch:
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
/// 코어 도구 상태 기계(XII.3.6)가 없으므로, 창별 현재 도구만 셸이 보관합니다. 제한:
/// - 실행 가능한 명령은 도구 선택 11개, Close Project, 그리고 창 session 상태에 따른 Undo·Redo·Save Now(XII.6.4)와
///   Zoom In·Zoom Out·Actual Size(XII.4.3)입니다. Undo는 그 창 undo stack이, Redo는 redo stack이 비어 있지 않을 때,
///   Save Now는 창 session에 작업 문서가 있을 때, 줌은 active view가 있고 배율이 경계(2%·25600%)에 닿지 않았을 때
///   활성입니다(`setSessionState`, 코어 command ID가 있는 명령만). 배율 표시는 코어 viewport 값입니다. 편집·객체 명령과
///   Zoom to Fit·Zoom to Selection은 코어 경로가 없어 숨기지 않고 비활성입니다.
/// - 프로젝트가 ready가 아니면 도구 선택도 비활성입니다.
/// - 도구 선택은 코어로 보내지 않습니다. 다른 명령은 `commandHandler`(프로젝트 창)로 넘깁니다.
final class ShellWindowToolsModelStub: ToolsMenuModel {
    /// ready 상태에서 항상 실행할 수 있는 명령(창 session 상태와 무관).
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

    /// 지금 활성인 창 session 명령(ready일 때만 반영).
    private(set) var sessionCommands: Set<ShellCommand> = []
    private var isReady = false

    /// 프로젝트 상태에 맞춰 실행 가능한 명령을 바꿉니다. ready가 아니면 창 session 명령도 비활성입니다.
    func setProjectReady(_ ready: Bool) {
        isReady = ready
        if !ready { sessionCommands = [] }
        republishEnabledCommands()
    }

    /// 코어 viewport 배율 경계(`stellar-commands` `MIN_ZOOM`·`MAX_ZOOM`과 같은 값).
    static let minimumZoomScale = 0.02
    static let maximumZoomScale = 256.0

    /// 창 session·view 정보로 실행할 수 있는 창 session 명령을 계산합니다(XII.6.4, XII.4.3). 정보가 없으면 비어 있습니다.
    static func sessionCommands(info: StellarWindowSessionInfo?, view: StellarWindowViewInfo?) -> Set<ShellCommand> {
        var commands: Set<ShellCommand> = []
        if let info {
            if info.undo_depth > 0 { commands.insert(.undo) }
            if info.redo_depth > 0 { commands.insert(.redo) }
            if info.has_document != 0 { commands.insert(.saveNow) }
        }
        if let view, view.has_view != 0 {
            if view.zoom < maximumZoomScale { commands.insert(.zoomIn) }
            if view.zoom > minimumZoomScale { commands.insert(.zoomOut) }
            if view.zoom != 1.0 { commands.insert(.zoomToActualSize) }
        }
        return commands
    }

    /// 창 session 명령 활성 상태와 배율 표시를 바꿉니다. `zoomScale`이 nil이면 마지막 배율 표시를 유지합니다.
    func setSessionState(commands: Set<ShellCommand>, zoomScale: Double?) {
        sessionCommands = commands
        if let zoomScale, zoomScale != snapshot.zoomScale {
            var next = snapshot
            next.zoomScale = zoomScale
            publish(next)
        }
        republishEnabledCommands()
    }

    private func republishEnabledCommands() {
        let commands = isReady ? Self.readyCommands.union(sessionCommands) : Self.notReadyCommands
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
    /// 창 도구 모델 → 하단 도구 막대(셸 프레임 snapshot 도구 목록) 구독.
    private var toolsObservation: ToolsMenuObservation?

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
        // 캔버스 도구는 상단 툴바가 아니라 캔버스 하단 가운데 도구 막대에 둡니다(shell_frame 6.1, 2026-11-14 사용자 결정).
        // 창 도구 모델의 도구 목록·현재 도구를 셸 프레임 snapshot으로 보내고, 도구 막대 선택은 router로 보냅니다.
        toolsObservation = ShellProjectWindow.connectToolBar(dataSource: dataSource, toolsModel: toolsModel, router: router)
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

    /// 창 도구 모델과 하단 도구 막대를 잇습니다. 도구 목록은 `CanvasTool` 전체, 선택 표시는 창 도구 모델의 현재
    /// 도구이고, 막대 활성 상태는 셸 프레임 상태(ready)로 정해집니다. 돌려준 구독 토큰을 창이 보관합니다.
    private static func connectToolBar(
        dataSource: ShellProjectFrameDataSource,
        toolsModel: ShellWindowToolsModelStub,
        router: ShellCommandRouter
    ) -> ToolsMenuObservation {
        let tools = CanvasTool.allCases.map(\.shellToolDescriptor)
        dataSource.setTools(tools, activeToolID: toolsModel.snapshot.activeTool.shellToolDescriptor.id)
        dataSource.toolSelectionHandler = { [weak router] id in
            guard let router, let tool = ShellCommand(rawValue: id.rawValue)?.canvasTool else { return }
            _ = router.dispatch(tool.command)
        }
        return toolsModel.addSnapshotObserver { [weak dataSource] snapshot in
            dataSource?.setTools(tools, activeToolID: snapshot.activeTool.shellToolDescriptor.id)
        }
    }

    /// 창을 닫기 전에 확인이 필요한지(활성 에이전트 세션, ADR-0005 결정 4.5).
    var requiresConfirmationBeforeClosing: Bool {
        panelsSlot.requiresConfirmationBeforeClosing
    }

    /// 코어가 프로젝트와 이 창의 창 session을 열었습니다. 창 controller가 창 session handle을 소유합니다(XII.2.2).
    func didOpen(_ session: ShellProjectSessionHandle, windowSession: ShellWindowSessionHandle) {
        windowController.windowSession?.close()
        windowController.windowSession = windowSession
        dataSource.didOpen(session)
        toolsModel.setProjectReady(true)
        refreshSessionCommands()
        window?.invalidateRestorableState()
    }

    /// 저장 실패를 알리는 곳(창, 제목, 메시지). coordinator가 앱 alert 경로를 넣습니다.
    var alertPresenter: ((_ title: String, _ message: String, _ window: NSWindow?) -> Void)?

    /// 창 session 명령(Undo·Redo·Save Now)이 끝날 때마다 받은 완료 event(검사용).
    private(set) var lastSessionCommandEvent: CoreRuntimeEvent?

    /// 창 session 정보로 Undo·Redo·Save Now 활성 상태를 다시 계산합니다.
    func refreshSessionCommands() {
        let session = windowController.windowSession
        let view = session?.viewInfo
        let commands = ShellWindowToolsModelStub.sessionCommands(info: session?.info, view: view)
            .filter { coreCommandIDs.coreCommandID(for: $0) != nil }
        toolsModel.setSessionState(commands: commands, zoomScale: view.flatMap { $0.has_view != 0 ? $0.zoom : nil })
    }

    /// 코어 command ID 표(XII.6.5). 코어가 공유 이름으로 알려 준 ID로만 명령을 보냅니다.
    var coreCommandIDs = ShellCommandCoreIDs.empty

    /// 코어 명령(Undo·Redo·Save Now)을 이 창의 창 session(활성 창 session)에 보냅니다(XII.6.4, XII.6.5). 코어 ID가 없는
    /// 명령이면 false입니다.
    ///
    /// 메뉴·단축키는 응답자 체인으로 key 창의 router가 받으므로 여기 오는 명령은 언제나 이 창(활성 창)의 것입니다.
    @discardableResult
    func performSessionCommand(_ command: ShellCommand) -> Bool {
        guard let code = coreCommandIDs.coreCommandID(for: command), let session = windowController.windowSession else {
            return false
        }
        return session.perform(coreCommand: code) { [weak self] event in
            self?.finishSessionCommand(command, event: event)
        }
    }

    /// 이 창 session의 선택을 바꿉니다(그 창 history에 기록, 끝나면 명령 활성 상태 갱신). 캔버스 선택 도구가 생기기 전에는
    /// 검사만 부릅니다.
    @discardableResult
    func selectNodes(_ ids: [UUID]) -> Bool {
        guard let session = windowController.windowSession else { return false }
        return session.select(ids) { [weak self] _ in
            self?.refreshSessionCommands()
        }
    }

    private func finishSessionCommand(_ command: ShellCommand, event: CoreRuntimeEvent) {
        lastSessionCommandEvent = event
        refreshSessionCommands()
        guard command == .saveNow, case .requestFailed(_, let failure, _) = event else { return }
        let stale = Int32(bitPattern: failure) == STELLAR_FAILURE_SAVE_STALE
        alertPresenter?(
            String(localized: "The Design Couldn’t Be Saved", comment: "Save Now failure alert title"),
            stale
                ? String(
                    localized: "This project was saved from another window or app after this window opened it. This window’s changes weren’t saved.",
                    comment: "Save Now failure: the project changed on disk since the window opened it"
                )
                : String(
                    localized: "Stellar couldn’t write the design files. Make sure the project folder is available and you can write to it, then choose Save Now again.",
                    comment: "Save Now failure: writing failed"
                ),
            window
        )
    }

    /// 열기에 실패했습니다. 창에 원인과 복구 동작을 보여 줍니다.
    func didFail(_ failure: ShellProjectOpenFailure, path: String) {
        windowController.windowSession?.close()
        windowController.windowSession = nil
        dataSource.didFail(title: failure.title, message: failure.message(path: path))
        toolsModel.setProjectReady(false)
        toolsModel.setSessionState(commands: [], zoomScale: nil)
    }

    /// 창을 닫습니다: 창 session을 먼저, 그다음 프로젝트 session을 한 번씩 닫습니다.
    func closeSessions() {
        windowController.windowSession?.close()
        dataSource.session?.close()
    }

    /// 다른 폴더로 다시 엽니다.
    func beginOpening(rootPath: String) {
        closeSessions()
        windowController.windowSession = nil
        dataSource.beginOpening(rootPath: rootPath)
        toolsModel.setProjectReady(false)
        window?.invalidateRestorableState()
    }
}

extension CanvasTool {
    /// 하단 도구 막대에 보낼 셸 프레임 도구 항목. 명령 ID는 셸 명령 ID(예: `tool.move`)입니다.
    var shellToolDescriptor: ShellToolDescriptor {
        ShellToolDescriptor(id: ShellCommandID(command.rawValue), label: label, symbolName: symbolName)
    }
}
