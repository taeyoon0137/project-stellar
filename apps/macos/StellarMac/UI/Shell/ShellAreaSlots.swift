import AppKit

// 다른 UI 영역(사이드바, 인스펙터, Git·에이전트 패널)을 셸 프레임 슬롯에 끼우는 어댑터.
//
// 각 영역의 공개 view controller를 그대로 쓰고, 셸 슬롯 protocol만 덧붙입니다. 영역 파일은 수정하지 않습니다.
// 영역별 코어 adapter(사이드바 snapshot, 선택 속성, Git 상태, 에이전트 세션)는 아직 없으므로 프로젝트 창은
// 아래 `ShellAreaDataStubs`의 placeholder data source를 씁니다.

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

/// 캔버스 아래 보조 패널 슬롯: Git 패널과 에이전트 패널을 탭으로 둡니다.
///
/// Git 패널 위치(가이드 O4, XIV.13.5)와 에이전트 패널 위치(가이드 6.5, ADR-0005 후속 WBS)는 아직 정하지 않았으므로,
/// 셸 프레임이 마련한 보조 패널 자리(처음에는 접힘, 툴바 버튼 없음, View 메뉴로만 표시)에 둡니다. 탭 전환은 표준
/// `NSTabViewController`(segmented control)가 맡습니다.
@MainActor
final class ShellPanelTabsSlot: ShellAuxiliaryPanelSlot {
    let tabs: NSTabViewController
    let gitPanel: GitPanelViewController
    let agentPanel: AgentPanelViewController

    init(gitPanel: GitPanelViewController, agentPanel: AgentPanelViewController) {
        self.gitPanel = gitPanel
        self.agentPanel = agentPanel
        let tabs = NSTabViewController()
        tabs.tabStyle = .segmentedControlOnTop
        let gitItem = NSTabViewItem(viewController: gitPanel)
        gitItem.label = String(localized: "Git", comment: "Tab title of the Git panel")
        gitItem.identifier = "git"
        let agentItem = NSTabViewItem(viewController: agentPanel)
        agentItem.label = String(localized: "Agent", comment: "Tab title of the agent panel")
        agentItem.identifier = "agent"
        tabs.addTabViewItem(gitItem)
        tabs.addTabViewItem(agentItem)
        self.tabs = tabs
    }

    var paneViewController: NSViewController { tabs }
    var paneTitle: String { String(localized: "Panels", comment: "Name of the area below the canvas; used in Show/Hide Panels") }

    /// 활성 에이전트 세션이 있으면 창을 닫기 전에 확인해야 합니다(ADR-0005 결정 4.5).
    var requiresConfirmationBeforeClosing: Bool {
        agentPanel.requiresConfirmationBeforeClosing
    }
}

/// 프로젝트 창이 영역 view controller에 넘기는 data source 묶음.
///
/// 영역별 코어 adapter가 생기기 전까지 쓰는 의도적 placeholder입니다(계획 1.2 규칙 6, 이름에 "Stub").
/// 열린 프로젝트와 어긋난 표본 내용을 보여 주지 않도록 각 Stub의 "비어 있음"에 가까운 상태를 고릅니다.
/// - 사이드바: 페이지·레이어 없음(코어가 문서 내용을 아직 셸에 주지 않음).
/// - 인스펙터: 선택 없음.
/// - 에이전트: 세션 없음.
@MainActor
enum ShellAreaDataStubs {
    static func makeSidebarProvider() -> SidebarDataProvider {
        ShellEmptySidebarProviderStub()
    }

    static func makeInspectorDataSource() -> InspectorDataSource {
        InspectorStubDataSource(scenario: .noSelection)
    }

    static func makeAgentSource() -> AgentPanelSource {
        AgentSessionStub(scenario: .noSession)
    }
}

/// 페이지·레이어가 없는 사이드바 공급자(의도적 placeholder).
///
/// 코어가 사이드바 snapshot을 내보내기 전까지 프로젝트 창은 빈 문서 상태를 보여 줍니다. 모든 명령은 지원하지 않음으로
/// 검증하고 보내도 아무것도 바꾸지 않습니다(받은 명령은 `performedCommands`에 기록).
@MainActor
final class ShellEmptySidebarProviderStub: SidebarDataProvider {
    private(set) var snapshot: SidebarSnapshot
    var snapshotDidChange: (() -> Void)?
    private(set) var performedCommands: [SidebarCommand] = []

    init() {
        var snapshot = SidebarSnapshot.empty
        snapshot.generation = 1
        snapshot.loadState = .ready
        self.snapshot = snapshot
    }

    func validate(_ command: SidebarCommand) -> SidebarCommandValidation {
        .denied(.unsupported)
    }

    func perform(_ command: SidebarCommand) {
        performedCommands.append(command)
    }
}
