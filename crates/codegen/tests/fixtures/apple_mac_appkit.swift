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
