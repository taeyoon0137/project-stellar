import AppKit

/// 한 프로젝트 창에서 연 독립 원본의 목록과 선택한 원본의 루트 크기 편집.
@MainActor
final class ShellComponentLibrary: NSObject, NSWindowDelegate, NSTableViewDataSource, NSTableViewDelegate {
    private let runtime: UInt64
    private let session: UInt64
    private let worker = DispatchQueue(label: "xyz.taeyoon.stellar.component-library", qos: .userInitiated)
    let table = NSTableView()
    let rootWidth = NSTextField()
    let rootHeight = NSTextField()
    let placeInstance = NSButton(title: String(localized: "Place Instance…"), target: nil, action: nil)
    var beginPlacement: ((LibraryComponent) -> Void)?
    let applySize = NSButton(title: String(localized: "Apply Size"), target: nil, action: nil)
    private var editingEntry: LibraryComponent?
    private var saveMessage: String?
    private let refreshButton = NSButton(title: String(localized: "Refresh"), target: nil, action: nil)
    private(set) var rows: [LibraryComponent] = []
    var didReadSources: (() -> Void)?
    private(set) var isLoading = false
    private var needsRefresh = false
    private var invalidated = false
    let message = NSTextField(wrappingLabelWithString: "")
    let details = NSTextField(wrappingLabelWithString: "")
    let panel: NSPanel

    init(runtime: UInt64, session: UInt64, projectName: String) {
        self.runtime = runtime
        self.session = session
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 660, height: 390),
                        styleMask: [.titled, .closable, .resizable, .utilityWindow], backing: .buffered, defer: false)
        super.init()
        panel.title = String(localized: "Component Library") + " — " + projectName
        panel.minSize = NSSize(width: 420, height: 260)
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        for (id, title, width) in [("name", String(localized: "Name"), 200),
                                   ("file", String(localized: "Source File"), 250),
                                   ("frames", String(localized: "Frames"), 60),
                                   ("size", String(localized: "Root Size"), 90)] {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(id))
            column.title = title
            column.width = CGFloat(width)
            table.addTableColumn(column)
        }
        table.dataSource = self
        table.delegate = self
        table.allowsMultipleSelection = false
        table.usesAlternatingRowBackgroundColors = true
        table.setAccessibilityLabel(String(localized: "Components"))
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.documentView = table
        let bar = NSStackView(views: [message, NSView(), refreshButton])
        bar.orientation = .horizontal
        refreshButton.target = self
        refreshButton.action = #selector(refresh)
        details.isSelectable = true
        details.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        details.textColor = .secondaryLabelColor
        let sizing = NSStackView()
        sizing.orientation = .horizontal
        for (label, field) in [(String(localized: "Width"), rootWidth), (String(localized: "Height"), rootHeight)] {
            sizing.addArrangedSubview(NSTextField(labelWithString: label))
            field.setAccessibilityLabel(label)
            field.alignment = .right
            field.widthAnchor.constraint(equalToConstant: 80).isActive = true
            field.isEnabled = false
            field.target = self
            field.action = #selector(commitRootSize)
            sizing.addArrangedSubview(field)
        }
        sizing.addArrangedSubview(NSTextField(labelWithString: "px"))
        applySize.target = self
        applySize.action = #selector(commitRootSize)
        applySize.isEnabled = false
        sizing.addArrangedSubview(applySize)
        placeInstance.target = self
        placeInstance.action = #selector(placeSelected)
        placeInstance.isEnabled = false
        sizing.addArrangedSubview(placeInstance)
        let stack = NSStackView(views: [bar, scroll, details, sizing])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
        panel.contentView = stack
        for view in [bar, scroll, details] {
            view.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24).isActive = true
        }
        scroll.setContentHuggingPriority(.defaultLow, for: .vertical)
    }

    func show(relativeTo parent: NSWindow) {
        if !panel.isVisible {
            panel.setFrameTopLeftPoint(NSPoint(x: parent.frame.midX - panel.frame.width / 2,
                                               y: parent.frame.midY + panel.frame.height / 2))
        }
        panel.makeKeyAndOrderFront(nil)
        if !isLoading { refresh() }
    }

    func windowDidBecomeKey(_ notification: Notification) { if !isLoading { refresh() } }

    @objc func refresh() {
        guard !invalidated else { return }
        guard !isLoading else { needsRefresh = true; return }
        isLoading = true
        refreshButton.isEnabled = false
        setEditorEnabled(false)
        message.stringValue = String(localized: "Loading components…")
        let runtime = self.runtime, session = self.session
        worker.async { [weak self] in
            let result = CoreRuntime.componentLibrary(handle: runtime, session: session)
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.invalidated else { return }
                self.isLoading = false
                self.refreshButton.isEnabled = true
                let selected = self.rows.indices.contains(self.table.selectedRow) ? self.rows[self.table.selectedRow].id : nil
                switch result {
                case .loaded(let components, let chunks):
                    self.rows = components
                    if chunks > 0 {
                        self.message.stringValue = String(localized: "This project stores its component library in a binary generation.")
                    } else if components.isEmpty {
                        self.message.stringValue = String(localized: "No components. Use Insert → New Component… to create one.")
                    } else { self.message.stringValue = "" }
                case .failed(let error):
                    self.rows = []
                    self.message.stringValue = error
                }
                self.table.reloadData()
                self.didReadSources?()
                if let index = self.rows.firstIndex(where: { $0.id == selected }) {
                    self.table.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
                } else { self.table.deselectAll(nil) }
                self.updateDetails()
                if let reason = self.saveMessage {
                    self.message.stringValue = reason
                    self.saveMessage = nil
                }
                if self.needsRefresh { self.needsRefresh = false; self.refresh() }
            }
        }
    }

    func invalidate() {
        invalidated = true
        rows = []
        table.reloadData()
        editingEntry = nil
        rootWidth.stringValue = ""
        rootHeight.stringValue = ""
        setEditorEnabled(false)
        details.stringValue = ""
        message.stringValue = ""
        panel.close()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let key = tableColumn?.identifier, rows.indices.contains(row) else { return nil }
        let cell = tableView.makeView(withIdentifier: key, owner: self) as? NSTextField ?? NSTextField(labelWithString: "")
        cell.identifier = key
        cell.lineBreakMode = .byTruncatingMiddle
        let item = rows[row]
        switch key.rawValue {
        case "name": cell.stringValue = item.name
        case "file": cell.stringValue = item.filename
        case "frames": cell.stringValue = String(item.nodeCount)
        default: cell.stringValue = item.width.formatted() + " × " + item.height.formatted()
        }
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) { updateDetails() }
    private func updateDetails() {
        let entry = rows.indices.contains(table.selectedRow) ? rows[table.selectedRow] : nil
        editingEntry = entry
        details.stringValue = entry.map { "design/components/" + $0.filename + "\n" + $0.id.uuidString.lowercased() } ?? ""
        rootWidth.stringValue = entry.map { sizeString($0.width) } ?? ""
        rootHeight.stringValue = entry.map { sizeString($0.height) } ?? ""
        setEditorEnabled(!isLoading && !invalidated && entry != nil)
    }

    private func setEditorEnabled(_ enabled: Bool) {
        rootWidth.isEnabled = enabled
        rootHeight.isEnabled = enabled
        applySize.isEnabled = enabled
        placeInstance.isEnabled = enabled
    }

    private func sizeString(_ number: Float) -> String {
        String(number).replacingOccurrences(of: ".", with: Locale.current.decimalSeparator ?? ".")
    }

    private func sizeValue(_ string: String) -> Float? {
        let text = string.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: Locale.current.decimalSeparator ?? ".", with: ".")
        guard let value = Float(text), value.isFinite, value >= 0 else { return nil }
        return value
    }

    @objc private func placeSelected() {
        guard !invalidated, !isLoading, let entry = editingEntry else { return }
        beginPlacement?(entry)
    }

    @objc func commitRootSize() {
        guard !invalidated, !isLoading, let entry = editingEntry else { return }
        guard let width = sizeValue(rootWidth.stringValue), let height = sizeValue(rootHeight.stringValue) else {
            message.stringValue = String(localized: "Enter a finite width and height of zero or greater.")
            return
        }
        isLoading = true
        setEditorEnabled(false)
        refreshButton.isEnabled = false
        message.stringValue = String(localized: "Saving component size…")
        let runtime = self.runtime, session = self.session
        worker.async { [weak self] in
            let error = CoreRuntime.resizeComponent(handle: runtime, session: session, entry: entry, width: width, height: height)
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.invalidated else { return }
                self.isLoading = false
                self.needsRefresh = false
                self.saveMessage = error
                self.refresh()
            }
        }
    }
}

extension ShellProjectWindow {
    func reloadComponentCanvas() {
        guard let project = dataSource.session, let session = windowController.windowSession, !session.isClosing, !session.isClosed else { return }
        _ = session.track({ project.pump.reloadComponentCanvas(session.session, completion: $0) }) { [weak self, weak session] event in
            guard let self, self.windowController.windowSession === session, case .windowViewChanged = event else { return }
            self.canvasHost.canvasView.invalidateComponentFrames()
        }
    }

    func showComponentLibrary() {
        guard let window, let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosed, !session.isClosing, gitSource.trusted else { return }
        if componentLibrary == nil {
            let library = ShellComponentLibrary(runtime: project.pump.runtimeHandle, session: session.session, projectName: project.name)
            library.didReadSources = { [weak self] in self?.reloadComponentCanvas() }
            library.beginPlacement = { [weak self] entry in self?.promptForInstance(entry) }
            componentLibrary = library
        }
        componentLibrary?.show(relativeTo: window)
    }
}

extension ShellProjectWindow {
    fileprivate func promptForInstance(_ entry: LibraryComponent) {
        guard gitSource.trusted, let project = dataSource.session, let session = windowController.windowSession,
              let library = componentLibrary, let view = session.viewInfo, view.has_view != 0, let state = session.info,
              !session.isClosing, !session.isClosed else { return }
        let viewID = UUID(uuid: view.active_view)
        let sheet = NSAlert()
        sheet.messageText = String(localized: "Place Instance")
        sheet.informativeText = String(localized: "Position this instance in the active View, in logical pixels.")
        let fields = ["X", "Y"].map { label -> NSTextField in
            let field = NSTextField(string: "0")
            field.setAccessibilityLabel(label)
            field.identifier = NSUserInterfaceItemIdentifier("instance." + label.lowercased())
            field.widthAnchor.constraint(equalToConstant: 82).isActive = true
            return field
        }
        let coordinates = NSStackView(views: [NSTextField(labelWithString: "X"), fields[0],
                                              NSTextField(labelWithString: "Y"), fields[1]])
        coordinates.frame = NSRect(x: 0, y: 0, width: 250, height: 24)
        sheet.accessoryView = coordinates
        sheet.addButton(withTitle: String(localized: "Place"))
        sheet.addButton(withTitle: String(localized: "Cancel"))
        sheet.window.initialFirstResponder = fields[0]
        sheet.beginSheetModal(for: library.panel) { [weak self, weak library] response in
            guard response == .alertFirstButtonReturn, let self, let library,
                  self.componentLibrary === library, self.windowController.windowSession === session,
                  self.gitSource.trusted, !session.isClosing, !session.isClosed else { return }
            func coordinate(_ field: NSTextField) -> Float? {
                let separator = Locale.current.decimalSeparator ?? "."
                return Float(field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
                    .replacingOccurrences(of: separator, with: "."))
            }
            guard let x = coordinate(fields[0]), let y = coordinate(fields[1]), x.isFinite, y.isFinite else {
                library.message.stringValue = String(localized: "Enter finite X and Y coordinates.")
                return
            }
            let submitted = session.track({ done in
                project.pump.placeInstance(session: session.session, view: viewID, revision: state.revision,
                                           entry: entry, x: x, y: y, completion: done)
            }) { [weak self, weak library] event in
                self?.refreshSessionCommands()
                if case .windowViewChanged = event { self?.reloadComponentCanvas() }
                guard let library, self?.componentLibrary === library else { return }
                if case .requestFailed(_, _, let reason) = event { library.message.stringValue = reason }
                else { library.message.stringValue = String(localized: "Instance placed.") }
            }
            if !submitted { library.message.stringValue = String(localized: "The project is busy. Try again.") }
        }
    }
}
