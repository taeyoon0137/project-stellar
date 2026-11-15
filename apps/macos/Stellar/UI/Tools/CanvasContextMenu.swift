import AppKit

/// 캔버스 우클릭 문맥 메뉴(UX.3.2, 명세 5.6).
///
/// 메뉴 막대와 같은 `ShellCommand`(같은 command ID)를 쓰고, 지금 실행할 수 있는 명령만 넣습니다. 코어에 연결되지
/// 않았거나 지금 대상에 동작하지 않는 명령은 항목을 만들지 않습니다(비활성 진열 없음). 문맥 메뉴에는 단축키 표시를
/// 넣지 않습니다(HIG Context menus).
enum CanvasContextMenu {
    /// 명세 5.6의 순서(편집 · 순서 · 숨기기·잠금). 연결된 명령만 실제로 나타납니다.
    static let groups: [[ShellCommand]] = [
        [.cut, .copy, .paste, .duplicate, .delete],
        [.bringToFront, .bringForward, .sendBackward, .sendToBack],
        [.toggleVisibility, .toggleLock],
    ]

    /// 활성 명령만 담은 메뉴를 만듭니다. 넣을 항목이 없으면 nil입니다.
    ///
    /// - Parameters:
    ///   - target: 항목 action을 받을 창 router. 응답자 체인을 거치지 않고 그 창 session으로 보냅니다.
    ///   - excluded: snapshot은 활성이지만 지금 대상에서는 코어가 거부할 명령(예: View root 선택의 삭제).
    static func make(snapshot: ToolsMenuSnapshot, isTextEditing: Bool, target: AnyObject,
                     excluded: Set<ShellCommand> = []) -> NSMenu? {
        let menu = NSMenu(title: "Canvas")
        menu.autoenablesItems = false
        for group in groups {
            let items: [NSMenuItem] = group.compactMap { command in
                guard !excluded.contains(command) else { return nil }
                let presentation = ShellCommandPresentation.make(command, snapshot: snapshot, isTextEditing: isTextEditing)
                guard presentation.isEnabled else { return nil }
                let item = MainMenuBuilder.makeItem(for: command, target: target, includeShortcut: false)
                item.target = target
                item.title = presentation.title
                return item
            }
            guard !items.isEmpty else { continue }
            if !menu.items.isEmpty { menu.addItem(.separator()) }
            items.forEach(menu.addItem)
        }
        guard !menu.items.isEmpty else { return nil }
        MainMenuBuilder.normalizeIconGroups(in: menu) { ShellCommand.resolve(from: $0)?.symbolName }
        return menu
    }
}
