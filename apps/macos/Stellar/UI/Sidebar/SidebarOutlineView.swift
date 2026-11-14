import AppKit

/// 사이드바 목록 행에 대한 동작.
enum SidebarOutlineAction: CaseIterable {
    case rename
    case duplicate
    case delete
    case group
    case ungroup
    case toggleVisibility
    case toggleLock
}

/// 동작의 검증·실행을 맡는 쪽(사이드바 controller).
@MainActor
protocol SidebarOutlineActionHandling: AnyObject {
    func sidebarOutline(_ outline: SidebarOutlineView, canPerform action: SidebarOutlineAction) -> Bool
    func sidebarOutline(_ outline: SidebarOutlineView, perform action: SidebarOutlineAction)
}

/// 사이드바 키보드 단축키(디자인 가이드 8.1).
///
/// 메뉴 막대 구성(XII.6.5, 열린 결정 O8)이 생기기 전까지는 목록이 first responder일 때만 여기서 처리합니다.
/// 메뉴 막대에 같은 항목이 생기면 그 메뉴가 아래 responder action(`delete(_:)` 등)으로 연결하고
/// 이 표는 메뉴 key equivalent의 원본으로 옮깁니다.
enum SidebarKeyboardShortcuts {
    struct Shortcut: Equatable {
        /// `charactersIgnoringModifiers`의 소문자 값.
        let key: String
        let modifiers: NSEvent.ModifierFlags
    }

    /// ⌘D: Finder의 Duplicate와 같은 표준 의미.
    static let duplicate = Shortcut(key: "d", modifiers: [.command])
    /// ⌥⌘G·⇧⌥⌘G: ⌘G(Find Next) 표준 의미를 지키기 위해 Apple 앱(Keynote·Pages)의 Group 단축키를 따릅니다.
    static let group = Shortcut(key: "g", modifiers: [.command, .option])
    static let ungroup = Shortcut(key: "g", modifiers: [.command, .option, .shift])
    /// ⇧⌘H·⇧⌘L: 디자인 도구 관례. 메뉴 막대 결정(O8) 전까지 잠정값입니다.
    static let toggleVisibility = Shortcut(key: "h", modifiers: [.command, .shift])
    static let toggleLock = Shortcut(key: "l", modifiers: [.command, .shift])

    static func shortcut(for action: SidebarOutlineAction) -> Shortcut? {
        switch action {
        case .duplicate: return duplicate
        case .group: return group
        case .ungroup: return ungroup
        case .toggleVisibility: return toggleVisibility
        case .toggleLock: return toggleLock
        case .rename, .delete: return nil
        }
    }

    /// 키 이벤트에 대응하는 동작. `plainKeys`가 `true`면 수식키 없는 Return·Delete도 봅니다.
    static func action(for event: NSEvent, plainKeys: Bool) -> SidebarOutlineAction? {
        let modifiers = event.modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting([.capsLock, .numericPad, .function])
        if plainKeys, modifiers.isEmpty {
            switch event.specialKey {
            case .carriageReturn?, .enter?, .newline?: return .rename
            case .delete?, .deleteForward?, .backspace?: return .delete
            default: break
            }
        }
        guard modifiers.contains(.command), let key = event.charactersIgnoringModifiers?.lowercased() else {
            return nil
        }
        let pressed = Shortcut(key: key, modifiers: modifiers)
        return SidebarOutlineAction.allCases.first { shortcut(for: $0) == pressed }
    }

    /// 메뉴 막대 항목에 key equivalent를 표시합니다(표시용, 실제 처리는 목록 또는 메뉴 막대).
    /// 문맥 메뉴에는 쓰지 않습니다(HIG Context menus).
    static func apply(_ action: SidebarOutlineAction, to item: NSMenuItem) {
        switch action {
        case .delete:
            item.keyEquivalent = "\u{8}"
            item.keyEquivalentModifierMask = []
        case .rename:
            item.keyEquivalent = "\r"
            item.keyEquivalentModifierMask = []
        default:
            guard let shortcut = shortcut(for: action) else { return }
            item.keyEquivalent = shortcut.key
            item.keyEquivalentModifierMask = shortcut.modifiers
        }
    }
}

/// 사이드바용 `NSOutlineView`.
///
/// 화살표 이동·펼침, type-select, 다중 선택, 드래그는 NSOutlineView 기본 동작을 그대로 씁니다
/// (HIG Outline views, 디자인 가이드 6.2). 이 하위 클래스는 Return(이름 변경), Delete(삭제)와
/// 앱 동작 단축키만 더하고, 메뉴 막대가 연결할 responder action을 제공합니다.
final class SidebarOutlineView: NSOutlineView {
    weak var actionHandler: SidebarOutlineActionHandling?

    override func keyDown(with event: NSEvent) {
        if let action = SidebarKeyboardShortcuts.action(for: event, plainKeys: true), run(action) {
            return
        }
        super.keyDown(with: event)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // key equivalent는 포커스와 무관하게 뷰 계층 전체에 전달되므로 목록이 first responder일 때만 처리합니다.
        if window?.firstResponder === self,
           let action = SidebarKeyboardShortcuts.action(for: event, plainKeys: false),
           run(action) {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    @discardableResult
    private func run(_ action: SidebarOutlineAction) -> Bool {
        guard let handler = actionHandler, handler.sidebarOutline(self, canPerform: action) else {
            return false
        }
        handler.sidebarOutline(self, perform: action)
        return true
    }

    // MARK: 메뉴 막대용 responder action (target 없는 first responder action, 가이드 8.2)

    @objc func delete(_ sender: Any?) { run(.delete) }
    @objc func duplicate(_ sender: Any?) { run(.duplicate) }
    @objc func renameSidebarItem(_ sender: Any?) { run(.rename) }
    @objc func groupSidebarLayers(_ sender: Any?) { run(.group) }
    @objc func ungroupSidebarLayers(_ sender: Any?) { run(.ungroup) }
    @objc func toggleSidebarLayerVisibility(_ sender: Any?) { run(.toggleVisibility) }
    @objc func toggleSidebarLayerLock(_ sender: Any?) { run(.toggleLock) }

    private static let actionSelectors: [Selector: SidebarOutlineAction] = [
        #selector(delete(_:)): .delete,
        #selector(duplicate(_:)): .duplicate,
        #selector(renameSidebarItem(_:)): .rename,
        #selector(groupSidebarLayers(_:)): .group,
        #selector(ungroupSidebarLayers(_:)): .ungroup,
        #selector(toggleSidebarLayerVisibility(_:)): .toggleVisibility,
        #selector(toggleSidebarLayerLock(_:)): .toggleLock,
    ]

    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if let selector = item.action, let action = Self.actionSelectors[selector] {
            return actionHandler?.sidebarOutline(self, canPerform: action) ?? false
        }
        return super.validateUserInterfaceItem(item)
    }
}
