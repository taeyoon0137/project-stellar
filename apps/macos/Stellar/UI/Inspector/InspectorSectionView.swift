import AppKit

/// 섹션 하나: 접기 머리와 행 목록. 접힘 상태는 노드 종류별로 기억합니다.
@MainActor
final class InspectorSectionView: NSView {
    let kind: InspectorSectionKind
    /// 접힘이 바뀌면 부릅니다.
    var onToggle: ((InspectorSectionKind, Bool) -> Void)?

    private let disclosure = NSButton()
    private let rows = NSStackView()
    private(set) var isCollapsed: Bool

    init(kind: InspectorSectionKind, collapsed: Bool) {
        self.kind = kind
        isCollapsed = collapsed
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        disclosure.bezelStyle = .disclosure
        disclosure.setButtonType(.pushOnPushOff)
        disclosure.title = ""
        disclosure.target = self
        disclosure.action = #selector(toggle)
        let title = NSButton(title: InspectorPropertyCatalog.title(kind), target: self, action: #selector(toggleFromTitle))
        title.isBordered = false
        title.font = InspectorFonts.sectionTitle
        title.contentTintColor = .labelColor
        let head = NSStackView(views: [disclosure, title])
        head.spacing = 2
        head.setAccessibilityElement(false)
        rows.orientation = .vertical
        rows.alignment = .leading
        rows.spacing = InspectorMetrics.rowSpacing
        let column = NSStackView(views: [head, rows])
        column.orientation = .vertical
        column.alignment = .leading
        column.spacing = InspectorMetrics.rowSpacing
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: leadingAnchor),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor),
            rows.widthAnchor.constraint(equalTo: column.widthAnchor),
        ])
        disclosure.setAccessibilityLabel(InspectorPropertyCatalog.title(kind))
        applyCollapse()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("코드로만 생성합니다") }

    func add(_ row: NSView) {
        rows.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
    }

    /// 접힘을 바꿉니다(텍스트 편집 시작 때 Typography 펼침 등). 기억도 갱신합니다.
    func setCollapsed(_ collapsed: Bool) {
        guard collapsed != isCollapsed else { return }
        isCollapsed = collapsed
        applyCollapse()
        onToggle?(kind, collapsed)
    }

    private func applyCollapse() {
        disclosure.state = isCollapsed ? .off : .on
        rows.isHidden = isCollapsed
        disclosure.setAccessibilityValue(isCollapsed ? String(localized: "collapsed") : String(localized: "expanded"))
    }

    @objc private func toggle() {
        setCollapsed(disclosure.state == .off)
    }

    @objc private func toggleFromTitle() {
        setCollapsed(!isCollapsed)
    }
}
