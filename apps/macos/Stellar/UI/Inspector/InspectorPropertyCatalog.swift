import Foundation

/// 속성 패널 표시 문구(섹션 제목, 속성 설명). 어느 행이 있는지와 값은 코어가 정하고, 여기에는 표시 문구만 둡니다.
enum InspectorPropertyCatalog {
    /// 섹션 제목(속성 모델 12장 CSS 범주 이름). 종류별 섹션은 선택한 노드 종류 이름입니다(UX 명세 6.2: Polygon 변 수,
    /// Line 끝점, Image `object-fit`). 종류별 행은 선택한 노드가 모두 같은 종류일 때만 나옵니다.
    static func title(_ section: InspectorSectionKind, classes: InspectorNodeClasses) -> String {
        switch section {
        case .layer: return String(localized: "Layer")
        case .position: return String(localized: "Position")
        case .size: return String(localized: "Size")
        case .overflow: return String(localized: "Overflow")
        case .typography: return String(localized: "Typography")
        case .background: return String(localized: "Background")
        case .fillStroke: return String(localized: "Fill & Stroke")
        case .border: return String(localized: "Border")
        case .kind: return InspectorSymbols.kindName(classes)
        }
    }

    /// 속성의 쉬운 설명(툴팁). 없는 이름은 빈 문자열.
    static func summary(_ name: String) -> String {
        switch name {
        case "name": return String(localized: "Layer name shown in Layers.")
        case "visible": return String(localized: "Whether the layer is drawn. Hidden layers export as display: none.")
        case "locked": return String(localized: "Locked layers can’t be selected on the canvas or edited.")
        case "left": return String(localized: "Horizontal position from the parent’s left edge.")
        case "top": return String(localized: "Vertical position from the parent’s top edge.")
        case "width": return String(localized: "The element’s width.")
        case "height": return String(localized: "The element’s height.")
        case "overflow": return String(localized: "Whether children outside the frame are clipped.")
        case "color": return String(localized: "Text color.")
        case "background-color": return String(localized: "Frame background color. none is transparent.")
        case "fill": return String(localized: "Shape fill color. none removes the fill.")
        case "stroke": return String(localized: "Outline color. none removes the outline.")
        case "stroke-width": return String(localized: "Outline thickness.")
        case "border": return String(localized: "Border width, style, and color on all four sides.")
        case "border-width": return String(localized: "Border thickness on all four sides.")
        case "border-color": return String(localized: "Border color on all four sides. none removes the border.")
        case "border-radius": return String(localized: "Corner radius. One to four values: top-left, top-right, bottom-right, bottom-left.")
        case "border-top-left-radius": return String(localized: "Top-left corner radius.")
        case "border-top-right-radius": return String(localized: "Top-right corner radius.")
        case "border-bottom-right-radius": return String(localized: "Bottom-right corner radius.")
        case "border-bottom-left-radius": return String(localized: "Bottom-left corner radius.")
        case "x1", "y1": return String(localized: "Line start point in the line’s own coordinates.")
        case "x2", "y2": return String(localized: "Line end point in the line’s own coordinates.")
        case "polygon-sides": return String(localized: "Number of polygon sides (3–1000).")
        case "polygon-rotation": return String(localized: "Rotation of the polygon’s points inside its box.")
        case "polygon-corner-radius": return String(localized: "Rounding of the polygon’s corners.")
        case "object-fit": return String(localized: "How the image fills its box.")
        default: return ""
        }
    }

    /// 툴팁·접근성 도움말: 쉬운 설명, CSS longhand 이름, 변환 진단(UX 명세 6.3). 진단은 코어 행 flag에서 옵니다.
    /// 묶음 행은 코어가 그 행 바로 뒤에 넘긴 longhand 행 이름(`longhands`)으로 설명합니다.
    static func help(for row: InspectorRowState, longhands: [String] = []) -> String {
        var lines = [summary(row.name)].filter { !$0.isEmpty }
        if row.flags.contains(.group) {
            let names = ListFormatter.localizedString(byJoining: longhands)
            lines.append(names.isEmpty ? row.name : String(localized: "Shorthand for \(names)."))
        } else {
            lines.append(row.name)
        }
        if row.flags.contains(.stellarOnly) {
            lines.append(String(localized: "Stellar only: not exported as CSS."))
        }
        if row.flags.contains(.numeric) {
            lines.append(String(localized: "Drag the label or press ↑↓ (Shift ×10) to change."))
        }
        return lines.joined(separator: "\n")
    }

    /// 묶음 행 `index` 바로 뒤에 코어가 넘긴 longhand 행 이름들.
    static func longhands(after index: Int, in rows: [InspectorRowState]) -> [String] {
        rows.dropFirst(index + 1).prefix { $0.flags.contains(.longhand) }.map(\.name)
    }
}

/// 노드 종류별로 접은 섹션(보는 사람의 편의 설정, 문서에 저장하지 않음).
enum InspectorCollapseMemory {
    private static let key = "StellarInspectorCollapsedSections"

    static func collapsed(for classes: InspectorNodeClasses) -> Set<InspectorSectionKind> {
        let stored = UserDefaults.standard.dictionary(forKey: key)?["\(classes.rawValue)"] as? [Int] ?? []
        return Set(stored.compactMap { InspectorSectionKind(rawValue: UInt32($0)) })
    }

    static func set(_ sections: Set<InspectorSectionKind>, for classes: InspectorNodeClasses) {
        var all = UserDefaults.standard.dictionary(forKey: key) ?? [:]
        all["\(classes.rawValue)"] = sections.map { Int($0.rawValue) }.sorted()
        UserDefaults.standard.set(all, forKey: key)
    }
}
