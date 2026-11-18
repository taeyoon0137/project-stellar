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
    /// 문서 revision이 바뀌기 전에 부모별로 읽어 둔 페이지 수. 새 revision에서 같은 수만큼 다시 읽습니다.
    private var restorePages: [SidebarStableID?: Int] = [:]
    private var selectedOrder: [SidebarStableID: Int] = [:]
    private var running: Request?
    private var epoch: UInt64 = 0
    private var version: UInt64 = 0
    private(set) var snapshot = SidebarSnapshot.empty
    var snapshotDidChange: (() -> Void)?
    var editSelection: ((ShellCommand, StellarSelectionTarget, [UUID]) -> Void)?
    var canEditSelection: ((ShellCommand) -> Bool)?
    var select: (([UUID]) -> Void)?
    var rename: ((_ view: UUID, _ revision: UInt64, _ id: UUID, _ name: String) -> Void)?
    var toggleFlag: ((_ view: UUID, _ revision: UInt64, _ ids: [UUID], _ flag: ShellLayerFlag) -> Void)?
    var moveLayer: ((_ view: UUID, _ revision: UInt64, _ node: UUID, _ parent: UUID?, _ index: Int) -> Void)?
    /// 머리 줄 팝업에서 고른 View로 전환합니다.
    var openView: ((UUID) -> Void)?
    /// 창 작업 문서의 Page·View 목록과 그것을 읽은 창·revision. 머리 줄 팝업이 씁니다.
    private var outline: (key: OutlineKey, pages: [SidebarPageRow])?
    private var outlineRequest: OutlineKey?
    private struct OutlineKey: Equatable {
        let session: UInt64
        let revision: UInt64
    }

    /// 빈 레이어 목록 안내 한 줄: 다음 행동을 캔버스 단일 키로 알립니다(UX 명세 흐름 1의 6번, 7.2절). 키는
    /// `CanvasTool.singleKey`에서 읽어 실제 배정과 같습니다. 버튼은 두지 않습니다.
    var emptyLayersHint: String {
        func key(_ tool: CanvasTool) -> String { tool.singleKey.map { String($0).uppercased() } ?? "" }
        let frame = key(.frame), rectangle = key(.rectangle), text = key(.text)
        return String(
            localized: "Press \(frame) to draw a frame, \(rectangle) for a rectangle, or \(text) for text.",
            comment: "Empty layer list hint; the arguments are the Frame, Rectangle, and Text tool keys"
        )
    }

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
        let previous = display
        let selectionOnly = display?.session == next.session && display?.view == next.view && display?.revision == next.revision
        display = next
        loadOutline()
        if selectionOnly && !loaded.isEmpty {
            // 선택 변경은 사용자가 열어 둔 형제 페이지와 disclosure를 유지합니다.
            epoch &+= 1
            selectedOrder.removeAll()
            for request in loaded where !queued.contains(request) { queued.append(request) }
            if let running, !queued.contains(running) { queued.append(running) }
            startNext()
        } else {
            // 같은 창·View에서 revision만 바뀌면(편집) 부모별로 이어 읽은 페이지 수를 기억해 다시 읽습니다.
            let sameView = previous?.session == next.session && previous?.view == next.view
            let pages = sameView ? Dictionary(grouping: loaded, by: \.parent).mapValues(\.count) : [:]
            reset()
            restorePages = pages
            enqueue(Request(parent: nil, after: nil))
        }
    }

    /// 창 문서 revision마다 Page·View 목록을 다시 읽습니다. 복구·외부 변경 반영처럼 같은 창에서 문서가 바뀌어도
    /// 팝업이 옛 이름·View를 보이지 않게 합니다(Page·View 수가 적어 worker 읽기 비용이 작음).
    private func loadOutline() {
        guard let display, let runtime else { return }
        let key = OutlineKey(session: display.session, revision: display.revision)
        guard outline?.key != key, outlineRequest != key else { return }
        outlineRequest = key
        queue.async { [weak self] in
            let pages = CoreRuntime.viewOutline(handle: runtime, session: key.session)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.outlineRequest == key else { return }
                self.outlineRequest = nil
                guard let pages, self.display?.session == key.session, self.display?.revision == key.revision else { return }
                self.outline = (key, pages)
                if let active = self.snapshot.activePageID, pages.contains(where: { $0.id == active }) {
                    self.snapshot.pages = self.merged(pages, active: self.activePage)
                    self.publish(structureChanged: false)
                }
            }
        }
    }

    /// 레이어 페이지가 함께 읽은 활성 Page·View 이름. 목록보다 나중에 읽은 값이므로 이름은 이 값을 우선합니다.
    private var activePage: SidebarPageRow?

    private func merged(_ pages: [SidebarPageRow], active: SidebarPageRow?) -> [SidebarPageRow] {
        guard let active, let view = active.views.first else { return pages }
        return pages.map { page in
            guard page.id == active.id else { return page }
            return SidebarPageRow(id: page.id, name: active.name,
                                  views: page.views.map { $0.id == view.id ? view : $0 })
        }
    }

    private func reset() {
        epoch &+= 1
        queued.removeAll()
        loaded.removeAll()
        restorePages.removeAll()
        selectedOrder.removeAll()
        activePage = nil
        snapshot = .empty
    }

    private func publish(structureChanged: Bool = true) {
        version &+= 1
        snapshot.generation = version
        if structureChanged { snapshot.layers.structureRevision = version }
        snapshotDidChange?()
    }

    /// 표시·잠금을 바꿀 수 있는 행들인지. 잠금 규칙(D8)은 코어가 행마다 계산한 값입니다.
    private func canSetFlag(_ ids: [SidebarStableID], _ allowed: (SidebarLayerRow) -> Bool) -> Bool {
        guard display != nil, window?.isClosing == false, !ids.isEmpty, Set(ids).count == ids.count else { return false }
        return ids.allSatisfy { snapshot.layers.rows[$0].map(allowed) == true }
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
        case .setVisible(let ids, _):
            return canSetFlag(ids, \.canHide) ? .allowed : .denied(.unsupported)
        case .setLocked(let ids, _):
            return canSetFlag(ids, \.canLock) ? .allowed : .denied(.unsupported)
        case .activateView(let id):
            return display != nil && window?.isClosing == false && id != snapshot.activeViewID
                && snapshot.pages.contains { $0.views.contains { $0.id == id } } ? .allowed : .denied(.unknownNode)
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
        case .move(let ids, let target):
            guard display != nil, window?.isClosing == false, ids.count == 1, let id = ids.first,
                  let row = snapshot.layers.rows[id], target.index >= 0 else { return .denied(.unsupported) }
            let siblings: [SidebarStableID]
            if let parent = target.parentID {
                guard let parentRow = snapshot.layers.rows[parent],
                      snapshot.layers.childIDs[parent] != nil || !parentRow.hasChildren else {
                    return .denied(.unknownNode)
                }
                siblings = snapshot.layers.childIDs[parent] ?? []
            } else {
                siblings = snapshot.layers.topLevelIDs
            }
            guard target.index <= siblings.count else { return .denied(.unknownNode) }
            var current = target.parentID
            while let parent = current {
                guard parent != id, let parentRow = snapshot.layers.rows[parent],
                      parentRow.kind == .frame || parentRow.kind == .fragment,
                      !parentRow.isLocked else { return .denied(.unsupported) }
                current = parentRow.parentID
            }
            current = row.parentID
            while let ancestor = current {
                guard let parentRow = snapshot.layers.rows[ancestor], !parentRow.isLocked else { return .denied(.unsupported) }
                current = parentRow.parentID
            }
            return row.isLocked ? .denied(.unsupported) : .allowed
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
        case .setVisible(let ids, _):
            setFlag(ids, flag: .visible, selectionCommand: .toggleVisibility)
        case .setLocked(let ids, _):
            setFlag(ids, flag: .locked, selectionCommand: .toggleLock)
        case .activateView(let id):
            openView?(id.uuid)
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
        case .move(let ids, let target):
            if let display, let id = ids.first {
                moveLayer?(display.view, display.revision, id.uuid, target.parentID?.uuid, target.index)
            }
        default: break
        }
    }

    /// 대상이 창 선택 전체이면 메뉴 막대와 같은 선택 토글로, 아니면 그 행들을 코어에서 토글합니다.
    private func setFlag(_ ids: [SidebarStableID], flag: ShellLayerFlag, selectionCommand: ShellCommand) {
        guard let display else { return }
        if matchesSelection(ids) {
            editSelection?(selectionCommand, display.target, ids.map(\.uuid))
        } else {
            toggleFlag?(display.view, display.revision, ids.map(\.uuid), flag)
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
                    } else if display.target.matches(self.window?.selectionTarget) {
                        self.queued.removeAll()
                        self.snapshot.loadState = .failed(message: String(localized: "The layer list changed. Try again."))
                        self.publish()
                    }
                    // 창이 이미 다른 revision·선택을 게시했으면 조용히 버립니다. 다음 refresh가 다시 읽습니다.
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
        activePage = page.page
        if let outline, outline.key.session == display?.session, outline.pages.contains(where: { $0.id == page.page.id }) {
            snapshot.pages = merged(outline.pages, active: page.page)
        } else {
            snapshot.pages = [page.page]
        }
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
        if !refreshing, page.hasMore, let last = ids.last {
            // 편집 전에 이어 읽은 페이지까지, 그리고 primary로 가는 행이 나올 때까지 같은 부모의 다음 페이지를 읽습니다.
            let pages = loaded.filter { $0.parent == request.parent }.count
            let restoring = pages < (restorePages[request.parent] ?? 0)
            let revealing = page.revealChild.map { !ids.contains($0) } ?? false
            if restoring || revealing { enqueue(Request(parent: request.parent, after: last)) }
        }
        if let child = page.revealChild, child != page.primary, snapshot.layers.rows[child]?.hasChildren == true,
           snapshot.layers.childIDs[child] == nil {
            enqueue(Request(parent: child, after: nil))
        }
    }
}
