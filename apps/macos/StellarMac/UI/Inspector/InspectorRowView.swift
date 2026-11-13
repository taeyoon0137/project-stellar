import AppKit

/// 인스펙터 한 줄: 레이블 열 + 필드 컨트롤 + 진단 표시 + Value Binding 표시(placeholder).
///
/// 접근성 트리에서는 행 하나를 group 요소로 묶고, 행 레이블을 group 레이블로 씁니다(가이드 8.3).
final class InspectorRowView: NSView {
    private(set) var row: InspectorRow
    /// 읽기 전용 강제 비활성 상태.
    private(set) var forceDisabled: Bool
    private(set) var fieldControllers: [InspectorFieldController] = []
    private let stack = NSStackView()
    private let label: NSTextField
    private var bindingButton: NSButton?
    private var issueView: NSImageView?
    private let send: (InspectorCommand) -> Void

    init(row: InspectorRow, forceDisabled: Bool, send: @escaping (InspectorCommand) -> Void) {
        self.row = row
        self.forceDisabled = forceDisabled
        self.send = send
        label = InspectorFieldController.makeLabel(row.label, color: .secondaryLabelColor)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build(forceDisabled: forceDisabled)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorRowView는 코드로만 생성합니다")
    }

    /// 같은 뷰 구성으로 새 행을 반영할 수 있는지.
    func canUpdate(to newRow: InspectorRow) -> Bool {
        guard newRow.id == row.id, newRow.fields.count == fieldControllers.count,
              (newRow.issue == nil) == (row.issue == nil),
              Self.hasBinding(newRow) == Self.hasBinding(row) else { return false }
        return zip(fieldControllers, newRow.fields).allSatisfy { $0.canUpdate(to: $1) }
    }

    /// 값을 제자리에서 갱신합니다. 먼저 `canUpdate(to:)`로 확인합니다.
    func update(_ newRow: InspectorRow, forceDisabled: Bool) {
        row = newRow
        self.forceDisabled = forceDisabled
        label.stringValue = newRow.label
        for (controller, field) in zip(fieldControllers, newRow.fields) {
            controller.update(field, forceDisabled: forceDisabled)
        }
        refreshDecorations()
    }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] {
        fieldControllers.flatMap(\.keyViews) + (bindingButton.map { [$0] } ?? [])
    }

    // MARK: - 구성

    private func build(forceDisabled: Bool) {
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.distribution = .fill
        stack.spacing = InspectorMetrics.groupSpacing
        // 좁은 폭에서는 필드(400)·popup·segmented(450)가 먼저 줄어들고, 그래도 모자라면 창을 넓히지 않고
        // 행 안에서 잘리게 합니다. 창 크기 유지 우선순위(windowSizeStayPut, 500)보다 낮아야 인스펙터 내용이
        // pane·창 폭을 밀어내지 않습니다.
        stack.setClippingResistancePriority(.init(490), for: .horizontal)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        label.setAccessibilityElement(false)
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        stack.addArrangedSubview(label)
        stack.setCustomSpacing(InspectorMetrics.rowSpacing, after: label)
        let labelWidth = label.widthAnchor.constraint(equalToConstant: InspectorMetrics.labelColumnWidth)
        labelWidth.priority = .init(999)

        var flexible: [NSView] = []
        for field in row.fields {
            let controller = InspectorFieldController(field: field, forceDisabled: forceDisabled)
            controller.send = send
            fieldControllers.append(controller)
            for view in controller.views {
                stack.addArrangedSubview(view)
            }
            if let last = controller.views.last {
                stack.setCustomSpacing(InspectorMetrics.rowSpacing, after: last)
            }
            flexible += controller.flexibleViews
        }

        // 남는 폭: 유연한 컨트롤이 같은 폭으로 나눠 갖고, 없으면 spacer가 가져갑니다(모호성 방지).
        var constraints = [labelWidth]
        if let first = flexible.first {
            for view in flexible.dropFirst() {
                constraints.append(view.widthAnchor.constraint(equalTo: first.widthAnchor))
            }
        } else {
            let spacer = NSView()
            spacer.translatesAutoresizingMaskIntoConstraints = false
            spacer.setContentHuggingPriority(.init(1), for: .horizontal)
            spacer.setAccessibilityElement(false)
            stack.addArrangedSubview(spacer)
        }

        if row.issue != nil {
            let issue = NSImageView()
            issue.image = InspectorSymbols.image(InspectorSymbols.issue, label: "Issue")
            issue.contentTintColor = .systemOrange
            issue.setAccessibilityElement(true)
            issue.setAccessibilityRole(.image)
            InspectorFieldController.hug(issue)
            issueView = issue
            stack.addArrangedSubview(issue)
        }
        if Self.hasBinding(row) {
            let button = NSButton()
            button.title = ""
            button.target = self
            button.action = #selector(bindingPressed(_:))
            button.isBordered = false
            button.imagePosition = .imageOnly
            button.controlSize = InspectorMetrics.controlSize
            InspectorFieldController.hug(button)
            bindingButton = button
            stack.addArrangedSubview(button)
        }

        constraints += [
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ]
        NSLayoutConstraint.activate(constraints)

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        refreshDecorations()
    }

    /// 행 레이블·진단·binding 표시를 갱신합니다.
    private func refreshDecorations() {
        let groupLabel = row.label.isEmpty ? (row.fields.first?.label ?? "") : row.label
        setAccessibilityLabel(groupLabel)

        if let issueView, let issue = row.issue {
            issueView.toolTip = issue
            issueView.setAccessibilityLabel("Issue: \(issue)")
        }
        if let bindingButton, let binding = Self.aggregateBinding(row) {
            let symbol = InspectorSymbols.binding(binding)
            let description = "Value Binding for \(groupLabel)"
            let image = InspectorSymbols.image(symbol, label: description)?
                .withSymbolConfiguration(NSImage.SymbolConfiguration(textStyle: .body, scale: .small))
            bindingButton.image = image
            switch binding {
            case .fixed: bindingButton.contentTintColor = .tertiaryLabelColor
            case .orphaned: bindingButton.contentTintColor = .systemOrange
            default: bindingButton.contentTintColor = .controlAccentColor
            }
            let summary = row.fields.compactMap { field in
                field.binding.map { "\(field.label): \($0.summary)" }
            }.joined(separator: "; ")
            bindingButton.toolTip = summary
            bindingButton.setAccessibilityLabel(description)
            bindingButton.setAccessibilityValueDescription(summary)
        }
    }

    @objc private func bindingPressed(_ sender: NSButton) {
        guard let field = row.fields.first(where: { $0.binding != nil }) else { return }
        send(.showBinding(field.id))
    }

    private static func hasBinding(_ row: InspectorRow) -> Bool {
        row.fields.contains { $0.binding != nil }
    }

    /// 행 대표 binding 상태: 끊어진 연결 > 연결 > 고정값 순.
    private static func aggregateBinding(_ row: InspectorRow) -> InspectorBinding? {
        let bindings = row.fields.compactMap(\.binding)
        if let orphaned = bindings.first(where: { if case .orphaned = $0 { return true } else { return false } }) {
            return orphaned
        }
        return bindings.first(where: \.isBound) ?? bindings.first
    }
}

/// 여러 행과 보조 설명을 세로로 쌓은 뷰. 섹션 본문과 modifier 상세에 함께 씁니다.
final class InspectorRowsView: NSView {
    private let stack = NSStackView()
    private(set) var rowViews: [InspectorRowView] = []
    private var noteLabel: InspectorWrappingLabel?
    private let send: (InspectorCommand) -> Void

    init(send: @escaping (InspectorCommand) -> Void) {
        self.send = send
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = InspectorMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorRowsView는 코드로만 생성합니다")
    }

    /// 행을 반영합니다. 구성이 같은 행은 제자리 갱신하고, 다르면 그 행만 새로 만듭니다.
    func apply(rows: [InspectorRow], note: String?, forceDisabled: Bool) {
        if rows.count == rowViews.count {
            for (index, row) in rows.enumerated() {
                let existing = rowViews[index]
                if existing.row == row, existing.forceDisabled == forceDisabled {
                    continue // 바뀌지 않은 행은 건드리지 않습니다.
                }
                if existing.canUpdate(to: row) {
                    existing.update(row, forceDisabled: forceDisabled)
                } else {
                    let replacement = InspectorRowView(row: row, forceDisabled: forceDisabled, send: send)
                    stack.insertArrangedSubview(replacement, at: index)
                    pinWidth(replacement)
                    existing.removeFromSuperview()
                    rowViews[index] = replacement
                }
            }
        } else {
            rowViews.forEach { $0.removeFromSuperview() }
            rowViews = rows.enumerated().map { index, row in
                let view = InspectorRowView(row: row, forceDisabled: forceDisabled, send: send)
                stack.insertArrangedSubview(view, at: index)
                pinWidth(view)
                return view
            }
        }
        applyNote(note)
    }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] { rowViews.flatMap(\.keyViews) }

    private func applyNote(_ note: String?) {
        guard let note else {
            noteLabel?.removeFromSuperview()
            noteLabel = nil
            return
        }
        if let noteLabel {
            noteLabel.stringValue = note
            return
        }
        let label = InspectorWrappingLabel(text: note)
        stack.addArrangedSubview(label)
        pinWidth(label)
        noteLabel = label
    }

    private func pinWidth(_ view: NSView) {
        view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }
}

/// 폭에 맞춰 줄바꿈하는 보조 설명 레이블.
final class InspectorWrappingLabel: NSTextField {
    init(text: String, font: NSFont = InspectorFonts.caption, color: NSColor = .secondaryLabelColor) {
        super.init(frame: .zero)
        stringValue = text
        isEditable = false
        isSelectable = false
        isBordered = false
        drawsBackground = false
        self.font = font
        textColor = color
        lineBreakMode = .byWordWrapping
        maximumNumberOfLines = 0
        cell?.wraps = true
        translatesAutoresizingMaskIntoConstraints = false
        setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorWrappingLabel은 코드로만 생성합니다")
    }

    override func layout() {
        super.layout()
        // 줄바꿈 높이를 현재 폭 기준으로 다시 계산합니다.
        if preferredMaxLayoutWidth != bounds.width {
            preferredMaxLayoutWidth = bounds.width
            invalidateIntrinsicContentSize()
        }
    }
}
