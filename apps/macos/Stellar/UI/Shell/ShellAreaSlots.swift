import AppKit

// 다른 UI 영역(사이드바, 인스펙터)을 셸 프레임 슬롯에 끼우는 어댑터.
//
// 각 영역의 공개 view controller를 그대로 쓰고, 셸 슬롯 protocol만 덧붙입니다. 영역 파일은 수정하지 않습니다.
// 프로젝트 창이 각 영역의 데이터를 연결합니다. 캔버스 아래 보조 패널은 두지 않습니다(UX 명세 3.2).

/// 왼쪽 사이드바 슬롯: `SidebarViewController`와 그 레이어 필터 accessory.
@MainActor
final class ShellSidebarAreaSlot: ShellSidebarSlot {
    let sidebar: SidebarViewController

    init(sidebar: SidebarViewController) {
        self.sidebar = sidebar
    }

    var paneViewController: NSViewController { sidebar }
    var paneTitle: String { String(localized: "Sidebar", comment: "Accessibility name and View menu name of the sidebar") }
    /// 사이드바가 만드는 split item(`makeSplitViewItem()`)과 같은 accessory를 셸이 만든 item에 붙입니다.
    var topAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { [sidebar.filterAccessory] }
}

/// 오른쪽 인스펙터 슬롯: `InspectorViewController`.
@MainActor
final class ShellInspectorAreaSlot: ShellInspectorSlot {
    let inspector: InspectorViewController
    private let tabs: ShellRightTabsViewController

    init(inspector: InspectorViewController) {
        self.inspector = inspector
        tabs = ShellRightTabsViewController(inspector: inspector)
    }

    var paneViewController: NSViewController { tabs }
    var paneTitle: String { String(localized: "Inspector", comment: "Accessibility name of the inspector") }

    /// 기능이 연결된 뒤에만 두 번째 탭을 만듭니다. 선택 상태는 각 controller가 그대로 소유합니다.
    func installCodeTab(_ controller: NSViewController, onSelect: @escaping () -> Void) {
        tabs.installCodeTab(controller, onSelect: onSelect)
    }

    var isCodeSelected: Bool { tabs.isCodeSelected }
}

/// 오른쪽 영역의 기능 탭. 속성 하나만 있을 때는 탭 줄 없이 기존 화면을 그대로 채웁니다.
@MainActor
private final class ShellRightTabsViewController: NSViewController {
    private let inspector: InspectorViewController
    private var code: NSViewController?
    private let selector = NSSegmentedControl()
    private let content = NSView()
    private var selectorHeight: NSLayoutConstraint?
    private var onCodeSelected: (() -> Void)?
    var isCodeSelected: Bool { code != nil && selector.selectedSegment == 1 }

    init(inspector: InspectorViewController) {
        self.inspector = inspector
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("ShellRightTabsViewController는 코드로만 생성합니다") }

    override func loadView() {
        let root = NSView()
        selector.translatesAutoresizingMaskIntoConstraints = false
        selector.segmentCount = 2
        selector.setLabel(String(localized: "Properties"), forSegment: 0)
        selector.setLabel(String(localized: "Code"), forSegment: 1)
        selector.selectedSegment = 0
        selector.target = self
        selector.action = #selector(selectTab(_:))
        selector.isHidden = true
        content.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(selector)
        root.addSubview(content)
        let height = selector.heightAnchor.constraint(equalToConstant: 0)
        selectorHeight = height
        NSLayoutConstraint.activate([
            selector.topAnchor.constraint(equalTo: root.topAnchor),
            selector.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 12),
            selector.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -12),
            height,
            content.topAnchor.constraint(equalTo: selector.bottomAnchor),
            content.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
        show(inspector)
    }

    func installCodeTab(_ controller: NSViewController, onSelect: @escaping () -> Void) {
        guard code == nil else { return }
        code = controller
        onCodeSelected = onSelect
        loadViewIfNeeded()
        selectorHeight?.constant = 28
        selector.isHidden = false
    }

    @objc private func selectTab(_ sender: NSSegmentedControl) {
        show(sender.selectedSegment == 1 ? (code ?? inspector) : inspector)
        if isCodeSelected { onCodeSelected?() }
    }

    private func show(_ controller: NSViewController) {
        for child in children where child !== controller {
            child.view.removeFromSuperview()
            child.removeFromParent()
        }
        guard controller.parent !== self else { return }
        addChild(controller)
        let panel = controller.view
        panel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(panel)
        NSLayoutConstraint.activate([
            panel.topAnchor.constraint(equalTo: content.topAnchor),
            panel.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            panel.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            panel.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])
    }
}
