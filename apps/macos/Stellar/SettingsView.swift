import AppKit
import SwiftUI

/// 새 도형의 기본 채우기·선 색(사용자 결정 D7, 해석은 ADR-0071 UX.1.5 5항).
///
/// 사용자 앱 설정(`UserDefaults`)에만 저장하고 프로젝트 파일에는 쓰지 않습니다. 새 Rectangle·Ellipse·Polygon은
/// `fill`만, Line은 `stroke`와 1px 두께만 씁니다(닫힌 도형에 기본 선을 넣지 않음). 셸이 생성 요청에 스타일을 넘기며
/// core는 기본 색을 정하지 않습니다.
struct CanvasShapeDefaults: Equatable {
    static let fillKey = "canvas.defaultShapeFill"
    static let strokeKey = "canvas.defaultLineStroke"
    /// 초기 채우기 #D9D9D9.
    static let initialFill = InspectorColor(space: .srgb, red: 217.0 / 255, green: 217.0 / 255, blue: 217.0 / 255, alpha: 1)
    /// 초기 선 #000000.
    static let initialStroke = InspectorColor(space: .srgb, red: 0, green: 0, blue: 0, alpha: 1)
    /// 선 두께(논리 픽셀).
    static let strokeWidth: Float = 1

    var fill: InspectorColor
    var stroke: InspectorColor

    static func load(_ defaults: UserDefaults = .standard) -> CanvasShapeDefaults {
        CanvasShapeDefaults(fill: color(defaults.array(forKey: fillKey)) ?? initialFill,
                            stroke: color(defaults.array(forKey: strokeKey)) ?? initialStroke)
    }

    func save(_ defaults: UserDefaults = .standard) {
        defaults.set(Self.components(fill), forKey: Self.fillKey)
        defaults.set(Self.components(stroke), forKey: Self.strokeKey)
    }

    /// 코어 생성 명령에 넘길 칠. 색이 nil이면 그 modifier를 만들지 않습니다(ADR-0071 UX.1.5).
    struct Paint: Equatable {
        var fill: InspectorColor?
        var stroke: InspectorColor?
        var strokeWidth: Double
    }

    /// Rectangle·Ellipse·Polygon은 채우기, Line은 선 색 1px를 씁니다.
    static func paint(for kind: InspectorShapeKind, defaults: UserDefaults = .standard) -> Paint {
        let values = load(defaults)
        if kind == .line {
            return Paint(fill: nil, stroke: values.stroke, strokeWidth: Double(strokeWidth))
        }
        return Paint(fill: values.fill, stroke: nil, strokeWidth: 0)
    }

    private static func components(_ color: InspectorColor) -> [Double] {
        [Double(color.space.rawValue), color.red, color.green, color.blue, color.alpha]
    }

    private static func color(_ stored: [Any]?) -> InspectorColor? {
        guard let values = stored as? [Double], values.count == 5, values.allSatisfy(\.isFinite),
              let space = InspectorColor.Space(rawValue: UInt8(exactly: values[0]) ?? 0),
              values[1...].allSatisfy({ (0...1).contains($0) }) else { return nil }
        return InspectorColor(space: space, red: values[1], green: values[2], blue: values[3], alpha: values[4])
    }
}

/// 앱 설정 화면(SwiftUI, Settings ▸ Canvas).
///
/// 캔버스는 AppKit view로 두고 설정 화면만 SwiftUI로 둡니다. 지금은 캔버스의 새 도형 기본 색만 있습니다.
struct SettingsView: View {
    @State private var shapes = CanvasShapeDefaults.load()

    var body: some View {
        Form {
            Section(String(localized: "Canvas", comment: "Settings section")) {
                ColorPicker(String(localized: "Default Shape Fill", comment: "Settings: fill of new rectangles, ellipses and polygons"),
                            selection: binding(\.fill), supportsOpacity: true)
                ColorPicker(String(localized: "Default Line Tool Color", comment: "Settings: stroke color of new lines drawn with the Line tool (1px)"),
                            selection: binding(\.stroke), supportsOpacity: true)
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
    }

    private func binding(_ path: WritableKeyPath<CanvasShapeDefaults, InspectorColor>) -> Binding<Color> {
        Binding(
            get: { Color(nsColor: shapes[keyPath: path].nsColor) },
            set: { value in
                guard let color = InspectorColor(NSColor(value)) else { return }
                shapes[keyPath: path] = color
                shapes.save()
            }
        )
    }
}
