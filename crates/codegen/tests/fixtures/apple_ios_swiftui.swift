import SwiftUI

enum StellarColors {
    static let surface = Color(.displayP3, red: 0.96, green: 0.96, blue: 0.98, opacity: 1)
    static let accent = Color(.sRGB, red: 0, green: 0.48, blue: 1, opacity: 1)
    static let n00000000000000000000000000000001_4 = surface
    static let n00000000000000000000000000000002_5 = Color(.sRGB, red: 0.8, green: 0.1, blue: 0.1, opacity: 1)
    static let n0000000000000000000000000000000c_4 = accent
}

struct StellarAccessibility00000000000000000000000000000001: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
    }
}

struct StellarAccessibility00000000000000000000000000000002: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
            .accessibilitySortPriority(5)
    }
}

struct StellarAccessibility00000000000000000000000000000004: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isImage)
            .accessibilityLabel(Text(verbatim: "프로필 사진"))
            .accessibilitySortPriority(6)
    }
}

struct StellarAccessibility00000000000000000000000000000005: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility00000000000000000000000000000006: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(verbatim: "크기"))
            .accessibilitySortPriority(4)
    }
}

struct StellarAccessibility00000000000000000000000000000008: ViewModifier {
    @Binding var selectedSize: String
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAddTraits(selectedSize == "00000000-0000-0000-0000-000000000008" ? .isSelected : [])
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003d")
                selectedSize = "00000000-0000-0000-0000-000000000008"
            }
    }
}

struct StellarAccessibility00000000000000000000000000000009: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
    }
}

struct StellarAccessibility0000000000000000000000000000000a: ViewModifier {
    @Binding var selectedSize: String
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(Text(verbatim: "크게"))
            .accessibilityAddTraits(selectedSize == "00000000-0000-0000-0000-00000000000a" ? .isSelected : [])
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003e")
                selectedSize = "00000000-0000-0000-0000-00000000000a"
            }
    }
}

struct StellarAccessibility0000000000000000000000000000000b: ViewModifier {
    @Binding var memo: String

    func body(content: Content) -> some View {
        content
            .accessibilityLabel(Text(verbatim: "메모"))
            .accessibilityValue(Text(memo))
            .accessibilitySortPriority(3)
    }
}

struct StellarAccessibility0000000000000000000000000000000c: ViewModifier {
    @Binding var isSaving: Bool
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(Text(verbatim: "저장"))
            .disabled(isSaving)
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003c")
            }
            .accessibilitySortPriority(2)
    }
}

struct StellarAccessibility00000000000000000000000000000010: ViewModifier {
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(verbatim: "알림"))
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape) {
                onAction("00000000-0000-0000-0000-00000000003f")
            }
            .accessibilitySortPriority(1)
    }
}

struct StellarAccessibility00000000000000000000000000000011: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
    }
}
