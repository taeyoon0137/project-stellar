import AppKit

/// 메인 프로젝트 창(가이드 2장).
///
/// 표준 `NSWindow`(titled, closable, miniaturizable, resizable, full-size content)와 통합 `NSToolbar`,
/// `NSSplitViewController` 3열로 구성합니다. 창 크롬, 창 컨트롤, 툴바 높이는 시스템이 정합니다.
/// 창 프레임은 AppKit frame autosave, pane 상태는 split view autosave, 창 복원은
/// `NSWindowRestoration`이 맡습니다(가이드 2.6).
///
/// 이 controller가 창의 코어 `ProjectWindowSession` handle(`windowSession`) 하나를 소유합니다(가이드 2.5, XII.2.2).
/// 프로젝트 창에서는 data source(`ShellProjectFrameDataSource`)가 코어 프로젝트 session handle을 소유하고, 이
/// controller가 data source와 창 session handle을 소유합니다. 영역 검사처럼 코어 없이 만든 창은 handle이 없습니다.
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

    /// 이 창이 소유하는 코어 창 session handle(XII.2.2). 프로젝트가 열리기 전·실패 상태·코어 없는 창에서는 nil입니다.
    /// 창이 닫히면 소유자(프로젝트 창)가 한 번 닫습니다.
    var windowSession: ShellWindowSessionHandle?

    /// 창을 닫아도 되는지 묻는 hook(예: 활성 에이전트 세션 확인). nil이면 항상 닫습니다.
    var shouldCloseHandler: (() -> Bool)?
    /// 다른 시트가 끝나기를 기다리는 닫기 요청이 있으면 알려 줍니다.
    var didEndSheetHandler: (() -> Void)?
    /// 창이 key가 될 때 한 번 호출합니다(다른 앱이 바꾼 클립보드로 Paste 활성을 다시 계산).
    var didBecomeKeyHandler: (() -> Void)?
    /// 창이 닫힐 때 한 번 호출합니다(프로젝트 session 닫기, 시작 창 표시).
    var willCloseHandler: (() -> Void)?
    /// 창 복원 상태에 값을 더하는 hook(예: 프로젝트 경로). `NSWindowRestoration`이 다시 읽습니다(가이드 2.6).
    var restorableStateEncoder: ((NSCoder) -> Void)?

    init(
        configuration: ShellWindowConfiguration = .standard,
        dataSource: any ShellFrameDataSource,
        slots: ShellFrameSlots,
        toolbarItemProvider: ShellToolbarItemProviding? = nil
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
        // 제목·부제목은 툴바 제목 항목이 그리고, 누르면 제목 문서 메뉴가 열립니다(UX 명세 3.3). 창 제목 값은 창 메뉴와
        // 접근성을 위해 그대로 둡니다.
        window.titleVisibility = .hidden
        window.contentMinSize = ShellFrameMetrics.minimumContentSize
        // Tab·Shift-Tab이 사이드바 → 캔버스 → 인스펙터 순서로 이동하도록 시스템이 key view loop를
        // 계산하게 합니다(가이드 8.2).
        window.autorecalculatesKeyViewLoop = true
        window.isRestorable = true
        window.identifier = NSUserInterfaceItemIdentifier(configuration.restorationIdentifier)
        window.restorationClass = ShellWindowRestoration.self

        splitController = ShellSplitViewController(slots: slots, configuration: configuration)
        toolbarController = ShellToolbarController(
            identifier: configuration.toolbarIdentifier, snapshot: initial, itemProvider: toolbarItemProvider
        )
        super.init(window: window)

        window.delegate = self
        window.contentViewController = splitController
        // contentViewController 지정은 창을 fitting size로 바꾸므로 첫 크기를 다시 정합니다.
        window.setContentSize(ShellFrameMetrics.initialContentSize)

        toolbarController.commandHandler = { [weak self] command in
            self?.dataSource.send(command)
        }
        window.toolbar = toolbarController.toolbar
        // 상단 바(제목·부제목·제목 문서 메뉴)는 늘 보입니다(UX 명세 3.2). 툴바 숨김 명령은 두지 않고, 이전 버전에서
        // 숨긴 채 저장된 툴바 구성도 다시 보이게 합니다.
        toolbarController.toolbar.isVisible = true
        toolbarController.ensureTitleItem()
        // 상단 바: 구분선 없이 블러 배경만 둡니다.
        window.titlebarSeparatorStyle = .none
        for item in splitController.splitViewItems {
            item.titlebarSeparatorStyle = .none
        }
        Self.installTitlebarBackground(in: splitController.contentContainer.view)
        Self.installToolBar(toolbarController.toolBar, in: splitController.contentContainer.view)
        Self.installHint(toolbarController.hintView, above: toolbarController.toolBar,
                         in: splitController.contentContainer.view)

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

    /// 한 줄 안내를 도구 막대 바로 위 가운데에 둡니다. 캔버스 폭보다 넓어지지 않습니다.
    private static func installHint(_ hint: ShellCanvasHintView, above toolBar: ShellToolBarView, in contentView: NSView) {
        hint.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(hint)
        NSLayoutConstraint.activate([
            hint.centerXAnchor.constraint(equalTo: toolBar.centerXAnchor),
            hint.bottomAnchor.constraint(equalTo: toolBar.topAnchor, constant: -ShellCanvasHintView.spacing),
            hint.widthAnchor.constraint(lessThanOrEqualTo: contentView.safeAreaLayoutGuide.widthAnchor, constant: -32),
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

    // 동작 메서드의 guard는 활성화 검사(`validate(_:)`)와 같은 조건을 씁니다. 메뉴·툴바 밖의
    // `sendAction`이나 직접 호출도 ready가 아닌 창에서 명령을 보내지 않습니다.

    /// 제목 문서 메뉴 동작의 메뉴 막대 항목(`ShellFrameMenuItems.makeFrameActionItem`). sender의 명령 ID를 보냅니다.
    @objc func performFrameAction(_ sender: Any?) {
        guard window?.attachedSheet == nil, let action = frameAction(for: sender), action.isEnabled else { return }
        dataSource.send(.perform(action.id))
    }

    /// 메뉴 항목이 가리키는 제목 문서 동작. 번갈아 쓰는 항목(`makeFrameActionItem(title:alternatives:)`)이면 활성 창
    /// snapshot에 있는 쪽입니다.
    private func frameAction(for sender: Any?) -> ShellActionDescriptor? {
        let ids: [String]
        switch (sender as? NSMenuItem)?.representedObject {
        case let id as String: ids = [id]
        case let alternatives as [String]: ids = alternatives
        default: return nil
        }
        return snapshot.moreActions.first { ids.contains($0.id.rawValue) }
    }

    // first responder가 창 자체일 때도 pane 명령이 split controller에 닿도록 창 delegate에서
    // 전달합니다. first responder가 pane 안에 있으면 split controller가 먼저 받습니다.

    @objc func toggleSidebar(_ sender: Any?) {
        splitController.toggleSidebar(sender)
    }

    @objc func toggleInspector(_ sender: Any?) {
        splitController.toggleInspector(sender)
    }

    // MARK: - NSWindowDelegate

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        shouldCloseHandler?() ?? true
    }

    func windowDidEndSheet(_ notification: Notification) {
        didEndSheetHandler?()
    }

    func windowDidBecomeKey(_ notification: Notification) {
        didBecomeKeyHandler?()
    }

    func windowWillClose(_ notification: Notification) {
        let handler = willCloseHandler
        willCloseHandler = nil
        handler?()
    }

    func window(_ window: NSWindow, willEncodeRestorableState state: NSCoder) {
        restorableStateEncoder?(state)
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
        case #selector(performFrameAction(_:)):
            guard let action = frameAction(for: item) else { return false }
            // 창에 sheet가 떠 있으면 그 sheet에 먼저 답해야 합니다.
            guard window?.attachedSheet == nil else { return false }
            if let menuItem = item as? NSMenuItem, menuItem.representedObject is [String], menuItem.title != action.title {
                menuItem.title = action.title
            }
            return action.isEnabled
        case #selector(toggleSidebar(_:)), #selector(toggleInspector(_:)):
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
