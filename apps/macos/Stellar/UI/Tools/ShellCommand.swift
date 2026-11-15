import AppKit

/// 셸 명령 하나(XII.6.4, XII.6.5).
///
/// 메뉴 항목, 툴바 항목, 캔버스 단일 키 도구 단축키가 모두 이 enum으로 명령을 지정합니다.
/// `rawValue`는 코어와 공유하는 명령 이름입니다(예: `edit.undo`). 코어 command ID는 코어가 정하고, 셸은 이 이름으로
/// 코어에 물어 `ShellCommandCoreIDs` 표를 만듭니다(계획 XII.6.5 "command ID는 core에서 공유"). 단축키(`shortcut`)는
/// macOS 관례로 셸이 정합니다.
enum ShellCommand: String, CaseIterable {
    // MARK: 앱 메뉴
    case about = "app.about"
    case settings = "app.settings"
    case hideApp = "app.hide"
    case hideOthers = "app.hide_others"
    case showAll = "app.show_all"
    case quit = "app.quit"

    // MARK: File
    case newProject = "file.new_project"
    case openProject = "file.open_project"
    case cloneProject = "file.clone_project"
    case closeWindow = "file.close_window"
    case closeProject = "file.close_project"
    case saveNow = "file.save_now"

    // MARK: Edit
    case undo = "edit.undo"
    case redo = "edit.redo"
    case cut = "edit.cut"
    case copy = "edit.copy"
    case paste = "edit.paste"
    case delete = "edit.delete"
    case duplicate = "edit.duplicate"
    case selectAll = "edit.select_all"
    case deselectAll = "edit.deselect_all"
    case find = "edit.find"
    case findNext = "edit.find_next"
    case findPrevious = "edit.find_previous"

    // MARK: View
    case toggleToolbar = "view.toggle_toolbar"
    case customizeToolbar = "view.customize_toolbar"
    case toggleSidebar = "view.toggle_sidebar"
    case toggleInspector = "view.toggle_inspector"
    case zoomIn = "view.zoom_in"
    case zoomOut = "view.zoom_out"
    case zoomToActualSize = "view.zoom_actual_size"
    case zoomToFit = "view.zoom_to_fit"
    case zoomToSelection = "view.zoom_to_selection"
    case toggleRulers = "view.toggle_rulers"
    case toggleFullScreen = "view.toggle_full_screen"

    // MARK: 도구(View·Insert 메뉴, 툴바 도구 그룹)
    case selectMoveTool = "tool.move"
    case componentFromSelection = "object.component_from_selection"
    case selectFrameTool = "tool.frame"
    case selectRectangleTool = "tool.rectangle"
    case selectEllipseTool = "tool.ellipse"
    case selectLineTool = "tool.line"
    case selectPolygonTool = "tool.polygon"
    case selectTextTool = "tool.text"
    case selectImageTool = "tool.image"
    case selectHandTool = "tool.hand"
    case selectZoomTool = "tool.zoom"

    // MARK: Object
    case group = "object.group"
    case ungroup = "object.ungroup"
    case frameSelection = "object.frame_selection"
    case bringToFront = "object.bring_to_front"
    case bringForward = "object.bring_forward"
    case sendBackward = "object.send_backward"
    case sendToBack = "object.send_to_back"
    case alignLeft = "object.align_left"
    case alignCenter = "object.align_center"
    case alignRight = "object.align_right"
    case alignTop = "object.align_top"
    case alignMiddle = "object.align_middle"
    case alignBottom = "object.align_bottom"
    case distributeHorizontally = "object.distribute_horizontally"
    case distributeVertically = "object.distribute_vertically"
    case flipHorizontal = "object.flip_horizontal"
    case flipVertical = "object.flip_vertical"
    case toggleLock = "object.toggle_lock"
    case toggleVisibility = "object.toggle_visibility"

    // MARK: Window
    case minimize = "window.minimize"
    case minimizeAll = "window.minimize_all"
    case zoomWindow = "window.zoom"
    case bringAllToFront = "window.bring_all_to_front"

    // MARK: Help
    case help = "help.app_help"

    /// 명령이 닿는 범위.
    enum Scope {
        /// 창이 없어도 동작(앱 수준 대상이 받음).
        case application
        /// 활성 창의 session으로 갑니다(응답자 체인).
        case window
    }

    /// 명령을 최종 처리하는 쪽.
    enum Owner {
        /// 셸 전용(창·메뉴·툴바·시스템 동작). 코어 command ID가 없습니다.
        case shell
        /// 코어 command로 전달(ID는 코어가 정하고 `ShellCommandCoreIDs`가 담음, 코어가 아직 구현하지 않았으면 ID 없음).
        case core
    }

    /// 메뉴 항목 기본 제목(HIG title-style). 상태에 따라 바뀌는 제목은 `ShellCommandPresentation`이 정합니다.
    var title: String {
        switch self {
        case .about: return "About \(MainMenuBuilder.applicationName)"
        case .settings: return "Settings…"
        case .hideApp: return "Hide \(MainMenuBuilder.applicationName)"
        case .hideOthers: return "Hide Others"
        case .showAll: return "Show All"
        case .quit: return "Quit \(MainMenuBuilder.applicationName)"
        case .newProject: return "New Project…"
        case .componentFromSelection: return "Create Component from Selection…"
        case .openProject: return "Open Project…"
        case .cloneProject: return "Clone Project…"
        case .closeWindow: return "Close"
        case .closeProject: return "Close Project"
        case .saveNow: return "Save Now"
        case .undo: return "Undo"
        case .redo: return "Redo"
        case .cut: return "Cut"
        case .copy: return "Copy"
        case .paste: return "Paste"
        case .delete: return "Delete"
        case .duplicate: return "Duplicate"
        case .selectAll: return "Select All"
        case .deselectAll: return "Deselect All"
        case .find: return "Find…"
        case .findNext: return "Find Next"
        case .findPrevious: return "Find Previous"
        case .toggleToolbar: return "Show Toolbar"
        case .customizeToolbar: return "Customize Toolbar…"
        case .toggleSidebar: return "Show Sidebar"
        case .toggleInspector: return "Show Inspector"
        case .zoomIn: return "Zoom In"
        case .zoomOut: return "Zoom Out"
        case .zoomToActualSize: return "Actual Size"
        case .zoomToFit: return "Zoom to Fit"
        case .zoomToSelection: return "Zoom to Selection"
        case .toggleRulers: return "Show Rulers"
        case .toggleFullScreen: return "Enter Full Screen"
        case .selectMoveTool: return "Move Tool"
        case .selectHandTool: return "Hand Tool"
        case .selectZoomTool: return "Zoom Tool"
        case .selectFrameTool, .selectRectangleTool, .selectEllipseTool, .selectLineTool,
             .selectPolygonTool, .selectTextTool, .selectImageTool:
            return canvasTool?.label ?? rawValue
        case .group: return "Group"
        case .ungroup: return "Ungroup"
        case .frameSelection: return "Frame Selection"
        case .bringToFront: return "Bring to Front"
        case .bringForward: return "Bring Forward"
        case .sendBackward: return "Send Backward"
        case .sendToBack: return "Send to Back"
        case .alignLeft: return "Align Left"
        case .alignCenter: return "Align Center"
        case .alignRight: return "Align Right"
        case .alignTop: return "Align Top"
        case .alignMiddle: return "Align Middle"
        case .alignBottom: return "Align Bottom"
        case .distributeHorizontally: return "Distribute Horizontally"
        case .distributeVertically: return "Distribute Vertically"
        case .flipHorizontal: return "Flip Horizontal"
        case .flipVertical: return "Flip Vertical"
        case .toggleLock: return "Lock"
        case .toggleVisibility: return "Hide"
        case .minimize: return "Minimize"
        case .minimizeAll: return "Minimize All"
        case .zoomWindow: return "Zoom"
        case .bringAllToFront: return "Bring All to Front"
        case .help: return "\(MainMenuBuilder.applicationName) Help"
        }
    }

    /// 실행 전에 추가 입력(대화상자·파일 선택·검색어)이 필요한 명령. 제목에 말줄임표를 붙입니다(HIG The menu bar).
    var requiresMoreInput: Bool {
        switch self {
        // 도구 선택(Image 포함)은 모드 전환이라 그 자체로 추가 입력을 받지 않습니다. 이미지 파일 선택은 도구를
        // 쓰는 단계(캔버스 클릭)에서 열리므로 메뉴 제목에 말줄임표를 붙이지 않습니다.
        case .settings, .newProject, .componentFromSelection, .openProject, .cloneProject, .find, .customizeToolbar:
            return true
        default:
            return false
        }
    }

    var scope: Scope {
        switch self {
        case .about, .settings, .hideApp, .hideOthers, .showAll, .quit,
             .newProject, .openProject, .cloneProject, .bringAllToFront, .minimizeAll, .help:
            return .application
        default:
            return .window
        }
    }

    var owner: Owner {
        switch self {
        case .about, .settings, .hideApp, .hideOthers, .showAll, .quit,
             .closeWindow,
             .toggleToolbar, .customizeToolbar, .toggleSidebar, .toggleInspector,
             .toggleRulers, .toggleFullScreen,
             .minimize, .minimizeAll, .zoomWindow, .bringAllToFront, .help:
            return .shell
        default:
            return .core
        }
    }

    /// AppKit 표준 action. nil이면 `ShellCommandRouter.performShellCommand(_:)`로 보냅니다.
    ///
    /// 표준 편집 action(undo/redo/cut/copy/paste/delete/selectAll)은 target 없이 응답자 체인으로 보내므로,
    /// 텍스트 편집 중에는 field editor가 먼저 받고 캔버스 포커스에서는 `ShellCommandRouter`가 받습니다
    /// (디자인 가이드 8.2).
    var standardAction: Selector? {
        switch self {
        case .about: return #selector(NSApplication.orderFrontStandardAboutPanel(_:))
        case .hideApp: return #selector(NSApplication.hide(_:))
        case .hideOthers: return #selector(NSApplication.hideOtherApplications(_:))
        case .showAll: return #selector(NSApplication.unhideAllApplications(_:))
        case .quit: return #selector(NSApplication.terminate(_:))
        case .closeWindow: return #selector(NSWindow.performClose(_:))
        case .undo: return #selector(ShellCommandRouter.undo(_:))
        case .redo: return #selector(ShellCommandRouter.redo(_:))
        case .cut: return #selector(NSText.cut(_:))
        case .copy: return #selector(NSText.copy(_:))
        case .paste: return #selector(NSText.paste(_:))
        case .delete: return #selector(NSText.delete(_:))
        case .selectAll: return #selector(NSText.selectAll(_:))
        case .toggleToolbar: return #selector(NSWindow.toggleToolbarShown(_:))
        case .customizeToolbar: return #selector(NSWindow.runToolbarCustomizationPalette(_:))
        case .toggleSidebar: return #selector(NSSplitViewController.toggleSidebar(_:))
        case .toggleInspector: return #selector(NSSplitViewController.toggleInspector(_:))
        case .toggleFullScreen: return #selector(NSWindow.toggleFullScreen(_:))
        case .minimize: return #selector(NSWindow.performMiniaturize(_:))
        case .minimizeAll: return #selector(NSApplication.miniaturizeAll(_:))
        case .zoomWindow: return #selector(NSWindow.performZoom(_:))
        case .bringAllToFront: return #selector(NSApplication.arrangeInFront(_:))
        case .help: return #selector(NSApplication.showHelp(_:))
        default: return nil
        }
    }

    /// 메뉴 key equivalent. 단일 키 도구 단축키는 여기에 두지 않습니다(`CanvasTool.singleKey`).
    var shortcut: KeyShortcut? {
        switch self {
        case .settings: return .command(",")
        case .hideApp: return .command("h")
        case .hideOthers: return KeyShortcut("h", [.option, .command])
        case .quit: return .command("q")
        case .newProject: return .command("n")
        case .openProject: return .command("o")
        case .closeWindow: return .command("w")
        case .closeProject: return KeyShortcut("w", [.shift, .command])
        case .saveNow: return .command("s")
        case .undo: return .command("z")
        case .redo: return KeyShortcut("z", [.shift, .command])
        case .cut: return .command("x")
        case .copy: return .command("c")
        case .paste: return .command("v")
        case .duplicate: return .command("d")
        case .selectAll: return .command("a")
        case .deselectAll: return KeyShortcut("a", [.shift, .command])
        case .find: return .command("f")
        case .findNext: return .command("g")
        case .findPrevious: return KeyShortcut("g", [.shift, .command])
        case .toggleToolbar: return KeyShortcut("t", [.option, .command])
        case .toggleSidebar: return KeyShortcut("s", [.control, .command])
        case .toggleInspector: return KeyShortcut("i", [.option, .command])
        case .zoomIn: return .command("+")
        case .zoomOut: return .command("-")
        case .zoomToActualSize: return .command("0")
        case .zoomToFit: return .command("9")
        case .zoomToSelection: return .command("*")
        case .toggleRulers: return .command("r")
        case .toggleFullScreen: return KeyShortcut("f", [.control, .command])
        case .selectImageTool: return KeyShortcut("k", [.shift, .command])
        case .group: return KeyShortcut("g", [.option, .command])
        case .ungroup: return KeyShortcut("g", [.option, .shift, .command])
        case .bringForward: return .command("]")
        case .sendBackward: return .command("[")
        case .alignLeft: return .command("{")
        case .alignCenter: return .command("|")
        case .alignRight: return .command("}")
        case .minimize: return .command("m")
        case .minimizeAll: return KeyShortcut("m", [.option, .command])
        case .help: return .command("?")
        default: return nil
        }
    }

    /// 메뉴에 보이지 않지만 같은 명령을 실행하는 보조 key equivalent.
    ///
    /// Zoom In은 ⌘+(⇧⌘=)가 표시 단축키이고, Shift 없이 누르는 ⌘=도 받습니다.
    var hiddenAlternateShortcuts: [KeyShortcut] {
        switch self {
        case .zoomIn: return [.command("=")]
        default: return []
        }
    }

    /// Option을 누르면 바로 앞 항목 자리에 나타나는 동적 메뉴 항목(HIG The menu bar "Dynamic menu items").
    var isDynamicAlternate: Bool {
        self == .minimizeAll
    }

    /// 현재 창의 실제 형제 위치에 따라 필요한 이동 가능 방향입니다.
    var layerOrderFlag: UInt32? {
        switch self {
        case .bringForward, .bringToFront: return UInt32(STELLAR_ORDER_CAN_FORWARD)
        case .sendBackward, .sendToBack: return UInt32(STELLAR_ORDER_CAN_BACKWARD)
        default: return nil
        }
    }

    /// 텍스트 컨트롤 편집 중에는 비활성화하는 캔버스 객체 명령.
    ///
    /// Copy·Paste·Delete는 텍스트 입력이 먼저 받고, router에 직접 와도 캔버스 객체 편집으로 보내지 않습니다.
    var isSuppressedDuringTextEditing: Bool {
        switch self {
        case .copy, .paste, .delete, .duplicate, .deselectAll,
             .group, .ungroup, .frameSelection,
             .bringToFront, .bringForward, .sendBackward, .sendToBack,
             .alignLeft, .alignCenter, .alignRight, .alignTop, .alignMiddle, .alignBottom,
             .distributeHorizontally, .distributeVertically,
             .flipHorizontal, .flipVertical, .toggleLock, .toggleVisibility:
            return true
        default:
            return false
        }
    }

    /// 도구 선택 명령이면 그 도구.
    var canvasTool: CanvasTool? {
        CanvasTool.allCases.first { $0.command == self }
    }

    /// 메뉴 아이콘 SF Symbol. 표준 action 항목은 macOS가 붙이는 아이콘을 우선하고,
    /// 같은 그룹에서 시스템 아이콘이 없을 때만 이 값으로 채웁니다(디자인 가이드 8.1).
    var symbolName: String {
        if let tool = canvasTool { return tool.symbolName }
        switch self {
        case .about: return "info.circle"
        case .settings: return "gearshape"
        case .hideApp: return "eye.slash"
        case .hideOthers: return "rectangle.on.rectangle.slash"
        case .showAll: return "rectangle.on.rectangle"
        case .quit: return "power"
        case .newProject: return "folder.badge.plus"
        case .componentFromSelection: return "square.3.layers.3d"
        case .openProject: return "folder"
        case .cloneProject: return "square.and.arrow.down.on.square"
        case .closeWindow: return "xmark"
        case .closeProject: return "xmark.rectangle"
        case .saveNow: return "square.and.arrow.down"
        case .undo: return "arrow.uturn.backward"
        case .redo: return "arrow.uturn.forward"
        case .cut: return "scissors"
        case .copy: return "document.on.document"
        case .paste: return "document.on.clipboard"
        case .delete: return "trash"
        case .duplicate: return "plus.square.on.square"
        case .selectAll: return "checkmark.circle"
        case .deselectAll: return "circle.dashed"
        case .find: return "magnifyingglass"
        case .findNext: return "chevron.forward"
        case .findPrevious: return "chevron.backward"
        case .toggleToolbar: return "menubar.rectangle"
        case .customizeToolbar: return "wrench.adjustable"
        case .toggleSidebar: return "sidebar.left"
        case .toggleInspector: return "sidebar.right"
        case .zoomIn: return "plus.magnifyingglass"
        case .zoomOut: return "minus.magnifyingglass"
        case .zoomToActualSize: return "1.magnifyingglass"
        case .zoomToFit: return "arrow.up.left.and.arrow.down.right"
        case .zoomToSelection: return "viewfinder"
        case .toggleRulers: return "ruler"
        case .toggleFullScreen: return "arrow.up.left.and.arrow.down.right.rectangle"
        case .group: return "square.on.square.dashed"
        case .ungroup: return "square.on.square"
        case .frameSelection: return "number.square"
        case .bringToFront: return "square.3.layers.3d.top.filled"
        case .bringForward: return "square.2.layers.3d.top.filled"
        case .sendBackward: return "square.2.layers.3d.bottom.filled"
        case .sendToBack: return "square.3.layers.3d.bottom.filled"
        case .alignLeft: return "align.horizontal.left"
        case .alignCenter: return "align.horizontal.center"
        case .alignRight: return "align.horizontal.right"
        case .alignTop: return "align.vertical.top"
        case .alignMiddle: return "align.vertical.center"
        case .alignBottom: return "align.vertical.bottom"
        case .distributeHorizontally: return "distribute.horizontal.center"
        case .distributeVertically: return "distribute.vertical.center"
        case .flipHorizontal: return "arrow.left.and.right.righttriangle.left.righttriangle.right"
        case .flipVertical: return "arrow.up.and.down.righttriangle.up.righttriangle.down"
        case .toggleLock: return "lock"
        case .toggleVisibility: return "eye.slash"
        case .minimize: return "minus.rectangle"
        case .minimizeAll: return "minus.rectangle.portrait"
        case .zoomWindow: return "arrow.up.left.and.arrow.down.right.square"
        case .bringAllToFront: return "square.3.layers.3d"
        case .help: return "questionmark.circle"
        case .selectMoveTool, .selectFrameTool, .selectRectangleTool, .selectEllipseTool, .selectLineTool,
             .selectPolygonTool, .selectTextTool, .selectImageTool, .selectHandTool,
             .selectZoomTool:
            return "questionmark"
        }
    }

    /// 메뉴 항목 identifier 접두사. 시스템 identifier와 섞이지 않게 번들 ID를 붙입니다.
    static let menuItemIdentifierPrefix = "xyz.taeyoon.stellar.command."

    /// 메뉴 항목에 붙이는 identifier.
    var menuItemIdentifier: NSUserInterfaceItemIdentifier {
        NSUserInterfaceItemIdentifier(Self.menuItemIdentifierPrefix + rawValue)
    }

    /// 메뉴 항목 identifier에서 명령을 찾습니다.
    init?(menuItemIdentifier identifier: NSUserInterfaceItemIdentifier) {
        let raw = identifier.rawValue
        guard raw.hasPrefix(Self.menuItemIdentifierPrefix) else { return nil }
        self.init(rawValue: String(raw.dropFirst(Self.menuItemIdentifierPrefix.count)))
    }

    /// 표준 action selector에서 명령을 찾습니다(builder 밖에서 만든 항목용).
    init?(standardAction action: Selector) {
        guard let match = ShellCommand.allCases.first(where: { $0.standardAction == action }) else {
            return nil
        }
        self = match
    }

    /// action sender에서 명령을 찾습니다. 메뉴 항목은 identifier, 그다음 표준 action 순서로 봅니다.
    static func resolve(from sender: Any?) -> ShellCommand? {
        if let item = sender as? NSMenuItem {
            if let identifier = item.identifier, let command = ShellCommand(menuItemIdentifier: identifier) {
                return command
            }
            if let action = item.action {
                return ShellCommand(standardAction: action)
            }
        }
        return nil
    }

    /// 줌 명령 묶음(툴바 줌 메뉴와 View 메뉴가 공유).
    static let zoomCommands: [ShellCommand] = [
        .zoomIn, .zoomOut, .zoomToActualSize, .zoomToFit, .zoomToSelection,
    ]
}
