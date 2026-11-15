import AppKit

/// 속성 행 하나: CSS 이름 레이블과 값 컨트롤. 값은 코어 snapshot 텍스트를 그대로 보이고, 입력은 텍스트로 보냅니다.
@MainActor
final class InspectorRowView: NSView, NSTextFieldDelegate {
    private(set) var row: InspectorRowState
    /// 편집 요청(이름, 입력, 단계, 완료 시 진단).
    var onEdit: ((String, InspectorInput, InspectorEditPhase, @escaping @MainActor (String?) -> Void) -> Void)?

    /// 묶음 행(shorthand)의 펼침이 바뀌면 부릅니다.
    var onToggleGroup: ((Bool) -> Void)?

    let label = NSTextField(labelWithString: "")
    let field = NSTextField()
    private let stack = NSStackView()
    /// 묶음 행의 펼침 단추(longhand 행 보이기).
    private var disclosure: NSButton?

    /// - Parameters:
    ///   - labelWidth: 같은 패널 모든 행의 레이블 열 폭(가장 긴 CSS 이름에 맞춤).
    ///   - expanded: 묶음 행이면 처음 펼침 상태.
    init(row: InspectorRowState, labelWidth: CGFloat, expanded: Bool = false) {
        self.row = row
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        // 레이블은 CSS 속성 이름 그대로, 시스템 글꼴(UX 명세 6.3 권장안).
        label.font = InspectorFonts.label
        label.textColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingMiddle
        label.translatesAutoresizingMaskIntoConstraints = false
        label.toolTip = InspectorPropertyCatalog.help(for: row)
        field.font = InspectorFonts.value
        field.controlSize = InspectorMetrics.controlSize
        field.delegate = self
        field.identifier = NSUserInterfaceItemIdentifier(row.name)
        field.target = self
        field.action = #selector(commitField)
        (field.cell as? NSTextFieldCell)?.sendsActionOnEndEditing = true
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setAccessibilityLabel(row.name)
        field.setAccessibilityHelp(InspectorPropertyCatalog.help(for: row))
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.alignment = .centerY
        stack.translatesAutoresizingMaskIntoConstraints = false
        var labelColumn = labelWidth
        if row.flags.contains(.group) {
            let button = NSButton()
            button.bezelStyle = .disclosure
            button.setButtonType(.pushOnPushOff)
            button.title = ""
            button.state = expanded ? .on : .off
            button.target = self
            button.action = #selector(toggleGroup)
            button.setAccessibilityLabel(String(localized: "Show \(row.name) longhands"))
            stack.addArrangedSubview(button)
            disclosure = button
            labelColumn -= 18
        }
        stack.addArrangedSubview(label)
        stack.addArrangedSubview(field)
        addSubview(stack)
        // longhand 행은 묶음 행 아래에 들여 씁니다.
        let indent: CGFloat = row.flags.contains(.longhand) ? 12 : 0
        if indent > 0 { labelColumn -= indent }
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: indent),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            label.widthAnchor.constraint(equalToConstant: max(labelColumn, 40)),
            field.widthAnchor.constraint(greaterThanOrEqualToConstant: InspectorMetrics.minimumFieldWidth),
        ])
        update(row)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("코드로만 생성합니다") }

    /// 같은 구성의 새 값으로 제자리 갱신합니다. 편집 중인 필드는 덮어쓰지 않습니다.
    func update(_ next: InspectorRowState) {
        row = next
        label.stringValue = next.name
        let editing = field.currentEditor() != nil
        if !editing {
            field.stringValue = next.value
        }
        field.placeholderString = next.flags.contains(.mixed) ? String(localized: "Mixed") : nil
        field.isEditable = !next.flags.contains(.readOnly)
        field.isEnabled = !next.flags.contains(.readOnly)
    }

    @objc private func toggleGroup() {
        onToggleGroup?(disclosure?.state == .on)
    }

    @objc private func commitField() {
        let text = field.stringValue
        guard text != row.value || row.flags.contains(.mixed) && !text.isEmpty else { return }
        onEdit?(row.name, .text(text), .commit) { [weak self] diagnostic in
            guard let self, diagnostic != nil else { return }
            self.field.stringValue = self.row.value
        }
    }
}
