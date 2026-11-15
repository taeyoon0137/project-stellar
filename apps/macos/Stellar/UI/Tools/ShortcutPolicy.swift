import Foundation

/// 메뉴 단축키 충돌·관례 감사(XII.6.5, 디자인 가이드 8.1).
///
/// 기준은 Apple HIG Keyboards의 표준 단축키 표와 사용자 정의 단축키 규칙입니다.
/// - 시스템 전역 단축키는 쓰지 않습니다.
/// - 표준 단축키는 원래 의미의 명령에만 씁니다.
/// - 사용자 정의 단축키는 Command를 주 수식키로 쓰고 Control은 피합니다.
/// - 관련 없는 기존 단축키에 수식키만 더한 단축키를 새로 만들지 않습니다.
enum ShortcutPolicy {
    /// 감사 대상 하나.
    struct Binding {
        let command: ShellCommand
        let shortcut: KeyShortcut
        /// 메뉴에 보이지 않는 보조 key equivalent.
        let isHiddenAlternate: Bool
    }

    /// HIG 표준 단축키 하나.
    struct Standard {
        let meaning: String
        /// 이 단축키를 써도 되는 명령. 비어 있으면 Stellar가 쓰지 않는 예약 단축키입니다.
        let allowed: Set<ShellCommand>
    }

    /// 모든 `ShellCommand`의 메뉴 단축키 목록.
    static var allBindings: [Binding] {
        ShellCommand.allCases.flatMap { command -> [Binding] in
            var bindings: [Binding] = []
            if let shortcut = command.shortcut {
                bindings.append(Binding(command: command, shortcut: shortcut, isHiddenAlternate: false))
            }
            for shortcut in command.hiddenAlternateShortcuts {
                bindings.append(Binding(command: command, shortcut: shortcut, isHiddenAlternate: true))
            }
            return bindings
        }
    }

    /// 시스템 기능이 쓰는 단축키(앱이 쓰지 않음). HIG Keyboards 표준 표의 시스템 항목과,
    /// 표 밖의 macOS 시스템 단축키(⇧⌘5 스크린샷, ⌃⌘Q 화면 잠금)입니다.
    static let systemReserved: [KeyShortcut: String] = {
        let entries: [(KeyShortcut, String)] = [
            (.command(" "), "Spotlight"),
            (KeyShortcut(" ", [.shift, .command]), "시스템 지정(Varies)"),
            (KeyShortcut(" ", [.option, .command]), "Spotlight 결과 창"),
            (KeyShortcut(" ", [.control, .command]), "Special Characters"),
            (.command("\t"), "앱 전환"),
            (KeyShortcut("\t", [.shift, .command]), "앱 역순 전환"),
            (KeyShortcut("\u{1b}", [.option, .command]), "강제 종료"),
            (.command("`"), "앱 안 다음 창"),
            (KeyShortcut("`", [.shift, .command]), "앱 안 이전 창"),
            (KeyShortcut("`", [.option, .command]), "창 서랍 포커스"),
            (KeyShortcut("-", [.option, .command]), "화면 확대/축소: 축소"),
            (KeyShortcut("=", [.option, .command]), "화면 확대/축소: 확대"),
            (KeyShortcut("3", [.shift, .command]), "화면 캡처(파일)"),
            (KeyShortcut("3", [.control, .shift, .command]), "화면 캡처(클립보드)"),
            (KeyShortcut("4", [.shift, .command]), "선택 영역 캡처(파일)"),
            (KeyShortcut("4", [.control, .shift, .command]), "선택 영역 캡처(클립보드)"),
            (KeyShortcut("5", [.shift, .command]), "스크린샷 도구(HIG 표 밖)"),
            (KeyShortcut("8", [.option, .command]), "화면 확대/축소 켜기"),
            (KeyShortcut("8", [.control, .option, .command]), "색상 반전"),
            (KeyShortcut("d", [.option, .command]), "Dock 보기/숨기기"),
            (KeyShortcut("d", [.control, .command]), "사전 정의 보기"),
            (KeyShortcut("q", [.shift, .command]), "로그아웃"),
            (KeyShortcut("q", [.option, .shift, .command]), "확인 없이 로그아웃"),
            (KeyShortcut("q", [.control, .command]), "화면 잠금(HIG 표 밖)"),
            (KeyShortcut(",", [.control, .option, .command]), "화면 대비 낮추기"),
            (KeyShortcut(".", [.control, .option, .command]), "화면 대비 높이기"),
            (KeyShortcut("/", [.option, .command]), "글꼴 다듬기"),
        ]
        return Dictionary(uniqueKeysWithValues: entries.map { ($0.0.normalized, $0.1) })
    }()

    /// 앱 표준 단축키와 허용 명령(HIG Keyboards 표준 표).
    static let standard: [KeyShortcut: Standard] = {
        let entries: [(KeyShortcut, String, Set<ShellCommand>)] = [
            (.command("a"), "Select All", [.selectAll]),
            (KeyShortcut("a", [.shift, .command]), "Deselect All", []),
            (.command("b"), "Bold", []),
            (.command("c"), "Copy", [.copy]),
            (KeyShortcut("c", [.shift, .command]), "Colors 창", []),
            (KeyShortcut("c", [.option, .command]), "Copy Style", []),
            (KeyShortcut("c", [.control, .command]), "서식 복사", []),
            (.command("e"), "Use Selection for Find", []),
            (.command("f"), "Find", []),
            (KeyShortcut("f", [.option, .command]), "검색 필드로 이동", []),
            (KeyShortcut("f", [.control, .command]), "Enter Full Screen", [.toggleFullScreen]),
            (.command("g"), "Find Next", []),
            (KeyShortcut("g", [.shift, .command]), "Find Previous", []),
            (.command("h"), "Hide 앱", [.hideApp]),
            (KeyShortcut("h", [.option, .command]), "Hide Others", [.hideOthers]),
            (.command("i"), "Italic / Get Info", []),
            (KeyShortcut("i", [.option, .command]), "Inspector", [.toggleInspector]),
            (.command("j"), "Scroll to Selection", []),
            (.command("m"), "Minimize", [.minimize]),
            (KeyShortcut("m", [.option, .command]), "Minimize All", [.minimizeAll]),
            (.command("n"), "New", [.newProject]),
            (.command("o"), "Open", [.openProject]),
            (.command("p"), "Print", []),
            (KeyShortcut("p", [.shift, .command]), "Page Setup", []),
            (.command("q"), "Quit", [.quit]),
            (.command("s"), "Save", [.saveNow]),
            (KeyShortcut("s", [.shift, .command]), "Duplicate / Save As", []),
            (.command("t"), "Fonts 창", []),
            (KeyShortcut("t", [.option, .command]), "Show/Hide Toolbar", []),
            (.command("u"), "Underline", []),
            (.command("v"), "Paste", [.paste]),
            (KeyShortcut("v", [.shift, .command]), "Paste as", []),
            (KeyShortcut("v", [.option, .command]), "Apply Style", []),
            (KeyShortcut("v", [.option, .shift, .command]), "Paste and Match Style", []),
            (KeyShortcut("v", [.control, .command]), "서식 붙이기", []),
            (.command("w"), "Close", [.closeWindow]),
            (KeyShortcut("w", [.shift, .command]), "Close File(관련 창 모두)", [.closeProject]),
            (KeyShortcut("w", [.option, .command]), "Close All", []),
            (.command("x"), "Cut", [.cut]),
            (.command("z"), "Undo", [.undo]),
            (KeyShortcut("z", [.shift, .command]), "Redo", [.redo]),
            (.command("-"), "선택 크기 줄이기", [.zoomOut]),
            (.command("+"), "선택 크기 키우기", [.zoomIn]),
            (.command("{"), "왼쪽 정렬", []),
            (.command("}"), "오른쪽 정렬", []),
            (.command("|"), "가운데 정렬", []),
            (.command(":"), "Spelling 창", []),
            (.command(";"), "맞춤법 오류 찾기", []),
            (.command(","), "Settings", [.settings]),
            (.command("."), "작업 취소", []),
            (.command("?"), "Help", []),
        ]
        return Dictionary(uniqueKeysWithValues: entries.map {
            ($0.0.normalized, Standard(meaning: $0.1, allowed: $0.2))
        })
    }()

    /// Control을 쓰는 사용자 정의 단축키의 예외와 근거.
    static let controlModifierExceptions: [ShellCommand: String] = [
        .toggleSidebar: "AppKit 기본 메인 메뉴 템플릿의 Show Sidebar(⌃⌘S) 관례",
    ]

    /// 관련 없는 기존 단축키에 수식키를 더한 형태를 의도적으로 쓰는 예외와 근거.
    static let derivedModifierExceptions: [ShellCommand: String] = [
        .toggleSidebar: "AppKit 기본 메인 메뉴 템플릿의 Show Sidebar(⌃⌘S) 관례",
    ]

    /// Shift·Option 변형이 서로 관련된 명령 묶음(같은 묶음 안의 파생은 허용).
    static let relatedFamilies: [Set<ShellCommand>] = [
        [.undo, .redo],
        [.closeWindow, .closeProject],
        [.minimize, .minimizeAll],
        [.hideApp, .hideOthers],
        [.zoomIn],
    ]

    /// 감사 결과 문제 목록. 비어 있으면 통과입니다.
    static func audit(_ bindings: [Binding] = allBindings) -> [String] {
        var issues: [String] = []
        var owners: [KeyShortcut: ShellCommand] = [:]

        // 1) 같은 물리 키 조합을 서로 다른 명령이 쓰면 충돌입니다.
        for binding in bindings {
            let key = binding.shortcut.normalized
            if let existing = owners[key], existing != binding.command {
                issues.append("충돌: \(binding.shortcut.displayString) — \(existing.rawValue) vs \(binding.command.rawValue)")
            } else {
                owners[key] = binding.command
            }
        }

        for binding in bindings {
            let shortcut = binding.shortcut
            let key = shortcut.normalized
            let name = "\(binding.command.rawValue) \(shortcut.displayString)"

            // 2) 시스템 전역 단축키 금지.
            if let meaning = systemReserved[key] {
                issues.append("시스템 단축키 사용: \(name) (\(meaning))")
                continue
            }

            // 3) 표준 단축키는 지정된 의미의 명령만.
            if let standard = standard[key] {
                if !standard.allowed.contains(binding.command) {
                    issues.append("표준 단축키 재사용: \(name) (원래 의미 \(standard.meaning))")
                }
                continue
            }

            // 4) 사용자 정의: Command 필수, Control은 예외만.
            if !shortcut.modifiers.contains(.command) {
                issues.append("Command 없는 메뉴 단축키: \(name)")
            }
            if shortcut.modifiers.contains(.control), controlModifierExceptions[binding.command] == nil {
                issues.append("Control 수식키 사용: \(name)")
            }

            // 5) 관련 없는 기존 단축키에 수식키만 더한 파생 금지(예외 목록 제외).
            if derivedModifierExceptions[binding.command] == nil {
                for base in reducedVariants(of: key) {
                    if let other = owners[base], other != binding.command, !related(binding.command, other) {
                        issues.append("파생 단축키: \(name) ← \(base.displayString)(\(other.rawValue))")
                    } else if owners[base] == nil, let standard = standard[base],
                              !standard.allowed.contains(binding.command),
                              !standard.allowed.contains(where: { related(binding.command, $0) }) {
                        issues.append("파생 단축키: \(name) ← 표준 \(base.displayString)(\(standard.meaning))")
                    }
                }
            }
        }
        return issues
    }

    /// Shift·Option·Control을 하나 이상 뺀 조합(Command는 유지).
    private static func reducedVariants(of shortcut: KeyShortcut) -> [KeyShortcut] {
        let extras: [ShortcutModifiers] = [.shift, .option, .control].filter { shortcut.modifiers.contains($0) }
        guard !extras.isEmpty else { return [] }
        var variants: [KeyShortcut] = []
        // 공집합이 아닌 모든 부분집합을 뺍니다.
        for mask in 1..<(1 << extras.count) {
            var modifiers = shortcut.modifiers
            for (index, extra) in extras.enumerated() where mask & (1 << index) != 0 {
                modifiers.remove(extra)
            }
            variants.append(KeyShortcut(shortcut.key, modifiers))
        }
        return variants
    }

    private static func related(_ lhs: ShellCommand, _ rhs: ShellCommand) -> Bool {
        relatedFamilies.contains { $0.contains(lhs) && $0.contains(rhs) }
    }
}
