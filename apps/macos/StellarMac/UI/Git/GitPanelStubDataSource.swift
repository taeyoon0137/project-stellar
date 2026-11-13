import Foundation

// 의도적인 placeholder 구현입니다(계획 규칙: 이름에 "Stub" 포함).
//
// 코어 Git API(XIV.13.1~XIV.13.4)가 없으므로, 백그라운드 큐에서 합성 데이터로 같은 계약
// (명령 → pull 이벤트, 세대·페이지 단위 목록)을 흉내 냅니다. 실제 Git을 실행하지 않고 파일을
// 읽거나 쓰지 않습니다. 코어 연결 시 이 파일은 FFI adapter로 교체합니다.

/// Stub이 흉내 내는 저장소 상황.
struct GitPanelStubScenario {
    var name: String
    var availability: GitAvailability = .ready
    var identityConfigured = true
    var documentCount = 6
    var objectsPerDocument = 4
    var changesPerObject = 3
    /// 전역 변경 순번이 이 값의 배수면 staged로 시작합니다. 0이면 없음.
    var stagedEvery = 4
    /// 전역 변경 순번 % 값 == 값-1이면 부분 staged로 시작합니다. 0이면 없음.
    var partialEvery = 7
    /// 객체 순번이 이 값의 배수면 새 객체(첫 변경이 AddObject)입니다. 0이면 없음.
    var newObjectEvery = 3
    var includesConflicts = false
    var remotes = ["origin"]
    var hasUpstream = true
    var ahead = 2
    var behind = 0
    var pendingAutosave = false
    var semanticStatusError: String?
    /// 작업 진행 모사 간격(초).
    var stepDelay: TimeInterval = 0.02

    static let typical = GitPanelStubScenario(name: "typical", pendingAutosave: true)
    static let nothingStaged = GitPanelStubScenario(name: "nothing-staged", stagedEvery: 0, partialEvery: 0)
    static let identityMissing = GitPanelStubScenario(name: "identity-missing", identityConfigured: false)
    static let restricted = GitPanelStubScenario(name: "restricted", availability: .restricted)
    static let clean = GitPanelStubScenario(name: "clean", documentCount: 0, ahead: 0)
    static let conflicts = GitPanelStubScenario(name: "conflicts", includesConflicts: true)
    /// 100만 semantic change(1,000 문서 × 100 객체 × 10 변경). XIV.13.1 pagination 규모입니다.
    static let million = GitPanelStubScenario(
        name: "million",
        documentCount: 1_000,
        objectsPerDocument: 100,
        changesPerObject: 10,
        stagedEvery: 5,
        partialEvery: 0,
        newObjectEvery: 0
    )
}

/// `GitPanelDataSource`의 Stub 구현. 메인 스레드 쪽 adapter입니다.
@MainActor
final class GitPanelStubDataSource: GitPanelDataSource {
    var onEventsAvailable: (() -> Void)?
    private let backend: GitStubBackend
    /// 검사용: 보낸 명령 기록(최근 256개).
    private(set) var sentCommands: [GitPanelCommand] = []

    init(scenario: GitPanelStubScenario = .typical) {
        backend = GitStubBackend(scenario: scenario)
        backend.setWakeup { [weak self] in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.onEventsAvailable?() }
            }
        }
    }

    func send(_ command: GitPanelCommand) {
        sentCommands.append(command)
        if sentCommands.count > 256 { sentCommands.removeFirst(sentCommands.count - 256) }
        backend.submit(command)
    }

    func pollEvent() -> GitPanelEvent? {
        backend.popEvent()
    }
}

// MARK: - 백그라운드 "코어" 모사

/// 큐 하나에서만 상태를 바꾸는 Stub backend. 이벤트 큐만 lock으로 보호합니다.
final class GitStubBackend: @unchecked Sendable {
    private let queue = DispatchQueue(label: "dev.stellar.git-panel-stub", qos: .userInitiated)
    private let lock = NSLock()
    private var outbox: [GitPanelEvent] = []
    private var outboxHead = 0
    private var wakeScheduled = false
    private var wakeup: (() -> Void)?
    private var cancelledOperations: Set<UInt64> = []

    // 아래 상태는 queue에서만 접근합니다.
    private let scenario: GitPanelStubScenario
    private var store: GitStubChangeStore?
    private var conflicts: [GitConflictRow]
    private var changeGeneration: UInt64 = 1
    private var conflictGeneration: UInt64 = 1
    private var nextOperationID: UInt64 = 1
    private var nextPromptID: UInt64 = 1
    private var pendingStage: (id: UInt64, changes: IndexSet, required: IndexSet, staged: Bool)?
    private var headHash = "3f9a2c1"
    private var commitCounter = 0
    private var ahead: Int
    private var hasUpstream: Bool
    private var pendingAutosave: Bool
    private var mergeState: GitMergeState

    init(scenario: GitPanelStubScenario) {
        self.scenario = scenario
        ahead = scenario.ahead
        hasUpstream = scenario.hasUpstream
        pendingAutosave = scenario.pendingAutosave
        conflicts = scenario.includesConflicts ? GitStubBackend.sampleConflicts() : []
        mergeState = scenario.includesConflicts ? .merging : .none
    }

    func setWakeup(_ handler: @escaping () -> Void) {
        lock.lock()
        wakeup = handler
        lock.unlock()
    }

    func submit(_ command: GitPanelCommand) {
        if case .cancelOperation(let id) = command {
            lock.lock()
            cancelledOperations.insert(id)
            lock.unlock()
            return
        }
        queue.async { self.handle(command) }
    }

    func popEvent() -> GitPanelEvent? {
        lock.lock()
        defer { lock.unlock() }
        guard outboxHead < outbox.count else {
            outbox.removeAll(keepingCapacity: true)
            outboxHead = 0
            wakeScheduled = false
            return nil
        }
        let event = outbox[outboxHead]
        outboxHead += 1
        return event
    }

    private func post(_ event: GitPanelEvent) {
        lock.lock()
        outbox.append(event)
        let handler = wakeScheduled ? nil : wakeup
        wakeScheduled = true
        lock.unlock()
        handler?()
    }

    private func isCancelled(_ id: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelledOperations.contains(id)
    }

    // MARK: 명령 처리

    private var isReady: Bool { scenario.availability == .ready }

    private func handle(_ command: GitPanelCommand) {
        switch command {
        case .refreshStatus:
            if isReady, store == nil { store = GitStubChangeStore(scenario: scenario) }
            post(.status(makeStatus()))

        case .loadChangePage(let generation, let rows):
            guard generation == changeGeneration, let store else { return }
            let upper = min(rows.upperBound, store.rowCount)
            guard rows.lowerBound < upper else { return }
            let page = (rows.lowerBound..<upper).map(store.row(at:))
            post(.changePage(GitChangePage(generation: generation, startRow: rows.lowerBound, rows: page)))

        case .loadChangeDetail(let generation, let row):
            guard generation == changeGeneration, let store, row < store.rowCount else { return }
            post(.changeDetail(store.detail(at: row, generation: generation)))

        case .setStaged(let generation, let rows, let staged):
            requestStage(generation: generation, rows: rows, staged: staged)

        case .setGroupExpanded(let generation, let row, let expanded):
            guard generation == changeGeneration, let store else { return }
            store.setExpanded(row: row, expanded: expanded)
            changeGeneration += 1
            post(.status(makeStatus()))

        case .answerDependencyPrompt(let requestID, let include):
            guard let pending = pendingStage, pending.id == requestID else { return }
            pendingStage = nil
            if include {
                runStage(pending.changes.union(pending.required), staged: pending.staged)
            } else {
                let id = beginOperation(pending.staged ? .stage : .unstage, message: "", cancellable: false)
                finish(id, pending.staged ? .stage : .unstage, .cancelled)
            }

        case .commit(let message):
            commit(message: message)

        case .push(let remote):
            push(remote: remote)

        case .loadConflictPage(let generation, let rows):
            guard generation == conflictGeneration else { return }
            let upper = min(rows.upperBound, conflicts.count)
            guard rows.lowerBound < upper else { return }
            post(.conflictPage(GitConflictPage(
                generation: generation,
                startRow: rows.lowerBound,
                rows: Array(conflicts[rows.lowerBound..<upper])
            )))

        case .chooseResolution(let generation, let rows, let choice):
            guard isReady, generation == conflictGeneration else { return }
            for row in rows where row < conflicts.count && conflicts[row].allowedChoices.contains(choice) {
                conflicts[row].choice = choice
            }
            conflictGeneration += 1
            post(.status(makeStatus()))

        case .applyResolutions:
            applyResolutions()

        case .revealConflict:
            // 캔버스 선택 연결은 코어·캔버스 작업의 몫입니다. Stub은 기록만 합니다.
            break

        case .resolvePendingAutosave(let saveFirst):
            pendingAutosave = false
            if saveFirst {
                let id = beginOperation(.saveBeforeStatus, message: "Saving…", cancellable: false)
                post(.status(makeStatus()))
                finish(id, .saveBeforeStatus, .succeeded(message: "Saved."))
            } else {
                post(.status(makeStatus()))
            }

        case .cancelOperation:
            break
        }
    }

    private func beginOperation(_ kind: GitOperationKind, message: String, cancellable: Bool) -> UInt64 {
        let id = nextOperationID
        nextOperationID += 1
        post(.operationProgress(GitOperationProgress(id: id, kind: kind, fraction: nil, message: message, isCancellable: cancellable)))
        return id
    }

    private func finish(_ id: UInt64, _ kind: GitOperationKind, _ outcome: GitOperationOutcome) {
        post(.operationFinished(GitOperationResult(id: id, kind: kind, outcome: outcome)))
    }

    private func fail(_ kind: GitOperationKind, _ message: String) {
        let id = beginOperation(kind, message: "", cancellable: false)
        finish(id, kind, .failed(message: message))
    }

    private func requestStage(generation: UInt64, rows: IndexSet, staged: Bool) {
        let kind: GitOperationKind = staged ? .stage : .unstage
        guard isReady, let store else { return fail(kind, "Git isn't available for this project.") }
        guard generation == changeGeneration else {
            return fail(kind, "The change list was updated. Try again.")
        }
        let changes = store.changeIndexes(forRows: rows)
        let required = staged ? store.requiredForStaging(changes) : store.requiredForUnstaging(changes)
        if !required.isEmpty {
            let id = nextPromptID
            nextPromptID += 1
            pendingStage = (id, changes, required, staged)
            post(.dependencyPrompt(GitDependencyPrompt(
                requestID: id,
                isStaging: staged,
                requiredCount: required.count,
                examples: store.summaries(of: required, limit: 3)
            )))
            return
        }
        runStage(changes, staged: staged)
    }

    private func runStage(_ changes: IndexSet, staged: Bool) {
        guard let store else { return }
        let kind: GitOperationKind = staged ? .stage : .unstage
        let id = beginOperation(kind, message: staged ? "Staging…" : "Unstaging…", cancellable: false)
        let count = store.setStaged(changes, staged: staged)
        changeGeneration += 1
        post(.status(makeStatus()))
        let noun = count == 1 ? "1 change" : "\(count) changes"
        finish(id, kind, .succeeded(message: staged ? "Staged \(noun)." : "Unstaged \(noun)."))
    }

    private func commit(message: String) {
        guard isReady, let store else { return fail(.commit, "Git isn't available for this project.") }
        guard scenario.identityConfigured else {
            return fail(.commit, "Git needs your name and email (user.name and user.email) to commit.")
        }
        guard conflicts.isEmpty else { return fail(.commit, "Resolve conflicts before committing.") }
        guard store.stagedTotal > 0 else { return fail(.commit, "There are no staged changes to commit.") }
        guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return fail(.commit, "Enter a commit message.")
        }
        let id = beginOperation(.commit, message: "Committing…", cancellable: false)
        queue.asyncAfter(deadline: .now() + scenario.stepDelay) {
            store.commitStaged()
            self.commitCounter += 1
            self.headHash = String(format: "%07x", 0x3f9a2c1 &+ self.commitCounter &* 0x1f3d)
            self.ahead += 1
            self.changeGeneration += 1
            self.post(.status(self.makeStatus()))
            self.finish(id, .commit, .succeeded(message: "Committed \(self.headHash)."))
        }
    }

    private func push(remote: String) {
        guard isReady else { return fail(.push, "Git isn't available for this project.") }
        guard scenario.remotes.contains(remote) else { return fail(.push, "Remote “\(remote)” isn't set up.") }
        guard ahead > 0 || !hasUpstream else { return fail(.push, "No commits to push.") }
        let id = beginOperation(.push, message: "Pushing to \(remote)…", cancellable: true)
        pushStep(id: id, remote: remote, step: 1, steps: 5)
    }

    private func pushStep(id: UInt64, remote: String, step: Int, steps: Int) {
        queue.asyncAfter(deadline: .now() + scenario.stepDelay) {
            if self.isCancelled(id) {
                self.finish(id, .push, .cancelled)
                return
            }
            if step > steps {
                self.ahead = 0
                self.hasUpstream = true
                self.post(.status(self.makeStatus()))
                self.finish(id, .push, .succeeded(message: "Pushed to \(remote)."))
                return
            }
            let fraction = Double(step) / Double(steps + 1)
            self.post(.operationProgress(GitOperationProgress(
                id: id,
                kind: .push,
                fraction: fraction,
                message: "Pushing to \(remote)… \(Int(fraction * 100))%",
                isCancellable: true
            )))
            self.pushStep(id: id, remote: remote, step: step + 1, steps: steps)
        }
    }

    private func applyResolutions() {
        guard isReady else { return fail(.applyResolutions, "Git isn't available for this project.") }
        let semantic = conflicts.filter { $0.kind != .raw }
        guard !semantic.isEmpty else { return fail(.applyResolutions, "There are no semantic conflicts to apply.") }
        guard semantic.allSatisfy({ $0.choice != nil }) else {
            return fail(.applyResolutions, "Choose a resolution for every conflict first.")
        }
        let id = beginOperation(.applyResolutions, message: "Applying resolutions…", cancellable: false)
        queue.asyncAfter(deadline: .now() + scenario.stepDelay) {
            self.conflicts.removeAll { $0.kind != .raw }
            self.conflictGeneration += 1
            self.post(.status(self.makeStatus()))
            let noun = semantic.count == 1 ? "1 conflict" : "\(semantic.count) conflicts"
            self.finish(id, .applyResolutions, .succeeded(message: "Resolved \(noun). Continue the merge in Terminal."))
        }
    }

    private func makeStatus() -> GitStatusSnapshot {
        let semantic = conflicts.filter { $0.kind != .raw }
        let raw = conflicts.count - semantic.count
        let ready = isReady
        return GitStatusSnapshot(
            availability: scenario.availability,
            head: ready ? .branch(name: "main") : .unknown,
            upstream: hasUpstream ? GitUpstreamInfo(remote: "origin", branch: "main", ahead: ahead, behind: scenario.behind) : nil,
            remotes: scenario.remotes,
            identityConfigured: scenario.identityConfigured,
            stagedChangeCount: ready ? store?.stagedTotal ?? 0 : 0,
            unstagedChangeCount: ready ? store?.unstagedTotal ?? 0 : 0,
            mergeState: mergeState,
            semanticConflictCount: ready ? semantic.count : 0,
            unresolvedConflictCount: ready ? semantic.filter { $0.choice == nil }.count : 0,
            rawConflictCount: ready ? raw : 0,
            stagedSnapshotIssue: nil,
            semanticStatusError: scenario.semanticStatusError,
            pendingAutosave: ready && pendingAutosave,
            changes: GitListInfo(generation: changeGeneration, rowCount: ready ? store?.rowCount ?? 0 : 0),
            conflicts: GitListInfo(generation: conflictGeneration, rowCount: ready ? conflicts.count : 0)
        )
    }

    private static func sampleConflicts() -> [GitConflictRow] {
        let all: Set<GitConflictChoice> = [.base, .ours, .theirs]
        return [
            GitConflictRow(title: "Button/Primary", path: "Home / Button/Primary / fill.color", kind: .propertyValue,
                           allowedChoices: all, choice: nil,
                           baseSummary: "Blue 500", oursSummary: "Blue 600", theirsSummary: "Indigo 500"),
            GitConflictRow(title: "Card", path: "Home / Card", kind: .deleteVersusModify,
                           allowedChoices: [.ours, .theirs], choice: nil,
                           baseSummary: "Present", oursSummary: "Deleted", theirsSummary: "corner.radius 4 → 8"),
            GitConflictRow(title: "color/accent", path: "Tokens / color/accent", kind: .typeVersusValue,
                           allowedChoices: all, choice: nil,
                           baseSummary: "Color · Blue 500", oursSummary: "Gradient · Blue → Indigo",
                           theirsSummary: "Color · Blue 700"),
            GitConflictRow(title: "Tab Bar", path: "Home / Tab Bar / children", kind: .conflictingReorder,
                           allowedChoices: all, choice: nil,
                           baseSummary: "Home, Search, Profile", oursSummary: "Search, Home, Profile",
                           theirsSummary: "Home, Profile, Search"),
            GitConflictRow(title: "Avatar", path: "Components / Avatar / size", kind: .componentPropertyRemoved,
                           allowedChoices: [.ours, .theirs], choice: nil,
                           baseSummary: "Defined", oursSummary: "Removed", theirsSummary: "Used by 3 instances"),
            GitConflictRow(title: "README.md", path: "README.md", kind: .raw,
                           allowedChoices: [], choice: nil,
                           baseSummary: nil, oursSummary: nil, theirsSummary: nil),
        ]
    }
}

// MARK: - 합성 변경 저장소

/// 문서 → 객체 → 변경 3단 합성 데이터와 평탄화 색인. queue 하나에서만 씁니다.
final class GitStubChangeStore {
    struct Change {
        var kind: GitChangeKind
        var state: GitStageState
        var object: Int32
        var variant: UInt16
    }

    struct Object {
        var document: Int32
        var nameIndex: Int32
        var changes: Range<Int>
        var isNew: Bool
        var collapsed = false
        /// staged + 부분 staged 수.
        var stagedCount = 0
        /// unstaged + 부분 staged 수.
        var unstagedCount = 0
    }

    struct Document {
        var nameIndex: Int
        var objects: Range<Int>
        var collapsed = false
    }

    enum RowRef {
        case document(Int)
        case object(Int)
        case change(Int)
    }

    private(set) var changes: [Change] = []
    private(set) var objects: [Object] = []
    private(set) var documents: [Document] = []
    private var documentRowStart: [Int] = [0]
    /// 객체 머리 행의 문서 머리 행 기준 상대 위치.
    private var objectRowOffset: [Int] = []
    private(set) var stagedTotal = 0
    private(set) var unstagedTotal = 0

    var rowCount: Int { documentRowStart.last ?? 0 }

    private static let documentNames = ["Home", "Onboarding", "Settings", "Profile", "Checkout", "Components"]
    private static let objectNames = ["Header", "Button/Primary", "Card", "Avatar", "Tab Bar", "Title", "List Row", "Badge"]
    private static let properties: [(path: String, head: String, index: String, working: String)] = [
        ("fill.color", "Blue 500", "Blue 550", "Blue 600"),
        ("layout.padding.top", "8", "10", "12"),
        ("corner.radius", "4", "6", "8"),
        ("text.content", "“Continue”", "“Next”", "“Next step”"),
        ("opacity", "100%", "90%", "80%"),
        ("stroke.width", "1", "1.5", "2"),
        ("binding.source", "tokens.color.accent", "tokens.color.brand", "props.tint"),
        ("variant.state", "default", "hover", "pressed"),
    ]
    private static let kindCycle: [GitChangeKind] = [
        .setProperty, .setProperty, .setProperty, .reorderObject, .setProperty, .moveObject,
        .setProperty, .changeToken, .setProperty, .componentInterface, .setProperty, .assetReference,
    ]

    init(scenario: GitPanelStubScenario) {
        let total = scenario.documentCount * scenario.objectsPerDocument * scenario.changesPerObject
        changes.reserveCapacity(total)
        objects.reserveCapacity(scenario.documentCount * scenario.objectsPerDocument)
        documents.reserveCapacity(scenario.documentCount)
        var globalObject = 0
        for documentIndex in 0..<scenario.documentCount {
            let firstObject = objects.count
            for objectIndex in 0..<scenario.objectsPerDocument {
                let isNew = scenario.newObjectEvery > 0 && globalObject % scenario.newObjectEvery == 0
                let first = changes.count
                for changeIndex in 0..<scenario.changesPerObject {
                    let global = changes.count
                    let kind: GitChangeKind = isNew && changeIndex == 0
                        ? .addObject
                        : Self.kindCycle[(global &+ objectIndex) % Self.kindCycle.count]
                    var state = GitStageState.unstaged
                    if scenario.partialEvery > 0, global % scenario.partialEvery == scenario.partialEvery - 1 {
                        state = .partiallyStaged
                    } else if scenario.stagedEvery > 0, global % scenario.stagedEvery == 0 {
                        state = .staged
                    }
                    changes.append(Change(kind: kind, state: state, object: Int32(objects.count), variant: UInt16(global % 997)))
                }
                var object = Object(
                    document: Int32(documentIndex),
                    nameIndex: Int32(objectIndex),
                    changes: first..<changes.count,
                    isNew: isNew
                )
                // 새 객체의 속성이 Index에 있으면 AddObject도 Index에 있어야 일관됩니다.
                if isNew, changes[(first + 1)..<changes.count].contains(where: { $0.state != .unstaged }) {
                    changes[first].state = .staged
                }
                for change in changes[object.changes] {
                    let (staged, unstaged) = Self.contribution(change.state)
                    object.stagedCount += staged
                    object.unstagedCount += unstaged
                }
                stagedTotal += object.stagedCount
                unstagedTotal += object.unstagedCount
                objects.append(object)
                globalObject += 1
            }
            documents.append(Document(nameIndex: documentIndex, objects: firstObject..<objects.count))
        }
        rebuildIndex()
    }

    private static func contribution(_ state: GitStageState) -> (staged: Int, unstaged: Int) {
        switch state {
        case .unstaged: return (0, 1)
        case .staged: return (1, 0)
        case .partiallyStaged: return (1, 1)
        }
    }

    private static func aggregate(staged: Int, unstaged: Int) -> GitStageState {
        if staged == 0 { return .unstaged }
        if unstaged == 0 { return .staged }
        return .partiallyStaged
    }

    private func rebuildIndex() {
        documentRowStart = [0]
        documentRowStart.reserveCapacity(documents.count + 1)
        objectRowOffset = Array(repeating: 1, count: objects.count)
        var total = 0
        for document in documents {
            var rows = 1
            if !document.collapsed {
                for index in document.objects {
                    objectRowOffset[index] = rows
                    rows += 1 + (objects[index].collapsed ? 0 : objects[index].changes.count)
                }
            }
            total += rows
            documentRowStart.append(total)
        }
    }

    func resolve(_ row: Int) -> RowRef? {
        guard row >= 0, row < rowCount, !documents.isEmpty else { return nil }
        var low = 0
        var high = documents.count - 1
        while low < high {
            let mid = (low + high + 1) / 2
            if documentRowStart[mid] <= row { low = mid } else { high = mid - 1 }
        }
        let document = low
        let relative = row - documentRowStart[document]
        if relative == 0 { return .document(document) }
        let range = documents[document].objects
        var first = range.lowerBound
        var last = range.upperBound - 1
        while first < last {
            let mid = (first + last + 1) / 2
            if objectRowOffset[mid] <= relative { first = mid } else { last = mid - 1 }
        }
        let inner = relative - objectRowOffset[first]
        if inner == 0 { return .object(first) }
        return .change(objects[first].changes.lowerBound + inner - 1)
    }

    private func documentName(_ index: Int) -> String {
        let names = Self.documentNames
        return index < names.count ? names[index] : "\(names[index % names.count]) \(index / names.count + 1)"
    }

    private func objectName(_ object: Object) -> String {
        let names = Self.objectNames
        let index = Int(object.nameIndex)
        return index < names.count ? names[index] : "\(names[index % names.count]) \(index / names.count + 1)"
    }

    private func documentChanges(_ document: Document) -> Range<Int> {
        objects[document.objects.lowerBound].changes.lowerBound..<objects[document.objects.upperBound - 1].changes.upperBound
    }

    private func documentCounts(_ document: Document) -> (staged: Int, unstaged: Int, total: Int) {
        var staged = 0
        var unstaged = 0
        for index in document.objects {
            staged += objects[index].stagedCount
            unstaged += objects[index].unstagedCount
        }
        return (staged, unstaged, documentChanges(document).count)
    }

    private func changeText(_ change: Change) -> (title: String, detail: String) {
        let property = Self.properties[Int(change.variant) % Self.properties.count]
        let variant = Int(change.variant)
        switch change.kind {
        case .addObject: return (GitPanelStrings.kind(.addObject), "")
        case .removeObject: return (GitPanelStrings.kind(.removeObject), "")
        case .moveObject: return (GitPanelStrings.kind(.moveObject), "Group \(variant % 9 + 1) → Group \(variant % 9 + 2)")
        case .reorderObject: return (GitPanelStrings.kind(.reorderObject), "Position \(variant % 5 + 1) → \(variant % 5 + 2)")
        case .setProperty:
            let working = change.state == .staged ? property.index : property.working
            return (property.path, "\(property.head) → \(working)")
        case .addToken: return ("spacing/xl", "Added")
        case .removeToken: return ("radius/legacy", "Removed")
        case .changeToken: return ("color/accent", "Blue 500 → Indigo 500")
        case .componentInterface: return ("Property “size”", "Added")
        case .assetReference: return ("hero.png", "→ hero-wide.png")
        case .rawFile: return ("README.md", "Modified")
        }
    }

    func row(at index: Int) -> GitChangeRow {
        switch resolve(index) {
        case .document(let documentIndex)?:
            let document = documents[documentIndex]
            let counts = documentCounts(document)
            return GitChangeRow(
                level: .document,
                title: documentName(document.nameIndex),
                detail: GitPanelStrings.changeCount(counts.total),
                kind: nil,
                stageState: Self.aggregate(staged: counts.staged, unstaged: counts.unstaged),
                isExpanded: !document.collapsed,
                changeCount: counts.total
            )
        case .object(let objectIndex)?:
            let object = objects[objectIndex]
            let count = GitPanelStrings.changeCount(object.changes.count)
            return GitChangeRow(
                level: .object,
                title: objectName(object),
                detail: object.isNew ? "New · \(count)" : count,
                kind: nil,
                stageState: Self.aggregate(staged: object.stagedCount, unstaged: object.unstagedCount),
                isExpanded: !object.collapsed,
                changeCount: object.changes.count
            )
        case .change(let changeIndex)?:
            let change = changes[changeIndex]
            let text = changeText(change)
            return GitChangeRow(
                level: .change,
                title: text.title,
                detail: text.detail,
                kind: change.kind,
                stageState: change.state,
                isExpanded: false,
                changeCount: 1
            )
        case nil:
            return GitChangeRow(level: .change, title: "", detail: "", kind: nil, stageState: .unstaged, isExpanded: false, changeCount: 0)
        }
    }

    func detail(at row: Int, generation: UInt64) -> GitChangeDetail {
        let base = self.row(at: row)
        guard case .change(let changeIndex)? = resolve(row) else {
            return GitChangeDetail(generation: generation, row: row, title: base.title, kind: nil,
                                   objectPath: base.title, propertyPath: nil, headValue: nil, indexValue: nil,
                                   workingValue: nil, requiredChanges: [])
        }
        let change = changes[changeIndex]
        let object = objects[Int(change.object)]
        let path = "\(documentName(Int(object.document))) / \(objectName(object))"
        let property = Self.properties[Int(change.variant) % Self.properties.count]
        var head: String? = property.head
        var index: String?
        var working: String?
        switch change.state {
        case .unstaged: working = property.working
        case .staged: index = property.index
        case .partiallyStaged:
            index = property.index
            working = property.working
        }
        if change.kind == .addObject { head = "(none)" }
        var required: [String] = []
        if object.isNew, change.kind != .addObject, changes[object.changes.lowerBound].state == .unstaged {
            required.append("Add \(objectName(object))")
        }
        return GitChangeDetail(
            generation: generation,
            row: row,
            title: base.title,
            kind: change.kind,
            objectPath: path,
            propertyPath: change.kind == .setProperty ? property.path : nil,
            headValue: head,
            indexValue: index,
            workingValue: working,
            requiredChanges: required
        )
    }

    /// 행 선택을 변경 순번 집합으로 바꿉니다. 묶음 행은 접혀 있어도 하위 전체를 포함합니다.
    func changeIndexes(forRows rows: IndexSet) -> IndexSet {
        if rows.count >= rowCount { return IndexSet(integersIn: 0..<changes.count) }
        var result = IndexSet()
        for range in rows.rangeView {
            for row in range {
                switch resolve(row) {
                case .document(let index)?: result.insert(integersIn: documentChanges(documents[index]))
                case .object(let index)?: result.insert(integersIn: objects[index].changes)
                case .change(let index)?: result.insert(index)
                case nil: break
                }
            }
        }
        return result
    }

    /// 새 객체의 속성을 Stage하려면 AddObject도 Index에 있어야 합니다(XIV.5.3 모사).
    func requiredForStaging(_ targets: IndexSet) -> IndexSet {
        var required = IndexSet()
        for index in targets {
            let change = changes[index]
            let object = objects[Int(change.object)]
            guard object.isNew, change.kind != .addObject else { continue }
            let add = object.changes.lowerBound
            if changes[add].state == .unstaged, !targets.contains(add) { required.insert(add) }
        }
        return required
    }

    /// 새 객체의 AddObject를 Unstage하면 그 객체의 staged 속성도 함께 빠져야 합니다(XIV.7.2 모사).
    func requiredForUnstaging(_ targets: IndexSet) -> IndexSet {
        var required = IndexSet()
        for index in targets where changes[index].kind == .addObject {
            let object = objects[Int(changes[index].object)]
            for other in object.changes where other != index
                && changes[other].state != .unstaged && !targets.contains(other) {
                required.insert(other)
            }
        }
        return required
    }

    func summaries(of indexes: IndexSet, limit: Int) -> [String] {
        indexes.prefix(limit).map { index in
            let change = changes[index]
            let object = objects[Int(change.object)]
            return change.kind == .addObject ? "Add \(objectName(object))" : "\(objectName(object)) \(changeText(change).title)"
        }
    }

    /// 상태를 바꾸고 실제로 바뀐 변경 수를 반환합니다.
    @discardableResult
    func setStaged(_ targets: IndexSet, staged: Bool) -> Int {
        let target: GitStageState = staged ? .staged : .unstaged
        var changed = 0
        for index in targets where changes[index].state != target {
            let objectIndex = Int(changes[index].object)
            let old = Self.contribution(changes[index].state)
            let new = Self.contribution(target)
            objects[objectIndex].stagedCount += new.staged - old.staged
            objects[objectIndex].unstagedCount += new.unstaged - old.unstaged
            stagedTotal += new.staged - old.staged
            unstagedTotal += new.unstaged - old.unstaged
            changes[index].state = target
            changed += 1
        }
        return changed
    }

    func setExpanded(row: Int, expanded: Bool) {
        switch resolve(row) {
        case .document(let index)?: documents[index].collapsed = !expanded
        case .object(let index)?: objects[index].collapsed = !expanded
        default: return
        }
        rebuildIndex()
    }

    /// staged 변경은 목록에서 빠지고, 부분 staged는 unstaged로 남습니다(커밋 후 Index == HEAD).
    func commitStaged() {
        var newChanges: [Change] = []
        var newObjects: [Object] = []
        var newDocuments: [Document] = []
        newChanges.reserveCapacity(changes.count)
        stagedTotal = 0
        unstagedTotal = 0
        for document in documents {
            let firstObject = newObjects.count
            for objectIndex in document.objects {
                var object = objects[objectIndex]
                let first = newChanges.count
                for var change in changes[object.changes] where change.state != .staged {
                    change.state = .unstaged
                    change.object = Int32(newObjects.count)
                    newChanges.append(change)
                }
                guard newChanges.count > first else { continue }
                object.changes = first..<newChanges.count
                object.isNew = object.isNew && newChanges[first].kind == .addObject
                object.stagedCount = 0
                object.unstagedCount = object.changes.count
                unstagedTotal += object.unstagedCount
                newObjects.append(object)
            }
            guard newObjects.count > firstObject else { continue }
            var kept = document
            kept.objects = firstObject..<newObjects.count
            newDocuments.append(kept)
        }
        changes = newChanges
        objects = newObjects
        documents = newDocuments
        rebuildIndex()
    }
}
