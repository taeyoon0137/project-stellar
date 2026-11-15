import AppKit

// 속성 패널 표시 모델(ADR-0070).
//
// 코어(`stellar_window_inspector`)가 만든 행을 그대로 담습니다. 이름은 CSS longhand 이름, 값은 코어가 정규화한
// CSS 값 텍스트이고, 셸은 값을 해석하지 않습니다.

/// 도형 종류. 저장 tag와 같은 값입니다(캔버스 도형 생성 요청에 씀).
enum InspectorShapeKind: UInt8, CaseIterable {
    case rectangle = 1
    case ellipse = 2
    case line = 3
    case polygon = 4
}

/// 문서 색 값(core-types `ColorSpace` tag와 같은 `space`, 비연관 알파 RGBA).
struct InspectorColor: Equatable, Sendable {
    enum Space: UInt8, Sendable {
        case srgb = 1
        case displayP3 = 2
    }

    var space: Space
    var red: Double
    var green: Double
    var blue: Double
    var alpha: Double
}

extension InspectorColor {
    /// 표시용 NSColor(색공간 유지).
    var nsColor: NSColor {
        switch space {
        case .srgb: return NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha)
        case .displayP3: return NSColor(displayP3Red: red, green: green, blue: blue, alpha: alpha)
        }
    }

    /// 색 패널에서 고른 색. Display P3 색은 P3로, 나머지는 sRGB로 옮깁니다. 성분으로 바꿀 수 없으면 nil.
    init?(_ color: NSColor) {
        if color.type == .componentBased,
           color.colorSpace == .displayP3 || color.colorSpace.cgColorSpace?.name == CGColorSpace.extendedDisplayP3 {
            self.init(space: .displayP3, red: Double(color.redComponent), green: Double(color.greenComponent),
                      blue: Double(color.blueComponent), alpha: Double(color.alphaComponent))
            return
        }
        guard let srgb = color.usingColorSpace(.sRGB) else { return nil }
        self.init(space: .srgb, red: Double(srgb.redComponent), green: Double(srgb.greenComponent),
                  blue: Double(srgb.blueComponent), alpha: Double(srgb.alphaComponent))
    }

    /// 코어에 넘길 CSS 색 텍스트. 정규화는 코어가 다시 합니다.
    var cssText: String {
        func channel(_ value: Double) -> String { String(Float(value)) }
        let space = self.space == .displayP3 ? "display-p3" : "srgb"
        let body = "\(channel(red)) \(channel(green)) \(channel(blue))"
        return alpha >= 1 ? "color(\(space) \(body))" : "color(\(space) \(body) / \(channel(alpha)))"
    }

    /// 코어가 쓴 정규 색 텍스트(`#rrggbb[aa]`, `color(srgb|display-p3 r g b [/ a])`)를 견본 표시용으로 읽습니다.
    /// 해석 결과는 표시에만 쓰고 문서에 쓰지 않습니다.
    init?(css text: String) {
        if text.hasPrefix("#") {
            let hex = Array(text.dropFirst())
            guard hex.count == 6 || hex.count == 8 else { return nil }
            func byte(_ index: Int) -> Double? {
                UInt8(String(hex[index..<index + 2]), radix: 16).map { Double($0) / 255 }
            }
            guard let r = byte(0), let g = byte(2), let b = byte(4) else { return nil }
            self.init(space: .srgb, red: r, green: g, blue: b, alpha: hex.count == 8 ? (byte(6) ?? 1) : 1)
            return
        }
        guard text.hasPrefix("color("), text.hasSuffix(")") else { return nil }
        let body = text.dropFirst(6).dropLast()
        let parts = body.replacingOccurrences(of: "/", with: " ").split(separator: " ").map(String.init)
        guard parts.count == 4 || parts.count == 5,
              let r = Double(parts[1]), let g = Double(parts[2]), let b = Double(parts[3]) else { return nil }
        let alpha = parts.count == 5 ? (Double(parts[4]) ?? 1) : 1
        self.init(space: parts[0] == "display-p3" ? .displayP3 : .srgb, red: r, green: g, blue: b, alpha: alpha)
    }
}

/// 코어 행 flag(`STELLAR_INSPECTOR_*`).
struct InspectorRowFlags: OptionSet, Equatable {
    let rawValue: UInt32
    static let mixed = InspectorRowFlags(rawValue: 1 << 0)
    static let readOnly = InspectorRowFlags(rawValue: 1 << 1)
    static let color = InspectorRowFlags(rawValue: 1 << 2)
    static let group = InspectorRowFlags(rawValue: 1 << 3)
    static let longhand = InspectorRowFlags(rawValue: 1 << 4)
    static let stellarOnly = InspectorRowFlags(rawValue: 1 << 5)
    static let numeric = InspectorRowFlags(rawValue: 1 << 6)
    static let keyword = InspectorRowFlags(rawValue: 1 << 7)
    static let bool = InspectorRowFlags(rawValue: 1 << 8)
}

/// 섹션(속성 모델 12장 CSS 범주). 숫자 순서가 표시 순서이며 코어 값과 같습니다.
enum InspectorSectionKind: UInt32, CaseIterable {
    case layer = 1
    case position = 2
    case size = 3
    case overflow = 4
    case typography = 5
    case background = 6
    case fillStroke = 7
    case border = 8
    case kind = 9
}

/// 선택에 있는 node 분류(코어 `NodeClass` bit).
struct InspectorNodeClasses: OptionSet, Equatable, Hashable {
    let rawValue: UInt32
    static let frame = InspectorNodeClasses(rawValue: 1 << 0)
    static let rectangle = InspectorNodeClasses(rawValue: 1 << 1)
    static let ellipse = InspectorNodeClasses(rawValue: 1 << 2)
    static let line = InspectorNodeClasses(rawValue: 1 << 3)
    static let polygon = InspectorNodeClasses(rawValue: 1 << 4)
    static let text = InspectorNodeClasses(rawValue: 1 << 5)
    static let image = InspectorNodeClasses(rawValue: 1 << 6)
    static let other = InspectorNodeClasses(rawValue: 1 << 7)
}

/// 속성 행 하나.
struct InspectorRowState: Equatable {
    /// CSS longhand 이름(코어와 주고받는 이름).
    var name: String
    /// 저장 위치 PropertyKey(묶음 행은 0).
    var key: UInt32
    var flags: InspectorRowFlags
    var section: InspectorSectionKind
    /// 정규 값 텍스트(혼합이면 빈 문자열).
    var value: String
    /// keyword 선택지.
    var choices: [String]

    /// 행 구성을 바꾸는 부분(값 제외). 같으면 값만 제자리에서 갱신합니다.
    var structure: String { "\(section.rawValue):\(name):\(flags.subtracting([.mixed]).rawValue)" }
}

/// 확정 선택 전체의 속성 snapshot.
struct InspectorSnapshot: Equatable {
    var count: Int
    var classes: InspectorNodeClasses
    var displayName: String
    var revision: UInt64
    var historySequence: UInt64
    var view: UUID
    var rows: [InspectorRowState]

    static let empty = InspectorSnapshot(count: 0, classes: [], displayName: "", revision: 0, historySequence: 0,
                                         view: UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)), rows: [])

    /// 섹션·행 구성 키(값 제외).
    var structure: [String] { ["\(classes.rawValue)"] + rows.map(\.structure) }
}

/// 편집 입력.
enum InspectorInput: Equatable {
    /// CSS 값 텍스트로 설정.
    case text(String)
    /// 각 node 현재 값에 더함(px·deg·개수, 혼합 값의 화살표·scrub).
    case adjust(Double)
}

/// 편집 단계(코어 Undo 단위).
///
/// - 레이블 scrub(포인터 제스처)은 `continuous` 여러 개 뒤 `ended`(또는 `cancel`) 하나가 Undo 한 항목입니다. 코어는
///   그동안 preview를 열고, `cancel`은 표시 상태와 무관하게 preview를 닫습니다.
/// - 화살표·색 견본·색 패널은 `repeated`입니다. 누를 때마다 바로 적용되고, 같은 속성을 0.5초 안에 이어 바꾸면
///   코어가 Undo 한 항목으로 합칩니다(preview 없음, 그 사이 Undo·선택이 막히지 않음).
enum InspectorEditPhase: UInt32 {
    case commit = 0
    case continuous = 1
    case ended = 2
    case cancel = 3
    case repeated = 4
}

/// 편집 요청이 받아들여지지 않은 이유.
struct InspectorEditFailure: Error {
    let status: StellarStatus
    /// 코어 진단(`code: 이유`). 상태 오류면 빈 문자열.
    let diagnostic: String
}
