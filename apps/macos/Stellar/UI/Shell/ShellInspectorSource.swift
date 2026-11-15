import Foundation

/// 프로젝트 창의 확정 선택(모든 node 종류·다중 선택)을 속성 패널에 잇습니다(UX.2.3, ADR-0070).
///
/// snapshot은 코어가 게시한 상태에서 읽고(문서 질의 없음), 편집은 한 번에 하나씩 순서대로 보냅니다.
/// - 같은 속성의 scrub 중간 값은 앞 요청이 끝나기 전에 새 값이 오면 마지막 값만 남깁니다(값·delta 모두 조작 시작
///   기준이라 건너뛰어도 결과가 같음). 같은 속성의 반복 입력(화살표 delta, 색)도 보내기 전에 합칩니다.
/// - scrub의 끝·취소는 기다리지 않고 바로 보냅니다. 코어는 요청을 받은 순서대로 처리하므로 앞서 보낸 중간 값 뒤,
///   사용자가 이어서 하는 Undo·캔버스 선택보다 먼저 처리되어 preview가 열린 채 남지 않습니다. 선택이 사라졌으면
///   끝 대신 취소를 보냅니다(취소는 코어가 표시 상태와 무관하게 받음).
/// 편집은 신뢰와 무관합니다(D1).
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
        guard next != snapshot else { return }
        snapshot = next
        if next.count == 0 {
            // 선택이 없어지면 아직 보내지 않은 편집은 버리고 이유를 알립니다(scrub 끝·취소는 큐에 두지 않음).
            let dropped = queue
            queue.removeAll()
            for item in dropped { item.completion(String(localized: "The selection changed.")) }
        }
        snapshotHandler?(next)
    }

    func edit(_ name: String, _ input: InspectorInput, phase: InspectorEditPhase,
              completion: @escaping @MainActor (String?) -> Void) {
        if phase == .ended || phase == .cancel {
            finishScrub(name, input, phase: phase, completion: completion)
            return
        }
        guard snapshot.count > 0 else {
            completion(String(localized: "Select a layer first."))
            return
        }
        let pending = Pending(name: name, input: input, phase: phase, selection: selectionKey, completion: completion)
        if let last = queue.last, last.name == name, last.phase == phase, let merged = Self.merge(last, pending) {
            last.completion(nil)
            queue[queue.count - 1] = merged
        } else {
            queue.append(pending)
        }
        sendNext()
    }

    /// 아직 보내지 않은 같은 속성 입력 둘을 하나로 합칩니다. 합칠 수 없으면 nil.
    private static func merge(_ earlier: Pending, _ later: Pending) -> Pending? {
        switch (later.phase, earlier.input, later.input) {
        case (.continuous, _, _), (.repeated, .text, .text):
            return later
        case (.repeated, .adjust(let first), .adjust(let second)):
            return Pending(name: later.name, input: .adjust(first + second), phase: .repeated,
                           selection: later.selection, completion: later.completion)
        default:
            return nil
        }
    }

    /// scrub 끝·취소: 같은 속성의 남은 중간 값을 버리고 바로 보냅니다.
    private func finishScrub(_ name: String, _ input: InspectorInput, phase: InspectorEditPhase,
                             completion: @escaping @MainActor (String?) -> Void) {
        let superseded = queue.filter { $0.name == name && $0.phase == .continuous }
        queue.removeAll { $0.name == name && $0.phase == .continuous }
        for item in superseded { item.completion(nil) }
        guard let send else {
            completion(String(localized: "The window isn’t ready."))
            return
        }
        send(name, input, snapshot.count > 0 ? phase : .cancel, snapshot, completion)
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
