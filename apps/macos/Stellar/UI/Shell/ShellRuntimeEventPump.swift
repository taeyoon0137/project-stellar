import CoreGraphics
import Foundation

/// 코어 완료 event를 메인 스레드에서 pull해 요청별 완료 handler로 나눠 주는 펌프(ADR-0009, 아키텍처 §8.2).
///
/// 코어는 callback 없이 event를 큐에 쌓기만 하므로(II.4.5), 셸이 꺼내야 합니다. 이 펌프는 기다리는 요청이 있을
/// 때만 짧은 주기 timer를 켜고, 한 번에 꺼내는 event 수를 제한해 메인 스레드 점유를 짧게 둡니다(아키텍처 §6.3).
/// 기다리는 요청이 없으면 timer를 끕니다. 모든 호출은 메인 스레드에서 합니다.
///
/// 요청 없는 event(request 0, 예: 컴포넌트 원본 감시가 바꾼 창 표시)는 코어의 event 준비 신호 fd가 깨우면 꺼내
/// `unsolicitedEventHandler`로 넘깁니다(ADR-0014). 기다리는 요청이 없어도 timer를 켜지 않습니다.
@MainActor
final class ShellRuntimeEventPump {
    /// 기다리는 요청이 있을 때 event를 확인하는 주기(초).
    nonisolated static let pollInterval: TimeInterval = 1.0 / 120.0
    /// 한 번 깨어날 때 꺼내는 event 상한. 남으면 다음 주기에 이어서 꺼냅니다.
    nonisolated static let maximumEventsPerDrain = 64

    /// 요청 밖 상태 변경(자동 저장 등)을 관찰하는 주기(초, ADR-0072).
    nonisolated static let statePollInterval: TimeInterval = 0.25

    let runtimeHandle: UInt64
    private var completions: [UInt64: (CoreRuntimeEvent) -> Void] = [:]
    private var timer: Timer?
    /// 꺼냈지만 기다리는 요청이 없어 버린 event 수(검사·진단용).
    private(set) var unclaimedEventCount = 0
    /// 요청 없는 event를 받을 곳(앱 구성이 창으로 나눕니다).
    var unsolicitedEventHandler: ((CoreRuntimeEvent) -> Void)?
    private var readySource: DispatchSourceRead?

    /// 요청 밖 상태 변경 관찰자. 코어는 callback을 부르지 않으므로(ADR-0009) 관찰자 하나라도 `isWaiting`이 참인
    /// 동안만 낮은 주기로 상태 snapshot 세대를 읽고, 세대가 바뀌면 모든 관찰자의 `changed`를 부릅니다(ADR-0072).
    private struct StateObserver {
        let isWaiting: () -> Bool
        let changed: () -> Void
    }
    private var stateObservers: [UUID: StateObserver] = [:]
    private var stateTimer: Timer?
    private var observedGeneration: UInt64?

    init(runtimeHandle: UInt64) {
        self.runtimeHandle = runtimeHandle
        let fd = CoreRuntime.eventReadyFD(handle: runtimeHandle)
        guard fd >= 0 else { return }
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: .main)
        source.setEventHandler { [weak self] in
            // 신호는 "꺼낼 것이 있을 수 있음"이므로 남은 신호를 모두 읽고 한 번 꺼냅니다.
            var byte: UInt8 = 0
            while Darwin.recv(fd, &byte, 1, 0) == 1 {}
            MainActor.assumeIsolated { _ = self?.drain() }
        }
        source.setCancelHandler { Darwin.close(fd) }
        source.resume()
        readySource = source
    }

    deinit { readySource?.cancel() }

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

    func cloneProject(remote: String, target: String, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.cloneProject(handle: runtimeHandle, remote: remote, target: target), completion)
    }

    /// 프로젝트 열기를 요청하고 완료 event를 `completion`으로 받습니다.
    @discardableResult
    func openProject(
        path: String,
        completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.openProject(handle: runtimeHandle, path: path), completion)
    }

    func placePNG(_ image: PickedPNG, parent: UUID, at position: CGPoint,
                  completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.placePNG(handle: runtimeHandle, target: image.target, parent: parent, position: position,
                                      path: image.url.path, digest: image.digest), completion)
    }

    func pasteSubtree(target: StellarSelectionTarget, data: Data,
                      completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.pasteSubtree(handle: runtimeHandle, target: target, data: data), completion)
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

    func zoomAt(session: UInt64, anchor: CGPoint, fitFactor: Double?, zoomOut: Bool,
                completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.zoomAt(handle: runtimeHandle, session: session, anchor: anchor, fitFactor: fitFactor, zoomOut: zoomOut), completion)
    }

    func zoomBy(session: UInt64, anchor: CGPoint, factor: Double,
                completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.zoomBy(handle: runtimeHandle, session: session, anchor: anchor, factor: factor), completion)
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

    func editSelection(target: StellarSelectionTarget, action: UInt32, ids: [UUID]?,
                       completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.editSelection(handle: runtimeHandle, target: target, action: action, ids: ids), completion)
    }

    func insertShape(session: UInt64, parent: CanvasFrameHit, kind: InspectorShapeKind, start: CGPoint, end: CGPoint,
                     paint: CanvasShapeDefaults.Paint,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.insertShape(handle: runtimeHandle, session: session, view: parent.viewID, parent: parent.id,
                                         revision: parent.revision, kind: kind, start: start, end: end, paint: paint), completion)
    }

    func insertFrame(session: UInt64, parent: CanvasFrameHit, rect: CGRect,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.insertFrame(handle: runtimeHandle, session: session, view: parent.viewID,
                                         parent: parent.id, revision: parent.revision, rect: rect), completion)
    }

    func canvasPick(session: UInt64, view: UUID, leaf: UUID?, flags: UInt32,
                    completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasPick(handle: runtimeHandle, session: session, view: view, leaf: leaf, flags: flags), completion)
    }

    func canvasPickArea(session: UInt64, view: UUID, from: CGPoint, to: CGPoint, flags: UInt32,
                        completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasPickArea(handle: runtimeHandle, session: session, view: view, from: from, to: to,
                                            flags: flags), completion)
    }

    func canvasResize(session: UInt64, view: UUID, node: UUID, target: StellarSelectionTarget, rect: CGRect,
                      completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasResize(handle: runtimeHandle, session: session, view: view, node: node,
                                          target: target, rect: rect), completion)
    }

    func canvasRadius(session: UInt64, view: UUID, node: UUID, corner: Int?, target: StellarSelectionTarget, radius: CGFloat,
                      completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasRadius(handle: runtimeHandle, session: session, view: view, node: node, corner: corner,
                                          target: target, radius: radius), completion)
    }

    func canvasLineEnd(session: UInt64, view: UUID, node: UUID, end: Bool, target: StellarSelectionTarget, point: CGPoint,
                       completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasLineEnd(handle: runtimeHandle, session: session, view: view, node: node, end: end,
                                           target: target, point: point), completion)
    }

    func canvasMove(session: UInt64, view: UUID, anchor: UUID?, delta: CGVector, flags: UInt32,
                    completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.canvasMove(handle: runtimeHandle, session: session, view: view, anchor: anchor,
                                        delta: delta, flags: flags), completion)
    }

    func renameLayer(session: UInt64, view: UUID, node: UUID, revision: UInt64, name: String,
                     completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.renameLayer(handle: runtimeHandle, session: session, view: view,
                                         node: node, revision: revision, name: name), completion)
    }

    func toggleLayerFlag(session: UInt64, view: UUID, revision: UInt64, nodes: [UUID], flag: ShellLayerFlag,
                      completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.toggleLayerFlag(handle: runtimeHandle, session: session, view: view, revision: revision,
                                             nodes: nodes, flag: flag), completion)
    }

    func openView(session: UInt64, view: UUID, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.openView(handle: runtimeHandle, session: session, view: view), completion)
    }

    func contextSelect(target: StellarSelectionTarget, node: UUID?, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.contextSelect(handle: runtimeHandle, target: target, node: node), completion)
    }

    func reloadComponentCanvas(_ session: UInt64, completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.reloadComponentCanvas(handle: runtimeHandle, session: session), completion)
    }

    func recoverDocument(_ session: UInt64, candidate: StellarRecoveryCandidate, action: UInt32,
                         completion: @escaping (CoreRuntimeEvent) -> Void) -> Result<UInt64, CoreRuntimeCallError> {
        register(CoreRuntime.recoverDocument(handle: runtimeHandle, session: session, candidate: candidate, action: action),
                 completion)
    }

    func register(
        _ result: Result<UInt64, CoreRuntimeCallError>,
        _ completion: @escaping (CoreRuntimeEvent) -> Void
    ) -> Result<UInt64, CoreRuntimeCallError> {
        if case .success(let request) = result {
            completions[request] = completion
            startPollingIfNeeded()
        }
        return result
    }

    /// 지금 쌓인 event를 꺼내 완료 handler를 부릅니다. 꺼낸 event 수를 돌려줍니다.
    @discardableResult
    func drain(limit: Int = ShellRuntimeEventPump.maximumEventsPerDrain) -> Int {
        var drained = 0
        while drained < limit, let event = CoreRuntime.pollEvent(handle: runtimeHandle) {
            drained += 1
            if let request = event.request, let completion = completions.removeValue(forKey: request) {
                completion(event)
            } else if event.request == 0, let unsolicitedEventHandler {
                unsolicitedEventHandler(event)
            } else {
                unclaimedEventCount += 1
            }
        }
        if completions.isEmpty {
            stopPolling()
        }
        // 완료한 편집이 자동 저장 대기를 만들었을 수 있습니다.
        if drained > 0 { updateStateWatch() }
        return drained
    }

    /// 요청 밖 상태 변경을 관찰합니다. `isWaiting`이 참인 관찰자가 있는 동안 `statePollInterval`마다 상태 세대를 읽고,
    /// 바뀌면 `changed`를 부릅니다. 돌려준 token을 `removeStateObserver`에 넘기면 멈춥니다.
    func observeState(isWaiting: @escaping () -> Bool, changed: @escaping () -> Void) -> UUID {
        let token = UUID()
        stateObservers[token] = StateObserver(isWaiting: isWaiting, changed: changed)
        updateStateWatch()
        return token
    }

    func removeStateObserver(_ token: UUID) {
        stateObservers[token] = nil
        updateStateWatch()
    }

    /// 관찰을 계속할지 다시 판단합니다(요청 완료 뒤, 관찰자 등록·해제, 매 관찰 뒤).
    func updateStateWatch() {
        let waiting = stateObservers.values.contains { $0.isWaiting() }
        if !waiting {
            stateTimer?.invalidate()
            stateTimer = nil
            return
        }
        guard stateTimer == nil else { return }
        observedGeneration = CoreRuntime.state(handle: runtimeHandle)?.generation
        let timer = Timer(timeInterval: Self.statePollInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollState() }
        }
        RunLoop.main.add(timer, forMode: .common)
        stateTimer = timer
    }

    private func pollState() {
        let generation = CoreRuntime.state(handle: runtimeHandle)?.generation
        if generation != observedGeneration {
            observedGeneration = generation
            for observer in Array(stateObservers.values) { observer.changed() }
        }
        updateStateWatch()
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
