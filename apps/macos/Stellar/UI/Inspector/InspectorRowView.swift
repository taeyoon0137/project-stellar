import AppKit

/// 속성 행 하나: CSS 이름 레이블과 값 컨트롤. 값은 코어 snapshot 텍스트를 그대로 보이고, 입력은 텍스트로 보냅니다.
@MainActor
final class InspectorRowView: NSView, NSTextFieldDelegate {
    private(set) var row: InspectorRowState
    /// 편집 요청(이름, 입력, 단계, 완료 시 진단).
    var onEdit: ((String, InspectorInput, InspectorEditPhase, @escaping @MainActor (String?) -> Void) -> Void)?

    let label = NSTextField(labelWithString: "")
    let field = NSTextField()
    private let stack = NSStackView()

    init(row: InspectorRowState) {
        self.row = row
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        label.font = InspectorFonts.label
        label.textColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingMiddle
        label.translatesAutoresizingMaskIntoConstraints = false
        field.font = InspectorFonts.value
        field.controlSize = InspectorMetrics.controlSize
        field.delegate = self
        field.identifier = NSUserInterfaceItemIdentifier(row.name)
        field.target = self
        field.action = #selector(commitField)
        (field.cell as? NSTextFieldCell)?.sendsActionOnEndEditing = true
        field.translatesAutoresizingMaskIntoConstraints = false
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.alignment = .centerY
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(label)
        stack.addArrangedSubview(field)
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            label.widthAnchor.constraint(equalToConstant: InspectorMetrics.labelColumnWidth),
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

    @objc private func commitField() {
        let text = field.stringValue
        guard text != row.value || row.flags.contains(.mixed) && !text.isEmpty else { return }
        onEdit?(row.name, .text(text), .commit) { [weak self] diagnostic in
            guard let self, diagnostic != nil else { return }
            self.field.stringValue = self.row.value
        }
    }
}
