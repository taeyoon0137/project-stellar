import Foundation

// 의도적인 placeholder 구현입니다(계획 규칙: 이름에 "Stub" 포함).
//
// 코어 Git API(XIV.13.1~XIV.13.4)가 없으므로, 백그라운드 큐에서 합성 데이터로 같은 계약
// (명령 → pull 이벤트, 세대·부모별 페이지 목록, 처리하지 않은 명령의 거부 이벤트)을 흉내 냅니다.
// 실제 Git을 실행하지 않고 파일을 읽거나 쓰지 않습니다. 코어 연결 시 이 파일은 FFI adapter로 교체합니다.

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
    /// 한 문서 아래 자식이 페이지 여러 개인 경우(1 문서 × 2,000 객체 × 2 변경). 부모별 페이지 경로를 확인합니다.
    static let wide = GitPanelStubScenario(
        name: "wide",
        documentCount: 1,
        objectsPerDocument: 2_000,
        changesPerObject: 2,
        stagedEvery: 0,
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
    /// 검사용: 이벤트를 하나 꺼낼 때마다 메인 스레드를 이 시간(초)만큼 점유합니다(느린 디코딩 모사). 0이면 없음.
    var pollDelay: TimeInterval = 0
    /// 검사용: 아직 꺼내지 않은 이벤트 수.
    var queuedEventCount: Int { backend.queuedEventCount }

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
        guard let event = backend.popEvent() else { return nil }
        if pollDelay > 0 {
            let end = ProcessInfo.processInfo.systemUptime + pollDelay
            while ProcessInfo.processInfo.systemUptime < end {}
        }
        return event
    }
}

// MARK: - 백그라운드 "코어" 모사

/// 큐 하나에서만 상태를 바꾸는 Stub backend. 이벤트 큐만 lock으로 보호합니다.
///
/// 세대가 다르거나 대상이 없는 명령, 그리고 Commit·Push·해결 적용이 진행 중일 때 들어온 이 세 명령은 실행하지 않고
/// 거부 이벤트로 알립니다.
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
    /// 진행 중인 배타 작업(Commit·Push·해결 적용). 끝날 때까지 이 세 명령은 busy로 거부합니다.
    private var exclusiveOperation: UInt64?
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

    /// 아직 꺼내지 않은 이벤트 수(검사용).
    var queuedEventCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return outbox.count - outboxHead
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

    private func reject(_ command: GitPanelCommand, _ reason: GitRejectionReason) {
        post(.commandRejected(GitCommandRejection(command: command, reason: reason)))
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

        case .loadChangePage(let generation, let parent, let children):
            guard let store else { return reject(command, .unavailable) }
            guard generation == changeGeneration else { return reject(command, .staleGeneration) }
            guard let rows = store.children(of: parent, in: children) else { return reject(command, .unknownTarget) }
            post(.changePage(GitChangePage(generation: generation, parent: parent, startIndex: children.lowerBound, rows: rows)))

        case .loadChangeDetail(let generation, let node):
            guard let store else { return reject(command, .unavailable) }
            guard generation == changeGeneration else { return reject(command, .staleGeneration) }
            guard let detail = store.detail(of: node, generation: generation) else { return reject(command, .unknownTarget) }
            post(.changeDetail(detail))

        case .setStaged(let generation, let targets, let staged):
            requestStage(command, generation: generation, targets: targets, staged: staged)

        case .answerDependencyPrompt(let requestID, let include):
            guard let pending = pendingStage, pending.id == requestID else { return reject(command, .unknownTarget) }
            pendingStage = nil
            if include {
                runStage(pending.changes.union(pending.required), staged: pending.staged)
            } else {
                let id = beginOperation(pending.staged ? .stage : .unstage, message: "", cancellable: false)
                finish(id, pending.staged ? .stage : .unstage, .cancelled)
            }

        case .commit(let message):
            guard exclusiveOperation == nil else { return reject(command, .busy) }
            commit(message: message)

        case .push(let remote):
            guard exclusiveOperation == nil else { return reject(command, .busy) }
            push(remote: remote)

        case .loadConflictPage(let generation, let rows):
            guard generation == conflictGeneration else { return reject(command, .staleGeneration) }
            let upper = min(rows.upperBound, conflicts.count)
            guard rows.lowerBound < upper else { return reject(command, .unknownTarget) }
            post(.conflictPage(GitConflictPage(
                generation: generation,
                startRow: rows.lowerBound,
                rows: Array(conflicts[rows.lowerBound..<upper])
            )))

        case .chooseResolution(let generation, let rows, let choice):
            guard isReady else { return reject(command, .unavailable) }
            guard generation == conflictGeneration else { return reject(command, .staleGeneration) }
            for row in rows where row < conflicts.count && conflicts[row].allowedChoices.contains(choice) {
                conflicts[row].choice = choice
            }
            conflictGeneration += 1
            post(.status(makeStatus()))

        case .applyResolutions:
            guard exclusiveOperation == nil else { return reject(command, .busy) }
            applyResolutions()

        case .revealConflict(let generation, _):
            // 캔버스 선택 연결은 코어·캔버스 작업의 몫입니다. Stub은 세대만 확인합니다.
            if generation != conflictGeneration { reject(command, .staleGeneration) }

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
        if exclusiveOperation == id { exclusiveOperation = nil }
        post(.operationFinished(GitOperationResult(id: id, kind: kind, outcome: outcome)))
    }

    private func fail(_ kind: GitOperationKind, _ message: String) {
        let id = beginOperation(kind, message: "", cancellable: false)
        finish(id, kind, .failed(message: message))
    }

    /// 세대 불일치와 없는 대상은 거부 이벤트로, Git 사용 불가는 실패 결과로 알립니다.
    private func requestStage(_ command: GitPanelCommand, generation: UInt64, targets: [GitChangeRange], staged: Bool) {
        let kind: GitOperationKind = staged ? .stage : .unstage
        guard isReady, let store else { return fail(kind, "Git isn't available for this project.") }
        guard generation == changeGeneration else { return reject(command, .staleGeneration) }
        guard pendingStage == nil else { return reject(command, .busy) }
        guard let changes = store.changeIndexes(for: targets) else { return reject(command, .unknownTarget) }
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
        exclusiveOperation = id
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
        exclusiveOperation = id
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
        exclusiveOperation = id
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
            changes: GitChangeTreeInfo(
                generation: changeGeneration,
                structure: store?.structure ?? 0,
                documentCount: ready ? store?.documentCount ?? 0 : 0
            ),
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

/// 문서 → 객체 → 변경 3단 합성 데이터. queue 하나에서만 씁니다.
///
/// 노드 ID는 생성 시점의 순번(문서·객체 key, 객체 안 변경 key)에서 만들어 커밋으로 노드가 빠져도 남은 노드의
/// ID가 바뀌지 않습니다. 구조 번호는 노드가 빠지는 커밋에서만 올라갑니다.
final class GitStubChangeStore {
    struct Change {
        var kind: GitChangeKind
        var state: GitStageState
        var object: Int32
        var variant: UInt16
        /// 객체 안에서의 생성 순번(ID용).
        var localKey: UInt16
    }

    struct Object {
        /// 생성 순번(ID용).
        var key: Int32
        var document: Int32
        var nameIndex: Int32
        var changes: Range<Int>
        var isNew: Bool
        /// staged + 부분 staged 수.
        var stagedCount = 0
        /// unstaged + 부분 staged 수.
        var unstagedCount = 0
    }

    struct Document {
        /// 생성 순번(ID용, 이름 순번과 같음).
        var key: Int32
        var objects: Range<Int>
    }

    enum Node {
        case document(Int)
        case object(Int)
        case change(Int)
    }

    private(set) var changes: [Change] = []
    private(set) var objects: [Object] = []
    private(set) var documents: [Document] = []
    private var documentByKey: [Int32: Int] = [:]
    private var objectByKey: [Int32: Int] = [:]
    private(set) var stagedTotal = 0
    private(set) var unstagedTotal = 0
    private(set) var structure: UInt64 = 1

    var documentCount: Int { documents.count }

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
                    changes.append(Change(
                        kind: kind, state: state, object: Int32(objects.count),
                        variant: UInt16(global % 997), localKey: UInt16(truncatingIfNeeded: changeIndex)))
                }
                var object = Object(
                    key: Int32(globalObject),
                    document: Int32(documents.count),
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
            documents.append(Document(key: Int32(documentIndex), objects: firstObject..<objects.count))
        }
        rebuildKeys()
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

    private func rebuildKeys() {
        documentByKey = Dictionary(uniqueKeysWithValues: documents.enumerated().map { ($0.element.key, $0.offset) })
        objectByKey = Dictionary(uniqueKeysWithValues: objects.enumerated().map { ($0.element.key, $0.offset) })
    }

    // MARK: 노드 ID

    private static let documentTag: UInt64 = 1 << 60
    private static let objectTag: UInt64 = 2 << 60
    private static let changeTag: UInt64 = 3 << 60
    private static let tagMask: UInt64 = 0xF << 60

    private func id(_ node: Node) -> GitChangeNodeID {
        switch node {
        case .document(let index):
            return GitChangeNodeID(rawValue: Self.documentTag | UInt64(UInt32(bitPattern: documents[index].key)))
        case .object(let index):
            return GitChangeNodeID(rawValue: Self.objectTag | UInt64(UInt32(bitPattern: objects[index].key)))
        case .change(let index):
            let change = changes[index]
            let objectKey = UInt64(UInt32(bitPattern: objects[Int(change.object)].key))
            return GitChangeNodeID(rawValue: Self.changeTag | (objectKey << 16) | UInt64(change.localKey))
        }
    }

    func resolve(_ id: GitChangeNodeID) -> Node? {
        let payload = id.rawValue & ~Self.tagMask
        switch id.rawValue & Self.tagMask {
        case Self.documentTag:
            return documentByKey[Int32(truncatingIfNeeded: payload)].map(Node.document)
        case Self.objectTag:
            return objectByKey[Int32(truncatingIfNeeded: payload)].map(Node.object)
        case Self.changeTag:
            guard let object = objectByKey[Int32(truncatingIfNeeded: payload >> 16)] else { return nil }
            let local = UInt16(truncatingIfNeeded: payload)
            return objects[object].changes.first { changes[$0].localKey == local }.map(Node.change)
        default:
            return nil
        }
    }

    // MARK: 조회

    private func documentName(_ key: Int32) -> String {
        let names = Self.documentNames
        let index = Int(key)
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

    /// 부모(nil이면 최상위)의 자식 범위를 현재 순번으로 바꿉니다. 부모가 없거나 변경 행이면 nil입니다.
    private func childNodes(of parent: GitChangeNodeID?) -> (count: Int, node: (Int) -> Node)? {
        guard let parent else { return (documents.count, { Node.document($0) }) }
        switch resolve(parent) {
        case .document(let index)?:
            let range = documents[index].objects
            return (range.count, { Node.object(range.lowerBound + $0) })
        case .object(let index)?:
            let range = objects[index].changes
            return (range.count, { Node.change(range.lowerBound + $0) })
        default:
            return nil
        }
    }

    func children(of parent: GitChangeNodeID?, in range: Range<Int>) -> [GitChangeRow]? {
        guard let (count, node) = childNodes(of: parent), range.lowerBound >= 0, range.lowerBound < count else { return nil }
        return (range.lowerBound..<min(range.upperBound, count)).map { row(for: node($0)) }
    }

    func row(for node: Node) -> GitChangeRow {
        switch node {
        case .document(let index):
            let document = documents[index]
            let counts = documentCounts(document)
            return GitChangeRow(
                id: id(node),
                level: .document,
                title: documentName(document.key),
                detail: GitPanelStrings.changeCount(counts.total),
                kind: nil,
                stageState: Self.aggregate(staged: counts.staged, unstaged: counts.unstaged),
                childCount: document.objects.count,
                changeCount: counts.total
            )
        case .object(let index):
            let object = objects[index]
            let count = GitPanelStrings.changeCount(object.changes.count)
            return GitChangeRow(
                id: id(node),
                level: .object,
                title: objectName(object),
                detail: object.isNew ? "New · \(count)" : count,
                kind: nil,
                stageState: Self.aggregate(staged: object.stagedCount, unstaged: object.unstagedCount),
                childCount: object.changes.count,
                changeCount: object.changes.count
            )
        case .change(let index):
            let change = changes[index]
            let text = changeText(change)
            return GitChangeRow(
                id: id(node),
                level: .change,
                title: text.title,
                detail: text.detail,
                kind: change.kind,
                stageState: change.state,
                childCount: 0,
                changeCount: 1
            )
        }
    }

    func detail(of id: GitChangeNodeID, generation: UInt64) -> GitChangeDetail? {
        guard let node = resolve(id) else { return nil }
        let base = row(for: node)
        guard case .change(let changeIndex) = node else {
            return GitChangeDetail(generation: generation, node: id, title: base.title, kind: nil,
                                   objectPath: base.title, propertyPath: nil, headValue: nil, indexValue: nil,
                                   workingValue: nil, requiredChanges: [])
        }
        let change = changes[changeIndex]
        let object = objects[Int(change.object)]
        let path = "\(documentName(documents[Int(object.document)].key)) / \(objectName(object))"
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
            node: id,
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

    /// 범위를 변경 순번 집합으로 바꿉니다. 묶음 노드는 하위 전체입니다. 없는 부모·순번이 있으면 nil입니다.
    func changeIndexes(for targets: [GitChangeRange]) -> IndexSet? {
        var result = IndexSet()
        for target in targets {
            guard let (count, node) = childNodes(of: target.parent) else { return nil }
            for range in target.children.rangeView {
                guard range.lowerBound >= 0, range.upperBound <= count else { return nil }
                for index in range {
                    switch node(index) {
                    case .document(let document): result.insert(integersIn: documentChanges(documents[document]))
                    case .object(let object): result.insert(integersIn: objects[object].changes)
                    case .change(let change): result.insert(change)
                    }
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

    /// 상태를 바꾸고 실제로 바뀐 변경 수를 반환합니다. 구조는 그대로입니다.
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

    /// staged 변경은 목록에서 빠지고, 부분 staged는 unstaged로 남습니다(커밋 후 Index == HEAD). 구조 번호가 올라갑니다.
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
                object.document = Int32(newDocuments.count)
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
        rebuildKeys()
        structure += 1
    }
}
