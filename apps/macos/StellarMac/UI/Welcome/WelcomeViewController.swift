import AppKit

/// 시작 창 콘텐츠: 왼쪽 동작 열(새로 만들기·열기·Clone, Git 안내, 진행 표시)과 오른쪽 최근 프로젝트 목록.
///
/// 빈 상태와 첫 실행 원칙은 디자인 가이드 7장, 표준 컨트롤과 색은 4~5장을 따릅니다.
/// 메인 스레드에서는 snapshot 반영과 패널 표시만 합니다(아키텍처 §6.3, §9.2).
final class WelcomeViewController: NSViewController, NSOpenSavePanelDelegate {
    let dataSource: WelcomeDataSource
    let appVersion: String

    let appIconView = NSImageView()
    let titleLabel = WelcomeViews.label(WelcomeStrings.appName, style: .largeTitle)
    let versionLabel = WelcomeViews.label("", style: .subheadline, color: .secondaryLabelColor)
    let newProjectButton = WelcomeViews.actionButton(
        WelcomeStrings.newProject,
        symbol: "plus.square",
        target: nil,
        action: nil
    )
    let openProjectButton = WelcomeViews.actionButton(
        WelcomeStrings.openProject,
        symbol: "folder",
        target: nil,
        action: nil
    )
    let cloneProjectButton = WelcomeViews.actionButton(
        WelcomeStrings.cloneProject,
        symbol: "square.and.arrow.down.on.square",
        target: nil,
        action: nil
    )
    let actionsColumn = NSStackView()
    let activityView = WelcomeActivityView()
    let gitNoticeView = WelcomeGitNoticeView()
    let columnSeparator = NSBox()
    let recentsView = WelcomeRecentProjectsView()

    /// 마지막으로 그린 snapshot.
    private(set) var renderedSnapshot: WelcomeSnapshot?
    private var renderScheduled = false
    private var presentedNoticeID: UInt64?
    private var sheetObserver: NSObjectProtocol?

    init(dataSource: WelcomeDataSource, appVersion: String) {
        self.dataSource = dataSource
        self.appVersion = appVersion
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeViewController는 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView(frame: NSRect(origin: .zero, size: WelcomeMetrics.windowContentSize))
        view = root
        buildActionsColumn(in: root)
        buildRecents(in: root)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        dataSource.snapshotDidChange = { [weak self] in self?.scheduleRender() }
        renderNow()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard let window = view.window, sheetObserver == nil else { return }
        // 다른 sheet가 떠 있는 동안 도착한 안내는 그 sheet가 닫힌 뒤 띄웁니다.
        sheetObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didEndSheetNotification,
            object: window,
            queue: .main
        ) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self, let notice = self.renderedSnapshot?.notice else { return }
                self.present(notice)
            }
        }
    }

    deinit {
        if let sheetObserver { NotificationCenter.default.removeObserver(sheetObserver) }
    }

    // MARK: 구성

    private func buildActionsColumn(in root: NSView) {
        appIconView.image = NSApp.applicationIconImage
        appIconView.imageScaling = .scaleProportionallyUpOrDown
        appIconView.setAccessibilityElement(false)
        versionLabel.stringValue = WelcomeStrings.version(appVersion)
        titleLabel.alignment = .center
        versionLabel.alignment = .center
        titleLabel.setAccessibilityRole(.staticText)

        newProjectButton.target = self
        newProjectButton.action = #selector(newStellarProject(_:))
        newProjectButton.keyEquivalent = "n"
        newProjectButton.keyEquivalentModifierMask = .command
        openProjectButton.target = self
        openProjectButton.action = #selector(openStellarProject(_:))
        openProjectButton.keyEquivalent = "o"
        openProjectButton.keyEquivalentModifierMask = .command
        cloneProjectButton.target = self
        cloneProjectButton.action = #selector(cloneStellarProject(_:))

        let actionButtons = [newProjectButton, openProjectButton, cloneProjectButton]
        let buttons = NSStackView(views: actionButtons)
        buttons.orientation = .vertical
        buttons.alignment = .leading
        buttons.spacing = WelcomeMetrics.rowSpacing
        // 세 동작 버튼은 열 폭을 채워 같은 크기로 보이게 합니다.
        NSLayoutConstraint.activate(actionButtons.map { $0.widthAnchor.constraint(equalTo: buttons.widthAnchor) })

        activityView.onCancel = { [weak self] in self?.dataSource.send(.cancelActivity) }
        gitNoticeView.onCheckAgain = { [weak self] in self?.dataSource.send(.recheckGit) }
        gitNoticeView.onLocateGit = { [weak self] in self?.locateGit() }

        for view in [appIconView, titleLabel, versionLabel, buttons, activityView, gitNoticeView] as [NSView] {
            actionsColumn.addArrangedSubview(view)
        }
        actionsColumn.orientation = .vertical
        actionsColumn.alignment = .centerX
        actionsColumn.spacing = WelcomeMetrics.rowSpacing
        actionsColumn.setCustomSpacing(WelcomeMetrics.groupSpacing, after: versionLabel)
        actionsColumn.setCustomSpacing(WelcomeMetrics.groupSpacing, after: buttons)
        actionsColumn.setCustomSpacing(WelcomeMetrics.groupSpacing, after: activityView)
        actionsColumn.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(actionsColumn)

        columnSeparator.boxType = .separator
        columnSeparator.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(columnSeparator)

        let margin = WelcomeMetrics.outerMargin
        let safe = root.safeAreaLayoutGuide
        let centered = actionsColumn.centerYAnchor.constraint(equalTo: safe.centerYAnchor)
        centered.priority = .defaultLow
        NSLayoutConstraint.activate([
            actionsColumn.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: margin),
            actionsColumn.widthAnchor.constraint(equalToConstant: WelcomeMetrics.actionsColumnWidth),
            actionsColumn.topAnchor.constraint(greaterThanOrEqualTo: safe.topAnchor, constant: margin),
            actionsColumn.bottomAnchor.constraint(lessThanOrEqualTo: root.bottomAnchor, constant: -margin),
            centered,
            appIconView.widthAnchor.constraint(equalToConstant: WelcomeMetrics.appIconSize),
            appIconView.heightAnchor.constraint(equalToConstant: WelcomeMetrics.appIconSize),
            titleLabel.widthAnchor.constraint(lessThanOrEqualTo: actionsColumn.widthAnchor),
            versionLabel.widthAnchor.constraint(lessThanOrEqualTo: actionsColumn.widthAnchor),
            buttons.widthAnchor.constraint(equalTo: actionsColumn.widthAnchor),
            activityView.widthAnchor.constraint(equalTo: actionsColumn.widthAnchor),
            gitNoticeView.widthAnchor.constraint(equalTo: actionsColumn.widthAnchor),
            columnSeparator.leadingAnchor.constraint(equalTo: actionsColumn.trailingAnchor, constant: margin),
            columnSeparator.topAnchor.constraint(equalTo: root.topAnchor),
            columnSeparator.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            columnSeparator.widthAnchor.constraint(equalToConstant: 1),
        ])
    }

    private func buildRecents(in root: NSView) {
        recentsView.translatesAutoresizingMaskIntoConstraints = false
        recentsView.onOpen = { [weak self] project in self?.dataSource.send(.openRecent(id: project.id)) }
        recentsView.onRemove = { [weak self] project in self?.dataSource.send(.removeRecent(id: project.id)) }
        recentsView.onRetry = { [weak self] in self?.dataSource.send(.reloadRecentProjects) }
        recentsView.onReveal = { project in
            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: project.path)])
        }
        root.addSubview(recentsView)
        NSLayoutConstraint.activate([
            recentsView.leadingAnchor.constraint(equalTo: columnSeparator.trailingAnchor),
            recentsView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            recentsView.topAnchor.constraint(equalTo: root.safeAreaLayoutGuide.topAnchor),
            recentsView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
    }

    // MARK: snapshot 반영

    /// 연속 변경은 다음 런 루프에서 한 번만 반영합니다(디자인 가이드 9장).
    private func scheduleRender() {
        guard !renderScheduled else { return }
        renderScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.renderNow()
        }
    }

    /// 현재 snapshot을 즉시 반영합니다(검사에서도 씁니다).
    func renderNow() {
        renderScheduled = false
        let snapshot = dataSource.currentSnapshot()
        if let rendered = renderedSnapshot, snapshot.generation < rendered.generation {
            return  // 오래된 세대는 버립니다.
        }
        guard snapshot != renderedSnapshot else { return }
        renderedSnapshot = snapshot

        recentsView.apply(snapshot.recentProjects)

        let gitAllowed = snapshot.git.allowsGitOperations
        let busy = snapshot.activity != nil
        newProjectButton.isEnabled = gitAllowed && !busy
        cloneProjectButton.isEnabled = gitAllowed && !busy
        openProjectButton.isEnabled = !busy
        newProjectButton.toolTip = gitAllowed ? nil : WelcomeStrings.requiresGit
        cloneProjectButton.toolTip = gitAllowed ? nil : WelcomeStrings.requiresGit
        gitNoticeView.isHidden = !gitNoticeView.apply(snapshot.git)

        if let activity = snapshot.activity {
            activityView.apply(activity)
            activityView.isHidden = false
        } else {
            activityView.stop()
            activityView.isHidden = true
        }

        if let notice = snapshot.notice {
            present(notice)
        }
    }

    // MARK: 안내

    /// 안내 종류 코드에서 alert를 만듭니다. 문구는 셸이 정합니다.
    static func makeAlert(for notice: WelcomeNotice) -> NSAlert {
        let alert = NSAlert()
        alert.alertStyle = .warning
        switch notice.kind {
        case .notStellarProject(let path):
            alert.messageText = WelcomeStrings.notStellarProjectTitle
            alert.informativeText = WelcomeStrings.notStellarProjectMessage((path as NSString).lastPathComponent)
            alert.alertStyle = .informational
        case .projectMissing(let path):
            alert.messageText = WelcomeStrings.projectMissingTitle
            alert.informativeText = WelcomeStrings.projectMissingMessage((path as NSString).abbreviatingWithTildeInPath)
        case .folderAlreadyExists(let path):
            alert.messageText = WelcomeStrings.folderExistsTitle
            alert.informativeText = WelcomeStrings.folderExistsMessage((path as NSString).abbreviatingWithTildeInPath)
        case .gitIdentityRequired:
            alert.messageText = WelcomeStrings.gitIdentityTitle
            alert.informativeText = WelcomeStrings.gitIdentityMessage
            alert.alertStyle = .informational
        case .operationFailed(let detail):
            alert.messageText = WelcomeStrings.operationFailedTitle
            alert.informativeText = detail
        }
        alert.addButton(withTitle: WelcomeStrings.ok)
        return alert
    }

    private func present(_ notice: WelcomeNotice) {
        guard presentedNoticeID != notice.id, let window = view.window, window.attachedSheet == nil else { return }
        presentedNoticeID = notice.id
        let alert = WelcomeViewController.makeAlert(for: notice)
        alert.beginSheetModal(for: window) { [weak self] _ in
            self?.dataSource.send(.dismissNotice(id: notice.id))
        }
    }

    // MARK: 동작(메뉴 항목도 first responder action으로 같은 selector를 씁니다)

    private var defaultParentDirectory: URL {
        if let path = renderedSnapshot?.defaultParentDirectory {
            return URL(fileURLWithPath: path, isDirectory: true)
        }
        return FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
    }

    /// 새 프로젝트: 상위 폴더와 새 폴더 이름만 받아 코어에 넘깁니다(아키텍처 §25.1, 계획 XII.6.1).
    @objc func newStellarProject(_ sender: Any?) {
        guard newProjectButton.isEnabled, let window = view.window else { return }
        let panel = NSSavePanel()
        panel.title = WelcomeStrings.newProjectPanelTitle
        panel.prompt = WelcomeStrings.create
        panel.nameFieldLabel = WelcomeStrings.projectName
        panel.nameFieldStringValue = WelcomeStrings.untitledProject
        panel.canCreateDirectories = true
        panel.showsTagField = false
        panel.directoryURL = defaultParentDirectory
        panel.delegate = self
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            self?.dataSource.send(
                .createProject(
                    parentDirectory: url.deletingLastPathComponent().path,
                    folderName: url.lastPathComponent
                )
            )
        }
    }

    /// 기존 폴더를 덮어쓰는 것처럼 보이지 않도록, 이미 있는 이름은 패널에서 막습니다.
    func panel(_ sender: Any, validate url: URL) throws {
        guard sender is NSSavePanel, !(sender is NSOpenPanel) else { return }
        if FileManager.default.fileExists(atPath: url.path) {
            throw NSError(
                domain: NSCocoaErrorDomain,
                code: NSFileWriteFileExistsError,
                userInfo: [NSLocalizedDescriptionKey: WelcomeStrings.folderExists(url.lastPathComponent)]
            )
        }
    }

    /// 기존 프로젝트 열기(계획 XII.6.2). 폴더 판정(ProjectProbe)은 코어가 합니다.
    @objc func openStellarProject(_ sender: Any?) {
        guard openProjectButton.isEnabled, let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.message = WelcomeStrings.openPanelMessage
        panel.prompt = WelcomeStrings.open
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            self?.dataSource.send(.openProject(directory: url.path))
        }
    }

    /// Clone(계획 XII.6.3): URL과 대상 폴더를 sheet로 받습니다.
    @objc func cloneStellarProject(_ sender: Any?) {
        guard cloneProjectButton.isEnabled else { return }
        let sheet = WelcomeCloneSheetController(defaultParentDirectory: defaultParentDirectory.path)
        sheet.onClone = { [weak self] request in
            self?.dataSource.send(
                .cloneProject(
                    remoteURL: request.remoteURL,
                    parentDirectory: request.parentDirectory,
                    folderName: request.folderName
                )
            )
        }
        presentAsSheet(sheet)
    }

    /// Git 실행 파일 직접 지정(아키텍처 §24.1 탐색 순서 1).
    private func locateGit() {
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.treatsFilePackagesAsDirectories = true
        panel.showsHiddenFiles = true
        panel.message = WelcomeStrings.locateGitMessage
        panel.directoryURL = URL(fileURLWithPath: "/usr/bin", isDirectory: true)
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            self?.dataSource.send(.useGitExecutable(path: url.path))
        }
    }
}

/// 시작 창. 표준 `NSWindow`(제목, 닫기)이며 크기는 고정합니다(디자인 가이드 2.1).
final class WelcomeWindowController: NSWindowController {
    static let frameAutosaveName = "StellarWelcomeWindow"

    let welcomeViewController: WelcomeViewController
    private let installQueue = DispatchQueue(label: "xyz.taeyoon.stellar.welcome.command-status", qos: .utility)

    init(dataSource: WelcomeDataSource, appVersion: String = WelcomeWindowController.bundleVersion) {
        welcomeViewController = WelcomeViewController(dataSource: dataSource, appVersion: appVersion)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: WelcomeMetrics.windowContentSize),
            styleMask: [.titled, .closable, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )
        // 제목은 Window 메뉴와 VoiceOver가 창을 구분하는 데 쓰고, 화면에는 큰 앱 이름을 보여 줍니다.
        window.title = WelcomeStrings.windowTitle
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.contentViewController = welcomeViewController
        window.setContentSize(WelcomeMetrics.windowContentSize)
        window.autorecalculatesKeyViewLoop = true
        window.center()
        window.setFrameAutosaveName(WelcomeWindowController.frameAutosaveName)
        super.init(window: window)
        if case .loaded(let projects)? = welcomeViewController.renderedSnapshot?.recentProjects, !projects.isEmpty {
            window.initialFirstResponder = welcomeViewController.recentsView.tableView
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeWindowController는 코드로만 생성합니다")
    }

    static var bundleVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    /// 첫 실행이면 `stellar` 명령 설치 제안 sheet를 띄웁니다(ADR-0005 §7, 계획 XXIV.5.6).
    ///
    /// 상태 판정(파일 조회)은 메인 스레드 밖에서 하고(XXIV.5.5), 창이 보이고 다른 sheet가 없을 때만 띄웁니다.
    /// `completion`은 검사용이며 판정 결과와 띄운 sheet(없으면 nil)를 메인 스레드에서 받습니다.
    func presentCommandInstallOfferIfNeeded(
        makeInstaller: @escaping () -> StellarCommandInstaller = StellarCommandInstaller.forRunningApp,
        policy: WelcomeFirstLaunchPolicy = WelcomeFirstLaunchPolicy(),
        completion: ((StellarCommandInstallStatus, WelcomeCommandInstallSheetController?) -> Void)? = nil
    ) {
        StellarCommandStatusCheck.evaluate(on: installQueue, makeInstaller: makeInstaller) { [weak self] installer, status in
            guard let self, policy.shouldOfferCommandInstall(for: status),
                let window = self.window, window.isVisible, window.attachedSheet == nil
            else {
                completion?(status, nil)
                return
            }
            let sheet = WelcomeCommandInstallSheetController(installer: installer, status: status)
            sheet.onResponse = { _ in policy.recordCommandInstallOffered() }
            self.welcomeViewController.presentAsSheet(sheet)
            completion?(status, sheet)
        }
    }

    /// 관리 진입점(앱 메뉴 "Set Up the stellar Command…"): 현재 상태를 확인해 같은 sheet를 띄웁니다.
    /// 설치를 되돌리는(Remove Command) 경로도 여기서 제공합니다(XXIV.5.6).
    @objc func showStellarCommandSetup(_ sender: Any?) {
        guard let window, window.attachedSheet == nil else { return }
        let policy = WelcomeFirstLaunchPolicy()
        StellarCommandStatusCheck.evaluate(on: installQueue, makeInstaller: StellarCommandInstaller.forRunningApp) {
            [weak self] installer, status in
            guard let self, let window = self.window, window.attachedSheet == nil else { return }
            let sheet = WelcomeCommandInstallSheetController(installer: installer, status: status)
            // 메뉴로 먼저 답했으면 첫 실행 제안을 다시 띄우지 않습니다.
            sheet.onResponse = { _ in policy.recordCommandInstallOffered() }
            self.welcomeViewController.presentAsSheet(sheet)
        }
    }
}

extension WelcomeWindowController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(showStellarCommandSetup(_:)) {
            return window?.attachedSheet == nil
        }
        return true
    }
}

/// `stellar` 명령 설치 상태 확인(XXIV.5.5). 설치기 생성(PATH 파일 읽기)과 판정을 `queue`에서 하고,
/// 결과만 메인 스레드로 넘깁니다(아키텍처 §6.4, UI 스레드 파일 조회 0).
enum StellarCommandStatusCheck {
    static func evaluate(
        on queue: DispatchQueue,
        makeInstaller: @escaping () -> StellarCommandInstaller,
        completion: @escaping (StellarCommandInstaller, StellarCommandInstallStatus) -> Void
    ) {
        queue.async {
            let installer = makeInstaller()
            let status = installer.status()
            DispatchQueue.main.async { completion(installer, status) }
        }
    }
}
