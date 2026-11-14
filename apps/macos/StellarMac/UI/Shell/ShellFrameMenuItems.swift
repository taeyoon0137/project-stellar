import AppKit

/// 셸 프레임 명령의 메뉴 막대 항목(가이드 6.1 "툴바의 모든 명령은 메뉴 막대에도", 8.1).
///
/// 메인 메뉴 구성은 XII.6.5 범위이므로 여기서는 항목만 만들고 설치하지 않습니다. 모든 항목은
/// target 없는(first responder) action이고, 툴바와 같은 동작 메서드로 연결됩니다. 제목은 활성화 검사
/// (`NSMenuItemValidation`)에서 현재 상태("Show …"/"Hide …")로 바뀝니다.
@MainActor
enum ShellFrameMenuItems {
    /// View 메뉴 항목: 사이드바·인스펙터(·보조 패널) 표시 전환.
    ///
    /// - Parameter auxiliaryTitle: 보조 패널 슬롯 이름. nil이면 보조 패널 항목을 만들지 않습니다.
    static func makeViewMenuItems(auxiliaryTitle: String?) -> [NSMenuItem] {
        var items = [
            // Control-Command-S: 사이드바 표시 전환(시스템 앱 관례, HIG Keyboard).
            makeItem(ShellFrameStrings.showSidebar, #selector(ShellWindowController.toggleSidebar(_:)), "s", [.control, .command]),
            // Option-Command-I: 인스펙터 표시 전환(HIG Keyboard 표준 단축키).
            makeItem(ShellFrameStrings.showInspector, #selector(ShellWindowController.toggleInspector(_:)), "i", [.option, .command]),
        ]
        if let auxiliaryTitle {
            // 보조 패널 단축키는 앱 고유 메뉴 구성(O8, XII.6.5)에서 정합니다.
            items.append(makeItem(ShellFrameStrings.showPane(auxiliaryTitle), #selector(ShellWindowController.toggleAuxiliaryPanel(_:)), "", []))
        }
        return items
    }

    /// 내비게이션·검색 항목: 뒤로, 앞으로, 프로젝트 검색.
    static func makeNavigationMenuItems() -> [NSMenuItem] {
        [
            // Command-[ / Command-]: 뒤로·앞으로(HIG Keyboard 표준 단축키).
            makeItem(ShellFrameStrings.back, #selector(ShellWindowController.navigateBack(_:)), "[", [.command]),
            makeItem(ShellFrameStrings.forward, #selector(ShellWindowController.navigateForward(_:)), "]", [.command]),
            // 추가 입력이 필요하므로 말줄임표를 붙입니다. 단축키는 XII.6.5에서 정합니다.
            makeItem(ShellFrameStrings.searchProjectMenu, #selector(ShellWindowController.focusProjectSearch(_:)), "", []),
        ]
    }

    private static func makeItem(
        _ title: String,
        _ action: Selector,
        _ keyEquivalent: String,
        _ modifiers: NSEvent.ModifierFlags
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
        item.keyEquivalentModifierMask = modifiers
        item.target = nil
        return item
    }
}
