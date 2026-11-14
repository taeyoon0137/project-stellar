import AppKit

/// 도구·메뉴 영역이 표시에 쓰는 값 snapshot(코어 → 셸).
///
/// Rust 타입 없이 값만 담습니다. 코어 연결 시 코어→셸 pull 이벤트 큐(ADR-0009)로 받은
/// 버전형 메시지를 이 구조체로 디코드합니다(아키텍처 §8.2, ADR-0009). 셸은 이 값을 읽기만 합니다.
struct ToolsMenuSnapshot: Equatable {
    /// 단조 증가 세대. 현재 적용한 세대보다 작은 snapshot은 버립니다(아키텍처 §9.3).
    var generation: UInt64
    /// 현재 캔버스 도구.
    var activeTool: CanvasTool
    /// 현재 viewport 배율(1.0 = 100%).
    var zoomScale: Double
    /// 지금 실행할 수 있는 명령. 여기에 없는 명령의 메뉴·툴바 항목은 숨기지 않고 비활성화합니다.
    var enabledCommands: Set<ShellCommand>
    /// Undo 대상 이름(예: "Move"). 있으면 메뉴 제목이 "Undo Move"가 됩니다.
    var undoActionName: String?
    /// Redo 대상 이름.
    var redoActionName: String?
    /// 눈금자 표시 여부(placeholder: 전용 WBS 없음).
    var rulersVisible: Bool
    /// 현재 선택이 잠겨 있는지(Lock/Unlock 제목).
    var selectionIsLocked: Bool
    /// 현재 선택이 숨겨져 있는지(Hide/Show 제목).
    var selectionIsHidden: Bool
}

/// 도구·메뉴 영역이 필요로 하는 데이터와 명령 경계.
///
/// 값 snapshot이 들어오고 명령이 나갑니다. 구현체는 명령을 코어 Document Thread 큐에 넣기만 하고
/// 결과는 다음 snapshot으로 돌려줍니다. 모든 호출은 메인 스레드에서 짧게 끝나야 합니다(아키텍처 §6.3, §9.2).
protocol ToolsMenuModel: AnyObject {
    /// 마지막으로 받은 snapshot. 메뉴 검증이 자주 읽으므로 계산 없이 저장 값을 돌려줘야 합니다.
    var snapshot: ToolsMenuSnapshot { get }
    /// snapshot 변경 구독을 추가합니다. 변경되면 메인 스레드에서 등록 순서대로 호출됩니다.
    ///
    /// 구독자는 여럿일 수 있습니다(툴바, 캔버스 호스트, 인스펙터 등 같은 창 session을 보는 영역). 반환한
    /// 토큰이 해제되거나 `cancel()`되면 구독이 끝납니다. 토큰을 보관하지 않으면 바로 해제됩니다.
    func addSnapshotObserver(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation
    /// 명령을 보냅니다(비동기 반영).
    func send(_ command: ShellCommand)
}

/// `ToolsMenuModel` snapshot 구독 토큰. 해제되면 구독이 끝납니다.
final class ToolsMenuObservation {
    private var onCancel: (() -> Void)?

    init(onCancel: @escaping () -> Void) {
        self.onCancel = onCancel
    }

    /// 구독을 끝냅니다. 여러 번 불러도 한 번만 처리합니다.
    func cancel() {
        let action = onCancel
        onCancel = nil
        action?()
    }

    deinit {
        cancel()
    }
}

/// 구현체가 공유하는 구독자 목록(메인 스레드 전용).
///
/// 알림 중에 구독이 추가·취소되어도 그 알림은 시작 시점의 구독자에게만 갑니다.
final class ToolsMenuObservers {
    private var handlers: [(id: UInt64, handler: (ToolsMenuSnapshot) -> Void)] = []
    private var nextID: UInt64 = 0

    /// 살아 있는 구독 수(검사용).
    var count: Int { handlers.count }

    func add(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation {
        nextID += 1
        let id = nextID
        handlers.append((id, handler))
        return ToolsMenuObservation { [weak self] in
            self?.handlers.removeAll { $0.id == id }
        }
    }

    func notify(_ snapshot: ToolsMenuSnapshot) {
        for entry in handlers {
            entry.handler(snapshot)
        }
    }
}

/// 상태에 따라 바뀌는 메뉴 표시(제목·체크·활성).
struct ShellCommandPresentation: Equatable {
    var title: String
    var isOn: Bool
    var isEnabled: Bool

    /// snapshot과 텍스트 편집 여부로 명령 표시를 계산합니다(메뉴 검증 경로, 할당 외 계산 없음).
    static func make(
        _ command: ShellCommand,
        snapshot: ToolsMenuSnapshot,
        isTextEditing: Bool
    ) -> ShellCommandPresentation {
        var title = command.title
        var isOn = false
        switch command {
        case .undo:
            if let name = snapshot.undoActionName, !name.isEmpty { title = "Undo \(name)" }
        case .redo:
            if let name = snapshot.redoActionName, !name.isEmpty { title = "Redo \(name)" }
        case .toggleRulers:
            title = snapshot.rulersVisible ? "Hide Rulers" : "Show Rulers"
        case .toggleLock:
            title = snapshot.selectionIsLocked ? "Unlock" : "Lock"
        case .toggleVisibility:
            title = snapshot.selectionIsHidden ? "Show" : "Hide"
        default:
            if let tool = command.canvasTool {
                isOn = snapshot.activeTool == tool
            }
        }
        var isEnabled = snapshot.enabledCommands.contains(command)
        if isTextEditing, command.isSuppressedDuringTextEditing {
            isEnabled = false
        }
        return ShellCommandPresentation(title: title, isOn: isOn, isEnabled: isEnabled)
    }
}

/// 줌 배율 표시 형식.
enum ZoomFormat {
    /// 배율을 지역화된 백분율 문자열로 바꿉니다(예: 1.0 → "100%", 0.125 → "12.5%").
    static func percentString(_ scale: Double) -> String {
        guard scale.isFinite, scale > 0 else { return "–" }
        return scale.formatted(.percent.precision(.fractionLength(0...1)))
    }
}

/// `ShellCommand` → 코어 command ID 대응 Stub(계획 XII.6.5).
///
/// 코어에는 아직 command ID 체계가 없습니다(`crates/commands`는 골격). 코어가 ID를 정의하면 이 Stub을
/// 코어가 내보내는 ID 표(FFI 조회 또는 생성 헤더)로 바꿉니다. 셸 전용 명령은 nil입니다.
enum ShellCommandCoreIDStub {
    /// 임시 ID 접두사. 코어 ID가 생기면 제거합니다.
    static let provisionalPrefix = "stellar.stub."

    /// 코어 command ID(임시 문자열). 셸 전용이면 nil.
    static func coreCommandID(for command: ShellCommand) -> String? {
        guard command.owner == .core else { return nil }
        return provisionalPrefix + command.rawValue
    }
}

/// 코어 없이 도구·메뉴 영역을 띄우는 샘플 구현(의도적 placeholder).
///
/// 제한:
/// - 코어(FFI)를 호출하지 않습니다. 받은 명령은 `sentCommands`에 기록만 합니다.
/// - 도구 전환, 줌(×2 / ÷2, 2%~25600%), Lock·Hide 토글만 snapshot에 흉내 냅니다.
/// - 눈금자는 전용 WBS가 없으므로 `toggleRulers`를 항상 비활성으로 둡니다.
/// - Undo/Redo 이름과 활성 여부는 초기값에서 바뀌지 않습니다.
final class ToolsMenuModelStub: ToolsMenuModel {
    /// 샘플 snapshot: Move 도구, 100%, Undo 가능("Move"), Redo 불가, 눈금자 비활성.
    static let sampleSnapshot = ToolsMenuSnapshot(
        generation: 1,
        activeTool: .move,
        zoomScale: 1.0,
        enabledCommands: Set(ShellCommand.allCases).subtracting([.redo, .toggleRulers]),
        undoActionName: "Move",
        redoActionName: nil,
        rulersVisible: false,
        selectionIsLocked: false,
        selectionIsHidden: false
    )

    static let minimumZoomScale = 0.02
    static let maximumZoomScale = 256.0

    private(set) var snapshot: ToolsMenuSnapshot
    let observers = ToolsMenuObservers()
    /// 받은 명령 기록(검사용).
    private(set) var sentCommands: [ShellCommand] = []

    init(snapshot: ToolsMenuSnapshot = ToolsMenuModelStub.sampleSnapshot) {
        self.snapshot = snapshot
    }

    func addSnapshotObserver(_ handler: @escaping (ToolsMenuSnapshot) -> Void) -> ToolsMenuObservation {
        observers.add(handler)
    }

    func send(_ command: ShellCommand) {
        sentCommands.append(command)
        var next = snapshot
        if let tool = command.canvasTool {
            next.activeTool = tool
        }
        switch command {
        case .zoomIn:
            next.zoomScale = min(snapshot.zoomScale * 2, Self.maximumZoomScale)
        case .zoomOut:
            next.zoomScale = max(snapshot.zoomScale / 2, Self.minimumZoomScale)
        case .zoomToActualSize, .zoomToFit, .zoomToSelection:
            next.zoomScale = 1.0
        case .toggleLock:
            next.selectionIsLocked.toggle()
        case .toggleVisibility:
            next.selectionIsHidden.toggle()
        default:
            break
        }
        guard next != snapshot else { return }
        next.generation = snapshot.generation + 1
        snapshot = next
        observers.notify(next)
    }

    /// 검사·미리보기용: 외부에서 만든 snapshot을 그대로 반영합니다(코어 이벤트 수신 흉내).
    func receive(_ incoming: ToolsMenuSnapshot) {
        guard incoming.generation > snapshot.generation else { return }
        snapshot = incoming
        observers.notify(incoming)
    }
}
