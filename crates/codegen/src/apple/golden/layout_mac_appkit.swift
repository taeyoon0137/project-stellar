import AppKit

enum StellarColors {
}

@MainActor
enum StellarAccessibility {
    static func apply00000000000000000000000000000001(to view: NSView) {
        view.setAccessibilityElement(false)
    }

    static func apply00000000000000000000000000000002(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply00000000000000000000000000000005(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply00000000000000000000000000000008(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply0000000000000000000000000000000b(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply0000000000000000000000000000000c(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
    }

    static func apply0000000000000000000000000000000e(to view: NSView) {
        view.setAccessibilityElement(false)
        view.setAccessibilityChildren([])
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

    private func applyAccessibility() {
        if let view = views["00000000-0000-0000-0000-000000000001"] {
            StellarAccessibility.apply00000000000000000000000000000001(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000002"] {
            StellarAccessibility.apply00000000000000000000000000000002(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000005"] {
            StellarAccessibility.apply00000000000000000000000000000005(to: view)
        }
        if let view = views["00000000-0000-0000-0000-000000000008"] {
            StellarAccessibility.apply00000000000000000000000000000008(to: view)
        }
        if let view = views["00000000-0000-0000-0000-00000000000b"] {
            StellarAccessibility.apply0000000000000000000000000000000b(to: view)
        }
        if let view = views["00000000-0000-0000-0000-00000000000e"] {
            StellarAccessibility.apply0000000000000000000000000000000e(to: view)
        }
    }

    private func makeNode00000000000000000000000000000001() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 8
        view.edgeInsets = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
        view.distribution = .equalSpacing
        let child1 = makeNode00000000000000000000000000000002()
        view.addArrangedSubview(child1)
        child1.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -32).isActive = true
        let child2 = makeNode00000000000000000000000000000005()
        view.addArrangedSubview(child2)
        child2.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -32).isActive = true
        let child3 = makeNode00000000000000000000000000000008()
        view.addArrangedSubview(child3)
        child3.widthAnchor.constraint(equalTo: view.widthAnchor, constant: -32).isActive = true
        let child4 = makeNode0000000000000000000000000000000b()
        view.addArrangedSubview(child4)
        let child5 = makeNode0000000000000000000000000000000e()
        view.addArrangedSubview(child5)
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 392).isActive = true
        view.heightAnchor.constraint(equalToConstant: 672).isActive = true
        views["00000000-0000-0000-0000-000000000001"] = view
        return view
    }

    private func makeNode00000000000000000000000000000002() -> NSView {
        let view = NSStackView()
        view.orientation = .horizontal
        view.alignment = .firstBaseline
        view.spacing = 4
        view.distribution = .gravityAreas
        let child1 = makeNode00000000000000000000000000000003()
        view.addView(child1, in: .leading)
        let child2 = makeNode00000000000000000000000000000004()
        view.addView(child2, in: .leading)
        child2.setContentCompressionResistancePriority(.required, for: .horizontal)
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000002"] = view
        return view
    }

    private func makeNode00000000000000000000000000000003() -> NSView {
        let view = NSTextField(labelWithString: "가")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000003"] = view
        return view
    }

    private func makeNode00000000000000000000000000000004() -> NSView {
        let view = NSTextField(labelWithString: "나")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000004"] = view
        return view
    }

    private func makeNode00000000000000000000000000000005() -> NSView {
        let view = NSStackView()
        view.orientation = .horizontal
        view.alignment = .top
        view.spacing = 0
        view.distribution = .fill
        let child1 = makeNode00000000000000000000000000000007()
        view.addArrangedSubview(child1)
        child1.heightAnchor.constraint(equalTo: view.heightAnchor, constant: 0).isActive = true
        child1.setContentHuggingPriority(.init(1), for: .horizontal)
        let child2 = makeNode00000000000000000000000000000006()
        view.addArrangedSubview(child2)
        child2.heightAnchor.constraint(equalTo: view.heightAnchor, constant: 0).isActive = true
        child2.setContentHuggingPriority(.init(1), for: .horizontal)
        view.translatesAutoresizingMaskIntoConstraints = false
        view.heightAnchor.constraint(equalToConstant: 40).isActive = true
        views["00000000-0000-0000-0000-000000000005"] = view
        return view
    }

    private func makeNode00000000000000000000000000000006() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000006"] = view
        return view
    }

    private func makeNode00000000000000000000000000000007() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000007"] = view
        return view
    }

    private func makeNode00000000000000000000000000000008() -> NSView {
        let cell1 = makeNode00000000000000000000000000000009()
        let cell2 = makeNode0000000000000000000000000000000a()
        let grid = NSGridView(views: [[cell1, cell2, NSGridCell.emptyContentView]])
        grid.rowSpacing = 0
        grid.columnSpacing = 8
        grid.xPlacement = .fill
        grid.yPlacement = .fill
        grid.column(at: 0).width = 100
        grid.row(at: 0).height = 40
        let view = grid
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000008"] = view
        return view
    }

    private func makeNode00000000000000000000000000000009() -> NSView {
        let view = NSTextField(labelWithString: "넓은 칸")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-000000000009"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000a() -> NSView {
        let view = NSTextField(labelWithString: "자동")
        view.translatesAutoresizingMaskIntoConstraints = false
        views["00000000-0000-0000-0000-00000000000a"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000b() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 160).isActive = true
        view.widthAnchor.constraint(equalTo: view.heightAnchor, multiplier: 16 / 9).isActive = true
        views["00000000-0000-0000-0000-00000000000b"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000e() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        let overlay1 = makeNode0000000000000000000000000000000f()
        view.addSubview(overlay1)
        overlay1.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8).isActive = true
        overlay1.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -8).isActive = true
        let overlay2 = makeNode00000000000000000000000000000010()
        view.addSubview(overlay2)
        overlay2.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 0).isActive = true
        overlay2.topAnchor.constraint(equalTo: view.topAnchor, constant: 0).isActive = true
        view.widthAnchor.constraint(equalToConstant: 200).isActive = true
        view.heightAnchor.constraint(equalToConstant: 100).isActive = true
        views["00000000-0000-0000-0000-00000000000e"] = view
        return view
    }

    private func makeNode0000000000000000000000000000000f() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 20).isActive = true
        view.heightAnchor.constraint(equalToConstant: 20).isActive = true
        views["00000000-0000-0000-0000-00000000000f"] = view
        return view
    }

    private func makeNode00000000000000000000000000000010() -> NSView {
        let view = NSStackView()
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 0
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalToConstant: 10).isActive = true
        view.heightAnchor.constraint(equalToConstant: 10).isActive = true
        views["00000000-0000-0000-0000-000000000010"] = view
        return view
    }
}
