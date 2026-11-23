import AppKit

enum StellarColors {
    static let surface = NSColor(displayP3Red: 0.96, green: 0.96, blue: 0.98, alpha: 1)
    static let accent = NSColor(srgbRed: 0, green: 0.48, blue: 1, alpha: 1)
    static let n00000000000000000000000000000001_4 = surface
    static let n00000000000000000000000000000002_5 = NSColor(srgbRed: 0.8, green: 0.1, blue: 0.1, alpha: 1)
    static let n0000000000000000000000000000000c_4 = accent
    static let n0000000000000000000000000000000d_4 = NSColor(colorSpace: .extendedSRGB, components: [1.1, 0.5, -0.05, 1], count: 4)
}

@MainActor
enum StellarAccessibility {
    static func apply00000000000000000000000000000001(to view: NSView, views: [String: NSView]) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren(["00000000-0000-0000-0000-000000000004", "00000000-0000-0000-0000-000000000002", "00000000-0000-0000-0000-000000000006", "00000000-0000-0000-0000-00000000000b", "00000000-0000-0000-0000-00000000000c", "00000000-0000-0000-0000-00000000000d", "00000000-0000-0000-0000-000000000010"].compactMap { views[$0] })
    }

    static func apply00000000000000000000000000000002(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.staticText)
        view.setAccessibilityValue("설정")
    }

    static func apply00000000000000000000000000000004(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.image)
        view.setAccessibilityLabel("프로필 사진")
    }

    static func apply00000000000000000000000000000005(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply00000000000000000000000000000006(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.radioGroup)
        view.setAccessibilityLabel("크기")
    }

    static func apply00000000000000000000000000000008(to view: NSView, selectedSize: String, onAction: @escaping (String) -> Void, onStateChange: @escaping (String, String) -> Void) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.radioButton)
        view.setAccessibilityLabel("작게")
        view.setAccessibilityValue((selectedSize == "00000000-0000-0000-0000-000000000008") ? 1 : 0)
        view.setAccessibilityCustomActions([
            NSAccessibilityCustomAction(name: "activate") {
                onAction("00000000-0000-0000-0000-00000000003d")
                onStateChange("00000000-0000-0000-0000-000000000032", "00000000-0000-0000-0000-000000000008")
                return true
            },
        ])
    }

    static func apply00000000000000000000000000000009(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.staticText)
        view.setAccessibilityValue("작게")
    }

    static func apply0000000000000000000000000000000a(to view: NSView, selectedSize: String, onAction: @escaping (String) -> Void, onStateChange: @escaping (String, String) -> Void) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.radioButton)
        view.setAccessibilityLabel("크게")
        view.setAccessibilityValue((selectedSize == "00000000-0000-0000-0000-00000000000a") ? 1 : 0)
        view.setAccessibilityCustomActions([
            NSAccessibilityCustomAction(name: "activate") {
                onAction("00000000-0000-0000-0000-00000000003e")
                onStateChange("00000000-0000-0000-0000-000000000032", "00000000-0000-0000-0000-00000000000a")
                return true
            },
        ])
    }

    static func apply0000000000000000000000000000000b(to view: NSView, memo: String) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.textField)
        view.setAccessibilityLabel("메모")
        view.setAccessibilityValue(memo)
    }

    static func apply0000000000000000000000000000000c(to view: NSView, isSaving: Bool, onAction: @escaping (String) -> Void) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.button)
        view.setAccessibilityLabel("저장")
        view.setAccessibilityEnabled(!(isSaving))
        view.setAccessibilityCustomActions([
            NSAccessibilityCustomAction(name: "activate") {
                onAction("00000000-0000-0000-0000-00000000003c")
                return true
            },
        ])
    }

    static func apply0000000000000000000000000000000d(to view: NSView) {
        view.setAccessibilityElement(false)
    }

    static func apply0000000000000000000000000000000f(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.staticText)
        view.setAccessibilityValue("새 기능")
    }

    static func apply00000000000000000000000000000012(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply00000000000000000000000000000010(to view: NSView, onAction: @escaping (String) -> Void) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.group)
        view.setAccessibilitySubrole(.dialog)
        view.setAccessibilityLabel("알림")
        view.setAccessibilityModal(true)
        view.setAccessibilityCustomActions([
            NSAccessibilityCustomAction(name: "dismiss") {
                onAction("00000000-0000-0000-0000-00000000003f")
                return true
            },
        ])
    }

    static func apply00000000000000000000000000000011(to view: NSView) {
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.staticText)
        view.setAccessibilityValue("저장되었습니다")
    }
}

@MainActor
final class StellarActionTarget: NSObject {
    private let handler: () -> Void

    init(_ handler: @escaping () -> Void) {
        self.handler = handler
    }

    @objc func fire() {
        handler()
    }
}

@MainActor
final class StellarView: NSView {
    var selectedSize: String = "00000000-0000-0000-0000-000000000008" {
        didSet { applyAccessibility() }
    }
    var memo: String = "" {
        didSet { applyAccessibility() }
    }
    var isSaving: Bool = false {
        didSet { applyAccessibility() }
    }
    let onAction: (String) -> Void
    private var views: [String: NSView] = [:]
    private var targets: [StellarActionTarget] = []

    init(onAction: @escaping (String) -> Void = { _ in }) {
        self.onAction = onAction
        super.init(frame: .zero)
        let root = makeNode00000000000000000000000000000001()
        addSubview(root)
        root.leadingAnchor.constraint(equalTo: leadingAnchor).isActive = true
        root.topAnchor.constraint(equalTo: topAnchor).isActive = true
        root.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor).isActive = true
        applyAccessibility()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    override var isFlipped: Bool { true }

    private func setState(_ id: String, _ value: String) {
        switch id {
        case "00000000-0000-0000-0000-000000000032":
            selectedSize = value
        case "00000000-0000-0000-0000-000000000033":
            memo = value
        default:
            break
        }
    }

    private func applyAccessibility() {
        if let view = views["00000000-0000-0000-0000-000000000001"] {
            StellarAccessibility.apply00000000000000000000000000000001(to: view, views: views)
        }
        if let view = views["00000000-0000-0000-0000-000000000002"] {
            StellarAccessibility.apply00000000000000000000000000000002(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000004"] {
            StellarAccessibility.apply00000000000000000000000000000004(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000005"] {
            StellarAccessibility.apply00000000000000000000000000000005(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000006"] {
            StellarAccessibility.apply00000000000000000000000000000006(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000008"] {
            StellarAccessibility.apply00000000000000000000000000000008(to: view, selectedSize: selectedSize, onAction: onAction, onStateChange: { [unowned self] id, value in self.setState(id, value) })
        }
        if let view = views["00000000-0000-0000-0000-000000000009"] {
            StellarAccessibility.apply00000000000000000000000000000009(to: view)
        }
        if let view = views["00000000-0000-0000-0000-00000000000a"] {
            StellarAccessibility.apply0000000000000000000000000000000a(to: view, selectedSize: selectedSize, onAction: onAction, onStateChange: { [unowned self] id, value in self.setState(id, value) })
        }
        if let view = views["00000000-0000-0000-0000-00000000000b"] {
            StellarAccessibility.apply0000000000000000000000000000000b(to: view, memo: memo)
        }
        if let view = views["00000000-0000-0000-0000-00000000000c"] {
            StellarAccessibility.apply0000000000000000000000000000000c(to: view, isSaving: isSaving, onAction: onAction)
        }
        if let view = views["00000000-0000-0000-0000-00000000000d"] {
            StellarAccessibility.apply0000000000000000000000000000000d(to: view)
        }
        if let view = views["00000000-0000-0000-0000-00000000000f"] {
            StellarAccessibility.apply0000000000000000000000000000000f(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000012"] {
            StellarAccessibility.apply00000000000000000000000000000012(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000010"] {
            StellarAccessibility.apply00000000000000000000000000000010(to: view, onAction: onAction)
        }
        if let view = views["00000000-0000-0000-0000-000000000011"] {
            StellarAccessibility.apply00000000000000000000000000000011(to: view)
        }
    }

    private func makeNode00000000000000000000000000000001() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 16
        view.edgeInsets = NSEdgeInsets(top: 24, left: 24, bottom: 24, right: 24)
        view.distribution = .gravityAreas
        let child1 = makeNode00000000000000000000000000000002()
        view.addView(child1, in: .leading)
        let child2 = makeNode00000000000000000000000000000004()
        view.addView(child2, in: .leading)
        let child3 = makeNode00000000000000000000000000000005()
        view.addView(child3, in: .leading)
        child3.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -48).isActive = true
        let child4 = makeNode00000000000000000000000000000006()
        view.addView(child4, in: .leading)
        child4.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -48).isActive = true
        let child5 = makeNode0000000000000000000000000000000b()
        view.addView(child5, in: .leading)
        child5.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -48).isActive = true
        let child6 = makeNode0000000000000000000000000000000c()
        view.addView(child6, in: .leading)
        child6.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -48).isActive = true
        let child7 = makeNode0000000000000000000000000000000d()
        view.addView(child7, in: .leading)
        child7.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -48).isActive = true
        let child8 = makeNode00000000000000000000000000000010()
        view.addView(child8, in: .leading)
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 390).isActive = true
        view.heightAnchor.constraint(equalToConstant: 844).isActive = true
        views["00000000-0000-0000-0000-000000000001"] = view
        return view
    }

    private func makeNode00000000000000000000000000000002() -> NSView {
        let view = NSTextField(labelWithString: "설정")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000002"] = view
        return view
    }

    private func makeNode00000000000000000000000000000004() -> NSView {
        let view = NSImageView(image: NSImage(systemSymbolName: "person.crop.circle", accessibilityDescription: nil) ?? NSImage())
        view.imageScaling = .scaleProportionallyUpOrDown
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 64).isActive = true
        view.heightAnchor.constraint(equalToConstant: 64).isActive = true
        views["00000000-0000-0000-0000-000000000004"] = view
        return view
    }

    private func makeNode00000000000000000000000000000005() -> NSView {
        let view = NSView()
        view.translatesAutoresizingMaskIntoConstraints = false
        view.heightAnchor.constraint(equalToConstant: 1).isActive = true
        views["00000000-0000-0000-0000-000000000005"] = view
        return view
    }

    private func makeNode00000000000000000000000000000006() -> NSView {
        let view = NSStackView()
        view.orientation = .horizontal
        view.alignment = .centerY
        view.spacing = 8
        view.distribution = .fill
        let child1 = makeNode00000000000000000000000000000007()
        view.addArrangedSubview(child1)
        child1.setContentHuggingPriority(.init(1), for: .horizontal)
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000006"] = view
        return view
    }

    private func makeNode00000000000000000000000000000007() -> NSView {
        let view = NSStackView()
        view.orientation = .horizontal
        view.alignment = .centerY
        view.spacing = 8
        view.distribution = .fill
        let child1 = makeNode00000000000000000000000000000008()
        view.addArrangedSubview(child1)
        child1.setContentHuggingPriority(.init(1), for: .horizontal)
        let child2 = makeNode0000000000000000000000000000000a()
        view.addArrangedSubview(child2)
        child2.setContentHuggingPriority(.init(1), for: .horizontal)
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000007"] = view
        return view
    }

    private func makeNode00000000000000000000000000000008() -> NSView {
        let view = NSStackView()
        view.orientation = .horizontal
        view.alignment = .top
        view.spacing = 0
        view.edgeInsets = NSEdgeInsets(top: 12, left: 16, bottom: 12, right: 16)
        view.distribution = .gravityAreas
        let child1 = makeNode00000000000000000000000000000009()
        view.addView(child1, in: .center)
        child1.heightAnchor.constraint(equalTo: view.heightAnchor, constant: -24).isActive = true
        view.translatesAutoresizingMaskIntoConstraints = false
        let target = StellarActionTarget { [unowned self] in
            self.onAction("00000000-0000-0000-0000-00000000003d")
            self.selectedSize = "00000000-0000-0000-0000-000000000008"
        }
        targets.append(target)
        view.addGestureRecognizer(NSClickGestureRecognizer(target: target, action: #selector(StellarActionTarget.fire)))
        views["00000000-0000-0000-0000-000000000008"] = view
        return view
    }

    private func makeNode00000000000000000000000000000009() -> NSView {
        let view = NSTextField(labelWithString: "작게")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000009"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000a() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.edgeInsets = NSEdgeInsets(top: 12, left: 16, bottom: 12, right: 16)
        view.translatesAutoresizingMaskIntoConstraints = false
        let target = StellarActionTarget { [unowned self] in
            self.onAction("00000000-0000-0000-0000-00000000003e")
            self.selectedSize = "00000000-0000-0000-0000-00000000000a"
        }
        targets.append(target)
        view.addGestureRecognizer(NSClickGestureRecognizer(target: target, action: #selector(StellarActionTarget.fire)))
        views["00000000-0000-0000-0000-00000000000a"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000b() -> NSView {
        let view = NSTextField(string: memo)
        view.placeholderString = "메모"
        let target = StellarActionTarget { [unowned self, unowned view] in
            self.memo = view.stringValue
        }
        targets.append(target)
        view.target = target
        view.action = #selector(StellarActionTarget.fire)
        view.translatesAutoresizingMaskIntoConstraints = false
        let padded = pad(view, top: 8, left: 12, bottom: 8, right: 12)
        do {
            let view = padded
            view.heightAnchor.constraint(equalToConstant: 44).isActive = true
        }
        views["00000000-0000-0000-0000-00000000000b"] = padded
        return padded
    }

    private func makeNode0000000000000000000000000000000c() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        let target = StellarActionTarget { [unowned self] in
            guard !(self.isSaving) else { return }
            self.onAction("00000000-0000-0000-0000-00000000003c")
        }
        targets.append(target)
        view.addGestureRecognizer(NSClickGestureRecognizer(target: target, action: #selector(StellarActionTarget.fire)))
        view.widthAnchor.constraint(greaterThanOrEqualToConstant: 120).isActive = true
        view.heightAnchor.constraint(equalToConstant: 48).isActive = true
        views["00000000-0000-0000-0000-00000000000c"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000d() -> NSView {
        let cell1 = makeNode0000000000000000000000000000000f()
        let cell2 = makeNode00000000000000000000000000000012()
        let grid = NSGridView(views: [[cell1, cell2]])
        grid.rowSpacing = 0
        grid.columnSpacing = 8
        grid.xPlacement = .fill
        grid.yPlacement = .fill
        let view = grid
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-00000000000d"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000f() -> NSView {
        let view = NSTextField(labelWithString: "새 기능")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-00000000000f"] = view
        return view
    }

    private func makeNode00000000000000000000000000000012() -> NSView {
        let view = NSView()
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 24).isActive = true
        view.heightAnchor.constraint(equalToConstant: 24).isActive = true
        views["00000000-0000-0000-0000-000000000012"] = view
        return view
    }

    private func makeNode00000000000000000000000000000010() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        let overlay1 = makeNode00000000000000000000000000000011()
        view.addSubview(overlay1)
        overlay1.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16).isActive = true
        overlay1.topAnchor.constraint(equalTo: view.topAnchor, constant: 16).isActive = true
        view.widthAnchor.constraint(equalToConstant: 320).isActive = true
        view.heightAnchor.constraint(equalToConstant: 120).isActive = true
        views["00000000-0000-0000-0000-000000000010"] = view
        return view
    }

    private func makeNode00000000000000000000000000000011() -> NSView {
        let view = NSTextField(labelWithString: "저장되었습니다")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000011"] = view
        return view
    }

    private func pad(_ content: NSView, top: CGFloat, left: CGFloat, bottom: CGFloat, right: CGFloat) -> NSView {
        let view = NSView()
        view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: left),
            content.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -right),
            content.topAnchor.constraint(equalTo: view.topAnchor, constant: top),
            content.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -bottom),
        ])
        return view
    }
}
