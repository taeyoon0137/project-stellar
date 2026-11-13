import AppKit

/// 필드 하나(`InspectorField`)를 네이티브 컨트롤로 표시하고 편집을 명령으로 바꿉니다.
///
/// 뷰를 직접 소유하지 않고 행(`InspectorRowView`)에 넣을 뷰 목록을 만듭니다. 컨트롤 종류는 모두
/// AppKit 표준 컨트롤입니다(가이드 5.1, 5.2). 값의 의미·검증은 코어가 맡고, 여기서는 표시 단위
/// 변환(`displayScale`)과 범위 clamp만 합니다.
final class InspectorFieldController: NSObject, NSTextFieldDelegate {
    private(set) var field: InspectorField
    /// 행에 순서대로 넣을 뷰.
    private(set) var views: [NSView] = []
    /// 행의 남는 폭을 나눠 가질 뷰.
    private(set) var flexibleViews: [NSView] = []
    /// 키보드 포커스 순서에 들어갈 컨트롤.
    private(set) var keyViews: [NSView] = []
    /// 편집 명령 전달.
    var send: ((InspectorCommand) -> Void)?

    private var forceDisabled: Bool
    private var textField: NSTextField?
    private var stepper: NSStepper?
    private var toggle: NSSwitch?
    private var colorWell: NSColorWell?
    private var popup: NSPopUpButton?
    private var segmented: NSSegmentedControl?
    private var button: NSButton?
    private var valueLabel: NSTextField?
    private var mixedLabel: NSTextField?
    private let numberFormatter = NumberFormatter()

    /// 혼합 값 표시 문자열.
    static let mixedText = "Mixed"
    /// switch·색 well 옆 혼합 표시.
    static let mixedGlyph = "–"
    /// 값 없음 표시 문자열.
    static let noValueText = "—"

    init(field: InspectorField, forceDisabled: Bool) {
        self.field = field
        self.forceDisabled = forceDisabled
        super.init()
        build()
        refresh()
    }

    /// 같은 컨트롤 구성으로 새 값을 반영할 수 있는지.
    func canUpdate(to newField: InspectorField) -> Bool {
        newField.id == field.id && newField.control == field.control && newField.shortLabel == field.shortLabel
    }

    /// 값·편집 가능 상태를 제자리에서 갱신합니다.
    func update(_ newField: InspectorField, forceDisabled: Bool) {
        field = newField
        self.forceDisabled = forceDisabled
        refresh()
    }

    /// 사용자가 편집할 수 있는 상태인지.
    private var isInteractive: Bool { field.isEditable && !forceDisabled }

    // MARK: - 구성

    private func build() {
        if let short = field.shortLabel {
            let label = Self.makeLabel(short, color: .secondaryLabelColor)
            label.setAccessibilityElement(false)
            views.append(label)
        }
        switch field.control {
        case .number(let spec): buildNumber(spec)
        case .toggle: buildToggle()
        case .color: buildColor()
        case .popup(let choices): buildPopup(choices)
        case .segmented(let choices): buildSegmented(choices)
        case .text(let placeholder): buildText(placeholder)
        case .readOnly: buildReadOnly()
        case .action(let title, let symbolName): buildAction(title: title, symbolName: symbolName)
        }
    }

    private func buildNumber(_ spec: InspectorNumberSpec) {
        numberFormatter.numberStyle = .decimal
        numberFormatter.usesGroupingSeparator = false
        numberFormatter.minimumFractionDigits = 0
        numberFormatter.maximumFractionDigits = spec.fractionDigits
        numberFormatter.isLenient = true

        let field = NativeTextControls.makeGeneralField(placeholder: "")
        configureTextField(field, font: InspectorFonts.number)
        field.alignment = .right
        field.action = #selector(numberCommitted(_:))
        if spec.unit != .none {
            field.setAccessibilityHelp("Value in \(spec.unit.spokenName)")
        }
        textField = field
        views.append(field)
        flexibleViews.append(field)
        keyViews.append(field)

        if spec.unit != .none {
            let unit = Self.makeLabel(spec.unit.rawValue, color: .secondaryLabelColor)
            unit.setAccessibilityElement(false)
            views.append(unit)
        }
        if spec.showsStepper {
            let stepper = NSStepper()
            stepper.controlSize = InspectorMetrics.controlSize
            stepper.autorepeat = true
            stepper.valueWraps = false
            stepper.increment = spec.step
            stepper.target = self
            stepper.action = #selector(stepperChanged(_:))
            stepper.setAccessibilityLabel(self.field.label)
            stepper.identifier = NSUserInterfaceItemIdentifier(self.field.id.path + ".stepper")
            Self.hug(stepper)
            self.stepper = stepper
            views.append(stepper)
            keyViews.append(stepper)
        }
    }

    private func buildToggle() {
        let toggle = NSSwitch()
        toggle.controlSize = InspectorMetrics.controlSize
        toggle.target = self
        toggle.action = #selector(toggleChanged(_:))
        toggle.setAccessibilityLabel(field.label)
        toggle.toolTip = field.label
        toggle.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        Self.hug(toggle)
        self.toggle = toggle
        views.append(toggle)
        keyViews.append(toggle)
        appendMixedLabel()
    }

    private func buildColor() {
        let well = NSColorWell(style: .minimal)
        well.controlSize = InspectorMetrics.controlSize
        well.supportsAlpha = true
        well.target = self
        well.action = #selector(colorChanged(_:))
        well.setAccessibilityLabel(field.label)
        well.toolTip = field.label
        well.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        Self.hug(well)
        colorWell = well
        views.append(well)
        keyViews.append(well)
        appendMixedLabel()
    }

    private func buildPopup(_ choices: [InspectorChoice]) {
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        popup.controlSize = InspectorMetrics.controlSize
        popup.font = InspectorFonts.control
        popup.autoenablesItems = false
        for choice in choices {
            let item = NSMenuItem(title: choice.title, action: nil, keyEquivalent: "")
            item.representedObject = choice.id
            if let symbol = choice.symbolName {
                item.image = InspectorSymbols.image(symbol, label: choice.title)
            }
            popup.menu?.addItem(item)
        }
        popup.target = self
        popup.action = #selector(popupChanged(_:))
        popup.setAccessibilityLabel(field.label)
        popup.toolTip = field.label
        popup.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        // 좁은 폭에서는 제목을 줄여 표시합니다(행 밖으로 넘치지 않게).
        popup.setContentCompressionResistancePriority(.init(450), for: .horizontal)
        self.popup = popup
        views.append(popup)
        flexibleViews.append(popup)
        keyViews.append(popup)
    }

    private func buildSegmented(_ choices: [InspectorChoice]) {
        let control = NSSegmentedControl()
        control.segmentCount = choices.count
        control.trackingMode = .selectOne
        control.controlSize = InspectorMetrics.controlSize
        control.font = InspectorFonts.control
        control.segmentDistribution = .fillEqually
        for (index, choice) in choices.enumerated() {
            if let symbol = choice.symbolName, let image = InspectorSymbols.image(symbol, label: choice.title) {
                control.setImage(image, forSegment: index)
                control.setLabel("", forSegment: index)
            } else {
                control.setLabel(choice.title, forSegment: index)
            }
            control.setToolTip(choice.title, forSegment: index)
        }
        control.target = self
        control.action = #selector(segmentChanged(_:))
        control.setAccessibilityLabel(field.label)
        control.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        control.setContentCompressionResistancePriority(.init(450), for: .horizontal)
        segmented = control
        views.append(control)
        flexibleViews.append(control)
        keyViews.append(control)
    }

    private func buildText(_ placeholder: String) {
        let field = NativeTextControls.makeGeneralField(placeholder: placeholder)
        configureTextField(field, font: InspectorFonts.control)
        field.action = #selector(textCommitted(_:))
        textField = field
        views.append(field)
        flexibleViews.append(field)
        keyViews.append(field)
    }

    private func buildReadOnly() {
        let label = Self.makeLabel("", color: .labelColor)
        label.isSelectable = true
        label.lineBreakMode = .byTruncatingMiddle
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        label.setContentHuggingPriority(.init(200), for: .horizontal)
        label.setAccessibilityLabel(field.label)
        label.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        valueLabel = label
        views.append(label)
        flexibleViews.append(label)
    }

    private func buildAction(title: String, symbolName: String?) {
        // 편의 생성자(sizeToFit 호출)를 피하고 기본 생성 후 구성합니다.
        let button = NSButton()
        if title.isEmpty, let symbolName, let image = InspectorSymbols.image(symbolName, label: field.label) {
            button.title = ""
            button.image = image
            button.imagePosition = .imageOnly
        } else {
            button.title = title.isEmpty ? field.label : title
        }
        button.target = self
        button.action = #selector(actionPressed(_:))
        button.bezelStyle = .push
        button.controlSize = InspectorMetrics.controlSize
        button.font = InspectorFonts.control
        button.setAccessibilityLabel(field.label)
        button.toolTip = field.label
        button.identifier = NSUserInterfaceItemIdentifier(field.id.path)
        Self.hug(button)
        self.button = button
        views.append(button)
        keyViews.append(button)
    }

    /// switch·색 well은 혼합 상태를 그릴 수 없으므로 옆에 macOS 혼합 상태 표기(대시)를 둡니다.
    /// 화면 폭을 아끼려 글자 대신 대시를 쓰고, 툴팁과 컨트롤 접근성 값은 "Mixed"입니다.
    private func appendMixedLabel() {
        let label = Self.makeLabel(Self.mixedGlyph, color: .secondaryLabelColor)
        label.toolTip = Self.mixedText
        label.setAccessibilityElement(false)
        label.isHidden = true
        mixedLabel = label
        views.append(label)
    }

    private func configureTextField(_ field: NSTextField, font: NSFont) {
        field.controlSize = InspectorMetrics.controlSize
        field.font = font
        field.target = self
        field.delegate = self
        // 포커스를 잃을 때도 확정합니다(Return과 같은 경로).
        field.cell?.sendsActionOnEndEditing = true
        field.setAccessibilityLabel(self.field.label)
        field.toolTip = self.field.label
        field.identifier = NSUserInterfaceItemIdentifier(self.field.id.path)
        field.setContentHuggingPriority(.init(200), for: .horizontal)
        field.setContentCompressionResistancePriority(.init(400), for: .horizontal)
        // 최소 폭은 행 clipping 저항(490)보다 약간 낮게 둬, 아주 좁을 때는 잘리기 전에 필드가 먼저 줄게 합니다.
        let minimum = field.widthAnchor.constraint(greaterThanOrEqualToConstant: InspectorMetrics.minimumFieldWidth)
        minimum.priority = .init(480)
        minimum.isActive = true
    }

    // MARK: - 표시

    /// 현재 `field`를 컨트롤에 반영합니다. 편집 중인 텍스트는 덮어쓰지 않습니다.
    private func refresh() {
        let interactive = isInteractive
        let isMixed = field.value == .mixed

        if let textField {
            textField.isEnabled = interactive
            if textField.currentEditor() == nil {
                textField.stringValue = displayText()
            }
            textField.placeholderString = isMixed ? Self.mixedText : (field.value == .none ? Self.noValueText : nil)
            if case .text(let placeholder) = field.control, !isMixed, field.value != .none {
                textField.placeholderString = placeholder
            }
        }
        if let stepper, case .number(let spec) = field.control {
            stepper.isEnabled = interactive && field.value != .none
            if case .number(let value) = field.value {
                stepper.minValue = spec.minimum ?? -Double(Int32.max)
                stepper.maxValue = spec.maximum ?? Double(Int32.max)
                stepper.doubleValue = value
            } else {
                // 혼합 값: 기준 0에서 한 단계 이동량만 읽고 상대 조정으로 보냅니다.
                stepper.minValue = -Double(Int32.max)
                stepper.maxValue = Double(Int32.max)
                stepper.doubleValue = 0
            }
        }
        if let toggle {
            toggle.isEnabled = interactive
            if case .bool(let on) = field.value {
                toggle.state = on ? .on : .off
            } else {
                toggle.state = .off
            }
            toggle.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        if let colorWell {
            colorWell.isEnabled = interactive
            if case .color(let color) = field.value {
                colorWell.color = color.nsColor
            }
            colorWell.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        mixedLabel?.isHidden = !isMixed
        if let popup {
            popup.isEnabled = interactive
            refreshPopupSelection(popup, isMixed: isMixed)
        }
        if let segmented, case .segmented(let choices) = field.control {
            segmented.isEnabled = interactive
            if case .choice(let id) = field.value, let index = choices.firstIndex(where: { $0.id == id }) {
                segmented.selectedSegment = index
            } else {
                segmented.selectedSegment = -1
            }
            segmented.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        if let valueLabel {
            let text = displayText()
            valueLabel.stringValue = text
            valueLabel.toolTip = text
            valueLabel.setAccessibilityValue(text)
        }
        button?.isEnabled = interactive
    }

    private func refreshPopupSelection(_ popup: NSPopUpButton, isMixed: Bool) {
        let mixedItemIndex = popup.itemArray.firstIndex { $0.representedObject == nil }
        if isMixed {
            if mixedItemIndex == nil {
                let item = NSMenuItem(title: Self.mixedText, action: nil, keyEquivalent: "")
                item.isEnabled = false
                popup.menu?.insertItem(item, at: 0)
            }
            popup.selectItem(at: 0)
            return
        }
        if let mixedItemIndex {
            popup.removeItem(at: mixedItemIndex)
        }
        if case .choice(let id) = field.value,
           let index = popup.itemArray.firstIndex(where: { ($0.representedObject as? String) == id }) {
            popup.selectItem(at: index)
        } else {
            popup.select(nil)
        }
    }

    /// 읽기 전용·텍스트·수치 필드에 보일 문자열.
    private func displayText() -> String {
        switch field.value {
        case .none: return field.control == .readOnly ? Self.noValueText : ""
        case .mixed: return field.control == .readOnly ? Self.mixedText : ""
        case .number(let value):
            if case .number(let spec) = field.control {
                return numberFormatter.string(from: NSNumber(value: value * spec.displayScale)) ?? ""
            }
            return Self.plainNumber(value)
        case .bool(let on): return on ? "On" : "Off"
        case .color(let color): return color.hexString
        case .choice(let id): return id
        case .text(let text): return text
        }
    }

    // MARK: - 편집 → 명령

    @objc private func numberCommitted(_ sender: NSTextField) {
        guard case .number(let spec) = field.control, isInteractive else { return }
        let text = sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else {
            refresh()
            return
        }
        guard let number = numberFormatter.number(from: text)?.doubleValue ?? Double(text) else {
            NSSound.beep()
            refresh()
            return
        }
        let model = spec.clamped(number / spec.displayScale)
        if case .number(let current) = field.value, Self.same(current, model, spec: spec) {
            refresh()
            return
        }
        field.value = .number(model)
        refresh()
        send?(.setValue(field.id, .number(model), phase: .commit))
    }

    @objc private func stepperChanged(_ sender: NSStepper) {
        guard case .number(let spec) = field.control, isInteractive else { return }
        switch field.value {
        case .number:
            let model = spec.clamped(sender.doubleValue)
            field.value = .number(model)
            refresh()
            send?(.setValue(field.id, .number(model), phase: .continuous))
        case .mixed:
            let delta = sender.doubleValue
            sender.doubleValue = 0
            if delta != 0 {
                send?(.adjustNumber(field.id, delta: delta, phase: .continuous))
            }
        default:
            break
        }
    }

    @objc private func toggleChanged(_ sender: NSSwitch) {
        guard isInteractive else { return }
        let on = sender.state == .on
        field.value = .bool(on)
        refresh()
        send?(.setValue(field.id, .bool(on), phase: .commit))
    }

    @objc private func colorChanged(_ sender: NSColorWell) {
        guard isInteractive, let color = InspectorColor(sender.color) else { return }
        field.value = .color(color)
        mixedLabel?.isHidden = true
        // 색 패널 드래그는 연속 입력이므로 코어가 같은 속성의 연속 변경을 한 Undo로 묶습니다.
        send?(.setValue(field.id, .color(color), phase: .continuous))
    }

    @objc private func popupChanged(_ sender: NSPopUpButton) {
        guard isInteractive, let id = sender.selectedItem?.representedObject as? String else { return }
        field.value = .choice(id)
        refresh()
        send?(.setValue(field.id, .choice(id), phase: .commit))
    }

    @objc private func segmentChanged(_ sender: NSSegmentedControl) {
        guard isInteractive, case .segmented(let choices) = field.control,
              choices.indices.contains(sender.selectedSegment) else { return }
        let id = choices[sender.selectedSegment].id
        field.value = .choice(id)
        refresh()
        send?(.setValue(field.id, .choice(id), phase: .commit))
    }

    @objc private func textCommitted(_ sender: NSTextField) {
        guard isInteractive else { return }
        let text = sender.stringValue
        switch field.value {
        case .text(let current) where current == text:
            return
        case .mixed where text.isEmpty:
            return
        default:
            break
        }
        field.value = .text(text)
        send?(.setValue(field.id, .text(text), phase: .commit))
    }

    @objc private func actionPressed(_ sender: NSButton) {
        guard isInteractive else { return }
        send?(.perform(field.id))
    }

    // MARK: - NSTextFieldDelegate

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.cancelOperation(_:)):
            // Escape: 편집 전 값으로 되돌리고 포커스는 유지합니다.
            textView.string = displayText()
            textView.selectAll(nil)
            return true
        case #selector(NSResponder.moveUp(_:)):
            return nudge(by: 1, textView: textView)
        case #selector(NSResponder.moveDown(_:)):
            return nudge(by: -1, textView: textView)
        case #selector(NSResponder.moveUpAndModifySelection(_:)):
            return nudge(by: 10, textView: textView)
        case #selector(NSResponder.moveDownAndModifySelection(_:)):
            return nudge(by: -10, textView: textView)
        default:
            return false
        }
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        // 편집 중 건너뛴 snapshot 값을 다시 반영합니다.
        refresh()
    }

    /// 화살표 키 조정: 한 단계(Shift는 10단계). 혼합 값은 상대 조정 명령으로 보냅니다.
    private func nudge(by steps: Double, textView: NSTextView) -> Bool {
        guard case .number(let spec) = field.control else { return false }
        guard isInteractive else { return true }
        let delta = spec.step * steps
        let typed = numberFormatter.number(from: textView.string)?.doubleValue
        switch field.value {
        case .number(let current):
            let base = typed.map { $0 / spec.displayScale } ?? current
            let model = spec.clamped(base + delta)
            field.value = .number(model)
            textView.string = displayText()
            textView.selectAll(nil)
            stepper?.doubleValue = model
            send?(.setValue(field.id, .number(model), phase: .continuous))
        case .mixed:
            send?(.adjustNumber(field.id, delta: delta, phase: .continuous))
        default:
            break
        }
        return true
    }

    // MARK: - 도우미

    static func makeLabel(_ text: String, color: NSColor) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = InspectorFonts.control
        label.textColor = color
        label.lineBreakMode = .byTruncatingTail
        hug(label)
        return label
    }

    static func hug(_ view: NSView) {
        view.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        view.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
    }

    private static func plainNumber(_ value: Double) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 2
        formatter.usesGroupingSeparator = false
        return formatter.string(from: NSNumber(value: value)) ?? ""
    }

    private static func same(_ lhs: Double, _ rhs: Double, spec: InspectorNumberSpec) -> Bool {
        let tolerance = pow(10, -Double(spec.fractionDigits + 1)) / max(spec.displayScale, 1)
        return abs(lhs - rhs) < tolerance
    }
}

// MARK: - 문서 색 ↔ NSColor

extension InspectorColor {
    /// 문서 색을 표시용 NSColor로 바꿉니다(색공간 유지).
    var nsColor: NSColor {
        switch space {
        case .srgb:
            return NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha)
        case .displayP3:
            return NSColor(displayP3Red: red, green: green, blue: blue, alpha: alpha)
        }
    }

    /// 색 패널에서 고른 색을 문서 색으로 바꿉니다. Display P3 색은 P3로, 나머지는 sRGB로 옮깁니다.
    /// 패턴처럼 성분으로 바꿀 수 없는 색이면 `nil`입니다.
    init?(_ color: NSColor) {
        if color.type == .componentBased, color.colorSpace == .displayP3,
           let p3 = color.usingColorSpace(.displayP3) {
            self.init(space: .displayP3, red: Double(p3.redComponent), green: Double(p3.greenComponent),
                      blue: Double(p3.blueComponent), alpha: Double(p3.alphaComponent))
            return
        }
        guard let srgb = color.usingColorSpace(.sRGB) else { return nil }
        self.init(space: .srgb, red: Double(srgb.redComponent), green: Double(srgb.greenComponent),
                  blue: Double(srgb.blueComponent), alpha: Double(srgb.alphaComponent))
    }

    /// 표시용 16진수 문자열(알파가 1 미만이면 % 표기 추가).
    var hexString: String {
        func byte(_ value: Double) -> Int { Int((min(max(value, 0), 1) * 255).rounded()) }
        var text = String(format: "#%02X%02X%02X", byte(red), byte(green), byte(blue))
        if alpha < 1 {
            text += " \(Int((alpha * 100).rounded()))%"
        }
        if space == .displayP3 {
            text += " P3"
        }
        return text
    }
}
