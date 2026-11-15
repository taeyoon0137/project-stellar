import AppKit

/// 사이드바 셀 재사용 식별자.
enum SidebarCellIdentifier {
    static let header = NSUserInterfaceItemIdentifier("SidebarHeaderCell")
    static let item = NSUserInterfaceItemIdentifier("SidebarItemCell")
    static let layer = NSUserInterfaceItemIdentifier("SidebarLayerCell")
}

/// 사이드바 접근성 설정 보조(디자인 가이드 8.3).
///
/// AppKit 컨트롤이 접근성 서버에 내보내는 요소는 컨트롤 view가 아니라 그 cell(`NSButtonCell`, `NSImageCell`,
/// `NSSearchFieldCell` 등)입니다. view에만 넣은 값은 그 요소에 닿지 않습니다. 예를 들어 symbol 이미지만 있는 버튼은
/// view의 레이블 대신 이미지 기본 설명("show" 등)을 읽고, view만 접근성에서 뺀 `NSImageView`는 `AXImage`로 남습니다.
/// 그래서 view와 cell에 함께 설정합니다.
///
/// 목록 행의 레이블·custom action은 `NSTableCellView`에 둡니다. AppKit은 view-based 표의 각 행을 내부 `AXRow`
/// 요소로, 그 안의 셀 view를 `AXCell` 요소 하나로 내보내며, `AXCell`의 설명과 동작 목록을 셀 view의
/// `accessibilityLabel()`·`accessibilityCustomActions()`에서 가져옵니다(`SidebarUICheck`가 접근성 서버 경로로 확인).
/// 행 view(`NSTableRowView`)에 둔 레이블·custom action은 `AXRow`에 쓰이지 않습니다(macOS 27.2 실측).
@MainActor
enum SidebarAccessibility {
    /// 컨트롤의 접근성 레이블. 접근성 서버가 읽는 cell 요소에도 같은 값을 둡니다.
    static func setLabel(_ label: String, of control: NSControl) {
        control.setAccessibilityLabel(label)
        control.cell?.setAccessibilityLabel(label)
    }

    /// 장식용 컨트롤(아이콘)을 접근성 트리에서 뺍니다. 의미는 행·화면 레이블이 전달합니다.
    static func hideDecoration(_ control: NSControl) {
        control.setAccessibilityElement(false)
        control.cell?.setAccessibilityElement(false)
    }
}

/// 구획 제목 행(group row) 셀. "Pages", "Layers" 같은 source list 구획 제목을 보여 줍니다.
final class SidebarHeaderCellView: NSTableCellView {
    private let titleField = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = SidebarCellIdentifier.header
        titleField.translatesAutoresizingMaskIntoConstraints = false
        titleField.lineBreakMode = .byTruncatingTail
        titleField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        addSubview(titleField)
        textField = titleField
        NSLayoutConstraint.activate([
            titleField.leadingAnchor.constraint(equalTo: leadingAnchor),
            titleField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SidebarHeaderCellView는 코드로만 생성합니다")
    }

    func configure(title: String) {
        titleField.stringValue = title
        setAccessibilityLabel(title)
    }
}

/// 아이콘과 인라인 이름 필드를 가진 행 셀(페이지·View 행, 레이어 행의 기반).
///
/// 이름 필드는 AppKit 네이티브 `NSTextField`입니다(아키텍처 §11.2, 디자인 가이드 5.2).
/// 글꼴과 아이콘 크기는 `NSTableCellView`가 목록의 row size style(시스템 사이드바 크기 설정)에 맞춰 정하므로
/// 여기서 지정하지 않습니다(디자인 가이드 4.1, 4.4).
class SidebarNameCellView: NSTableCellView {
    let iconView = NSImageView()
    let nameField = NSTextField(string: "")

    /// 이 행이 가리키는 대상(페이지·View·노드)의 id.
    var itemID: SidebarStableID?

    init(identifier: NSUserInterfaceItemIdentifier, trailingAccessories: [NSView]) {
        super.init(frame: .zero)
        self.identifier = identifier

        iconView.translatesAutoresizingMaskIntoConstraints = false
        iconView.imageScaling = .scaleProportionallyDown
        iconView.setContentHuggingPriority(.required, for: .horizontal)
        iconView.setContentCompressionResistancePriority(.required, for: .horizontal)
        // 행 의미는 셀 접근성 레이블이 전달하므로 아이콘은 접근성 트리에서 뺍니다(디자인 가이드 8.3).
        SidebarAccessibility.hideDecoration(iconView)
        iconView.image = SidebarSymbols.image(SidebarSymbols.frame)

        nameField.translatesAutoresizingMaskIntoConstraints = false
        nameField.isEditable = true
        nameField.isSelectable = true
        nameField.isBordered = false
        nameField.isBezeled = false
        nameField.drawsBackground = false
        nameField.usesSingleLineMode = true
        nameField.allowsEditingTextAttributes = false
        nameField.importsGraphics = false
        // 편집하지 않을 때는 끝을 말줄임으로 자릅니다. scrollable로 두면 말줄임 대신 잘림(clipping)이 됩니다.
        nameField.lineBreakMode = .byTruncatingTail
        nameField.cell?.wraps = false
        nameField.cell?.truncatesLastVisibleLine = true
        nameField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        nameField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        addSubview(iconView)
        addSubview(nameField)
        imageView = iconView
        textField = nameField

        var constraints = [
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            nameField.leadingAnchor.constraint(equalToSystemSpacingAfter: iconView.trailingAnchor, multiplier: 0.5),
            nameField.centerYAnchor.constraint(equalTo: centerYAnchor),
        ]
        // 오른쪽 부속 컨트롤(레이어 행의 표시·잠금 토글)은 끝에서부터 시스템 간격으로 붙입니다.
        // 하위 뷰는 화면 순서(왼쪽→오른쪽)로 넣어 접근성 요소 순서도 같게 합니다.
        for accessory in trailingAccessories {
            accessory.translatesAutoresizingMaskIntoConstraints = false
            accessory.setContentHuggingPriority(.required, for: .horizontal)
            accessory.setContentCompressionResistancePriority(.required, for: .horizontal)
            addSubview(accessory)
        }
        var trailingAnchor = self.trailingAnchor
        for accessory in trailingAccessories.reversed() {
            constraints.append(trailingAnchor.constraint(equalToSystemSpacingAfter: accessory.trailingAnchor, multiplier: 0.5))
            constraints.append(accessory.centerYAnchor.constraint(equalTo: centerYAnchor))
            trailingAnchor = accessory.leadingAnchor
        }
        // 이름 필드가 남은 폭을 채우게 합니다(긴 이름은 끝을 말줄임, 토글 열 위치는 행마다 같음).
        constraints.append(trailingAnchor.constraint(equalToSystemSpacingAfter: nameField.trailingAnchor, multiplier: 0.5))
        NSLayoutConstraint.activate(constraints)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SidebarNameCellView는 코드로만 생성합니다")
    }

    /// 페이지·View 행을 채웁니다.
    func configure(id: SidebarStableID, name: String, placeholder: String, symbol: String, kindLabel: String) {
        itemID = id
        iconView.image = SidebarSymbols.image(symbol)
        nameField.stringValue = name
        nameField.placeholderString = placeholder
        let display = name.isEmpty ? placeholder : name
        // 행 레이블(AXCell 설명). 이름 필드는 값(이름)과 placeholder(종류)를 그대로 내보냅니다.
        setAccessibilityLabel("\(display), \(kindLabel)")
    }
}

/// 레이어 행 셀의 토글 버튼 이벤트를 받는 쪽.
@MainActor
protocol SidebarLayerCellDelegate: AnyObject {
    func layerCellDidToggleVisibility(_ cell: SidebarLayerCellView)
    func layerCellDidToggleLock(_ cell: SidebarLayerCellView)
}

/// 레이어 트리 행: 종류 아이콘, 인라인 이름, 표시·잠금 토글.
///
/// 토글은 마우스를 올린 행, 선택한 행, 그리고 기본값이 아닌 상태(숨김·잠김)에서 보입니다.
/// 대비 높이기가 켜져 있으면 항상 보입니다. 숨김·잠김은 색만이 아니라 아이콘 모양과 접근성 레이블로도
/// 알립니다(디자인 가이드 4.2 "선택 상태를 색에만 의존하지 않음").
final class SidebarLayerCellView: SidebarNameCellView {
    let visibilityButton: NSButton
    let lockButton: NSButton
    weak var delegate: SidebarLayerCellDelegate?

    private(set) var row: SidebarLayerRow?
    private var isHovered = false
    /// `true`이면 토글을 항상 보입니다(대비 높이기).
    var showsTogglesAlways = false {
        didSet { updateToggleVisibility() }
    }
    /// `false`이면 토글을 숨깁니다. 문서 편집으로 연결되지 않은 토글은 비활성 상태로 노출하지 않습니다.
    var togglesAvailable = true {
        didSet { updateToggleVisibility() }
    }
    private var hoverArea: NSTrackingArea?

    init() {
        let visibility = Self.makeToggleButton()
        let lock = Self.makeToggleButton()
        visibilityButton = visibility
        lockButton = lock
        super.init(identifier: SidebarCellIdentifier.layer, trailingAccessories: [visibility, lock])
        visibilityButton.target = self
        visibilityButton.action = #selector(toggleVisibility(_:))
        lockButton.target = self
        lockButton.action = #selector(toggleLock(_:))
        // 클릭 대상은 symbol 크기가 아니라 HIG 최소 컨트롤 크기 이상으로 둡니다. 이미지는 가운데에 그대로 그립니다
        // (디자인 가이드 4.4).
        let minimum = SidebarMetrics.minimumHitTarget
        NSLayoutConstraint.activate([visibility, lock].flatMap { button in
            [
                button.widthAnchor.constraint(greaterThanOrEqualToConstant: minimum),
                button.heightAnchor.constraint(greaterThanOrEqualToConstant: minimum),
            ]
        })
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SidebarLayerCellView는 코드로만 생성합니다")
    }

    private static func makeToggleButton() -> NSButton {
        let button = NSButton(title: "", target: nil, action: nil)
        button.isBordered = false
        button.imagePosition = .imageOnly
        button.setButtonType(.momentaryPushIn)
        // 행 안의 토글은 Tab 순서에 넣지 않습니다. 키보드는 단축키·문맥 메뉴, VoiceOver는 custom action으로 같은
        // 동작에 닿습니다(디자인 가이드 8.2, 8.3).
        button.refusesFirstResponder = true
        button.contentTintColor = .secondaryLabelColor
        return button
    }

    /// 레이어 행을 채웁니다.
    func configure(row: SidebarLayerRow) {
        self.row = row
        itemID = row.id
        let kindName = SidebarStrings.kindName(row.kind, shapeKind: row.shapeKind)
        let display = row.name.isEmpty ? kindName : row.name

        iconView.image = SidebarSymbols.image(SidebarSymbols.name(for: row.kind, shapeKind: row.shapeKind))
        nameField.stringValue = row.name
        nameField.placeholderString = kindName
        // 숨김 행과 필터 문맥(직접 일치하지 않은 조상) 행은 보조 label 계층으로 낮춥니다(디자인 가이드 4.1).
        switch (row.isVisible, row.isFilterMatch) {
        case (true, true): nameField.textColor = .labelColor
        case (false, false): nameField.textColor = .tertiaryLabelColor
        default: nameField.textColor = .secondaryLabelColor
        }

        // 레이블은 현재 상태가 아니라 누르면 일어날 동작을 말합니다(예: 보이는 행의 눈 아이콘 → "Hide Header").
        // 버튼 cell 레이블이 없으면 symbol 기본 설명(eye → "show", lock.fill → "lock")이 읽혀 동작과 반대가 되므로
        // cell과 symbol 이미지에 같은 레이블을 둡니다.
        let visibilityLabel = row.isVisible ? SidebarStrings.hideLayer(display) : SidebarStrings.showLayer(display)
        visibilityButton.image = SidebarSymbols.image(
            row.isVisible ? SidebarSymbols.visible : SidebarSymbols.hidden,
            label: visibilityLabel
        )
        SidebarAccessibility.setLabel(visibilityLabel, of: visibilityButton)
        visibilityButton.toolTip = row.isVisible ? SidebarStrings.hide : SidebarStrings.show

        let lockLabel = row.isLocked ? SidebarStrings.unlockLayer(display) : SidebarStrings.lockLayer(display)
        lockButton.image = SidebarSymbols.image(row.isLocked ? SidebarSymbols.locked : SidebarSymbols.unlocked, label: lockLabel)
        SidebarAccessibility.setLabel(lockLabel, of: lockButton)
        lockButton.toolTip = row.isLocked ? SidebarStrings.unlock : SidebarStrings.lock

        // 행 레이블(AXCell 설명). custom action은 controller가 같은 셀에 둡니다.
        var parts = [display, kindName]
        if !row.isVisible { parts.append(SidebarStrings.hiddenState) }
        if row.isLocked { parts.append(SidebarStrings.lockedState) }
        if !row.isFilterMatch { parts.append(SidebarStrings.contextOnly) }
        setAccessibilityLabel(parts.joined(separator: ", "))
        updateToggleVisibility()
    }

    @objc private func toggleVisibility(_ sender: Any?) {
        delegate?.layerCellDidToggleVisibility(self)
    }

    @objc private func toggleLock(_ sender: Any?) {
        delegate?.layerCellDidToggleLock(self)
    }

    // MARK: 토글 표시

    private var isRowSelected: Bool {
        (superview as? NSTableRowView)?.isSelected ?? false
    }

    private func updateToggleVisibility() {
        let reveal = showsTogglesAlways || isHovered || isRowSelected
        visibilityButton.isHidden = !togglesAvailable || !(reveal || !(row?.isVisible ?? true))
        lockButton.isHidden = !togglesAvailable || !(reveal || (row?.isLocked ?? false))
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { updateToggleVisibility() }
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        isHovered = false
        delegate = nil
        row = nil
        itemID = nil
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        hoverArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        updateToggleVisibility()
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
        updateToggleVisibility()
    }
}
