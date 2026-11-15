import AppKit

/// 캔버스 도구 집합(툴바 도구 그룹·Insert/View 메뉴·캔버스 단일 키가 공유).
///
/// 선언 순서가 하단 도구 막대의 순서입니다(10종). 지금은 현재 도구와 도구별 입력 해석(클릭 기본 크기, Shift·Option,
/// 핸들 상자·반경 계산)을 셸이 갖고 있고, 생성·이동·크기 결과만 코어 command로 보냅니다. 이 해석을 코어 도구 상태
/// 기계(XII.3.6, XII.10.1)로 옮기는 일은 UX WBS UX.3.8에서 합니다.
enum CanvasTool: String, CaseIterable {
    case move
    case frame
    case rectangle
    case ellipse
    case line
    case polygon
    case text
    case image
    case hand
    case zoom

    /// 툴바 항목 레이블과 접근성 레이블.
    var label: String {
        switch self {
        case .move: return "Move"
        case .frame: return "Frame"
        case .rectangle: return "Rectangle"
        case .ellipse: return "Ellipse"
        case .line: return "Line"
        case .polygon: return "Polygon"
        case .text: return "Text"
        case .image: return "Image"
        case .hand: return "Hand"
        case .zoom: return "Zoom"
        }
    }

    /// SF Symbol 이름(이미지 에셋을 쓰지 않음).
    var symbolName: String {
        switch self {
        case .move: return "cursorarrow"
        case .frame: return "number"
        case .rectangle: return "rectangle"
        case .ellipse: return "circle"
        case .line: return "line.diagonal"
        case .polygon: return "pentagon"
        case .text: return "textformat"
        case .image: return "photo"
        case .hand: return "hand.raised"
        case .zoom: return "magnifyingglass"
        }
    }

    /// 캔버스 포커스 중 수식키 없이 누르는 단일 키(소문자). 없으면 nil.
    ///
    /// 메뉴 key equivalent로 두지 않고 캔버스 뷰 `keyDown(with:)`에서만 해석합니다(디자인 가이드 8.2).
    var singleKey: Character? {
        switch self {
        case .move: return "v"
        case .frame: return "f"
        case .rectangle: return "r"
        case .ellipse: return "o"
        case .line: return "l"
        case .polygon: return "p"
        case .text: return "t"
        case .image: return nil
        case .hand: return "h"
        case .zoom: return "z"
        }
    }

    /// 이 도구를 고르는 셸 명령.
    var command: ShellCommand {
        switch self {
        case .move: return .selectMoveTool
        case .frame: return .selectFrameTool
        case .rectangle: return .selectRectangleTool
        case .ellipse: return .selectEllipseTool
        case .line: return .selectLineTool
        case .polygon: return .selectPolygonTool
        case .text: return .selectTextTool
        case .image: return .selectImageTool
        case .hand: return .selectHandTool
        case .zoom: return .selectZoomTool
        }
    }

    /// 도움말 태그(툴바 tooltip). 단일 키가 있으면 그 키를, 없으면 메뉴 단축키(예: Image ⇧⌘K)를 함께 보여 줍니다.
    var helpTag: String {
        if let key = singleKey { return "\(label) (\(String(key).uppercased()))" }
        if let shortcut = command.shortcut { return "\(label) (\(shortcut.displayString))" }
        return label
    }

    /// 도구 막대에서 이 도구 앞에 그룹 간격을 두는지. 그룹은 선택(Move) · 컨테이너(Frame) ·
    /// 도형(Rectangle·Ellipse·Line·Polygon) · 내용(Text·Image) · 시점(Hand·Zoom)입니다(UX 명세 5.1).
    var startsGroup: Bool {
        switch self {
        case .frame, .rectangle, .text, .hand: return true
        case .move, .ellipse, .line, .polygon, .image, .zoom: return false
        }
    }

    /// 캔버스 접근성 도움말: 단일 키 목록을 `singleKey`에서 만들어 실제 배정과 늘 같습니다. 목록 구분자는 "또는" 목록
    /// 형식이 정하고(문자열을 이어 붙이지 않음), 언어는 문장과 같게 앱이 실제로 쓰는 현지화를 따릅니다.
    static var shortcutHelp: String {
        let locale = Locale(identifier: Bundle.main.preferredLocalizations.first ?? "en")
        let list = allCases.compactMap(\.singleKey).map { String($0).uppercased() }
            .formatted(.list(type: .or).locale(locale))
        return String(localized: "Press \(list) to choose a tool.", comment: "Accessibility help of the canvas; the argument lists tool keys")
    }
}

/// 캔버스 단일 키 도구 단축키 해석기(디자인 가이드 8.2).
///
/// 캔버스 호스트의 `keyDown(with:)`가 호출합니다. 텍스트 컨트롤이 first responder이면 캔버스가 키를 받지
/// 않으므로 발동하지 않습니다. 수식키(Caps Lock 제외)가 있거나 키 반복이면 nil입니다.
/// 한글 등 비라틴 입력 소스에서는 문자가 라틴 글자가 아니므로 미국 ANSI 물리 키 코드로 대신 판정합니다.
enum CanvasToolShortcut {
    /// 미국 ANSI 배열 가상 키 코드(Carbon `kVK_ANSI_*`와 같은 값) → 글자.
    private static let ansiKeyCodes: [UInt16: Character] = [
        0x09: "v", 0x03: "f", 0x0F: "r", 0x1F: "o", 0x25: "l", 0x23: "p",
        0x11: "t", 0x04: "h", 0x06: "z",
    ]

    /// 키 이벤트 값으로 도구를 해석합니다.
    static func tool(
        characters: String?,
        keyCode: UInt16,
        modifierFlags: NSEvent.ModifierFlags,
        isARepeat: Bool
    ) -> CanvasTool? {
        guard !isARepeat else { return nil }
        let significant = modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting([.capsLock, .numericPad, .function])
        guard significant.isEmpty else { return nil }

        let letter: Character?
        if let text = characters?.lowercased(), text.count == 1, let first = text.first, first.isASCII {
            letter = first.isLetter ? first : nil
        } else {
            // 비라틴 입력 소스: 물리 키 위치로 판정합니다.
            letter = ansiKeyCodes[keyCode]
        }
        guard let letter else { return nil }
        return CanvasTool.allCases.first { $0.singleKey == letter }
    }

    /// `NSEvent` 편의 함수. keyDown 이외 이벤트는 nil입니다.
    static func tool(for event: NSEvent) -> CanvasTool? {
        guard event.type == .keyDown else { return nil }
        return tool(
            characters: event.charactersIgnoringModifiers,
            keyCode: event.keyCode,
            modifierFlags: event.modifierFlags,
            isARepeat: event.isARepeat
        )
    }
}
