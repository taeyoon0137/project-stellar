import AppKit

/// 메인 프로젝트 창(가이드 2장).
///
/// 표준 `NSWindow`(titled, closable, miniaturizable, resizable, full-size content)와 통합 `NSToolbar`,
/// `NSSplitViewController` 3열로 구성합니다. 창 크롬, 창 컨트롤, 툴바 높이는 시스템이 정합니다.
/// 창 프레임은 AppKit frame autosave, pane 상태는 split view autosave, 창 복원은
/// `NSWindowRestoration`이 맡습니다(가이드 2.6).
///
/// 코어 연결 시 이 controller가 창의 `ProjectWindowSession` handle을 소유합니다(가이드 2.5, XII.2.2).
/// 지금은 `ShellFrameDataSource`만 받습니다.
final class ShellWindowController: NSWindowController, NSWindowDelegate, NSMenuItemValidation,
    NSToolbarItemValidation
{
    let configuration: ShellWindowConfiguration
    let dataSource: any ShellFrameDataSource
    let splitController: ShellSplitViewController
    let toolbarController: ShellToolbarController

    /// 마지막으로 반영한 snapshot.
    private(set) var snapshot: ShellFrameSnapshot
    /// 다음 런 루프에서 반영할 snapshot(연속 변경은 한 번만 반영, 가이드 9장).
    private var pendingSnapshot: ShellFrameSnapshot?
    private var flushScheduled = false
    /// 반영 횟수(검사용).
    private(set) var appliedSnapshotCount = 0

    init(
        configuration: ShellWindowConfiguration = .standard,
        dataSource: any ShellFrameDataSource,
        slots: ShellFrameSlots
    ) {
        self.configuration = configuration
        self.dataSource = dataSource
        let initial = dataSource.currentSnapshot
        snapshot = initial

        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: ShellFrameMetrics.initialContentSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )
        // unified와 unified compact 중 선택은 열린 결정 O1이므로 기본값에서 시작합니다(가이드 2.1).
        window.toolbarStyle = .automatic
        window.titleVisibility = .visible
        window.contentMinSize = ShellFrameMetrics.minimumContentSize
        // Tab·Shift-Tab이 사이드바 → 캔버스 → 인스펙터 순서로 이동하도록 시스템이 key view loop를
        // 계산하게 합니다(가이드 8.2).
        window.autorecalculatesKeyViewLoop = true
        window.isRestorable = true
        window.identifier = NSUserInterfaceItemIdentifier(configuration.restorationIdentifier)
        window.restorationClass = ShellWindowRestoration.self

        splitController = ShellSplitViewController(slots: slots, configuration: configuration)
        toolbarController = ShellToolbarController(identifier: configuration.toolbarIdentifier, snapshot: initial)
        super.init(window: window)

        window.delegate = self
        window.contentViewController = splitController
        // contentViewController 지정은 창을 fitting size로 바꾸므로 첫 크기를 다시 정합니다.
        window.setContentSize(ShellFrameMetrics.initialContentSize)

        toolbarController.navigationTarget = self
        toolbarController.commandHandler = { [weak self] command in
            self?.dataSource.send(command)
        }
        window.toolbar = toolbarController.toolbar
        // 상단 바: 구분선 없이 블러 배경만 둡니다.
        window.titlebarSeparatorStyle = .none
        for item in splitController.splitViewItems {
            item.titlebarSeparatorStyle = .none
        }
        Self.installTitlebarBackground(in: splitController.contentContainer.view)
        Self.installToolBar(toolbarController.toolBar, in: splitController.contentContainer.view)

        splitController.contentContainer.actionHandler = { [weak self] id in
            self?.dataSource.send(.perform(id))
        }

        // 저장된 프레임이 있으면 AppKit이 복원하고, 없으면 가운데에 둡니다. 이름이 이미 다른 창에서
        // 쓰이면 autosave를 걸지 않습니다(창별 이름은 다중 창 작업 XII.2.4에서 정합니다).
        let restoredFrame = window.setFrameUsingName(configuration.autosaveName)
        _ = window.setFrameAutosaveName(configuration.autosaveName)
        if !restoredFrame {
            window.center()
        }
        // 복원한 프레임은 계단식 배치로 옮기지 않습니다.
        shouldCascadeWindows = !restoredFrame

        apply(initial)
        dataSource.snapshotHandler = { [weak self] snapshot in
            self?.receive(snapshot)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellWindowController는 코드로만 생성합니다")
    }

    /// 가운데 영역의 상단 바 뒤에 블러 배경을 깝니다(safe area 위쪽, 툴바 높이만큼).
    private static func installTitlebarBackground(in contentView: NSView) {
        let background = NSVisualEffectView()
        background.material = .titlebar
        background.blendingMode = .withinWindow
        background.state = .followsWindowActiveState
        background.translatesAutoresizingMaskIntoConstraints = false
        background.identifier = NSUserInterfaceItemIdentifier("stellar.shell.titlebarBackground")
        contentView.addSubview(background, positioned: .below, relativeTo: nil)
        NSLayoutConstraint.activate([
            background.topAnchor.constraint(equalTo: contentView.topAnchor),
            background.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            background.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            background.bottomAnchor.constraint(equalTo: contentView.safeAreaLayoutGuide.topAnchor),
        ])
    }

    /// 도구 막대를 캔버스 영역 하단 가운데에 띄웁니다.
    private static func installToolBar(_ toolBar: ShellToolBarView, in contentView: NSView) {
        toolBar.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(toolBar)
        let safeArea = contentView.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            toolBar.centerXAnchor.constraint(equalTo: safeArea.centerXAnchor),
            toolBar.bottomAnchor.constraint(
                equalTo: safeArea.bottomAnchor, constant: -ShellToolBarView.Metrics.bottomInset),
        ])
    }

    // MARK: - snapshot 반영

    /// 새 snapshot을 받습니다. 이미 반영했거나 대기 중인 세대 이하는 버리고, 연속 수신은 다음 런
    /// 루프에서 한 번만 반영합니다(가이드 9장, 아키텍처 §6.3, §9.2).
    func receive(_ incoming: ShellFrameSnapshot) {
        guard incoming.generation > snapshot.generation else { return }
        if let pendingSnapshot, pendingSnapshot.generation >= incoming.generation { return }
        pendingSnapshot = incoming
        guard !flushScheduled else { return }
        flushScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.flushPendingSnapshot()
        }
    }

    /// 대기 중인 snapshot을 지금 반영합니다.
    func flushPendingSnapshot() {
        flushScheduled = false
        guard let pending = pendingSnapshot else { return }
        pendingSnapshot = nil
        guard pending.generation > snapshot.generation else { return }
        apply(pending)
    }

    private func apply(_ newSnapshot: ShellFrameSnapshot) {
        snapshot = newSnapshot
        appliedSnapshotCount += 1
        if let window {
            if window.title != newSnapshot.title { window.title = newSnapshot.title }
            if window.subtitle != newSnapshot.subtitle { window.subtitle = newSnapshot.subtitle }
        }
        toolbarController.apply(newSnapshot)
        splitController.contentContainer.apply(status: newSnapshot.status)
    }

    // MARK: - 명령(메뉴 막대·툴바 공용, 가이드 6.1 "툴바의 모든 명령은 메뉴 막대에도")

    @objc func navigateBack(_ sender: Any?) {
        guard snapshot.navigation.canGoBack else { return }
        dataSource.send(.navigateBack)
    }

    @objc func navigateForward(_ sender: Any?) {
        guard snapshot.navigation.canGoForward else { return }
        dataSource.send(.navigateForward)
    }

    /// 툴바 프로젝트 검색 필드로 포커스를 옮깁니다(사용자 명령으로만, 가이드 8.2).
    @objc func focusProjectSearch(_ sender: Any?) {
        toolbarController.searchToolbarItem?.beginSearchInteraction()
    }

    // first responder가 창 자체일 때도 pane 명령이 split controller에 닿도록 창 delegate에서
    // 전달합니다. first responder가 pane 안에 있으면 split controller가 먼저 받습니다.

    @objc func toggleSidebar(_ sender: Any?) {
        splitController.toggleSidebar(sender)
    }

    @objc func toggleInspector(_ sender: Any?) {
        splitController.toggleInspector(sender)
    }

    @objc func toggleAuxiliaryPanel(_ sender: Any?) {
        splitController.toggleAuxiliaryPanel(sender)
    }

    // MARK: - 활성화 검사(숨기지 않고 비활성화, 가이드 8.1)

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        validate(menuItem)
    }

    func validateToolbarItem(_ item: NSToolbarItem) -> Bool {
        validate(item)
    }

    private func validate(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        switch item.action {
        case #selector(navigateBack(_:)):
            return snapshot.status == .ready && snapshot.navigation.canGoBack
        case #selector(navigateForward(_:)):
            return snapshot.status == .ready && snapshot.navigation.canGoForward
        case #selector(focusProjectSearch(_:)):
            return snapshot.status == .ready && toolbarController.searchToolbarItem != nil
        case #selector(toggleSidebar(_:)), #selector(toggleInspector(_:)), #selector(toggleAuxiliaryPanel(_:)):
            return splitController.validateUserInterfaceItem(item)
        default:
            return true
        }
    }
}

/// 메인 창 복원 진입점(`NSWindowRestoration`, 가이드 2.6).
///
/// 창을 만드는 데에는 앱 수준 의존성(runtime, 프로젝트 session)이 필요하므로, 앱이 시작할 때
/// `windowProvider`를 등록합니다. 등록 전에는 복원을 건너뛰고 AppKit에 실패를 알립니다.
final class ShellWindowRestoration: NSObject, NSWindowRestoration {
    /// 복원 식별자로 창을 만들어 돌려주는 factory. 앱 통합 시 등록합니다.
    @MainActor static var windowProvider: ((NSUserInterfaceItemIdentifier, NSCoder) -> NSWindow?)?

    static func restoreWindow(
        withIdentifier identifier: NSUserInterfaceItemIdentifier,
        state: NSCoder,
        completionHandler: @escaping (NSWindow?, (any Error)?) -> Void
    ) {
        guard let window = windowProvider?(identifier, state) else {
            completionHandler(nil, CocoaError(.featureUnsupported))
            return
        }
        completionHandler(window, nil)
    }
}
