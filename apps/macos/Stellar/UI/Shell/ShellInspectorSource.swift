import Foundation

/// 프로젝트 창의 확정 선택(모든 node 종류·다중 선택)을 속성 패널에 잇습니다(UX.2.3, ADR-0070).
///
/// snapshot은 코어가 게시한 상태에서 읽고(문서 질의 없음), 편집은 한 번에 하나씩 순서대로 보냅니다. 같은 속성의
/// 연속 조작 중간 값은 앞 요청이 끝나기 전에 새 값이 오면 마지막 값만 남깁니다(값과 delta가 모두 조작 시작 기준이라
/// 중간 값을 건너뛰어도 결과가 같음). 편집은 신뢰와 무관합니다(D1).
@MainActor
final class ShellInspectorSource: InspectorDataSource {
    private(set) var snapshot = InspectorSnapshot.empty
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)?
    /// 편집 하나를 코어로 보냅니다. 창이 연결합니다. `completion`은 성공이면 nil, 실패면 진단입니다.
    var send: ((_ name: String, _ input: InspectorInput, _ phase: InspectorEditPhase, _ basis: InspectorSnapshot,
                _ completion: @escaping @MainActor (String?) -> Void) -> Void)?

    private struct Pending {
        let name: String
        let input: InspectorInput
        let phase: InspectorEditPhase
        /// 편집을 만든 선택(이름·종류·수). 보내기 전에 선택이 바뀌었으면 버립니다.
        let selection: String
        let completion: @MainActor (String?) -> Void
    }

    private var queue: [Pending] = []
    private var inFlight = false

    private var selectionKey: String { "\(snapshot.classes.rawValue)|\(snapshot.count)|\(snapshot.displayName)|\(snapshot.view)" }

    /// 창 상태가 바뀔 때마다 부릅니다. 게시된 snapshot이 같으면 아무것도 하지 않습니다.
    func refresh(runtime: UInt64?, session: UInt64?) {
        let next = runtime.flatMap { runtime in
            session.flatMap { CoreRuntime.inspector(handle: runtime, session: $0) }
        } ?? .empty
        if next.count == 0 { queue.removeAll() }
        guard next != snapshot else { return }
        snapshot = next
        snapshotHandler?(next)
    }

    func edit(_ name: String, _ input: InspectorInput, phase: InspectorEditPhase,
              completion: @escaping @MainActor (String?) -> Void) {
        guard snapshot.count > 0 else {
            completion(String(localized: "Select a layer first."))
            return
        }
        let pending = Pending(name: name, input: input, phase: phase, selection: selectionKey, completion: completion)
        if phase == .continuous, let last = queue.last, last.phase == .continuous, last.name == name {
            queue[queue.count - 1] = pending
        } else {
            queue.append(pending)
        }
        sendNext()
    }

    private func sendNext() {
        guard !inFlight, !queue.isEmpty else { return }
        let next = queue.removeFirst()
        guard next.selection == selectionKey else {
            next.completion(String(localized: "The selection changed."))
            sendNext()
            return
        }
        guard let send else {
            next.completion(String(localized: "The window isn’t ready."))
            return
        }
        inFlight = true
        send(next.name, next.input, next.phase, snapshot) { [weak self] diagnostic in
            next.completion(diagnostic)
            guard let self else { return }
            self.inFlight = false
            self.sendNext()
        }
    }
}
