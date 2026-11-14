import AppKit

/// 한 프로젝트 창에서 연 독립 원본의 읽기 전용 목록. 선택한 행에는 프로젝트 상대 경로와 ID를 표시합니다.
@MainActor
final class ShellComponentLibrary: NSObject, NSWindowDelegate, NSTableViewDataSource, NSTableViewDelegate {
    private let runtime: UInt64
    private let session: UInt64
    private let worker = DispatchQueue(label: "xyz.taeyoon.stellar.component-library", qos: .userInitiated)
    private let table = NSTableView()
    private let refreshButton = NSButton(title: String(localized: "Refresh"), target: nil, action: nil)
    private(set) var rows: [LibraryComponent] = []
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
        let stack = NSStackView(views: [bar, scroll, details])
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
                if let index = self.rows.firstIndex(where: { $0.id == selected }) {
                    self.table.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
                } else { self.table.deselectAll(nil) }
                self.updateDetails()
                if self.needsRefresh { self.needsRefresh = false; self.refresh() }
            }
        }
    }

    func invalidate() {
        invalidated = true
        rows = []
        table.reloadData()
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
        guard rows.indices.contains(table.selectedRow) else { details.stringValue = ""; return }
        let item = rows[table.selectedRow]
        details.stringValue = "design/components/" + item.filename + "\n" + item.id.uuidString.lowercased()
    }
}

extension ShellProjectWindow {
    func showComponentLibrary() {
        guard let window, let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosed, !session.isClosing, gitSource.trusted else { return }
        if componentLibrary == nil {
            componentLibrary = ShellComponentLibrary(runtime: project.pump.runtimeHandle, session: session.session, projectName: project.name)
        }
        componentLibrary?.show(relativeTo: window)
    }
}
