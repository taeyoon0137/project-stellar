import AppKit

/// 메인 창 3열 분할: 사이드바 | 가운데(캔버스) | 인스펙터(가이드 2.3).
///
/// 사이드바는 `NSSplitViewItem(sidebarWithViewController:)`, 인스펙터는
/// `NSSplitViewItem(inspectorWithViewController:)`로 만들어 재질, 접기·펼치기, 폭 유지를 시스템에
/// 맡깁니다. pane 폭과 접힘 상태는 `NSSplitView.autosaveName`으로 유지합니다(가이드 2.6).
/// 캔버스 아래 보조 패널은 두지 않습니다(UX 명세 3.2).
final class ShellSplitViewController: NSSplitViewController {
    let sidebarItem: NSSplitViewItem
    let centerItem: NSSplitViewItem
    let inspectorItem: NSSplitViewItem
    /// 콘텐츠 슬롯과 상태 화면을 담은 controller.
    let contentContainer: ShellContentContainerViewController

    /// 동작 줄이기 설정 조회. 켜져 있으면 pane 전환을 애니메이션 없이 즉시 합니다(가이드 8.4, 8.5).
    /// 검사에서 결정적으로 실행하려고 바꿀 수 있게 둡니다.
    var prefersReducedMotion: () -> Bool = {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    init(slots: ShellFrameSlots, configuration: ShellWindowConfiguration) {
        contentContainer = ShellContentContainerViewController(contentSlot: slots.content)
        sidebarItem = NSSplitViewItem(sidebarWithViewController: slots.sidebar.paneViewController)
        centerItem = NSSplitViewItem(viewController: contentContainer)
        inspectorItem = NSSplitViewItem(inspectorWithViewController: slots.inspector.paneViewController)
        super.init(nibName: nil, bundle: nil)

        // view가 로드되기 전에 split view를 지정해야 autosave 이름이 item 추가 전에 적용됩니다.
        let split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.autosaveName = configuration.mainSplitAutosaveName
        splitView = split

        sidebarItem.minimumThickness = ShellFrameMetrics.sidebarThickness.lowerBound
        sidebarItem.maximumThickness = ShellFrameMetrics.sidebarThickness.upperBound
        inspectorItem.minimumThickness = ShellFrameMetrics.inspectorThickness.lowerBound
        inspectorItem.maximumThickness = ShellFrameMetrics.inspectorThickness.upperBound
        centerItem.minimumThickness = ShellFrameMetrics.contentMinimumSize.width
        // 캔버스가 남는 공간을 모두 차지합니다. 창 폭이 바뀌면 가운데 항목만 늘고 줄어듭니다.
        centerItem.holdingPriority = .defaultLow - 1
        // 열린 결정 O2 전에는 캔버스를 사이드바·인스펙터 아래로 확장하지 않습니다(가이드 2.3).
        centerItem.automaticallyAdjustsSafeAreaInsets = false

        Self.attachAccessories(of: slots.sidebar, to: sidebarItem)
        Self.attachAccessories(of: slots.content, to: centerItem)
        Self.attachAccessories(of: slots.inspector, to: inspectorItem)

        Self.labelPaneIfNeeded(slots.sidebar)
        Self.labelPaneIfNeeded(slots.inspector)

        addSplitViewItem(sidebarItem)
        addSplitViewItem(centerItem)
        addSplitViewItem(inspectorItem)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellSplitViewController는 코드로만 생성합니다")
    }

    /// 슬롯이 접근성 레이블을 지정하지 않았으면 pane 이름을 VoiceOver 영역 이름으로 붙입니다(가이드 8.3).
    /// 역할과 요소 여부는 슬롯 구현이 정합니다.
    static func labelPaneIfNeeded(_ slot: any ShellPaneSlot) {
        let view = slot.paneViewController.view
        if view.accessibilityLabel()?.isEmpty ?? true {
            view.setAccessibilityLabel(slot.paneTitle)
        }
    }

    static func attachAccessories(of slot: any ShellPaneSlot, to item: NSSplitViewItem) {
        for accessory in slot.topAccessoryViewControllers {
            item.addTopAlignedAccessoryViewController(accessory)
        }
        for accessory in slot.bottomAccessoryViewControllers {
            item.addBottomAlignedAccessoryViewController(accessory)
        }
    }

    // MARK: - 표준 pane 명령(View 메뉴, 단축키, 툴바가 같은 명령 하나로 연결, 가이드 2.3)

    override func toggleSidebar(_ sender: Any?) {
        if prefersReducedMotion() {
            sidebarItem.isCollapsed.toggle()
        } else {
            super.toggleSidebar(sender)
        }
    }

    override func toggleInspector(_ sender: Any?) {
        if prefersReducedMotion() {
            inspectorItem.isCollapsed.toggle()
        } else {
            super.toggleInspector(sender)
        }
    }

    override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        switch item.action {
        case #selector(toggleInspector(_:)):
            let title = inspectorItem.isCollapsed ? ShellFrameStrings.showInspector : ShellFrameStrings.hideInspector
            if let menuItem = item as? NSMenuItem, menuItem.title != title { menuItem.title = title }
            return true
        default:
            return super.validateUserInterfaceItem(item)
        }
    }
}
