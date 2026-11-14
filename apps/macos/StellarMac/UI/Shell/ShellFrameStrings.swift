import Foundation

/// 셸 프레임이 직접 쓰는 UI 문자열.
///
/// 모두 `String(localized:)`로 만들어 현지화 표(String Catalog)를 붙일 수 있게 둡니다. 상태를 반영하는 제목은
/// 문자열을 이어 붙이지 않고 형식 문자열(예: "Show %@") 하나로 만들어, 언어마다 어순을 바꿀 수 있게 합니다.
/// 현지화 표는 아직 없으므로 지금은 영문 키가 그대로 보입니다.
enum ShellFrameStrings {
    static var back: String { String(localized: "Back", comment: "Toolbar and menu: navigate back") }
    static var forward: String { String(localized: "Forward", comment: "Toolbar and menu: navigate forward") }
    static var backForward: String {
        String(localized: "Back/Forward", comment: "Toolbar customization label for the back/forward group")
    }
    static var tools: String { String(localized: "Tools", comment: "Accessibility label for the tool bar at the bottom of the canvas") }
    static var search: String { String(localized: "Search", comment: "Toolbar label for project search") }
    static var searchProject: String {
        String(localized: "Search Project", comment: "Toolbar palette label and help tag for project search")
    }
    static var searchProjectMenu: String {
        String(localized: "Search Project…", comment: "Menu item that focuses the project search field")
    }
    static var more: String { String(localized: "More", comment: "Toolbar label for the More menu") }
    static var canvas: String { String(localized: "Canvas", comment: "Accessibility name of the canvas area") }
    static var showSidebar: String { String(localized: "Show Sidebar", comment: "View menu item") }
    static var showInspector: String { String(localized: "Show Inspector", comment: "View menu item") }
    static var hideInspector: String { String(localized: "Hide Inspector", comment: "View menu item") }

    /// 보조 패널 표시 항목 제목. `name`은 pane 이름입니다.
    static func showPane(_ name: String) -> String {
        String(localized: "Show \(name)", comment: "View menu item that shows a pane; the argument is the pane name")
    }

    /// 보조 패널 숨김 항목 제목. `name`은 pane 이름입니다.
    static func hidePane(_ name: String) -> String {
        String(localized: "Hide \(name)", comment: "View menu item that hides a pane; the argument is the pane name")
    }
}
