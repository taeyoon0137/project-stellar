import AppKit

/// 사이드바 배치 상수. 디자인 가이드 0.2에 따라 이 작업에서 정한 값이며, 근거는 `docs/design/ui/sidebar.md`에 적습니다.
enum SidebarMetrics {
    /// 처음 표시할 때 페이지 구획이 차지하는 사이드바 높이 비율. 이후에는 사용자가 옮긴 분할선 위치를 따릅니다.
    static let initialPagesFraction: CGFloat = 0.3
    /// 분할선으로 줄일 수 있는 각 구획의 최소 행 수(제목 행 포함). 행 높이는 시스템 사이드바 크기 설정을 따릅니다.
    static let minimumPaneRows: CGFloat = 3
    /// 행 안 아이콘 버튼(표시·잠금 토글)의 최소 클릭 대상 한 변(point). HIG Accessibility의 macOS 최소 컨트롤 크기
    /// 20×20pt입니다(디자인 가이드 4.4). 제약은 버튼 alignment rect에 걸리며, 작은 사이드바 크기 설정의 행(24pt)에서도
    /// 눌리는 영역(버튼 frame과 셀의 겹침)이 이 크기 이상입니다(`SidebarUICheck`가 작게·중간·크게에서 확인).
    static let minimumHitTarget: CGFloat = 20
}

/// 사이드바(문서의 페이지·View 목록과 레이어 트리) view controller(디자인 가이드 2.3, 6.2).
///
/// 메인 창의 `NSSplitViewController`에 `makeSplitViewItem()`으로 붙입니다. 사이드바 재질, 접기·펼치기, 폭 유지는
/// 시스템 split view item이 맡고, 레이어 필터 필드는 그 item의 상단 accessory로 붙습니다.
///
/// provider의 snapshot 변경 통지는 다음 런 루프에서 한 번으로 모아 반영합니다(디자인 가이드 9장).
final class SidebarViewController: NSViewController, NSSplitViewDelegate {
    let provider: SidebarDataProvider
    let filterAccessory = SidebarFilterAccessoryViewController()
    let pagesController: SidebarPagesController
    let layersController: SidebarLayersController
    let stateView = SidebarStateView()

    private let splitView = NSSplitView()
    private let layersContainer = NSView()
    private(set) var appliedGeneration: UInt64 = 0
    private var applyScheduled = false
    private var didPlaceInitialDivider = false
    private var accessibilityObserver: NSObjectProtocol?

    init(provider: SidebarDataProvider) {
        self.provider = provider
        pagesController = SidebarPagesController(provider: provider)
        layersController = SidebarLayersController(provider: provider)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SidebarViewController는 코드로만 생성합니다")
    }

    deinit {
        if let accessibilityObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(accessibilityObserver)
        }
    }

    /// 사이드바 split view item. 필터 accessory를 상단에 붙여 반환합니다.
    func makeSplitViewItem() -> NSSplitViewItem {
        let item = NSSplitViewItem(sidebarWithViewController: self)
        item.addTopAlignedAccessoryViewController(filterAccessory)
        return item
    }

    /// 키보드 탐색 시작점(필터 필드)과 끝점(레이어 목록). 창의 key view loop 연결(XII.8.1)에 씁니다.
    var firstKeyView: NSView { filterAccessory.searchField }
    var lastKeyView: NSView { layersController.outline }

    /// 필터 필드로 포커스를 옮깁니다(메뉴 명령 연결용).
    func focusFilter() {
        filterAccessory.focus()
    }

    override func loadView() {
        let root = NSView()

        splitView.isVertical = false
        splitView.dividerStyle = .thin
        splitView.delegate = self
        splitView.translatesAutoresizingMaskIntoConstraints = false
        splitView.setAccessibilityLabel(SidebarStrings.pages + ", " + SidebarStrings.layers)

        let layersScroll = layersController.scrollView
        layersScroll.translatesAutoresizingMaskIntoConstraints = false
        stateView.translatesAutoresizingMaskIntoConstraints = false
        layersContainer.addSubview(layersScroll)
        layersContainer.addSubview(stateView)

        splitView.addArrangedSubview(pagesController.scrollView)
        splitView.addArrangedSubview(layersContainer)
        // 사이드바 높이가 바뀌면 레이어 구획이 늘고 줄며, 페이지 구획은 높이를 유지합니다.
        splitView.setHoldingPriority(.defaultLow + 1, forSubviewAt: 0)
        splitView.setHoldingPriority(.defaultLow - 1, forSubviewAt: 1)
        root.addSubview(splitView)

        NSLayoutConstraint.activate([
            splitView.topAnchor.constraint(equalTo: root.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            splitView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            layersScroll.topAnchor.constraint(equalTo: layersContainer.topAnchor),
            layersScroll.bottomAnchor.constraint(equalTo: layersContainer.bottomAnchor),
            layersScroll.leadingAnchor.constraint(equalTo: layersContainer.leadingAnchor),
            layersScroll.trailingAnchor.constraint(equalTo: layersContainer.trailingAnchor),
            stateView.topAnchor.constraint(equalTo: layersContainer.topAnchor),
            stateView.bottomAnchor.constraint(equalTo: layersContainer.bottomAnchor),
            stateView.leadingAnchor.constraint(equalTo: layersContainer.leadingAnchor),
            stateView.trailingAnchor.constraint(equalTo: layersContainer.trailingAnchor),
        ])
        view = root

        // 키보드 탐색: 필터 → 페이지 목록 → 레이어 목록(디자인 가이드 8.2).
        filterAccessory.loadViewIfNeeded()
        filterAccessory.searchField.nextKeyView = pagesController.outline
        pagesController.outline.nextKeyView = layersController.outline
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        provider.snapshotDidChange = { [weak self] in self?.scheduleSnapshotApply() }
        filterAccessory.onFilterChange = { [weak self] query in self?.provider.perform(.setFilter(query)) }
        stateView.onAction = { [weak self] content in self?.stateAction(content) }

        updateAccessibilityDisplayOptions()
        accessibilityObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: NSWorkspace.shared,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.updateAccessibilityDisplayOptions() }
        }
        applySnapshotNow()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        guard !didPlaceInitialDivider, splitView.bounds.height > 0 else { return }
        didPlaceInitialDivider = true
        // 페이지 목록은 상단 accessory·툴바 아래로 스크롤되므로(scroll edge effect) 비율은 safe area 아래 높이로 셉니다.
        let top = topInset
        splitView.setPosition(top + (splitView.bounds.height - top) * SidebarMetrics.initialPagesFraction, ofDividerAt: 0)
    }

    /// 사이드바 위쪽 safe area(툴바·필터 accessory) 높이.
    private var topInset: CGFloat {
        min(view.safeAreaInsets.top, splitView.bounds.height)
    }

    // MARK: Snapshot

    /// 연속된 변경 통지를 다음 런 루프에서 한 번만 반영합니다. 드래그 중(event tracking)에도 돌도록 common mode를 씁니다.
    private func scheduleSnapshotApply() {
        guard !applyScheduled else { return }
        applyScheduled = true
        perform(#selector(applyScheduledSnapshot), with: nil, afterDelay: 0, inModes: [.common])
    }

    @objc private func applyScheduledSnapshot() {
        applyScheduled = false
        applySnapshotNow()
    }

    /// provider의 최신 snapshot을 즉시 반영합니다. 이미 반영한 세대보다 오래된 snapshot은 버립니다.
    func applySnapshotNow() {
        loadViewIfNeeded()
        let snapshot = provider.snapshot
        guard snapshot.generation >= appliedGeneration else { return }
        appliedGeneration = snapshot.generation
        pagesController.apply(snapshot)
        layersController.apply(snapshot)
        filterAccessory.searchField.isEnabled = provider.validate(.setFilter("")).isAllowed
        filterAccessory.sync(query: snapshot.filter?.query ?? "")
        stateView.show(stateContent(for: snapshot))
    }

    /// 레이어 구획 상태 화면 결정(디자인 가이드 7.1).
    private func stateContent(for snapshot: SidebarSnapshot) -> SidebarStateView.Content {
        switch snapshot.loadState {
        case .loading:
            return .loading
        case .failed(let message):
            return .failed(message: message)
        case .ready:
            if snapshot.activeViewID == nil { return .noView }
            if let filter = snapshot.filter {
                return filter.matchCount == 0 ? .noResults(query: filter.query) : .hidden
            }
            return snapshot.layers.topLevelIDs.isEmpty ? .empty : .hidden
        }
    }

    private func stateAction(_ content: SidebarStateView.Content) {
        switch content {
        case .failed: provider.perform(.reload)
        case .noResults: filterAccessory.clear()
        case .hidden, .loading, .noView, .empty: break
        }
    }

    // MARK: 시스템 접근성 설정

    private func updateAccessibilityDisplayOptions() {
        // 대비 높이기: 마우스를 올려야 보이는 토글을 항상 보입니다. 투명도 줄이기·동작 줄이기는 표준 구성요소와
        // 시스템 의미 색이 처리하며, 이 영역은 직접 그리는 반투명 표면이나 애니메이션을 두지 않습니다(디자인 가이드 8.4).
        layersController.showsTogglesAlways = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast
    }

    // MARK: NSSplitViewDelegate

    private func minimumPaneExtent(for outline: NSOutlineView) -> CGFloat {
        let rowHeight = outline.numberOfRows > 0 ? outline.rect(ofRow: 0).height : outline.rowHeight
        return rowHeight * SidebarMetrics.minimumPaneRows
    }

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        max(proposedMinimumPosition, topInset + minimumPaneExtent(for: pagesController.outline))
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        min(proposedMaximumPosition, splitView.bounds.height - minimumPaneExtent(for: layersController.outline))
    }
}
