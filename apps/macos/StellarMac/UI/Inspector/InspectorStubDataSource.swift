import Foundation

/// 코어 연결 전 인스펙터용 의도적 placeholder 데이터 공급자(계획 규칙: placeholder는 이름에 "Stub").
///
/// 선택 종류별 예시 snapshot을 만들고, 받은 명령을 메모리 값에 적용해 새 세대 snapshot을 돌려줍니다.
/// 실제 문서·Undo·검증은 하지 않습니다. 코어 adapter는 같은 `InspectorPropertyCatalog` 템플릿을 쓰고
/// 값만 문서 snapshot에서 채우면 됩니다.
final class InspectorStubDataSource: InspectorDataSource {
    /// 예시 선택 상황.
    enum Scenario: String, CaseIterable {
        case noSelection, loading, failed
        case frame, rectangle, ellipse, line, polygon, path, text, image, componentInstance, fragment
        case multipleFrames, mixedKinds, locked, readOnly
    }

    private(set) var scenario: Scenario
    private(set) var snapshot: InspectorSnapshot
    var snapshotHandler: ((InspectorSnapshot) -> Void)?
    /// 받은 명령 기록(검사용).
    private(set) var receivedCommands: [InspectorCommand] = []
    /// 각 명령의 근거 세대(검사용, `receivedCommands`와 같은 순서).
    private(set) var receivedGenerations: [UInt64] = []
    private var selectionToken: UInt64 = 1
    private var model: InspectorStubModel
    private var generation: UInt64 = 1

    init(scenario: Scenario = .frame) {
        self.scenario = scenario
        model = InspectorStubModel.make(scenario)
        snapshot = model.snapshot(generation: generation, selectionToken: selectionToken)
    }

    /// 다른 예시 선택으로 바꿉니다(선택 변경 흉내).
    func select(_ scenario: Scenario) {
        self.scenario = scenario
        model = InspectorStubModel.make(scenario)
        selectionToken += 1
        publish()
    }

    func submit(_ command: InspectorCommand, basedOn generation: UInt64) {
        receivedCommands.append(command)
        receivedGenerations.append(generation)
        switch command {
        case .setValue(let id, let value, _):
            model.set(id.path, value, spec: Self.numberSpec(of: snapshot.field(id)))
        case .adjustNumber(let id, let delta, _):
            model.adjust(id.path, by: delta, spec: Self.numberSpec(of: snapshot.field(id)))
        case .perform(let id):
            switch id.path {
            case "shape.convertToPath":
                model = model.converted(to: .path)
            case "component.detach", "fragment.convertToFrame":
                model = model.converted(to: .frame)
            default: break // 반전·캔버스 편집은 Stub이 값 변화로 흉내 내지 않습니다.
            }
        case .showBinding:
            return // Value Binding UI는 후속 작업입니다.
        case .addModifier(let kind):
            model.addModifier(kind)
        case .removeModifier(let id):
            model.modifiers?.removeAll { $0.id == id }
        case .setModifierEnabled(let id, let isEnabled):
            if let index = model.modifiers?.firstIndex(where: { $0.id == id }) {
                model.modifiers?[index].isEnabled = isEnabled
            }
        case .moveModifier(let id, let toIndex):
            if var list = model.modifiers, let from = list.firstIndex(where: { $0.id == id }),
               list.indices.contains(toIndex) {
                let item = list.remove(at: from)
                list.insert(item, at: toIndex)
                model.modifiers = list
            }
        case .reload:
            if case .failed = model.state {
                model = InspectorStubModel.make(.frame)
                selectionToken += 1
            }
        }
        publish()
    }

    private func publish() {
        generation += 1
        snapshot = model.snapshot(generation: generation, selectionToken: selectionToken)
        snapshotHandler?(snapshot)
    }

    private static func numberSpec(of field: InspectorField?) -> InspectorNumberSpec? {
        if case .number(let spec)? = field?.control { return spec }
        return nil
    }
}

/// Stub 내부 상태: 경로별 값과 modifier 목록.
struct InspectorStubModel {
    struct Modifier {
        var id: String
        var kind: InspectorModifierKind
        var isEnabled: Bool
    }

    var state: InspectorContentState = .ready
    var selection = InspectorSelectionSummary.empty
    var locked = false
    var readOnlyReason: String?
    var values: [String: InspectorValue] = [:]
    var bindings: [String: InspectorBinding] = [:]
    var issues: [InspectorSectionID: [String: String]] = [:]
    var extraChoices: [String: [InspectorChoice]] = [:]
    var componentProperties: [(id: String, name: String, kind: InspectorPropertyCatalog.ComponentValueKind)] = []
    var componentSlots: [(id: String, name: String)] = []
    /// `nil`이면 modifier를 가질 수 없는 선택(Fragment), 빈 배열이면 modifier 없음.
    var modifiers: [Modifier]? = []
    var modifiersUnavailableReason: String?
    private var nextModifierSerial = 100

    // MARK: 명령 적용

    mutating func set(_ path: String, _ value: InspectorValue, spec: InspectorNumberSpec?) {
        if case .number(let number) = value, let spec {
            values[path] = .number(spec.clamped(number))
        } else {
            values[path] = value
        }
        if path == InspectorPropertyID.layerLocked.path, case .bool(let on) = value {
            locked = on
        }
        if path == "text.paragraph.lineHeight.mode", case .choice(let mode) = value {
            values["text.paragraph.lineHeight.value"] = mode == "auto" ? InspectorValue.none : .number(mode == "fixed" ? 24 : 1.2)
        }
    }

    mutating func adjust(_ path: String, by delta: Double, spec: InspectorNumberSpec?) {
        // 혼합 값은 node별 값이 필요하므로 Stub에서는 그대로 둡니다.
        guard case .number(let current)? = values[path] else { return }
        let next = current + delta
        values[path] = .number(spec?.clamped(next) ?? next)
    }

    mutating func addModifier(_ kind: InspectorModifierKind) {
        guard modifiers != nil else { return }
        nextModifierSerial += 1
        let id = "stub-mod-\(nextModifierSerial)"
        modifiers?.append(Modifier(id: id, kind: kind, isEnabled: true))
        // 문서 모델 `ModifierPayload::default_for`의 기본값.
        let black = InspectorValue.color(InspectorColor(space: .srgb, red: 0, green: 0, blue: 0, alpha: 1))
        let defaults: [String: InspectorValue]
        switch kind {
        case .transform:
            defaults = ["transform.x": .number(0), "transform.y": .number(0), "transform.rotation": .number(0),
                        "transform.scaleX": .number(1), "transform.scaleY": .number(1)]
        case .boolean: defaults = ["boolean.op": .choice("union"), "boolean.operands": .text("No operands")]
        case .strokeExpand: defaults = ["strokeExpand.width": .number(1)]
        case .corner: defaults = ["corner.radius": .number(0)]
        case .mask: defaults = ["mask.source": .text("Not set")]
        case .clip: defaults = ["clip.source": .text("Not set")]
        case .fill: defaults = ["fill.color": black]
        case .stroke: defaults = ["stroke.color": black, "stroke.width": .number(1)]
        case .shadow:
            defaults = ["shadow.color": black, "shadow.offset.x": .number(0), "shadow.offset.y": .number(0),
                        "shadow.blur": .number(0)]
        case .blur: defaults = ["blur.radius": .number(0)]
        case .blend: defaults = ["blend.mode": .choice("normal")]
        case .opacity: defaults = ["opacity.value": .number(1)]
        }
        for (name, value) in defaults {
            values["modifier.\(id).\(name)"] = value
        }
    }

    /// 파괴 변환(Path로 변환·Detach·Frame으로 변환) 흉내: 이름을 유지한 새 종류의 예시.
    func converted(to scenario: InspectorStubDataSource.Scenario) -> InspectorStubModel {
        var next = InspectorStubModel.make(scenario)
        next.values[InspectorPropertyID.layerName.path] = values[InspectorPropertyID.layerName.path]
        next.selection.displayName = selection.displayName
        return next
    }

    // MARK: snapshot

    func snapshot(generation: UInt64, selectionToken: UInt64) -> InspectorSnapshot {
        guard state == .ready else {
            return .placeholder(state, generation: generation, selectionToken: selectionToken)
        }
        let editability: InspectorEditability
        if let readOnlyReason {
            editability = .readOnly(reason: readOnlyReason)
        } else {
            editability = locked ? .locked : .editable
        }
        let resolve = resolver()

        var sections = [InspectorPropertyCatalog.layerSection.make(resolve)]
        if selection.nodeKinds.count == 1, let kind = selection.nodeKinds.first {
            for template in InspectorPropertyCatalog.kindSections(kind, shape: selection.shapeKind) {
                sections.append(template.make(resolve, issues: issues[template.id] ?? [:]))
            }
            if kind == .componentInstance {
                sections += componentSections(resolve)
            }
        }

        let stack: InspectorModifierStack?
        if let reason = modifiersUnavailableReason {
            stack = InspectorModifierStack(availability: .unavailable(reason: reason), items: [], addableKinds: [],
                                           isEditable: false)
        } else if let modifiers {
            stack = InspectorModifierStack(
                availability: .available,
                items: modifiers.map { modifier in
                    InspectorModifierItem(
                        id: modifier.id, kind: modifier.kind, isEnabled: modifier.isEnabled,
                        summary: summary(of: modifier),
                        parameters: InspectorPropertyCatalog.modifierRows(modifier.kind, modifierID: modifier.id)
                            .map { $0.make(resolve) }
                    )
                },
                addableKinds: InspectorModifierKind.allCases,
                isEditable: !locked && readOnlyReason == nil
            )
        } else {
            stack = nil
        }

        return InspectorSnapshot(
            generation: generation, selectionToken: selectionToken, state: .ready, selection: selection,
            editability: editability,
            sections: sections, modifiers: stack
        )
    }

    private func componentSections(_ resolve: InspectorFieldResolver) -> [InspectorSection] {
        let properties = InspectorSectionTemplate(.componentProperties, "Properties", rows: componentProperties.map {
            InspectorRowTemplate(id: $0.id, label: $0.name, fields: [
                InspectorPropertyCatalog.componentProperty(id: $0.id, name: $0.name, kind: $0.kind),
            ])
        })
        let slots = InspectorSectionTemplate(
            .componentSlots, "Slots",
            rows: componentSlots.map { InspectorPropertyCatalog.componentSlot(id: $0.id, name: $0.name) },
            note: "Add content to a slot on the canvas or in the layer list."
        )
        return [properties.make(resolve), slots.make(resolve)]
    }

    /// 템플릿 → 상태. 잠금·읽기 전용이면 레이어 이름·표시·잠금 외 편집을 막습니다.
    private func resolver() -> InspectorFieldResolver {
        { template in
            let path = template.id.path
            var state = InspectorFieldState(value: values[path] ?? InspectorValue.none)
            state.binding = bindings[path] ?? .fixed
            if readOnlyReason != nil {
                state.isEditable = false
            } else if locked {
                state.isEditable = path.hasPrefix("layer.")
            }
            // "<prefix>.enabled"가 꺼져 있으면 같은 prefix의 값 필드는 편집할 수 없습니다.
            if let dot = path.lastIndex(of: "."), !path.hasSuffix(".enabled"),
               case .bool(false)? = values[String(path[..<dot]) + ".enabled"] {
                state.isEditable = false
            }
            if path == "text.paragraph.lineHeight.value" {
                let mode = values["text.paragraph.lineHeight.mode"]
                state.control = .number(mode == .choice("fixed") ? .lineHeightFixed : .lineHeightMultiplier)
                if mode == .choice("auto") { state.isEditable = false }
            }
            if let extra = extraChoices[path], case .popup(let base) = template.control {
                state.control = .popup(base + extra)
            }
            return state
        }
    }

    private func summary(of modifier: InspectorStubModel.Modifier) -> String {
        func value(_ name: String) -> InspectorValue? { values["modifier.\(modifier.id).\(name)"] }
        func number(_ name: String, scale: Double = 1) -> String {
            guard case .number(let v)? = value(name) else { return "–" }
            let scaled = v * scale
            return scaled.rounded() == scaled ? String(Int(scaled)) : String(format: "%.1f", scaled)
        }
        func color(_ name: String) -> String {
            guard case .color(let c)? = value(name) else { return "–" }
            return c.hexString
        }
        func text(_ name: String) -> String {
            switch value(name) {
            case .text(let t)?: return t
            case .choice(let c)?: return c.capitalized
            default: return "–"
            }
        }
        switch modifier.kind {
        case .transform: return "\(number("transform.x")), \(number("transform.y")) · \(number("transform.rotation"))°"
        case .boolean: return "\(text("boolean.op")) · \(text("boolean.operands"))"
        case .strokeExpand: return "\(number("strokeExpand.width")) px"
        case .corner: return "\(number("corner.radius")) px"
        case .mask: return text("mask.source")
        case .clip: return text("clip.source")
        case .fill: return color("fill.color")
        case .stroke: return "\(color("stroke.color")), \(number("stroke.width")) px"
        case .shadow: return "\(number("shadow.offset.x")), \(number("shadow.offset.y")) · blur \(number("shadow.blur"))"
        case .blur: return "\(number("blur.radius")) px"
        case .blend: return text("blend.mode")
        case .opacity: return "\(number("opacity.value", scale: 100))%"
        }
    }

    // MARK: 예시

    static func make(_ scenario: InspectorStubDataSource.Scenario) -> InspectorStubModel {
        var model = InspectorStubModel()
        let rgb = { (r: Double, g: Double, b: Double, a: Double) -> InspectorValue in
            .color(InspectorColor(space: .srgb, red: r, green: g, blue: b, alpha: a))
        }

        func layer(_ name: String, _ kind: InspectorNodeKind, shape: InspectorShapeKind? = nil) {
            model.selection = InspectorSelectionSummary(count: 1, nodeKinds: [kind], shapeKind: shape, displayName: name)
            model.values["layer.name"] = .text(name)
            model.values["layer.visible"] = .bool(true)
            model.values["layer.locked"] = .bool(false)
        }
        func put(_ entries: [String: InspectorValue]) {
            for (path, value) in entries { model.values[path] = value }
        }
        func modifier(_ kind: InspectorModifierKind, _ values: [String: InspectorValue], enabled: Bool = true) {
            model.nextModifierSerial += 1
            let id = "stub-mod-\(model.nextModifierSerial)"
            model.modifiers?.append(Modifier(id: id, kind: kind, isEnabled: enabled))
            for (name, value) in values { model.values["modifier.\(id).\(name)"] = value }
        }
        func frameValues(width: InspectorValue = .number(320), x: InspectorValue = .number(24),
                         background: InspectorValue? = nil, borderEnabled: InspectorValue = .bool(true)) {
            put([
                "frame.transform.x": x, "frame.transform.y": .number(48),
                "frame.size.width": width, "frame.size.height": .number(200),
                "frame.transform.rotation": .number(0),
                "frame.clipping": .choice("frame-bounds"), "frame.layoutMode": .choice("none"),
                "frame.background.enabled": .bool(true),
                "frame.background.color": background ?? rgb(1, 1, 1, 1),
                "frame.border.enabled": borderEnabled, "frame.border.color": rgb(0.85, 0.86, 0.88, 1),
                "frame.border.thickness": .number(1),
                "frame.cornerRadius.topLeft": .number(12), "frame.cornerRadius.topRight": .number(12),
                "frame.cornerRadius.bottomLeft": .number(12), "frame.cornerRadius.bottomRight": .number(12),
            ])
            model.extraChoices["frame.layoutMode"] = [InspectorChoice("stub-layout-stack", "Vertical Stack")]
            model.bindings["frame.background.color"] = .token(name: "color.surface")
            model.bindings["frame.border.color"] = .token(name: "color.border")
        }

        switch scenario {
        case .noSelection:
            model.state = .noSelection
        case .loading:
            model.state = .loading
        case .failed:
            model.state = .failed(message: "The selection could not be read from the document.")
        case .frame, .locked:
            layer("Card", .frame)
            frameValues()
            modifier(.shadow, ["shadow.color": rgb(0, 0, 0, 0.15), "shadow.offset.x": .number(0),
                               "shadow.offset.y": .number(4), "shadow.blur": .number(12)])
            modifier(.opacity, ["opacity.value": .number(0.95)])
            if scenario == .locked {
                model.locked = true
                model.values["layer.locked"] = .bool(true)
            }
        case .rectangle, .readOnly:
            layer("Badge", .shape, shape: .rectangle)
            put([
                "shape.rectangle.size.width": .number(64), "shape.rectangle.size.height": .number(24),
                "shape.rectangle.cornerRadius.topLeft": .number(12), "shape.rectangle.cornerRadius.topRight": .number(12),
                "shape.rectangle.cornerRadius.bottomLeft": .number(12),
                "shape.rectangle.cornerRadius.bottomRight": .number(12),
            ])
            modifier(.fill, ["fill.color": rgb(0.2, 0.45, 0.95, 1)])
            modifier(.stroke, ["stroke.color": rgb(0.1, 0.3, 0.8, 1), "stroke.width": .number(1)], enabled: false)
            if scenario == .readOnly {
                model.readOnlyReason = "This project is in Restricted Mode. Trust the project to edit it."
            }
        case .ellipse:
            layer("Avatar Ring", .shape, shape: .ellipse)
            put(["shape.ellipse.bounds.x": .number(0), "shape.ellipse.bounds.y": .number(0),
                 "shape.ellipse.bounds.width": .number(48), "shape.ellipse.bounds.height": .number(48)])
            modifier(.stroke, ["stroke.color": rgb(0.2, 0.45, 0.95, 1), "stroke.width": .number(2)])
        case .line:
            layer("Divider", .shape, shape: .line)
            put(["shape.line.start.x": .number(0), "shape.line.start.y": .number(0),
                 "shape.line.end.x": .number(280), "shape.line.end.y": .number(0), "shape.line.cap": .choice("butt")])
            modifier(.stroke, ["stroke.color": rgb(0.85, 0.86, 0.88, 1), "stroke.width": .number(1)])
        case .polygon:
            layer("Hex Badge", .shape, shape: .polygon)
            put(["shape.polygon.bounds.x": .number(0), "shape.polygon.bounds.y": .number(0),
                 "shape.polygon.bounds.width": .number(40), "shape.polygon.bounds.height": .number(40),
                 "shape.polygon.sides": .number(6), "shape.polygon.rotation": .number(0),
                 "shape.polygon.cornerRadius": .number(2)])
            modifier(.fill, ["fill.color": rgb(0.95, 0.6, 0.1, 1)])
        case .path:
            layer("Logo Mark", .path)
            put(["path.winding": .choice("non-zero"), "path.pointCount": .text("18"), "path.subpathCount": .text("2"),
                 "path.bounds.x": .number(0), "path.bounds.y": .number(0),
                 "path.bounds.width": .number(96), "path.bounds.height": .number(96)])
            modifier(.boolean, ["boolean.op": .choice("subtract"), "boolean.operands": .text("Cutout")])
            modifier(.fill, ["fill.color": rgb(0.1, 0.1, 0.12, 1)])
        case .text:
            layer("Headline", .text)
            put([
                "text.content": .text("Ship designs straight from Git"),
                "text.style.typography": .choice("stub-type-title1"),
                "text.style.fill.enabled": .bool(true), "text.style.fill.color": rgb(0.1, 0.1, 0.12, 1),
                "text.style.features": .text("liga, kern · wght 600"),
                "text.paragraph.alignment": .choice("start"),
                "text.paragraph.lineHeight.mode": .choice("multiplier"),
                "text.paragraph.lineHeight.value": .number(1.2),
                "text.paragraph.letterSpacing": .number(-0.01),
                "text.paragraph.wrapping": .choice("wrap"),
            ])
            model.extraChoices["text.style.typography"] = [
                InspectorChoice("stub-type-title1", "Title 1"), InspectorChoice("stub-type-body", "Body"),
            ]
            model.bindings["text.style.fill.color"] = .token(name: "color.text.primary")
        case .image:
            layer("Hero Photo", .image)
            put(["image.asset": .text("hero.png"), "image.size.width": .number(320), "image.size.height": .number(180),
                 "image.fit": .choice("cover"), "image.crop.x": .number(0), "image.crop.y": .number(0),
                 "image.crop.width": .number(1), "image.crop.height": .number(1),
                 "image.sampling": .choice("linear"), "image.colorSpace": .choice("asset")])
            model.issues[.image] = ["asset": "Asset not found. The image keeps its size and shows a placeholder."]
            modifier(.blur, ["blur.radius": .number(0)], enabled: false)
        case .componentInstance:
            layer("Primary Button", .componentInstance)
            model.values["component.definition"] = .text("Button / Primary")
            model.componentProperties = [
                ("label", "Label", .string),
                ("enabled", "Enabled", .boolean),
                ("size", "Size", .enumeration([InspectorChoice("small", "Small"), InspectorChoice("medium", "Medium"),
                                               InspectorChoice("large", "Large")])),
                ("tint", "Tint", .color),
                ("radius", "Corner Radius", .length),
                ("icon", "Icon", .summaryOnly),
            ]
            put(["component.property.label": .text("Continue"), "component.property.enabled": .bool(true),
                 "component.property.size": .choice("medium"), "component.property.tint": rgb(0.2, 0.45, 0.95, 1),
                 "component.property.radius": .number(8), "component.property.icon": .text("arrow.right (resource)")])
            model.bindings["component.property.tint"] = .token(name: "color.accent")
            model.bindings["component.property.radius"] = .orphaned(reason: "Token radius.control was deleted")
            model.componentSlots = [("leading", "Leading"), ("trailing", "Trailing")]
            put(["component.slot.leading": .text("1 item (0–1)"), "component.slot.trailing": .text("Default (0–1)")])
            modifier(.shadow, ["shadow.color": rgb(0, 0, 0, 0.2), "shadow.offset.x": .number(0),
                               "shadow.offset.y": .number(1), "shadow.blur": .number(2)])
        case .fragment:
            layer("Group", .fragment)
            model.modifiers = nil
        case .multipleFrames:
            frameValues(width: .mixed, x: .mixed, background: .mixed, borderEnabled: .mixed)
            model.selection = InspectorSelectionSummary(count: 3, nodeKinds: [.frame], shapeKind: nil, displayName: nil)
            put(["layer.name": .mixed, "layer.visible": .bool(true), "layer.locked": .bool(false)])
            model.bindings["frame.background.color"] = .fixed
            model.modifiersUnavailableReason = "Select a single layer to edit its modifier stack."
        case .mixedKinds:
            model.selection = InspectorSelectionSummary(count: 3, nodeKinds: [.frame, .shape, .text], shapeKind: nil,
                                                        displayName: nil)
            put(["layer.name": .mixed, "layer.visible": .mixed, "layer.locked": .bool(false)])
            model.modifiersUnavailableReason = "Select a single layer to edit its modifier stack."
        }
        return model
    }
}
