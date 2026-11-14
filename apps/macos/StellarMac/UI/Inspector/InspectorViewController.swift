import AppKit

/// 오른쪽 인스펙터 pane의 view controller(가이드 2.3, 6.3).
///
/// 통합 시 `NSSplitViewItem(inspectorWithViewController:)`에 넣습니다. pane 재질은 시스템이 정하므로
/// 자체 배경을 그리지 않습니다(가이드 3.2). 데이터는 `InspectorDataSource` snapshot으로만 받고,
/// 편집은 `InspectorCommand`로만 보냅니다.
///
/// 성능(아키텍처 §6.3, 가이드 9장):
/// - 연속으로 도착한 snapshot은 다음 런 루프에서 한 번만 반영하고, 오래된 세대는 버립니다.
/// - 같은 구성의 snapshot은 바뀐 행만 제자리에서 갱신합니다.
/// - 구성(섹션 id 목록)별 페이지를 창 안에 숨겨 두고 재사용합니다. macOS 26에서는 popup·segmented·button
///   크기 계산과 뷰가 창에 들어갈 때의 제약 갱신 비용이 커서, 선택 종류가 바뀔 때 컨트롤을 새로 만들거나
///   창 밖으로 뺐다 넣지 않습니다. 처음 보는 구성만 새로 만듭니다.
final class InspectorViewController: NSViewController {
    /// 창에 숨겨 둘 수 있는 최대 페이지 수. 넘으면 가장 오래 안 쓴 페이지를 버립니다.
    /// 창의 제약 엔진 하나가 숨은 페이지 제약까지 함께 최적화하므로, 많이 둘수록 새 구성을 처음 만들 때 느려집니다.
    static let maximumCachedPages = 4

    let dataSource: InspectorDataSource
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
    private var evictionScheduled = false

    /// 인스펙터 안의 키보드 포커스 순서. 통합 시 창의 key view loop에 앞뒤를 잇습니다.
    private(set) var keyViewChain: [NSView] = []

    init(dataSource: InspectorDataSource) {
        self.dataSource = dataSource
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorViewController는 코드로만 생성합니다")
    }

    // MARK: - 뷰 구성

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: InspectorMetrics.typicalWidth, height: 600))
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel("Inspector")
        root.identifier = NSUserInterfaceItemIdentifier("inspector")

        stateView.onRetry = { [weak self] in self?.send(.reload) }
        stateView.isHidden = true
        root.addSubview(stateView)
        pin(stateView, to: root)
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        dataSource.snapshotHandler = { [weak self] snapshot in
            self?.receive(snapshot)
        }
        apply(dataSource.snapshot)
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
        if let applied = appliedSnapshot, applied.selectionToken != snapshot.selectionToken {
            // 선택이 바뀌기 전에, 진행 중인 편집을 이전 세대(이전 선택)로 확정합니다.
            finishInteractions()
        }
        appliedSnapshot = snapshot
        _ = view // loadView 보장

        guard snapshot.state == .ready else {
            stateView.show(snapshot.state)
            stateView.isHidden = false
            activePage?.isHidden = true
            activePage = nil
            rebuildKeyViewChain()
            return
        }
        stateView.isHidden = true

        let key = InspectorPageView.structureKey(of: snapshot)
        pageOrder.removeAll { $0 == key }
        pageOrder.append(key)
        let page: InspectorPageView
        if let existing = pages[key] {
            page = existing
            page.apply(snapshot, collapsedSections: collapsedSections)
        } else {
            page = makePage(key: key, snapshot: snapshot)
        }
        if page !== activePage {
            activePage?.isHidden = true
            page.syncExpansion(collapsedSections)
            page.isHidden = false
            activePage = page
        }
        rebuildKeyViewChain()
    }

    /// 새 구성의 페이지를 만듭니다. 창 밖에서 내용을 채우고 배치한 뒤 한 번에 창에 넣어,
    /// 창 안 다른 페이지가 만드는 중간 배치에 끌려 들어가지 않게 합니다.
    private func makePage(key: String, snapshot: InspectorSnapshot) -> InspectorPageView {
        let page = InspectorPageView(
            structureKey: key,
            send: { [weak self] command in self?.send(command) },
            onExpansionChange: { [weak self] id, expanded in self?.sectionExpansionChanged(id, expanded: expanded) }
        )
        page.onStructureChange = { [weak self] in self?.rebuildKeyViewChain() }
        page.apply(snapshot, collapsedSections: collapsedSections)
        page.isHidden = true
        // 페이지 외곽은 autoresizing으로 root를 따릅니다. 모든 페이지를 root 가장자리 제약으로 묶으면
        // 페이지들이 root 폭 변수를 공유해, 새 페이지를 넣을 때 숨은 페이지 제약까지 다시 풀게 됩니다.
        page.translatesAutoresizingMaskIntoConstraints = true
        page.autoresizingMask = [.width, .height]
        page.frame = view.bounds
        view.addSubview(page, positioned: .below, relativeTo: stateView)
        pages[key] = page
        scheduleEviction()
        return page
    }

    /// 넘친 페이지 제거는 선택 변경 경로에서 하지 않고, 이벤트 추적 중이 아닐 때 다음 런 루프에서 합니다
    /// (제약 수천 개 제거 비용이 큼).
    private func scheduleEviction() {
        guard pageOrder.count > Self.maximumCachedPages, !evictionScheduled else { return }
        evictionScheduled = true
        RunLoop.main.perform(inModes: [.default]) { [weak self] in
            MainActor.assumeIsolated { self?.evictOverflowPages() }
        }
    }

    /// 가장 오래 안 쓴 페이지부터 상한까지 제거합니다(활성 페이지는 남깁니다).
    func evictOverflowPages() {
        evictionScheduled = false
        var candidates = pageOrder.filter { pages[$0] !== activePage }
        while pages.count > Self.maximumCachedPages, !candidates.isEmpty {
            let oldest = candidates.removeFirst()
            pageOrder.removeAll { $0 == oldest }
            pages.removeValue(forKey: oldest)?.removeFromSuperview()
        }
    }

    /// 인스펙터 안에서 진행 중인 텍스트 편집을 확정하고, 색 패널과 연결된 색 well을 끊습니다.
    /// 확정 명령은 아직 바뀌지 않은 `appliedSnapshot` 세대로 나갑니다.
    private func finishInteractions() {
        if let window = view.window, let responder = window.firstResponder as? NSView {
            // 한 줄 필드 편집 중이면 first responder는 창의 field editor이고, 그 delegate가 필드입니다.
            let owner = ((responder as? NSText)?.delegate as? NSView) ?? responder
            if owner.isDescendant(of: view) {
                window.makeFirstResponder(nil)
            }
        }
        // 색 패널이 만들어진 적이 없으면 활성 well도 없으므로 뷰를 훑지 않습니다.
        if NSColorPanel.sharedColorPanelExists {
            Self.deactivateColorWells(in: view)
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
        } else if !stateView.retryButton.isHidden {
            chain.append(stateView.retryButton)
        }
        for (current, next) in zip(chain, chain.dropFirst()) {
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

    /// 창에 숨겨 둔 페이지 수.
    var cachedPageCount: Int { pages.count }

    /// 현재 화면(활성 페이지 또는 상태 화면)에서 식별자로 컨트롤을 찾습니다.
    func control(identifier: String) -> NSView? {
        if let activePage, let found = Self.find(identifier, in: activePage) { return found }
        return Self.find(identifier, in: stateView)
    }

    private static func find(_ identifier: String, in view: NSView) -> NSView? {
        if view.identifier?.rawValue == identifier { return view }
        for subview in view.subviews {
            if let found = find(identifier, in: subview) { return found }
        }
        return nil
    }

    private func pin(_ child: NSView, to parent: NSView) {
        child.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            child.leadingAnchor.constraint(equalTo: parent.leadingAnchor),
            child.trailingAnchor.constraint(equalTo: parent.trailingAnchor),
            child.topAnchor.constraint(equalTo: parent.topAnchor),
            child.bottomAnchor.constraint(equalTo: parent.bottomAnchor),
        ])
    }
}
