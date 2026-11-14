import AppKit

/// 레이어 구획의 상태 화면: 로드 중, 실패, View 없음, 빈 목록, 필터 결과 없음(디자인 가이드 7.1).
///
/// 목록 위에 겹쳐 두며 배경을 칠하지 않습니다. 버튼을 뺀 영역의 클릭은 아래 목록으로 통과시킵니다.
/// 애니메이션은 시스템 진행 표시기 외에는 쓰지 않습니다(동작 줄이기, 디자인 가이드 8.4).
final class SidebarStateView: NSView {
    enum Content: Equatable {
        case hidden
        case loading
        case failed(message: String)
        case noView
        case empty
        case noResults(query: String)
    }

    /// 상태 버튼(다시 시도, 필터 지우기)을 누르면 호출됩니다.
    var onAction: ((Content) -> Void)?

    private(set) var content: Content = .hidden

    let spinner = NSProgressIndicator()
    let symbolView = NSImageView()
    let titleField = NSTextField(wrappingLabelWithString: "")
    let messageField = NSTextField(wrappingLabelWithString: "")
    let actionButton = NSButton(title: "", target: nil, action: nil)
    private let stack = NSStackView()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)

        spinner.style = .spinning
        spinner.isIndeterminate = true
        spinner.isDisplayedWhenStopped = false
        spinner.controlSize = .regular

        symbolView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title1)
        symbolView.contentTintColor = .secondaryLabelColor
        // 상태 의미는 group 레이블이 전달하므로 아이콘은 접근성 트리에서 뺍니다(cell까지, 디자인 가이드 8.3).
        SidebarAccessibility.hideDecoration(symbolView)

        titleField.font = NSFont.preferredFont(forTextStyle: .headline)
        titleField.textColor = .labelColor
        titleField.alignment = .center

        messageField.font = NSFont.preferredFont(forTextStyle: .subheadline)
        messageField.textColor = .secondaryLabelColor
        messageField.alignment = .center

        actionButton.target = self
        actionButton.action = #selector(performAction(_:))

        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.translatesAutoresizingMaskIntoConstraints = false
        for view in [spinner, symbolView, titleField, messageField, actionButton] as [NSView] {
            stack.addArrangedSubview(view)
        }
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalToSystemSpacingAfter: leadingAnchor, multiplier: 1),
            trailingAnchor.constraint(equalToSystemSpacingAfter: stack.trailingAnchor, multiplier: 1),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stack.topAnchor.constraint(greaterThanOrEqualTo: topAnchor),
            titleField.widthAnchor.constraint(equalTo: stack.widthAnchor),
            messageField.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        show(.hidden)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SidebarStateView는 코드로만 생성합니다")
    }

    override func layout() {
        super.layout()
        // 줄바꿈 레이블의 높이를 현재 폭으로 계산하게 합니다.
        let width = stack.frame.width
        for field in [titleField, messageField] where field.preferredMaxLayoutWidth != width {
            field.preferredMaxLayoutWidth = width
            needsLayout = true
        }
    }

    /// 버튼만 클릭을 받고 나머지는 아래 목록으로 통과시킵니다.
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        return hit.isDescendant(of: actionButton) ? hit : nil
    }

    func show(_ content: Content) {
        self.content = content
        var symbol: String?
        var title = ""
        var message = ""
        var action: String?
        var loading = false

        switch content {
        case .hidden:
            break
        case .loading:
            loading = true
            title = SidebarStrings.loading
        case .failed(let reason):
            symbol = SidebarSymbols.failed
            title = SidebarStrings.failedTitle
            message = reason
            action = SidebarStrings.retry
        case .noView:
            symbol = SidebarSymbols.noView
            title = SidebarStrings.noViewTitle
            message = SidebarStrings.noViewMessage
        case .empty:
            symbol = SidebarSymbols.emptyLayers
            title = SidebarStrings.emptyTitle
            message = SidebarStrings.emptyMessage
        case .noResults(let query):
            symbol = SidebarSymbols.noResults
            title = SidebarStrings.noResultsTitle
            message = SidebarStrings.noResultsMessage(query)
            action = SidebarStrings.clearFilter
        }

        isHidden = content == .hidden
        if loading { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        spinner.isHidden = !loading
        symbolView.image = symbol.flatMap { SidebarSymbols.image($0) }
        symbolView.isHidden = symbol == nil
        titleField.stringValue = title
        titleField.isHidden = title.isEmpty
        messageField.stringValue = message
        messageField.isHidden = message.isEmpty
        actionButton.title = action ?? ""
        actionButton.isHidden = action == nil
        setAccessibilityLabel(message.isEmpty ? title : "\(title). \(message)")
    }

    @objc private func performAction(_ sender: Any?) {
        onAction?(content)
    }
}
