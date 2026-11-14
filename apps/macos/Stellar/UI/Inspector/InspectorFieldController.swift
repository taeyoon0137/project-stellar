import AppKit

/// 행 안 배치 단위 하나: 뷰, 남는 폭을 나눠 가질지, 만들 때 잰 정렬 사각형 크기.
struct InspectorRowItem {
    let view: NSView
    /// 남는 폭을 다른 유연한 뷰와 같은 폭으로 나눠 가지는지(입력 필드, popup, segmented, 읽기 전용 값).
    let isFlexible: Bool
    /// 정렬 사각형 기준 크기. 유연한 뷰는 높이만 씁니다.
    var size: NSSize
}

/// 필드 하나(`InspectorField`)를 네이티브 컨트롤로 표시하고 편집을 명령으로 바꿉니다.
///
/// 뷰를 직접 소유하지 않고 행(`InspectorRowView`)에 넣을 배치 단위를 만듭니다. 컨트롤 종류는 모두
/// AppKit 표준 컨트롤입니다(가이드 5.1, 5.2). 값의 의미·검증은 코어가 맡고, 여기서는 표시 단위
/// 변환(`displayScale`)과 범위 clamp만 합니다. 컨트롤 크기는 만들 때 한 번만 잽니다(배치는 행이 frame으로 함).
@MainActor
final class InspectorFieldController: NSObject, NSTextFieldDelegate {
    private(set) var field: InspectorField
    /// 행에 순서대로 넣을 배치 단위.
    private(set) var items: [InspectorRowItem] = []
    /// 키보드 포커스 순서에 들어갈 컨트롤.
    private(set) var keyViews: [NSView] = []
    /// 편집 명령 전달.
    var send: ((InspectorCommand) -> Void)?
    /// 배치 단위의 크기·표시가 바뀌어 행을 다시 배치해야 할 때 호출됩니다.
    var onLayoutChange: (() -> Void)?
    /// 구성 뒤 새 배치 단위(혼합 대시)를 만들었을 때 호출됩니다. 행이 뷰를 붙입니다.
    var onItemAdded: ((NSView) -> Void)?

    private var forceDisabled: Bool
    private var textField: NSTextField?
    private var unitLabel: NSTextField?
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
        measureItems()
    }

    deinit {
        // 화살표 키 조정 중에 행이 사라져도 monitor가 남지 않게 합니다(종료 명령은 보낼 곳이 없음).
        if let keyUpMonitor { NSEvent.removeMonitor(keyUpMonitor) }
    }

    /// 뷰 구성을 정하는 값(컨트롤 종류, 짧은 레이블, 단위·stepper 유무, 명령 버튼 내용).
    /// 같으면 컨트롤을 다시 만들지 않고 속성 id·레이블·값·선택지·수치 규격을 제자리에서 바꿀 수 있습니다.
    static func compositionKey(of field: InspectorField) -> String {
        var key: String
        switch field.control {
        case .number(let spec): key = "n" + (spec.unit == .none ? "" : "u") + (spec.showsStepper ? "s" : "")
        case .toggle: key = "t"
        case .color: key = "c"
        case .popup: key = "p"
        case .segmented: key = "g"
        case .text: key = "x"
        case .readOnly: key = "r"
        case .action(let title, let symbolName, let destructive):
            key = "a(\(title)|\(symbolName ?? "")|\(destructive ? "d" : ""))"
        }
        if let short = field.shortLabel { key += "[\(short)]" }
        return key
    }

    /// 처음 만들고 재는 데 0.3ms 이상 드는 컨트롤(popup, segmented, stepper, switch, push 버튼)을 쓰는지.
    /// 나눠 만들 때 이런 필드는 slice 하나를 혼자 씁니다(`InspectorBuildStep.Cost.heavy`).
    static func isExpensiveToBuild(_ field: InspectorField) -> Bool {
        switch field.control {
        case .popup, .segmented, .toggle, .action: return true
        case .number(let spec): return spec.showsStepper
        case .color, .text, .readOnly: return false
        }
    }

    /// 같은 컨트롤 구성으로 새 필드를 반영할 수 있는지.
    func canUpdate(to newField: InspectorField) -> Bool {
        Self.compositionKey(of: newField) == Self.compositionKey(of: field)
    }

    /// 값·편집 가능 상태·속성 id·선택지를 제자리에서 갱신합니다. 먼저 `canUpdate(to:)`로 확인합니다.
    func update(_ newField: InspectorField, forceDisabled: Bool) {
        let previous = field
        if previous.id != newField.id {
            // 다른 속성으로 바뀌기 전에 이전 속성의 편집·연속 조작을 이전 id로 끝냅니다.
            commitPendingEdit()
            endGesture()
        }
        field = newField
        self.forceDisabled = forceDisabled
        if previous.id != newField.id || previous.label != newField.label {
            retarget()
        }
        if previous.control != newField.control {
            reconfigure()
        }
        refresh()
        if previous.id != newField.id {
            showValueInEditor()
        }
    }

    /// 편집 중이면 편집 텍스트를 현재 값으로 바꾸고 전체 선택합니다(포커스는 유지).
    func showValueInEditor() {
        guard let editor = textField?.currentEditor() as? NSTextView else { return }
        editor.string = displayText()
        editor.selectAll(nil)
    }

    /// 사용자가 편집할 수 있는 상태인지.
    private var isInteractive: Bool { field.isEditable && !forceDisabled }

    // MARK: - 구성

    private func build() {
        if let short = field.shortLabel {
            let label = Self.makeLabel(short, color: .secondaryLabelColor)
            label.setAccessibilityElement(false)
            append(label, flexible: false)
        }
        switch field.control {
        case .number(let spec): buildNumber(spec)
        case .toggle: buildToggle()
        case .color: buildColor()
        case .popup(let choices): buildPopup(choices)
        case .segmented(let choices): buildSegmented(choices)
        case .text(let placeholder): buildText(placeholder)
        case .readOnly: buildReadOnly()
        case .action(let title, let symbolName, let destructive):
            buildAction(title: title, symbolName: symbolName, destructive: destructive)
        }
        retarget()
    }

    private func append(_ view: NSView, flexible: Bool, focusable: Bool = false) {
        items.append(InspectorRowItem(view: view, isFlexible: flexible, size: .zero))
        if focusable { keyViews.append(view) }
    }

    /// 정렬 크기를 한 번 잽니다(macOS 26 popup·segmented는 첫 측정이 비쌈). 이후에는 저장한 값만 씁니다.
    private func measureItems() {
        for index in items.indices {
            items[index].size = InspectorLayout.measure(items[index].view)
        }
    }

    private func buildNumber(_ spec: InspectorNumberSpec) {
        numberFormatter.numberStyle = .decimal
        numberFormatter.usesGroupingSeparator = false
        numberFormatter.minimumFractionDigits = 0
        // 표시 반올림을 `InspectorNumberSpec.modelValue(fromDisplay:)`와 같게 둡니다(0.5는 0에서 먼 쪽).
        numberFormatter.roundingMode = .halfUp
        numberFormatter.isLenient = true
        configureFormatter(spec)

        let field = NativeTextControls.makeGeneralField(placeholder: "")
        configureTextField(field, font: InspectorFonts.number)
        field.alignment = .right
        field.action = #selector(numberCommitted(_:))
        textField = field
        append(field, flexible: true, focusable: true)

        if spec.unit != .none {
            let unit = Self.makeLabel(spec.unit.rawValue, color: .secondaryLabelColor)
            unit.setAccessibilityElement(false)
            unitLabel = unit
            append(unit, flexible: false)
        }
        if spec.showsStepper {
            let stepper = NSStepper()
            stepper.controlSize = InspectorMetrics.controlSize
            stepper.autorepeat = true
            stepper.valueWraps = false
            stepper.increment = spec.step
            stepper.target = self
            stepper.action = #selector(stepperChanged(_:))
            self.stepper = stepper
            append(stepper, flexible: false, focusable: true)
        }
    }

    private func configureFormatter(_ spec: InspectorNumberSpec) {
        numberFormatter.maximumFractionDigits = spec.fractionDigits
    }

    private func buildToggle() {
        let toggle = NSSwitch()
        toggle.controlSize = InspectorMetrics.controlSize
        toggle.target = self
        toggle.action = #selector(toggleChanged(_:))
        self.toggle = toggle
        append(toggle, flexible: false, focusable: true)
    }

    private func buildColor() {
        let well = NSColorWell(style: .minimal)
        well.controlSize = InspectorMetrics.controlSize
        well.supportsAlpha = true
        well.target = self
        well.action = #selector(colorChanged(_:))
        colorWell = well
        append(well, flexible: false, focusable: true)
    }

    private func buildPopup(_ choices: [InspectorChoice]) {
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        popup.controlSize = InspectorMetrics.controlSize
        popup.font = InspectorFonts.control
        popup.autoenablesItems = false
        popup.target = self
        popup.action = #selector(popupChanged(_:))
        self.popup = popup
        setPopupChoices(choices)
        append(popup, flexible: true, focusable: true)
    }

    private func setPopupChoices(_ choices: [InspectorChoice]) {
        guard let popup else { return }
        popup.removeAllItems()
        for choice in choices {
            let item = NSMenuItem(title: choice.title, action: nil, keyEquivalent: "")
            item.representedObject = choice.id
            if let symbol = choice.symbolName {
                item.image = InspectorSymbols.image(symbol, label: choice.title)
            }
            popup.menu?.addItem(item)
        }
    }

    private func buildSegmented(_ choices: [InspectorChoice]) {
        let control = NSSegmentedControl()
        control.trackingMode = .selectOne
        control.controlSize = InspectorMetrics.controlSize
        control.font = InspectorFonts.control
        control.segmentDistribution = .fillEqually
        control.target = self
        control.action = #selector(segmentChanged(_:))
        segmented = control
        setSegmentChoices(choices)
        append(control, flexible: true, focusable: true)
    }

    private func setSegmentChoices(_ choices: [InspectorChoice]) {
        guard let control = segmented else { return }
        control.segmentCount = choices.count
        for (index, choice) in choices.enumerated() {
            if let symbol = choice.symbolName, let image = InspectorSymbols.image(symbol, label: choice.title) {
                control.setImage(image, forSegment: index)
                control.setLabel("", forSegment: index)
            } else {
                control.setImage(nil, forSegment: index)
                control.setLabel(choice.title, forSegment: index)
            }
            control.setToolTip(choice.title, forSegment: index)
        }
    }

    private func buildText(_ placeholder: String) {
        let field = NativeTextControls.makeGeneralField(placeholder: placeholder)
        configureTextField(field, font: InspectorFonts.control)
        field.action = #selector(textCommitted(_:))
        textField = field
        append(field, flexible: true, focusable: true)
    }

    private func buildReadOnly() {
        let label = Self.makeLabel("", color: .labelColor)
        label.isSelectable = true
        label.lineBreakMode = .byTruncatingMiddle
        valueLabel = label
        append(label, flexible: true)
    }

    private func buildAction(title: String, symbolName: String?, destructive: Bool) {
        // 편의 생성자(sizeToFit 호출)를 피하고 기본 생성 후 구성합니다.
        let button = NSButton()
        // 파괴 변환은 시스템 파괴 동작 표시를 쓰고, 기본(Return) 버튼으로 만들지 않습니다(가이드 5.1).
        button.hasDestructiveAction = destructive
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
        self.button = button
        append(button, flexible: false, focusable: true)
    }

    /// switch·색 well은 혼합 상태를 그릴 수 없으므로 옆에 macOS 혼합 상태 표기(대시)를 둡니다.
    /// 화면 폭을 아끼려 글자 대신 대시를 쓰고, 툴팁과 컨트롤 접근성 값은 "Mixed"입니다.
    /// 처음 혼합 값을 보일 때 만듭니다(혼합이 아닌 필드에 숨은 뷰를 두지 않음). 행이 새 배치 단위를 붙입니다.
    private func makeMixedLabel() -> NSTextField {
        let label = Self.makeLabel(Self.mixedGlyph, color: .secondaryLabelColor)
        label.toolTip = Self.mixedText
        label.setAccessibilityElement(false)
        mixedLabel = label
        let item = InspectorRowItem(view: label, isFlexible: false, size: InspectorLayout.measure(label))
        // 컨트롤 바로 뒤(이전 배치와 같은 자리)에 둡니다.
        let index = items.firstIndex { $0.view === toggle || $0.view === colorWell }.map { $0 + 1 } ?? items.count
        items.insert(item, at: index)
        onItemAdded?(label)
        return label
    }

    private func configureTextField(_ field: NSTextField, font: NSFont) {
        field.controlSize = InspectorMetrics.controlSize
        field.font = font
        field.target = self
        field.delegate = self
        // 포커스를 잃을 때도 확정합니다(Return과 같은 경로).
        field.cell?.sendsActionOnEndEditing = true
    }

    /// 속성 id·이름에 따라 붙는 식별자·접근성 레이블·툴팁을 맞춥니다(구성 후, 다른 속성으로 바꿀 때).
    private func retarget() {
        let path = field.id.path
        let label = field.label
        let identifier = NSUserInterfaceItemIdentifier(path)
        for control in [textField, toggle, colorWell, popup, segmented, button, valueLabel] as [NSView?] {
            guard let control else { continue }
            control.identifier = identifier
            control.setAccessibilityLabel(label)
            if control !== valueLabel { control.toolTip = label }
        }
        if let stepper {
            stepper.identifier = NSUserInterfaceItemIdentifier(path + ".stepper")
            stepper.setAccessibilityLabel(label)
        }
        if let button, case .action(let title, _, _) = field.control, title.isEmpty, button.image == nil {
            button.title = label
        }
        if case .number(let spec) = field.control {
            textField?.setAccessibilityHelp(spec.unit == .none ? nil : "Value in \(spec.unit.spokenName)")
        }
    }

    /// 같은 구성 안에서 바뀐 컨트롤 세부(수치 규격, popup·segmented 선택지)를 반영합니다.
    private func reconfigure() {
        switch field.control {
        case .number(let spec):
            configureFormatter(spec)
            stepper?.increment = spec.step
            textField?.setAccessibilityHelp(spec.unit == .none ? nil : "Value in \(spec.unit.spokenName)")
            if let unitLabel, unitLabel.stringValue != spec.unit.rawValue {
                unitLabel.stringValue = spec.unit.rawValue
                if let index = items.firstIndex(where: { $0.view === unitLabel }) {
                    items[index].size = InspectorLayout.measure(unitLabel)
                }
                onLayoutChange?()
            }
        case .popup(let choices):
            setPopupChoices(choices)
        case .segmented(let choices):
            setSegmentChoices(choices)
        default:
            break
        }
    }

    // MARK: - 표시

    /// 현재 `field`를 컨트롤에 반영합니다. 편집 중인 텍스트는 덮어쓰지 않고, 같은 값은 다시 넣지 않습니다
    /// (바뀐 컨트롤만 다시 그리게).
    private func refresh() {
        let interactive = isInteractive
        let isMixed = field.value == .mixed

        if let textField {
            Self.setEnabled(textField, interactive)
            if textField.currentEditor() == nil {
                let text = displayText()
                if textField.stringValue != text { textField.stringValue = text }
            }
            var placeholder = isMixed ? Self.mixedText : (field.value == .none ? Self.noValueText : nil)
            if case .text(let textPlaceholder) = field.control, !isMixed, field.value != .none {
                placeholder = textPlaceholder
            }
            if textField.placeholderString != placeholder { textField.placeholderString = placeholder }
        }
        if let stepper, case .number(let spec) = field.control {
            Self.setEnabled(stepper, interactive && field.value != .none)
            if case .number(let value) = field.value {
                stepper.minValue = spec.minimum ?? -Double(Int32.max)
                stepper.maxValue = spec.maximum ?? Double(Int32.max)
                if stepper.doubleValue != value { stepper.doubleValue = value }
            } else {
                // 혼합 값: 기준 0에서 한 단계 이동량만 읽고 상대 조정으로 보냅니다.
                stepper.minValue = -Double(Int32.max)
                stepper.maxValue = Double(Int32.max)
                if stepper.doubleValue != 0 { stepper.doubleValue = 0 }
            }
        }
        if let toggle {
            Self.setEnabled(toggle, interactive)
            var state = NSControl.StateValue.off
            if case .bool(true) = field.value { state = .on }
            if toggle.state != state { toggle.state = state }
            toggle.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        if let colorWell {
            Self.setEnabled(colorWell, interactive)
            if case .color(let color) = field.value, InspectorColor(colorWell.color) != color {
                colorWell.color = color.nsColor
            }
            colorWell.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        if toggle != nil || colorWell != nil {
            if isMixed, mixedLabel == nil {
                _ = makeMixedLabel()
                onLayoutChange?()
            } else if let mixedLabel, mixedLabel.isHidden == isMixed {
                mixedLabel.isHidden = !isMixed
                onLayoutChange?()
            }
        }
        if let popup {
            Self.setEnabled(popup, interactive)
            refreshPopupSelection(popup, isMixed: isMixed)
        }
        if let segmented, case .segmented(let choices) = field.control {
            Self.setEnabled(segmented, interactive)
            var selected = -1
            if case .choice(let id) = field.value, let index = choices.firstIndex(where: { $0.id == id }) {
                selected = index
            }
            if segmented.selectedSegment != selected { segmented.selectedSegment = selected }
            segmented.setAccessibilityValueDescription(isMixed ? Self.mixedText : nil)
        }
        if let valueLabel {
            let text = displayText()
            if valueLabel.stringValue != text {
                valueLabel.stringValue = text
                valueLabel.toolTip = text
                valueLabel.setAccessibilityValue(text)
            }
        }
        if let button { Self.setEnabled(button, interactive) }
    }

    private static func setEnabled(_ control: NSControl, _ enabled: Bool) {
        if control.isEnabled != enabled { control.isEnabled = enabled }
    }

    private func refreshPopupSelection(_ popup: NSPopUpButton, isMixed: Bool) {
        let mixedItemIndex = popup.itemArray.firstIndex { $0.representedObject == nil }
        if isMixed {
            if mixedItemIndex == nil {
                let item = NSMenuItem(title: Self.mixedText, action: nil, keyEquivalent: "")
                item.isEnabled = false
                popup.menu?.insertItem(item, at: 0)
            }
            if popup.indexOfSelectedItem != 0 { popup.selectItem(at: 0) }
            return
        }
        if let mixedItemIndex {
            popup.removeItem(at: mixedItemIndex)
        }
        if case .choice(let id) = field.value,
           let index = popup.itemArray.firstIndex(where: { ($0.representedObject as? String) == id }) {
            if popup.indexOfSelectedItem != index { popup.selectItem(at: index) }
        } else if popup.selectedItem != nil {
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

    /// 편집 중인 텍스트를 지금 값으로 확정합니다(속성이 바뀌기 전). 편집 중이 아니면 아무것도 하지 않습니다.
    func commitPendingEdit() {
        guard let textField, let editor = textField.currentEditor() else { return }
        let text = editor.string
        switch field.control {
        case .number: commitNumber(text: text)
        case .text: commitText(text)
        default: break
        }
    }

    // MARK: - 편집 → 명령

    @objc private func numberCommitted(_ sender: NSTextField) {
        commitNumber(text: sender.stringValue)
    }

    /// 수치 입력 확정(Return·포커스 이동). 화살표 키 조정 중이면 이 값으로 조정을 끝냅니다.
    private func commitNumber(text rawText: String) {
        guard case .number(let spec) = field.control, isInteractive else {
            endGesture()
            return
        }
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else {
            endGesture()
            refresh()
            return
        }
        let scanner = Scanner(string: text)
        scanner.locale = numberFormatter.locale
        let localized = scanner.scanDouble()
        guard let number = (scanner.isAtEnd ? localized : nil) ?? Double(text), number.isFinite else {
            NSSound.beep()
            endGesture()
            refresh()
            return
        }
        // NumberFormatter 해석은 반올림하지 않으므로(예: 자릿수 0에서 "6.7" → 6.7) 표시 자릿수로 직접 맞춥니다.
        let model = spec.modelValue(fromDisplay: number)
        guard model.isFinite else {
            endGesture()
            refresh()
            return
        }
        if gesture == .set {
            // 화살표 키 조정 뒤 확정: 확정 값을 연속 조작의 최종 값으로 보내 한 Undo 단위로 끝냅니다.
            // 값이 그대로여도 `.ended`를 보냅니다.
            field.value = .number(model)
            refresh()
            endGesture()
            return
        }
        // 혼합 값 상대 조정 중에 값을 입력했다면 상대 조정을 먼저 끝내고, 입력값은 별도 확정으로 보냅니다.
        endGesture()
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
            let model = spec.normalized(sender.doubleValue)
            field.value = .number(model)
            refresh()
            beginGesture(.set, endsWith: .pointerRelease)
            send?(.setValue(field.id, .number(model), phase: .continuous))
        case .mixed:
            let delta = sender.doubleValue
            sender.doubleValue = 0
            if delta != 0 {
                beginGesture(.adjust, endsWith: .pointerRelease)
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
        if let mixedLabel, !mixedLabel.isHidden {
            mixedLabel.isHidden = true
            onLayoutChange?()
        }
        // 색 패널 드래그는 연속 조작입니다. 마우스를 놓으면(또는 클릭 한 번이면 바로) `.ended`로 끝냅니다.
        beginGesture(.set, endsWith: .pointerRelease)
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
        commitText(sender.stringValue)
    }

    private func commitText(_ text: String) {
        guard isInteractive else { return }
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

    // MARK: - 연속 조작(아키텍처 §23.3)

    /// 연속 조작 종류: 값 설정(`setValue`) 또는 혼합 값 상대 조정(`adjustNumber`).
    enum Gesture: Equatable {
        case set
        case adjust
    }

    /// 연속 조작이 끝나는 조건.
    enum GestureEnd {
        /// 마우스 추적(stepper 누름·자동 반복, 색 패널 드래그)이 끝나고 왼쪽 버튼이 놓였을 때.
        /// 키보드·VoiceOver로 한 단계 바꾼 경우에는 다음 런 루프에서 바로 끝납니다.
        case pointerRelease
        /// 수치 필드 화살표 키를 놓았을 때, 또는 편집이 끝났을 때(Return·포커스 이동).
        case arrowKeyRelease
    }

    /// 진행 중인 연속 조작. `nil`이면 없음.
    private(set) var gesture: Gesture?
    private var gestureEndScheduled = false
    private var keyUpMonitor: Any?

    /// 연속 조작을 시작하거나 이어 갑니다. 종류가 바뀌면 이전 조작을 먼저 끝냅니다.
    private func beginGesture(_ kind: Gesture, endsWith end: GestureEnd) {
        if let gesture, gesture != kind { endGesture() }
        gesture = kind
        switch end {
        case .pointerRelease:
            scheduleGestureEndCheck(after: 0)
        case .arrowKeyRelease:
            installKeyUpMonitor()
        }
    }

    /// 마우스 추적 루프는 이벤트 추적 모드에서 돌므로 기본 모드 블록은 추적이 끝난 뒤 실행됩니다.
    /// 그때도 왼쪽 버튼이 눌려 있으면(추적 루프 밖 드래그) 한 프레임 뒤 다시 확인합니다. 시간 제한으로 끝을
    /// 추측하지 않고 버튼 상태로 판단합니다.
    private func scheduleGestureEndCheck(after delay: TimeInterval) {
        guard !gestureEndScheduled else { return }
        gestureEndScheduled = true
        let check: @Sendable () -> Void = { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.gestureEndScheduled = false
                guard self.gesture != nil, self.keyUpMonitor == nil else { return }
                if NSEvent.pressedMouseButtons & 1 != 0 {
                    self.scheduleGestureEndCheck(after: 1.0 / 60)
                } else {
                    self.endGesture()
                }
            }
        }
        if delay > 0 {
            let timer = Timer(timeInterval: delay, repeats: false) { _ in check() }
            RunLoop.main.add(timer, forMode: .default)
        } else {
            RunLoop.main.perform(inModes: [.default], block: check)
        }
    }

    /// 화살표 키를 놓으면 조정을 끝냅니다. 텍스트 편집은 중첩 추적 루프가 아니므로 key up이
    /// `sendEvent`를 거쳐 로컬 monitor에 들어옵니다.
    private func installKeyUpMonitor() {
        guard keyUpMonitor == nil else { return }
        keyUpMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyUp) { [weak self] event in
            if event.specialKey == .upArrow || event.specialKey == .downArrow {
                MainActor.assumeIsolated { self?.endGesture() }
            }
            return event
        }
    }

    /// 진행 중인 연속 조작을 끝내고 최종 값과 함께 `.ended`를 보냅니다. 값이 더 바뀌지 않았어도 보냅니다.
    /// 선택 변경 전에도 불러, 아직 바뀌지 않은 세대(이전 선택)로 끝을 알립니다.
    func endGesture() {
        if let keyUpMonitor {
            NSEvent.removeMonitor(keyUpMonitor)
            self.keyUpMonitor = nil
        }
        guard let gesture else { return }
        self.gesture = nil
        switch gesture {
        case .set:
            switch field.value {
            case .number, .color:
                send?(.setValue(field.id, field.value, phase: .ended))
            default:
                break
            }
        case .adjust:
            send?(.adjustNumber(field.id, delta: 0, phase: .ended))
        }
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
        // AppKit은 이 통지를 확정 action보다 먼저 보냅니다. 화살표 키 조정 중이면 입력한 값으로 지금 확정해
        // 조정을 끝냅니다(뒤따르는 action은 같은 값이라 명령을 다시 보내지 않음).
        if gesture != nil, let textField {
            let editorText = (notification.userInfo?["NSFieldEditor"] as? NSText)?.string ?? textField.stringValue
            commitNumber(text: editorText)
        }
        endGesture()
        // 편집 중 건너뛴 snapshot 값을 다시 반영합니다.
        refresh()
    }

    /// 화살표 키 조정: 한 단계(Shift는 10단계). 혼합 값은 상대 조정 명령으로 보냅니다.
    /// 키를 누르고 있는 동안(자동 반복 포함)이 연속 조작 하나이고, 키를 놓거나 편집이 끝나면 `.ended`를 보냅니다.
    private func nudge(by steps: Double, textView: NSTextView) -> Bool {
        guard case .number(let spec) = field.control else { return false }
        guard isInteractive else { return true }
        let delta = spec.step * steps
        let typed = numberFormatter.number(from: textView.string)?.doubleValue
        switch field.value {
        case .number(let current):
            let base = typed.map { spec.modelValue(fromDisplay: $0) } ?? current
            let model = spec.normalized(base + delta)
            field.value = .number(model)
            textView.string = displayText()
            textView.selectAll(nil)
            stepper?.doubleValue = model
            beginGesture(.set, endsWith: .arrowKeyRelease)
            send?(.setValue(field.id, .number(model), phase: .continuous))
        case .mixed:
            beginGesture(.adjust, endsWith: .arrowKeyRelease)
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
        return label
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
        if color.type == .componentBased,
           color.colorSpace == .displayP3 || color.colorSpace.cgColorSpace?.name == CGColorSpace.extendedDisplayP3 {
            self.init(space: .displayP3, red: Double(color.redComponent), green: Double(color.greenComponent),
                      blue: Double(color.blueComponent), alpha: Double(color.alphaComponent))
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
