import AppKit

/// 오른쪽 인스펙터 pane의 view controller(가이드 2.3, 6.3).
///
/// 통합 시 `NSSplitViewItem(inspectorWithViewController:)`에 넣습니다. pane 재질은 시스템이 정하므로
/// 자체 배경을 그리지 않습니다(가이드 3.2). 데이터는 `InspectorDataSource` snapshot으로만 받고,
/// 편집은 `InspectorCommand`로만 보냅니다.
///
/// 성능(아키텍처 §6.3·§6.4, 가이드 9장, docs/design/ui/inspector.md 8.3절):
/// - 연속으로 도착한 snapshot은 다음 런 루프에서 한 번만 반영하고, 오래된 세대는 버립니다.
/// - 같은 구성의 snapshot은 바뀐 행만 제자리에서 갱신합니다.
/// - 구성별 페이지를 창 안에 alpha 0으로 미리 만들어 두고(예열), 선택이 바뀌면 alpha만 바꿔 전환합니다.
///   예열은 `InspectorBuildQueue`가 컨트롤 하나 정도의 단계로 나눠 유휴 런 루프에서 합니다.
/// - 아직 없는 구성(캐시 미스)은 저비용 대체(로딩 상태)를 먼저 보이고, 페이지를 최우선으로 나눠 만든 뒤
///   보입니다(아키텍처 §6.4 "저비용 대체 결과를 우선 표시하고 비동기 정교화"). 선택 경로에서 컨트롤을 만들지 않습니다.
final class InspectorViewController: NSViewController {
    /// 창에 둘 수 있는 최대 페이지 수. 넘으면 가장 오래 안 쓴 대기 페이지를 유휴 시간에 나눠 제거합니다.
    /// 대기 페이지는 제약을 쓰지 않아 배치 엔진 비용은 없지만, 페이지당 메모리 약 6.5MB와 창의 그리기 주기마다
    /// 약 0.02~0.05ms(뷰 약 480개 순회)를 씁니다(명세 8.3절 측정). 그래서 상한을 작게 둡니다.
    static let maximumCachedPages = 6
    /// 처음 표시 뒤 예열할 카탈로그 구성 수(자주 쓰는 순서: Frame, Text, Rectangle, Image).
    static let prewarmedCatalogPageCount = 4

    let dataSource: InspectorDataSource
    /// 페이지 예열·제거·캐시 미스 구성을 나눠 실행하는 대기열.
    let buildQueue = InspectorBuildQueue()
    /// 처음 표시 뒤 카탈로그 구성 페이지를 유휴 시간에 예열할지.
    let prewarmsCatalogPages: Bool
    /// 마지막으로 화면에 반영한 snapshot.
    private(set) var appliedSnapshot: InspectorSnapshot?
    private var pendingSnapshot: InspectorSnapshot?
    private var applyScheduled = false
    /// 접은 섹션(창별 UI 상태, 문서에 저장하지 않음).
    private var collapsedSections: Set<InspectorSectionID> = []

    private let stateView = InspectorStateView()
    /// 구성 키 → 페이지. `pageOrder`는 최근 사용 순입니다(마지막이 최신).
    private var pages: [String: InspectorPageView] = [:]
    private var pageOrder: [String] = []
    private(set) var activePage: InspectorPageView?
    private var laidOutSize: NSSize = .zero
    /// 캐시 미스로 페이지를 기다리는 동안 맡아 둔 포커스(속성 식별자). 그 페이지가 보이면 같은 속성 컨트롤로 돌려줍니다.
    private var pendingFocusIdentifier: String?

    /// 인스펙터 안의 키보드 포커스 순서. 통합 시 창의 key view loop에 앞뒤를 잇습니다.
    private(set) var keyViewChain: [NSView] = []

    init(dataSource: InspectorDataSource, prewarmsCatalogPages: Bool = true) {
        self.dataSource = dataSource
        self.prewarmsCatalogPages = prewarmsCatalogPages
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorViewController는 코드로만 생성합니다")
    }

    // MARK: - 뷰 구성

    override func loadView() {
        let root = InspectorRootView(frame: NSRect(x: 0, y: 0, width: InspectorMetrics.typicalWidth, height: 600))
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel("Inspector")
        root.identifier = NSUserInterfaceItemIdentifier("inspector")
        root.onLayout = { [weak self] in self?.layoutRoot() }

        stateView.onRetry = { [weak self] in self?.send(.reload) }
        stateView.isHidden = true
        stateView.frame = root.bounds
        root.addSubview(stateView)
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        dataSource.snapshotHandler = { [weak self] snapshot in
            self?.receive(snapshot)
        }
        apply(dataSource.snapshot)
        if prewarmsCatalogPages {
            prewarm(Array(InspectorPropertyCatalog.prewarmSnapshots().prefix(Self.prewarmedCatalogPageCount)))
        }
    }

    // MARK: - 예열

    /// 구성 페이지를 유휴 시간에 나눠 미리 만듭니다. 이미 있는 구성은 건너뜁니다. 코어 adapter는 문서의
    /// Component 정의처럼 카탈로그에 없는 구성도 이 경로로 예열할 수 있습니다(값은 쓰지 않고 구성만 봄).
    func prewarm(_ snapshots: [InspectorSnapshot]) {
        _ = view
        for snapshot in snapshots where snapshot.state == .ready {
            let key = InspectorPageView.structureKey(of: snapshot)
            guard pages[key] == nil else { continue }
            makePage(key: key, snapshot: snapshot, priority: .background)
        }
    }

    /// 구성 페이지가 완성돼 있는지(검사·진단용).
    func hasCompletePage(for snapshot: InspectorSnapshot) -> Bool {
        pages[InspectorPageView.structureKey(of: snapshot)]?.isComplete ?? false
    }

    // MARK: - snapshot 수신

    /// 새 snapshot을 받습니다. 오래된 세대는 버리고, 다음 런 루프에서 한 번만 반영합니다.
    func receive(_ snapshot: InspectorSnapshot) {
        let latest = pendingSnapshot?.generation ?? appliedSnapshot?.generation ?? 0
        guard snapshot.generation >= latest else { return }
        pendingSnapshot = snapshot
        guard !applyScheduled else { return }
        applyScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.flushPendingSnapshot()
        }
    }

    /// 대기 중인 snapshot을 즉시 반영합니다(런 루프를 돌리지 않는 검사용).
    func flushPendingSnapshot() {
        applyScheduled = false
        guard let snapshot = pendingSnapshot else { return }
        pendingSnapshot = nil
        apply(snapshot)
    }

    private func apply(_ snapshot: InspectorSnapshot) {
        if let applied = appliedSnapshot, snapshot.generation < applied.generation { return }
        let selectionChanged = appliedSnapshot.map { $0.selectionToken != snapshot.selectionToken } ?? false
        if selectionChanged {
            // 선택이 바뀌기 전에, 진행 중인 편집과 연속 조작을 이전 세대(이전 선택)로 확정합니다.
            // 포커스는 옮기지 않습니다(가이드 8.2).
            finishInteractions()
            pendingFocusIdentifier = nil
        } else if let activePage,
                  snapshot.state != .ready || InspectorPageView.structureKey(of: snapshot) != activePage.structureKey {
            // 같은 선택이라도 페이지가 바뀌면(예: Frame으로 변환) 사라질 컨트롤의 연속 조작을 먼저 끝냅니다.
            activePage.endGestures()
        }
        appliedSnapshot = snapshot
        _ = view // loadView 보장

        guard snapshot.state == .ready else {
            showState(snapshot.state)
            return
        }

        let key = InspectorPageView.structureKey(of: snapshot)
        pageOrder.removeAll { $0 == key }
        pageOrder.append(key)
        // 이전 선택 때문에 급하게 만들던 다른 페이지는 배경으로 낮춥니다(지금 선택의 페이지가 먼저).
        buildQueue.demote(keysWithPrefix: "page:")
        if let page = pages[key], page.isComplete {
            page.apply(snapshot, collapsedSections: collapsedSections)
            activate(page)
            // 선택이 바뀐 경우에만: 편집 중인 필드에 새 선택의 값을 보입니다(같은 선택의 snapshot은 입력 중인 글자를 지우지 않음).
            if selectionChanged { refreshEditingField() }
            return
        }
        // 캐시 미스: 로딩 상태를 먼저 보이고, 페이지를 최우선으로 나눠 만든 뒤 보입니다.
        showState(.loading)
        if pages[key] != nil {
            buildQueue.promote(Self.buildKey(key))
        } else {
            makePage(key: key, snapshot: snapshot, priority: .active)
        }
    }

    /// 새 구성의 페이지를 대기 상태로 창에 넣고 나눠 만들기를 맡깁니다.
    @discardableResult
    private func makePage(key: String, snapshot: InspectorSnapshot, priority: InspectorBuildQueue.Priority)
        -> InspectorPageView
    {
        let page = InspectorPageView(
            structureKey: key,
            send: { [weak self] command in self?.send(command) },
            onExpansionChange: { [weak self] id, expanded in self?.sectionExpansionChanged(id, expanded: expanded) }
        )
        page.onStructureChange = { [weak self] in self?.rebuildKeyViewChain() }
        page.scheduleBuild = { [weak self, weak page] buildKey, steps, completion in
            guard let self else { return }
            self.buildQueue.enqueue(
                key: buildKey + "@" + key, priority: .active, steps: steps,
                afterSlice: { page?.layoutSubtreeIfNeeded() },
                completion: { [weak self] in
                    completion()
                    self?.rebuildKeyViewChain()
                }
            )
        }
        page.frame = view.bounds
        pages[key] = page
        if !pageOrder.contains(key) {
            // 예열 페이지는 아직 쓰지 않았으므로 가장 오래된 쪽에 둡니다.
            pageOrder.insert(key, at: 0)
        }
        // 첫 단계에서 페이지를 창에 넣습니다(선택 경로에서 뷰를 창에 넣는 비용을 뺌).
        let attach = InspectorBuildStep.light { [weak self, weak page] in
            guard let self, let page, page.superview == nil else { return }
            page.frame = self.view.bounds
            self.view.addSubview(page, positioned: .below, relativeTo: self.stateView)
        }
        buildQueue.enqueue(
            key: Self.buildKey(key), priority: priority,
            // 예열(배경)은 견본 값 변경까지 미리 치르고, 지금 필요한 구성(미스)은 바로 보이게 건너뜁니다.
            steps: [attach] + page.buildSteps(from: snapshot, collapsedSections: collapsedSections,
                                              warmsUpValues: priority == .background),
            afterSlice: { [weak page] in page?.layoutSubtreeIfNeeded() },
            completion: { [weak self, weak page] in
                guard let self, let page else { return }
                self.pageDidFinishBuilding(page)
            }
        )
        evictOverflowPages()
        return page
    }

    private static func buildKey(_ structureKey: String) -> String { "page:" + structureKey }

    /// 다 만든 페이지가 지금 필요한 구성이면 최신 snapshot을 반영해 보입니다.
    private func pageDidFinishBuilding(_ page: InspectorPageView) {
        // 구성 중이라 제거하지 못한 넘친 페이지가 있으면 지금 제거를 맡깁니다.
        evictOverflowPages()
        guard let snapshot = appliedSnapshot, snapshot.state == .ready,
              InspectorPageView.structureKey(of: snapshot) == page.structureKey else { return }
        page.apply(snapshot, collapsedSections: collapsedSections)
        let focus = pendingFocusIdentifier
        pendingFocusIdentifier = nil
        activate(page, restoringFocusTo: focus)
    }

    /// 페이지를 화면에 보입니다. 이전 페이지는 대기(alpha 0)로 돌립니다.
    ///
    /// 포커스(가이드 8.2 "사용자 조작 없이 포커스를 옮기지 않음"): 이전 페이지 안 컨트롤에 포커스가 있었으면 새 페이지의
    /// 같은 속성 컨트롤(같은 식별자, 예: 레이어 이름)로 이어 줍니다. 같은 속성이 없으면 포커스한 컨트롤이 화면에서
    /// 사라지므로, 보이지 않는 컨트롤로 키 입력이 가지 않게 포커스를 내려놓습니다.
    private func activate(_ page: InspectorPageView, restoringFocusTo pendingIdentifier: String? = nil) {
        if activePage !== page {
            let previous = activePage
            let focused = previous.flatMap(focusedIdentifier(in:)) ?? pendingIdentifier
            previous?.isActive = false
            page.syncExpansion(collapsedSections)
            if page.frame != view.bounds { page.frame = view.bounds }
            page.isActive = true
            activePage = page
            if let focused { moveFocus(toControlWithIdentifier: focused, in: page, leaving: previous) }
        }
        stateView.hide()
        rebuildKeyViewChain()
    }

    private func showState(_ state: InspectorContentState) {
        stateView.show(state)
        if let activePage {
            // 페이지를 기다리는 로딩이면 포커스한 속성을 맡아 두었다가 페이지가 보일 때 돌려줍니다.
            if let focused = focusedIdentifier(in: activePage) {
                pendingFocusIdentifier = state == .loading ? focused : nil
                view.window?.makeFirstResponder(nil)
            }
            activePage.isActive = false
            self.activePage = nil
        }
        rebuildKeyViewChain()
    }

    /// 창의 first responder가 `page` 안 컨트롤이면 그 식별자(속성 경로). 편집 중이면 field editor의 delegate가 필드입니다.
    private func focusedIdentifier(in page: InspectorPageView) -> String? {
        guard let owner = focusedControl(), owner.isDescendant(of: page) else { return nil }
        return owner.identifier?.rawValue ?? ""
    }

    private func focusedControl() -> NSView? {
        guard let responder = view.window?.firstResponder as? NSView else { return nil }
        return ((responder as? NSText)?.delegate as? NSView) ?? responder
    }

    private func moveFocus(toControlWithIdentifier identifier: String, in page: InspectorPageView,
                           leaving previous: InspectorPageView?) {
        guard let window = view.window else { return }
        if !identifier.isEmpty, let target = Self.find(identifier, in: page), target.acceptsFirstResponder,
           window.makeFirstResponder(target) {
            return
        }
        // 같은 속성 컨트롤이 없음: 대기 페이지 안 컨트롤에 포커스가 남지 않게 합니다.
        if let previous, let owner = focusedControl(), owner.isDescendant(of: previous) {
            window.makeFirstResponder(nil)
        }
    }

    /// 같은 페이지에서 선택이 바뀐 뒤, 편집 중인 필드에 새 선택의 값을 보입니다(포커스와 편집은 유지).
    private func refreshEditingField() {
        guard let responder = view.window?.firstResponder as? NSText,
              let field = responder.delegate as? NSTextField, let activePage, field.isDescendant(of: activePage),
              let controller = field.delegate as? InspectorFieldController else { return }
        controller.showValueInEditor()
    }

    /// 넘친 페이지 제거. 활성·구성 중 페이지는 남기고, 가장 오래 안 쓴 페이지부터 유휴 시간에 나눠 제거합니다.
    func evictOverflowPages() {
        while pages.count > Self.maximumCachedPages {
            guard let victim = pageOrder.first(where: { key in
                pages[key] !== activePage && !buildQueue.contains(Self.buildKey(key))
            }), let page = pages.removeValue(forKey: victim) else { break }
            pageOrder.removeAll { $0 == victim }
            buildQueue.enqueue(key: "evict:\(ObjectIdentifier(page).hashValue)", priority: .background,
                               steps: page.teardownSteps())
        }
    }

    /// 루트 배치: 상태 화면과 활성 페이지를 루트에 맞추고, 폭이 바뀌면 대기 페이지를 유휴 시간에 다시 배치합니다.
    private func layoutRoot() {
        let bounds = view.bounds
        if stateView.frame != bounds { stateView.frame = bounds }
        if let activePage, activePage.frame != bounds { activePage.frame = bounds }
        guard bounds.size != laidOutSize else { return }
        let widthChanged = bounds.width != laidOutSize.width
        laidOutSize = bounds.size
        guard widthChanged else { return }
        for page in pages.values where page !== activePage && page.isComplete {
            buildQueue.enqueue(key: "relayout:" + page.structureKey, priority: .background,
                               steps: page.relayoutSteps(to: bounds.size))
        }
    }

    /// 인스펙터 안에서 진행 중인 텍스트 편집과 연속 조작을 확정하고, 색 패널과 연결된 색 well을 끊습니다.
    /// 확정 명령은 아직 바뀌지 않은 `appliedSnapshot` 세대로 나갑니다. 편집은 끝내지 않고(포커스 유지) 입력한
    /// 값만 이전 선택에 확정합니다. 이어서 새 선택의 값이 같은 필드에 보입니다.
    private func finishInteractions() {
        if let window = view.window, let responder = window.firstResponder as? NSText,
           let field = responder.delegate as? NSTextField, field.isDescendant(of: view),
           let controller = field.delegate as? InspectorFieldController {
            controller.commitPendingEdit()
        }
        activePage?.endGestures()
        // 색 패널이 만들어진 적이 없으면 활성 well도 없으므로 뷰를 훑지 않습니다.
        if NSColorPanel.sharedColorPanelExists, let activePage {
            Self.deactivateColorWells(in: activePage)
        }
    }

    private static func deactivateColorWells(in view: NSView) {
        if let well = view as? NSColorWell, well.isActive {
            well.deactivate()
        }
        view.subviews.forEach { deactivateColorWells(in: $0) }
    }

    private func sectionExpansionChanged(_ id: InspectorSectionID, expanded: Bool) {
        if expanded {
            collapsedSections.remove(id)
        } else {
            collapsedSections.insert(id)
        }
        rebuildKeyViewChain()
    }

    // MARK: - 명령

    private func send(_ command: InspectorCommand) {
        dataSource.submit(command, basedOn: appliedSnapshot?.generation ?? 0)
    }

    // MARK: - 키보드 순서

    /// 보이는 컨트롤을 화면 순서대로 잇습니다(가이드 8.2).
    private func rebuildKeyViewChain() {
        var chain: [NSView] = []
        if let activePage {
            chain = activePage.keyViews
        } else if !stateView.isHidden, !stateView.retryButton.isHidden {
            chain.append(stateView.retryButton)
        }
        for (current, next) in zip(chain, chain.dropFirst()) where current.nextKeyView !== next {
            current.nextKeyView = next
        }
        chain.last?.nextKeyView = nil
        keyViewChain = chain
    }

    // MARK: - 검사·통합 도우미

    /// 현재 페이지의 섹션 뷰(화면 순서, modifier 섹션 포함).
    var sectionViews: [InspectorSectionView] { activePage?.sectionViews ?? [] }

    /// 현재 페이지의 modifier 목록 controller(있으면).
    var modifierStackController: InspectorModifierStackController? { activePage?.modifierController }

    /// 창에 둔 페이지 수(제거 대기 중인 페이지 제외).
    var cachedPageCount: Int { pages.count }

    /// 창에 둔 모든 페이지(검사용).
    var allPages: [InspectorPageView] { Array(pages.values) }

    /// 상태 화면(선택 없음·로딩·오류)이 보이는지.
    var isShowingState: Bool { !stateView.isHidden }

    /// 로딩 spinner가 돌고 있는지(페이지를 보인 뒤에는 멈춰야 함).
    var isLoadingSpinnerRunning: Bool { stateView.isSpinnerRunning }

    /// 로딩 문구·spinner를 보이기 전 기다리는 중인지.
    var isLoadingRevealPending: Bool { stateView.isLoadingRevealPending }

    /// 현재 화면(활성 페이지 또는 상태 화면)에서 식별자로 컨트롤을 찾습니다.
    func control(identifier: String) -> NSView? {
        if let activePage, let found = Self.find(identifier, in: activePage) { return found }
        return stateView.isHidden ? nil : Self.find(identifier, in: stateView)
    }

    private static func find(_ identifier: String, in view: NSView) -> NSView? {
        if view.identifier?.rawValue == identifier { return view }
        for subview in view.subviews {
            if let found = find(identifier, in: subview) { return found }
        }
        return nil
    }
}

/// 인스펙터 루트. 상태 화면과 페이지를 frame으로 배치합니다(제약 없음).
final class InspectorRootView: NSView {
    var onLayout: (() -> Void)?

    override func setFrameSize(_ newSize: NSSize) {
        let changed = newSize != frame.size
        super.setFrameSize(newSize)
        if changed { needsLayout = true }
    }

    override func layout() {
        super.layout()
        onLayout?()
    }
}
