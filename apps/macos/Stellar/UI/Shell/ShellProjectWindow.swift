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
        case .toggleVisibility: action = UInt32(STELLAR_SELECTION_TOGGLE_VISIBILITY)
        case .toggleLock: action = UInt32(STELLAR_SELECTION_TOGGLE_LOCK)
        default: return false
        }
        return track({ pump.editSelection(target: target, action: action, ids: ids, completion: $0) }, completion: completion)
    }

    /// 문맥 메뉴를 열기 전 선택을 코어에 맡깁니다(누른 노드나 그 조상이 선택에 있으면 유지).
    func contextSelect(_ target: StellarSelectionTarget, node: UUID?, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing, target.session == session else { return false }
        return track({ pump.contextSelect(target: target, node: node, completion: $0) }, completion: completion)
    }

    func pasteSubtree(_ data: Data, target: StellarSelectionTarget, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing, target.session == session else { return false }
        return track({ pump.pasteSubtree(target: target, data: data, completion: $0) }, completion: completion)
    }

    func insertShape(parent: CanvasFrameHit, kind: InspectorShapeKind, start: CGPoint, end: CGPoint,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        // 새 도형의 칠은 앱 설정(Settings ▸ 캔버스, D7) 값을 코어에 넘깁니다.
        let paint = CanvasShapeDefaults.paint(for: kind)
        return track { done in pump.insertShape(session: session, parent: parent, kind: kind, start: start, end: end,
                                                paint: paint, completion: done) } completion: { completion($0) }
    }

    func insertFrame(parent: CanvasFrameHit, rect: CGRect, completion: @escaping (CoreRuntimeEvent) -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        return track { done in pump.insertFrame(session: session, parent: parent, rect: rect, completion: done) } completion: { completion($0) }
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

    /// 캔버스 원시 입력을 이 창 session으로 보냅니다. 코어가 새 요청을 접수하면 완료 뒤 `completion`을 부르고,
    /// 대기 중인 요청에 합쳤으면 그 요청의 완료가 반영합니다. 닫혔거나 접수하지 않으면 false입니다.
    func submitInput(_ event: StellarMacEvent, completion: @escaping () -> Void) -> Bool {
        guard !isClosed, !isClosing else { return false }
        switch CoreRuntime.submitWindowEvent(handle: pump.runtimeHandle, session: session, event: event) {
        case .failure:
            return false
        case .success(0):
            return true
        case .success(let request):
            // 창 닫기가 접수된 팬 완료를 기다리도록 다른 요청과 같이 추적합니다.
            return track({ pump.register(.success(request), $0) }, completion: { _ in completion() })
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

/// 프로젝트 창의 셸 프레임 명령 ID(제목 문서 메뉴·상태 화면 버튼).
enum ShellProjectCommandID {
    static let trustProject = ShellCommandID("project.trust")
    static let revokeTrust = ShellCommandID("project.revokeTrust")
    static let revealInFinder = ShellCommandID("project.revealInFinder")
    static let closeProject = ShellCommandID("project.close")
    static let locateFolder = ShellCommandID("project.locate")
    static let retryOpen = ShellCommandID("project.retryOpen")
}

/// 프로젝트 창의 셸 프레임 data source.
///
/// 제목은 프로젝트 이름, 부제목은 자동 저장 경고(저장 실패·stale, UX 명세 3.3)와 신뢰 상태(코어가 모든 프로젝트를
/// 신뢰되지 않음으로 열므로 "Restricted Mode", 가이드 7.2)입니다. 상태는 열기 중(loading) → ready, 또는 실패(failed, 복구 동작 포함)입니다. 도구 목록과 현재 도구는 창 도구 모델에서 받아
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
            tools: [],
            activeToolID: nil,
            moreActions: []
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
        trusted = false
        saveIssue = nil
        next.subtitle = composedSubtitle()
        next.status = .ready
        next.moreActions = [
            ShellActionDescriptor(
                id: ShellProjectCommandID.revealInFinder,
                title: String(localized: "Show in Finder", comment: "Title document menu item")
            ),
            Self.trustAction(trusted: false, enabled: !isCheckingTrust),
            ShellActionDescriptor(
                id: ShellProjectCommandID.closeProject,
                title: String(localized: "Close Project", comment: "Title document menu item")
            ),
        ]
        publish(next)
    }

    /// 제목 문서 메뉴의 신뢰 항목. 제한 모드면 신뢰 부여, 신뢰했으면 신뢰 철회입니다(UX 명세 3.3). 늘 한 항목이 있고,
    /// 신뢰 확인 요청 중에는 비활성입니다.
    static func trustAction(trusted: Bool, enabled: Bool = true) -> ShellActionDescriptor {
        trusted
            ? ShellActionDescriptor(id: ShellProjectCommandID.revokeTrust, title: String(localized: "Revoke Project Trust"),
                                    isEnabled: enabled)
            : ShellActionDescriptor(id: ShellProjectCommandID.trustProject,
                                    title: String(localized: "Trust Project…", comment: "Title document menu item"),
                                    isEnabled: enabled)
    }

    /// 신뢰 확인 요청 중인지. 요청 중에는 신뢰 항목을 누를 수 없으므로 비활성으로 보입니다.
    private(set) var isCheckingTrust = false

    func setTrustChecking(_ checking: Bool) {
        guard checking != isCheckingTrust else { return }
        isCheckingTrust = checking
        guard let index = currentSnapshot.moreActions.firstIndex(where: {
            $0.id == ShellProjectCommandID.trustProject || $0.id == ShellProjectCommandID.revokeTrust
        }) else { return }
        var next = currentSnapshot
        next.moreActions[index].isEnabled = !checking
        publish(next)
    }

    /// 부제목에 쓰는 신뢰 상태와 자동 저장 경고.
    private var trusted = false
    private(set) var saveIssue: ShellSaveIssue?

    /// 부제목: 저장 경고(있을 때) 다음 신뢰 상태. 둘 다 없으면 빈 문자열입니다.
    private func composedSubtitle() -> String {
        var parts: [String] = []
        switch saveIssue {
        case .failed?:
            parts.append(String(localized: "⚠︎ Not Saved", comment: "Window subtitle: the last autosave failed"))
        case .stale?:
            parts.append(String(localized: "⚠︎ Not Saved — Changed Elsewhere",
                                comment: "Window subtitle: another window or app saved the project first, autosave stopped"))
        case nil:
            break
        }
        if !trusted {
            parts.append(String(localized: "Restricted Mode", comment: "Window subtitle: the project isn't trusted yet"))
        }
        return parts.joined(separator: " · ")
    }

    /// 자동 저장 경고를 바꿉니다(ADR-0072 7절).
    func updateSaveIssue(_ issue: ShellSaveIssue?) {
        guard session != nil, issue != saveIssue else { return }
        saveIssue = issue
        var next = currentSnapshot
        next.subtitle = composedSubtitle()
        publish(next)
    }

    func updateTrust(_ trusted: Bool) {
        guard session != nil else { return }
        self.trusted = trusted
        var next = currentSnapshot
        next.subtitle = composedSubtitle()
        if let index = next.moreActions.firstIndex(where: {
            $0.id == ShellProjectCommandID.trustProject || $0.id == ShellProjectCommandID.revokeTrust
        }) {
            next.moreActions[index] = Self.trustAction(trusted: trusted, enabled: !isCheckingTrust)
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
/// 코어 도구 상태 기계(XII.3.6)가 없으므로, 창별 현재 도구만 셸이 보관합니다(코어 이전은 UX WBS UX.3.8). 제한:
/// - 실행 가능한 명령은 도구 선택 10개, Close Project, 그리고 창 session 상태에 따른 Undo·Redo·Save Now(XII.6.4)와
///   Zoom In·Zoom Out·Actual Size·Zoom to Fit(XII.4.3)입니다. Undo는 그 창 undo stack이, Redo는 redo stack이 비어 있지 않을 때,
///   Save Now는 창 session에 작업 문서가 있을 때, 줌은 active view가 있고 배율이 경계(2%·25600%)에 닿지 않았을 때
///   활성입니다. Zoom to Fit은 현재 캔버스 크기를 코어에 보내며, 배율 표시는 코어 viewport 값입니다.
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
    var toolChanged: ((CanvasTool) -> Void)?

    init() {
        snapshot = ToolsMenuSnapshot(
            generation: 1,
            activeTool: .move,
            zoomScale: 1.0,
            enabledCommands: Self.notReadyCommands,
            undoActionName: nil,
            redoActionName: nil,
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
        if !ready { sessionCommands = []; selectionCommands = [] }
        republishEnabledCommands()
    }

    /// 선택한 레이어의 Hide·Lock 명령과 메뉴 제목 상태(UX.3.4). 사이드바가 선택 행을 읽을 때마다 갱신합니다.
    private(set) var selectionCommands: Set<ShellCommand> = []

    func setSelectionState(commands: Set<ShellCommand>, hidden: Bool, locked: Bool) {
        selectionCommands = isReady ? commands : []
        if snapshot.selectionIsHidden != hidden || snapshot.selectionIsLocked != locked {
            var next = snapshot
            next.selectionIsHidden = hidden
            next.selectionIsLocked = locked
            publish(next)
        }
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
        let commands = isReady ? Self.readyCommands.union(sessionCommands).union(selectionCommands) : Self.notReadyCommands
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
    var canvasTextEditing: ShellCanvasTextEditing? {
        // 텍스트 편집에 들어가면 속성 탭의 Typography 섹션을 펼칩니다(UX.2.12).
        didSet { if canvasTextEditing != nil, oldValue == nil { inspectorSlot.inspector.reveal(.typography) } }
    }
    let toolsToolbar: ToolsToolbarController
    let canvasHost: ShellCanvasHostViewController
    let sidebarSlot: ShellSidebarAreaSlot
    let inspectorSlot: ShellInspectorAreaSlot
    /// Git 상태·신뢰의 원본. Git 패널은 캔버스 아래 보조 패널과 함께 빠졌고, 왼쪽 사이드바 변경 탭(UX.5.2)에서
    /// 이 data source로 다시 붙습니다.
    let gitSource = ShellGitDataSource()
    var choosePNG: (NSOpenPanel, NSWindow, @escaping (URL?) -> Void) -> Void = { panel, owner, selected in
        panel.beginSheetModal(for: owner) { selected($0 == .OK ? panel.url : nil) }
    }
    /// Image 도구를 취소하면 돌아갈 도구(마지막으로 고른 Image 밖 도구).
    var imageReturnTool: CanvasTool = .move
    var imagePickGeneration: UInt64 = 0
    var recoveryPrompt: NSAlert?
    let sidebarSource = ShellSidebarSource()
    private var displayedSelection: StellarSelectionTarget?
    /// 표시한 선택에 대한 코어 능력 mask(`STELLAR_SELECTION_CAN_*`). 메뉴 막대·문맥 메뉴 활성과 Hide·Lock 제목에 씁니다.
    private(set) var displayedSelectionFlags: UInt32 = 0
    let inspectorSource = ShellInspectorSource()
    /// 창 도구 모델 → 하단 도구 막대(셸 프레임 snapshot 도구 목록) 구독.
    private var toolsObservation: ToolsMenuObservation?

    var window: NSWindow? { windowController.window }

    /// 이 창이 완료를 기다리는 코어 열기 요청 id(복원·Try Again·Locate Folder…). 창이 닫히면 nil로 지웁니다.
    /// 완료 event의 요청 id가 이 값과 다르면(창이 닫혔거나 다른 요청으로 바뀜) 결과를 이 창에 붙이지 않습니다.
    var pendingOpenRequest: UInt64?

    var closeConfirmationPending = false
    var agentCloseConfirmed = false
    /// 자동 저장 상태 관찰(`ShellAutosaveStatus.swift`).
    var autosaveObservation: (pump: ShellRuntimeEventPump, token: UUID)?
    private var closeAttempt: UUID?
    private var closeTimer: Timer?
    private var quitSaveCompletion: ((Bool) -> Void)?
    var needsSaveBeforeClosing: Bool {
        guard let session = windowController.windowSession, !session.isClosed else { return false }
        return session.pendingCommandCount != 0 || session.info?.has_unsaved_changes != 0
    }
    /// 저장 응답을 기다리는 셸 watchdog. 코어 Save Now는 같은 프로젝트의 진행 중인 자동 저장을 최대
    /// `window_close_flush_timeout_ms`(10초, `config/performance-budgets.yaml`) 기다린 뒤 다시 쓰므로, watchdog은 그 상한과
    /// 쓰기 시간보다 길어야 저장에 성공한 닫기를 실패로 보고하지 않습니다. 같은 값을 코어에서 읽는 일은 XIII.1.6입니다.
    private let closeReplyTimeout: TimeInterval = 30
    var frameDrag: CanvasFrameDrag?
    /// root Frame 밖에서 생성 도구로 누른 뒤 놓을 때까지 금지 커서를 유지합니다(UX.2.8).
    var rejectedDraw = false
    var isWaitingToClose: Bool { closeAttempt != nil }

    /// - Parameters:
    ///   - rootPath: 열 프로젝트 폴더(절대 경로). 창은 열기 중 상태로 시작합니다.
    ///   - runtimeHandle: 캔버스 drawable·입력을 받을 코어 runtime handle.
    init(rootPath: String, runtimeHandle: UInt64?, configuration: ShellWindowConfiguration) {
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
        windowController = ShellWindowController(
            configuration: configuration,
            dataSource: dataSource,
            slots: ShellFrameSlots(sidebar: sidebarSlot, content: canvasHost, inspector: inspectorSlot),
            toolbarItemProvider: toolsToolbar
        )
        toolsModel.toolChanged = { [weak self] tool in
            _ = self?.canvasHost.canvasView.cancelFrameGesture?()
            self?.imageToolChanged(tool)
        }
        windowController.didBecomeKeyHandler = { [weak self] in
            // 다른 앱이 클립보드를 바꿨을 수 있어 Paste 활성을 다시 계산합니다.
            self?.refreshSessionCommands()
        }
        gitSource.trustDidChange = { [weak self] trusted in
            self?.dataSource.updateTrust(trusted)
            self?.reloadComponentCanvas()
            self?.refreshSessionCommands()
        }
        // 속성 패널 편집(ADR-0070). 편집은 신뢰와 무관합니다(D1).
        inspectorSource.send = { [weak self] name, input, phase, basis, completion in
            guard let self, let session = self.windowController.windowSession, !session.isClosed, !session.isClosing,
                  let pump = self.dataSource.session?.pump else {
                completion(String(localized: "The window isn’t ready."))
                return
            }
            var diagnostic: String?
            let accepted = session.track({ done in
                let result = pump.editProperty(session: session.session, basis: basis, name: name, input: input,
                                               phase: phase, completion: done)
                if case .failure(let failure) = result { diagnostic = failure.diagnostic }
                return result.mapError { CoreRuntimeCallError(status: $0.status) }
            }) { [weak self] event in
                self?.refreshSessionCommands()
                if case .requestFailed(_, _, let message) = event { completion(message) } else { completion(nil) }
            }
            if !accepted { completion(diagnostic ?? String(localized: "Stellar couldn’t apply the value.")) }
        }
        inspectorSlot.inspector.focusCanvas = { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self.canvasHost.canvasView)
        }
        inspectorSlot.inspector.isCanvasGestureActive = { [weak self] in
            self?.canvasHost.canvasView.frameGestureActive?() == true
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
                    // 막다른 alert 대신 한 줄 안내로 이유를 보이고, 그 행의 이름 편집으로 되돌립니다(UX 명세 3.5).
                    self.windowController.toolbarController.hintView.show(String(
                        localized: "Couldn’t rename the layer: \(message)",
                        comment: "Hint when renaming a layer fails; the argument is the reason"
                    ))
                    self.sidebarSlot.sidebar.layersController.beginRename(SidebarStableID(id))
                }
            }
        }
        gitSource.trustCheckDidChange = { [weak self] checking in self?.dataSource.setTrustChecking(checking) }
        connectFrameInteractions()
        connectLayerFlags()
        connectCanvasContextMenu()
        connectPagePath()
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
            // 화살표는 선택을 1px, Shift+화살표는 10px 옮깁니다(부모 공간, UX 명세 5.4).
            if let self, let delta = Self.arrowNudge(event), !self.canvasHost.canvasView.selectionBoxes.isEmpty {
                return self.moveSelection(anchor: nil, delta: delta, flags: UInt32(STELLAR_MOVE_PARENT_SPACE))
            }
            guard let router, let tool = CanvasToolShortcut.tool(for: event) else { return false }
            return router.dispatch(tool.command)
        }
        canvasHost.canvasView.windowInputSink = { [weak self] event in
            guard let session = self?.windowController.windowSession else { return false }
            return session.submitInput(event) { [weak self] in self?.canvasHost.canvasView.refreshFrames() }
        }
        canvasHost.canvasView.scrollZoomHandler = { [weak self] event in
            // ⌘ 스크롤은 커서 기준 줌입니다(위로 굴리면 확대, 10pt마다 약 10%, ADR-0071 UX.3.1).
            guard let self, event.modifierFlags.contains(.command),
                  event.modifierFlags.intersection([.option, .control]).isEmpty else { return false }
            let canvas = self.canvasHost.canvasView
            let dy = Double(event.scrollingDeltaY) * (event.hasPreciseScrollingDeltas ? 1.0 : 10.0)
            guard dy.isFinite else { return false }
            let point = CanvasInput.viewPoint(of: event, in: canvas)
            let anchor = CGPoint(x: point.x - canvas.bounds.midX, y: point.y - canvas.bounds.midY)
            return dy == 0 || self.zoomCanvas(by: exp(dy * 0.01), anchor: anchor)
        }
        canvasHost.canvasView.magnifyHandler = { [weak self] event in
            guard let self else { return false }
            let canvas = self.canvasHost.canvasView
            let point = CanvasInput.viewPoint(of: event, in: canvas)
            let anchor = CGPoint(x: point.x - canvas.bounds.midX, y: point.y - canvas.bounds.midY)
            return self.zoomCanvas(by: 1 + Double(event.magnification), anchor: anchor)
        }
        // 키 목록은 `CanvasTool.singleKey`에서 만들어 실제 배정과 같습니다.
        canvasHost.canvasView.setAccessibilityHelp(CanvasTool.shortcutHelp)
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

    /// 이 창에 활성 에이전트 세션이 있는지. 에이전트 세션 실행 경로(XXIX)가 연결되기 전에는 늘 false이고, 오른쪽
    /// 에이전트 탭이 들어오면 그 세션 상태를 여기에 넣습니다. 닫기·종료 확인 경로는 이 값으로 검사합니다.
    var hasActiveAgentSession: () -> Bool = { false }

    /// 창을 닫기 전에 확인이 필요한지(활성 에이전트 세션, ADR-0005 결정 4.5).
    var requiresConfirmationBeforeClosing: Bool { hasActiveAgentSession() }

    /// 코어가 프로젝트와 이 창의 창 session을 열었습니다. 창 controller가 창 session handle을 소유합니다(XII.2.2).
    func didOpen(_ session: ShellProjectSessionHandle, windowSession: ShellWindowSessionHandle) {
        cancelPendingClose()
        windowController.windowSession?.close()
        windowController.windowSession = windowSession
        dataSource.didOpen(session)
        gitSource.attach(session)
        toolsModel.setProjectReady(true)
        observeAutosave()
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
    /// 닫기 저장이 실패했을 때 다시 시도·저장하지 않고 닫기·취소 중 하나를 고르게 합니다(§22.2 선택 팝업).
    /// coordinator가 앱 시트 경로를 넣습니다.
    var closeFailureChooser: ((_ message: String, _ window: NSWindow?,
                               _ choose: @escaping (ShellCloseFailureChoice) -> Void) -> Void)?

    /// 창 session 명령(Undo·Redo·Save Now)이 끝날 때마다 받은 완료 event(검사용).
    private(set) var lastSessionCommandEvent: CoreRuntimeEvent?

    /// Inspector 값은 core가 문서에서 읽은 선택 snapshot 하나에서 가져옵니다(캔버스 투영을 보지 않음).
    private func refreshInspector() {
        let session = windowController.windowSession
        inspectorSource.refresh(runtime: dataSource.session?.pump.runtimeHandle,
                                session: session.flatMap { $0.isClosed ? nil : $0.session })
    }

    var subtreePasteboard: NSPasteboard = .general
    private static let subtreePasteboardType = NSPasteboard.PasteboardType("xyz.taeyoon.stellar.node-subtree.v1")
    /// 붙여 넣을 Stellar 노드가 클립보드에 있는지(메뉴 막대·문맥 메뉴의 Paste 활성).
    var hasSubtreeOnPasteboard: Bool { subtreePasteboard.types?.contains(Self.subtreePasteboardType) == true }
    let clipboardQueue = DispatchQueue(label: "xyz.taeyoon.stellar.clipboard", qos: .userInitiated)

    /// 창 session 정보로 Undo·Redo·Save Now 활성 상태를 다시 계산합니다.
    func refreshSessionCommands() {
        let session = windowController.windowSession
        canvasHost.canvasView.frameSession = session?.session
        refreshInspector()
        let view = session?.viewInfo
        let info = session?.info
        displayedSelection = session?.selectionTarget
        if let target = displayedSelection, let handle = dataSource.session?.pump.runtimeHandle {
            displayedSelectionFlags = CoreRuntime.selectionFlags(handle: handle, target: target)
        } else { displayedSelectionFlags = 0 }
        sidebarSource.refresh(runtime: dataSource.session?.pump.runtimeHandle, session: session, target: displayedSelection)
        if let drag = frameDrag,
           info?.revision != drag.hit.revision || view.map({ UUID(uuid: $0.active_view) }) != drag.hit.viewID ||
           view?.center_x != drag.viewport.center_x || view?.center_y != drag.viewport.center_y ||
           view?.zoom != drag.viewport.zoom {
            _ = canvasHost.canvasView.cancelFrameGesture?()
        }
        var commands = ShellWindowToolsModelStub.sessionCommands(info: info, view: view)
            .filter { coreCommandIDs.coreCommandID(for: $0) != nil }
        if view?.has_view == 1 { commands.formUnion([.zoomToFit, .selectAll]) }
        for command: ShellCommand in [.bringForward, .bringToFront, .sendBackward, .sendToBack] {
            if let flag = command.layerOrderFlag, displayedSelectionFlags & flag != 0 { commands.insert(command) }
        }
        // 활성은 코어 능력 mask(View root·잠금 규칙 반영)로 정하고, Paste는 클립보드에 노드가 있을 때만 켭니다.
        // Cut은 Copy 뒤 delete라 Copy와 Delete가 함께 되는 선택에서만 켭니다. 복사는 잠긴 자손을 막지 않지만
        // 삭제는 막으므로(ADR-0070 결정 5) 두 mask를 모두 봅니다.
        let selection = displayedSelectionFlags
        if selection & UInt32(STELLAR_SELECTION_CAN_DELETE) != 0 { commands.insert(.delete) }
        if selection & UInt32(STELLAR_SELECTION_CAN_DUPLICATE) != 0 { commands.insert(.duplicate) }
        if selection & UInt32(STELLAR_SELECTION_CAN_COPY) != 0 { commands.insert(.copy) }
        if selection & UInt32(STELLAR_SELECTION_CAN_COPY) != 0, selection & UInt32(STELLAR_SELECTION_CAN_DELETE) != 0 {
            commands.insert(.cut)
        }
        if selection & UInt32(STELLAR_SELECTION_CAN_PASTE) != 0, hasSubtreeOnPasteboard {
            commands.insert(.paste)
        }
        toolsModel.setSessionState(commands: commands, zoomScale: view.flatMap { $0.has_view != 0 ? $0.zoom : nil })
        refreshLayerFlagCommands()
        refreshAutosaveStatus()
    }

    /// 코어 command ID 표(XII.6.5). 코어가 공유 이름으로 알려 준 ID로만 명령을 보냅니다.
    var coreCommandIDs = ShellCommandCoreIDs.empty

    /// 이 창의 명령을 코어 창 session으로 보냅니다. Zoom to Fit에는 캔버스의 실제 표시 크기가 필요합니다.
    ///
    /// 메뉴·단축키는 응답자 체인으로 key 창의 router가 받으므로 여기 오는 명령은 언제나 이 창(활성 창)의 것입니다.
    @discardableResult
    func performSessionCommand(_ command: ShellCommand) -> Bool {
        if command == .zoomToFit { return fitRootFrame() }
        if command == .selectAll { return selectAllLayers() }
        // Cut = Copy 뒤 같은 선택을 코어 delete command로 지웁니다. 문서 변경은 delete 하나라 Undo 한 번이 되돌립니다.
        if command == .copy || command == .cut {
            guard !router.isTextEditing, let owner = window,
                  let session = windowController.windowSession, !session.isClosing, !session.isClosed,
                  session.pendingCommandCount == 0, let target = displayedSelection,
                  target.session == session.session, target.selection_count == 1,
                  let runtime = dataSource.session?.pump.runtimeHandle else { return false }
            let boardID = ObjectIdentifier(subtreePasteboard)
            let changeCount = subtreePasteboard.changeCount
            // 코어가 복사 시점에 표시한 선택을 확인합니다. 전달 시점에는 pasteboard·창·session만 확인해
            // 그사이 선택이 바뀌어도 사용자가 복사한 내용을 버리지 않습니다.
            clipboardQueue.async { [weak self, weak session, weak owner] in
                let data = CoreRuntime.copySubtree(handle: runtime, target: target)
                DispatchQueue.main.async { [weak self, weak session, weak owner] in
                    guard let self, let session, let owner, self.window === owner,
                          self.windowController.windowSession === session, !session.isClosed,
                          ObjectIdentifier(self.subtreePasteboard) == boardID, self.subtreePasteboard.changeCount == changeCount, let data else { return }
                    let item = NSPasteboardItem()
                    item.setData(data, forType: Self.subtreePasteboardType)
                    self.subtreePasteboard.clearContents()
                    self.subtreePasteboard.writeObjects([item])
                    if command == .cut { _ = self.editSelection(.delete, target: target, ids: nil) }
                    self.refreshSessionCommands()
                }
            }
            return true
        }
        if command == .paste {
            guard !router.isTextEditing,
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
        if let flag = command.layerOrderFlag, displayedSelectionFlags & flag == 0 { return false }
        if command == .delete || command == .duplicate || command.layerOrderFlag != nil
            || command == .toggleVisibility || command == .toggleLock {
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
        guard !router.isTextEditing, let session = windowController.windowSession else { return false }
        let affectsFlow = command.layerOrderFlag != nil && displayedSelectionFlags & UInt32(STELLAR_SELECTION_ORDER_FLOW_PARENT) != 0
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
    ///
    /// - 입력기 조합(marked text) 중이면 닫지 않습니다.
    /// - 편집을 끝낸 뒤 남은 요청이 문서를 바꾸지 않으면(조합 시작·갱신·취소, 새 Text의 caret만 있는 경우) 편집을 버리고
    ///   닫기를 막지 않습니다. 이미 보낸 요청은 창 session의 진행 요청이라 닫기 저장이 그 완료를 기다립니다.
    /// - 아직 보내지 않은 확정·입력·Undo가 남았으면 지금은 false이고, 그 요청을 모두 보낸 뒤 `retry`를 한 번 부릅니다.
    func finishInputBeforeClosing(retry: (@MainActor @Sendable () -> Void)? = nil) -> Bool {
        guard let window else { return true }
        if let editing = canvasTextEditing {
            guard !editing.input.hasMarkedText() else { return false }
            editing.stop()
            if canvasTextEditing === editing {
                if editing.hasUnsentDocumentChanges {
                    editing.onDocumentChangesSent = retry
                    return false
                }
                editing.abandonForClosing()
            }
        }
        if let editor = window.firstResponder as? NSTextView, editor.isFieldEditor {
            guard !editor.hasMarkedText() else { return false }
            return window.makeFirstResponder(nil)
        }
        return true
    }

    /// Text 입력의 남은 문서 변경 요청을 보내는 중이라 닫기·종료가 다시 시도되기를 기다리는지.
    var isWaitingForTextInput: Bool { canvasTextEditing?.onDocumentChangesSent != nil }

    /// 정상 NSWindow close 진입점. clean은 즉시 허용하고 dirty는 실제 저장 성공까지 창을 유지합니다.
    func shouldCloseAfterSaving(forQuit completion: ((Bool) -> Void)? = nil) -> Bool {
        var retryClose: (@MainActor @Sendable () -> Void)?
        if completion == nil {
            retryClose = { [weak self] in self?.window?.performClose(nil) }
        }
        guard closeAttempt == nil, finishInputBeforeClosing(retry: retryClose) else { return false }
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
                    localized: "Saving is taking longer than expected. This window is still open and saving may still finish.",
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
            failClose(String(localized: "The design’s save state couldn’t be checked. This window is still open."))
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
            failClose(String(localized: "Saving couldn’t start. This window and its changes are still open."))
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
            : String(localized: "The design couldn’t be saved. This window and its changes are still open. Check the project folder."))
    }

    /// 창과 편집은 그대로 둔 채 사용자에게 다음 동작을 묻습니다. 앱 종료 중이면 고른 결과를 종료 흐름에 돌려줍니다.
    private func failClose(_ message: String) {
        let quit = quitSaveCompletion
        quitSaveCompletion = nil
        let agentConfirmed = agentCloseConfirmed
        cancelPendingClose()
        guard let closeFailureChooser else {
            alertPresenter?(String(localized: "The Window Couldn’t Be Closed"), message, window)
            quit?(false)
            return
        }
        closeFailureChooser(message, window) { [weak self] choice in
            guard let self, let session = self.windowController.windowSession, !session.isClosed else {
                quit?(false)
                return
            }
            switch choice {
            case .retry:
                if let quit {
                    // 종료 흐름은 이 창의 저장 결과를 기다립니다. 입력을 끝낼 수 없으면 종료를 취소합니다.
                    guard self.finishInputBeforeClosing() else { return quit(false) }
                    if self.shouldCloseAfterSaving(forQuit: quit) { quit(true) }
                } else {
                    // 에이전트 확인은 이미 받았으므로 다시 묻지 않고 같은 닫기 경로로 저장을 재시도합니다.
                    self.agentCloseConfirmed = agentConfirmed
                    self.window?.performClose(nil)
                }
            case .discard:
                // 저수준 close는 저장하지 않는 명시적 폐기입니다. 창 session과 복구 체크포인트를 함께 버립니다.
                self.window?.close()
                quit?(true)
            case .cancel:
                quit?(false)
            }
        }
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
        inspectorSource.refresh(runtime: nil, session: nil)
        gitSource.attach(nil)
        displayedSelection = nil
        displayedSelectionFlags = 0
        sidebarSource.refresh(runtime: nil, session: nil, target: nil)
        cancelPendingClose()
        stopObservingAutosave()
        windowController.windowSession?.close()
        windowController.windowSession = nil
        dataSource.didFail(title: failure.title, message: failure.message(path: path))
        toolsModel.setProjectReady(false)
        toolsModel.setSessionState(commands: [], zoomScale: nil)
    }

    /// 창을 닫습니다: 창 session을 먼저, 그다음 프로젝트 session을 한 번씩 닫습니다.
    func closeSessions() {
        canvasHost.canvasView.frameSession = nil
        inspectorSource.refresh(runtime: nil, session: nil)
        gitSource.attach(nil)
        displayedSelection = nil
        displayedSelectionFlags = 0
        sidebarSource.refresh(runtime: nil, session: nil, target: nil)
        cancelPendingClose()
        stopObservingAutosave()
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
        // 신뢰는 코드 실행 계열(아키텍처 §24.3)만 엽니다. 편집·저장은 신뢰와 관계없이 됩니다(D1, UX 명세 8.7).
        alert.informativeText = String(
            localized: "In Restricted Mode, Stellar doesn’t run code from this folder: Git hooks and filters, the integrated terminal, MCP and agents, and external commands. It also doesn’t show the contents of symbolic links that point outside the project. Trust this folder if you know where it came from. Editing and saving work either way.",
            comment: "Trust alert message; the project folder path follows on the next line"
        ) + "\n" + session.rootPath
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

/// 닫기 저장이 실패했을 때 사용자가 고른 동작(D6).
enum ShellCloseFailureChoice {
    /// 같은 닫기 경로로 저장을 다시 시도합니다.
    case retry
    /// 저장하지 않고 창을 닫습니다. 메모리의 편집과 복구 체크포인트를 버립니다.
    case discard
    /// 창과 편집을 그대로 둡니다. 앱 종료 중이면 종료를 취소합니다.
    case cancel
}

extension CanvasTool {
    /// 하단 도구 막대에 보낼 셸 프레임 도구 항목. 명령 ID는 셸 명령 ID(예: `tool.move`)입니다.
    var shellToolDescriptor: ShellToolDescriptor {
        ShellToolDescriptor(id: ShellCommandID(command.rawValue), label: label, symbolName: symbolName,
                            helpTag: helpTag, startsGroup: startsGroup)
    }
}
