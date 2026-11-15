import Foundation

/// 열린 창의 Git worker 결과를 기존 변경 트리로 투영합니다. 코어가 보관한 표시 세대와 실제 행 순번으로 Stage합니다.
@MainActor
final class ShellGitDataSource: GitPanelDataSource {
    var onEventsAvailable: (() -> Void)?
    var trustDidChange: ((Bool) -> Void)?
    private weak var session: ShellProjectSessionHandle?
    private weak var windowSession: ShellWindowSessionHandle?
    private var events: [GitPanelEvent] = []
    private(set) var trusted = false
    private var message: String?
    private var trustRequest: UInt64?
    var isCheckingTrust: Bool { trustRequest != nil }
    private var operation: UInt64?
    private var generation: UInt64 = 0
    private var items: [CoreGitChange] = []
    private var otherChanges = false
    private var panelHead: CoreGitPanelHead?

    // 충돌 가능한 해시 대신 이 창 안에서 UUID·종류별 표시 ID를 발급합니다.
    private var ids: [UUID: [Int: GitChangeNodeID]] = [:]
    private var nextID: UInt64 = 1
    private var children: [GitChangeNodeID: [GitChangeNodeID]] = [:]
    private var roots: [GitChangeNodeID] = []
    private var rows: [GitChangeNodeID: GitChangeRow] = [:]
    private var selection: [GitChangeNodeID: [Int]] = [:]

    func attach(_ session: ShellProjectSessionHandle?, window: ShellWindowSessionHandle? = nil) {
        cancel()
        self.session = session
        self.windowSession = window
        trustRequest = nil
        events.removeAll()
        ids.removeAll()
        nextID = 1
        clearRows()
        updateState(false)
        if session != nil { updateTrust(nil) }
    }

    private func clearRows() {
        generation = 0
        otherChanges = false
        panelHead = nil
        items.removeAll()
        roots.removeAll()
        children.removeAll()
        rows.removeAll()
        selection.removeAll()
    }

    private func cancel() {
        if let session, let windowSession, let operation {
            CoreRuntime.cancelGitNames(handle: session.pump.runtimeHandle, session: windowSession.session, request: operation)
        }
        operation = nil
    }

    func updateTrust(_ decision: Bool?) {
        guard let session, !session.isClosed else { return }
        if decision == false { cancel(); clearRows() }
        let result = session.pump.projectTrust(session.project, decision: decision) { [weak self, weak session] event in
            guard let self, let session, self.session === session, !session.isClosed,
                  event.request == self.trustRequest else { return }
            self.trustRequest = nil
            switch event {
            case .projectTrust(_, let project, let trusted) where project == session.project:
                self.updateState(trusted)
                if trusted { self.request(nil) }
            case .requestFailed(_, _, let message):
                self.updateState(false, message: message)
            default:
                self.updateState(false, message: String(localized: "Couldn’t verify project trust."))
            }
        }
        switch result {
        case .success(let request): trustRequest = request
        case .failure:
            trustRequest = nil
            updateState(false, message: String(localized: "Couldn’t submit the request to verify project trust."))
        }
    }

    private func updateState(_ trusted: Bool, message: String? = nil) {
        self.trusted = trusted
        self.message = message
        if !trusted { cancel(); clearRows() }
        trustDidChange?(trusted)
        emitStatus()
    }

    func send(_ command: GitPanelCommand) {
        switch command {
        case .refreshStatus:
            if session == nil || trustRequest != nil || operation != nil { emitStatus() } else { updateTrust(nil) }
        case .loadChangePage(let requested, let parent, let range):
            let available = parent.map { children[$0] ?? [] } ?? roots
            guard requested == generation, range.lowerBound >= 0, range.lowerBound < available.count else {
                return reject(command, .staleGeneration)
            }
            let end = min(range.upperBound, available.count)
            emit(.changePage(GitChangePage(generation: requested, parent: parent, startIndex: range.lowerBound,
                rows: available[range.lowerBound..<end].compactMap { rows[$0] })))
        case .loadChangeDetail(let requested, let id):
            guard requested == generation, let row = rows[id] else { return reject(command, .staleGeneration) }
            let item = (selection[id]?.count == 1) ? selection[id]?.first.map { items[$0] } : nil
            emit(.changeDetail(GitChangeDetail(generation: requested, node: id, title: row.title, kind: row.kind,
                objectPath: item?.componentPath ?? item?.node.uuidString ?? "", propertyPath: row.level == .change && item?.componentPath == nil ? "displayName" : nil,
                headValue: item?.head, indexValue: item?.index, workingValue: item?.working, requiredChanges: [])))
        case .setStaged(let requested, let targets, let stage):
            guard requested == generation else { return reject(command, .staleGeneration) }
            guard trusted, operation == nil, trustRequest == nil else { return reject(command, .unavailable) }
            var selected = IndexSet()
            for target in targets {
                let siblings = target.parent.map { children[$0] ?? [] } ?? roots
                for index in target.children {
                    guard siblings.indices.contains(index), let matches = selection[siblings[index]] else {
                        return reject(command, .staleGeneration)
                    }
                    selected.formUnion(IndexSet(matches))
                }
            }
            guard selected.count == 1, let i = selected.first,
                  stage ? items[i].unstaged : items[i].staged else {
                return reject(command, .unknownTarget)
            }
            request((requested, UInt32(i)), stage: stage)
        case .resolvePendingAutosave(let saveFirst):
            if saveFirst {
                guard let windowSession, let code = CoreRuntime.commandCode(name: "file.save_now"),
                      windowSession.perform(coreCommand: code, completion: { [weak self] _ in self?.updateTrust(nil) }) else {
                    return reject(command, .unavailable)
                }
            } else { updateTrust(nil) }
        case .cancelOperation(let id):
            guard operation == id, let session, let windowSession else { return reject(command, .unknownTarget) }
            CoreRuntime.cancelGitNames(handle: session.pump.runtimeHandle, session: windowSession.session, request: id)
        default: reject(command, .unavailable)
        }
    }

    private func reject(_ cmd: GitPanelCommand, _ reason: GitRejectionReason) {
        emit(.commandRejected(GitCommandRejection(command: cmd, reason: reason)))
    }

    private func request(_ target: (UInt64, UInt32)?, stage: Bool = true) {
        guard operation == nil, let session, let window = windowSession, !session.isClosed, !window.isClosed, trusted else { return }
        let kind: GitOperationKind = target == nil ? .refresh : (stage ? .stage : .unstage)
        let result = session.pump.gitNames(window.session, selection: target, stage: stage) { [weak self, weak session, weak window] event in
            guard let self, let session, let window, self.session === session, self.windowSession === window,
                  !window.isClosed, let id = event.request, self.operation == id else { return }
            if case .gitNames(let generation, let owner, let count, let other, let applied, let head) = event, owner == window.session {
                let handle = session.pump.runtimeHandle
                DispatchQueue.global(qos: .userInitiated).async { [weak self, weak window] in
                    let found = (0..<count).compactMap {
                        CoreRuntime.gitPanelRow(handle: handle, session: owner, generation: generation, row: $0)
                    }
                    DispatchQueue.main.async { [weak self, weak window] in
                        guard let self, let window, self.windowSession === window, !window.isClosed,
                              self.trusted, self.operation == id else { return }
                        self.operation = nil
                        if found.count == Int(count) {
                            self.clearRows()
                            self.generation = generation
                            self.items = found
                            self.otherChanges = other
                            self.panelHead = head
                            self.message = nil
                            self.rebuild()
                            self.emit(.operationFinished(GitOperationResult(id: id, kind: kind,
                                outcome: .succeeded(message: applied ? (stage ? String(localized: "Staged selected change.") : String(localized: "Unstaged selected change.")) : ""))))
                        } else {
                            self.clearRows()
                            let message = String(localized: "The Git change list changed. Refresh to read it again.")
                            self.message = message
                            self.emit(.operationFinished(GitOperationResult(id: id, kind: kind, outcome: .failed(message: message))))
                        }
                        self.emitStatus()
                    }
                }
            } else {
                self.operation = nil
                self.clearRows()
                let message: String
                if case .requestFailed(_, _, let text) = event { message = text }
                else { message = String(localized: "Couldn’t load the saved changes.") }
                self.message = message
                self.emit(.operationFinished(GitOperationResult(id: id, kind: kind, outcome: .failed(message: message))))
                self.emitStatus()
            }
        }
        if case .success(let id) = result {
            operation = id
            emit(.operationProgress(GitOperationProgress(id: id, kind: kind, fraction: nil,
                message: kind == .stage ? String(localized: "Staging selected change…") : String(localized: "Reading saved changes…"),
                isCancellable: true)))
        } else {
            message = String(localized: "Another Git operation is still running. Refresh to try again.")
            emitStatus()
        }
    }

    private func id(_ uuid: UUID, _ level: Int) -> GitChangeNodeID {
        if let id = ids[uuid]?[level] { return id }
        let id = GitChangeNodeID(rawValue: nextID)
        nextID += 1
        ids[uuid, default: [:]][level] = id
        return id
    }

    private func rebuild() {
        let byDocument = Dictionary(grouping: items.indices, by: { items[$0].document })
        let documents = byDocument.keys.compactMap { $0 }.sorted { $0.uuidString < $1.uuidString }
        for document in documents {
            guard let indices = byDocument[document] else { continue }
            let doc = id(document, 0)
            roots.append(doc)
            selection[doc] = indices
            rows[doc] = GitChangeRow(id: doc, level: .document, title: document.uuidString, detail: "",
                kind: nil, stageState: aggregate(indices), childCount: indices.count, changeCount: indices.count)
            children[doc] = indices.map { i in
                let item = items[i], object = id(item.node, 1), property = id(item.node, 2), stage = aggregate([i])
                selection[object] = [i]; selection[property] = [i]
                children[object] = [property]
                rows[object] = GitChangeRow(id: object, level: .object, title: item.working, detail: item.node.uuidString,
                    kind: nil, stageState: stage, childCount: 1, changeCount: 1)
                rows[property] = GitChangeRow(id: property, level: .change, title: String(localized: "Node name"),
                    detail: item.index == item.working ? "\(item.head) → \(item.index)" : "\(item.index) → \(item.working)",
                    kind: .setProperty, stageState: stage, childCount: 0, changeCount: 1)
                return object
            }
        }
        let components = items.indices.compactMap { i in items[i].componentPath.map { (i, $0) } }
        if !components.isEmpty {
            let group = id(CoreRuntimeEvent.nilID, 3)
            roots.append(group)
            rows[group] = GitChangeRow(id: group, level: .document, title: String(localized: "Components"), detail: "",
                kind: nil, stageState: aggregate(components.map { $0.0 }), childCount: components.count, changeCount: components.count)
            selection[group] = components.map { $0.0 }
            children[group] = components.map { i, path in
                let file = items[i], object = id(file.node, 4), source = id(file.node, 5)
                let filename = (path as NSString).lastPathComponent
                children[object] = [source]
                selection[object] = [i]
                selection[source] = [i]
                rows[object] = GitChangeRow(id: object, level: .object, title: filename, detail: file.node.uuidString,
                    kind: nil, stageState: aggregate([i]), childCount: 1, changeCount: 1)
                rows[source] = GitChangeRow(id: source, level: .change, title: String(localized: "Component source"),
                    detail: path, kind: file.head.isEmpty ? .addObject : .rawFile,
                    stageState: aggregate([i]), childCount: 0, changeCount: 1)
                return object
            }
        }
    }

    private func aggregate(_ indexes: [Int]) -> GitStageState {
        let staged = indexes.contains { items[$0].staged }
        let unstaged = indexes.contains { items[$0].unstaged }
        return staged && unstaged ? .partiallyStaged : (staged ? .staged : .unstaged)
    }

    private func emitStatus() {
        let status = GitStatusSnapshot(availability: trusted ? .ready : .restricted, head: panelHead?.state ?? .unknown,
            upstream: nil, remotes: [], identityConfigured: false, stagedChangeCount: panelHead?.initialStagedFiles.map(Int.init) ?? items.filter(\.staged).count,
            unstagedChangeCount: items.filter { $0.unstaged }.count, mergeState: .none,
            semanticConflictCount: 0, unresolvedConflictCount: 0, rawConflictCount: 0,
            stagedSnapshotIssue: nil, semanticStatusError: message ?? (otherChanges && panelHead?.initialStagedFiles == nil ? String(localized: "Some saved changes can’t be staged here.") : nil),
            pendingAutosave: (windowSession?.info?.has_unsaved_changes ?? 0) != 0,
            changes: GitChangeTreeInfo(generation: generation, structure: generation, documentCount: roots.count), conflicts: .empty)
        emit(.status(status))
    }

    private func emit(_ event: GitPanelEvent) { events.append(event); onEventsAvailable?() }
    func pollEvent() -> GitPanelEvent? { events.isEmpty ? nil : events.removeFirst() }
}
