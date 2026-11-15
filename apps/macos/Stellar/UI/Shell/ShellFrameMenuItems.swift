import AppKit

/// 셸 프레임 명령의 메뉴 막대 항목(가이드 6.1 "툴바의 모든 명령은 메뉴 막대에도", 8.1).
///
/// 항목만 만들고, 앱 coordinator(`ShellMainMenuInstaller`)가 `MainMenuBuilder` 메뉴에 끼웁니다. 모든 항목은
/// target 없는(first responder) action이고, 툴바와 같은 동작 메서드로 연결됩니다. 제목은 활성화 검사
/// (`NSMenuItemValidation`)에서 현재 상태("Show …"/"Hide …")로 바뀝니다.
@MainActor
enum ShellFrameMenuItems {
    /// View 메뉴 항목: 사이드바·인스펙터 표시 전환.
    static func makeViewMenuItems() -> [NSMenuItem] {
        [
            // Control-Command-S: 사이드바 표시 전환(시스템 앱 관례, HIG Keyboard).
            makeItem(ShellFrameStrings.showSidebar, #selector(ShellWindowController.toggleSidebar(_:)), "s", [.control, .command]),
            // Option-Command-I: 인스펙터 표시 전환(HIG Keyboard 표준 단축키).
            makeItem(ShellFrameStrings.showInspector, #selector(ShellWindowController.toggleInspector(_:)), "i", [.option, .command]),
        ]
    }

    /// 제목 문서 메뉴 동작의 메뉴 막대 항목(가이드 6.1 "툴바의 모든 명령은 메뉴 막대에도").
    ///
    /// `representedObject`의 명령 ID를 활성 창의 `performFrameAction(_:)`으로 보냅니다. 활성 창 snapshot의 제목 문서
    /// 동작에 그 ID가 없거나 비활성이면 숨기지 않고 비활성화합니다.
    static func makeFrameActionItem(title: String, id: ShellCommandID) -> NSMenuItem {
        let item = makeItem(title, #selector(ShellWindowController.performFrameAction(_:)), "", [])
        item.representedObject = id.rawValue
        return item
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
