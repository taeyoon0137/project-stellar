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
    let pump: ShellRuntimeEventPump
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
    private(set) var pendingCommandCount = 0
    var isClosing = false
    var commandsDidFinish: (() -> Void)?
    var checkingComponentsOnReturn = false
    var recheckComponentsOnReturn = false

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
        commandsDidFinish = nil
        pump.closeWindowSession(session, completion: completion)
    }

    var selectionTarget: StellarSelectionTarget? {
        guard !isClosed else { return nil }
        return CoreRuntime.selectionTarget(handle: pump.runtimeHandle, session: session)
    }

    func editSelection(_ target: StellarSelectionTarget, command: ShellCommand, ids: [UUID]?,
                       completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing, target.session == session else { return false }
        let action: UInt32
        switch command {
        case .delete: action = UInt32(STELLAR_SELECTION_DELETE)
        case .duplicate: action = UInt32(STELLAR_SELECTION_DUPLICATE)
        case .bringForward: action = UInt32(STELLAR_SELECTION_FORWARD)
        case .sendBackward: action = UInt32(STELLAR_SELECTION_BACKWARD)
        case .bringToFront: action = UInt32(STELLAR_SELECTION_FRONT)
        case .sendToBack: action = UInt32(STELLAR_SELECTION_BACK)
        default: return false
        }
        return track({ pump.editSelection(target: target, action: action, ids: ids, completion: $0) }, completion: completion)
    }

    func pasteSubtree(_ data: Data, target: StellarSelectionTarget, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing, target.session == session else { return false }
        return track({ pump.pasteSubtree(target: target, data: data, completion: $0) }, completion: completion)
    }

    func insertShape(parent: CanvasFrameHit, kind: InspectorShapeKind, start: CGPoint, end: CGPoint,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        return track { done in pump.insertShape(session: session, parent: parent, kind: kind, start: start, end: end,
                                                completion: done) } completion: { completion($0) }
    }

    func insertFrame(parent: CanvasFrameHit, rect: CGRect, rectangle: Bool, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        return track { done in pump.insertFrame(session: session, parent: parent, rect: rect, rectangle: rectangle, completion: done) } completion: { completion($0) }
    }

    func moveFrame(_ hit: CanvasFrameHit, delta: CGVector, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        return track { done in
            pump.moveFrame(session: session, hit: hit, delta: delta, completion: done)
        } completion: { completion($0) }
    }

    /// 이 창 session에 코어 명령 코드를 보냅니다(XII.6.4). 닫혔거나 요청을 받지 않으면 false입니다.
    @discardableResult
    func perform(
        coreCommand code: UInt32, forClosing: Bool = false, completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Bool {
        guard !isClosed, !isClosing || forClosing else { return false }
        return track { done in pump.perform(session: session, command: code, completion: done) } completion: { event in
            completion(event)
        }
    }

    @discardableResult
    func pan(dx: Double, dy: Double, completion: @escaping () -> Void = {}) -> Bool {
        guard !isClosed, !isClosing, viewInfo?.has_view == 1 else { return false }
        if dx == 0, dy == 0 { return true }
        // 팬은 배율·문서 명령 상태를 바꾸지 않습니다. 창 닫기를 위해 진행 중인 요청만 추적합니다.
        return track({ pump.pan(session: session, dx: dx, dy: dy, completion: $0) }, completion: { _ in completion() })
    }

    /// 이 창 session의 선택을 바꿉니다(그 창 history에 기록). 닫혔거나 요청을 받지 않으면 false입니다.
    @discardableResult
    func select(_ ids: [UUID], completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        return track { done in pump.select(session: session, ids: ids, completion: done) } completion: { event in
            completion(event)
        }
    }

    /// 접수된 명령의 snapshot이 아직 게시되지 않았을 때 clean으로 닫지 않습니다.
    @discardableResult
    func track(
        _ submit: (@escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError>,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Bool {
        pendingCommandCount += 1
        let result = submit { [weak self] event in
            guard let self else { return }
            self.pendingCommandCount -= 1
            guard !self.isClosed else { return }
            completion(event)
            if self.pendingCommandCount == 0 { self.commandsDidFinish?() }
        }
        if case .success = result { return true }
        pendingCommandCount -= 1
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
    static let revokeTrust = ShellCommandID("project.revokeTrust")
    static let documentFont = ShellCommandID("project.documentFont")
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

    func updateTrust(_ trusted: Bool) {
        guard session != nil else { return }
        var next = currentSnapshot
        next.subtitle = trusted ? "" : String(localized: "Restricted Mode")
        next.moreActions.removeAll { $0.id == ShellProjectCommandID.revokeTrust || $0.id == ShellProjectCommandID.documentFont }
        if trusted {
            next.moreActions.append(ShellActionDescriptor(id: ShellProjectCommandID.documentFont, title: String(localized: "Document Font…")))
            next.moreActions.append(ShellActionDescriptor(id: ShellProjectCommandID.revokeTrust,
                title: String(localized: "Revoke Project Trust")))
        }
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
///   Zoom In·Zoom Out·Actual Size·Zoom to Fit(XII.4.3)입니다. Undo는 그 창 undo stack이, Redo는 redo stack이 비어 있지 않을 때,
///   Save Now는 창 session에 작업 문서가 있을 때, 줌은 active view가 있고 배율이 경계(2%·25600%)에 닿지 않았을 때
///   활성입니다. Zoom to Fit은 현재 캔버스 크기를 코어에 보내며, 배율 표시는 코어 viewport 값입니다.
///   Zoom to Selection은 비활성입니다.
/// - 프로젝트가 ready가 아니면 도구 선택도 비활성입니다.
/// - 도구 선택은 코어로 보내지 않습니다. 다른 명령은 `commandHandler`(프로젝트 창)로 넘깁니다.
final class ShellWindowToolsModelStub: ToolsMenuModel {
    /// ready 상태에서 항상 실행할 수 있는 명령(창 session 상태와 무관).
    static let readyCommands: Set<ShellCommand> = Set(CanvasTool.allCases.map(\.command)).subtracting([.selectImageTool]).union([.closeProject])
    /// ready가 아닐 때(열기 중·실패) 실행할 수 있는 명령.
    static let notReadyCommands: Set<ShellCommand> = [.closeProject]

    private(set) var snapshot: ToolsMenuSnapshot
    let observers = ToolsMenuObservers()
    /// 도구 선택 외 명령을 받는 쪽.
    var commandHandler: ((ShellCommand) -> Void)?
    var toolChanged: ((CanvasTool) -> Void)?

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
            toolChanged?(tool)
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
    var canvasTextEditing: ShellCanvasTextEditing?
    let toolsToolbar: ToolsToolbarController
    let canvasHost: ShellCanvasHostViewController
    let sidebarSlot: ShellSidebarAreaSlot
    let inspectorSlot: ShellInspectorAreaSlot
    let panelsSlot: ShellPanelTabsSlot
    let gitSource = ShellGitDataSource()
    var choosePNG: (NSOpenPanel, NSWindow, @escaping (URL?) -> Void) -> Void = { panel, owner, selected in
        panel.beginSheetModal(for: owner) { selected($0 == .OK ? panel.url : nil) }
    }
    var pickedPNG: PickedPNG?
    var imagePickGeneration: UInt64 = 0
    var componentFileWatch: ShellComponentFileWatch?
    var recoveryPrompt: NSAlert?
    let sidebarSource = ShellSidebarSource()
    private var displayedSelection: StellarSelectionTarget?
    private var displayedOrderFlags: UInt32 = 0
    let inspectorSource = ShellFrameInspectorSource()
    /// 창 도구 모델 → 하단 도구 막대(셸 프레임 snapshot 도구 목록) 구독.
    private var toolsObservation: ToolsMenuObservation?

    var window: NSWindow? { windowController.window }

    /// 이 창이 완료를 기다리는 코어 열기 요청 id(복원·Try Again·Locate Folder…). 창이 닫히면 nil로 지웁니다.
    /// 완료 event의 요청 id가 이 값과 다르면(창이 닫혔거나 다른 요청으로 바뀜) 결과를 이 창에 붙이지 않습니다.
    var pendingOpenRequest: UInt64?

    var closeConfirmationPending = false
    var agentCloseConfirmed = false
    private var closeAttempt: UUID?
    private var closeTimer: Timer?
    private var quitSaveCompletion: ((Bool) -> Void)?
    var needsSaveBeforeClosing: Bool {
        guard let session = windowController.windowSession, !session.isClosed else { return false }
        return session.pendingCommandCount != 0 || session.info?.has_unsaved_changes != 0
    }
    /// 저장 응답을 기다리는 셸 watchdog. AutosavePolicy의 미확정 성능 수치를 대체하지 않습니다.
    var closeReplyTimeout: TimeInterval = 10
    var frameDrag: CanvasFrameDrag?
    var isWaitingToClose: Bool { closeAttempt != nil }

    /// - Parameters:
    ///   - rootPath: 열 프로젝트 폴더(절대 경로). 창은 열기 중 상태로 시작합니다.
    ///   - runtimeHandle: 캔버스 drawable·입력을 받을 코어 runtime handle.
    init(rootPath: String, runtimeHandle: UInt64?, configuration: ShellWindowConfiguration, agentSource: AgentPanelSource) {
        dataSource = ShellProjectFrameDataSource(rootPath: rootPath)
        toolsModel = ShellWindowToolsModelStub()
        router = ShellCommandRouter(model: toolsModel)
        toolsToolbar = ToolsToolbarController(model: toolsModel, router: router)
        // 캔버스 도구는 상단 툴바가 아니라 캔버스 하단 가운데 도구 막대에 둡니다(shell_frame 6.1, 2026-11-14 사용자 결정).
        // 창 도구 모델의 도구 목록·현재 도구를 셸 프레임 snapshot으로 보내고, 도구 막대 선택은 router로 보냅니다.
        toolsObservation = ShellProjectWindow.connectToolBar(dataSource: dataSource, toolsModel: toolsModel, router: router)
        canvasHost = ShellCanvasHostViewController(runtimeHandle: runtimeHandle)
        sidebarSlot = ShellSidebarAreaSlot(
            sidebar: SidebarViewController(provider: sidebarSource)
        )
        inspectorSlot = ShellInspectorAreaSlot(
            inspector: InspectorViewController(dataSource: inspectorSource)
        )
        panelsSlot = ShellPanelTabsSlot(
            gitPanel: GitPanelViewController(dataSource: gitSource),
            agentPanel: AgentPanelViewController(source: agentSource)
        )
        windowController = ShellWindowController(
            configuration: configuration,
            dataSource: dataSource,
            slots: ShellFrameSlots(
                sidebar: sidebarSlot, content: canvasHost, inspector: inspectorSlot, auxiliary: panelsSlot
            ),
            toolbarItemProvider: toolsToolbar
        )
        toolsModel.toolChanged = { [weak self] tool in
            _ = self?.canvasHost.canvasView.cancelFrameGesture?()
            self?.imageToolChanged(tool)
        }
        windowController.didBecomeKeyHandler = { [weak self] in
            guard let self else { return }
            if self.componentFileWatch?.unavailable == true { self.updateComponentFileWatch(trusted: self.gitSource.trusted) }
            self.reloadComponentCanvas(onWindowReturn: true)
        }
        gitSource.trustDidChange = { [weak self] trusted in
            if !trusted { self?.canvasTextEditing?.invalidate() }
            self?.dataSource.updateTrust(trusted)
            self?.sidebarSource.trusted = trusted
            self?.canvasHost.canvasView.allowsImageContent = trusted
            if !trusted { self?.pickedPNG = nil; self?.imagePickGeneration &+= 1 }
            self?.updateComponentFileWatch(trusted: trusted)
            self?.reloadComponentCanvas()
            self?.refreshSessionCommands()
            if !trusted { _ = self?.canvasHost.canvasView.cancelFrameGesture?() }
        }
        inspectorSource.editFrame = { [weak self] frame, change, completion in
            guard let self, let session = self.windowController.windowSession, session.session == frame.session,
                  !session.isClosed, !session.isClosing, self.gitSource.trusted,
                  let pump = self.dataSource.session?.pump else { return false }
            return session.track({ pump.editFrame(frame, change: change, completion: $0) }) { event in
                self.refreshSessionCommands()
                if case .windowViewChanged = event { completion(true) } else { completion(false) }
            }
        }
        sidebarSource.editSelection = { [weak self] command, target, ids in
            _ = self?.editSelection(command, target: target, ids: ids)
        }
        sidebarSource.canEditSelection = { [weak self] command in
            self?.toolsModel.snapshot.enabledCommands.contains(command) == true
        }
        sidebarSource.select = { [weak self] ids in _ = self?.selectNodes(ids) }
        sidebarSource.rename = { [weak self] view, revision, id, name in
            guard let self, let session = self.windowController.windowSession, !session.isClosed, !session.isClosing,
                  let pump = self.dataSource.session?.pump else { return }
            session.track({ pump.renameLayer(session: session.session, view: view, node: id, revision: revision,
                                             name: name, completion: $0) }) { [weak self, weak session] event in
                guard let self, let session, self.windowController.windowSession === session else { return }
                self.refreshSessionCommands()
                if case .requestFailed(_, _, let message) = event {
                    self.alertPresenter?(String(localized: "Couldn’t Rename Layer"), message, self.window)
                }
            }
        }
        connectFrameInteractions()
        panelsSlot.gitPanel.host = self
        if let window = windowController.window {
            // 응답자 체인: content view → router → split controller → 창. 텍스트 편집 중에는 field editor가 먼저 받습니다.
            router.install(in: window)
        }
        // 캔버스 포커스 중 단일 키 도구 단축키(tools_menus 6.2). 도구로 해석되지 않거나 비활성이면 코어 입력으로 보냅니다.
        canvasHost.canvasView.keyDownHandler = { [weak self, weak router] event in
            if event.modifierFlags.intersection([.command, .shift, .option, .control]).isEmpty,
               event.keyCode == 51 || event.keyCode == 117 {
                return router?.dispatch(.delete) ?? false
            }
            if event.keyCode == 36, event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty,
               let self, let hit = self.canvasHost.canvasView.textHit() {
                return self.editCanvasText(hit, at: nil, canvas: self.canvasHost.canvasView)
            }
            guard let router, let tool = CanvasToolShortcut.tool(for: event) else { return false }
            return router.dispatch(tool.command)
        }
        canvasHost.canvasView.scrollWheelHandler = { [weak self] event in
            guard let session = self?.windowController.windowSession,
                  event.modifierFlags.intersection([.command, .shift, .option, .control]).isEmpty else { return false }
            // AppKit이 시스템 스크롤 방향을 적용한 delta를 그대로 사용합니다. flipped canvas의 y는 아래로 증가합니다.
            _ = self?.canvasHost.canvasView.cancelFrameGesture?()
            let pointsPerUnit = event.hasPreciseScrollingDeltas ? 1.0 : 10.0
            let dx = Double(event.scrollingDeltaX) * pointsPerUnit
            let dy = Double(event.scrollingDeltaY) * pointsPerUnit
            guard dx.isFinite, dy.isFinite else { return false }
            return session.pan(dx: dx, dy: dy) { [weak self] in self?.canvasHost.canvasView.refreshFrames() }
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
        cancelPendingClose()
        windowController.windowSession?.close()
        windowController.windowSession = windowSession
        dataSource.didOpen(session)
        gitSource.attach(session, window: windowSession)
        toolsModel.setProjectReady(true)
        refreshSessionCommands()
        offerDocumentRecovery()
        window?.invalidateRestorableState()
    }

    /// 현재 View의 루트 Frame을 실제 캔버스 크기에 맞춥니다.
    private func fitRootFrame() -> Bool {
        guard let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              session.viewInfo?.has_view == 1, let project = dataSource.session else { return false }
        window?.contentView?.layoutSubtreeIfNeeded()
        let size = canvasHost.canvasView.bounds.size
        guard size.width > 0, size.height > 0 else { return false }
        return session.track({ project.pump.fitRoot(session: session.session, size: size, completion: $0) }) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.finishSessionCommand(.zoomToFit, event: event)
        }
    }

    /// 저장 실패를 알리는 곳(창, 제목, 메시지). coordinator가 앱 alert 경로를 넣습니다.
    var alertPresenter: ((_ title: String, _ message: String, _ window: NSWindow?) -> Void)?

    /// 창 session 명령(Undo·Redo·Save Now)이 끝날 때마다 받은 완료 event(검사용).
    private(set) var lastSessionCommandEvent: CoreRuntimeEvent?

    private func refreshInspector() {
        let selected = windowController.windowSession?.info?.selection_count == 1
            ? canvasHost.canvasView.selectedInspectorFrame : nil
        inspectorSource.refresh(selected, trusted: gitSource.trusted)
    }

    /// 창 session 정보로 Undo·Redo·Save Now 활성 상태를 다시 계산합니다.
    var subtreePasteboard: NSPasteboard = .general
    var clipboardKeyWindow: () -> NSWindow? = { NSApp.keyWindow }
    private static let subtreePasteboardType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.node-subtree.v1")
    let clipboardQueue = DispatchQueue(label: "xyz.taeyoon.stellar.clipboard", qos: .userInitiated)

    func refreshSessionCommands() {
        let session = windowController.windowSession
        canvasHost.canvasView.frameSession = session?.session
        refreshInspector()
        let view = session?.viewInfo
        let info = session?.info
        displayedSelection = session?.selectionTarget
        if gitSource.trusted, let target = displayedSelection, target.selection_count == 1, let handle = dataSource.session?.pump.runtimeHandle {
            displayedOrderFlags = CoreRuntime.layerOrderFlags(handle: handle, target: target)
        } else { displayedOrderFlags = 0 }
        sidebarSource.refresh(runtime: dataSource.session?.pump.runtimeHandle, session: session, target: displayedSelection)
        if let drag = frameDrag,
           info?.revision != drag.hit.revision || view.map({ UUID(uuid: $0.active_view) }) != drag.hit.viewID ||
           view?.center_x != drag.viewport.center_x || view?.center_y != drag.viewport.center_y ||
           view?.zoom != drag.viewport.zoom {
            _ = canvasHost.canvasView.cancelFrameGesture?()
        }
        var commands = ShellWindowToolsModelStub.sessionCommands(info: info, view: view)
            .filter { coreCommandIDs.coreCommandID(for: $0) != nil }
        if view?.has_view == 1 { commands.insert(.zoomToFit) }
        if gitSource.trusted, session != nil { commands.insert(.selectImageTool) }
        for command: ShellCommand in [.bringForward, .bringToFront, .sendBackward, .sendToBack] {
            if let flag = command.layerOrderFlag, displayedOrderFlags & flag != 0 { commands.insert(command) }
        }
        if gitSource.trusted, selectedComponentFrame != nil { commands.insert(.componentFromSelection) }
        if gitSource.trusted, (displayedSelection?.selection_count ?? 0) > 0 { commands.insert(.delete) }
        if gitSource.trusted, displayedSelection?.selection_count == 1 { commands.formUnion([.duplicate, .copy]) }
        if gitSource.trusted, let target = displayedSelection, target.selection_count <= 1 { commands.insert(.paste) }
        toolsModel.setSessionState(commands: commands, zoomScale: view.flatMap { $0.has_view != 0 ? $0.zoom : nil })
    }

    /// 코어 command ID 표(XII.6.5). 코어가 공유 이름으로 알려 준 ID로만 명령을 보냅니다.
    var coreCommandIDs = ShellCommandCoreIDs.empty

    /// 이 창의 명령을 코어 창 session으로 보냅니다. Zoom to Fit에는 캔버스의 실제 표시 크기가 필요합니다.
    ///
    /// 메뉴·단축키는 응답자 체인으로 key 창의 router가 받으므로 여기 오는 명령은 언제나 이 창(활성 창)의 것입니다.
    @discardableResult
    func performSessionCommand(_ command: ShellCommand) -> Bool {
        if command == .zoomToFit { return fitRootFrame() }
        if command == .copy {
            guard gitSource.trusted, !router.isTextEditing, let owner = window, clipboardKeyWindow() === owner,
                  let session = windowController.windowSession, !session.isClosing, !session.isClosed,
                  session.pendingCommandCount == 0, let target = displayedSelection,
                  target.session == session.session, target.selection_count == 1,
                  let runtime = dataSource.session?.pump.runtimeHandle else { return false }
            let boardID = ObjectIdentifier(subtreePasteboard)
            let changeCount = subtreePasteboard.changeCount
            clipboardQueue.async { [weak self, weak session, weak owner] in
                let data = CoreRuntime.copySubtree(handle: runtime, target: target)
                DispatchQueue.main.async { [weak self, weak session, weak owner] in
                    guard let self, let session, let owner, self.window === owner, self.clipboardKeyWindow() === owner,
                          !self.router.isTextEditing, self.windowController.windowSession === session,
                          !session.isClosed, !session.isClosing, session.pendingCommandCount == 0, self.gitSource.trusted,
                          target.matches(self.displayedSelection), target.matches(session.selectionTarget),
                          ObjectIdentifier(self.subtreePasteboard) == boardID, self.subtreePasteboard.changeCount == changeCount, let data else { return }
                    let item = NSPasteboardItem()
                    item.setData(data, forType: Self.subtreePasteboardType)
                    self.subtreePasteboard.clearContents()
                    self.subtreePasteboard.writeObjects([item])
                }
            }
            return true
        }
        if command == .paste {
            guard gitSource.trusted, !router.isTextEditing, let owner = window, clipboardKeyWindow() === owner,
                  let session = windowController.windowSession, let target = displayedSelection,
                  target.session == session.session, session.pendingCommandCount == 0,
                  let data = subtreePasteboard.data(forType: Self.subtreePasteboardType),
                  !data.isEmpty, data.count <= 16 * 1024 * 1024 else { return false }
            _ = canvasHost.canvasView.cancelFrameGesture?()
            return session.pasteSubtree(data, target: target) { [weak self, weak session] event in
                guard let self, self.windowController.windowSession === session else { return }
                self.finishSessionCommand(command, event: event)
            }
        }
        if let flag = command.layerOrderFlag, displayedOrderFlags & flag == 0 { return false }
        if command == .delete || command == .duplicate || command.layerOrderFlag != nil {
            guard let target = displayedSelection else { return false }
            return editSelection(command, target: target, ids: nil)
        }
        guard let code = coreCommandIDs.coreCommandID(for: command), let session = windowController.windowSession else {
            return false
        }
        return session.perform(coreCommand: code) { [weak self, weak session] event in
            guard let self, self.windowController.windowSession === session else { return }
            self.finishSessionCommand(command, event: event)
        }
    }

    private func editSelection(_ command: ShellCommand, target: StellarSelectionTarget, ids: [UUID]?) -> Bool {
        guard gitSource.trusted, !router.isTextEditing, let session = windowController.windowSession else { return false }
        let affectsFlow = command.layerOrderFlag != nil && displayedOrderFlags & UInt32(STELLAR_ORDER_FLOW_PARENT) != 0
        _ = canvasHost.canvasView.cancelFrameGesture?()
        return session.editSelection(target, command: command, ids: ids) { [weak self, weak session] event in
            guard let self, self.windowController.windowSession === session else { return }
            self.finishSessionCommand(command, event: event)
            if affectsFlow, case .windowViewChanged = event, let window = self.window {
                NSAccessibility.post(element: window, notification: .announcementRequested, userInfo: [
                    .announcement: String(localized: "Layer order and layout order changed."),
                    .priority: NSAccessibilityPriorityLevel.medium.rawValue,
                ])
            }
        }
    }

    /// 조합 중인 입력은 그대로 두고, 확정된 편집은 session을 닫기 전에 코어에 도착하게 합니다.
    func finishInputBeforeClosing() -> Bool {
        guard let window else { return true }
        if let editing = canvasTextEditing {
            guard !editing.input.hasMarkedText() else { return false }
            editing.stop()
            // focus가 끝난 뒤에도 첫 paint 또는 다음 확정 입력을 기다리면 adapter가 계속 남습니다.
            guard canvasTextEditing == nil else { return false }
        }
        if let editor = window.firstResponder as? NSTextView, editor.isFieldEditor {
            guard !editor.hasMarkedText() else { return false }
            return window.makeFirstResponder(nil)
        }
        return true
    }

    /// 정상 NSWindow close 진입점. clean은 즉시 허용하고 dirty는 실제 저장 성공까지 창을 유지합니다.
    func shouldCloseAfterSaving(forQuit completion: ((Bool) -> Void)? = nil) -> Bool {
        guard closeAttempt == nil, finishInputBeforeClosing() else { return false }
        guard let session = windowController.windowSession, !session.isClosed else { return true }
        guard needsSaveBeforeClosing else { return true }
        quitSaveCompletion = completion
        let attempt = UUID()
        closeAttempt = attempt
        _ = canvasHost.canvasView.cancelFrameGesture?()
        session.isClosing = true
        let timer = Timer(timeInterval: closeReplyTimeout, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.closeAttempt == attempt else { return }
                self.failClose(String(
                    localized: "Saving is taking longer than expected. This window is still open and saving may still finish. Close the window again to retry.",
                    comment: "Close save response timeout; memory and window are retained"
                ))
            }
        }
        closeTimer = timer
        RunLoop.main.add(timer, forMode: .common)
        session.commandsDidFinish = { [weak self] in self?.continueClose(attempt: attempt) }
        // 이미 접수된 Undo/Redo/편집을 먼저 확인합니다. UI thread는 기다리지 않습니다.
        continueClose(attempt: attempt)
        return false
    }

    private func continueClose(attempt: UUID) {
        guard closeAttempt == attempt, let session = windowController.windowSession,
              !session.isClosed, session.pendingCommandCount == 0 else { return }
        session.commandsDidFinish = nil
        guard let info = session.info else {
            failClose(String(localized: "The design’s save state couldn’t be checked. This window is still open. Try closing it again."))
            return
        }
        if info.has_unsaved_changes == 0 {
            finishClose(attempt: attempt)
            return
        }
        guard let code = coreCommandIDs.coreCommandID(for: .saveNow),
              session.perform(coreCommand: code, forClosing: true, completion: { [weak self, weak session] event in
                  guard let self, self.closeAttempt == attempt,
                        self.windowController.windowSession === session else { return }
                  self.lastSessionCommandEvent = event
                  self.refreshSessionCommands()
                  if case .windowSessionSaved(_, let savedSession, _, _) = event,
                     savedSession == session?.session, session?.info?.has_unsaved_changes == 0 {
                      self.finishClose(attempt: attempt)
                  } else {
                      self.failCloseAfterSave(event)
                  }
              }) else {
            failClose(String(localized: "Saving couldn’t start. This window and its changes are still open. Try closing it again."))
            return
        }
    }

    private func finishClose(attempt: UUID) {
        guard closeAttempt == attempt else { return }
        if window?.attachedSheet != nil {
            // 저장 응답을 받았습니다. 다른 시트에서 사용자가 답하는 시간은 저장 timeout에서 제외합니다.
            closeTimer?.invalidate()
            closeTimer = nil
            windowController.didEndSheetHandler = { [weak self] in
                // AppKit이 시트 종료와 다음 시트 연결을 마치면 창의 현재 상태로 다시 판단합니다.
                DispatchQueue.main.async { [weak self] in self?.finishClose(attempt: attempt) }
            }
            return
        }
        if let completion = quitSaveCompletion {
            quitSaveCompletion = nil
            cancelPendingClose()
            completion(true)
            return
        }
        let confirmed = agentCloseConfirmed
        cancelPendingClose()
        agentCloseConfirmed = confirmed
        // delegate를 다시 거칩니다. 창을 앞으로 가져오거나 닫힌 창을 다시 열지 않습니다.
        window?.performClose(nil)
    }

    private func failCloseAfterSave(_ event: CoreRuntimeEvent) {
        let stale: Bool
        if case .requestFailed(_, let failure, _) = event {
            stale = Int32(bitPattern: failure) == STELLAR_FAILURE_SAVE_STALE
        } else { stale = false }
        failClose(stale
            ? String(localized: "This project changed on disk. This window and its unsaved changes are still open. The disk version wasn’t overwritten.")
            : String(localized: "The design couldn’t be saved. This window and its changes are still open. Check the project folder, then close the window again to retry."))
    }

    private func failClose(_ message: String) {
        cancelPendingClose()
        alertPresenter?(String(localized: "The Window Couldn’t Be Closed"), message, window)
    }

    /// 취소·실제 닫힘·session 교체 뒤 도착하는 저장 응답은 닫기 권한이 없습니다.
    func cancelPendingClose() {
        agentCloseConfirmed = false
        closeAttempt = nil
        closeTimer?.invalidate()
        closeTimer = nil
        windowController.didEndSheetHandler = nil
        windowController.windowSession?.isClosing = false
        windowController.windowSession?.commandsDidFinish = nil
        let completion = quitSaveCompletion
        quitSaveCompletion = nil
        completion?(false)
    }

    /// 캔버스가 고른 node를 이 창의 선택 history에 기록하고 명령 활성 상태를 갱신합니다.
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
        if isWaitingToClose {
            failCloseAfterSave(event)
            return
        }
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
        canvasHost.canvasView.frameSession = nil
        inspectorSource.refresh(nil, trusted: false)
        gitSource.attach(nil)
        displayedSelection = nil
        displayedOrderFlags = 0
        sidebarSource.refresh(runtime: nil, session: nil, target: nil)
        cancelPendingClose()
        windowController.windowSession?.close()
        windowController.windowSession = nil
        dataSource.didFail(title: failure.title, message: failure.message(path: path))
        toolsModel.setProjectReady(false)
        toolsModel.setSessionState(commands: [], zoomScale: nil)
    }

    /// 창을 닫습니다: 창 session을 먼저, 그다음 프로젝트 session을 한 번씩 닫습니다.
    func closeSessions() {
        canvasHost.canvasView.frameSession = nil
        inspectorSource.refresh(nil, trusted: false)
        gitSource.attach(nil)
        displayedSelection = nil
        displayedOrderFlags = 0
        sidebarSource.refresh(runtime: nil, session: nil, target: nil)
        cancelPendingClose()
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

extension ShellProjectWindow: GitPanelHostActions {
    func gitPanelRequestsProjectTrust() {
        requestProjectTrust()
    }

    func requestProjectTrust() {
        guard let window, window.attachedSheet == nil,
              let session = dataSource.session, !session.isClosed,
              let editor = windowController.windowSession, !editor.isClosing, !editor.isClosed,
              !gitSource.trusted, !gitSource.isCheckingTrust else { return }
        let alert = NSAlert()
        alert.messageText = String(localized: "Trust This Project?")
        alert.informativeText = String(localized: "Trusting this folder allows Stellar to edit project files and run Git commands for it.") + "\n" + session.rootPath
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.addButton(withTitle: String(localized: "Trust Project"))
        alert.beginSheetModal(for: window) { [weak self, weak session] response in
            guard response == .alertSecondButtonReturn, let self, let session,
                  self.dataSource.session === session, !session.isClosed,
                  self.windowController.windowSession === editor, !editor.isClosed, !editor.isClosing else { return }
            self.gitSource.updateTrust(true)
        }
    }

    func gitPanelRequestsIdentitySetup() {
        alertPresenter?(String(localized: "Git Identity"),
                        String(localized: "Set your Git name and email in the repository’s Git configuration."), window)
    }

    func gitPanelRequestsGitInstallHelp() {
        alertPresenter?(String(localized: "Git Is Unavailable"),
                        String(localized: "Install the Xcode Command Line Tools to use Git on this Mac."), window)
    }
}

extension CanvasTool {
    /// 하단 도구 막대에 보낼 셸 프레임 도구 항목. 명령 ID는 셸 명령 ID(예: `tool.move`)입니다.
    var shellToolDescriptor: ShellToolDescriptor {
        ShellToolDescriptor(id: ShellCommandID(command.rawValue), label: label, symbolName: symbolName)
    }
}
