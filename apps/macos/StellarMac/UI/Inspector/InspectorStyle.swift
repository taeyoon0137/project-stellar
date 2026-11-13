import AppKit

/// 인스펙터 공통 간격·크기 상수(가이드 4.4: 패널 바깥 여백, 행 간격, 컨트롤 묶음 간격을 각각 상수 하나로).
///
/// 값은 인스펙터 구현 후보값이며 통합 시 가이드 12장 "확정 수치 기록"에 근거와 함께 옮깁니다.
/// 컨트롤 높이·행 높이는 두지 않고 `controlSize`와 Auto Layout에 맡깁니다.
enum InspectorMetrics {
    /// 패널 바깥 여백.
    static let panelInset: CGFloat = 12
    /// 패널 안 행 간격.
    static let rowSpacing: CGFloat = 6
    /// 한 행 안 컨트롤 묶음 간격.
    static let groupSpacing: CGFloat = 4
    /// 행 레이블 열 너비.
    static let labelColumnWidth: CGFloat = 88
    /// 수치·텍스트 필드 선호 최소 너비(좁은 폭에서는 잘리기 전에 양보).
    static let minimumFieldWidth: CGFloat = 36
    /// 밀도 높은 인스펙터 컨트롤 크기(가이드 4.4 small·regular 중 small).
    static let controlSize: NSControl.ControlSize = .small
    /// 검사·미리보기에 쓰는 대표 인스펙터 폭.
    static let typicalWidth: CGFloat = 300
}

/// 인스펙터 글꼴. pt 값을 직접 쓰지 않고 시스템 글꼴 크기 메서드로 지정합니다(가이드 4.1).
enum InspectorFonts {
    /// 행 레이블·컨트롤 내용.
    static var control: NSFont { .systemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize)) }
    /// 수치 필드: 자리 정렬을 위해 고정폭 숫자(가이드 열린 결정 O7에 대한 제안).
    static var number: NSFont {
        .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize), weight: .regular)
    }
    /// 섹션 제목.
    static var sectionTitle: NSFont { .boldSystemFont(ofSize: NSFont.systemFontSize(for: InspectorMetrics.controlSize)) }
    /// 선택 요약 제목.
    static var headline: NSFont { .preferredFont(forTextStyle: .headline) }
    /// 보조 설명.
    static var caption: NSFont { .preferredFont(forTextStyle: .caption1) }
}

/// 인스펙터가 쓰는 SF Symbols 이름. 이미지 asset은 쓰지 않습니다(가이드 5.4).
enum InspectorSymbols {
    // 상태
    static let noSelection = "cursorarrow.rays"
    static let failure = "exclamationmark.triangle"
    static let locked = "lock.fill"
    static let readOnly = "pencil.slash"
    static let issue = "exclamationmark.triangle.fill"
    static let mixedSelection = "square.stack.3d.up"

    // node 종류
    static let frame = "rectangle.dashed"
    static let fragment = "square.on.square.dashed"
    static let rectangle = "rectangle"
    static let ellipse = "circle"
    static let line = "line.diagonal"
    static let polygon = "pentagon"
    static let path = "scribble.variable"
    static let text = "textformat"
    static let image = "photo"
    static let componentInstance = "diamond"

    // 명령
    static let add = "plus"
    static let remove = "minus"
    static let flipHorizontal = "arrow.left.and.right"
    static let flipVertical = "arrow.up.and.down"
    static let alignStart = "text.alignleft"
    static let alignCenter = "text.aligncenter"
    static let alignEnd = "text.alignright"
    static let alignJustify = "text.justify"

    // Value Binding 표시(placeholder)
    static let bindingFixed = "circle.dashed"
    static let bindingToken = "link.circle.fill"
    static let bindingComponentProperty = "arrow.down.right.circle.fill"
    static let bindingExpression = "function"
    static let bindingOrphaned = "exclamationmark.circle.fill"

    /// node 종류 아이콘.
    static func node(_ kind: InspectorNodeKind, shape: InspectorShapeKind?) -> String {
        switch kind {
        case .frame: return frame
        case .fragment: return fragment
        case .shape:
            switch shape ?? .rectangle {
            case .rectangle: return rectangle
            case .ellipse: return ellipse
            case .line: return line
            case .polygon: return polygon
            }
        case .path: return path
        case .text: return text
        case .image: return image
        case .componentInstance: return componentInstance
        }
    }

    /// modifier 종류 아이콘.
    static func modifier(_ kind: InspectorModifierKind) -> String {
        switch kind {
        case .transform: return "arrow.up.and.down.and.arrow.left.and.right"
        case .boolean: return "square.on.circle"
        case .strokeExpand: return "lineweight"
        case .corner: return "rectangle.roundedtop"
        case .mask: return "theatermasks"
        case .clip: return "scissors"
        case .fill: return "drop.fill"
        case .stroke: return "pencil.line"
        case .shadow: return "shadow"
        case .blur: return "aqi.medium"
        case .blend: return "circle.lefthalf.filled"
        case .opacity: return "circle.dotted"
        }
    }

    /// binding 상태 아이콘.
    static func binding(_ binding: InspectorBinding) -> String {
        switch binding {
        case .fixed: return bindingFixed
        case .token: return bindingToken
        case .componentProperty: return bindingComponentProperty
        case .expression: return bindingExpression
        case .orphaned: return bindingOrphaned
        }
    }

    /// 인스펙터가 쓰는 모든 이름(검사용).
    static var all: [String] {
        var names = [
            noSelection, failure, locked, readOnly, issue, mixedSelection,
            add, remove, flipHorizontal, flipVertical, alignStart, alignCenter, alignEnd, alignJustify,
            bindingFixed, bindingToken, bindingComponentProperty, bindingExpression, bindingOrphaned,
        ]
        for kind in InspectorNodeKind.allCases {
            names.append(node(kind, shape: nil))
        }
        for shape in InspectorShapeKind.allCases {
            names.append(node(.shape, shape: shape))
        }
        names += InspectorModifierKind.allCases.map(modifier)
        return Array(Set(names)).sorted()
    }

    /// 접근성 설명을 붙인 심볼 이미지.
    static func image(_ name: String, label: String) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: label)
    }
}
