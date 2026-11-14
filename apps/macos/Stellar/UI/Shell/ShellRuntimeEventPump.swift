import CoreGraphics
import Foundation

/// 코어 완료 event를 메인 스레드에서 pull해 요청별 완료 handler로 나눠 주는 펌프(ADR-0009, 아키텍처 §8.2).
///
/// 코어는 callback 없이 event를 큐에 쌓기만 하므로(II.4.5), 셸이 꺼내야 합니다. 이 펌프는 기다리는 요청이 있을
/// 때만 짧은 주기 timer를 켜고, 한 번에 꺼내는 event 수를 제한해 메인 스레드 점유를 짧게 둡니다(아키텍처 §6.3).
/// 기다리는 요청이 없으면 timer를 끕니다. 모든 호출은 메인 스레드에서 합니다.
@MainActor
final class ShellRuntimeEventPump {
    /// 기다리는 요청이 있을 때 event를 확인하는 주기(초).
    nonisolated static let pollInterval: TimeInterval = 1.0 / 120.0
    /// 한 번 깨어날 때 꺼내는 event 상한. 남으면 다음 주기에 이어서 꺼냅니다.
    nonisolated static let maximumEventsPerDrain = 64

    let runtimeHandle: UInt64
    private var completions: [UInt64: (CoreRuntimeEvent) -> Void] = [:]
    private var timer: Timer?
    /// 꺼냈지만 기다리는 요청이 없어 버린 event 수(검사·진단용).
    private(set) var unclaimedEventCount = 0

    init(runtimeHandle: UInt64) {
        self.runtimeHandle = runtimeHandle
    }

    /// 완료를 기다리는 요청 수.
    var pendingRequestCount: Int { completions.count }

    /// timer가 켜져 있는지(검사용).
    var isPolling: Bool { timer != nil }

    @discardableResult
    func createProject(
        path: String,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.createProject(handle: runtimeHandle, path: path), completion)
    }

    /// 프로젝트 열기를 요청하고 완료 event를 `completion`으로 받습니다.
    @discardableResult
    func openProject(
        path: String,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.openProject(handle: runtimeHandle, path: path), completion)
    }

    /// 프로젝트 session 닫기를 요청합니다. 완료 event는 `completion`이 없으면 버립니다.
    @discardableResult
    func closeProject(
        _ project: UInt64,
        completion: ((CoreRuntimeEvent) -> Void)? = nil
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.closeProject(handle: runtimeHandle, project: project), completion ?? { _ in })
    }

    /// 프로젝트 session 아래에서 창 session 열기를 요청하고 완료 event를 `completion`으로 받습니다(XII.2.2).
    @discardableResult
    func openWindowSession(
        project: UInt64,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.openWindowSession(handle: runtimeHandle, project: project), completion)
    }

    /// 창 session 닫기를 요청합니다. 완료 event는 `completion`이 없으면 버립니다.
    @discardableResult
    func closeWindowSession(
        _ session: UInt64,
        completion: ((CoreRuntimeEvent) -> Void)? = nil
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.closeWindowSession(handle: runtimeHandle, session: session), completion ?? { _ in })
    }

    /// 창 session에 명령 코드를 보내고 완료 event를 `completion`으로 받습니다(XII.6.4).
    @discardableResult
    func perform(
        session: UInt64,
        command: UInt32,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.perform(handle: runtimeHandle, session: session, command: command), completion)
    }

    func fitRoot(session: UInt64, size: CGSize, completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.fitRoot(handle: runtimeHandle, session: session, size: size), completion)
    }

    @discardableResult
    func pan(
        session: UInt64, dx: Double, dy: Double,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.pan(handle: runtimeHandle, session: session, dx: dx, dy: dy), completion)
    }

    /// 창 session의 선택을 바꾸고 완료 event를 `completion`으로 받습니다.
    @discardableResult
    func select(
        session: UInt64,
        ids: [UUID],
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.select(handle: runtimeHandle, session: session, ids: ids), completion)
    }

    @discardableResult
    func projectTrust(_ project: UInt64, decision: Bool?, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.projectTrust(handle: runtimeHandle, project: project, decision: decision), completion)
    }

    func gitNames(_ session: UInt64, selection: (UInt64, UInt32)?, stage: Bool = true, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.gitNames(handle: runtimeHandle, session: session, selection: selection, stage: stage), completion)
    }

    func insertFrame(session: UInt64, parent: CanvasFrameHit, rect: CGRect,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.insertFrame(handle: runtimeHandle, session: session, view: parent.viewID,
                                         parent: parent.id, revision: parent.revision, rect: rect), completion)
    }

    func moveFrame(session: UInt64, hit: CanvasFrameHit, delta: CGVector,
                   completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.moveFrame(handle: runtimeHandle, session: session, view: hit.viewID,
                                       node: hit.id, revision: hit.revision, delta: delta), completion)
    }

    func renameLayer(session: UInt64, view: UUID, node: UUID, revision: UInt64, name: String,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.renameLayer(handle: runtimeHandle, session: session, view: view,
                                         node: node, revision: revision, name: name), completion)
    }

    func editFrame(_ frame: CanvasInspectorFrame, change: FrameInspectorEdit, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        switch change {
        case .size(let size):
            return register(CoreRuntime.resizeFrame(handle: runtimeHandle, session: frame.session, view: frame.view,
                                                     node: frame.node, revision: frame.revision, size: size), completion)
        case .background(let color):
            return register(CoreRuntime.setFrameBackground(handle: runtimeHandle, session: frame.session, view: frame.view,
                                                           node: frame.node, revision: frame.revision, background: color), completion)
        }
    }

    private func register(
        _ result: Result<UInt64, CoreRuntimeCallError>,
        _ completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        if case .success(let request) = result {
            completions[request] = completion
            startPollingIfNeeded()
        }
        return result
    }

    #if DEBUG
    func checkWindowEdit(
        session: UInt64, node: UUID, name: String, verify: Bool = false, delayMS: UInt32 = 0,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.checkWindowEdit(
            handle: runtimeHandle, session: session, node: node, name: name, verify: verify, delayMS: delayMS
        ), completion)
    }
    #endif

    /// 지금 쌓인 event를 꺼내 완료 handler를 부릅니다. 꺼낸 event 수를 돌려줍니다.
    @discardableResult
    func drain(limit: Int = ShellRuntimeEventPump.maximumEventsPerDrain) -> Int {
        var drained = 0
        while drained < limit, let event = CoreRuntime.pollEvent(handle: runtimeHandle) {
            drained += 1
            if let request = event.request, let completion = completions.removeValue(forKey: request) {
                completion(event)
            } else {
                unclaimedEventCount += 1
            }
        }
        if completions.isEmpty {
            stopPolling()
        }
        return drained
    }

    /// 기다리는 요청이 모두 끝날 때까지 run loop를 돌립니다(검사·종료 경로 전용, 최대 `timeout`초).
    @discardableResult
    func waitForPendingRequests(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !completions.isEmpty, Date() < deadline {
            drain()
            if completions.isEmpty { break }
            RunLoop.current.run(until: Date().addingTimeInterval(Self.pollInterval))
        }
        return completions.isEmpty
    }

    private func startPollingIfNeeded() {
        guard timer == nil else { return }
        let timer = Timer(timeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                _ = self?.drain()
            }
        }
        // 메뉴 추적·modal sheet 중에도 완료를 받도록 common mode에 둡니다.
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func stopPolling() {
        timer?.invalidate()
        timer = nil
    }
}
