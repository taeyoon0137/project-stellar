import AppKit

// 메인 창 슬롯 계약, 창 구성 값, 배치 수치.
//
// 다른 UI 영역(사이드바 내용, 인스펙터 내용, Git·에이전트 패널, 캔버스)은 아래 슬롯 protocol을
// 따르는 view controller를 넘겨 메인 창에 끼웁니다. 셸 프레임은 슬롯 내용을 해석하지 않습니다.

/// 메인 창의 pane 하나에 들어가는 내용.
@MainActor
protocol ShellPaneSlot: AnyObject {
    /// pane에 넣을 view controller. 셸이 창 생성 시 한 번 읽고 창 수명 동안 유지합니다.
    var paneViewController: NSViewController { get }
    /// pane 이름. VoiceOver 영역 레이블과 View 메뉴 항목 제목에 씁니다(가이드 8.3).
    var paneTitle: String { get }
    /// pane 상단에 붙일 accessory(예: 레이어 필터 필드). safe area와 scroll edge effect는
    /// 시스템이 처리합니다(가이드 2.3, 6.2).
    var topAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { get }
    /// pane 하단에 붙일 accessory. 핵심 동작은 두지 않습니다(가이드 2.4, 6.2).
    var bottomAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { get }
}

extension ShellPaneSlot {
    var topAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { [] }
    var bottomAccessoryViewControllers: [NSSplitViewItemAccessoryViewController] { [] }
}

/// 왼쪽 사이드바(문서·페이지·View 목록, 레이어 트리) 슬롯(가이드 6.2).
@MainActor protocol ShellSidebarSlot: ShellPaneSlot {}

/// 가운데 콘텐츠(캔버스 호스트) 슬롯. 남는 공간을 모두 차지합니다(가이드 2.3).
@MainActor protocol ShellContentSlot: ShellPaneSlot {}

/// 오른쪽 인스펙터 슬롯(가이드 6.3).
@MainActor protocol ShellInspectorSlot: ShellPaneSlot {}

/// 콘텐츠 아래 보조 패널 슬롯(Git·에이전트·터미널 후보).
///
/// 가이드 2.4는 캔버스 아래 분할 영역을 로드맵 기능 자리로만 두고 초기 릴리스에 노출하지 않으며,
/// Git 패널 위치(O4)와 에이전트 패널 위치(6.5절)는 해당 WBS가 정합니다. 따라서 이 슬롯은 선택
/// 사항이고, 넘겨도 처음에는 접힌 상태로 시작하며 툴바 버튼을 두지 않습니다.
@MainActor protocol ShellAuxiliaryPanelSlot: ShellPaneSlot {}

/// 메인 창에 끼울 슬롯 묶음.
struct ShellFrameSlots {
    var sidebar: any ShellSidebarSlot
    var content: any ShellContentSlot
    var inspector: any ShellInspectorSlot
    /// 없으면 보조 패널 분할 항목 자체를 만들지 않습니다.
    var auxiliary: (any ShellAuxiliaryPanelSlot)?
}

/// 창 하나의 AppKit 상태 저장 이름 묶음.
///
/// 창 프레임은 `NSWindow` frame autosave, pane 폭·접힘은 `NSSplitView.autosaveName`, 툴바 구성은
/// `NSToolbar.autosavesConfiguration`, 창 복원은 `NSWindowRestoration`이 맡습니다. 셸은 값을 직접
/// 저장하지 않습니다(가이드 2.6). 같은 프로젝트를 여러 창으로 열 때는 창마다 다른 이름을 넘깁니다.
struct ShellWindowConfiguration {
    /// 창 frame autosave 이름. split view autosave 이름의 접두어로도 씁니다.
    var autosaveName: String
    /// 툴바 식별자. 같은 식별자의 툴바는 사용자 구성을 공유합니다.
    var toolbarIdentifier: String
    /// 창 복원 식별자(`NSWindow.identifier`).
    var restorationIdentifier: String

    static let standard = ShellWindowConfiguration(
        autosaveName: "StellarMainWindow",
        toolbarIdentifier: "StellarMainToolbar",
        restorationIdentifier: "StellarMainWindow"
    )

    /// 메인 분할(사이드바 | 가운데 | 인스펙터) autosave 이름.
    var mainSplitAutosaveName: String { autosaveName + ".MainSplit" }
    /// 가운데 분할(콘텐츠 / 보조 패널) autosave 이름.
    var centerSplitAutosaveName: String { autosaveName + ".CenterSplit" }
}

/// 셸 프레임 배치 수치를 한곳에서 정의합니다(가이드 2.3 "허용 범위는 코드 한곳에서 정의").
///
/// 가이드 0.2에 따라 이 값들은 셸 프레임 시안 값이며, XII.9.1에서 측정해 확정하고 가이드 12장에
/// 기록합니다.
enum ShellFrameMetrics {
    /// 저장된 프레임이 없을 때의 첫 창 콘텐츠 크기(point).
    static let initialContentSize = NSSize(width: 1200, height: 760)
    /// 창 콘텐츠 최소 크기. 더 좁아지면 사이드바·인스펙터가 시스템 규칙으로 접힙니다.
    static let minimumContentSize = NSSize(width: 640, height: 400)
    /// 사이드바 폭 허용 범위.
    static let sidebarThickness: ClosedRange<CGFloat> = 200...360
    /// 인스펙터 폭 허용 범위.
    static let inspectorThickness: ClosedRange<CGFloat> = 240...400
    /// 캔버스(가운데 콘텐츠) 최소 폭·높이.
    static let contentMinimumSize = NSSize(width: 320, height: 200)
    /// 보조 패널 높이 허용 범위.
    static let auxiliaryThickness: ClosedRange<CGFloat> = 140...480
    /// 상태·placeholder 안내문 최대 줄 폭.
    static let statusTextMaxWidth: CGFloat = 320
    /// 상태 화면 요소 사이 간격.
    static let statusStackSpacing: CGFloat = 8
}

/// 셸 프레임이 쓰는 SF Symbol 이름(가이드 5.4). 이미지 asset은 쓰지 않습니다.
enum ShellFrameSymbols {
    static let moreMenu = "ellipsis"
    static let emptyStatus = "square.dashed"
    static let failedStatus = "exclamationmark.triangle"

    /// 셸 프레임 자체가 쓰는 이름 전체(검사용).
    static let all = [moreMenu, emptyStatus, failedStatus]
}
