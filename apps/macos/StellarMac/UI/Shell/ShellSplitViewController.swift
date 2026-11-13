import AppKit

/// 메인 창 3열 분할: 사이드바 | 가운데(콘텐츠 / 보조 패널) | 인스펙터(가이드 2.3).
///
/// 사이드바는 `NSSplitViewItem(sidebarWithViewController:)`, 인스펙터는
/// `NSSplitViewItem(inspectorWithViewController:)`로 만들어 재질, 접기·펼치기, 폭 유지를 시스템에
/// 맡깁니다. pane 폭과 접힘 상태는 `NSSplitView.autosaveName`으로 유지합니다(가이드 2.6).
final class ShellSplitViewController: NSSplitViewController {
    let sidebarItem: NSSplitViewItem
    let centerItem: NSSplitViewItem
    let inspectorItem: NSSplitViewItem
    /// 가운데 분할(콘텐츠 / 보조 패널).
    let centerController: ShellCenterSplitViewController

    /// 동작 줄이기 설정 조회. 켜져 있으면 pane 전환을 애니메이션 없이 즉시 합니다(가이드 8.4, 8.5).
    /// 검사에서 결정적으로 실행하려고 바꿀 수 있게 둡니다.
    var prefersReducedMotion: () -> Bool = {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    /// 콘텐츠 슬롯과 상태 화면을 담은 controller.
    var contentContainer: ShellContentContainerViewController { centerController.contentContainer }

    init(slots: ShellFrameSlots, configuration: ShellWindowConfiguration) {
        centerController = ShellCenterSplitViewController(
            contentContainer: ShellContentContainerViewController(contentSlot: slots.content),
            auxiliary: slots.auxiliary,
            autosaveName: configuration.centerSplitAutosaveName
        )
        sidebarItem = NSSplitViewItem(sidebarWithViewController: slots.sidebar.paneViewController)
        centerItem = NSSplitViewItem(viewController: centerController)
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

    /// 보조 패널 표시 전환. 보조 패널 슬롯이 없으면 아무것도 하지 않습니다.
    @objc func toggleAuxiliaryPanel(_ sender: Any?) {
        centerController.toggleAuxiliaryPanel(animated: !prefersReducedMotion())
    }

    override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        switch item.action {
        case #selector(toggleInspector(_:)):
            (item as? NSMenuItem)?.title = inspectorItem.isCollapsed ? "Show Inspector" : "Hide Inspector"
            return true
        case #selector(toggleAuxiliaryPanel(_:)):
            guard let auxiliaryItem = centerController.auxiliaryItem else { return false }
            let title = centerController.auxiliaryTitle ?? ""
            (item as? NSMenuItem)?.title = auxiliaryItem.isCollapsed ? "Show \(title)" : "Hide \(title)"
            return true
        default:
            return super.validateUserInterfaceItem(item)
        }
    }
}

/// 가운데 세로 분할: 콘텐츠 위, 보조 패널 아래.
///
/// 보조 패널 슬롯이 없으면 콘텐츠 항목 하나만 둡니다. 있으면 처음에는 접힌 상태로 시작합니다
/// (가이드 2.4 "초기 릴리스에는 노출하지 않습니다"). 높이와 접힘은 split view autosave가 유지합니다.
final class ShellCenterSplitViewController: NSSplitViewController {
    let contentContainer: ShellContentContainerViewController
    let contentItem: NSSplitViewItem
    let auxiliaryItem: NSSplitViewItem?
    /// 보조 패널 이름(View 메뉴 항목 제목).
    let auxiliaryTitle: String?

    init(
        contentContainer: ShellContentContainerViewController,
        auxiliary: (any ShellAuxiliaryPanelSlot)?,
        autosaveName: String
    ) {
        self.contentContainer = contentContainer
        contentItem = NSSplitViewItem(viewController: contentContainer)
        contentItem.minimumThickness = ShellFrameMetrics.contentMinimumSize.height
        contentItem.holdingPriority = .defaultLow - 1

        if let auxiliary {
            let item = NSSplitViewItem(viewController: auxiliary.paneViewController)
            item.canCollapse = true
            item.isCollapsed = true
            item.minimumThickness = ShellFrameMetrics.auxiliaryThickness.lowerBound
            item.maximumThickness = ShellFrameMetrics.auxiliaryThickness.upperBound
            item.holdingPriority = .defaultLow
            ShellSplitViewController.attachAccessories(of: auxiliary, to: item)
            ShellSplitViewController.labelPaneIfNeeded(auxiliary)
            auxiliaryItem = item
            auxiliaryTitle = auxiliary.paneTitle
        } else {
            auxiliaryItem = nil
            auxiliaryTitle = nil
        }
        super.init(nibName: nil, bundle: nil)

        let split = NSSplitView()
        split.isVertical = false
        split.dividerStyle = .thin
        split.autosaveName = autosaveName
        splitView = split

        addSplitViewItem(contentItem)
        if let auxiliaryItem {
            addSplitViewItem(auxiliaryItem)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellCenterSplitViewController는 코드로만 생성합니다")
    }

    func toggleAuxiliaryPanel(animated: Bool) {
        guard let auxiliaryItem else { return }
        if animated {
            auxiliaryItem.animator().isCollapsed = !auxiliaryItem.isCollapsed
        } else {
            auxiliaryItem.isCollapsed.toggle()
        }
    }
}
