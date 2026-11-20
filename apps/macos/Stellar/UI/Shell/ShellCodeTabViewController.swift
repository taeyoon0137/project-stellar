import AppKit

/// 저장된 View·Frame의 HTML과 진단을 읽기 전용으로 보여 줍니다.
@MainActor
final class ShellCodeTabViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    var onSelectNode: ((UUID) -> Void)?
    private let titleField = NSTextField(labelWithString: "Web · Saved State")
    private let statusField = NSTextField(labelWithString: "Select a View or Frame to see HTML.")
    private let copyButton = NSButton(title: "Copy HTML", target: nil, action: nil)
    private let saveButton = NSButton(title: "Save…", target: nil, action: nil)
    private let diagnosticsTable = NSTableView()
    private let htmlView = NSTextView()
    private var result = SavedWebExport(html: nil, error: nil, diagnostics: [])

    override func loadView() {
        let root = NSView()
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        titleField.font = .preferredFont(forTextStyle: .headline)
        statusField.textColor = .secondaryLabelColor
        statusField.lineBreakMode = .byWordWrapping
        statusField.maximumNumberOfLines = 3
        let actions = NSStackView(views: [copyButton, saveButton])
        actions.orientation = .horizontal
        actions.spacing = 8
        copyButton.target = self
        copyButton.action = #selector(copyHTML(_:))
        saveButton.target = self
        saveButton.action = #selector(saveHTML(_:))
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("diagnostic"))
        diagnosticsTable.addTableColumn(column)
        diagnosticsTable.headerView = nil
        diagnosticsTable.dataSource = self
        diagnosticsTable.delegate = self
        let diagnosticScroll = NSScrollView()
        diagnosticScroll.documentView = diagnosticsTable
        diagnosticScroll.hasVerticalScroller = true
        diagnosticScroll.translatesAutoresizingMaskIntoConstraints = false
        diagnosticScroll.heightAnchor.constraint(equalToConstant: 120).isActive = true
        htmlView.isEditable = false
        htmlView.isSelectable = true
        htmlView.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        let htmlScroll = NSScrollView()
        htmlScroll.documentView = htmlView
        htmlScroll.hasVerticalScroller = true
        htmlScroll.hasHorizontalScroller = true
        htmlScroll.translatesAutoresizingMaskIntoConstraints = false
        for item in [titleField, statusField, actions, diagnosticScroll, htmlScroll] {
            stack.addArrangedSubview(item)
        }
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -12),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -12),
            diagnosticScroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
            htmlScroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
            htmlScroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 160),
        ])
        view = root
        update(SavedWebExport(html: nil, error: nil, diagnostics: []))
    }

    func showLoading() {
        loadViewIfNeeded()
        statusField.stringValue = "Generating HTML from the saved design…"
    }

    func update(_ next: SavedWebExport) {
        result = next
        guard isViewLoaded else { return }
        htmlView.string = next.html ?? ""
        copyButton.isEnabled = next.html != nil
        saveButton.isEnabled = next.html != nil
        statusField.stringValue = next.error ?? (next.html == nil ? "HTML is unavailable for this selection." : "Saved View or Frame HTML")
        diagnosticsTable.reloadData()
        diagnosticsTable.enclosingScrollView?.isHidden = next.diagnostics.isEmpty
    }

    func numberOfRows(in tableView: NSTableView) -> Int { result.diagnostics.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let diagnostic = result.diagnostics[row]
        let label = NSTextField(labelWithString: "\(diagnostic.error ? "Error" : "Note") · \(diagnostic.code): \(diagnostic.message)")
        label.lineBreakMode = .byTruncatingTail
        label.toolTip = diagnostic.message
        return label
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = diagnosticsTable.selectedRow
        guard row >= 0, row < result.diagnostics.count else { return }
        onSelectNode?(result.diagnostics[row].node)
    }

    @objc private func copyHTML(_ sender: Any?) {
        guard let html = result.html else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(html, forType: .string)
    }

    @objc private func saveHTML(_ sender: Any?) {
        guard let html = result.html, let owner = view.window else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "view.html"
        panel.beginSheetModal(for: owner) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            DispatchQueue.global(qos: .userInitiated).async {
                let error = Result { try html.write(to: url, atomically: true, encoding: .utf8) }.failureDescription
                DispatchQueue.main.async { if let error { self?.statusField.stringValue = error } }
            }
        }
    }
}

private extension Result where Success == Void {
    var failureDescription: String? {
        if case .failure(let error) = self { return error.localizedDescription }
        return nil
    }
}
