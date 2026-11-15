import Foundation

/// 실제 창에서 열어 본 부모만 읽습니다. UI는 디스크 또는 document thread를 기다리지 않습니다.
@MainActor
final class ShellSidebarSource: SidebarDataProvider {
    private struct Display: Equatable {
        let target: StellarSelectionTarget
        var session: UInt64 { target.session }
        var view: UUID { UUID(uuid: target.view) }
        var revision: UInt64 { target.revision }

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.session == rhs.session && lhs.view == rhs.view && lhs.revision == rhs.revision
                && lhs.target.history_sequence == rhs.target.history_sequence
        }
    }
    private struct Request: Hashable {
        let parent: SidebarStableID?
        let after: SidebarStableID?
    }
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.layers", qos: .userInitiated)
    private var runtime: UInt64?
    private var display: Display?
    private weak var window: ShellWindowSessionHandle?
    private var queued: [Request] = []
    private var loaded: [Request] = []
    private var selectedOrder: [SidebarStableID: Int] = [:]
    private var running: Request?
    var isReadingLayers: Bool { running != nil }
    private var epoch: UInt64 = 0
    private var version: UInt64 = 0
    private(set) var snapshot = SidebarSnapshot.empty
    var snapshotDidChange: (() -> Void)?
    var editSelection: ((ShellCommand, StellarSelectionTarget, [UUID]) -> Void)?
    var canEditSelection: ((ShellCommand) -> Bool)?
    var select: (([UUID]) -> Void)?
    var rename: ((_ view: UUID, _ revision: UInt64, _ id: UUID, _ name: String) -> Void)?

    func refresh(runtime: UInt64?, session: ShellWindowSessionHandle?, target: StellarSelectionTarget?) {
        self.runtime = runtime
        window = session
        guard let session, !session.isClosed, let target, session.session == target.session else {
            if display != nil || snapshot.loadState != .ready {
                display = nil
                reset()
                snapshot.loadState = .ready
                publish()
            }
            return
        }
        let next = Display(target: target)
        guard next != display else { startNext(); return }
        let selectionOnly = display?.session == next.session && display?.view == next.view && display?.revision == next.revision
        display = next
        if selectionOnly && !loaded.isEmpty {
            // 선택 변경은 사용자가 열어 둔 형제 페이지와 disclosure를 유지합니다.
            epoch &+= 1
            selectedOrder.removeAll()
            for request in loaded where !queued.contains(request) { queued.append(request) }
            if let running, !queued.contains(running) { queued.append(running) }
            startNext()
        } else {
            reset()
            enqueue(Request(parent: nil, after: nil))
        }
    }

    private func reset() {
        epoch &+= 1
        queued.removeAll()
        loaded.removeAll()
        selectedOrder.removeAll()
        snapshot = .empty
    }

    private func publish(structureChanged: Bool = true) {
        version &+= 1
        snapshot.generation = version
        if structureChanged { snapshot.layers.structureRevision = version }
        snapshotDidChange?()
    }

    func validate(_ command: SidebarCommand) -> SidebarCommandValidation {
        switch command {
        case .duplicate(let ids):
            return canEditSelection?(.duplicate) == true && ids.count == 1 && matchesSelection(ids)
                ? .allowed : .denied(.unsupported)
        case .delete(let ids):
            return canEditSelection?(.delete) == true && !ids.isEmpty && matchesSelection(ids)
                ? .allowed : .denied(.unsupported)
        case .reload: return display != nil ? .allowed : .denied(.unsupported)
        case .renameNode(let id, _):
            guard window?.isClosing == false else { return .denied(.unsupported) }
            var current: SidebarStableID? = id
            while let target = current {
                guard let row = snapshot.layers.rows[target] else { return .denied(.unknownNode) }
                if row.isLocked { return .denied(.unsupported) }
                current = row.parentID
            }
            return .allowed
        case .setSelection(let ids, _):
            return display != nil && ids.allSatisfy { snapshot.layers.rows[$0] != nil } ? .allowed : .denied(.unknownNode)
        case .loadChildren(let id):
            return snapshot.layers.rows[id]?.hasChildren == true ? .allowed : .denied(.unknownNode)
        case .loadMore(let id):
            return snapshot.layers.moreParents.contains(id) ? .allowed : .denied(.unsupported)
        default: return .denied(.unsupported)
        }
    }

    private func matchesSelection(_ ids: [SidebarStableID]) -> Bool {
        display?.target.selection_count == UInt64(ids.count) && ids.count == selectedOrder.count
            && Set(ids) == Set(selectedOrder.keys)
    }

    func perform(_ command: SidebarCommand) {
        guard validate(command).isAllowed else { return }
        switch command {
        case .duplicate(let ids):
            if let display { editSelection?(.duplicate, display.target, ids.map(\.uuid)) }
        case .delete(let ids):
            if let display { editSelection?(.delete, display.target, ids.map(\.uuid)) }
        case .reload:
            reset()
            enqueue(Request(parent: nil, after: nil))
        case .loadChildren(let id):
            if snapshot.layers.childIDs[id] == nil { enqueue(Request(parent: id, after: nil)) }
        case .renameNode(let id, let name):
            if let display, snapshot.layers.rows[id]?.name != name {
                rename?(display.view, display.revision, id.uuid, name)
            }
        case .loadMore(let parent):
            let ids = parent.map { snapshot.layers.childIDs[$0] ?? [] } ?? snapshot.layers.topLevelIDs
            if let last = ids.last { enqueue(Request(parent: parent, after: last)) }
        case .setSelection(let ids, let primary):
            // 기존 코어 API는 배열의 마지막 항목을 primary로 해석합니다.
            if let primary, ids.last != primary {
                select?((ids.filter { $0 != primary } + [primary]).map(\.uuid))
            } else {
                select?(ids.map(\.uuid))
            }
        default: break
        }
    }

    private func enqueue(_ request: Request) {
        if !queued.contains(request) { queued.append(request) }
        startNext()
    }

    private func startNext() {
        guard running == nil, !queued.isEmpty, let display, let runtime, let window, !window.isClosed else { return }
        let request = queued.removeFirst(), epoch = self.epoch
        running = request
        queue.async { [weak self] in
            let page = CoreRuntime.layerPage(handle: runtime, session: display.session, view: display.view,
                                             revision: display.revision, parent: request.parent, after: request.after)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.running = nil
                if self.epoch == epoch, self.window?.isClosed == false {
                    if let page {
                        if page.historySequence == display.target.history_sequence {
                            self.apply(page, for: request)
                        } else {
                            // Undo가 원래 선택을 복원했으면 다시 읽고, 아니면 다음 창 상태 갱신까지 기다립니다.
                            if !self.queued.contains(request) { self.queued.append(request) }
                            guard display.target.matches(self.window?.selectionTarget) else { return }
                        }
                    } else {
                        self.queued.removeAll()
                        self.snapshot.loadState = .failed(message: String(localized: "The layer list changed. Try again."))
                        self.publish()
                    }
                }
                self.startNext()
            }
        }
    }

    private func apply(_ page: WindowLayerPage, for request: Request) {
        let refreshing = loaded.contains(request)
        var ids = request.parent.map { snapshot.layers.childIDs[$0] ?? [] } ?? snapshot.layers.topLevelIDs
        // 삭제·이동 또는 같은 페이지의 중복 요청 결과를 현재 목록에 이어 붙이지 않습니다.
        guard refreshing || request.after == nil || ids.last == request.after else { return }
        snapshot.pages = [page.page]
        snapshot.activePageID = page.page.id
        snapshot.activeViewID = page.page.views.first?.id
        if !refreshing {
            loaded.append(request)
            for row in page.rows {
                ids.append(row.id)
                snapshot.layers.rows[row.id] = row
            }
            if let parent = request.parent { snapshot.layers.childIDs[parent] = ids }
            else { snapshot.layers.topLevelIDs = ids }
            if page.hasMore { snapshot.layers.moreParents.insert(request.parent) }
            else { snapshot.layers.moreParents.remove(request.parent) }
        }
        for row in page.rows { selectedOrder[row.id] = page.selected[row.id] }
        snapshot.selection = selectedOrder.sorted { $0.value < $1.value }.map(\.key)
        snapshot.primarySelection = page.primary
        snapshot.loadState = .ready
        publish(structureChanged: !refreshing)
        if let child = page.revealChild, child != page.primary, snapshot.layers.rows[child]?.hasChildren == true,
           snapshot.layers.childIDs[child] == nil {
            enqueue(Request(parent: child, after: nil))
        }
    }
}
