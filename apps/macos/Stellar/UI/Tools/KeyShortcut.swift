import AppKit

/// 메뉴 단축키 수식키 집합(XII.6.5).
///
/// `NSEvent.ModifierFlags`는 Hashable이 아니므로 충돌 비교용으로 네 수식키만 따로 둡니다.
struct ShortcutModifiers: OptionSet, Hashable {
    let rawValue: UInt8

    static let control = ShortcutModifiers(rawValue: 1 << 0)
    static let option = ShortcutModifiers(rawValue: 1 << 1)
    static let shift = ShortcutModifiers(rawValue: 1 << 2)
    static let command = ShortcutModifiers(rawValue: 1 << 3)

    /// AppKit 메뉴 항목에 넣을 수식키.
    var eventFlags: NSEvent.ModifierFlags {
        var flags: NSEvent.ModifierFlags = []
        if contains(.control) { flags.insert(.control) }
        if contains(.option) { flags.insert(.option) }
        if contains(.shift) { flags.insert(.shift) }
        if contains(.command) { flags.insert(.command) }
        return flags
    }

    /// HIG가 정한 표기 순서(Control, Option, Shift, Command)의 기호 문자열.
    var symbols: String {
        var text = ""
        if contains(.control) { text += "⌃" }
        if contains(.option) { text += "⌥" }
        if contains(.shift) { text += "⇧" }
        if contains(.command) { text += "⌘" }
        return text
    }
}

/// 메뉴 key equivalent 하나(키 + 수식키).
///
/// `key`는 `NSMenuItem.keyEquivalent`에 그대로 넣는 문자입니다. 글자는 소문자로 두고 Shift는
/// `modifiers`로 표시합니다. 두 글자 키의 윗글자(+, {, }, |, *, ?)는 HIG Keyboards 규칙대로 Shift 없이
/// 그 글자 자체로 씁니다(예: Help는 ⇧⌘/ 가 아니라 ⌘?).
struct KeyShortcut: Hashable {
    let key: String
    let modifiers: ShortcutModifiers

    init(_ key: String, _ modifiers: ShortcutModifiers) {
        self.key = key
        self.modifiers = modifiers
    }

    /// Command만 쓰는 단축키.
    static func command(_ key: String) -> KeyShortcut {
        KeyShortcut(key, [.command])
    }

    /// 메뉴 표시와 같은 형식의 문자열(예: "⌥⇧⌘G", "⌘+").
    var displayString: String {
        modifiers.symbols + Self.displayName(for: key)
    }

    /// 미국 ANSI 배열의 물리 키와 실제로 눌리는 수식키로 정규화한 값(충돌 비교용).
    ///
    /// 예: ⌘+ → ⇧⌘=, ⌘{ → ⇧⌘[, ⌘G(대문자) → ⇧⌘g.
    var normalized: KeyShortcut {
        if let base = Self.shiftedSymbolBase[key] {
            return KeyShortcut(base, modifiers.union(.shift))
        }
        let lower = key.lowercased()
        if lower != key {
            return KeyShortcut(lower, modifiers.union(.shift))
        }
        return self
    }

    /// 미국 ANSI 배열에서 Shift로 입력하는 윗글자 → 아랫글자.
    private static let shiftedSymbolBase: [String: String] = [
        "+": "=", "_": "-", "{": "[", "}": "]", "|": "\\", ":": ";", "\"": "'",
        "<": ",", ">": ".", "?": "/", "~": "`", "!": "1", "@": "2", "#": "3",
        "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0",
    ]

    private static func displayName(for key: String) -> String {
        switch key {
        case " ": return "Space"
        case "\t": return "Tab"
        case "\u{1b}": return "Esc"
        case "\u{8}": return "⌫"
        default: return key.uppercased()
        }
    }
}
