import AppKit

/// `stellar` 명령 설치·제거 sheet(ADR-0005 §2·§7, 계획 XXIV.5.6).
///
/// 첫 실행에는 설치 제안으로, 앱 메뉴 "Set Up the stellar Command…"에서는 관리(상태 확인·제거) 화면으로 같은 sheet를 씁니다.
/// - 동의: 사용자가 Install(또는 Install as stellar-cli, Update)을 눌러야만 파일을 바꿉니다. Return은 주 동작, Escape는 Not Now입니다.
/// - 권한 상승 없음, 남의 파일 덮어쓰기 없음, 대체 이름 규칙: `StellarCommandInstaller`가 보장합니다.
/// - 실제로 설치한 이름(`stellar` 또는 `stellar-cli`)을 제목과 링크 줄에 보여 줍니다.
/// - 결과와 오류를 같은 sheet에서 바로 보여 줍니다. 상태 확인과 파일 작업은 메인 스레드 밖 큐에서 합니다.
final class WelcomeCommandInstallSheetController: NSViewController {
    /// sheet가 닫힐 때 사용자의 답.
    enum Response: Equatable {
        case installed
        case removed
        case declined
        case closed
    }

    /// 화면 단계.
    enum Phase: Equatable {
        /// 상태를 확인하는 중(메인 스레드 밖).
        case checking
        case status(StellarCommandInstallStatus)
        case working(removing: Bool)
        case installed(StellarCommandInstallation)
        case removed(StellarCommandUninstallResult)
        case installFailed(StellarCommandInstallError, plan: StellarCommandInstallPlan)
    }

    var onResponse: ((Response) -> Void)?

    let iconView = WelcomeViews.symbolView("terminal", style: .largeTitle, scale: .large)
    let titleLabel = WelcomeViews.wrappingLabel("", style: .headline)
    let messageLabel = WelcomeViews.wrappingLabel("", style: .callout)
    /// 링크 위치와 대상 두 줄. 줄마다 높이를 내용에 맞추도록 레이블 행으로 둡니다.
    let detailRows = NSStackView()
    let linkKeyLabel = WelcomeViews.label(WelcomeStrings.commandLink, style: .callout, color: .secondaryLabelColor)
    let targetKeyLabel = WelcomeViews.label(WelcomeStrings.commandTarget, style: .callout, color: .secondaryLabelColor)
    let linkValueLabel = WelcomeViews.codeLabel("")
    let targetValueLabel = WelcomeViews.codeLabel("")
    let noteLabel = WelcomeViews.wrappingLabel("", style: .callout, color: .secondaryLabelColor)
    let pathLineLabel = WelcomeViews.codeLabel("")
    let pathCopyButton = WelcomeCopyButton(accessibilityLabel: WelcomeStrings.copyCommand)
    let pathLineRow = NSStackView()
    let spinner = NSProgressIndicator()
    let destructiveButton = WelcomeViews.button(WelcomeStrings.removeCommand, target: nil, action: nil)
    let secondaryButton = WelcomeViews.button(WelcomeStrings.notNow, target: nil, action: nil)
    let primaryButton = WelcomeViews.button(WelcomeStrings.install, target: nil, action: nil)

    let installer: StellarCommandInstaller
    private let workQueue = DispatchQueue(label: "xyz.taeyoon.stellar.welcome.command-installer", qos: .userInitiated)
    private(set) var phase: Phase
    /// 진행 중 작업 식별자. sheet가 닫히거나 다른 작업이 시작되면 늦게 온 결과를 버립니다.
    private var operationToken = 0
    private var responded = false
    private var primaryAction: (() -> Void)?
    private var secondaryAction: (() -> Void)?

    /// `status`가 nil이면 확인 단계로 시작하고 곧바로 메인 스레드 밖에서 상태를 확인합니다.
    init(installer: StellarCommandInstaller, status: StellarCommandInstallStatus?) {
        self.installer = installer
        phase = status.map(Phase.status) ?? .checking
        super.init(nibName: nil, bundle: nil)
        if status == nil { refreshStatus() }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeCommandInstallSheetController는 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView()
        view = root

        for (key, value) in [(linkKeyLabel, linkValueLabel), (targetKeyLabel, targetValueLabel)] {
            key.alignment = .right
            key.setContentCompressionResistancePriority(.required, for: .horizontal)
            let row = WelcomeViews.row([key, value])
            row.alignment = .firstBaseline
            detailRows.addArrangedSubview(row)
        }
        detailRows.orientation = .vertical
        detailRows.alignment = .leading
        detailRows.spacing = WelcomeMetrics.rowSpacing
        detailRows.setHuggingPriority(.defaultHigh, for: .vertical)
        // 두 줄의 값 시작 위치를 맞춥니다(짧은 쪽 열쇠 레이블이 긴 쪽 폭을 따름).
        linkKeyLabel.widthAnchor.constraint(equalTo: targetKeyLabel.widthAnchor).isActive = true
        linkValueLabel.setAccessibilityLabel(WelcomeStrings.commandLink)
        targetValueLabel.setAccessibilityLabel(WelcomeStrings.commandTarget)

        for view in [pathLineLabel, pathCopyButton] as [NSView] { pathLineRow.addArrangedSubview(view) }
        WelcomeViews.configureRow(pathLineRow)

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false

        destructiveButton.hasDestructiveAction = true
        destructiveButton.target = self
        destructiveButton.action = #selector(removeCommand(_:))
        secondaryButton.target = self
        secondaryButton.action = #selector(secondaryPressed(_:))
        primaryButton.target = self
        primaryButton.action = #selector(primaryPressed(_:))

        // 버튼 줄: 파괴적 동작은 앞쪽, 진행 표시와 보조·주 동작은 뒤쪽(HIG Sheets, Buttons).
        let buttons = NSStackView()
        buttons.orientation = .horizontal
        buttons.alignment = .centerY
        buttons.spacing = WelcomeMetrics.rowSpacing * 2
        buttons.setViews([destructiveButton], in: .leading)
        buttons.setViews([spinner, secondaryButton, primaryButton], in: .trailing)

        let texts = NSStackView(views: [titleLabel, messageLabel, detailRows, noteLabel, pathLineRow])
        texts.orientation = .vertical
        texts.alignment = .leading
        texts.spacing = WelcomeMetrics.rowSpacing * 2
        texts.setCustomSpacing(WelcomeMetrics.rowSpacing, after: titleLabel)

        for view in [iconView, texts, buttons] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(view)
        }
        let margin = WelcomeMetrics.outerMargin
        let gap = WelcomeMetrics.groupSpacing + WelcomeMetrics.rowSpacing
        // 버튼 줄은 아이콘과 글 묶음 중 더 아래쪽 밑에 둡니다. 이 제약은 창 크기 유지(500)보다 강하고
        // 레이블·행의 세로 hugging(750)보다 약하므로, 단계가 바뀌면 내용을 늘이지 않고 sheet 높이가 맞춰집니다.
        let followsTexts = buttons.topAnchor.constraint(equalTo: texts.bottomAnchor, constant: gap)
        followsTexts.priority = NSLayoutConstraint.Priority(NSLayoutConstraint.Priority.defaultHigh.rawValue - 1)
        texts.setHuggingPriority(.defaultHigh, for: .vertical)
        NSLayoutConstraint.activate([
            root.widthAnchor.constraint(equalToConstant: WelcomeMetrics.sheetWidth),
            iconView.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: margin),
            iconView.topAnchor.constraint(equalTo: root.topAnchor, constant: margin),
            texts.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: WelcomeMetrics.groupSpacing),
            texts.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -margin),
            texts.topAnchor.constraint(equalTo: root.topAnchor, constant: margin),
            titleLabel.widthAnchor.constraint(equalTo: texts.widthAnchor),
            messageLabel.widthAnchor.constraint(equalTo: texts.widthAnchor),
            noteLabel.widthAnchor.constraint(equalTo: texts.widthAnchor),
            detailRows.widthAnchor.constraint(lessThanOrEqualTo: texts.widthAnchor),
            pathLineRow.widthAnchor.constraint(lessThanOrEqualTo: texts.widthAnchor),
            buttons.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: margin),
            buttons.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -margin),
            buttons.topAnchor.constraint(greaterThanOrEqualTo: texts.bottomAnchor, constant: gap),
            buttons.topAnchor.constraint(greaterThanOrEqualTo: iconView.bottomAnchor, constant: gap),
            followsTexts,
            buttons.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -margin),
        ])
        render()
    }

    // MARK: 화면

    /// 단계를 바꾸고 다시 그립니다.
    func show(_ newPhase: Phase) {
        phase = newPhase
        if isViewLoaded { render() }
    }

    private func render() {
        var title = ""
        var message = ""
        var notes: [String] = []
        var link: (path: String, target: String)?
        var pathHint: (directory: String, line: String)?
        var primary: (String, () -> Void)?
        var secondary: (String, () -> Void)?
        var showsRemove = false
        var working = false
        let home = installer.homeDirectory

        func hint(for directory: String, onPath: Bool) {
            guard !onPath else { return }
            pathHint = (directory, StellarCommandInstaller.pathExportLine(for: directory, homeDirectory: home))
        }
        func conflictSentence(_ conflict: StellarCommandNameConflict) -> String {
            WelcomeStrings.nameConflict(conflict, path: abbreviated(conflict.path))
        }
        func offer(_ plan: StellarCommandInstallPlan, button: String) {
            link = (plan.linkPath, plan.destination)
            if plan.createsDirectory { notes.append(WelcomeStrings.createsFolder(abbreviated(plan.directory))) }
            if !plan.hiddenProgramPaths.isEmpty {
                notes.append(
                    WelcomeStrings.hidesPrograms(
                        plan.commandName, paths: plan.hiddenProgramPaths.map(abbreviated).joined(separator: ", ")))
            }
            hint(for: plan.directory, onPath: plan.directoryOnSearchPath)
            primary = (button, { [weak self] in self?.install(plan) })
            secondary = (WelcomeStrings.notNow, { [weak self] in self?.finish(.declined) })
        }
        let close: (String, () -> Void) = (WelcomeStrings.close, { [weak self] in self?.finish(.closed) })

        switch phase {
        case .checking:
            title = WelcomeStrings.checkingTitle
            working = true
        case .status(.ready(let plan)):
            if let conflict = plan.primaryNameConflict {
                title = WelcomeStrings.fallbackInstallTitle(plan.commandName)
                message = conflictSentence(conflict) + " " + WelcomeStrings.fallbackInstallMessage(plan.commandName)
                offer(plan, button: WelcomeStrings.installAs(plan.commandName))
            } else {
                title = WelcomeStrings.commandInstallTitle
                message = WelcomeStrings.commandInstallMessage
                offer(plan, button: WelcomeStrings.install)
            }
        case .status(.needsRepair(let plan)):
            title = WelcomeStrings.repairTitle(plan.commandName)
            message = WelcomeStrings.repairMessage(plan.commandName)
            offer(plan, button: WelcomeStrings.update)
        case .status(.shadowed(let installation, let conflict, let fallback)):
            title = WelcomeStrings.shadowedTitle(installation.commandName)
            showsRemove = true
            if let fallback {
                message = conflictSentence(conflict) + " " + WelcomeStrings.shadowedFallbackMessage(fallback.commandName)
                offer(fallback, button: WelcomeStrings.installAs(fallback.commandName))
            } else {
                message = conflictSentence(conflict) + " " + WelcomeStrings.shadowedNoFallbackMessage
                link = (installation.linkPath, installer.helperPath)
                primary = close
            }
        case .status(.namesUnavailable(let conflicts)):
            title = WelcomeStrings.namesUnavailableTitle
            message = (conflicts.map(conflictSentence) + [WelcomeStrings.namesUnavailableMessage]).joined(separator: " ")
            primary = close
        case .status(.installed(let installation)), .installed(let installation):
            title = WelcomeStrings.installedTitle(installation.commandName)
            message = WelcomeStrings.installedMessage(installation.commandName)
            link = (installation.linkPath, installer.helperPath)
            hint(for: installation.directory, onPath: installation.directoryOnSearchPath)
            showsRemove = true
            let response: Response = phase == .status(.installed(installation)) ? .closed : .installed
            primary = (WelcomeStrings.done, { [weak self] in self?.finish(response) })
        case .status(.helperUnavailable):
            title = WelcomeStrings.helperUnavailableTitle
            message = WelcomeStrings.helperUnavailableMessage
            primary = close
        case .status(.unsupportedAppLocation):
            title = WelcomeStrings.locationTitle
            message = WelcomeStrings.locationMessage
            primary = close
        case .status(.noWritableLocation(let suggested)):
            title = WelcomeStrings.noLocationTitle
            message = WelcomeStrings.noLocationMessage(abbreviated(suggested))
            primary = close
        case .working(let removing):
            title = removing ? WelcomeStrings.removing : WelcomeStrings.installing
            working = true
        case .removed(let result):
            if result.failures.isEmpty {
                title = WelcomeStrings.removedTitle
                message = result.removedPaths.isEmpty
                    ? WelcomeStrings.nothingToRemove
                    : WelcomeStrings.removedMessage(result.removedPaths.map(abbreviated).joined(separator: ", "))
            } else {
                title = WelcomeStrings.removeFailedTitle
                message = result.failures.map { "\(abbreviated($0.path)): \($0.message)" }.joined(separator: "\n")
            }
            primary = (WelcomeStrings.done, { [weak self] in self?.finish(.removed) })
        case .installFailed(let error, let plan):
            title = WelcomeStrings.installFailedTitle(plan.commandName)
            switch error {
            case .helperUnavailable:
                message = WelcomeStrings.helperUnavailableMessage
                primary = close
            case .conflict(let path):
                // 상황이 바뀌었으므로 같은 계획을 다시 실행하지 않고 상태부터 다시 확인합니다(다시 동의 필요).
                message = WelcomeStrings.conflictMessage(abbreviated(path))
                primary = (WelcomeStrings.checkAgainButton, { [weak self] in self?.refreshStatus() })
                secondary = close
            case .fileSystem(let failure):
                message = "\(abbreviated(failure.path)): \(failure.message)"
                primary = (WelcomeStrings.tryAgain, { [weak self] in self?.install(plan) })
                secondary = close
            }
        }

        titleLabel.stringValue = title
        messageLabel.stringValue = message
        messageLabel.isHidden = message.isEmpty
        if let link {
            linkValueLabel.stringValue = abbreviated(link.path)
            linkValueLabel.toolTip = link.path
            targetValueLabel.stringValue = abbreviated(link.target)
            targetValueLabel.toolTip = link.target
        }
        detailRows.isHidden = link == nil
        if let pathHint {
            notes.append(WelcomeStrings.pathUnconfirmed(abbreviated(pathHint.directory)))
            pathLineLabel.stringValue = pathHint.line
            pathCopyButton.textToCopy = pathHint.line
        }
        noteLabel.stringValue = notes.joined(separator: " ")
        noteLabel.isHidden = notes.isEmpty
        pathLineRow.isHidden = pathHint == nil

        if working { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        spinner.isHidden = !working
        destructiveButton.isHidden = !showsRemove

        primaryAction = primary?.1
        secondaryAction = secondary?.1
        primaryButton.title = primary?.0 ?? ""
        primaryButton.isHidden = primary == nil
        // 기본(Return) 버튼은 주 동작 하나뿐입니다. 파괴적 동작은 기본 버튼이 되지 않습니다(가이드 5.1).
        primaryButton.keyEquivalent = primary == nil ? "" : "\r"
        secondaryButton.title = secondary?.0 ?? ""
        secondaryButton.isHidden = secondary == nil
        // 보조 버튼이 없는 화면의 Escape는 `cancelOperation(_:)`이 처리합니다.
        secondaryButton.keyEquivalent = secondary == nil ? "" : "\u{1b}"

        // sheet 창 제목과 접근성 레이블을 현재 단계 제목으로 맞춥니다.
        self.title = title
        view.setAccessibilityLabel(title)
        NSAccessibility.post(element: titleLabel, notification: .valueChanged)
    }

    private func abbreviated(_ path: String) -> String {
        let home = installer.homeDirectory
        if path == home { return "~" }
        if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }

    // MARK: 동작

    @objc private func primaryPressed(_ sender: Any?) {
        primaryAction?()
    }

    @objc private func secondaryPressed(_ sender: Any?) {
        secondaryAction?()
    }

    /// 상태를 메인 스레드 밖에서 다시 확인합니다(XXIV.5.5). 결과 화면에서 다시 동의를 받습니다.
    func refreshStatus() {
        operationToken += 1
        let token = operationToken
        let installer = installer
        show(.checking)
        workQueue.async { [weak self] in
            let status = installer.status()
            DispatchQueue.main.async {
                guard let self, token == self.operationToken else { return }
                self.show(.status(status))
            }
        }
    }

    /// Escape(cancelOperation)는 보조 버튼이 없을 때도 sheet를 닫습니다. 파일 작업 중에는 무시합니다.
    override func cancelOperation(_ sender: Any?) {
        if let secondaryAction {
            secondaryAction()
        } else if case .working = phase {
            return
        } else {
            finish(.closed)
        }
    }

    @objc private func removeCommand(_ sender: Any?) {
        operationToken += 1
        let token = operationToken
        let installer = installer
        show(.working(removing: true))
        workQueue.async { [weak self] in
            let result = installer.uninstall()
            DispatchQueue.main.async {
                guard let self, token == self.operationToken else { return }
                self.show(.removed(result))
            }
        }
    }

    /// 동의한 계획대로 설치합니다.
    func install(_ plan: StellarCommandInstallPlan) {
        operationToken += 1
        let token = operationToken
        let installer = installer
        show(.working(removing: false))
        workQueue.async { [weak self] in
            let result = installer.install(plan)
            DispatchQueue.main.async {
                guard let self, token == self.operationToken else { return }
                switch result {
                case .success(let installation): self.show(.installed(installation))
                case .failure(let error): self.show(.installFailed(error, plan: plan))
                }
            }
        }
    }

    private func finish(_ response: Response) {
        operationToken += 1
        if !responded {
            responded = true
            onResponse?(response)
        }
        if presentingViewController != nil {
            dismiss(self)
        }
    }
}
