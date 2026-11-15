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

    init(inspector: InspectorViewController) {
        self.inspector = inspector
    }

    var paneViewController: NSViewController { inspector }
    var paneTitle: String { String(localized: "Inspector", comment: "Accessibility name of the inspector") }
}
