import AppKit

/// 속성 행 하나: CSS 이름 레이블(끌면 scrub)과 값 컨트롤, 행 아래 진단 한 줄.
///
/// 값은 코어 snapshot 텍스트를 그대로 보이고, 입력은 CSS 값 텍스트·수치 delta로 보냅니다(UX 명세 6.4).
/// - Return 적용, Escape 취소 후 캔버스로, Tab 다음 필드.
/// - ↑↓ ±1, Shift ±10. 이어 누른 화살표는 연속 조작 하나(Undo 한 번)입니다.
/// - 레이블을 좌우로 끌면 scrub. 끌기 한 번이 Undo 한 번입니다.
/// - 잘못된 값은 적용되지 않고 행 아래 진단(`unsupported-unit: …`)으로 보입니다.
@MainActor
final class InspectorRowView: NSView, NSTextFieldDelegate {
    private(set) var row: InspectorRowState
    /// 편집 요청(이름, 입력, 단계, 완료 시 진단).
    var onEdit: ((String, InspectorInput, InspectorEditPhase, @escaping @MainActor (String?) -> Void) -> Void)?
    /// 묶음 행(shorthand)의 펼침이 바뀌면 부릅니다.
    var onToggleGroup: ((Bool) -> Void)?
    /// Escape로 입력을 끝냈을 때(포커스를 캔버스로).
    var onEscape: (() -> Void)?
    /// scrub·화살표 같은 연속 조작이 시작·끝날 때(섹션 구성 고정, UX.2.12).
    var onGesture: ((Bool) -> Void)?

    let label = InspectorScrubLabel(labelWithString: "")
    let field = NSTextField()
    private var popup: NSPopUpButton?
    /// 색 행의 견본: 누르면 popover, 그 안에서 시스템 색 패널로 이어집니다(UX 명세 6.4).
    private var swatch: NSColorWell?
    private var colorTimer: Timer?
    private var lastColorText: String?
    private var checkbox: NSButton?
    private let diagnostic = NSTextField(wrappingLabelWithString: "")
    private let line = NSStackView()
    /// 묶음 행의 펼침 단추(longhand 행 보이기).
    private var disclosure: NSButton?

    /// 사용자가 필드에 입력을 시작했는지(그 사이에는 snapshot 값으로 덮어쓰지 않음).
    private var userTyped = false
    /// 이어 누른 화살표의 누적 delta와 끝 타이머.
    private var arrowTotal = 0.0
    private var arrowTimer: Timer?

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
        label.scrubs = row.flags.contains(.numeric) && !row.flags.contains(.readOnly)
        label.onScrub = { [weak self] delta, phase in self?.scrub(delta, phase: phase) }
        label.onClick = { [weak self] in
            guard let self, !self.field.isHidden else { return }
            self.window?.makeFirstResponder(self.field)
        }
        line.orientation = .horizontal
        line.spacing = 6
        line.alignment = .centerY
        line.translatesAutoresizingMaskIntoConstraints = false
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
            line.addArrangedSubview(button)
            disclosure = button
            labelColumn -= 18
        }
        line.addArrangedSubview(label)
        buildControl()
        diagnostic.font = InspectorFonts.caption
        diagnostic.textColor = .systemRed
        diagnostic.isHidden = true
        diagnostic.translatesAutoresizingMaskIntoConstraints = false
        let column = NSStackView(views: [line, diagnostic])
        column.orientation = .vertical
        column.alignment = .leading
        column.spacing = 2
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        // longhand 행은 묶음 행 아래에 들여 씁니다.
        let indent: CGFloat = row.flags.contains(.longhand) ? 12 : 0
        labelColumn -= indent
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: leadingAnchor, constant: indent),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor),
            line.widthAnchor.constraint(equalTo: column.widthAnchor),
            diagnostic.trailingAnchor.constraint(equalTo: column.trailingAnchor),
        ])
        // 패널이 아주 좁아도 제약 충돌 없이 값 필드가 먼저 줄어듭니다.
        let labelWidthConstraint = label.widthAnchor.constraint(equalToConstant: max(labelColumn, 40))
        labelWidthConstraint.priority = .init(900)
        labelWidthConstraint.isActive = true
        update(row)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("코드로만 생성합니다") }

    private func buildControl() {
        let help = InspectorPropertyCatalog.help(for: row)
        if row.flags.contains(.bool) {
            let box = NSButton(checkboxWithTitle: "", target: self, action: #selector(toggleBool))
            box.controlSize = InspectorMetrics.controlSize
            box.setAccessibilityLabel(row.name)
            box.setAccessibilityHelp(help)
            line.addArrangedSubview(box)
            checkbox = box
            field.isHidden = true
            return
        }
        if row.flags.contains(.keyword) {
            let menu = NSPopUpButton(frame: .zero, pullsDown: false)
            menu.controlSize = InspectorMetrics.controlSize
            menu.font = InspectorFonts.value
            menu.target = self
            menu.action = #selector(chooseKeyword)
            menu.setAccessibilityLabel(row.name)
            menu.setAccessibilityHelp(help)
            menu.identifier = NSUserInterfaceItemIdentifier(row.name)
            line.addArrangedSubview(menu)
            let minimum = menu.widthAnchor.constraint(greaterThanOrEqualToConstant: InspectorMetrics.minimumFieldWidth)
            minimum.priority = .init(800)
            minimum.isActive = true
            popup = menu
            field.isHidden = true
            return
        }
        if row.flags.contains(.color) {
            let well = NSColorWell(style: .minimal)
            well.controlSize = InspectorMetrics.controlSize
            well.supportsAlpha = true
            well.target = self
            well.action = #selector(pickColor)
            well.setAccessibilityLabel(String(localized: "\(row.name) color"))
            well.setAccessibilityHelp(String(localized: "Opens colors; Show Colors opens the system color panel."))
            well.translatesAutoresizingMaskIntoConstraints = false
            line.addArrangedSubview(well)
            NSLayoutConstraint.activate([
                well.widthAnchor.constraint(equalToConstant: 28),
                well.heightAnchor.constraint(equalToConstant: 18),
            ])
            swatch = well
        }
        field.font = InspectorFonts.value
        field.controlSize = InspectorMetrics.controlSize
        field.delegate = self
        field.identifier = NSUserInterfaceItemIdentifier(row.name)
        field.target = self
        field.action = #selector(commitField)
        (field.cell as? NSTextFieldCell)?.sendsActionOnEndEditing = true
        field.lineBreakMode = .byTruncatingTail
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setAccessibilityLabel(row.name)
        field.setAccessibilityHelp(help)
        line.addArrangedSubview(field)
        let minimum = field.widthAnchor.constraint(greaterThanOrEqualToConstant: InspectorMetrics.minimumFieldWidth)
        minimum.priority = .init(800)
        minimum.isActive = true
        field.setContentHuggingPriority(.init(100), for: .horizontal)
    }

    /// 키보드 포커스를 받는 값 컨트롤(Tab 순서).
    var keyControl: NSView? {
        if let checkbox { return checkbox }
        if let popup { return popup }
        return field
    }

    /// 같은 구성의 새 값으로 제자리 갱신합니다. 사용자가 입력 중인 필드는 덮어쓰지 않습니다.
    func update(_ next: InspectorRowState) {
        let changed = next.value != row.value || next.flags != row.flags
        row = next
        label.stringValue = next.name
        let editable = !next.flags.contains(.readOnly)
        let mixed = next.flags.contains(.mixed)
        if !userTyped { display(next.value) }
        field.placeholderString = mixed ? String(localized: "Mixed") : nil
        if field.currentEditor() == nil || !editable {
            field.isEditable = editable
            field.isEnabled = editable
        }
        label.scrubs = next.flags.contains(.numeric) && editable
        if let popup {
            popup.removeAllItems()
            if mixed { popup.addItem(withTitle: String(localized: "Mixed")) }
            popup.addItems(withTitles: next.choices)
            if !mixed { popup.selectItem(withTitle: next.value) }
            popup.isEnabled = editable
        }
        if let swatch, colorTimer == nil {
            // 견본은 코어가 정규화한 값을 표시만 합니다. none·Mixed는 투명.
            swatch.color = mixed ? .clear : (InspectorColor(css: next.value)?.nsColor ?? .clear)
            swatch.isEnabled = editable
            swatch.toolTip = mixed ? String(localized: "Mixed") : next.value
        }
        if let checkbox {
            checkbox.allowsMixedState = mixed
            checkbox.state = mixed ? .mixed : (next.value == "true" ? .on : .off)
            checkbox.isEnabled = editable
        }
        if changed { show(nil) }
    }

    /// 필드에 값을 보입니다. 포커스가 있으면 편집을 끝내지 않고 field editor 내용만 바꿉니다(필드 값을 직접 바꾸면
    /// 편집이 끝나 화살표 반복이 끊김).
    private func display(_ text: String) {
        guard let editor = field.currentEditor() else {
            field.stringValue = text
            return
        }
        guard editor.string != text else { return }
        let selection = editor.selectedRange
        editor.string = text
        let length = text.utf16.count
        let location = min(selection.location, length)
        editor.selectedRange = NSRange(location: location, length: min(selection.length, length - location))
    }

    /// 행 아래 진단 한 줄. nil이면 숨깁니다.
    func show(_ message: String?) {
        diagnostic.stringValue = message ?? ""
        diagnostic.isHidden = message?.isEmpty ?? true
        if let message, !message.isEmpty {
            NSAccessibility.post(element: field, notification: .announcementRequested,
                                 userInfo: [.announcement: message, .priority: NSAccessibilityPriorityLevel.high.rawValue])
        }
    }

    private func send(_ input: InspectorInput, _ phase: InspectorEditPhase) {
        onEdit?(row.name, input, phase) { [weak self] message in
            guard let self else { return }
            if let message {
                self.show(message)
                if !self.userTyped { self.display(self.row.value) }
            } else if phase != .continuous {
                self.show(nil)
            }
        }
    }

    // MARK: - 입력

    @objc private func toggleGroup() {
        onToggleGroup?(disclosure?.state == .on)
    }

    /// 색 견본·색 패널 변경. 끄는 동안은 연속 조작이고, 0.4초 쉬면 끝냅니다(Undo 한 번).
    @objc private func pickColor() {
        guard let swatch, let color = InspectorColor(swatch.color) else { return }
        let text = color.cssText
        guard text != lastColorText else { return }
        if colorTimer == nil { onGesture?(true) }
        lastColorText = text
        send(.text(text), .continuous)
        colorTimer?.invalidate()
        colorTimer = Timer.scheduledTimer(withTimeInterval: 0.4, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.endColor() }
        }
    }

    private func endColor() {
        colorTimer?.invalidate()
        colorTimer = nil
        if let text = lastColorText { send(.text(text), .ended) }
        lastColorText = nil
        onGesture?(false)
    }

    @objc private func toggleBool() {
        guard let checkbox else { return }
        let value = checkbox.state == .off ? "false" : "true"
        checkbox.allowsMixedState = false
        send(.text(value), .commit)
    }

    @objc private func chooseKeyword() {
        guard let title = popup?.titleOfSelectedItem, row.choices.contains(title) else { return }
        send(.text(title), .commit)
    }

    @objc private func commitField() {
        let text = field.stringValue
        let typed = userTyped
        userTyped = false
        guard typed, text != row.value || row.flags.contains(.mixed) else { return }
        guard !text.isEmpty || !row.flags.contains(.mixed) else { return }
        send(.text(text), .commit)
    }

    func controlTextDidChange(_ notification: Notification) {
        userTyped = true
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.cancelOperation(_:)):
            // Escape: 입력을 버리고 snapshot 값으로 돌린 뒤 캔버스로.
            userTyped = false
            show(nil)
            field.abortEditing()
            field.stringValue = row.value
            onEscape?()
            return true
        case #selector(NSResponder.moveUp(_:)), #selector(NSResponder.moveUpAndModifySelection(_:)):
            return arrow(+1, shift: selector == #selector(NSResponder.moveUpAndModifySelection(_:)))
        case #selector(NSResponder.moveDown(_:)), #selector(NSResponder.moveDownAndModifySelection(_:)):
            return arrow(-1, shift: selector == #selector(NSResponder.moveDownAndModifySelection(_:)))
        default:
            return false
        }
    }

    /// ↑↓ ±1, Shift ±10. 이어 누르면 연속 조작 하나로 묶고, 0.5초 쉬면 끝냅니다.
    private func arrow(_ sign: Double, shift: Bool) -> Bool {
        guard row.flags.contains(.numeric), !row.flags.contains(.readOnly) else { return false }
        if userTyped { commitField() }
        let shiftDown = shift || NSApp.currentEvent?.modifierFlags.contains(.shift) == true
        if arrowTimer == nil { onGesture?(true) }
        arrowTotal += sign * (shiftDown ? 10 : 1)
        send(.adjust(arrowTotal), .continuous)
        arrowTimer?.invalidate()
        arrowTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.endArrows() }
        }
        return true
    }

    private func endArrows() {
        arrowTimer?.invalidate()
        arrowTimer = nil
        let total = arrowTotal
        arrowTotal = 0
        if total != 0 { send(.adjust(total), .ended) }
        onGesture?(false)
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        if arrowTimer != nil { endArrows() }
    }

    private func scrub(_ delta: Double, phase: InspectorEditPhase) {
        switch phase {
        case .continuous:
            onGesture?(true)
            send(.adjust(delta), .continuous)
        case .ended:
            send(.adjust(delta), delta != 0 ? .ended : .cancel)
            onGesture?(false)
        case .cancel:
            send(.adjust(0), .cancel)
            onGesture?(false)
        case .commit:
            break
        }
    }
}

/// 좌우로 끌면 값을 바꾸는 행 레이블(scrub). 1pt = 1, Shift ×10, Option ×0.1. 끌기 한 번이 Undo 한 번입니다.
@MainActor
final class InspectorScrubLabel: NSTextField {
    var scrubs = false {
        didSet { if scrubs != oldValue { window?.invalidateCursorRects(for: self) } }
    }
    /// 누적 delta와 단계(continuous 여러 번 뒤 ended 또는 cancel).
    var onScrub: ((Double, InspectorEditPhase) -> Void)?
    /// 끌지 않고 누르기만 했을 때.
    var onClick: (() -> Void)?

    private var start: NSPoint?
    private var total = 0.0
    private var moved = false

    override func resetCursorRects() {
        if scrubs { addCursorRect(bounds, cursor: .resizeLeftRight) }
    }

    override func mouseDown(with event: NSEvent) {
        start = scrubs ? event.locationInWindow : nil
        total = 0
        moved = false
        if !scrubs { onClick?() }
    }

    override func mouseDragged(with event: NSEvent) {
        guard let start else { return }
        let distance = Double(event.locationInWindow.x - start.x)
        let scale = event.modifierFlags.contains(.shift) ? 10 : (event.modifierFlags.contains(.option) ? 0.1 : 1)
        let next = (distance * scale * 10).rounded() / 10
        guard next != total else { return }
        moved = true
        total = next
        onScrub?(total, .continuous)
    }

    override func mouseUp(with event: NSEvent) {
        defer { start = nil }
        guard start != nil else { return }
        if moved {
            onScrub?(total, .ended)
        } else {
            onClick?()
        }
    }
}
