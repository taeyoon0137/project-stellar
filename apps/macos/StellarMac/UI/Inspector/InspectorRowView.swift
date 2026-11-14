import AppKit

/// 인스펙터 한 줄: 레이블 열 + 필드 컨트롤 + 진단 표시 + Value Binding 표시(placeholder).
///
/// frame으로 배치합니다(Auto Layout 제약 없음, `InspectorLayout`). 배치 규칙은 이전 stack 배치와 같습니다:
/// 레이블 열 88pt → 6pt → 필드(필드 안 4pt, 필드 사이 6pt) → 진단·binding 표시. 유연한 컨트롤(입력 필드,
/// popup, segmented, 읽기 전용 값)이 남는 폭을 같은 폭으로 나누고, 없으면 진단·binding 표시를 오른쪽 끝에 둡니다.
/// 필드는 나눠 만들 수 있습니다(페이지 예열). 먼저 모든 필드 컨트롤을 만들고 재 두고(`prepareNextField()`, 창 밖),
/// 그다음 하나씩 붙입니다(`attachNextField()`). 붙일 때는 모든 필드 크기를 알고 있으므로 이미 붙인 컨트롤을
/// 다시 배치·그리지 않습니다.
///
/// 접근성 트리에서는 행 하나를 group 요소로 묶고, 행 레이블을 group 레이블로 씁니다(가이드 8.3).
final class InspectorRowView: InspectorFlowView {
    private(set) var row: InspectorRow
    /// 읽기 전용 강제 비활성 상태.
    private(set) var forceDisabled: Bool
    /// 행에 붙인 필드 controller(화면 순서).
    private(set) var fieldControllers: [InspectorFieldController] = []
    /// 만들고 재 두었지만 아직 붙이지 않은 필드 controller.
    private var preparedControllers: [InspectorFieldController] = []
    private let label: NSTextField
    private var labelHeight: CGFloat = 0
    /// 진단 표시. 진단이 처음 생길 때 만들고(작은 이미지 뷰 하나), 없어지면 숨깁니다. 진단 유무는 행 구성에
    /// 넣지 않습니다(진단이 생겨도 행을 다시 만들지 않음). 진단이 없는 행에는 뷰를 두지 않아 창의 뷰 수를 줄입니다.
    private var issueView: NSImageView?
    private var issueSize = NSSize.zero
    private var bindingButton: NSButton?
    private var bindingSize = NSSize.zero
    private var bindingSymbol: String?
    private var bindingDescription: String?
    private let send: (InspectorCommand) -> Void

    /// `buildsFieldsNow`가 false이면 레이블·표시만 만들고 필드는 `prepareNextField()`·`attachNextField()`로
    /// 나눠 만듭니다.
    init(row: InspectorRow, forceDisabled: Bool, send: @escaping (InspectorCommand) -> Void,
         buildsFieldsNow: Bool = true) {
        self.row = row
        self.forceDisabled = forceDisabled
        self.send = send
        label = InspectorFieldController.makeLabel(row.label, color: .secondaryLabelColor)
        super.init(frame: .zero)
        buildShell()
        if buildsFieldsNow {
            while !isComplete { attachNextField() }
        }
    }

    /// 뷰 구성을 정하는 값. 같으면 제자리에서 갱신할 수 있습니다(행 id·레이블·값은 포함하지 않음).
    static func compositionKey(of row: InspectorRow) -> String {
        row.fields.map(InspectorFieldController.compositionKey(of:)).joined(separator: ",")
            + (hasBinding(row) ? "|b" : "")
    }

    /// 같은 뷰 구성으로 새 행을 반영할 수 있는지.
    func canUpdate(to newRow: InspectorRow) -> Bool {
        isComplete && Self.compositionKey(of: newRow) == Self.compositionKey(of: row)
    }

    /// 값을 제자리에서 갱신합니다. 먼저 `canUpdate(to:)`로 확인합니다.
    func update(_ newRow: InspectorRow, forceDisabled: Bool) {
        let issueChanged = (newRow.issue == nil) != (row.issue == nil)
        row = newRow
        self.forceDisabled = forceDisabled
        if label.stringValue != newRow.label { label.stringValue = newRow.label }
        for (controller, field) in zip(fieldControllers, newRow.fields) {
            controller.update(field, forceDisabled: forceDisabled)
        }
        refreshDecorations()
        if issueChanged { invalidateFlow() }
    }

    /// 화면이 이미 이 행을 그대로 보이고 있는지. 편집 직후 필드가 낙관적으로 바꿔 둔 값이 있으면 false라서,
    /// 코어가 같은 값을 다시 보내도(명령 거부·다른 선택에 적용) 표시가 snapshot 값으로 돌아옵니다.
    func isShowing(_ newRow: InspectorRow, forceDisabled: Bool) -> Bool {
        isComplete && row == newRow && self.forceDisabled == forceDisabled
            && zip(fieldControllers, newRow.fields).allSatisfy { $0.field == $1 }
    }

    /// 모든 필드를 붙였는지.
    var isComplete: Bool { fieldControllers.count == row.fields.count }

    /// 붙인 것과 붙일 것을 합친 필드 controller(배치 폭 계산용).
    private var allControllers: [InspectorFieldController] { fieldControllers + preparedControllers }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] {
        fieldControllers.flatMap(\.keyViews) + (bindingButton.map { [$0] } ?? [])
    }

    /// 예열 페이지에서 필드마다 같은 종류의 견본 값으로 한 번 바꿉니다(대기 페이지라 보이지 않음). 첫 값 변경 비용을
    /// 예열 slice에서 치르기 위함이며, 실제 snapshot을 반영하면 그 값으로 다시 바뀝니다.
    func warmUpValues() {
        guard isComplete else { return }
        var sample = row
        sample.fields = row.fields.map { field in
            var changed = field
            changed.value = Self.sampleValue(for: field)
            return changed
        }
        update(sample, forceDisabled: forceDisabled)
    }

    private static func sampleValue(for field: InspectorField) -> InspectorValue {
        switch field.control {
        case .number(let spec): return .number((spec.minimum ?? 0) + spec.step)
        case .toggle: return .bool(field.value != .bool(true))
        case .color: return .color(InspectorColor(space: .srgb, red: 0.5, green: 0.5, blue: 0.5, alpha: 1))
        case .popup(let choices), .segmented(let choices):
            let other = choices.first { field.value != .choice($0.id) }
            return other.map { .choice($0.id) } ?? field.value
        case .text, .readOnly: return .text("Sample")
        case .action: return field.value
        }
    }

    // MARK: - 구성

    private func buildShell() {
        label.setAccessibilityElement(false)
        addSubview(label)
        labelHeight = InspectorLayout.measure(label).height

        if Self.hasBinding(row) {
            let button = NSButton()
            button.title = ""
            button.target = self
            button.action = #selector(bindingPressed(_:))
            button.isBordered = false
            button.imagePosition = .imageOnly
            button.controlSize = InspectorMetrics.controlSize
            bindingButton = button
            addSubview(button)
        }

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        refreshDecorations()
        bindingSize = bindingButton.map(InspectorLayout.measure) ?? .zero
    }

    /// 다음 필드 컨트롤을 만들고 크기를 재 둡니다(창 밖, 붙이지 않음). 나눠 만들기 단계 하나입니다.
    func prepareNextField() {
        let index = fieldControllers.count + preparedControllers.count
        guard index < row.fields.count else { return }
        let controller = InspectorFieldController(field: row.fields[index], forceDisabled: forceDisabled)
        controller.send = send
        controller.onLayoutChange = { [weak self] in self?.invalidateFlow() }
        controller.onItemAdded = { [weak self, weak controller] view in
            guard let self, let controller, self.fieldControllers.contains(where: { $0 === controller }) else { return }
            self.addSubview(view)
            self.invalidateFlow()
        }
        preparedControllers.append(controller)
    }

    /// 재 둔 다음 필드를 행에 붙입니다(없으면 먼저 만듭니다). 나눠 만들기 단계 하나입니다.
    func attachNextField() {
        guard !isComplete else { return }
        if preparedControllers.isEmpty { prepareNextField() }
        let controller = preparedControllers.removeFirst()
        fieldControllers.append(controller)
        for item in controller.items {
            addSubview(item.view)
        }
        invalidateFlow()
    }

    // MARK: - 배치

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        var height = labelHeight
        for controller in allControllers {
            for item in controller.items where !item.view.isHidden {
                height = max(height, item.size.height)
            }
        }
        if let issueView, !issueView.isHidden { height = max(height, issueSize.height) }
        if bindingButton != nil { height = max(height, bindingSize.height) }
        return ceil(height)
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        let lineHeight = bounds.height

        // 필드 배치 단위(숨긴 것 제외)와 각 단위 뒤 간격. 아직 붙이지 않은 필드도 폭 계산에 넣습니다.
        var fieldItems: [(item: InspectorRowItem, spacing: CGFloat)] = []
        for controller in allControllers {
            let visible = controller.items.filter { !$0.view.isHidden }
            for (index, item) in visible.enumerated() {
                let isLast = index == visible.count - 1
                fieldItems.append((item, isLast ? InspectorMetrics.rowSpacing : InspectorMetrics.groupSpacing))
            }
        }
        var trailing: [(view: NSView, size: NSSize)] = []
        if let issueView, !issueView.isHidden { trailing.append((issueView, issueSize)) }
        if let bindingButton { trailing.append((bindingButton, bindingSize)) }
        let trailingWidth = trailing.reduce(0) { $0 + $1.size.width }
            + CGFloat(max(trailing.count - 1, 0)) * InspectorMetrics.groupSpacing

        var fixed: CGFloat = 0
        var flexibleCount = 0
        for (index, entry) in fieldItems.enumerated() {
            if entry.item.isFlexible { flexibleCount += 1 } else { fixed += entry.item.size.width }
            // 마지막 필드 뒤 간격은 뒤에 진단·binding 표시가 있을 때만 씁니다.
            if index < fieldItems.count - 1 || !trailing.isEmpty { fixed += entry.spacing }
        }
        // 좁은 폭: 유연한 컨트롤이 최소 폭(36pt)을 갖도록 먼저 레이블 열을 줄이고(최소 40pt, 이전 배치의 레이블
        // 압축 저항이 가장 낮던 것과 같은 순서), 그래도 모자라면 유연한 컨트롤이 줄어듭니다.
        let fieldsWidth = fixed + trailingWidth + CGFloat(flexibleCount) * InspectorMetrics.minimumFieldWidth
        let roomForLabel = width - InspectorMetrics.rowSpacing - fieldsWidth
        let labelWidth = max(min(InspectorMetrics.labelColumnWidth, roomForLabel, width), min(40, width))
        InspectorLayout.place(label, x: 0, width: labelWidth,
                              size: NSSize(width: labelWidth, height: labelHeight), lineY: 0, lineHeight: lineHeight)
        var x = labelWidth + InspectorMetrics.rowSpacing
        let flexibleWidth = flexibleCount > 0
            ? max(0, (width - x - fixed - trailingWidth) / CGFloat(flexibleCount)) : 0
        for entry in fieldItems {
            let itemWidth = entry.item.isFlexible ? flexibleWidth : entry.item.size.width
            if entry.item.view.superview === self {
                InspectorLayout.place(entry.item.view, x: x, width: itemWidth, size: entry.item.size,
                                      lineY: 0, lineHeight: lineHeight)
            }
            x += itemWidth + entry.spacing
        }
        // 유연한 컨트롤이 없으면 진단·binding 표시를 오른쪽 끝에 붙입니다(이전 spacer와 같음).
        var trailingX = flexibleCount > 0 ? x : max(x, width - trailingWidth)
        for entry in trailing {
            InspectorLayout.place(entry.view, x: trailingX, width: entry.size.width, size: entry.size,
                                  lineY: 0, lineHeight: lineHeight)
            trailingX += entry.size.width + InspectorMetrics.groupSpacing
        }
    }

    /// 행 레이블·진단·binding 표시를 갱신합니다.
    private func refreshDecorations() {
        let groupLabel = row.label.isEmpty ? (row.fields.first?.label ?? "") : row.label
        setAccessibilityLabel(groupLabel)

        if let issue = row.issue {
            let view = issueView ?? makeIssueView()
            if view.isHidden { view.isHidden = false }
            view.toolTip = issue
            view.setAccessibilityLabel("Issue: \(issue)")
        } else if let issueView, !issueView.isHidden {
            issueView.isHidden = true
        }
        if let bindingButton, let binding = Self.aggregateBinding(row) {
            let symbol = InspectorSymbols.binding(binding)
            let description = "Value Binding for \(groupLabel)"
            if bindingSymbol != symbol || bindingDescription != description {
                bindingButton.image = InspectorSymbols.image(symbol, label: description)?
                    .withSymbolConfiguration(NSImage.SymbolConfiguration(textStyle: .body, scale: .small))
                bindingSymbol = symbol
                bindingDescription = description
            }
            let tint: NSColor
            switch binding {
            case .fixed: tint = .tertiaryLabelColor
            case .orphaned: tint = .systemOrange
            default: tint = .controlAccentColor
            }
            if bindingButton.contentTintColor != tint { bindingButton.contentTintColor = tint }
            let summary = row.fields.compactMap { field in
                field.binding.map { "\(field.label): \($0.summary)" }
            }.joined(separator: "; ")
            bindingButton.toolTip = summary
            bindingButton.setAccessibilityLabel(description)
            bindingButton.setAccessibilityValueDescription(summary)
        }
    }

    private func makeIssueView() -> NSImageView {
        let view = NSImageView()
        view.image = InspectorSymbols.image(InspectorSymbols.issue, label: "Issue")
        view.contentTintColor = .systemOrange
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.image)
        addSubview(view)
        issueSize = InspectorLayout.measure(view)
        issueView = view
        return view
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
final class InspectorRowsView: InspectorFlowView {
    private(set) var rowViews: [InspectorRowView] = []
    private var noteLabel: InspectorWrappingLabel?
    private let send: (InspectorCommand) -> Void

    init(send: @escaping (InspectorCommand) -> Void) {
        self.send = send
        super.init(frame: .zero)
    }

    /// 뷰 구성을 정하는 값(행 구성 목록 + 보조 설명 유무).
    static func compositionKey(rows: [InspectorRow], note: String?) -> String {
        rows.map(InspectorRowView.compositionKey(of:)).joined(separator: ";") + (note == nil ? "" : "|note")
    }

    /// 나눠 만들기 단계: 행마다 "행 틀"(레이블·표시) 단계, 필드마다 "만들고 재기" 단계(비싼 컨트롤은 `heavy`),
    /// 필드마다 "붙이기" 단계, 마지막으로 보조 설명 단계. 빈 상태에서만 씁니다.
    ///
    /// `warmsUpValues`가 true이면(예열) 행마다 견본 값으로 한 번 바꾸는 단계를 더합니다. macOS 26 컨트롤은 만든 뒤
    /// 처음 값이 바뀔 때 크기를 다시 재는 비용이 커서(popup·segmented), 이 비용을 예열 slice에서 미리 치르면
    /// 처음 실제 선택을 보일 때 비용이 약 절반으로 줍니다(명세 8.3절 측정).
    func buildSteps(rows: [InspectorRow], note: String?, forceDisabled: Bool,
                    warmsUpValues: Bool = false) -> [InspectorBuildStep] {
        var steps: [InspectorBuildStep] = []
        for row in rows {
            var rowView: InspectorRowView?
            steps.append(.light { [weak self] in
                guard let self else { return }
                let view = InspectorRowView(row: row, forceDisabled: forceDisabled, send: self.send,
                                            buildsFieldsNow: false)
                self.rowViews.append(view)
                self.addSubview(view)
                rowView = view
                self.invalidateFlow()
            })
            for field in row.fields {
                let prepare: () -> Void = { rowView?.prepareNextField() }
                steps.append(InspectorFieldController.isExpensiveToBuild(field) ? .heavy(prepare) : .light(prepare))
            }
            for _ in row.fields {
                steps.append(.light { rowView?.attachNextField() })
            }
            if warmsUpValues {
                let warm: () -> Void = { rowView?.warmUpValues() }
                let heavy = row.fields.contains { field in
                    switch field.control {
                    case .popup, .segmented: return true
                    default: return false
                    }
                }
                steps.append(heavy ? .heavy(warm) : .light(warm))
            }
        }
        if let note {
            steps.append(.light { [weak self] in self?.applyNote(note) })
        }
        return steps
    }

    /// 행을 반영합니다. 구성이 같은 행은 제자리 갱신하고, 다르면 그 행만 새로 만듭니다(동기).
    func apply(rows: [InspectorRow], note: String?, forceDisabled: Bool) {
        if rows.count == rowViews.count {
            for (index, row) in rows.enumerated() {
                let existing = rowViews[index]
                if existing.isShowing(row, forceDisabled: forceDisabled) {
                    continue // 바뀌지 않은 행은 건드리지 않습니다.
                }
                if existing.canUpdate(to: row) {
                    existing.update(row, forceDisabled: forceDisabled)
                } else {
                    // 사라질 컨트롤의 연속 조작은 끝을 알린 뒤 버립니다.
                    existing.fieldControllers.forEach { $0.endGesture() }
                    let replacement = InspectorRowView(row: row, forceDisabled: forceDisabled, send: send)
                    addSubview(replacement)
                    existing.removeFromSuperview()
                    rowViews[index] = replacement
                    invalidateFlow()
                }
            }
        } else {
            endGestures()
            rowViews.forEach { $0.removeFromSuperview() }
            rowViews = rows.map { row in
                let view = InspectorRowView(row: row, forceDisabled: forceDisabled, send: send)
                addSubview(view)
                return view
            }
            invalidateFlow()
        }
        applyNote(note)
    }

    /// 키보드 포커스 순서.
    var keyViews: [NSView] { rowViews.flatMap(\.keyViews) }

    /// 진행 중인 연속 조작을 끝냅니다.
    func endGestures() {
        rowViews.forEach { $0.fieldControllers.forEach { $0.endGesture() } }
    }

    private func applyNote(_ note: String?) {
        guard let note else {
            if noteLabel != nil {
                noteLabel?.removeFromSuperview()
                noteLabel = nil
                invalidateFlow()
            }
            return
        }
        if let noteLabel {
            if noteLabel.stringValue != note {
                noteLabel.stringValue = note
                invalidateFlow()
            }
            return
        }
        let label = InspectorWrappingLabel(text: note)
        addSubview(label)
        noteLabel = label
        invalidateFlow()
    }

    // MARK: - 배치

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        var heights = rowViews.map { $0.flowHeight(forWidth: width) }
        if let noteLabel { heights.append(noteLabel.flowHeight(forWidth: width)) }
        guard !heights.isEmpty else { return 0 }
        return heights.reduce(0, +) + CGFloat(heights.count - 1) * InspectorMetrics.rowSpacing
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        var y: CGFloat = 0
        for rowView in rowViews {
            let height = rowView.flowHeight(forWidth: width)
            let frame = NSRect(x: 0, y: y, width: width, height: height)
            if rowView.frame != frame { rowView.frame = frame }
            y += height + InspectorMetrics.rowSpacing
        }
        if let noteLabel {
            let frame = NSRect(x: 0, y: y, width: width, height: noteLabel.flowHeight(forWidth: width))
            if noteLabel.frame != frame { noteLabel.frame = frame }
        }
    }
}
