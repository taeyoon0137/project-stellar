import Foundation

// 인스펙터가 표시하는 값 snapshot 타입.
//
// 모든 타입은 Rust 타입을 포함하지 않는 plain value입니다. 코어 연결 시 FFI adapter가
// pull 이벤트(`stellar_runtime_poll_event`)로 받은 선택 snapshot을 이 타입으로 옮기고,
// 셸의 편집은 `InspectorCommand`로 코어에 보냅니다(아키텍처 §8.2, 가이드 9장).
// 숫자 tag가 있는 enum은 문서 모델의 저장 tag와 같은 값을 씁니다(직접 매핑용).

// MARK: - 식별자와 종류

/// 인스펙터가 편집하는 속성 하나의 식별자.
///
/// `path`는 셸과 코어 adapter가 공유하는 안정 문자열 경로입니다(예: `frame.size.width`).
/// `coreKey`는 문서 모델에 `PropertyKey` 숫자 값이 이미 정해진 속성에만 채웁니다
/// (예: Frame size = `0x0101`, Fill color modifier = `0x1071`). 표시 이름은 식별에 쓰지 않습니다.
struct InspectorPropertyID: Hashable {
    let path: String
    let coreKey: UInt32?

    init(_ path: String, coreKey: UInt32? = nil) {
        self.path = path
        self.coreKey = coreKey
    }
}

/// scene node 종류. 숫자 값은 문서 모델 `NodeKind` 저장 tag와 같습니다.
enum InspectorNodeKind: UInt16, CaseIterable {
    case frame = 1
    case fragment = 2
    case shape = 3
    case path = 4
    case text = 5
    case image = 6
    case componentInstance = 7

    /// 화면 표시 이름.
    var title: String {
        switch self {
        case .frame: return "Frame"
        case .fragment: return "Fragment"
        case .shape: return "Shape"
        case .path: return "Path"
        case .text: return "Text"
        case .image: return "Image"
        case .componentInstance: return "Component Instance"
        }
    }
}

/// 파라메트릭 도형 종류. 숫자 값은 문서 모델 `ShapeKind` 저장 tag와 같습니다.
enum InspectorShapeKind: UInt8, CaseIterable {
    case rectangle = 1
    case ellipse = 2
    case line = 3
    case polygon = 4

    /// 화면 표시 이름.
    var title: String {
        switch self {
        case .rectangle: return "Rectangle"
        case .ellipse: return "Ellipse"
        case .line: return "Line"
        case .polygon: return "Polygon"
        }
    }
}

/// modifier 종류. 숫자 값은 문서 모델 `ModifierKind` 저장 tag와 같습니다.
enum InspectorModifierKind: UInt16, CaseIterable {
    case transform = 1
    case boolean = 2
    case strokeExpand = 3
    case corner = 4
    case mask = 5
    case clip = 6
    case fill = 7
    case stroke = 8
    case shadow = 9
    case blur = 10
    case blend = 11
    case opacity = 12

    /// 화면 표시 이름.
    var title: String {
        switch self {
        case .transform: return "Transform"
        case .boolean: return "Boolean"
        case .strokeExpand: return "Stroke Expand"
        case .corner: return "Corner"
        case .mask: return "Mask"
        case .clip: return "Clip"
        case .fill: return "Fill"
        case .stroke: return "Stroke"
        case .shadow: return "Shadow"
        case .blur: return "Blur"
        case .blend: return "Blend"
        case .opacity: return "Opacity"
        }
    }
}

// MARK: - 값

/// 문서 색 값. 앱 크롬 색이 아니라 사용자 디자인 콘텐츠의 색입니다(가이드 4.2).
/// `space` 숫자 값은 core-types `ColorSpace` tag와 같습니다. 채널은 비연관 알파 RGBA입니다.
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

/// 필드 값. 다중 선택에서 값이 서로 다르면 `.mixed`입니다.
enum InspectorValue: Equatable {
    /// 값 없음(표시할 값이 없는 읽기 전용 항목 등).
    case none
    /// 다중 선택에서 선택 대상의 값이 서로 다름.
    case mixed
    /// 수치(모델 단위). 표시는 `InspectorNumberSpec.displayScale`을 곱합니다.
    case number(Double)
    case bool(Bool)
    case color(InspectorColor)
    /// 선택지 id(`InspectorChoice.id`).
    case choice(String)
    case text(String)
}

/// 수치 표시 단위. 표시 전용이며 값 의미는 코어가 정합니다.
enum InspectorUnit: String {
    case none = ""
    case pixels = "px"
    case degrees = "°"
    case percent = "%"
    case em = "em"
    case multiplier = "×"
    case milliseconds = "ms"

    /// VoiceOver용 단위 이름.
    var spokenName: String {
        switch self {
        case .none: return ""
        case .pixels: return "pixels"
        case .degrees: return "degrees"
        case .percent: return "percent"
        case .em: return "em"
        case .multiplier: return "times"
        case .milliseconds: return "milliseconds"
        }
    }
}

/// 수치 필드 규격.
struct InspectorNumberSpec: Equatable {
    var unit: InspectorUnit
    /// 모델 단위 하한(없으면 제한 없음). 최종 검증은 코어가 합니다.
    var minimum: Double?
    /// 모델 단위 상한.
    var maximum: Double?
    /// 화살표 키·stepper 한 단계(모델 단위). Shift는 10배입니다.
    var step: Double
    /// 표시 소수 자릿수.
    var fractionDigits: Int
    /// 표시 값 = 모델 값 × displayScale(예: 불투명도 0..1 → 0..100%).
    var displayScale: Double
    /// 단일 수치 행에 stepper를 함께 보일지.
    var showsStepper: Bool

    init(
        unit: InspectorUnit,
        minimum: Double? = nil,
        maximum: Double? = nil,
        step: Double = 1,
        fractionDigits: Int = 2,
        displayScale: Double = 1,
        showsStepper: Bool = false
    ) {
        self.unit = unit
        self.minimum = minimum
        self.maximum = maximum
        self.step = step
        self.fractionDigits = fractionDigits
        self.displayScale = displayScale
        self.showsStepper = showsStepper
    }

    /// 모델 값을 범위 안으로 맞춥니다.
    func clamped(_ value: Double) -> Double {
        var result = value
        if let minimum { result = max(minimum, result) }
        if let maximum { result = min(maximum, result) }
        return result
    }

    /// 입력한 표시 값을 표시 자릿수(`fractionDigits`)로 반올림(0.5는 0에서 먼 쪽)하고 모델 단위로 바꿔
    /// 범위 안으로 맞춥니다. 정수 필드(`fractionDigits == 0`, 예: 다각형 변 수, ms)는 정수만 보냅니다.
    /// 화면에 보이는 값과 코어로 보내는 값이 같아야 합니다.
    func modelValue(fromDisplay display: Double) -> Double {
        let factor = pow(10, Double(max(fractionDigits, 0)))
        let rounded = (display * factor).rounded(.toNearestOrAwayFromZero) / factor
        return clamped(rounded / displayScale)
    }

    /// 모델 값을 표시 자릿수 격자에 맞춥니다(화살표 키·stepper 누적 오차 제거).
    func normalized(_ model: Double) -> Double {
        modelValue(fromDisplay: model * displayScale)
    }
}

/// popup·segmented 선택지. `id`는 코어 enum 이름(저장 tag와 1:1)입니다.
struct InspectorChoice: Equatable {
    var id: String
    var title: String
    /// 아이콘 이름(SF Symbols). segmented에서 아이콘만 보일 때도 `title`은 접근성 레이블로 씁니다.
    var symbolName: String?

    init(_ id: String, _ title: String, symbol: String? = nil) {
        self.id = id
        self.title = title
        self.symbolName = symbol
    }
}

/// 필드를 표시할 네이티브 컨트롤 종류.
enum InspectorControl: Equatable {
    /// `NSTextField` + `NumberFormatter`(+ 단위 레이블, 선택적 `NSStepper`).
    case number(InspectorNumberSpec)
    /// `NSSwitch`.
    case toggle
    /// `NSColorWell`.
    case color
    /// `NSPopUpButton`.
    case popup([InspectorChoice])
    /// `NSSegmentedControl`(선택 하나).
    case segmented([InspectorChoice])
    /// 한 줄 `NSTextField`.
    case text(placeholder: String)
    /// 선택 가능한 읽기 전용 레이블.
    case readOnly
    /// 명령 버튼(`NSButton`). 아이콘만 있으면 `title`은 비우고 필드 `label`을 접근성 레이블로 씁니다.
    /// `destructive`는 되돌리기 어려운 파괴 변환(Path로 변환, Detach)이며 `hasDestructiveAction`으로
    /// 표시합니다(가이드 5.1).
    case action(title: String, symbolName: String?, destructive: Bool = false)
}

/// Value Binding 상태(아키텍처 §14.2). 지금은 표시용 placeholder이며 편집 UI는 후속 작업입니다.
enum InspectorBinding: Equatable {
    /// 고정값.
    case fixed
    /// 토큰에 연결됨.
    case token(name: String)
    /// Component Property에 연결됨.
    case componentProperty(name: String)
    /// 조건식·연산식.
    case expression(summary: String)
    /// 연결이 끊어짐(아키텍처 §14.4).
    case orphaned(reason: String)

    /// VoiceOver·툴팁용 설명.
    var summary: String {
        switch self {
        case .fixed: return "Fixed value"
        case .token(let name): return "Token \(name)"
        case .componentProperty(let name): return "Component property \(name)"
        case .expression(let summary): return "Expression \(summary)"
        case .orphaned(let reason): return "Broken binding: \(reason)"
        }
    }

    /// 고정값이 아닌 연결 상태인지.
    var isBound: Bool {
        if case .fixed = self { return false }
        return true
    }
}

// MARK: - 필드·행·섹션

/// 속성 하나의 표시 상태.
struct InspectorField: Equatable {
    var id: InspectorPropertyID
    /// 전체 이름(접근성 레이블·툴팁). 예: "Width".
    var label: String
    /// 행 안의 짧은 표시. 예: "W". 없으면 표시하지 않습니다.
    var shortLabel: String?
    var control: InspectorControl
    var value: InspectorValue
    var isEditable: Bool
    /// Value Binding 상태. binding 대상이 아닌 항목(명령 버튼·읽기 전용)은 `nil`입니다.
    var binding: InspectorBinding?
}

/// 인스펙터 한 줄: 행 레이블과 하나 이상의 필드.
struct InspectorRow: Equatable {
    /// 섹션 안에서 유일한 id.
    var id: String
    /// 행 레이블. 같은 묶음의 이어지는 줄이면 빈 문자열입니다.
    var label: String
    var fields: [InspectorField]
    /// 미해결 참조·누락 리소스 같은 진단(아키텍처 §14.4, III.2.4). 없으면 `nil`.
    var issue: String?

    init(id: String, label: String, fields: [InspectorField], issue: String? = nil) {
        self.id = id
        self.label = label
        self.fields = fields
        self.issue = issue
    }
}

/// 인스펙터 섹션 식별자. 접힘 상태는 이 id로 기억합니다.
enum InspectorSectionID: String, CaseIterable {
    case layer
    case fragment
    case frameLayout
    case frameAppearance
    case shape
    case path
    case textContent
    case textStyle
    case paragraph
    case image
    case component
    case componentProperties
    case componentSlots
    case modifiers
}

/// 속성 섹션.
struct InspectorSection: Equatable {
    var id: InspectorSectionID
    var title: String
    var rows: [InspectorRow]
    /// 섹션 아래 보조 설명(예: Fragment 제한 안내). 없으면 `nil`.
    var note: String?

    init(id: InspectorSectionID, title: String, rows: [InspectorRow], note: String? = nil) {
        self.id = id
        self.title = title
        self.rows = rows
        self.note = note
    }
}

// MARK: - Modifier stack

/// modifier stack 항목 하나(문서 모델 `ModifierEntry`).
struct InspectorModifierItem: Equatable {
    /// `ModifierId`(StableId) 문자열 표현.
    var id: String
    var kind: InspectorModifierKind
    var isEnabled: Bool
    /// 목록 행에 보일 한 줄 요약(코어가 만든 표시 문자열).
    var summary: String
    /// 선택한 항목 아래에 보일 파라미터 행.
    var parameters: [InspectorRow]
}

/// modifier stack을 편집할 수 있는지.
enum InspectorModifierAvailability: Equatable {
    case available
    /// 편집할 수 없는 이유(예: 다중 선택).
    case unavailable(reason: String)
}

/// 선택 대상의 modifier stack snapshot. 순서는 `OrderKey` 오름차순입니다.
struct InspectorModifierStack: Equatable {
    var availability: InspectorModifierAvailability
    var items: [InspectorModifierItem]
    /// 추가 메뉴에 보일 종류.
    var addableKinds: [InspectorModifierKind]
    /// 추가·삭제·재정렬·활성 전환 가능 여부(잠금·읽기 전용이면 false).
    var isEditable: Bool
}

// MARK: - 선택과 전체 snapshot

/// 선택 요약.
struct InspectorSelectionSummary: Equatable {
    /// 선택한 node 수.
    var count: Int
    /// 선택에 포함된 서로 다른 node 종류(저장 tag 오름차순).
    var nodeKinds: [InspectorNodeKind]
    /// 모든 선택이 같은 도형 종류일 때의 종류.
    var shapeKind: InspectorShapeKind?
    /// 단일 선택이면 레이어 표시 이름. 다중 선택이면 `nil`.
    var displayName: String?

    static let empty = InspectorSelectionSummary(count: 0, nodeKinds: [], shapeKind: nil, displayName: nil)
}

/// 선택 전체의 편집 가능 상태.
enum InspectorEditability: Equatable {
    case editable
    /// 선택한 레이어가 잠겨 있음. 잠금 해제 외 속성 편집을 막습니다.
    case locked
    /// 프로젝트·라이브러리 상태 때문에 읽기 전용(예: 신뢰 전 제한 모드, 외부 라이브러리 원본).
    case readOnly(reason: String)
}

/// 인스펙터 전체 상태.
enum InspectorContentState: Equatable {
    /// 선택 없음.
    case noSelection
    /// 코어가 선택 snapshot을 준비 중(예: 비활성 청크 materialization).
    case loading
    /// snapshot을 만들지 못함.
    case failed(message: String)
    /// `sections`를 표시.
    case ready
}

/// 코어에서 셸로 전달하는 인스펙터 snapshot 하나.
struct InspectorSnapshot: Equatable {
    /// 단조 증가 세대 번호. 현재보다 오래된 snapshot은 버립니다(가이드 9장).
    var generation: UInt64
    /// 선택 집합 식별 토큰. 코어가 선택이 바뀔 때마다 새 값을 발급합니다. 같은 이름의 다른 레이어를
    /// 골라도 바뀌므로, 셸은 이 값이 바뀌면 진행 중인 편집을 이전 세대로 확정하고 새 snapshot을 반영합니다.
    var selectionToken: UInt64
    var state: InspectorContentState
    var selection: InspectorSelectionSummary
    var editability: InspectorEditability
    var sections: [InspectorSection]
    /// modifier stack. Fragment처럼 modifier를 가질 수 없는 선택이면 `nil`.
    var modifiers: InspectorModifierStack?

    /// 섹션이 없는 상태 snapshot(선택 없음·로딩·오류).
    static func placeholder(
        _ state: InspectorContentState, generation: UInt64, selectionToken: UInt64 = 0
    ) -> InspectorSnapshot {
        InspectorSnapshot(
            generation: generation,
            selectionToken: selectionToken,
            state: state,
            selection: .empty,
            editability: .editable,
            sections: [],
            modifiers: nil
        )
    }

    /// id로 필드를 찾습니다(섹션 → modifier 파라미터 순).
    func field(_ id: InspectorPropertyID) -> InspectorField? {
        for section in sections {
            for row in section.rows {
                if let field = row.fields.first(where: { $0.id == id }) { return field }
            }
        }
        for item in modifiers?.items ?? [] {
            for row in item.parameters {
                if let field = row.fields.first(where: { $0.id == id }) { return field }
            }
        }
        return nil
    }
}
