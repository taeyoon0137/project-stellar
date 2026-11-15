import AppKit

/// 속성 패널 간격·크기 상수.
enum InspectorMetrics {
    /// 패널 바깥 여백.
    static let panelInset: CGFloat = 12
    /// 행 간격.
    static let rowSpacing: CGFloat = 4
    /// 섹션 간격.
    static let sectionSpacing: CGFloat = 10
    /// 행 레이블 열 너비. 가장 긴 CSS 이름(`border-bottom-right-radius`)이 320pt 패널에서 잘리지 않는 폭입니다.
    static let labelColumnWidth: CGFloat = 150
    /// 값 필드 최소 너비.
    static let minimumFieldWidth: CGFloat = 60
    /// 컨트롤 크기.
    static let controlSize: NSControl.ControlSize = .small
}

/// 속성 패널 글꼴. 레이블은 시스템 글꼴(고정폭 아님, UX 명세 6.3)입니다.
enum InspectorFonts {
    static var label: NSFont { .systemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize)) }
    static var value: NSFont {
        .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize), weight: .regular)
    }
    static var sectionTitle: NSFont { .boldSystemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize)) }
    static var headline: NSFont { .preferredFont(forTextStyle: .headline) }
    static var caption: NSFont { .preferredFont(forTextStyle: .caption1) }
}

/// 속성 패널 SF Symbols.
enum InspectorSymbols {
    static let noSelection = "cursorarrow.rays"
    static let mixedSelection = "square.stack.3d.up"

    /// 선택 분류 하나의 글리프.
    static func node(_ classes: InspectorNodeClasses) -> String {
        switch classes {
        case .frame: return "rectangle.dashed"
        case .rectangle: return "rectangle"
        case .ellipse: return "circle"
        case .line: return "line.diagonal"
        case .polygon: return "pentagon"
        case .text: return "textformat"
        case .image: return "photo"
        default: return "square.on.square.dashed"
        }
    }

    /// 선택 분류 하나의 이름.
    static func kindName(_ classes: InspectorNodeClasses) -> String {
        switch classes {
        case .frame: return String(localized: "Frame")
        case .rectangle: return String(localized: "Rectangle")
        case .ellipse: return String(localized: "Ellipse")
        case .line: return String(localized: "Line")
        case .polygon: return String(localized: "Polygon")
        case .text: return String(localized: "Text")
        case .image: return String(localized: "Image")
        default: return String(localized: "Layer")
        }
    }
}
