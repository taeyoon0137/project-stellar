import AppKit

/// 사이드바 배치 상수. 디자인 가이드 0.2에 따라 이 작업에서 정한 값이며, 근거는 `docs/design/ui/sidebar.md`에 적습니다.
enum SidebarMetrics {
    /// 머리 줄(Page › View 팝업)의 좌우 여백과 아래 간격(point). 레이어 행의 들여쓰기 시작점과 맞춥니다.
    static let headerInset: CGFloat = 12
    /// 행 안 아이콘 버튼(표시·잠금 토글)의 최소 클릭 대상 한 변(point). HIG Accessibility의 macOS 최소 컨트롤 크기
    /// 20×20pt입니다(디자인 가이드 4.4). 제약은 버튼 alignment rect에 걸리며, 작은 사이드바 크기 설정의 행(24pt)에서도
    /// 눌리는 영역(버튼 frame과 셀의 겹침)이 이 크기 이상입니다(`SidebarUICheck`가 작게·중간·크게에서 확인).
    static let minimumHitTarget: CGFloat = 20
}

/// 사이드바(머리 줄 Page › View 팝업과 레이어 트리) view controller(디자인 가이드 2.3, 6.2, 명세 7.1).
///
/// 메인 창의 `NSSplitViewController`에 `makeSplitViewItem()`으로 붙입니다. 사이드바 재질, 접기·펼치기, 폭 유지는
/// 시스템 split view item이 맡고, 레이어 필터 필드는 그 item의 상단 accessory로 붙습니다. Page·View는 별도 구획
/// 없이 머리 줄 한 줄(`SidebarPagePopup`)로 보입니다.
///
/// provider의 snapshot 변경 통지는 다음 런 루프에서 한 번으로 모아 반영합니다(디자인 가이드 9장).
final class SidebarViewController: NSViewController {
    let provider: SidebarDataProvider
    let filterAccessory = SidebarFilterAccessoryViewController()
    let pagePopup: SidebarPagePopup
    let layersController: SidebarLayersController
    let stateView = SidebarStateView()

    private let layersContainer = NSView()
    private(set) var appliedGeneration: UInt64 = 0
    private var applyScheduled = false
    private var accessibilityObserver: NSObjectProtocol?

    init(provider: SidebarDataProvider) {
        self.provider = provider
        pagePopup = SidebarPagePopup(provider: provider)
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

        let popup = pagePopup.button
        let layersScroll = layersController.scrollView
        layersScroll.translatesAutoresizingMaskIntoConstraints = false
        stateView.translatesAutoresizingMaskIntoConstraints = false
        layersContainer.translatesAutoresizingMaskIntoConstraints = false
        layersContainer.addSubview(layersScroll)
        layersContainer.addSubview(stateView)
        root.addSubview(popup)
        root.addSubview(layersContainer)

        // 머리 줄은 툴바·필터 accessory 아래(safe area)에 두고, 레이어 목록은 그 아래 높이를 모두 씁니다.
        let inset = SidebarMetrics.headerInset
        NSLayoutConstraint.activate([
            popup.topAnchor.constraint(equalTo: root.safeAreaLayoutGuide.topAnchor, constant: inset / 2),
            popup.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: inset),
            popup.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -inset),
            layersContainer.topAnchor.constraint(equalTo: popup.bottomAnchor, constant: inset / 2),
            layersContainer.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            layersContainer.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            layersContainer.trailingAnchor.constraint(equalTo: root.trailingAnchor),
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

        // 키보드 탐색: 필터 → 머리 줄 팝업 → 레이어 목록(디자인 가이드 8.2).
        filterAccessory.loadViewIfNeeded()
        filterAccessory.searchField.nextKeyView = popup
        popup.nextKeyView = layersController.outline
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
        pagePopup.apply(snapshot)
        layersController.apply(snapshot)
        // 필터를 지원하지 않는 provider에서는 비활성 필드 대신 accessory를 숨깁니다.
        let filterable = provider.validate(.setFilter("")).isAllowed
        filterAccessory.searchField.isEnabled = filterable
        if filterAccessory.isHidden == filterable { filterAccessory.isHidden = !filterable }
        filterAccessory.sync(query: snapshot.filter?.query ?? "")
        stateView.emptyMessage = provider.emptyLayersHint
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
}
