import Foundation

// 문서 모델 속성 → 인스펙터 컨트롤 대응표(코드 원본).
//
// crates/document의 payload(frame.rs, shape.rs, path.rs, text.rs, image.rs, modifier.rs,
// component/*)와 metadata registry(frame_metadata.rs, modifier_metadata.rs)를 읽어 만든 표입니다.
// 섹션·행 배치와 컨트롤 종류만 정하고, 값·편집 가능 여부·binding 상태는 adapter(지금은 Stub)가
// resolver로 채웁니다. 대응표 문서: docs/design/ui/inspector.md "속성 → 컨트롤 대응표".

// MARK: - 템플릿

/// 필드 템플릿: 속성 id·레이블·컨트롤.
struct InspectorFieldTemplate {
    let id: InspectorPropertyID
    let label: String
    let shortLabel: String?
    let control: InspectorControl
    /// Value Binding 대상인지(명령 버튼·구조 정보는 false).
    let bindable: Bool

    init(
        _ path: String,
        coreKey: UInt32? = nil,
        label: String,
        short: String? = nil,
        control: InspectorControl,
        bindable: Bool = true
    ) {
        self.id = InspectorPropertyID(path, coreKey: coreKey)
        self.label = label
        self.shortLabel = short
        self.control = control
        self.bindable = bindable
    }
}

/// adapter가 채우는 필드 상태.
struct InspectorFieldState {
    var value: InspectorValue
    var isEditable: Bool = true
    var binding: InspectorBinding = .fixed
    /// 템플릿 컨트롤 대신 쓸 컨트롤(동적 popup 선택지, 모드에 따라 바뀌는 수치 규격 등).
    var control: InspectorControl? = nil
}

/// 템플릿에 대한 상태를 돌려주는 resolver.
typealias InspectorFieldResolver = (InspectorFieldTemplate) -> InspectorFieldState

/// 행 템플릿.
struct InspectorRowTemplate {
    let id: String
    let label: String
    let fields: [InspectorFieldTemplate]

    func make(_ resolve: InspectorFieldResolver, issue: String? = nil) -> InspectorRow {
        InspectorRow(
            id: id,
            label: label,
            fields: fields.map { template in
                let state = resolve(template)
                return InspectorField(
                    id: template.id,
                    label: template.label,
                    shortLabel: template.shortLabel,
                    control: state.control ?? template.control,
                    value: state.value,
                    isEditable: state.isEditable,
                    binding: template.bindable ? state.binding : nil
                )
            },
            issue: issue
        )
    }
}

/// 섹션 템플릿.
struct InspectorSectionTemplate {
    let id: InspectorSectionID
    let title: String
    let rows: [InspectorRowTemplate]
    let note: String?

    init(_ id: InspectorSectionID, _ title: String, rows: [InspectorRowTemplate], note: String? = nil) {
        self.id = id
        self.title = title
        self.rows = rows
        self.note = note
    }

    /// `issues`: 행 id → 진단 메시지.
    func make(_ resolve: InspectorFieldResolver, issues: [String: String] = [:]) -> InspectorSection {
        InspectorSection(
            id: id,
            title: title,
            rows: rows.map { $0.make(resolve, issue: issues[$0.id]) },
            note: note
        )
    }
}

// MARK: - 수치 규격

extension InspectorNumberSpec {
    /// 길이(논리 픽셀, 부호 있음).
    static let length = InspectorNumberSpec(unit: .pixels)
    /// 비음수 길이(size, radius, 두께).
    static let nonNegativeLength = InspectorNumberSpec(unit: .pixels, minimum: 0)
    /// 비음수 길이 + stepper(단일 행).
    static let nonNegativeLengthStepper = InspectorNumberSpec(unit: .pixels, minimum: 0, showsStepper: true)
    /// 각도(도).
    static let angle = InspectorNumberSpec(unit: .degrees, fractionDigits: 1, showsStepper: true)
    /// 0..1 값을 %로 표시(불투명도).
    static let unitPercent = InspectorNumberSpec(
        unit: .percent, minimum: 0, maximum: 1, step: 0.01, fractionDigits: 0, displayScale: 100, showsStepper: true
    )
    /// 0..1 정규화 좌표를 %로 표시(crop).
    static let normalizedPercent = InspectorNumberSpec(
        unit: .percent, minimum: 0, maximum: 1, step: 0.01, fractionDigits: 1, displayScale: 100
    )
    /// 배율(%, transform modifier scale).
    static let scalePercent = InspectorNumberSpec(unit: .percent, step: 0.01, fractionDigits: 1, displayScale: 100)
    /// 정다각형 변 수(VI.5.5: 3..=1000).
    static let polygonSides = InspectorNumberSpec(
        unit: .none, minimum: 3, maximum: 1000, step: 1, fractionDigits: 0, showsStepper: true
    )
    /// 자간(em, VI.7.3: -1..=10).
    static let letterSpacing = InspectorNumberSpec(
        unit: .em, minimum: -1, maximum: 10, step: 0.01, fractionDigits: 2, showsStepper: true
    )
    /// 줄 높이 배수(VI.7.3: 0 < 값 ≤ 10).
    static let lineHeightMultiplier = InspectorNumberSpec(
        unit: .multiplier, minimum: 0.01, maximum: 10, step: 0.1, fractionDigits: 2
    )
    /// 고정 줄 높이(VI.7.3: 0 < 값 ≤ 10000 px).
    static let lineHeightFixed = InspectorNumberSpec(unit: .pixels, minimum: 0.01, maximum: 10_000, step: 1)
    /// 단위 없는 수(Component Number Property).
    static let plain = InspectorNumberSpec(unit: .none)
    /// 지속 시간(ms, Component Duration Property).
    static let duration = InspectorNumberSpec(unit: .milliseconds, minimum: 0, step: 10, fractionDigits: 0)
}

// MARK: - 카탈로그

/// 문서 모델 속성의 인스펙터 표현.
enum InspectorPropertyCatalog {
    // MARK: 공통 레이어(NodeRecord 공통 필드, VI.2.2)

    static let layerName = InspectorFieldTemplate(
        InspectorPropertyID.layerName.path, label: "Name", control: .text(placeholder: "Layer name"), bindable: false
    )
    static let layerVisible = InspectorFieldTemplate(
        InspectorPropertyID.layerVisible.path, label: "Visible", control: .toggle
    )
    static let layerLocked = InspectorFieldTemplate(
        InspectorPropertyID.layerLocked.path, label: "Locked", control: .toggle, bindable: false
    )

    static let layerSection = InspectorSectionTemplate(.layer, "Layer", rows: [
        InspectorRowTemplate(id: "name", label: "Name", fields: [layerName]),
        InspectorRowTemplate(id: "visible", label: "Visible", fields: [layerVisible]),
        InspectorRowTemplate(id: "locked", label: "Locked", fields: [layerLocked]),
    ])

    // MARK: Fragment(VI.4)

    static let fragmentSection = InspectorSectionTemplate(
        .fragment, "Fragment",
        rows: [
            InspectorRowTemplate(id: "convert", label: "", fields: [
                InspectorFieldTemplate(
                    "fragment.convertToFrame", label: "Convert to Frame",
                    control: .action(title: "Convert to Frame", symbolName: nil), bindable: false
                ),
            ]),
        ],
        note: "Fragments group layers without their own layout, transform, or effects. Convert to a Frame to add them."
    )

    // MARK: Frame(VI.3, frame_metadata.rs key 0x0101~0x0107)

    static let frameClipChoices = [
        InspectorChoice("none", "None"),
        InspectorChoice("frame-bounds", "Frame Bounds"),
        InspectorChoice("path", "Path"),
    ]

    static let frameSizeRow = InspectorRowTemplate(id: "size", label: "Size", fields: [
        InspectorFieldTemplate("frame.size.width", coreKey: 0x0101, label: "Width", short: "W",
                               control: .number(.nonNegativeLength)),
        InspectorFieldTemplate("frame.size.height", coreKey: 0x0101, label: "Height", short: "H",
                               control: .number(.nonNegativeLength)),
    ])

    static let frameLayoutSection = InspectorSectionTemplate(.frameLayout, "Frame", rows: [
        InspectorRowTemplate(id: "position", label: "Position", fields: [
            InspectorFieldTemplate("frame.transform.x", coreKey: 0x0102, label: "X", short: "X", control: .number(.length)),
            InspectorFieldTemplate("frame.transform.y", coreKey: 0x0102, label: "Y", short: "Y", control: .number(.length)),
        ]),
        frameSizeRow,
        InspectorRowTemplate(id: "rotation", label: "Rotation", fields: [
            InspectorFieldTemplate("frame.transform.rotation", coreKey: 0x0102, label: "Rotation",
                                   control: .number(.angle)),
        ]),
        InspectorRowTemplate(id: "flip", label: "Flip", fields: [
            InspectorFieldTemplate("frame.transform.flipHorizontal", coreKey: 0x0102, label: "Flip Horizontally",
                                   control: .action(title: "", symbolName: InspectorSymbols.flipHorizontal),
                                   bindable: false),
            InspectorFieldTemplate("frame.transform.flipVertical", coreKey: 0x0102, label: "Flip Vertically",
                                   control: .action(title: "", symbolName: InspectorSymbols.flipVertical),
                                   bindable: false),
        ]),
        InspectorRowTemplate(id: "clip", label: "Clip", fields: [
            InspectorFieldTemplate("frame.clipping", coreKey: 0x0103, label: "Clip Content",
                                   control: .popup(frameClipChoices)),
        ]),
        InspectorRowTemplate(id: "layout", label: "Layout", fields: [
            // 선택지는 문서의 layout 정의(Phase X)에서 adapter가 채웁니다.
            InspectorFieldTemplate("frame.layoutMode", coreKey: 0x0107, label: "Layout Mode",
                                   control: .popup([InspectorChoice("none", "None")])),
        ]),
    ])

    static let frameBackgroundRow = InspectorRowTemplate(id: "background", label: "Background", fields: [
        InspectorFieldTemplate("frame.background.enabled", coreKey: 0x0104, label: "Background",
                               control: .toggle, bindable: false),
        InspectorFieldTemplate("frame.background.color", coreKey: 0x0104, label: "Background Color",
                               control: .color),
    ])

    static let frameAppearanceSection = InspectorSectionTemplate(.frameAppearance, "Appearance", rows: [
        frameBackgroundRow,
        InspectorRowTemplate(id: "border", label: "Border", fields: [
            InspectorFieldTemplate("frame.border.enabled", coreKey: 0x0105, label: "Border",
                                   control: .toggle, bindable: false),
            InspectorFieldTemplate("frame.border.color", coreKey: 0x0105, label: "Border Color", control: .color),
            InspectorFieldTemplate("frame.border.thickness", coreKey: 0x0105, label: "Border Width",
                                   control: .number(.nonNegativeLength)),
        ]),
        radiusRow(id: "radiusTop", label: "Radius", prefix: "frame.cornerRadius", coreKey: 0x0106,
                  corners: [("topLeft", "Top Left Radius", "TL"), ("topRight", "Top Right Radius", "TR")]),
        radiusRow(id: "radiusBottom", label: "", prefix: "frame.cornerRadius", coreKey: 0x0106,
                  corners: [("bottomLeft", "Bottom Left Radius", "BL"), ("bottomRight", "Bottom Right Radius", "BR")]),
    ])

    // MARK: Shape(VI.5)

    static func shapeSection(_ kind: InspectorShapeKind) -> InspectorSectionTemplate {
        let convert = InspectorRowTemplate(id: "convert", label: "", fields: [
            // 파라메트릭 정보를 잃는 명시적 파괴 변환입니다(아키텍처 §12.6).
            InspectorFieldTemplate("shape.convertToPath", label: "Convert to Path",
                                   control: .action(title: "Convert to Path", symbolName: nil, destructive: true),
                                   bindable: false),
        ])
        switch kind {
        case .rectangle:
            return InspectorSectionTemplate(.shape, "Rectangle", rows: [
                InspectorRowTemplate(id: "size", label: "Size", fields: [
                    InspectorFieldTemplate("shape.rectangle.size.width", label: "Width", short: "W",
                                           control: .number(.nonNegativeLength)),
                    InspectorFieldTemplate("shape.rectangle.size.height", label: "Height", short: "H",
                                           control: .number(.nonNegativeLength)),
                ]),
                radiusRow(id: "radiusTop", label: "Radius", prefix: "shape.rectangle.cornerRadius", coreKey: nil,
                          corners: [("topLeft", "Top Left Radius", "TL"), ("topRight", "Top Right Radius", "TR")]),
                radiusRow(id: "radiusBottom", label: "", prefix: "shape.rectangle.cornerRadius", coreKey: nil,
                          corners: [("bottomLeft", "Bottom Left Radius", "BL"),
                                    ("bottomRight", "Bottom Right Radius", "BR")]),
                convert,
            ])
        case .ellipse:
            return InspectorSectionTemplate(.shape, "Ellipse",
                                            rows: boundsRows(prefix: "shape.ellipse.bounds") + [convert])
        case .line:
            return InspectorSectionTemplate(.shape, "Line", rows: [
                InspectorRowTemplate(id: "start", label: "Start", fields: [
                    InspectorFieldTemplate("shape.line.start.x", label: "Start X", short: "X", control: .number(.length)),
                    InspectorFieldTemplate("shape.line.start.y", label: "Start Y", short: "Y", control: .number(.length)),
                ]),
                InspectorRowTemplate(id: "end", label: "End", fields: [
                    InspectorFieldTemplate("shape.line.end.x", label: "End X", short: "X", control: .number(.length)),
                    InspectorFieldTemplate("shape.line.end.y", label: "End Y", short: "Y", control: .number(.length)),
                ]),
                InspectorRowTemplate(id: "cap", label: "Cap", fields: [
                    InspectorFieldTemplate("shape.line.cap", label: "Line Cap", control: .segmented([
                        InspectorChoice("butt", "Butt"),
                        InspectorChoice("round", "Round"),
                        InspectorChoice("square", "Square"),
                    ])),
                ]),
                convert,
            ])
        case .polygon:
            return InspectorSectionTemplate(.shape, "Polygon", rows: boundsRows(prefix: "shape.polygon.bounds") + [
                InspectorRowTemplate(id: "sides", label: "Sides", fields: [
                    InspectorFieldTemplate("shape.polygon.sides", label: "Sides", control: .number(.polygonSides)),
                ]),
                InspectorRowTemplate(id: "rotation", label: "Rotation", fields: [
                    InspectorFieldTemplate("shape.polygon.rotation", label: "Rotation", control: .number(.angle)),
                ]),
                InspectorRowTemplate(id: "radius", label: "Radius", fields: [
                    InspectorFieldTemplate("shape.polygon.cornerRadius", label: "Corner Radius",
                                           control: .number(.nonNegativeLengthStepper)),
                ]),
                convert,
            ])
        }
    }

    // MARK: Path(VI.6)

    static let pathSection = InspectorSectionTemplate(
        .path, "Path",
        rows: [
            InspectorRowTemplate(id: "winding", label: "Fill Rule", fields: [
                InspectorFieldTemplate("path.winding", label: "Fill Rule", control: .segmented([
                    InspectorChoice("non-zero", "Non-Zero"),
                    InspectorChoice("even-odd", "Even-Odd"),
                ])),
            ]),
            InspectorRowTemplate(id: "points", label: "Points", fields: [
                InspectorFieldTemplate("path.pointCount", label: "Points", control: .readOnly, bindable: false),
            ]),
            InspectorRowTemplate(id: "subpaths", label: "Subpaths", fields: [
                InspectorFieldTemplate("path.subpathCount", label: "Subpaths", control: .readOnly, bindable: false),
            ]),
            InspectorRowTemplate(id: "origin", label: "Bounds", fields: [
                InspectorFieldTemplate("path.bounds.x", label: "Bounds X", short: "X", control: .readOnly,
                                       bindable: false),
                InspectorFieldTemplate("path.bounds.y", label: "Bounds Y", short: "Y", control: .readOnly,
                                       bindable: false),
            ]),
            InspectorRowTemplate(id: "extent", label: "", fields: [
                InspectorFieldTemplate("path.bounds.width", label: "Bounds Width", short: "W", control: .readOnly,
                                       bindable: false),
                InspectorFieldTemplate("path.bounds.height", label: "Bounds Height", short: "H", control: .readOnly,
                                       bindable: false),
            ]),
        ],
        note: "Edit points and handles on the canvas."
    )

    // MARK: Text(VI.7)

    static let textContentSection = InspectorSectionTemplate(.textContent, "Text", rows: [
        InspectorRowTemplate(id: "content", label: "Content", fields: [
            InspectorFieldTemplate("text.content", label: "Content", control: .readOnly, bindable: false),
        ]),
        InspectorRowTemplate(id: "edit", label: "", fields: [
            // 캔버스 Text 편집은 공통 editing session이 맡습니다(아키텍처 §11.1, 가이드 5.2).
            InspectorFieldTemplate("text.editOnCanvas", label: "Edit Text on Canvas",
                                   control: .action(title: "Edit on Canvas", symbolName: nil), bindable: false),
        ]),
    ])

    static let textStyleSection = InspectorSectionTemplate(.textStyle, "Character", rows: [
        InspectorRowTemplate(id: "typography", label: "Typography", fields: [
            // 선택지는 문서의 typography 정의에서 adapter가 채웁니다.
            InspectorFieldTemplate("text.style.typography", label: "Typography",
                                   control: .popup([InspectorChoice("default", "Default")])),
        ]),
        InspectorRowTemplate(id: "fill", label: "Fill", fields: [
            InspectorFieldTemplate("text.style.fill.enabled", label: "Text Fill", control: .toggle, bindable: false),
            InspectorFieldTemplate("text.style.fill.color", label: "Text Fill Color", control: .color),
        ]),
        InspectorRowTemplate(id: "features", label: "OpenType", fields: [
            InspectorFieldTemplate("text.style.features", label: "OpenType Features and Variations",
                                   control: .readOnly, bindable: false),
        ]),
    ])

    static let lineHeightModeChoices = [
        InspectorChoice("auto", "Auto"),
        InspectorChoice("multiplier", "Multiple"),
        InspectorChoice("fixed", "Fixed"),
    ]

    static let paragraphSection = InspectorSectionTemplate(.paragraph, "Paragraph", rows: [
        InspectorRowTemplate(id: "alignment", label: "Alignment", fields: [
            InspectorFieldTemplate("text.paragraph.alignment", label: "Alignment", control: .segmented([
                InspectorChoice("start", "Align Start", symbol: InspectorSymbols.alignStart),
                InspectorChoice("center", "Align Center", symbol: InspectorSymbols.alignCenter),
                InspectorChoice("end", "Align End", symbol: InspectorSymbols.alignEnd),
                InspectorChoice("justify", "Justify", symbol: InspectorSymbols.alignJustify),
            ])),
        ]),
        InspectorRowTemplate(id: "lineHeight", label: "Line Height", fields: [
            InspectorFieldTemplate("text.paragraph.lineHeight.mode", label: "Line Height Mode",
                                   control: .popup(lineHeightModeChoices)),
            // 수치 규격은 mode에 따라 adapter가 `lineHeightMultiplier`·`lineHeightFixed`로 바꿉니다.
            InspectorFieldTemplate("text.paragraph.lineHeight.value", label: "Line Height",
                                   control: .number(.lineHeightMultiplier)),
        ]),
        InspectorRowTemplate(id: "letterSpacing", label: "Letter Spacing", fields: [
            InspectorFieldTemplate("text.paragraph.letterSpacing", label: "Letter Spacing",
                                   control: .number(.letterSpacing)),
        ]),
        InspectorRowTemplate(id: "wrapping", label: "Wrapping", fields: [
            InspectorFieldTemplate("text.paragraph.wrapping", label: "Wrapping", control: .segmented([
                InspectorChoice("wrap", "Wrap"),
                InspectorChoice("no-wrap", "No Wrap"),
            ])),
        ]),
    ])

    // MARK: Image(VI.8)

    static let imageSection = InspectorSectionTemplate(.image, "Image", rows: [
        InspectorRowTemplate(id: "asset", label: "Asset", fields: [
            InspectorFieldTemplate("image.asset", coreKey: 0x0601, label: "Asset", control: .readOnly,
                                   bindable: false),
        ]),
        InspectorRowTemplate(id: "size", label: "Size", fields: [
            InspectorFieldTemplate("image.size.width", label: "Width", short: "W", control: .number(.nonNegativeLength)),
            InspectorFieldTemplate("image.size.height", label: "Height", short: "H",
                                   control: .number(.nonNegativeLength)),
        ]),
        InspectorRowTemplate(id: "fit", label: "Fit", fields: [
            InspectorFieldTemplate("image.fit", label: "Fit", control: .popup([
                InspectorChoice("fill", "Fill"),
                InspectorChoice("contain", "Contain"),
                InspectorChoice("cover", "Cover"),
                InspectorChoice("none", "Original Size"),
            ])),
        ]),
        InspectorRowTemplate(id: "cropOrigin", label: "Crop", fields: [
            InspectorFieldTemplate("image.crop.x", label: "Crop X", short: "X", control: .number(.normalizedPercent)),
            InspectorFieldTemplate("image.crop.y", label: "Crop Y", short: "Y", control: .number(.normalizedPercent)),
        ]),
        InspectorRowTemplate(id: "cropSize", label: "", fields: [
            InspectorFieldTemplate("image.crop.width", label: "Crop Width", short: "W",
                                   control: .number(.normalizedPercent)),
            InspectorFieldTemplate("image.crop.height", label: "Crop Height", short: "H",
                                   control: .number(.normalizedPercent)),
        ]),
        InspectorRowTemplate(id: "sampling", label: "Sampling", fields: [
            InspectorFieldTemplate("image.sampling", label: "Sampling", control: .segmented([
                InspectorChoice("linear", "Linear"),
                InspectorChoice("nearest", "Nearest"),
            ])),
        ]),
        InspectorRowTemplate(id: "colorSpace", label: "Color Space", fields: [
            InspectorFieldTemplate("image.colorSpace", label: "Color Space Override", control: .popup([
                InspectorChoice("asset", "Asset Default"),
                InspectorChoice("srgb", "sRGB"),
                InspectorChoice("display-p3", "Display P3"),
            ])),
        ]),
    ])

    // MARK: ComponentInstance(IX.2, component/instance.rs key 0x0A01~0x0A03)

    static let componentSection = InspectorSectionTemplate(.component, "Component", rows: [
        InspectorRowTemplate(id: "definition", label: "Component", fields: [
            InspectorFieldTemplate("component.definition", coreKey: 0x0A01, label: "Component Definition",
                                   control: .readOnly, bindable: false),
        ]),
        InspectorRowTemplate(id: "detach", label: "", fields: [
            // 자유 편집은 명시적 Detach로만 엽니다(아키텍처 §15.1, IX.2.6). 정의와의 연결을 끊는 파괴 동작입니다.
            InspectorFieldTemplate("component.detach", label: "Detach Instance",
                                   control: .action(title: "Detach Instance", symbolName: nil, destructive: true),
                                   bindable: false),
        ]),
    ])

    /// Component Property 값 종류(core-types `TypedValue` variant).
    enum ComponentValueKind {
        case boolean, number, string, length, percentage, angle, duration, color
        case enumeration([InspectorChoice])
        /// Gradient·Shadow·Typography·ResourceRef: 전용 편집기 전까지 읽기 전용 요약.
        case summaryOnly
    }

    /// 공개 Component Property 하나의 필드 템플릿. override는 `PROPERTY_OVERRIDES`(0x0A02)에 저장됩니다.
    static func componentProperty(id: String, name: String, kind: ComponentValueKind) -> InspectorFieldTemplate {
        let control: InspectorControl
        switch kind {
        case .boolean: control = .toggle
        case .number: control = .number(.plain)
        case .string: control = .text(placeholder: name)
        case .length: control = .number(.length)
        case .percentage: control = .number(InspectorNumberSpec(unit: .percent, fractionDigits: 1))
        case .angle: control = .number(.angle)
        case .duration: control = .number(.duration)
        case .color: control = .color
        case .enumeration(let choices): control = .popup(choices)
        case .summaryOnly: control = .readOnly
        }
        return InspectorFieldTemplate("component.property.\(id)", coreKey: 0x0A02, label: name, control: control)
    }

    /// 공개 Slot 하나의 행(읽기 전용 요약). slot override는 `SLOT_OVERRIDES`(0x0A03)입니다.
    static func componentSlot(id: String, name: String) -> InspectorRowTemplate {
        InspectorRowTemplate(id: "slot.\(id)", label: name, fields: [
            InspectorFieldTemplate("component.slot.\(id)", coreKey: 0x0A03, label: "\(name) Slot",
                                   control: .readOnly, bindable: false),
        ])
    }

    /// 공개 Component Property 섹션(정의마다 다름). adapter·Stub·예열이 같은 템플릿을 씁니다.
    static func componentPropertiesSection(
        _ properties: [(id: String, name: String, kind: ComponentValueKind)]
    ) -> InspectorSectionTemplate {
        InspectorSectionTemplate(.componentProperties, "Properties", rows: properties.map {
            InspectorRowTemplate(id: $0.id, label: $0.name, fields: [
                componentProperty(id: $0.id, name: $0.name, kind: $0.kind),
            ])
        })
    }

    /// 공개 Slot 섹션(정의마다 다름).
    static func componentSlotsSection(_ slots: [(id: String, name: String)]) -> InspectorSectionTemplate {
        InspectorSectionTemplate(
            .componentSlots, "Slots",
            rows: slots.map { componentSlot(id: $0.id, name: $0.name) },
            note: "Add content to a slot on the canvas or in the layer list."
        )
    }

    // MARK: Modifier(VI.9, modifier_metadata.rs key 0x1001~0x10C1)

    static let booleanOpChoices = [
        InspectorChoice("union", "Union"),
        InspectorChoice("subtract", "Subtract"),
        InspectorChoice("intersect", "Intersect"),
        InspectorChoice("exclude", "Exclude"),
    ]

    static let blendModeChoices = [
        InspectorChoice("normal", "Normal"),
        InspectorChoice("multiply", "Multiply"),
        InspectorChoice("screen", "Screen"),
        InspectorChoice("overlay", "Overlay"),
        InspectorChoice("darken", "Darken"),
        InspectorChoice("lighten", "Lighten"),
    ]

    /// modifier 종류별 파라미터 행. enabled(0x1001)는 목록 행의 체크박스, order(0x1002)는 재정렬로 편집합니다.
    static func modifierRows(_ kind: InspectorModifierKind, modifierID: String) -> [InspectorRowTemplate] {
        func f(_ name: String, _ key: UInt32, _ label: String, short: String? = nil,
               _ control: InspectorControl, bindable: Bool = true) -> InspectorFieldTemplate {
            InspectorFieldTemplate("modifier.\(modifierID).\(name)", coreKey: key, label: label, short: short,
                                   control: control, bindable: bindable)
        }
        switch kind {
        case .transform:
            // 행렬(0x1011)은 코어가 분해한 값으로 표시하고 다시 합성합니다.
            return [
                InspectorRowTemplate(id: "offset", label: "Offset", fields: [
                    f("transform.x", 0x1011, "Offset X", short: "X", .number(.length)),
                    f("transform.y", 0x1011, "Offset Y", short: "Y", .number(.length)),
                ]),
                InspectorRowTemplate(id: "rotation", label: "Rotation", fields: [
                    f("transform.rotation", 0x1011, "Rotation", .number(.angle)),
                ]),
                InspectorRowTemplate(id: "scale", label: "Scale", fields: [
                    f("transform.scaleX", 0x1011, "Scale X", short: "X", .number(.scalePercent)),
                    f("transform.scaleY", 0x1011, "Scale Y", short: "Y", .number(.scalePercent)),
                ]),
            ]
        case .boolean:
            return [
                InspectorRowTemplate(id: "op", label: "Operation", fields: [
                    f("boolean.op", 0x1021, "Boolean Operation", .popup(booleanOpChoices)),
                ]),
                InspectorRowTemplate(id: "operands", label: "Operands", fields: [
                    f("boolean.operands", 0x1022, "Boolean Operands", .readOnly, bindable: false),
                ]),
            ]
        case .strokeExpand:
            return [InspectorRowTemplate(id: "width", label: "Width", fields: [
                f("strokeExpand.width", 0x1031, "Expand Width", .number(.nonNegativeLengthStepper)),
            ])]
        case .corner:
            return [InspectorRowTemplate(id: "radius", label: "Radius", fields: [
                f("corner.radius", 0x1041, "Corner Radius", .number(.nonNegativeLengthStepper)),
            ])]
        case .mask:
            return [InspectorRowTemplate(id: "source", label: "Source", fields: [
                f("mask.source", 0x1051, "Mask Source", .readOnly, bindable: false),
            ])]
        case .clip:
            return [InspectorRowTemplate(id: "source", label: "Source", fields: [
                f("clip.source", 0x1061, "Clip Source", .readOnly, bindable: false),
            ])]
        case .fill:
            return [InspectorRowTemplate(id: "color", label: "Color", fields: [
                f("fill.color", 0x1071, "Fill Color", .color),
            ])]
        case .stroke:
            return [
                InspectorRowTemplate(id: "color", label: "Color", fields: [
                    f("stroke.color", 0x1081, "Stroke Color", .color),
                ]),
                InspectorRowTemplate(id: "width", label: "Width", fields: [
                    f("stroke.width", 0x1082, "Stroke Width", .number(.nonNegativeLengthStepper)),
                ]),
            ]
        case .shadow:
            return [
                InspectorRowTemplate(id: "color", label: "Color", fields: [
                    f("shadow.color", 0x1091, "Shadow Color", .color),
                ]),
                InspectorRowTemplate(id: "offset", label: "Offset", fields: [
                    f("shadow.offset.x", 0x1092, "Shadow Offset X", short: "X", .number(.length)),
                    f("shadow.offset.y", 0x1092, "Shadow Offset Y", short: "Y", .number(.length)),
                ]),
                InspectorRowTemplate(id: "blur", label: "Blur", fields: [
                    f("shadow.blur", 0x1093, "Shadow Blur", .number(.nonNegativeLengthStepper)),
                ]),
            ]
        case .blur:
            return [InspectorRowTemplate(id: "radius", label: "Radius", fields: [
                f("blur.radius", 0x10A1, "Blur Radius", .number(.nonNegativeLengthStepper)),
            ])]
        case .blend:
            return [InspectorRowTemplate(id: "mode", label: "Mode", fields: [
                f("blend.mode", 0x10B1, "Blend Mode", .popup(blendModeChoices)),
            ])]
        case .opacity:
            return [InspectorRowTemplate(id: "value", label: "Opacity", fields: [
                f("opacity.value", 0x10C1, "Opacity", .number(.unitPercent)),
            ])]
        }
    }

    // MARK: 선택 종류 → 섹션

    /// 같은 종류만 선택했을 때 공통 레이어 섹션 다음에 오는 섹션.
    static func kindSections(_ kind: InspectorNodeKind, shape: InspectorShapeKind?) -> [InspectorSectionTemplate] {
        switch kind {
        case .frame: return [frameLayoutSection, frameAppearanceSection]
        case .fragment: return [fragmentSection]
        case .shape: return [shapeSection(shape ?? .rectangle)]
        case .path: return [pathSection]
        case .text: return [textContentSection, textStyleSection, paragraphSection]
        case .image: return [imageSection]
        // Component Property·Slot 섹션은 정의마다 다르므로 adapter가 동적으로 만듭니다.
        case .componentInstance: return [componentSection]
        }
    }

    // MARK: 예열(docs/design/ui/inspector.md 8.3절)

    /// 카탈로그만으로 정해지는 구성(선택 종류·도형 종류·다른 종류 다중 선택)의 예열용 snapshot.
    ///
    /// 페이지 구성 키는 섹션·행·필드 구성으로 정하므로(값·속성 id 제외) 실제 선택 snapshot이 같은 페이지를 씁니다.
    /// 값은 모두 없음(`.none`)입니다(견본 값으로 예열해도 처음 표시 비용이 줄지 않고 대기 페이지만 무거워짐, 8.3절).
    /// Binding 대상 필드는 실제 adapter처럼 binding 상태를 가집니다(`.fixed`). Component Property·Slot 구성은
    /// 정의마다 달라 공개 Property·Slot이 없는 경우만 넣습니다(정의별 구성은 adapter가
    /// `InspectorViewController.prewarm(_:)`으로 예열). 순서는 자주 쓰는 종류부터이고, 인스펙터는 앞에서부터
    /// `prewarmedCatalogPageCount`개만 예열합니다.
    static func prewarmSnapshots() -> [InspectorSnapshot] {
        let resolve: InspectorFieldResolver = { _ in InspectorFieldState(value: .none) }
        let stack = InspectorModifierStack(availability: .available, items: [],
                                           addableKinds: InspectorModifierKind.allCases, isEditable: true)
        func snapshot(_ kinds: [InspectorNodeKind], shape: InspectorShapeKind? = nil) -> InspectorSnapshot {
            var sections = [layerSection.make(resolve)]
            var modifiers: InspectorModifierStack? = stack
            if kinds.count == 1 {
                let kind = kinds[0]
                sections += kindSections(kind, shape: shape).map { $0.make(resolve) }
                if kind == .componentInstance {
                    sections.append(componentPropertiesSection([]).make(resolve))
                    sections.append(componentSlotsSection([]).make(resolve))
                }
                if kind == .fragment { modifiers = nil }
            }
            return InspectorSnapshot(
                generation: 0, selectionToken: 0, state: .ready,
                selection: InspectorSelectionSummary(count: kinds.count, nodeKinds: kinds, shapeKind: shape,
                                                     displayName: nil),
                editability: .editable, sections: sections, modifiers: modifiers
            )
        }
        return [
            snapshot([.frame]), snapshot([.text]), snapshot([.shape], shape: .rectangle), snapshot([.image]),
            snapshot([.path]), snapshot([.shape], shape: .ellipse), snapshot([.shape], shape: .line),
            snapshot([.shape], shape: .polygon), snapshot([.componentInstance]), snapshot([.fragment]),
            snapshot([.frame, .shape]),
        ]
    }

    // MARK: 내부 도우미

    private static func radiusRow(
        id: String, label: String, prefix: String, coreKey: UInt32?,
        corners: [(name: String, label: String, short: String)]
    ) -> InspectorRowTemplate {
        InspectorRowTemplate(id: id, label: label, fields: corners.map { corner in
            InspectorFieldTemplate("\(prefix).\(corner.name)", coreKey: coreKey, label: corner.label,
                                   short: corner.short, control: .number(.nonNegativeLength))
        })
    }

    private static func boundsRows(prefix: String) -> [InspectorRowTemplate] {
        [
            InspectorRowTemplate(id: "position", label: "Position", fields: [
                InspectorFieldTemplate("\(prefix).x", label: "X", short: "X", control: .number(.length)),
                InspectorFieldTemplate("\(prefix).y", label: "Y", short: "Y", control: .number(.length)),
            ]),
            InspectorRowTemplate(id: "size", label: "Size", fields: [
                InspectorFieldTemplate("\(prefix).width", label: "Width", short: "W",
                                       control: .number(.nonNegativeLength)),
                InspectorFieldTemplate("\(prefix).height", label: "Height", short: "H",
                                       control: .number(.nonNegativeLength)),
            ]),
        ]
    }
}
