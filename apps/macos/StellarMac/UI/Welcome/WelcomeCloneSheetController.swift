import AppKit

/// Clone 요청 값(셸 → 코어). URL은 민감값이므로 로그·진단에 남기지 않습니다(아키텍처 §39).
struct WelcomeCloneRequest: Equatable {
    var remoteURL: String
    var parentDirectory: String
    var folderName: String
}

/// Clone Project sheet(계획 XII.6.3): 원격 URL, 새 폴더 이름, 상위 폴더를 받습니다.
///
/// 입력은 AppKit 네이티브 텍스트 컨트롤입니다(아키텍처 §11.2). Return은 Clone, Escape는 취소입니다.
final class WelcomeCloneSheetController: NSViewController, NSTextFieldDelegate {
    var onClone: ((WelcomeCloneRequest) -> Void)?

    let titleLabel = WelcomeViews.label(WelcomeStrings.cloneTitle, style: .headline)
    let messageLabel = WelcomeViews.wrappingLabel(WelcomeStrings.cloneMessage, style: .callout, color: .secondaryLabelColor)
    let urlField = NativeTextControls.makeGeneralField(placeholder: WelcomeStrings.repositoryPlaceholder)
    let nameField = NativeTextControls.makeGeneralField(placeholder: "")
    let locationLabel = WelcomeViews.codeLabel("")
    let chooseButton = WelcomeViews.button(WelcomeStrings.choose, target: nil, action: nil)
    let cancelButton = WelcomeViews.button(WelcomeStrings.cancel, target: nil, action: nil)
    let cloneButton = WelcomeViews.button(WelcomeStrings.clone, target: nil, action: nil)

    private(set) var parentDirectory: String
    /// 사용자가 이름을 직접 고치면 URL에서 이름을 다시 만들지 않습니다.
    private var nameEditedByUser = false

    init(defaultParentDirectory: String) {
        parentDirectory = defaultParentDirectory
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("WelcomeCloneSheetController는 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView()
        view = root
        title = WelcomeStrings.cloneTitle

        urlField.delegate = self
        nameField.delegate = self
        urlField.setAccessibilityLabel(WelcomeStrings.repositoryURL)
        nameField.setAccessibilityLabel(WelcomeStrings.folderName)
        urlField.contentType = .URL
        locationLabel.setAccessibilityLabel(WelcomeStrings.location)
        chooseButton.target = self
        chooseButton.action = #selector(chooseLocation(_:))
        cancelButton.target = self
        cancelButton.action = #selector(cancel(_:))
        cancelButton.keyEquivalent = "\u{1b}"
        cloneButton.target = self
        cloneButton.action = #selector(clone(_:))
        cloneButton.keyEquivalent = "\r"

        // 양식: 오른쪽 정렬 이름표 열은 가장 긴 이름표 폭에 맞추고, 입력 열이 남는 폭을 씁니다(HIG Layout).
        let urlLabel = NSTextField(labelWithString: WelcomeStrings.repositoryURL)
        let nameLabel = NSTextField(labelWithString: WelcomeStrings.folderName)
        let locationKey = NSTextField(labelWithString: WelcomeStrings.location)
        let keys = [urlLabel, nameLabel, locationKey]
        for key in keys {
            key.alignment = .right
            key.setContentHuggingPriority(.defaultHigh, for: .horizontal)
            key.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
        for value in [urlField, nameField, locationLabel] as [NSView] {
            value.setContentHuggingPriority(.defaultLow, for: .horizontal)
        }
        chooseButton.setContentHuggingPriority(.required, for: .horizontal)
        func formRow(_ views: [NSView]) -> NSStackView {
            let row = NSStackView(views: views)
            row.orientation = .horizontal
            row.distribution = .fill
            row.alignment = .firstBaseline
            row.spacing = WelcomeMetrics.rowSpacing
            return row
        }
        let rows = [
            formRow([urlLabel, urlField]),
            formRow([nameLabel, nameField]),
            formRow([locationKey, locationLabel, chooseButton]),
        ]
        let grid = NSStackView(views: rows)
        grid.orientation = .vertical
        grid.alignment = .leading
        grid.spacing = WelcomeMetrics.rowSpacing * 2
        NSLayoutConstraint.activate(
            rows.map { $0.widthAnchor.constraint(equalTo: grid.widthAnchor) }
                + [
                    nameLabel.widthAnchor.constraint(equalTo: urlLabel.widthAnchor),
                    locationKey.widthAnchor.constraint(equalTo: urlLabel.widthAnchor),
                ]
        )

        // 버튼 줄: 취소와 기본 동작을 끝쪽에 둡니다(HIG Sheets).
        let buttons = NSStackView()
        buttons.orientation = .horizontal
        buttons.spacing = WelcomeMetrics.rowSpacing * 2
        buttons.setViews([cancelButton, cloneButton], in: .trailing)

        let stack = NSStackView(views: [titleLabel, messageLabel, grid, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = WelcomeMetrics.groupSpacing
        stack.edgeInsets = NSEdgeInsets(
            top: WelcomeMetrics.outerMargin,
            left: WelcomeMetrics.outerMargin,
            bottom: WelcomeMetrics.outerMargin,
            right: WelcomeMetrics.outerMargin
        )
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)
        let inner = -2 * WelcomeMetrics.outerMargin
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: root.topAnchor),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            root.widthAnchor.constraint(equalToConstant: WelcomeMetrics.sheetWidth),
            messageLabel.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: inner),
            grid.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: inner),
            buttons.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: inner),
        ])

        updateLocation()
        validate()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.initialFirstResponder = urlField
        view.window?.makeFirstResponder(urlField)
    }

    // MARK: 값

    /// 현재 입력으로 만든 요청. 유효하지 않으면 nil.
    var request: WelcomeCloneRequest? {
        let url = urlField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty, !url.contains(where: \.isWhitespace),
            WelcomeCloneSheetController.isValidFolderName(name),
            !parentDirectory.isEmpty
        else { return nil }
        return WelcomeCloneRequest(remoteURL: url, parentDirectory: parentDirectory, folderName: name)
    }

    /// 새 폴더 이름 규칙: 비어 있지 않고, 경로 구분자·숨김 접두·상대 경로가 아님.
    static func isValidFolderName(_ name: String) -> Bool {
        !name.isEmpty && !name.contains("/") && !name.contains(":") && !name.hasPrefix(".")
    }

    /// 저장소 URL에서 기본 폴더 이름을 만듭니다(예: `…/design-system.git` → `design-system`).
    static func suggestedFolderName(forRemoteURL url: String) -> String {
        var trimmed = url.trimmingCharacters(in: .whitespacesAndNewlines)
        while trimmed.hasSuffix("/") { trimmed.removeLast() }
        let last = trimmed.split(whereSeparator: { $0 == "/" || $0 == ":" }).last.map(String.init) ?? ""
        let name = last.hasSuffix(".git") ? String(last.dropLast(4)) : last
        return isValidFolderName(name) ? name : ""
    }

    private func updateLocation() {
        locationLabel.stringValue = (parentDirectory as NSString).abbreviatingWithTildeInPath
        locationLabel.toolTip = parentDirectory
    }

    private func validate() {
        cloneButton.isEnabled = request != nil
    }

    // MARK: NSTextFieldDelegate

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        if field === urlField, !nameEditedByUser {
            nameField.stringValue = WelcomeCloneSheetController.suggestedFolderName(forRemoteURL: urlField.stringValue)
        } else if field === nameField {
            nameEditedByUser = !nameField.stringValue.isEmpty
        }
        validate()
    }

    /// 검사용: 입력을 흉내 냅니다.
    func simulateInput(url: String) {
        urlField.stringValue = url
        controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: urlField))
    }

    // MARK: 동작

    @objc private func chooseLocation(_ sender: Any?) {
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.message = WelcomeStrings.chooseLocationMessage
        panel.prompt = WelcomeStrings.choose.replacingOccurrences(of: "…", with: "")
        panel.directoryURL = URL(fileURLWithPath: parentDirectory, isDirectory: true)
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            self.parentDirectory = url.path
            self.updateLocation()
            self.validate()
        }
    }

    @objc private func cancel(_ sender: Any?) {
        close()
    }

    @objc private func clone(_ sender: Any?) {
        guard let request else { return }
        onClone?(request)
        close()
    }

    /// sheet로 표시된 경우에만 닫습니다(검사처럼 직접 붙인 경우 제외).
    private func close() {
        if presentingViewController != nil {
            dismiss(self)
        }
    }
}
