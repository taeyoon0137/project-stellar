import AppKit

/// 메인 메뉴 막대를 코드로 구성합니다(XII.6.5, 디자인 가이드 8.1).
///
/// 순서는 HIG The menu bar를 따릅니다: 앱(Stellar), File, Edit, View, 앱 고유 메뉴(Insert, Object), Window, Help.
/// Format 메뉴는 서식 있는 텍스트 편집 명령이 생길 때 Edit와 View 사이에 넣습니다(열린 결정 O8).
/// 모든 명령 항목은 `ShellCommand` 하나에 대응하며 identifier로 그 명령을 싣습니다.
/// - 표준 action 명령: target 없이 응답자 체인으로 보냅니다(시스템·field editor·`ShellCommandRouter`).
/// - 창 수준 셸 명령: target 없이 `performShellCommand(_:)`를 보내 활성 창의 router가 받습니다.
/// - 앱 수준 셸 명령: `applicationTarget`을 명시 target으로 씁니다(창이 없어도 동작).
enum MainMenuBuilder {
    /// 앱 메뉴와 About·Hide·Quit·Help 제목에 쓰는 짧은 앱 이름(HIG The menu bar, 16자 이하).
    static let applicationName = "Stellar"

    /// 메뉴 막대 최상위 제목(순서 고정).
    static let topLevelTitles = [applicationName, "File", "Edit", "View", "Insert", "Object", "Window", "Help"]

    /// 구성 결과. `install(on:)`으로 앱에 연결합니다.
    struct MainMenu {
        let menu: NSMenu
        let servicesMenu: NSMenu
        let windowsMenu: NSMenu
        let helpMenu: NSMenu

        /// 앱 메인 메뉴와 시스템 관리 메뉴(Services, 창 목록, Help 검색)를 연결합니다.
        func install(on application: NSApplication) {
            application.mainMenu = menu
            application.servicesMenu = servicesMenu
            application.windowsMenu = windowsMenu
            application.helpMenu = helpMenu
        }
    }

    /// 메뉴 한 칸의 구성 요소.
    private indirect enum Entry {
        case command(ShellCommand)
        case submenu(title: String, symbol: String, groups: [[Entry]])
        case services
    }

    /// 최상위 메뉴별 구성(그룹 사이에 구분선).
    private static let layout: [(title: String, groups: [[Entry]])] = [
        (applicationName, [
            [.command(.about)],
            [.command(.settings)],
            [.services],
            [.command(.hideApp), .command(.hideOthers), .command(.showAll)],
            [.command(.quit)],
        ]),
        ("File", [
            [.command(.newProject), .command(.openProject), .command(.cloneProject)],
            [.command(.closeWindow), .command(.closeProject)],
            [.command(.saveNow)],
        ]),
        ("Edit", [
            [.command(.undo), .command(.redo)],
            [.command(.cut), .command(.copy), .command(.paste), .command(.delete), .command(.duplicate)],
            [.command(.selectAll), .command(.deselectAll)],
            [.submenu(title: "Find", symbol: "magnifyingglass", groups: [
                [.command(.find), .command(.findNext), .command(.findPrevious)],
            ])],
        ]),
        ("View", [
            [.command(.toggleToolbar), .command(.customizeToolbar)],
            [.command(.toggleSidebar), .command(.toggleInspector), .command(.componentLibrary)],
            ShellCommand.zoomCommands.map { .command($0) },
            [.command(.selectMoveTool), .command(.selectHandTool), .command(.selectZoomTool)],
            [.command(.toggleRulers)],
            [.command(.toggleFullScreen)],
        ]),
        ("Insert", [
            [.command(.newComponent)],
            [.command(.selectFrameTool)],
            [.command(.selectRectangleTool), .command(.selectEllipseTool),
             .command(.selectLineTool), .command(.selectPolygonTool)],
            [.command(.selectPenTool), .command(.selectTextTool)],
            [.command(.selectImageTool)],
        ]),
        ("Object", [
            [.command(.group), .command(.ungroup), .command(.frameSelection)],
            [.command(.bringToFront), .command(.bringForward), .command(.sendBackward), .command(.sendToBack)],
            [
                .submenu(title: "Align", symbol: "align.horizontal.left", groups: [
                    [.command(.alignLeft), .command(.alignCenter), .command(.alignRight)],
                    [.command(.alignTop), .command(.alignMiddle), .command(.alignBottom)],
                ]),
                .submenu(title: "Distribute", symbol: "distribute.horizontal.center", groups: [
                    [.command(.distributeHorizontally), .command(.distributeVertically)],
                ]),
            ],
            [.command(.flipHorizontal), .command(.flipVertical)],
            [.command(.toggleLock), .command(.toggleVisibility)],
        ]),
        ("Window", [
            [.command(.minimize), .command(.minimizeAll), .command(.zoomWindow)],
            [.command(.bringAllToFront)],
        ]),
        ("Help", [
            [.command(.help)],
        ]),
    ]

    /// 메인 메뉴를 만듭니다. 앱에 연결하지는 않습니다(`MainMenu.install(on:)`).
    ///
    /// - Parameter applicationTarget: 앱 수준 셸 명령(New/Open/Clone Project, Settings…)의 명시 target. 창이 없을 때도
    ///   이 명령들이 동작해야 하므로 반드시 넘깁니다. 빠뜨리면 창 router가 없는 동안 항목이 비활성으로 남습니다.
    static func build(applicationTarget: AnyObject) -> MainMenu {
        let main = NSMenu(title: "Main Menu")
        let services = NSMenu(title: "Services")
        var topMenus: [String: NSMenu] = [:]

        func makeMenu(title: String, groups: [[Entry]]) -> NSMenu {
            let menu = NSMenu(title: title)
            for (index, group) in groups.enumerated() {
                if index > 0 { menu.addItem(.separator()) }
                for entry in group {
                    switch entry {
                    case .command(let command):
                        let target: AnyObject? = command.scope == .application ? applicationTarget : nil
                        menu.addItem(makeItem(for: command, target: target))
                        for alternate in command.hiddenAlternateShortcuts {
                            let hidden = makeItem(for: command, target: target)
                            hidden.keyEquivalent = alternate.key
                            hidden.keyEquivalentModifierMask = alternate.modifiers.eventFlags
                            hidden.isHidden = true
                            hidden.allowsKeyEquivalentWhenHidden = true
                            menu.addItem(hidden)
                        }
                    case .submenu(let title, let symbol, let subgroups):
                        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                        item.submenu = makeMenu(title: title, groups: subgroups)
                        item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
                        menu.addItem(item)
                    case .services:
                        let item = NSMenuItem(title: "Services", action: nil, keyEquivalent: "")
                        item.submenu = services
                        menu.addItem(item)
                    }
                }
            }
            return menu
        }

        for (title, groups) in layout {
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            let submenu = makeMenu(title: title, groups: groups)
            item.submenu = submenu
            main.addItem(item)
            topMenus[title] = submenu
        }

        normalizeIconGroups(in: main) { item in
            ShellCommand.resolve(from: item)?.symbolName
        }

        // layout에 Window·Help가 항상 있으므로 비어 있지 않습니다.
        return MainMenu(
            menu: main,
            servicesMenu: services,
            windowsMenu: topMenus["Window"] ?? NSMenu(title: "Window"),
            helpMenu: topMenus["Help"] ?? NSMenu(title: "Help")
        )
    }

    /// 명령 하나의 메뉴 항목(툴바 줌 메뉴와 공유).
    ///
    /// - Parameters:
    ///   - target: 표준 action이 없는 명령의 명시 target. nil이면 응답자 체인으로 보냅니다.
    ///   - includeShortcut: false이면 key equivalent를 넣지 않습니다(메인 메뉴 밖 메뉴용).
    static func makeItem(for command: ShellCommand, target: AnyObject?, includeShortcut: Bool = true) -> NSMenuItem {
        let action = command.standardAction ?? #selector(ShellCommandRouter.performShellCommand(_:))
        let item = NSMenuItem(title: command.title, action: action, keyEquivalent: "")
        item.identifier = command.menuItemIdentifier
        if command.standardAction == nil {
            item.target = target
            // 표준 action이 아닌 항목은 시스템 아이콘이 없으므로 SF Symbol을 붙입니다(디자인 가이드 8.1).
            item.image = NSImage(systemSymbolName: command.symbolName, accessibilityDescription: nil)
        }
        if includeShortcut, let shortcut = command.shortcut {
            item.keyEquivalent = shortcut.key
            item.keyEquivalentModifierMask = shortcut.modifiers.eventFlags
        }
        if let tool = command.canvasTool, let key = tool.singleKey {
            // 단일 키는 메뉴 key equivalent가 아니므로 도움말 태그로 알립니다(디자인 가이드 8.1, 8.2).
            item.toolTip = "Canvas shortcut: \(String(key).uppercased())"
        }
        item.isAlternate = command.isDynamicAlternate
        return item
    }

    /// 구분선으로 나뉜 그룹마다 아이콘을 모두 넣거나 모두 빼도록 맞춥니다(디자인 가이드 8.1).
    ///
    /// macOS는 표준 action 항목에 시스템 아이콘을 붙입니다. 그룹에 아이콘이 하나라도 있으면 빈 항목을
    /// `symbol(item)`의 SF Symbol로 채웁니다. 이미 있는 시스템 아이콘은 바꾸지 않습니다.
    static func normalizeIconGroups(in menu: NSMenu, symbol: (NSMenuItem) -> String?) {
        var group: [NSMenuItem] = []
        func flush() {
            let visible = group.filter { !$0.isHidden }
            if visible.contains(where: { $0.image != nil }) {
                for item in visible where item.image == nil {
                    if let name = symbol(item) {
                        item.image = NSImage(systemSymbolName: name, accessibilityDescription: nil)
                    }
                }
            }
            group.removeAll()
        }
        for item in menu.items {
            if item.isSeparatorItem {
                flush()
            } else {
                group.append(item)
            }
            if let submenu = item.submenu {
                normalizeIconGroups(in: submenu, symbol: symbol)
            }
        }
        flush()
    }
}
