import AppKit

/// 캔버스 하단 가운데에 떠 있는 도구 막대.
///
/// 상단 툴바에 있던 도구 그룹(`NSToolbarItemGroup`, selectOne, 펼친 표현)을 위치만 옮긴 것입니다.
/// 같은 모양을 유지하려고 툴바가 그리는 구조와 치수를 그대로 씁니다: Liquid Glass 캡슐
/// (`NSGlassEffectView`, 높이 36pt, 모서리 반경 18pt) 안에 도구 칸 38.5pt, 선택한 도구 뒤에
/// 32.5×28pt 둥근 회색 선택 표시, 구분선 없음. 일반 `NSSegmentedControl`은 툴바 밖에서
/// 구분선과 강조색 선택 표시를 그려 모양이 달라지므로 쓰지 않습니다.
///
/// 도구 그룹(선택 · 컨테이너 · 도형 · 내용 · 시점) 사이는 구분선 없이 간격으로만 나눕니다(UX 명세 5.1).
final class ShellToolBarView: NSView {
    /// 툴바 도구 그룹과 같은 치수.
    enum Metrics {
        static let height: CGFloat = 36
        static let cellWidth: CGFloat = 38.5
        static let selectionSize = NSSize(width: 32.5, height: 28)
        /// 도구 그룹 사이 간격.
        static let groupSpacing: CGFloat = 10
        /// 캔버스 아래 가장자리와의 간격.
        static let bottomInset: CGFloat = 16
    }

    /// 사용자가 도구를 고르면 그 index로 부릅니다.
    var selectionHandler: ((Int) -> Void)?

    private let glass = NSGlassEffectView()
    private let stack = NSStackView()
    private(set) var buttons: [ShellToolButton] = []

    /// 선택한 도구 index. 선택이 없으면 -1.
    var selectedIndex: Int = -1 {
        didSet {
            for (index, button) in buttons.enumerated() {
                button.isSelectedTool = index == selectedIndex
            }
        }
    }

    /// 막대 전체의 활성 상태(프로젝트가 준비되지 않으면 끕니다).
    var isEnabled: Bool = true {
        didSet {
            buttons.forEach { $0.isEnabled = isEnabled }
        }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        glass.cornerRadius = Metrics.height / 2
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)

        stack.orientation = .horizontal
        stack.spacing = 0
        stack.distribution = .fill
        stack.translatesAutoresizingMaskIntoConstraints = false
        glass.contentView = stack

        NSLayoutConstraint.activate([
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.heightAnchor.constraint(equalToConstant: Metrics.height),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.toolbar)
        setAccessibilityLabel(ShellFrameStrings.tools)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellToolBarView는 코드로만 생성합니다")
    }

    /// 도구 목록으로 버튼을 다시 만듭니다.
    func setTools(_ tools: [ShellToolDescriptor]) {
        buttons.forEach { $0.removeFromSuperview() }
        buttons = tools.enumerated().map { index, tool in
            let button = ShellToolButton(tool: tool)
            button.tag = index
            button.target = self
            button.action = #selector(toolPressed(_:))
            button.isEnabled = isEnabled
            if tool.startsGroup, let previous = stack.arrangedSubviews.last {
                stack.setCustomSpacing(Metrics.groupSpacing, after: previous)
            }
            stack.addArrangedSubview(button)
            NSLayoutConstraint.activate([
                button.widthAnchor.constraint(equalToConstant: Metrics.cellWidth),
                button.heightAnchor.constraint(equalToConstant: Metrics.height),
            ])
            return button
        }
        let current = selectedIndex
        selectedIndex = current
    }

    @objc private func toolPressed(_ sender: ShellToolButton) {
        selectionHandler?(sender.tag)
    }
}

/// 도구 막대의 도구 한 칸. 툴바 도구 그룹 항목처럼 테두리 없는 아이콘과, 선택 시 뒤에 그리는
/// 둥근 회색 선택 표시만 있습니다.
final class ShellToolButton: NSButton {
    let tool: ShellToolDescriptor

    var isSelectedTool = false {
        didSet {
            guard isSelectedTool != oldValue else { return }
            needsDisplay = true
            setAccessibilityValue(isSelectedTool ? 1 : 0)
        }
    }

    init(tool: ShellToolDescriptor) {
        self.tool = tool
        super.init(frame: .zero)
        isBordered = false
        imagePosition = .imageOnly
        imageScaling = .scaleNone
        image = NSImage(systemSymbolName: tool.symbolName, accessibilityDescription: tool.label)?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(scale: .large))
        contentTintColor = .labelColor
        toolTip = tool.helpTag ?? tool.label
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityLabel(tool.label)
        setAccessibilityRole(.radioButton)
        setAccessibilityValue(0)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellToolButton은 코드로만 생성합니다")
    }

    override func draw(_ dirtyRect: NSRect) {
        if isSelectedTool {
            let size = ShellToolBarView.Metrics.selectionSize
            let rect = NSRect(
                x: (bounds.width - size.width) / 2,
                y: (bounds.height - size.height) / 2,
                width: size.width,
                height: size.height
            )
            NSColor.labelColor.withAlphaComponent(0.1).setFill()
            NSBezierPath(roundedRect: rect, xRadius: size.height / 2, yRadius: size.height / 2).fill()
        }
        super.draw(dirtyRect)
    }
}
