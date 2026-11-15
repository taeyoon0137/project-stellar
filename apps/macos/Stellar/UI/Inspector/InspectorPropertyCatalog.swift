import Foundation

/// 속성 패널 표시 문구(섹션 제목, 속성 설명). 어느 행이 있는지와 값은 코어가 정하고, 여기에는 표시 문구만 둡니다.
enum InspectorPropertyCatalog {
    /// 섹션 제목(속성 모델 12장 CSS 범주 이름).
    static func title(_ section: InspectorSectionKind) -> String {
        switch section {
        case .layer: return String(localized: "Layer")
        case .position: return String(localized: "Position")
        case .size: return String(localized: "Size")
        case .overflow: return String(localized: "Overflow")
        case .typography: return String(localized: "Typography")
        case .background: return String(localized: "Background")
        case .fillStroke: return String(localized: "Fill & Stroke")
        case .border: return String(localized: "Border")
        case .kind: return String(localized: "Shape")
        }
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
